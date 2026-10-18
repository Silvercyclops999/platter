//! The listening diary: a private journal of albums played start to finish.
//!
//! An entry is written on its own when the listen log shows an album heard
//! through in one sitting ([`detect`]), or by hand with "Log this album".
//! Each keeps the date, how much was played, where (this computer's name
//! and the output device), and the user's rating and note.
//!
//! Entries live in the local sync store's `diary` namespace. They reach the
//! sync file only while "Sync the diary to my devices" is on (the engine's
//! `diary` group); otherwise they never leave this computer. Nothing here
//! is shared, scrobbled or sent to a server. A deleted entry leaves a
//! tombstone, so the same listening never comes back on its own.

pub mod detect;
pub mod export;
mod service;

pub use service::*;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Longest note, in characters.
pub const NOTE_MAX: usize = 500;

/// One diary entry as the sync store keeps it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiaryEntry {
    pub album_id: String,
    /// The album as it was when logged, so the entry reads after the album
    /// leaves the library.
    pub title: String,
    pub artist: String,
    pub year: Option<i32>,
    /// When the album was finished (or logged), unix ms.
    pub at: i64,
    /// When the listening started (written automatically only), unix ms.
    pub started_at: Option<i64>,
    /// Songs heard, and songs on the album then.
    pub songs_played: u32,
    pub songs_total: u32,
    /// Minutes listened.
    pub minutes: u32,
    /// The computer it was played on.
    pub device: Option<String>,
    /// The output device's name, when known.
    pub output: Option<String>,
    /// 0.5 to 5 in half steps; None for no rating.
    pub rating: Option<f32>,
    /// Plain text, up to [`NOTE_MAX`] characters.
    pub note: String,
    /// Written on its own from the listen log (false: logged by hand).
    pub auto: bool,
}

/// What kind of output an entry was heard on, from the device's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListenedOn {
    Headphones,
    Speakers,
}

impl ListenedOn {
    pub fn label(self) -> &'static str {
        match self {
            ListenedOn::Headphones => "headphones",
            ListenedOn::Speakers => "speakers",
        }
    }
}

/// Headphones or speakers from an output device's name, when it says.
pub fn listened_on(output: &str) -> Option<ListenedOn> {
    let n = output.to_lowercase();
    const PHONES: [&str; 12] =
        ["headphone", "headset", "earphone", "earbud", "airpods", "buds", "in-ear", "iem", "wh-1000", "hd 6", "hd6", "dt 770"];
    const SPEAKERS: [&str; 6] = ["speaker", "monitor", "soundbar", "homepod", "sonos", "receiver"];
    if PHONES.iter().any(|w| n.contains(w)) {
        Some(ListenedOn::Headphones)
    } else if SPEAKERS.iter().any(|w| n.contains(w)) {
        Some(ListenedOn::Speakers)
    } else {
        None
    }
}

/// The user's part of an entry, replaced as a whole by an edit.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiaryEdit {
    pub rating: Option<f32>,
    pub note: String,
    /// Move the entry to another time (unix ms); None keeps it.
    pub at: Option<i64>,
}

/// A rating in half steps from 0.5 to 5.
pub fn check_rating(r: Option<f32>) -> AppResult<Option<f32>> {
    match r {
        None => Ok(None),
        Some(r) if r.is_finite() && (0.5..=5.0).contains(&r) && (r * 2.0).fract() == 0.0 => Ok(Some(r)),
        Some(_) => Err(AppError::Invalid("A rating is from half a star to five stars.".into())),
    }
}

/// A note as plain text: control characters go (line breaks stay), runs of
/// blank lines become one, and it must fit [`NOTE_MAX`].
pub fn clean_note(note: &str) -> AppResult<String> {
    let text: String = note.replace("\r\n", "\n").chars().filter(|c| *c == '\n' || !c.is_control()).collect();
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.trim().lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    if out.chars().count() > NOTE_MAX {
        return Err(AppError::Invalid(format!("A note can be up to {NOTE_MAX} characters.")));
    }
    Ok(out)
}
