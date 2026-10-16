//! Which lyrics to show: every candidate found for a song, in order, and
//! the one rule that picks among them (pure, so it can be tested alone).
//!
//! Order: the user's own lyrics when they prefer them, the server's, the
//! user's own, LRCLIB's exact lookup, LRCLIB's title search hits (closest
//! length first), then a transcription made on this computer.
//!
//! With the audio check, the first candidate that matches the recording
//! wins; ones it is sure don't match are skipped. Without it (or when it
//! can't tell), the first usable candidate is shown as not checked, with
//! stricter rules: LRCLIB timing only when the length is close, and a song
//! that is another version (unreleased, alternate, live, version tag)
//! doesn't take lyrics found for the released version unless the user says
//! so.
//!
//! A transcription made on this computer only fills a song nothing found
//! can be shown for (none found, every one surely wrong, or held back as
//! another version's). It never outranks found lyrics unless the user picks
//! it ("Transcribe this song" picks the transcription it asks for).

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::check::{self, Point, Verdict};
use crate::error::AppResult;
use crate::plex::details::{Lyrics, LyricsSource};
use crate::share::matching::title_key;

/// LRCLIB timing is used when the length is this close...
pub const SYNC_CLOSE_S: f64 = 2.0;
/// ...or this close with the same album.
pub const SYNC_ALBUM_S: f64 = 5.0;

/// Where a candidate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    /// Written or saved by the user.
    User,
    /// The song's server (its own lyrics or a file next to the song).
    Server,
    /// LRCLIB's lookup by artist, title, album and length.
    LrclibExact,
    /// LRCLIB's search by artist and title.
    LrclibSearch,
    /// Transcribed on this computer.
    Auto,
}

/// One set of lyrics that might belong to the song.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// "user", "server", "auto", or "lrclib:<hash>".
    pub key: String,
    pub origin: Origin,
    pub lyrics: Lyrics,
    /// [`check::lyrics_hash`] of the lyrics as found.
    pub hash: String,
    pub album: Option<String>,
    pub duration_ms: Option<i64>,
    /// Found for the released version while this song is another version.
    pub other_version: bool,
    /// Its timing can be shown without the audio check.
    pub sync_ok: bool,
}

impl Candidate {
    pub fn new(key: impl Into<String>, origin: Origin, lyrics: Lyrics) -> Self {
        let hash = check::lyrics_hash(&lyrics);
        Self { key: key.into(), origin, lyrics, hash, album: None, duration_ms: None, other_version: false, sync_ok: true }
    }

    /// The user's own lyrics: shown without the check.
    pub fn trusted(&self) -> bool {
        self.origin == Origin::User
    }

    /// Judged by the audio check (not the user's own, not a transcription
    /// of this very recording).
    pub fn checkable(&self) -> bool {
        !matches!(self.origin, Origin::User | Origin::Auto)
    }

    /// Shown while its check runs (not one found only by its title).
    fn safe_while_checking(&self) -> bool {
        matches!(self.origin, Origin::Server | Origin::LrclibExact)
    }

    /// Found by artist and title (Plex's licensed lyrics, LRCLIB): the
    /// timing is whichever version they were written against.
    pub fn borrows_timing(&self) -> bool {
        matches!(self.lyrics.source, LyricsSource::Plex | LyricsSource::Lrclib)
    }
}

/// A key for an LRCLIB candidate.
pub fn lrclib_key(hash: &str) -> String {
    format!("lrclib:{hash}")
}

/// The stricter rule for LRCLIB timing without the audio check.
pub fn lrclib_sync_ok(track_ms: i64, track_album: &str, rec_duration_s: Option<f64>, rec_album: Option<&str>) -> bool {
    let Some(d) = rec_duration_s else { return false };
    let diff = (d - track_ms as f64 / 1000.0).abs();
    diff <= SYNC_CLOSE_S || (diff <= SYNC_ALBUM_S && album_matches(track_album, rec_album.unwrap_or("")))
}

