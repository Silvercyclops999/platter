//! Deezer's public catalogue (no key): which album is ours, and which of
//! its songs are explicit.

use serde_json::Value;

use crate::identity::fold::{fold_album, fold_artist, fold_text, fold_title};

pub const API: &str = "https://api.deezer.com/";

/// One album a search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DzAlbum {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub tracks: u32,
    pub explicit: bool,
}

/// One song on a Deezer album. `explicit` is None when Deezer doesn't know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DzTrack {
    pub title: String,
    pub disc: u32,
    pub position: u32,
    pub duration_s: u64,
    pub explicit: Option<bool>,
}

/// One of our songs on the album being matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTrack {
    pub id: String,
    pub title: String,
    pub disc: u32,
    pub number: u32,
    pub duration_ms: u64,
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

fn num(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

/// Deezer reports errors (quota, bad query) inside a 200.
pub fn is_error(v: &Value) -> bool {
    v.get("error").is_some()
}

/// The albums in a `search/album` answer.
pub fn albums(v: &Value) -> Vec<DzAlbum> {
    let Some(list) = v.get("data").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|a| {
            let id = num(a, "id");
            let title = text(a, "title");
            (id > 0 && !title.is_empty()).then(|| DzAlbum {
                id,
                title,
                artist: a.get("artist").map(|x| text(x, "name")).unwrap_or_default(),
                tracks: num(a, "nb_tracks") as u32,
                explicit: a.get("explicit_lyrics").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// A song's answer: explicit when Deezer says so; clean only when its
/// advisory says not explicit (0) or edited (3). Unknown and "no advice"
/// stay unknown.
fn track_explicit(t: &Value) -> Option<bool> {
    if t.get("explicit_lyrics").and_then(Value::as_bool) == Some(true) {
        return Some(true);
    }
    match t.get("explicit_content_lyrics").and_then(Value::as_u64) {
        Some(0 | 3) => Some(false),
        Some(1) => Some(true),
        Some(_) => None,
        None => t.get("explicit_lyrics").and_then(Value::as_bool),
    }
}

/// The songs in an `album/{id}/tracks` answer.
pub fn tracks(v: &Value) -> Vec<DzTrack> {
    let Some(list) = v.get("data").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .map(|t| {
            let short = text(t, "title_short");
            DzTrack {
                title: if short.is_empty() { text(t, "title") } else { short },
                disc: num(t, "disk_number").max(1) as u32,
                position: num(t, "track_position") as u32,
                duration_s: num(t, "duration"),
                explicit: track_explicit(t),
            }
        })
        .collect()
}

/// Whether our album's title says it's the clean or edited one.
pub fn wants_clean(title: &str) -> bool {
    fold_text(title).split(|c: char| !c.is_alphanumeric()).any(|w| matches!(w, "clean" | "edited"))
}

fn same_artist(a: &str, b: &str) -> bool {
    let (a, b) = (fold_artist(a), fold_artist(b));
    // Both "Various Artists" (folded to nothing): the title decides.
    if a.is_empty() || b.is_empty() {
        return a == b;
    }
    a == b || a.contains(&b) || b.contains(&a)
}

/// The search result that is our album: same artist and title (version
/// notes such as "(Deluxe)" or "(Explicit)" set aside), then the full
/// title, then the nearest song count, then the explicit edition unless
/// ours says clean. None when nothing is surely ours.
pub fn pick<'a>(title: &str, artist: &str, count: usize, found: &'a [DzAlbum]) -> Option<&'a DzAlbum> {
    let base = fold_album(title);
    if base.is_empty() {
        return None;
    }
    let full = fold_title(title);
    let clean = wants_clean(title);
    found
        .iter()
        .filter(|a| same_artist(&a.artist, artist) && fold_album(&a.title) == base)
        .min_by_key(|a| {
            let f = fold_title(&a.title);
            let version_differs = f.version != full.version && !f.version.contains("explicit") && !f.version.contains("clean");
            (version_differs, (a.tracks as i64 - count as i64).unsigned_abs(), a.explicit == clean, a.id)
        })
}

/// Each of our songs Deezer has an answer for: by title, the same disc and
/// place first when a title is there twice; else, when the two albums have
/// as many songs, by disc and place when the lengths agree.
pub fn match_tracks(local: &[LocalTrack], dz: &[DzTrack]) -> Vec<(String, bool)> {
    let key = |t: &str| fold_title(t).title;
    let same_count = local.len() == dz.len();
    let mut out = Vec::new();
    for l in local {
        let k = key(&l.title);
        let by_title = dz
            .iter()
            .filter(|d| !k.is_empty() && key(&d.title) == k)
            .min_by_key(|d| (d.disc != l.disc.max(1) || d.position != l.number, d.position));
        let by_place = || {
            same_count
                .then(|| dz.iter().find(|d| d.disc == l.disc.max(1) && d.position == l.number))
                .flatten()
                .filter(|d| (d.duration_s as i64 - (l.duration_ms / 1000) as i64).abs() <= 10)
        };
        if let Some(e) = by_title.or_else(by_place).and_then(|d| d.explicit) {
            out.push((l.id.clone(), e));
        }
    }
    out
}

/// Strips what would break a quoted search term.
fn quoted(s: &str) -> String {
    s.replace(['"', '\\'], " ").trim().to_string()
}

fn encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// The album search, artist and title as fields.
pub fn search_url(base: &str, artist: &str, title: &str) -> String {
    let q = format!("artist:\"{}\" album:\"{}\"", quoted(artist), quoted(title));
    format!("{base}search/album?q={}&limit=25", encode(&q))
}

/// The looser search, for titles the field search misses.
pub fn loose_url(base: &str, artist: &str, title: &str) -> String {
    format!("{base}search/album?q={}&limit=25", encode(&format!("{} {}", quoted(artist), quoted(title))))
}

pub fn tracks_url(base: &str, id: u64) -> String {
    format!("{base}album/{id}/tracks?limit=300")
}
