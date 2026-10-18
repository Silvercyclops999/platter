//! MusicBrainz artist search, to find the MusicBrainz id of a credited
//! artist (a name in a track's artist line with no library artist, so no
//! Plex id). Only an exact match is taken: score 100 and the same name or
//! alias once case and spacing are folded. MusicBrainz asks for at most one
//! request a second and a user agent that names the app.

use serde_json::Value;
use url::Url;

use super::http::InfoHttp;
use super::limit::Limiter;
use super::{Fetch, FetchError, fold_name, is_mbid};

pub const API: &str = "https://musicbrainz.org/ws/2/artist/";

pub struct MusicBrainz<'a> {
    pub http: &'a dyn InfoHttp,
    pub limiter: &'a Limiter,
}

/// A Lucene phrase: quotes and backslashes escaped.
fn phrase(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push('"');
    for ch in name.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

pub fn search_url(name: &str) -> String {
    let p = phrase(name.trim());
    let mut u = Url::parse(API).expect("static url");
    u.query_pairs_mut()
        .append_pair("query", &format!("artist:{p} OR alias:{p}"))
        .append_pair("limit", "5")
        .append_pair("fmt", "json");
    u.into()
}

/// The id of the first exact match in a search reply.
pub fn pick(v: &Value, name: &str) -> Option<String> {
    let want = fold_name(name);
    v.get("artists")?.as_array()?.iter().find_map(|a| {
        let score = a.get("score").and_then(Value::as_u64)?;
        if score != 100 {
            return None;
        }
        let named = a.get("name").and_then(Value::as_str).is_some_and(|n| fold_name(n) == want);
        let aliased = a
            .get("aliases")
            .and_then(Value::as_array)
            .is_some_and(|al| al.iter().any(|x| x.get("name").and_then(Value::as_str).is_some_and(|n| fold_name(n) == want)));
        let id = a.get("id").and_then(Value::as_str)?;
        ((named || aliased) && is_mbid(id)).then(|| id.to_owned())
    })
}

pub const RELEASE_GROUP_API: &str = "https://musicbrainz.org/ws/2/release-group/";

/// A release group search by title and artist. The title is sent without
/// edition notes or featured artists, so "Record (Deluxe Edition)" finds
/// "Record".
pub fn release_group_url(artist: &str, title: &str) -> String {
    let t = phrase(&crate::share::matching::title_key(title));
    let a = phrase(artist.trim());
    let mut u = Url::parse(RELEASE_GROUP_API).expect("static url");
    u.query_pairs_mut()
        .append_pair("query", &format!("releasegroup:{t} AND artist:{a}"))
        .append_pair("limit", "10")
        .append_pair("fmt", "json");
    u.into()
}

/// A release group's artist credit as one line ("A feat. B").
fn credit_line(rg: &Value) -> String {
    rg.get("artist-credit")
        .and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .map(|c| {
                    let name = c.get("name").and_then(Value::as_str).unwrap_or_default();
                    let join = c.get("joinphrase").and_then(Value::as_str).unwrap_or_default();
                    format!("{name}{join}")
                })
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// The release group a search reply surely means: score 100, the same title
/// once case, accents, punctuation, featured artists and edition notes are
/// folded, and the same artists. Two different groups that both match
/// (an artist with two albums of one name) is no answer.
pub fn pick_release_group(v: &Value, artist: &str, title: &str) -> Option<String> {
    use crate::share::matching::{artist_names, title_key};
    let (want_title, want_artists) = (title_key(title), artist_names(artist));
    if want_title.is_empty() || want_artists.is_empty() {
        return None;
    }
    let mut found: Option<String> = None;
    for rg in v.get("release-groups")?.as_array()? {
        if rg.get("score").and_then(Value::as_u64) != Some(100) {
            continue;
        }
        let Some(id) = rg.get("id").and_then(Value::as_str).filter(|id| is_mbid(id)) else { continue };
        let same_title = rg.get("title").and_then(Value::as_str).is_some_and(|t| title_key(t) == want_title);
        if !same_title || artist_names(&credit_line(rg)) != want_artists {
            continue;
        }
        match &found {
            Some(f) if f != id => return None,
            _ => found = Some(id.to_owned()),
        }
    }
    found
}

pub const RELEASE_API: &str = "https://musicbrainz.org/ws/2/release/";
/// What a release lookup includes: the release group (original date), credit
/// lines, label, tracks, and artist relations on the release, its
/// recordings and their works (producer, writer and so on), studios (place
/// relations) and the release's annotation.
pub const RELEASE_INC: &str =
    "release-groups+artist-credits+labels+recordings+artist-rels+recording-level-rels+work-rels+work-level-rels+place-rels+annotation";

pub fn release_url(release_id: &str) -> String {
    format!("{RELEASE_API}{release_id}?inc={RELEASE_INC}&fmt=json")
}

/// A release group with its releases and their track counts.
pub fn release_group_releases_url(rg: &str) -> String {
    format!("{RELEASE_GROUP_API}{rg}?inc=releases+media&fmt=json")
}

/// How far an official release's track count may be from the library
/// album's and still be the same album (a bonus track or two).
pub const TRACK_SLACK: usize = 2;

/// An album MusicBrainz has: its release group and the release that fits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundRelease {
    pub release_group_id: String,
    pub release_id: String,
}

/// The official release in a release group (`inc=releases+media`) whose
/// track count is closest to `track_count`, within [`TRACK_SLACK`]; the
/// earliest one on a tie. None when no official release is that close.
pub fn pick_official_near(v: &Value, track_count: usize) -> Option<String> {
    let mut best: Option<(usize, String, String)> = None;
    for r in v.get("releases").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        let Some(id) = r.get("id").and_then(Value::as_str).filter(|id| is_mbid(id)) else { continue };
        let official = r.get("status").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("official"));
        if !official {
            continue;
        }
        let n: u64 = r
            .get("media")
            .and_then(Value::as_array)
            .map(|ms| ms.iter().filter_map(|m| m.get("track-count").and_then(Value::as_u64)).sum())
            .unwrap_or(0);
        let off = (n as usize).abs_diff(track_count);
        if n == 0 || off > TRACK_SLACK {
            continue;
        }
        let date = r.get("date").and_then(Value::as_str).filter(|d| !d.is_empty()).unwrap_or("9999").to_owned();
        let better = match &best {
            None => true,
            Some((o, d, _)) => (off, &date) < (*o, d),
        };
        if better {
            best = Some((off, date, id.to_owned()));
        }
    }
    best.map(|(_, _, id)| id)
}

