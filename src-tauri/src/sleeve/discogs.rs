//! Reading a Discogs release for the sleeve notes: its credits (Discogs'
//! "extra artists", on the release and on each track), label and catalogue
//! number, country, barcode, format, studios and notes. Only what the notes
//! show is kept in the cache.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{LibSong, Part, RawCredit, TrackPart};
use crate::identity::fold::{fold_album, fold_artist};
use crate::physical::clean_discogs_artist;

/// Longest notes kept (characters).
pub const NOTES_MAX: usize = 2000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DCredit {
    pub name: String,
    pub role: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DTrack {
    pub position: String,
    pub title: String,
    pub credits: Vec<DCredit>,
}

/// A Discogs release as the cache keeps it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscogsRelease {
    pub id: u64,
    pub released: Option<String>,
    pub country: Option<String>,
    pub label: Option<String>,
    pub catalogue: Option<String>,
    pub barcode: Option<String>,
    pub format: Option<String>,
    pub edition: Option<String>,
    pub notes: Option<String>,
    /// (what was done there, studio).
    pub studios: Vec<(String, String)>,
    pub credits: Vec<DCredit>,
    pub tracks: Vec<DTrack>,
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty())
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// Word starts that make a credit a performance; the credit's text becomes the instrument.
const INSTRUMENTS: &[&str] = &[
    "vocal", "voice", "guitar", "bass", "drum", "percussion", "keyboard", "piano", "organ", "synth", "saxophone", "sax", "trumpet",
    "trombone", "horn", "violin", "viola", "cello", "strings", "flute", "clarinet", "harmonica", "banjo", "mandolin", "harp",
    "turntables", "rap", "programmed", "programming", "electronics", "sampler", "tabla", "sitar", "oboe", "tuba", "accordion",
];

/// Splits Discogs' role text ("Recorded By, Mixed By", "Guitar [Lead], Vocals")
/// on commas outside brackets.
fn split_roles(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut cur) = (0i32, String::new());
    for ch in text.chars() {
        match ch {
            '[' => depth += 1,
            ']' => depth -= 1,
            ',' if depth <= 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    out.push(cur);
    out.into_iter().map(|r| r.trim().to_owned()).filter(|r| !r.is_empty()).collect()
}

/// The sleeve roles (with an instrument for performers) a Discogs role text names.
pub fn roles(text: &str) -> Vec<(&'static str, Option<String>)> {
    let mut out = Vec::new();
    for part in split_roles(text) {
        // "Guitar [Lead]" reads as "guitar"; the bracket is a refinement.
        let base = part.split('[').next().unwrap_or(&part).trim().to_lowercase();
        let has = |w: &str| base.contains(w);
        let role: Option<&'static str> = if has("executive") || has("assistant") || has("lacquer") {
            None
        } else if has("mastered") || has("mastering") {
            Some("mastering")
        } else if has("mixed") || has("mixing") || base == "mixer" {
            Some("mixer")
        } else if has("producer") || has("produced") {
            Some("producer")
        } else if has("written") || has("songwriter") || has("words and music") {
            Some("writer")
        } else if has("lyrics") || has("words by") {
            Some("lyricist")
        } else if has("music by") || has("composed") || base == "composer" {
            Some("composer")
        } else if has("arranged") || has("arranger") || has("orchestrated") {
            Some("arranger")
        } else if has("conductor") {
            Some("conductor")
        } else if has("featuring") {
            Some("featured")
        } else if has("engineer") || has("recorded by") {
            Some("engineer")
        } else if base.split(|c: char| !c.is_alphanumeric()).any(|w| INSTRUMENTS.iter().any(|i| w.starts_with(i))) {
            let detail = if has("programmed") { "programming".to_owned() } else { base.clone() };
            out.push(("performer", Some(detail)));
            continue;
        } else {
            None
        };
        if let Some(r) = role
            && !out.iter().any(|(x, _)| *x == r)
        {
            out.push((r, None));
        }
    }
    out
}

