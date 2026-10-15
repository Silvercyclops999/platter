//! Headphone correction profiles from AutoEq (github.com/jaakkopasanen/AutoEq,
//! MIT). A small index of names and folders ships with the app
//! (`autoeq_index.txt`, rebuilt by `scripts/autoeq-index.mjs`); a profile's
//! parametric EQ text is fetched only when someone picks it, from an allowed
//! host, and cached on disk.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::dsp::{CORRECTION_MAX, FilterKind, PeqFilter};
use crate::error::{AppError, AppResult};

const INDEX: &str = include_str!("autoeq_index.txt");
/// Where profiles are fetched from.
pub const PROFILE_BASE: &str = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/";
/// The only hosts a profile is fetched from.
pub const ALLOWED_HOSTS: &[&str] = &["raw.githubusercontent.com"];
/// Profiles are a few hundred bytes; anything far bigger isn't one.
const MAX_PROFILE_BYTES: usize = 64 * 1024;
pub const SEARCH_LIMIT: usize = 50;

/// One headphone in the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    /// Who measured it, and on what rig when they used several.
    pub source: String,
    /// earbud, in-ear or over-ear.
    pub form: String,
    /// `<folder>/<name>`, the profile's place in the results.
    pub path: String,
}

struct Row {
    name: &'static str,
    dir: usize,
    lower: String,
}

struct Index {
    /// (folder, source label, form)
    dirs: Vec<(&'static str, &'static str, &'static str)>,
    rows: Vec<Row>,
}

fn index() -> &'static Index {
    static I: OnceLock<Index> = OnceLock::new();
    I.get_or_init(|| parse_index(INDEX))
}

fn parse_index(text: &'static str) -> Index {
    let mut dirs = Vec::new();
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('@') {
            let (dir, label) = rest.split_once('\t').unwrap_or((rest, rest));
            let tail = dir.rsplit('/').next().unwrap_or(dir);
            let form = ["over-ear", "in-ear", "earbud"].into_iter().find(|f| tail.ends_with(f)).unwrap_or("");
            dirs.push((dir, label, form));
        } else if !dirs.is_empty() {
            rows.push(Row { name: line, dir: dirs.len() - 1, lower: fold(line) });
        }
    }
    Index { dirs, rows }
}

/// Lowercase without accents, so "bohm" finds "BÖHM".
fn fold(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut out = String::with_capacity(s.len());
    for c in s.nfkd().flat_map(char::to_lowercase) {
        match c {
            'ø' => out.push('o'),
            'æ' => out.push_str("ae"),
            'ß' => out.push_str("ss"),
            'ł' => out.push('l'),
            'đ' => out.push('d'),
            c if c.is_alphanumeric() || c.is_whitespace() || c.is_ascii_punctuation() => out.push(c),
            _ => {}
        }
    }
    out
}

fn entry(i: &Index, r: &Row) -> Entry {
    let (dir, label, form) = i.dirs[r.dir];
    Entry { name: r.name.into(), source: label.into(), form: form.into(), path: format!("{dir}/{}", r.name) }
}

/// How many headphones the bundled index lists.
pub fn index_len() -> usize {
    index().rows.len()
}

/// Headphones whose name has every word of `query`, best matches first:
/// names starting with the query, then names with a word starting with it,
/// then the rest; shorter names first within each.
pub fn search(query: &str, limit: usize) -> Vec<Entry> {
    let q = fold(query.trim());
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let i = index();
    let mut hits: Vec<(u8, usize, &Row)> = i
        .rows
        .iter()
        .filter(|r| words.iter().all(|w| r.lower.contains(w)))
        .map(|r| {
            let rank = if r.lower.starts_with(&q) {
                0
            } else if r.lower.contains(&format!(" {q}")) {
                1
            } else {
                2
            };
            (rank, r.name.len(), r)
        })
        .collect();
    hits.sort_by(|a, b| (a.0, a.1, a.2.name).cmp(&(b.0, b.1, b.2.name)));
    hits.into_iter().take(limit.min(SEARCH_LIMIT)).map(|(_, _, r)| entry(i, r)).collect()
}

/// The index entry at `path` (None: not a headphone the app knows).
pub fn find(path: &str) -> Option<Entry> {
    let i = index();
    let (dir, name) = path.rsplit_once('/')?;
    i.rows.iter().find(|r| r.name == name && i.dirs[r.dir].0 == dir).map(|r| entry(i, r))
}

