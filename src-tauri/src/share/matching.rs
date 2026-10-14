//! Matching a library song or album against store search results.
//!
//! A result counts only when every field matches closely: artist, title and
//! album for a song (plus the length, within 3 seconds), artist and title for
//! an album. Names are compared after folding case, accents and punctuation,
//! with featured artists and edition notes ("Remastered", "Deluxe Edition")
//! taken out. Anything less is no match: a wrong link is worse than none.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Largest length difference for a song match.
pub const DURATION_SLACK_MS: i64 = 3000;

/// Words that mark a bracket or dash part as an edition note, not part of
/// the name. Live, acoustic, remix and the like stay: those are different
/// recordings.
const EDITION_WORDS: &[&str] = &[
    "remaster",
    "deluxe",
    "edition",
    "expanded",
    "anniversary",
    "bonus",
    "mono",
    "stereo",
    "explicit",
    "clean",
    "album version",
    "single version",
    "radio edit",
];

const FEAT_WORDS: &[&str] = &["feat.", "feat ", "ft.", "ft ", "featuring ", "with "];

/// Lower case, no accents, compatibility forms folded.
fn fold(s: &str) -> String {
    s.nfkd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase).collect()
}

fn is_edition(part: &str) -> bool {
    let p = part.trim();
    EDITION_WORDS.iter().any(|w| p.contains(w)) || p == "single" || p == "ep"
}

fn is_feat(part: &str) -> bool {
    let p = part.trim_start();
    FEAT_WORDS.iter().any(|w| p.starts_with(w))
}

