//! Lyrics the user wrote in the app: the `user_lyrics` table, one row per
//! track, holding LRC text (synced) or plain lines, and whether the user
//! wants them shown ahead of Plex's for that song.
//!
//! Lyrics transcribed on this computer (lyrics::transcribe) are stored here
//! too, with `auto` naming the model, so the editor opens and fixes them like
//! any others. Saving in the editor makes them the user's own (auto cleared).

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
    /// Transcribed on this computer by this model ("small" or "medium"), not
    /// yet edited. None for lyrics the user wrote or saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<String>,
}

impl OwnLyrics {
    /// As lyrics for the views (source `auto` while not yet edited).
    pub fn lyrics(&self) -> Option<Lyrics> {
        if self.auto.is_some() {
            let mut l = to_lyrics(&crate::lyrics::transcribe::lines::strip_music_symbols(&self.text))?;
            l.source = LyricsSource::Auto;
            return Some(l);
        }
        to_lyrics(&self.text)
    }
}

/// Library track ids, with their server tag: a Plex rating key (a number),
/// a Jellyfin id (hex, maybe with dashes) or a Subsonic id. Letters, digits,
/// "-" and "_" only, so an id is always safe in a file name.
pub fn valid_track_id(id: &str) -> bool {
    let k = crate::sid::key(id);
    !k.is_empty() && k.len() <= 64 && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
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
        "SELECT track_id, body, prefer, updated_at, auto FROM user_lyrics WHERE track_id = ?1",
        params![track_id],
        |r| {
            Ok(OwnLyrics {
                track_id: r.get(0)?,
                text: r.get(1)?,
                prefer: r.get::<_, i64>(2)? != 0,
                updated_at: r.get(3)?,
                auto: r.get(4)?,
            })
        },
    )
    .optional()?)
}

/// Save the user's lyrics. They become the user's own (no longer auto).
pub fn put(c: &Connection, track_id: &str, text: &str, prefer: bool, now: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO user_lyrics (track_id, body, prefer, updated_at, auto) VALUES (?1, ?2, ?3, ?4, NULL)
         ON CONFLICT (track_id) DO UPDATE SET body = excluded.body, prefer = excluded.prefer,
           updated_at = excluded.updated_at, auto = NULL",
        params![track_id, text, prefer as i64, now],
    )?;
    noted(c, track_id)
}

/// Save a transcription. Never replaces lyrics the user wrote or edited;
/// returns false when there were some.
pub fn put_auto(c: &Connection, track_id: &str, text: &str, model: &str, now: i64) -> AppResult<bool> {
    put_auto_timed(c, track_id, text, model, now, crate::lyrics::transcribe::engine::TIMING_VERSION)
}

/// [`put_auto`] for a transcription made with the given timing version
/// (one from another device may be older).
pub fn put_auto_timed(c: &Connection, track_id: &str, text: &str, model: &str, now: i64, timing: i64) -> AppResult<bool> {
    let n = c.execute(
        "INSERT INTO user_lyrics (track_id, body, prefer, updated_at, auto) VALUES (?1, ?2, 0, ?3, ?4)
         ON CONFLICT (track_id) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at,
           auto = excluded.auto WHERE user_lyrics.auto IS NOT NULL",
        params![track_id, text, now, model],
    )?;
    if n > 0 {
        set_timing(c, track_id, timing)?;
        noted(c, track_id)?;
    }
    Ok(n > 0)
}

/// Keep lyric search (crate::lyricsearch) in step with what is stored.
fn noted(c: &Connection, track_id: &str) -> AppResult<()> {
    match get(c, track_id)?.and_then(|own| own.lyrics().map(|l| (l, own.auto.is_some()))) {
        Some((l, auto)) => crate::lyricsearch::note_or_log(c, track_id, &l, auto),
        None => crate::lyricsearch::forget_or_log(c, track_id),
    }
    Ok(())
}

fn set_timing(c: &Connection, track_id: &str, timing: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO lyrics_auto_timing (track_id, timing) VALUES (?1, ?2)
         ON CONFLICT (track_id) DO UPDATE SET timing = excluded.timing",
        params![track_id, timing],
    )?;
    Ok(())
}

/// Transcriptions made with an older timing than `timing`: (track, text), up to `limit`.
pub fn auto_timed_before(c: &Connection, timing: i64, limit: usize) -> AppResult<Vec<(String, String)>> {
    let mut st = c.prepare(
        "SELECT u.track_id, u.body FROM user_lyrics u LEFT JOIN lyrics_auto_timing t ON t.track_id = u.track_id
         WHERE u.auto IS NOT NULL AND COALESCE(t.timing, 1) < ?1 ORDER BY u.track_id LIMIT ?2",
    )?;
    let rows = st.query_map(params![timing, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Put a transcription's re-timed text in place (`None`: nothing to move),
/// unless it was edited or replaced meanwhile. True when it was.
pub fn retimed(c: &Connection, track_id: &str, old: &str, text: Option<&str>, timing: i64) -> AppResult<bool> {
    let n = c.execute(
        "UPDATE user_lyrics SET body = COALESCE(?3, body) WHERE track_id = ?1 AND body = ?2 AND auto IS NOT NULL",
        params![track_id, old, text],
    )?;
    if n > 0 {
        set_timing(c, track_id, timing)?;
        if text.is_some() {
            noted(c, track_id)?;
        }
    }
    Ok(n > 0)
}

/// The timing version a song's transcription was made with.
pub fn auto_timing(c: &Connection, track_id: &str) -> AppResult<Option<i64>> {
    Ok(c
        .query_row(
            "SELECT COALESCE(t.timing, 1) FROM user_lyrics u LEFT JOIN lyrics_auto_timing t ON t.track_id = u.track_id
             WHERE u.track_id = ?1 AND u.auto IS NOT NULL",
            params![track_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// True when there was a row to remove.
pub fn delete(c: &Connection, track_id: &str) -> AppResult<bool> {
    let gone = c.execute("DELETE FROM user_lyrics WHERE track_id = ?1", params![track_id])? > 0;
    if gone {
        crate::lyricsearch::forget_or_log(c, track_id);
    }
    Ok(gone)
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
