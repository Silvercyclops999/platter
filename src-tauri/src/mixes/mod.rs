//! Made-for-you mixes, radio and the daylist, all worked out on this
//! computer from the library cache and the listen log (no server part).
//!
//! - [`library`] the snapshot the engine reads (tracks, tags, plays)
//! - [`engine`]  Daily Mixes, Discover, On Repeat, Rediscover, New in your
//!   library, genre and mood mixes, the daylist
//! - [`radio`]   song and artist radio, and "keep playing similar music"
//! - [`store`]   the cache of generated mixes (SQLite)
//! - [`service`] when mixes are made (daily after 4 am, the daylist every
//!   few hours, on demand), always on the blocking pool
//! - [`commands`] Tauri commands (docs/IPC.md "Mixes and radio")

pub mod commands;
pub mod engine;
pub mod library;
pub mod radio;
pub mod service;
pub mod store;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use crate::model::Track;

pub use service::MixService;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MixKind {
    Daily,
    Discover,
    NewInLibrary,
    OnRepeat,
    Rediscover,
    Genre,
    Mood,
    Daylist,
    /// All-Time Top Songs (rows show play counts).
    TopSongs,
    /// Song or artist radio (a page made when it opens; not stored).
    Radio,
}

impl MixKind {
    pub fn key(self) -> &'static str {
        match self {
            MixKind::Daily => "daily",
            MixKind::Discover => "discover",
            MixKind::NewInLibrary => "newInLibrary",
            MixKind::OnRepeat => "onRepeat",
            MixKind::Rediscover => "rediscover",
            MixKind::Genre => "genre",
            MixKind::Mood => "mood",
            MixKind::Daylist => "daylist",
            MixKind::TopSongs => "topSongs",
            MixKind::Radio => "radio",
        }
    }

    pub fn from_key(k: &str) -> Option<MixKind> {
        Some(match k {
            "daily" => MixKind::Daily,
            "discover" => MixKind::Discover,
            "newInLibrary" => MixKind::NewInLibrary,
            "onRepeat" => MixKind::OnRepeat,
            "rediscover" => MixKind::Rediscover,
            "genre" => MixKind::Genre,
            "mood" => MixKind::Mood,
            "daylist" => MixKind::Daylist,
            "topSongs" => MixKind::TopSongs,
            "radio" => MixKind::Radio,
            _ => return None,
        })
    }
}

/// How much of a mix is favourites (the Variety setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Variety {
    #[default]
    Balanced,
    Familiar,
    Adventurous,
}

/// A steering chip on a mix page: the same songs to pick from, weighted
/// towards favourites or towards songs you play less.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MixSteer {
    Familiar,
    New,
}

/// What the mix settings decide.
#[derive(Debug, Clone, PartialEq)]
pub struct MixOptions {
    pub variety: Variety,
    pub exclude_artists: Vec<String>,
    pub exclude_genres: Vec<String>,
    pub exclude_albums: Vec<String>,
    pub include_unreleased: bool,
    pub daylist_hours: u32,
    /// Explicit filter and fan edits, as everywhere else.
    pub tag_view: crate::tags::TagView,
    pub liked_min: f64,
}

impl Default for MixOptions {
    fn default() -> Self {
        Self {
            variety: Variety::Balanced,
            exclude_artists: Vec::new(),
            exclude_genres: Vec::new(),
            exclude_albums: Vec::new(),
            include_unreleased: true,
            daylist_hours: 3,
            tag_view: crate::tags::TagView::default(),
            liked_min: crate::db::likes::LIKED_RATING,
        }
    }
}

impl MixOptions {
    /// Changes when anything that shapes the mixes changes (stored with them).
    pub fn fingerprint(&self) -> String {
        format!(
            "{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{}",
            self.variety,
            self.exclude_artists,
            self.exclude_genres,
            self.exclude_albums,
            self.include_unreleased,
            self.daylist_hours,
            self.tag_view.bits(),
            self.liked_min
        )
    }
}

/// A mix as cards and page headers show it (`MixSummary` in docs/IPC.md).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MixSummary {
    pub id: String,
    pub kind: MixKind,
    pub title: String,
    /// "A, B, C and more", or what the mix is.
    pub description: String,
    /// The card's name strip, `#rrggbb`.
    pub color: String,
    /// A photo of the mix's leading artist (Plex image path), if any.
    pub art: Option<String>,
    /// Album covers for a collage when there's no artist photo. On a radio
    /// page: the seed's picture first, then two related artists' photos.
    pub covers: Vec<String>,
    pub artist_ids: Vec<String>,
    pub tag: Option<String>,
    pub track_count: u32,
    pub duration_ms: u64,
    /// When it's next made again (the daylist), unix ms.
    pub next_update_at: Option<i64>,
}

/// Home's mix shelves.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MixShelves {
    /// Daily Mixes, daylist, Discover, New in your library, On Repeat,
    /// Rediscover.
    pub made_for_you: Vec<MixSummary>,
    pub moods: Vec<MixSummary>,
    pub genres: Vec<MixSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MixPage {
    pub mix: MixSummary,
    pub tracks: Vec<Track>,
    /// Per song: a favourite (true) or something less played.
    pub familiar: Vec<bool>,
    pub steer: Option<MixSteer>,
}