/// An artist with its artist relations (members, subgroups and so on).
pub fn artist_url(mbid: &str) -> String {
    format!("{API}{mbid}?inc=artist-rels&fmt=json")
}

impl MusicBrainz<'_> {
    /// One lookup by id: `Ok(None)` when MusicBrainz doesn't know it.
    async fn lookup(&self, url: &str) -> Fetch<Value> {
        let resp = self.limiter.get(self.http, url, &[]).await?;
        match resp.status {
            200 => Ok(Some(resp.json().map_err(FetchError::from)?)),
            400 | 404 => Ok(None),
            s => Err(FetchError::Unavailable(format!("MusicBrainz answered {s}"))),
        }
    }

    /// An artist's members and relationships.
    pub async fn artist(&self, mbid: &str) -> Fetch<super::mbdetails::MbArtist> {
        if !is_mbid(mbid) {
            return Ok(None);
        }
        Ok(self.lookup(&artist_url(mbid)).await?.as_ref().and_then(super::mbdetails::parse_artist))
    }

    /// A release with its dates and credits.
    pub async fn release(&self, release_id: &str) -> Fetch<super::mbdetails::MbRelease> {
        if !is_mbid(release_id) {
            return Ok(None);
        }
        Ok(self.lookup(&release_url(release_id)).await?.as_ref().and_then(super::mbdetails::parse_release))
    }

    /// An album's release: the one Plex knows (one request), else a sure
    /// release group match by artist and title, then its best release
    /// (three requests).
    pub async fn album_release(
        &self,
        artist: &str,
        title: &str,
        track_count: usize,
        known: Option<&str>,
    ) -> Fetch<super::mbdetails::MbRelease> {
        if let Some(id) = known.filter(|k| is_mbid(k))
            && let Some(r) = self.release(id).await?
        {
            return Ok(Some(r));
        }
        let Some(rg) = self.release_group_id(artist, title).await? else { return Ok(None) };
        let Some(v) = self.lookup(&release_group_releases_url(&rg)).await? else { return Ok(None) };
        let Some(id) = super::mbdetails::pick_release(&v, track_count) else { return Ok(None) };
        self.release(&id).await
    }

    /// The release group id of an album, only for a sure match.
    pub async fn release_group_id(&self, artist: &str, title: &str) -> Fetch<String> {
        if artist.trim().is_empty() || title.trim().is_empty() {
            return Ok(None);
        }
        let resp = self.limiter.get(self.http, &release_group_url(artist, title), &[]).await?;
        if !resp.ok() {
            return Err(FetchError::Unavailable(format!("MusicBrainz answered {}", resp.status)));
        }
        let v: Value = resp.json().map_err(FetchError::from)?;
        Ok(pick_release_group(&v, artist, title))
    }

    /// Whether MusicBrainz has an album as an official release: a sure
    /// release group match (see [`pick_release_group`]) with an official
    /// release whose track count is within [`TRACK_SLACK`] of ours. Two
    /// requests at most. `Ok(None)`: no such release.
    pub async fn release_check(&self, artist: &str, title: &str, track_count: usize) -> Fetch<FoundRelease> {
        let Some(rg) = self.release_group_id(artist, title).await? else { return Ok(None) };
        let Some(v) = self.lookup(&release_group_releases_url(&rg)).await? else { return Ok(None) };
        Ok(pick_official_near(&v, track_count).map(|release_id| FoundRelease { release_group_id: rg, release_id }))
    }

    /// `Ok(None)` when there is no exact match.
    pub async fn artist_id(&self, name: &str) -> Fetch<String> {
        if name.trim().is_empty() {
            return Ok(None);
        }
        let resp = self.limiter.get(self.http, &search_url(name), &[]).await?;
        if !resp.ok() {
            return Err(FetchError::Unavailable(format!("MusicBrainz answered {}", resp.status)));
        }
        let v: Value = resp.json().map_err(FetchError::from)?;
        Ok(pick(&v, name))
    }
}

