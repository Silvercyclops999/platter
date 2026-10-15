//! The user's lyrics choices per song (the `lyrics_choice` table): which
//! candidate to show ("Wrong lyrics?" or "Use anyway"), "none" for no
//! lyrics, and a timing offset from the nudge buttons or "Tap to sync".
//! They win over everything automatic and travel in the sync file (the
//! `lyricsChoices` namespace, see sync::lyrics).

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// The pick that means "no lyrics for this song".
pub const NONE: &str = "none";
/// Offsets further than this are refused (a slip of the finger).
pub const MAX_OFFSET_MS: i32 = 60_000;

/// One song's choice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    /// A candidate key, [`NONE`], or None for automatic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick: Option<String>,
    /// Move the lyrics this much later (negative: earlier).
    #[serde(default)]
    pub offset_ms: i32,
}

impl Choice {
    pub fn is_empty(&self) -> bool {
        self.pick.is_none() && self.offset_ms == 0
    }
}

/// A pick the table takes: a known candidate key shape or "none".
pub fn valid_pick(p: &str) -> bool {
    matches!(p, NONE | "user" | "server" | "auto")
        || p.strip_prefix("lrclib:").is_some_and(|h| !h.is_empty() && h.len() <= 32 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub fn check(c: &Choice) -> AppResult<()> {
    if c.pick.as_deref().is_some_and(|p| !valid_pick(p)) {
        return Err(AppError::Invalid("unknown lyrics choice".into()));
    }
    if c.offset_ms.abs() > MAX_OFFSET_MS {
        return Err(AppError::Invalid("That offset is too far.".into()));
    }
    Ok(())
}

pub fn get(c: &Connection, track_id: &str) -> AppResult<Option<Choice>> {
    Ok(c.query_row("SELECT pick, offset_ms FROM lyrics_choice WHERE track_id = ?1", params![track_id], |r| {
        Ok(Choice { pick: r.get(0)?, offset_ms: r.get(1)? })
    })
    .optional()?)
}

/// Save a song's choice; an empty one removes the row. Returns whether it changed.
pub fn put(c: &Connection, track_id: &str, choice: &Choice, now: i64) -> AppResult<bool> {
    if get(c, track_id)?.as_ref() == Some(choice) {
        return Ok(false);
    }
    if choice.is_empty() {
        return Ok(c.execute("DELETE FROM lyrics_choice WHERE track_id = ?1", params![track_id])? > 0);
    }
    c.execute(
        "INSERT INTO lyrics_choice (track_id, pick, offset_ms, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (track_id) DO UPDATE SET pick = excluded.pick, offset_ms = excluded.offset_ms,
           updated_at = excluded.updated_at",
        params![track_id, choice.pick, choice.offset_ms, now],
    )?;
    Ok(true)
}

/// Every song with a choice.
pub fn all(c: &Connection) -> AppResult<Vec<(String, Choice)>> {
    let mut st = c.prepare("SELECT track_id, pick, offset_ms FROM lyrics_choice ORDER BY track_id")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, Choice { pick: r.get(1)?, offset_ms: r.get(2)? })))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Forget every choice; returns the songs that had one.
pub fn clear(c: &Connection) -> AppResult<Vec<String>> {
    let ids: Vec<String> = all(c)?.into_iter().map(|(id, _)| id).collect();
    c.execute("DELETE FROM lyrics_choice", [])?;
    Ok(ids)
}
