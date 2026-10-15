//! Matching imported songs to the library through `identity::`.
//!
//! Each song becomes a [`SongKey`] from its names (and ISRC and length when
//! the file has them), and the song index ranks the candidates: ISRC first,
//! then folded names with lengths within the slack, then names alone. The
//! result is one of:
//!
//! - matched: a clear best candidate
//! - unsure: something close but not clearly the same recording (another
//!   version, or several songs on other albums with no length to decide);
//!   these wait in the review queue
//! - missing: nothing in the library
//!
//! Unreleased songs in the library never match (that rule is in identity).

use std::collections::HashMap;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::SongIn;
use crate::db::merge::Rank;
use crate::error::AppResult;
use crate::identity::index::{Candidate, lookup};
use crate::identity::{Context, MatchKind, SongKey, Tier, fold};

/// Most candidates kept for the review queue.
pub const MAX_CANDIDATES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Matched,
    Unsure,
    Missing,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Matched => "matched",
            Status::Unsure => "unsure",
            Status::Missing => "missing",
        }
    }

    pub fn parse(s: &str) -> Status {
        match s {
            "matched" => Status::Matched,
            "unsure" => Status::Unsure,
            _ => Status::Missing,
        }
    }
}

/// The rule that matched, as stored and shown.
pub fn tier_name(t: Tier) -> &'static str {
    match t {
        Tier::Isrc => "isrc",
        Tier::RecordingMbid => "recordingId",
        Tier::PlexTrackMbid => "trackId",
        Tier::Text => "names",
        Tier::TextOtherVersion => "otherVersion",
    }
}

/// How one song came out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub status: Status,
    /// The library song, when matched.
    pub track_id: Option<String>,
    pub tier: Option<Tier>,
    pub confidence: u8,
    /// The best few library songs, for the review queue.
    pub candidates: Vec<String>,
}

impl Outcome {
    fn missing() -> Self {
        Outcome { status: Status::Missing, track_id: None, tier: None, confidence: 0, candidates: Vec::new() }
    }
}

/// The key an imported song is looked up by.
pub fn key_of(s: &SongIn) -> SongKey {
    let mut k = SongKey::from_names(&s.artist, &s.title, s.album.as_deref().unwrap_or(""), s.duration_ms.unwrap_or(0));
    k.isrc = s.isrc.as_deref().and_then(crate::tags::clean_isrc);
    k
}

/// One stable text key per song within a source: folded artist, title,
/// version notes and album.
pub fn song_key(s: &SongIn) -> String {
    let k = key_of(s);
    format!("{}\u{1f}{}\u{1f}{}\u{1f}{}", k.artist_fold, k.title_fold, k.version_fold, k.album_fold)
}

/// Decides from the ranked candidates (best first, as `identity::index::lookup`
/// gives them).
pub fn decide(cands: &[Candidate], album_known: bool) -> Outcome {
    let Some(best) = cands.first() else { return Outcome::missing() };
    let ids: Vec<String> = cands.iter().take(MAX_CANDIDATES).map(|c| c.track_id.clone()).collect();
    let m = best.matched;
    let matched = |confidence| Outcome {
        status: Status::Matched,
        track_id: Some(best.track_id.clone()),
        tier: Some(m.tier),
        confidence,
        candidates: ids.clone(),
    };
    let unsure = || Outcome { status: Status::Unsure, track_id: None, tier: Some(m.tier), confidence: m.confidence, candidates: ids.clone() };
    if m.kind == MatchKind::OtherVersion {
        return unsure();
    }
    match m.tier {
        Tier::Isrc | Tier::RecordingMbid | Tier::PlexTrackMbid => matched(m.confidence),
        // Lengths agreed.
        Tier::Text if m.confidence >= 85 => matched(m.confidence),
        Tier::Text => {
            // Names only. Fine on the same album, or when there's only one
            // song it could be; several on other albums need a person.
            let same_recordings: Vec<&Candidate> = cands.iter().filter(|c| c.matched.kind == MatchKind::SameRecording).collect();
            let albums_differ = album_known && !best.same_album;
            if best.same_album || !album_known {
                matched(m.confidence)
            } else if same_recordings.len() == 1 && albums_differ {
                matched(m.confidence.saturating_sub(5))
            } else {
                unsure()
            }
        }
        Tier::TextOtherVersion => unsure(),
    }
}

/// Matches songs against the song index, once per distinct song.
pub struct Matcher<'a> {
    c: &'a Connection,
    ranks: HashMap<String, Rank>,
    seen: HashMap<String, Outcome>,
}

impl<'a> Matcher<'a> {
    pub fn new(c: &'a Connection, ranks: HashMap<String, Rank>) -> Self {
        Self { c, ranks, seen: HashMap::new() }
    }

    pub fn find(&mut self, s: &SongIn) -> AppResult<Outcome> {
        let sk = format!("{}\u{1f}{}", song_key(s), s.isrc.as_deref().unwrap_or(""));
        if let Some(o) = self.seen.get(&sk) {
            return Ok(o.clone());
        }
        let key = key_of(s);
        let cands = lookup(self.c, &key, Context::Single, &self.ranks)?;
        let album_known = !key.album_fold.is_empty();
        let o = decide(&cands, album_known);
        self.seen.insert(sk, o.clone());
        Ok(o)
    }
}

/// Whether the library has an album by this artist with this title.
pub fn album_found(c: &Connection, artist: &str, album: &str) -> AppResult<bool> {
    let a = fold::fold_artist(artist);
    let t = fold::fold_album(album);
    if t.is_empty() {
        return Ok(false);
    }
    let mut st = c.prepare_cached("SELECT 1 FROM song_index WHERE artist_fold = ?1 AND album_fold = ?2 AND unreleased = 0 LIMIT 1")?;
    Ok(st.exists(rusqlite::params![a, t])?)
}
