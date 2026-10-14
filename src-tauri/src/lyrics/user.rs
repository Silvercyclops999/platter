//! Lyrics the user wrote in the app: the `user_lyrics` table, one row per
//! track, holding LRC text (synced) or plain lines, and whether the user
//! wants them shown ahead of Plex's for that song.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::plex::details::{Lyrics, LyricsSource, parse_lyrics_text};

/// Far more than any song needs; keeps a stray paste from bloating the db.
pub const MAX_BYTES: usize = 64 * 1024;

/// A track's own lyrics as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnLyrics {
    pub track_id: String,
    /// LRC text, or plain lines when they aren't synced.
    pub text: String,
    /// Show these ahead of Plex's lyrics for this song.
    pub prefer: bool,
    /// Unix seconds.
    pub updated_at: i64,
}

/// Library track ids are Plex rating keys (numbers).
pub fn valid_track_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// The text to store: line endings made plain, outer blank lines trimmed.
/// Errors when it has no words or is too long.
pub fn clean(text: &str) -> AppResult<String> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim().to_owned();
    if text.len() > MAX_BYTES {
        return Err(AppError::Invalid("Those lyrics are too long to save.".into()));
    }
    if to_lyrics(&text).is_none() {
        return Err(AppError::Invalid("There are no lyrics to save.".into()));
    }
    Ok(text)
}

/// Stored text as lyrics for the views. None when it has no words.
pub fn to_lyrics(text: &str) -> Option<Lyrics> {
    let mut l = parse_lyrics_text(text);
    if l.lines.iter().all(|s| s.is_empty()) {
        return None;
    }
    l.source = LyricsSource::User;
    Some(l)
}

pub fn get(c: &Connection, track_id: &str) -> AppResult<Option<OwnLyrics>> {
    Ok(c.query_row(
        "SELECT track_id, body, prefer, updated_at FROM user_lyrics WHERE track_id = ?1",
        params![track_id],
        |r| Ok(OwnLyrics { track_id: r.get(0)?, text: r.get(1)?, prefer: r.get::<_, i64>(2)? != 0, updated_at: r.get(3)? }),
    )
    .optional()?)
}

pub fn put(c: &Connection, track_id: &str, text: &str, prefer: bool, now: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO user_lyrics (track_id, body, prefer, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (track_id) DO UPDATE SET body = excluded.body, prefer = excluded.prefer,
           updated_at = excluded.updated_at",
        params![track_id, text, prefer as i64, now],
    )?;
    Ok(())
}

/// True when there was a row to remove.
pub fn delete(c: &Connection, track_id: &str) -> AppResult<bool> {
    Ok(c.execute("DELETE FROM user_lyrics WHERE track_id = ?1", params![track_id])? > 0)
}

/// Every track that has its own lyrics (for "Add" or "Edit lyrics" in menus).
pub fn ids(c: &Connection) -> AppResult<Vec<String>> {
    let mut st = c.prepare("SELECT track_id FROM user_lyrics ORDER BY track_id")?;
    let rows = st.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A file name for exported lyrics: the song's name with characters no file
/// system takes replaced, ending in .lrc.
pub fn export_file_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '_' } else { c })
        .collect();
    s = s.trim().trim_end_matches('.').trim().to_owned();
    if s.chars().count() > 120 {
        s = s.chars().take(120).collect();
    }
    if s.is_empty() {
        s = "Lyrics".into();
    }
    if !s.to_ascii_lowercase().ends_with(".lrc") {
        s.push_str(".lrc");
    }
    s
}