/// A parsed profile.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Parsed {
    pub preamp_db: f32,
    pub filters: Vec<PeqFilter>,
}

/// Reads parametric EQ text (`Preamp: -6.2 dB` and lines like
/// `Filter 1: ON PK Fc 105 Hz Gain 6.6 dB Q 0.70`). Peaking (PK), low shelf
/// (LSC, LS) and high shelf (HSC, HS) filters are used; ones marked OFF are
/// skipped. A preamp above 0 dB is taken as 0, and when the file has none
/// one is worked out from the biggest boost, so the curve can't clip.
pub fn parse(text: &str) -> AppResult<Parsed> {
    let bad = |line: &str| AppError::Decode(format!("not a parametric EQ line: {line}"));
    let mut preamp: Option<f32> = None;
    let mut filters = Vec::new();
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("Preamp:") {
            let v = rest.split_whitespace().next().and_then(|v| v.parse::<f32>().ok()).ok_or_else(|| bad(line))?;
            preamp = Some(v);
            continue;
        }
        let Some((head, body)) = line.split_once(':') else { return Err(bad(line)) };
        if !head.trim_start().starts_with("Filter") {
            return Err(bad(line));
        }
        let t: Vec<&str> = body.split_whitespace().collect();
        if t.first() != Some(&"ON") {
            continue;
        }
        let kind = match t.get(1).copied() {
            Some("PK") | Some("PEQ") => FilterKind::Peaking,
            Some("LSC") | Some("LS") => FilterKind::LowShelf,
            Some("HSC") | Some("HS") => FilterKind::HighShelf,
            Some(other) => return Err(AppError::Decode(format!("filter type {other} isn't supported"))),
            None => return Err(bad(line)),
        };
        let num = |key: &str| -> Option<f32> {
            let at = t.iter().position(|w| *w == key)?;
            t.get(at + 1)?.parse::<f32>().ok().filter(|v| v.is_finite())
        };
        let freq = num("Fc").filter(|f| *f > 0.0).ok_or_else(|| bad(line))?;
        let gain_db = num("Gain").ok_or_else(|| bad(line))?.clamp(-30.0, 30.0);
        let q = num("Q").filter(|q| *q > 0.0).unwrap_or(std::f32::consts::FRAC_1_SQRT_2).min(40.0);
        filters.push(PeqFilter { kind, freq, gain_db, q });
    }
    if filters.is_empty() {
        return Err(AppError::Decode("the profile has no filters".into()));
    }
    filters.truncate(CORRECTION_MAX);
    let preamp_db = match preamp {
        Some(p) => p.min(0.0),
        None => -filters.iter().map(|f| f.gain_db).fold(0.0f32, f32::max),
    };
    Ok(Parsed { preamp_db, filters })
}