/// Same album once folded, with edition notes ("Deluxe", "Remastered") taken out.
pub fn album_matches(a: &str, b: &str) -> bool {
    let (a, b) = (title_key(a), title_key(b));
    !a.is_empty() && a == b
}

/// A stored check result.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub verdict: Verdict,
    pub score: f32,
    /// The measured offsets: one (steady) or two (drift).
    pub points: Vec<Point>,
}

/// Why nothing (or nothing yet) is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Notice {
    /// Lyrics found for the released version; "Use anyway".
    VersionHeld,
    /// The audio check is running and nothing found can be shown before it.
    Checking,
    /// Every candidate disagreed with the recording.
    NoneMatched,
    /// The user chose no lyrics for this song.
    ChosenNone,
}

/// The note under shown lyrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Note {
    /// The audio check says they match.
    Checked,
    /// Not checked against the recording.
    NotChecked,
    /// Being checked now.
    Checking,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Show { idx: usize, note: Option<Note> },
    Held { idx: usize },
    Notice(Notice),
    Nothing,
}

/// The first candidate the user's own or the audio vouches for (never a
/// transcription: it only fills in).
pub fn winner<'a>(cands: &'a [Candidate], verdict: impl Fn(&Candidate) -> Option<&'a Stored>) -> Option<usize> {
    cands.iter().position(|c| c.origin != Origin::Auto && (c.trusted() || verdict(c).is_some_and(|v| v.verdict.matched())))
}

/// Pick what to show. `verdict` gives a candidate's stored check;
/// `checking`: the audio check is on and will judge candidates it hasn't;
/// `released_ok`: "Use released lyrics for other versions".
pub fn decide<'a>(
    cands: &'a [Candidate],
    verdict: impl Fn(&Candidate) -> Option<&'a Stored>,
    checking: bool,
    released_ok: bool,
) -> Decision {
    let winner = winner(cands, &verdict);
    // A transcription: shown only when nothing found can be.
    let fill = cands.iter().position(|c| c.origin == Origin::Auto);
    let before = winner.unwrap_or(cands.len());
    let (mut soft, mut pending, mut held, mut no) = (None, None, None, false);
    for (i, c) in cands[..before].iter().enumerate() {
        if c.origin == Origin::Auto {
            continue;
        }
        let held_back = c.other_version && !released_ok;
        match verdict(c).map(|v| v.verdict) {
            Some(Verdict::NoMatch) => no = true,
            None if checking => {
                pending.get_or_insert(i);
            }
            _ if held_back => {
                held.get_or_insert(i);
            }
            _ => {
                soft.get_or_insert(i);
            }
        }
    }
    if let Some(p) = pending {
        if let Some(s) = soft {
            return Decision::Show { idx: s, note: Some(Note::Checking) };
        }
        let c = &cands[p];
        if c.safe_while_checking() && !(c.other_version && !released_ok) {
            return Decision::Show { idx: p, note: Some(Note::Checking) };
        }
        if let Some(w) = winner {
            return Decision::Show { idx: w, note: note_for(&cands[w], &verdict) };
        }
        if let Some(a) = fill {
            return Decision::Show { idx: a, note: None };
        }
        return Decision::Notice(Notice::Checking);
    }
    if let Some(w) = winner {
        if !cands[w].trusted() {
            return Decision::Show { idx: w, note: Some(Note::Checked) };
        }
        if let Some(s) = soft {
            return Decision::Show { idx: s, note: Some(Note::NotChecked) };
        }
        return Decision::Show { idx: w, note: None };
    }
    if let Some(s) = soft {
        return Decision::Show { idx: s, note: Some(Note::NotChecked) };
    }
    if let Some(a) = fill {
        return Decision::Show { idx: a, note: None };
    }
    if let Some(h) = held {
        return Decision::Held { idx: h };
    }
    if no {
        return Decision::Notice(Notice::NoneMatched);
    }
    Decision::Nothing
}

