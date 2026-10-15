//! MusicBrainz searches and lookups for music that isn't in the library:
//! URLs and the parsing of their answers. Requests go through
//! [`MbGet`](crate::lidarr::mbpick::MbGet) (the shared one-a-second limiter,
//! Platter's user agent and a cache).

use serde_json::Value;
use url::Url;

use super::{CatArtist, CatSong, CatTrack};
use crate::artistinfo::is_mbid;
use crate::lidarr::mbpick::{Group, VARIOUS_ARTISTS, WS, parse_group};
use crate::lidarr::request::cover_thumb;

/// Results asked for per kind.
pub const SEARCH_LIMIT: usize = 10;
/// Lowest MusicBrainz search score kept.
pub const MIN_SCORE: u64 = 70;

/// The query with Lucene's special characters escaped, so what the user
/// typed is searched as words.
pub fn escape(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 8);
    for ch in q.trim().chars() {
        if matches!(ch, '+' | '-' | '&' | '|' | '!' | '(' | ')' | '{' | '}' | '[' | ']' | '^' | '"' | '~' | '*' | '?' | ':' | '\\' | '/') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn search(kind: &str, query: &str) -> String {
    let mut u = Url::parse(&format!("{WS}/{kind}")).expect("static url");
    u.query_pairs_mut()
        .append_pair("query", query)
        .append_pair("limit", &SEARCH_LIMIT.to_string())
        .append_pair("fmt", "json");
    u.into()
}

pub fn artist_search_url(q: &str) -> String {
    search("artist", &escape(q))
}

pub fn group_search_url(q: &str) -> String {
    search("release-group", &escape(q))
}

pub fn recording_search_url(q: &str) -> String {
    search("recording", &escape(q))
}

/// An artist with its relationships (members, collaborations...).
pub fn artist_url(mbid: &str) -> String {
    crate::artistinfo::musicbrainz::artist_url(mbid)
}

/// A release with its tracks, credits and label.
pub fn release_url(id: &str) -> String {
    format!("{WS}/release/{id}?inc=recordings+artist-credits+labels&fmt=json")
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

fn score(v: &Value) -> u64 {
    v.get("score").and_then(Value::as_u64).unwrap_or(0)
}

/// A credit as one line ("A feat. B").
pub fn credit_line(v: &Value) -> String {
    v.get("artist-credit")
        .and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .map(|c| {
                    let name = c.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).or_else(|| c.pointer("/artist/name").and_then(Value::as_str));
                    format!("{}{}", name.unwrap_or_default(), s(c, "joinphrase"))
                })
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// The first credited artist's id (lower case).
pub fn first_artist_id(v: &Value) -> Option<String> {
    v.pointer("/artist-credit/0/artist/id").and_then(Value::as_str).map(str::to_ascii_lowercase).filter(|m| is_mbid(m))
}

fn artist_of(v: &Value) -> Option<CatArtist> {
    let mbid = s(v, "id").to_ascii_lowercase();
    let name = s(v, "name").trim();
    if !is_mbid(&mbid) || name.is_empty() || mbid == VARIOUS_ARTISTS {
        return None;
    }
    Some(CatArtist {
        mbid,
        name: name.to_string(),
        disambiguation: Some(s(v, "disambiguation").trim().to_string()).filter(|d| !d.is_empty()),
        kind: Some(s(v, "type").trim().to_string()).filter(|d| !d.is_empty()),
        photo: None,
        library_id: None,
    })
}

/// Artists in a search answer, best first, with a fair score.
pub fn parse_artists(v: &Value) -> Vec<CatArtist> {
    let mut out: Vec<CatArtist> = Vec::new();
    for a in v.get("artists").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        if score(a) < MIN_SCORE {
            continue;
        }
        if let Some(x) = artist_of(a)
            && !out.iter().any(|o| o.mbid == x.mbid)
        {
            out.push(x);
        }
    }
    out
}

/// What can show as a catalogue album: at least one official release, an
/// album, EP or single, not credited to Various Artists.
pub fn showable(g: &Group) -> bool {
    g.statuses.iter().any(|x| x == "official")
        && matches!(g.primary.as_deref(), Some("album" | "ep" | "single"))
        && !g.credit.iter().any(|c| c.id == VARIOUS_ARTISTS)
}

/// Release groups in a search answer with their credit line and first
/// credited artist, best first.
pub fn parse_groups(v: &Value) -> Vec<(Group, String, Option<String>)> {
    let mut out: Vec<(Group, String, Option<String>)> = Vec::new();
    for rg in v.get("release-groups").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        if score(rg) < MIN_SCORE {
            continue;
        }
        let Some(g) = parse_group(rg) else { continue };
        if !showable(&g) || out.iter().any(|(o, _, _)| o.id == g.id) {
            continue;
        }
        out.push((g, credit_line(rg), first_artist_id(rg)));
    }
    out
}

