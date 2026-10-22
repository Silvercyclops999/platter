//! Library import: playlists, liked songs, saved albums and listening
//! history from files other services let you download, matched to the
//! library.
//!
//! - [`files`]    reads what the user picked: one file, a folder, or a zip
//! - [`sources`]  the parsers, one per kind of file, behind [`sources::Parser`]
//! - [`matching`] every imported song through `identity::` (ISRC, then names
//!   and length, then names alone) into matched, unsure or missing
//! - [`store`]    the import tables, the review queue and the Missing list
//! - [`played`]   history plays with no library match ("Played a lot")
//! - [`bring`]    bringing music over: one door, three steps
//! - [`commands`] Tauri commands
//!
//! Nothing here writes to Plex, except bringing music over (`bring`). Saving an imported playlist to Plex or liking
//! its songs there goes through the usual playlist and like commands, and
//! only when the user asks.
//!
//! Privacy: parsers copy out the song, the time and how long it played, and
//! nothing else. Network addresses, countries, devices and the like in
//! history files are never read into anything that is kept. Plays marked
//! private are dropped.

pub mod account;
pub mod bring;
pub mod commands;
pub mod files;
pub mod matching;
pub mod played;
pub mod sources;
pub mod store;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod bring_tests;
#[cfg(test)]
mod dryrun_tests;
#[cfg(test)]
mod played_tests;

use serde::Serialize;

/// The name Platter suggests for a playlist made of the user's liked
/// songs (select all in the liked list, drag onto a new playlist). A
/// playlist with this name comes in as liked songs, not as a playlist.
pub const LIKED_LIST_NAME: &str = "Liked Songs for Platter";

/// Whether a playlist is the liked songs under [`LIKED_LIST_NAME`]
/// (letters and digits compared, case ignored).
pub fn is_liked_list(name: &str) -> bool {
    let squash = |s: &str| -> String { s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect() };
    squash(name) == squash(LIKED_LIST_NAME)
}

/// A song as an import file names it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SongIn {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    /// Length, when the file has it.
    pub duration_ms: Option<u64>,
    pub isrc: Option<String>,
    /// The service's own id for the song (kept for re-imports, never matched on).
    pub service_id: Option<String>,
}

/// A playlist from an import file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistIn {
    pub name: String,
    pub songs: Vec<SongIn>,
}

/// A saved album.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumIn {
    pub artist: String,
    pub album: String,
}

/// One play from a history file: the song, when and for how long. Nothing
/// else from the record is kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayIn {
    /// When the play ended, unix ms (history files log the end).
    pub ended_at: i64,
    pub ms_played: u64,
    pub song: SongIn,
}

/// What was left out, and why.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Skipped {
    /// Plays marked private (incognito).
    pub private_plays: u64,
    /// Podcast, episode and audiobook records.
    pub podcasts: u64,
    /// Records with no song name (some history files have them).
    pub no_metadata: u64,
    /// Plays under 30 seconds (skips).
    pub short_plays: u64,
    /// Rows with no title or no artist.
    pub incomplete_rows: u64,
    /// Files that couldn't be read as anything this source knows.
    pub unknown_files: u64,
    /// Playlists other people own, which a connected account can't read
    /// the songs of (the download has them).
    pub others_playlists: u64,
}

/// Everything a parser found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub playlists: Vec<PlaylistIn>,
    /// Liked songs; None when the files had no liked list (an earlier
    /// import's likes then stay as they were).
    pub liked: Option<Vec<SongIn>>,
    /// Saved albums; None when the files had none.
    pub albums: Option<Vec<AlbumIn>>,
    pub plays: Vec<PlayIn>,
    pub skipped: Skipped,
}

impl Parsed {
    /// Whether nothing at all came out.
    pub fn is_empty(&self) -> bool {
        self.playlists.is_empty() && self.liked.is_none() && self.albums.is_none() && self.plays.is_empty()
    }

    /// Adds another file's results.
    pub fn absorb(&mut self, other: Parsed) {
        self.playlists.extend(other.playlists);
        if let Some(l) = other.liked {
            self.liked.get_or_insert_with(Vec::new).extend(l);
        }
        if let Some(a) = other.albums {
            self.albums.get_or_insert_with(Vec::new).extend(a);
        }
        self.plays.extend(other.plays);
        let s = &mut self.skipped;
        let o = other.skipped;
        s.private_plays += o.private_plays;
        s.podcasts += o.podcasts;
        s.no_metadata += o.no_metadata;
        s.short_plays += o.short_plays;
        s.incomplete_rows += o.incomplete_rows;
        s.unknown_files += o.unknown_files;
        s.others_playlists += o.others_playlists;
    }
}
