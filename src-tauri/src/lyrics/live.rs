//! Whether a track is a live recording. Lyrics found by artist and title
//! (Plex's licensed lyrics, LRCLIB) are usually timed to the studio version,
//! so on a live version their timing is shown as borrowed.
//!
//! A track counts as live when its album has the live release type (Plex's
//! album format or subformat), its ARCHIVE_VERSION is `live`, or the track
//! or album title says so: "(Live)", "[Live ...]", "- Live", "Unplugged",
//! and for albums "Live at", "Live from" or "Live in".

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;

/// What live detection reads about a track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveFacts {
    pub title: String,
    pub album_title: String,
    pub format: Option<String>,
    pub subformats: Vec<String>,
    /// ARCHIVE_VERSION from the file, else from the album's files.
    pub version: Option<String>,
}

/// The release type says live (format or any subformat, any case).
pub fn release_is_live<S: AsRef<str>>(format: Option<&str>, subformats: &[S]) -> bool {
    let live = |s: &str| s.trim().to_ascii_lowercase().split(['/', ',', ';']).any(|p| p.trim() == "live");
    format.is_some_and(live) || subformats.iter().any(|s| live(s.as_ref()))
}

/// The marker appears in `text` as its own word (not "(Lively").
fn has_marker(text: &str, marker: &str) -> bool {
    let mut from = 0;
    while let Some(i) = text[from..].find(marker) {
        let end = from + i + marker.len();
        if !text[end..].chars().next().is_some_and(char::is_alphanumeric) {
            return true;
        }
        from = end;
    }
    false
}

/// A track title that names a live take: "(Live)", "[Live at ...]",
/// "- Live", "Unplugged".
pub fn track_title_says_live(title: &str) -> bool {
    let t = title.to_lowercase();
    ["(live", "[live", "unplugged"].iter().any(|m| has_marker(&t, m)) || ["- live", "\u{2013} live"].iter().any(|m| dash_live(&t, m))
}

/// "- Live" ends the title or leads into where or when ("- Live at the
/// Hall", "- Live 1986"), so "Song - Live and Let Die" doesn't count.
fn dash_live(t: &str, marker: &str) -> bool {
    t.match_indices(marker).any(|(i, _)| {
        let rest = t[i + marker.len()..].trim_start();
        rest.is_empty()
            || rest.starts_with(|c: char| c.is_ascii_digit() || matches!(c, ')' | ']' | '(' | '[' | '/' | ',' | '-' | '\u{2013}'))
            || ["at ", "from ", "in ", "version", "recording", "session", "take"].iter().any(|w| rest.starts_with(w))
    })
}

/// An album title that names a live recording: what a track title would,
/// or "Live at/from/in ...".
pub fn album_title_says_live(title: &str) -> bool {
    if track_title_says_live(title) {
        return true;
    }
    // "Live in Paris", "Tour: Live at the Hall", but not "Songs We Live In Between".
    let t = format!(" {}", title.to_lowercase());
    [" live at ", " live from ", " live in "].iter().any(|m| {
        t.match_indices(m).any(|(i, _)| {
            let before = t[..i].trim_end();
            before.is_empty() || before.ends_with(['(', '[', '-', ':', '\u{2013}', ','])
        })
    })
}

/// The one live rule.
pub fn is_live(f: &LiveFacts) -> bool {
    release_is_live(f.format.as_deref(), &f.subformats)
        || f.version.as_deref().is_some_and(|v| v.trim().eq_ignore_ascii_case("live"))
        || track_title_says_live(&f.title)
        || album_title_says_live(&f.album_title)
}

/// What the cache knows about a track for [`is_live`]; None for a track
/// that isn't in it.
pub fn facts(c: &Connection, track_id: &str) -> AppResult<Option<LiveFacts>> {
    Ok(c
        .query_row(
            "SELECT t.title, t.album_title, al.format, al.subformats,
                    coalesce((SELECT ft.version FROM file_tags ft WHERE ft.part_key = t.part_key),
                             (SELECT at.version FROM album_tags at WHERE at.album_id = t.album_id))
             FROM tracks t LEFT JOIN albums al ON al.id = t.album_id
             WHERE t.id = ?1",
            params![track_id],
            |r| {
                let subs: Option<String> = r.get(3)?;
                Ok(LiveFacts {
                    title: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    album_title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    format: r.get(2)?,
                    subformats: serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default(),
                    version: r.get(4)?,
                })
            },
        )
        .optional()?)
}

/// Whether the track is a live version (false when it isn't known).
pub fn track_is_live(c: &Connection, track_id: &str) -> AppResult<bool> {
    Ok(facts(c, track_id)?.is_some_and(|f| is_live(&f)))
}