#[cfg(test)]
mod tests {
    use super::super::http::fake::FakeHttp;
    use super::*;
    use serde_json::json;

    const A: &str = "11111111-2222-3333-4444-555555555555";
    const B: &str = "66666666-2222-3333-4444-555555555555";

    #[test]
    fn exact_name_or_alias_with_full_score_only() {
        let v = json!({"artists": [
            {"id": B, "name": "Rue Ellis Band", "score": 100},
            {"id": A, "name": "Rue Ellis", "score": 97},
        ]});
        assert_eq!(pick(&v, "Rue Ellis"), None, "close is not enough");
        let v = json!({"artists": [{"id": A, "name": "RUE  ellis", "score": 100}]});
        assert_eq!(pick(&v, "Rue Ellis").as_deref(), Some(A));
        let v = json!({"artists": [{"id": A, "name": "Ruth Ellis", "score": 100, "aliases": [{"name": "Rue Ellis"}]}]});
        assert_eq!(pick(&v, "rue ellis").as_deref(), Some(A));
        let v = json!({"artists": [{"id": "not-an-id", "name": "Rue Ellis", "score": 100}]});
        assert_eq!(pick(&v, "Rue Ellis"), None);
        assert_eq!(pick(&json!({}), "Rue Ellis"), None);
    }

    #[test]
    fn release_groups_match_on_folded_title_and_the_same_artists() {
        let v = json!({"release-groups": [{
            "id": A, "score": 100, "title": "Night Drive",
            "artist-credit": [{"name": "Rue Ellis", "joinphrase": " feat. "}, {"name": "Guest"}]
        }]});
        assert_eq!(pick_release_group(&v, "Rue Ellis feat. Guest", "Night Drive (2019 Remaster)").as_deref(), Some(A));
        assert_eq!(pick_release_group(&v, "rue ellis, guest", "NIGHT DRIVE!").as_deref(), Some(A));
        assert_eq!(pick_release_group(&v, "Rue Ellis", "Night Drive"), None, "a missing artist is a different credit");
        assert_eq!(pick_release_group(&v, "Rue Ellis feat. Guest", "Night Drive Live"), None);
        assert_eq!(pick_release_group(&v, "", "Night Drive"), None);
        let u = Url::parse(&release_group_url("Rue Ellis", "Night Drive (Deluxe Edition)")).unwrap();
        let q = u.query_pairs().find(|(k, _)| k == "query").unwrap().1.into_owned();
        assert_eq!(q, r#"releasegroup:"night drive" AND artist:"Rue Ellis""#);
    }

    #[test]
    fn the_query_is_an_escaped_phrase() {
        let u = Url::parse(&search_url("Say \"Hi\" $")).unwrap();
        let q = u.query_pairs().find(|(k, _)| k == "query").unwrap().1.into_owned();
        assert_eq!(q, r#"artist:"Say \"Hi\" $" OR alias:"Say \"Hi\" $""#);
        assert!(u.as_str().contains("fmt=json"));
    }

    #[tokio::test]
    async fn asks_once_and_reads_the_answer() {
        let http = FakeHttp::new();
        http.route("musicbrainz.org/ws/2/artist", 200, &json!({"artists": [{"id": A, "name": "Guest", "score": 100}]}).to_string());
        let l = Limiter::per_second(100.0);
        let mb = MusicBrainz { http: &http, limiter: &l };
        assert_eq!(mb.artist_id("Guest").await, Ok(Some(A.to_string())));
        assert_eq!(mb.artist_id("  ").await, Ok(None));
        assert_eq!(http.calls("musicbrainz.org"), 1);
        http.route("musicbrainz.org/ws/2/artist", 400, "{}");
        assert!(matches!(mb.artist_id("Guest").await, Err(FetchError::Unavailable(_))));
    }
}