/// Drop bracketed parts that are edition notes or featured artists.
fn strip_brackets(s: &str, keep_editions: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find(['(', '[']) {
        let close_ch = if rest[open..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[open + 1..].find(close_ch) else { break };
        let inner = &rest[open + 1..open + 1 + len];
        out.push_str(&rest[..open]);
        if !(is_feat(inner) || (!keep_editions && is_edition(inner))) {
            out.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    out.push_str(rest);
    out
}

/// Drop " - Remastered 2011", " - Single", " - feat. X" style suffixes.
fn strip_dash_parts(s: &str, keep_editions: bool) -> String {
    let mut parts = s.split(" - ");
    let mut out = parts.next().unwrap_or_default().to_string();
    for p in parts {
        if is_feat(p) || (!keep_editions && is_edition(p)) {
            continue;
        }
        out.push_str(" - ");
        out.push_str(p);
    }
    out
}

/// Cut a trailing "feat. X" that isn't in brackets.
fn strip_trailing_feat(s: &str) -> String {
    for w in [" feat. ", " feat ", " ft. ", " featuring "] {
        if let Some(i) = s.find(w) {
            return s[..i].to_string();
        }
    }
    s.to_string()
}

/// Letters and digits only, single spaces, "&" read as "and".
fn squash(s: &str) -> String {
    let s = s.replace(['&', '+'], " and ");
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// A song or album title for comparing: featured artists and edition notes
/// removed.
pub fn title_key(s: &str) -> String {
    let f = fold(s);
    squash(&strip_trailing_feat(&strip_dash_parts(&strip_brackets(&f, false), false)))
}

/// A title with edition notes kept (only featured artists removed): two
/// titles equal here are the very same release.
pub fn exact_key(s: &str) -> String {
    let f = fold(s);
    squash(&strip_trailing_feat(&strip_dash_parts(&strip_brackets(&f, true), true)))
}

/// The separate names in an artist credit ("A & B", "A, B feat. C").
pub fn artist_names(s: &str) -> Vec<String> {
    let f = fold(s);
    let f = strip_brackets(&f, true);
    let mut names = Vec::new();
    let mut cur = f.as_str().to_string();
    for sep in [" feat. ", " feat ", " ft. ", " featuring ", " with ", " x ", " vs. ", " & ", " and ", ", ", " / ", "; "] {
        cur = cur.replace(sep, "\u{1}");
    }
    for part in cur.split('\u{1}') {
        let n = squash(part);
        if !n.is_empty() && !names.contains(&n) {
            names.push(n);
        }
    }
    names
}

/// Two artist credits match when they name the same artists, or one names
/// a subset of the other and both lead with the same artist.
pub fn artists_match(a: &str, b: &str) -> bool {
    if squash(&fold(a)) == squash(&fold(b)) {
        return true;
    }
    let (x, y) = (artist_names(a), artist_names(b));
    if x.is_empty() || y.is_empty() {
        return false;
    }
    let subset = |p: &[String], q: &[String]| p.iter().all(|n| q.contains(n));
    x[0] == y[0] && (subset(&x, &y) || subset(&y, &x))
}

/// A library song to find.
#[derive(Debug, Clone, PartialEq)]
pub struct SongQuery {
    /// The song's own artist credit (track artist, else album artist).
    pub artist: String,
    /// The album artist, also accepted as the artist.
    pub album_artist: String,
    pub title: String,
    pub album: String,
    pub duration_ms: i64,
}

/// A library album to find.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumQuery {
    pub artist: String,
    pub title: String,
    pub track_count: i64,
}

/// One song result from the store.
#[derive(Debug, Clone, PartialEq)]
pub struct SongHit {
    pub id: u64,
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration_ms: Option<i64>,
}

/// One album result from the store.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumHit {
    pub id: u64,
    pub artist: String,
    pub title: String,
    pub track_count: Option<i64>,
}

/// Whether a song result matches on every field.
pub fn song_matches(q: &SongQuery, h: &SongHit) -> bool {
    let artist_ok = artists_match(&q.artist, &h.artist) || artists_match(&q.album_artist, &h.artist);
    let title_ok = !title_key(&q.title).is_empty() && title_key(&q.title) == title_key(&h.title);
    let album_ok = album_titles_match(&q.album, &h.album);
    let length_ok = match (q.duration_ms, h.duration_ms) {
        (0, _) => false,
        (q, Some(h)) => (q - h).abs() <= DURATION_SLACK_MS,
        (_, None) => false,
    };
    artist_ok && title_ok && album_ok && length_ok
}

/// Album names match after edition notes are removed (a store single named
/// "Song - Single" matches a library single named "Song").
fn album_titles_match(lib: &str, store: &str) -> bool {
    let (a, b) = (title_key(lib), title_key(store));
    !a.is_empty() && a == b
}

/// Whether an album result matches on artist and title.
pub fn album_matches(q: &AlbumQuery, h: &AlbumHit) -> bool {
    artists_match(&q.artist, &h.artist) && !title_key(&q.title).is_empty() && title_key(&q.title) == title_key(&h.title)
}

/// The best matching song: the same edition first, then the closest length.
pub fn best_song(q: &SongQuery, hits: &[SongHit]) -> Option<u64> {
    let album = exact_key(&q.album);
    hits.iter()
        .filter(|h| song_matches(q, h))
        .min_by_key(|h| {
            let same_edition = exact_key(&h.album) == album;
            let gap = (q.duration_ms - h.duration_ms.unwrap_or(0)).abs();
            (!same_edition, gap)
        })
        .map(|h| h.id)
}

/// The best matching album: the same edition first, then the same number of
/// songs.
pub fn best_album(q: &AlbumQuery, hits: &[AlbumHit]) -> Option<u64> {
    let title = exact_key(&q.title);
    hits.iter()
        .filter(|h| album_matches(q, h))
        .min_by_key(|h| {
            let same_edition = exact_key(&h.title) == title;
            let count_gap = h.track_count.map_or(i64::MAX, |n| (n - q.track_count).abs());
            (!same_edition, count_gap)
        })
        .map(|h| h.id)
}

/// The search words sent for a song or album: artist and title without
/// brackets or featured artists (the store matches words, not phrases).
pub fn search_term(artist: &str, title: &str) -> String {
    let lead = artist_names(artist).into_iter().next().unwrap_or_else(|| squash(&fold(artist)));
    format!("{} {}", lead, title_key(title)).trim().to_string()
}