fn note_for<'a>(c: &'a Candidate, verdict: &impl Fn(&Candidate) -> Option<&'a Stored>) -> Option<Note> {
    if c.trusted() {
        None
    } else if verdict(c).is_some_and(|v| v.verdict.matched()) {
        Some(Note::Checked)
    } else {
        Some(Note::NotChecked)
    }
}

/// Shown lyrics disagree with another usable candidate (different songs
/// under one title): "can't tell", so "Wrong lyrics?" is offered plainly.
pub fn disagrees<'a>(cands: &'a [Candidate], idx: usize, verdict: impl Fn(&Candidate) -> Option<&'a Stored>) -> bool {
    let shown = &cands[idx];
    cands.iter().enumerate().any(|(i, c)| {
        i != idx
            && c.checkable()
            && verdict(c).is_none_or(|v| v.verdict != Verdict::NoMatch)
            && check::disagree(&shown.lyrics, &c.lyrics)
    })
}

/// Drop the timing of lyrics (kept: the words, in order).
pub fn unsync(l: &mut Lyrics) {
    if l.synced && l.lines.is_empty() {
        l.lines = l.timed.iter().map(|t| t.text.clone()).collect();
    }
    l.synced = false;
    l.timed.clear();
}

// ---------------------------------------------------------------- versions

/// Title words that name another version of a song.
const VERSION_MARKERS: &[&str] =
    &["demo", "alternate", "alt version", "alt.", "acoustic", "remix", "rough", "early version", "snippet", "session", "reference"];

/// A track title that names another version: "(Demo)", "[Alternate Take]", "- Remix".
pub fn title_says_version(title: &str) -> bool {
    let t = title.to_lowercase();
    let mut parts: Vec<&str> = Vec::new();
    let mut open = None;
    for (i, ch) in t.char_indices() {
        match ch {
            '(' | '[' => open = Some(i + 1),
            ')' | ']' => {
                if let Some(s) = open.take() {
                    parts.push(&t[s..i]);
                }
            }
            _ => {}
        }
    }
    if let Some(s) = open {
        parts.push(&t[s..]);
    }
    parts.extend(t.split(" - ").skip(1));
    parts.iter().any(|p| VERSION_MARKERS.iter().any(|m| p.contains(m)))
}

/// What the candidate rules read about the song.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SongFacts {
    pub album: String,
    pub duration_ms: i64,
    pub live: bool,
    /// Unreleased, an alternate version, live, or with a version tag.
    pub other_version: bool,
}

/// The song's facts from the cache (None when it isn't in it).
pub fn song_facts(c: &Connection, track_id: &str) -> AppResult<Option<SongFacts>> {
    let row = c
        .query_row(
            "SELECT t.title, t.album_title, t.album_id, t.duration_ms, al.format, al.subformats, al.matched, al.studio,
                    coalesce((SELECT coalesce(ft.version, ft.release) FROM file_tags ft WHERE ft.part_key = t.part_key),
                             (SELECT coalesce(at.version, at.release) FROM album_tags at WHERE at.album_id = t.album_id)),
                    (SELECT mc.found FROM album_mb_check mc WHERE mc.album_id = t.album_id)
             FROM tracks t LEFT JOIN albums al ON al.id = t.album_id WHERE t.id = ?1",
            params![track_id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<bool>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<bool>>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((title, album, album_id, duration_ms, format, subs, matched, studio, version, mb)) = row else {
        return Ok(None);
    };
    let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
    let live = super::live::is_live(&super::live::LiveFacts {
        title: title.clone(),
        album_title: album.clone(),
        format: format.clone(),
        subformats: subs.clone(),
        version: version.clone(),
    });
    let tagged = version.as_deref().map(str::trim).is_some_and(|v| !v.is_empty() && !v.eq_ignore_ascii_case("released"));
    let unreleased = crate::db::release::track_unreleased(
        track_id,
        &album_id,
        format.as_deref(),
        &subs,
        matched,
        studio.as_deref(),
        version.as_deref(),
        mb,
    );
    Ok(Some(SongFacts {
        album,
        duration_ms,
        live,
        other_version: live || tagged || unreleased || title_says_version(&title),
    }))
}