/// Songs in a search answer: each with the first official release it's on
/// (an album, EP or single), best first. Songs with no official release
/// are left out.
pub fn parse_recordings(v: &Value) -> Vec<CatSong> {
    let mut out: Vec<CatSong> = Vec::new();
    for rec in v.get("recordings").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        if score(rec) < MIN_SCORE {
            continue;
        }
        let id = s(rec, "id").to_ascii_lowercase();
        let title = s(rec, "title").trim();
        if !is_mbid(&id) || title.is_empty() {
            continue;
        }
        let official = rec.get("releases").and_then(Value::as_array).and_then(|rs| {
            rs.iter().find(|r| {
                s(r, "status").eq_ignore_ascii_case("official")
                    && matches!(
                        r.pointer("/release-group/primary-type").and_then(Value::as_str).map(str::to_ascii_lowercase).as_deref(),
                        Some("album" | "ep" | "single")
                    )
            })
        });
        let Some(rel) = official else { continue };
        let rgid = rel.pointer("/release-group/id").and_then(Value::as_str).map(str::to_ascii_lowercase).filter(|m| is_mbid(m));
        let artist = credit_line(rec);
        if artist.trim().is_empty() || first_artist_id(rec).as_deref() == Some(VARIOUS_ARTISTS) {
            continue;
        }
        let song = CatSong {
            recording_id: id,
            title: title.to_string(),
            artist: artist.trim().to_string(),
            artist_mbid: first_artist_id(rec),
            album: rel.pointer("/release-group/title").and_then(Value::as_str).map(str::to_string).or_else(|| Some(s(rel, "title").to_string())),
            thumb: rgid.as_deref().and_then(cover_thumb),
            release_group_id: rgid,
            duration_ms: rec.get("length").and_then(Value::as_u64),
        };
        // The same song on several releases comes back more than once.
        if !out.iter().any(|o| o.title.eq_ignore_ascii_case(&song.title) && o.artist.eq_ignore_ascii_case(&song.artist)) {
            out.push(song);
        }
    }
    out
}

/// What the artist page header says about an artist (`artist_url` answer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistHead {
    pub artist: Option<CatArtist>,
    pub area: Option<String>,
    pub life_span: Option<String>,
}

pub fn parse_head(v: &Value) -> ArtistHead {
    let details = crate::artistinfo::mbdetails::parse_artist(v);
    ArtistHead {
        artist: artist_of(v),
        area: v.pointer("/area/name").and_then(Value::as_str).map(str::to_string).filter(|a| !a.is_empty()),
        life_span: details.and_then(|d| d.life_span),
    }
}

/// The official release to show a release group's tracks from (its
/// `inc=releases` lookup): the earliest dated one.
pub fn pick_release(v: &Value) -> Option<String> {
    let mut best: Option<(String, String)> = None;
    for r in v.get("releases").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        let id = s(r, "id").to_ascii_lowercase();
        if !is_mbid(&id) || !s(r, "status").eq_ignore_ascii_case("official") {
            continue;
        }
        let date = Some(s(r, "date")).filter(|d| !d.is_empty()).unwrap_or("9999").to_string();
        if best.as_ref().is_none_or(|(d, _)| date < *d) {
            best = Some((date, id));
        }
    }
    best.map(|(_, id)| id)
}

/// A release's tracks (every medium), its label and date.
pub fn parse_release(v: &Value) -> (Vec<CatTrack>, Option<String>, Option<String>) {
    let album_artist = credit_line(v);
    let media = v.get("media").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let many = media.len() > 1;
    let mut tracks = Vec::new();
    for (i, m) in media.iter().enumerate() {
        let disc = m.get("position").and_then(Value::as_u64).unwrap_or(i as u64 + 1) as u32;
        for (j, t) in m.get("tracks").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
            let title = Some(s(t, "title")).filter(|x| !x.is_empty()).or_else(|| t.pointer("/recording/title").and_then(Value::as_str)).unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let artist = credit_line(t);
            tracks.push(CatTrack {
                disc: many.then_some(disc),
                number: t.get("position").and_then(Value::as_u64).unwrap_or(j as u64 + 1) as u32,
                title: title.to_string(),
                artist: Some(artist).filter(|a| !a.is_empty() && *a != album_artist),
                duration_ms: t.get("length").and_then(Value::as_u64).or_else(|| t.pointer("/recording/length").and_then(Value::as_u64)),
            });
        }
    }
    let label = v.pointer("/label-info/0/label/name").and_then(Value::as_str).map(str::to_string).filter(|l| !l.is_empty());
    let date = Some(s(v, "date").to_string()).filter(|d| !d.is_empty());
    (tracks, label, date)
}