/// The address of a profile's parametric EQ text.
pub fn profile_url(base: &str, path: &str) -> AppResult<url::Url> {
    use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
    const SEG: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?').add(b'`').add(b'{').add(b'}').add(b'/').add(b'\\').add(b'+').add(b'&');
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut s = base.to_string();
    for seg in path.split('/') {
        s.push_str(&utf8_percent_encode(seg, SEG).to_string());
        s.push('/');
    }
    s.push_str(&utf8_percent_encode(&format!("{name} ParametricEQ.txt"), SEG).to_string());
    url::Url::parse(&s).map_err(|e| AppError::Internal(e.to_string()))
}

/// Fetches profiles from an allowed host and keeps them on disk.
#[derive(Clone)]
pub struct Fetcher {
    base: String,
    allowed: Vec<String>,
    cache_dir: PathBuf,
    client: reqwest::Client,
}

impl Fetcher {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self::with_base(cache_dir, PROFILE_BASE, ALLOWED_HOSTS)
    }

    pub fn with_base(cache_dir: PathBuf, base: &str, allowed: &[&str]) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self { base: base.into(), allowed: allowed.iter().map(|h| h.to_string()).collect(), cache_dir, client }
    }

    fn cache_file(&self, path: &str) -> PathBuf {
        let hash = Sha256::digest(path.as_bytes());
        let hex: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
        self.cache_dir.join(format!("{hex}.txt"))
    }

    /// The profile at `path` (an index path), from the cache or the web.
    pub async fn profile(&self, path: &str) -> AppResult<Parsed> {
        let file = self.cache_file(path);
        if let Ok(text) = tokio::fs::read_to_string(&file).await
            && let Ok(p) = parse(&text)
        {
            return Ok(p);
        }
        let url = profile_url(&self.base, path)?;
        if !url.host_str().is_some_and(|h| self.allowed.iter().any(|a| a == h)) {
            return Err(AppError::Invalid(format!("{} isn't an allowed host for profiles", url.host_str().unwrap_or("?"))));
        }
        let res = self.client.get(url).send().await.map_err(|e| AppError::Network(e.to_string()))?;
        match res.status().as_u16() {
            200 => {}
            404 => return Err(AppError::NotFound("That headphone's profile isn't there any more.".into())),
            s => return Err(AppError::Server(format!("profile download failed ({s})"))),
        }
        let body = res.bytes().await.map_err(|e| AppError::Network(e.to_string()))?;
        if body.len() > MAX_PROFILE_BYTES {
            return Err(AppError::Decode("the profile is too big".into()));
        }
        let text = String::from_utf8_lossy(&body).into_owned();
        let parsed = parse(&text)?;
        if let Err(e) = write_cache(&file, &text).await {
            tracing::debug!("headphone profile cache: {e}");
        }
        Ok(parsed)
    }
}

async fn write_cache(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    tokio::fs::write(file, text).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::dsp::Correction;

    const HD600: &str = "Preamp: -6.3 dB
Filter 1: ON LSC Fc 105 Hz Gain 6.5 dB Q 0.70
Filter 2: ON PK Fc 125 Hz Gain -2.7 dB Q 0.55
Filter 3: ON PK Fc 8445 Hz Gain 3.3 dB Q 1.61
Filter 4: ON PK Fc 522 Hz Gain 0.7 dB Q 1.02
Filter 5: ON PK Fc 1298 Hz Gain -1.2 dB Q 2.14
Filter 6: ON HSC Fc 10000 Hz Gain -3.1 dB Q 0.70
Filter 7: ON PK Fc 3158 Hz Gain -1.8 dB Q 3.67
Filter 8: ON PK Fc 2166 Hz Gain 0.9 dB Q 3.32
Filter 9: ON PK Fc 6639 Hz Gain 2.2 dB Q 5.82
Filter 10: ON PK Fc 5433 Hz Gain -1.2 dB Q 5.70
";

    #[test]
    fn parses_a_published_profile() {
        let p = parse(HD600).unwrap();
        assert_eq!(p.preamp_db, -6.3);
        assert_eq!(p.filters.len(), 10);
        assert_eq!(p.filters[0], PeqFilter { kind: FilterKind::LowShelf, freq: 105.0, gain_db: 6.5, q: 0.7 });
        assert_eq!(p.filters[5].kind, FilterKind::HighShelf);
        assert_eq!(p.filters[9], PeqFilter { kind: FilterKind::Peaking, freq: 5433.0, gain_db: -1.2, q: 5.7 });
    }

    #[test]
    fn skips_off_filters_and_reads_short_shelf_names() {
        let p = parse("\u{feff}Preamp: -3 dB\r\nFilter 1: OFF PK Fc 100 Hz Gain 3 dB Q 1\r\nFilter 2: ON LS Fc 80 Hz Gain 3 dB\r\nFilter 3: ON HS Fc 9000 Hz Gain -2 dB Q 0.7\r\n").unwrap();
        assert_eq!(p.filters.len(), 2);
        assert_eq!(p.filters[0].kind, FilterKind::LowShelf);
        // No Q: the usual shelf slope.
        assert!((p.filters[0].q - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(p.filters[1].kind, FilterKind::HighShelf);
    }

    #[test]
    fn a_missing_or_positive_preamp_stays_clip_safe() {
        let p = parse("Filter 1: ON PK Fc 1000 Hz Gain 4 dB Q 1\nFilter 2: ON PK Fc 3000 Hz Gain -2 dB Q 1").unwrap();
        assert_eq!(p.preamp_db, -4.0);
        let p = parse("Preamp: 2 dB\nFilter 1: ON PK Fc 1000 Hz Gain 1 dB Q 1").unwrap();
        assert_eq!(p.preamp_db, 0.0);
    }

    #[test]
    fn rejects_text_that_isnt_a_profile() {
        assert!(parse("").is_err());
        assert!(parse("<html>Not found</html>").is_err());
        assert!(parse("Preamp: -1 dB\nFilter 1: ON LP Fc 100 Hz").is_err());
        assert!(parse("Filter 1: ON PK Fc x Hz Gain 1 dB Q 1").is_err());
    }

    #[test]
    fn the_parsed_curve_is_what_the_profile_says() {
        let p = parse(HD600).unwrap();
        let c = Correction::new(p.preamp_db, &p.filters);
        // Bass shelf: about +6.5 dB well below 105 Hz, minus the preamp
        // and the 125 Hz cut's tail.
        let low = c.response_db(48_000, 20.0);
        assert!((low - (6.5 - 6.3)).abs() < 1.0, "20 Hz: {low}");
        // Mid band: little but the preamp.
        let mid = c.response_db(48_000, 800.0);
        assert!((mid + 6.3).abs() < 1.5, "800 Hz: {mid}");
        // The 8445 Hz peak is there.
        let peak = c.response_db(48_000, 8445.0);
        assert!(peak > mid + 1.0, "8445 Hz: {peak}");
        // Never louder than the input (the preamp covers the boosts).
        assert!(c.max_gain(48_000) <= 1.07);
    }

    #[test]
    fn bundled_index_searches() {
        assert!(index_len() > 1000);
        let hits = search("hd 600", 10);
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.name.to_lowercase().contains("hd 600")));
        let first = &hits[0];
        assert!(first.name.starts_with("Sennheiser HD 600") || first.name.contains("HD 600"));
        assert!(["over-ear", "in-ear", "earbud"].contains(&first.form.as_str()));
        assert_eq!(find(&first.path).as_ref(), Some(first));
        assert!(search("   ", 10).is_empty());
        assert!(search("hd", 500).len() <= SEARCH_LIMIT);
        assert_eq!(find("nowhere/Nothing"), None);
    }

    #[test]
    fn search_ignores_accents_and_case() {
        let hits = search("rode nth-100", 5);
        assert!(hits.iter().any(|h| h.name.contains("NTH-100")), "{hits:?}");
    }

    #[test]
    fn profile_urls_are_encoded_per_segment() {
        let u = profile_url(PROFILE_BASE, "oratory1990/over-ear/Sennheiser HD 600").unwrap();
        assert_eq!(
            u.as_str(),
            "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/oratory1990/over-ear/Sennheiser%20HD%20600/Sennheiser%20HD%20600%20ParametricEQ.txt"
        );
        let u = profile_url(PROFILE_BASE, "x/1MORE Aero (ANC On) #2").unwrap();
        assert!(u.as_str().ends_with("/x/1MORE%20Aero%20(ANC%20On)%20%232/1MORE%20Aero%20(ANC%20On)%20%232%20ParametricEQ.txt"));
    }

    #[tokio::test]
    async fn fetches_once_then_uses_the_cache() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/r/src/over-ear/Test%20Phone/Test%20Phone%20ParametricEQ.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(HD600))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let base = format!("{}/r/", server.uri());
        let f = Fetcher::with_base(dir.path().to_path_buf(), &base, &["127.0.0.1"]);
        let a = f.profile("src/over-ear/Test Phone").await.unwrap();
        let b = f.profile("src/over-ear/Test Phone").await.unwrap();
        assert_eq!(a, b);
        assert_eq!(a.filters.len(), 10);
    }

    #[tokio::test]
    async fn only_allowed_hosts_are_asked() {
        let dir = tempfile::tempdir().unwrap();
        let f = Fetcher::with_base(dir.path().to_path_buf(), "http://127.0.0.1:9/r/", &["raw.githubusercontent.com"]);
        let e = f.profile("a/b").await.unwrap_err();
        assert!(matches!(e, AppError::Invalid(_)), "{e:?}");
    }

    #[tokio::test]
    async fn a_page_that_isnt_a_profile_is_an_error_and_not_cached() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_string("<html>")).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let f = Fetcher::with_base(dir.path().to_path_buf(), &format!("{}/", server.uri()), &["127.0.0.1"]);
        assert!(f.profile("a/b").await.is_err());
        assert_eq!(std::fs::read_dir(dir.path()).map(|d| d.count()).unwrap_or(0), 0);
    }
}