fn credits(list: &[Value]) -> Vec<DCredit> {
    let mut out: Vec<DCredit> = Vec::new();
    for a in list {
        let Some(name) = s(a, "name").map(clean_discogs_artist).filter(|n| !n.is_empty()) else { continue };
        for (role, detail) in roles(s(a, "role").unwrap_or_default()) {
            let c = DCredit { name: name.clone(), role: role.to_owned(), detail };
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// Discogs' markup in notes as plain text: `[a=Name]` and `[l=Label]` keep
/// the name, `[url=...]text[/url]` keeps the text, other tags go.
pub fn plain_notes(n: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = n;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find(']') else {
            out.push('[');
            rest = after;
            continue;
        };
        let tag = &after[..j];
        if let Some((k, v)) = tag.split_once('=')
            && matches!(k, "a" | "l")
        {
            out.push_str(&clean_discogs_artist(v));
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    let t = out.trim().replace("\r\n", "\n");
    if t.is_empty() {
        return None;
    }
    Some(if t.chars().count() > NOTES_MAX { format!("{}…", t.chars().take(NOTES_MAX).collect::<String>().trim_end()) } else { t })
}

/// Generic format words that say nothing about an edition.
const PLAIN_FORMATS: &[&str] = &["lp", "album", "cd", "ep", "single", "stereo", "mono", "33 ⅓ rpm", "45 rpm", "12\"", "7\"", "10\"", "cassette", "file", "flac", "mp3"];

pub fn parse_release(v: &Value) -> Option<DiscogsRelease> {
    let id = v.get("id").and_then(Value::as_u64).filter(|i| *i > 0)?;
    let labels = arr(v, "labels");
    let formats = arr(v, "formats");
    let format = {
        let parts: Vec<String> = formats
            .iter()
            .filter_map(|f| {
                let name = s(f, "name")?;
                let qty = s(f, "qty").and_then(|q| q.parse::<u32>().ok()).unwrap_or(1);
                Some(if qty > 1 { format!("{qty}×{name}") } else { name.to_owned() })
            })
            .collect();
        (!parts.is_empty()).then(|| parts.join(" + "))
    };
    let edition = formats.first().and_then(|f| {
        s(f, "text").map(str::to_owned).or_else(|| {
            let d: Vec<&str> = arr(f, "descriptions")
                .iter()
                .filter_map(Value::as_str)
                .filter(|d| !PLAIN_FORMATS.contains(&d.trim().to_lowercase().as_str()))
                .collect();
            (!d.is_empty()).then(|| d.join(", "))
        })
    });
    let studios = arr(v, "companies")
        .iter()
        .filter_map(|c| {
            let role = match s(c, "entity_type_name")? {
                "Recorded At" => "recorded",
                "Mixed At" => "mixed",
                "Mastered At" => "mastered",
                _ => return None,
            };
            Some((role.to_owned(), clean_discogs_artist(s(c, "name")?)))
        })
        .collect();
    let tracks = arr(v, "tracklist")
        .iter()
        .filter(|t| s(t, "type_").is_none_or(|k| k == "track"))
        .map(|t| DTrack {
            position: s(t, "position").unwrap_or_default().to_owned(),
            title: s(t, "title").unwrap_or_default().to_owned(),
            credits: credits(arr(t, "extraartists")),
        })
        .collect();
    Some(DiscogsRelease {
        id,
        released: s(v, "released").filter(|d| !d.starts_with("0")).map(|d| d.trim_end_matches("-00").to_owned()),
        country: s(v, "country").map(str::to_owned),
        label: labels.first().and_then(|l| s(l, "name")).map(clean_discogs_artist),
        catalogue: labels.first().and_then(|l| s(l, "catno")).filter(|c| !c.eq_ignore_ascii_case("none")).map(str::to_owned),
        barcode: arr(v, "identifiers")
            .iter()
            .find(|i| s(i, "type") == Some("Barcode"))
            .and_then(|i| s(i, "value"))
            .map(|b| b.chars().filter(|c| !c.is_whitespace()).collect()),
        format,
        edition,
        notes: s(v, "notes").and_then(plain_notes),
        studios,
        credits: credits(arr(v, "extraartists")),
        tracks,
    })
}

/// The release a database search found: the first result whose
/// "Artist - Title" is the album's (folded). None without a sure match.
pub fn pick_search(v: &Value, artist: &str, title: &str) -> Option<u64> {
    let (a, t) = (fold_artist(artist), fold_album(title));
    if t.is_empty() {
        return None;
    }
    arr(v, "results").iter().find_map(|r| {
        let id = r.get("id").and_then(Value::as_u64)?;
        let (ra, rt) = s(r, "title")?.split_once(" - ")?;
        (fold_album(rt) == t && (a.is_empty() || fold_artist(&clean_discogs_artist(ra)) == a)).then_some(id)
    })
}

/// A Discogs release as a source part, its tracks matched to the library's
/// songs by order when the counts agree, else by title.
pub fn to_part(r: &DiscogsRelease, songs: &[LibSong]) -> Part {
    use crate::share::matching::title_key;
    let conv = |cs: &[DCredit]| cs.iter().map(|c| RawCredit::new(&c.name, &c.role, c.detail.as_deref())).collect::<Vec<_>>();
    let mut p = Part {
        released: r.released.clone(),
        country: r.country.clone(),
        label: r.label.clone(),
        catalogue: r.catalogue.clone(),
        barcode: r.barcode.clone(),
        format: r.format.clone(),
        edition: r.edition.clone(),
        notes: r.notes.clone(),
        studios: r.studios.clone(),
        credits: conv(&r.credits),
        link: Some(format!("https://www.discogs.com/release/{}", r.id)),
        ..Default::default()
    };
    let by_order = r.tracks.len() == songs.len();
    for (i, t) in r.tracks.iter().enumerate() {
        let song = if by_order {
            songs.get(i)
        } else {
            let k = title_key(&t.title);
            songs.iter().find(|s| !k.is_empty() && title_key(&s.title) == k)
        };
        if let Some(song) = song
            && !t.credits.is_empty()
        {
            p.tracks.insert(song.id.clone(), TrackPart { credits: conv(&t.credits), recorded: None });
        }
    }
    p
}
