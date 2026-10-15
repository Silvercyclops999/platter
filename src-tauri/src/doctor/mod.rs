//! The library doctor: one list of things worth fixing in the library, each
//! with a plain explanation, the evidence behind it and a fix through the
//! app's usual edit flows. Nothing here writes to Plex or to files; fixes
//! run from the UI with their own confirm.
//!
//! - [`checks`]   what each album artist's albums and songs say (duplicate
//!   files, album tags that differ, albums split in two, missing album
//!   artists, low-quality copies, albums MusicBrainz knows that Plex missed)
//! - [`covers`]   each album cover's size, read from the image's first bytes
//! - [`sounds`]   songs whose sound was identified as another recording
//!   than their tags say (`soundid::`), with the tags it suggests
//! - [`store`]    findings kept per artist, and the list the page shows
//! - [`commands`] the Tauri commands
//!
//! The check runs in the background: after a library sync, after file tags
//! are read, and when a mark or a setting changes, once things have been
//! quiet for a little while. It goes one album artist at a time and skips
//! artists whose albums haven't changed since their last check. Ignored
//! findings live in the sync store so they follow the user.

pub mod checks;
pub mod commands;
pub mod covers;
pub mod sounds;
pub mod store;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::db::DbHandle;
use crate::tags::fetch::Reader;
use crate::tags::scan::PartSource;

/// Fired when the findings or what's ignored change: `{ count }`.
pub const EV_CHANGED: &str = "doctor://changed";

/// The low-res cover sizes Settings offers (longest side, pixels).
pub const COVER_PX_CHOICES: [u32; 6] = [300, 500, 600, 800, 1000, 1400];
pub const DEFAULT_COVER_PX: u32 = 600;

/// What a finding is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// The same recording twice on one album.
    Duplicate,
    /// No cover, or one smaller than the setting.
    Cover,
    /// Album name, album artist or year differ between an album's files.
    AlbumTags,
    /// One album that Plex shows as two.
    SplitAlbum,
    /// Files with no album artist on an album whose songs credit several.
    AlbumArtist,
    /// A better copy of the same recording is on another album.
    LowQuality,
    /// Plex couldn't match it, but MusicBrainz has the release.
    Unmatched,
    /// An untagged or mislabelled file: its sound matched another recording.
    SoundsLike,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::Duplicate,
        Kind::Cover,
        Kind::AlbumTags,
        Kind::SplitAlbum,
        Kind::AlbumArtist,
        Kind::LowQuality,
        Kind::Unmatched,
        Kind::SoundsLike,
    ];

    /// Stable string stored in SQLite.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Duplicate => "duplicate",
            Kind::Cover => "cover",
            Kind::AlbumTags => "albumTags",
            Kind::SplitAlbum => "splitAlbum",
            Kind::AlbumArtist => "albumArtist",
            Kind::LowQuality => "lowQuality",
            Kind::Unmatched => "unmatched",
            Kind::SoundsLike => "soundsLike",
        }
    }

    pub fn from_key(k: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|x| x.key() == k)
    }
}

/// The one-click fix a finding offers. Each opens one of the app's own
/// flows, which asks before anything is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fix {
    /// Edit > Change cover on the album.
    Cover,
    /// Edit > Fix match on the album.
    Match,
    /// Merge the albums into one.
    MergeAlbums,
    /// Open the album (the copy to keep, or the songs to look at).
    OpenAlbum,
    /// Show each song's tags (they're changed in a tagger for now).
    Tags,
    /// Show the tags its sound suggests next to its own (nothing is written).
    SuggestedTags,
}

/// Another album a finding names: the other half of a split album, or the
/// album with the better copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherAlbum {
    pub id: String,
    pub title: String,
    pub thumb: Option<String>,
}

/// One song's own tags, for the album tag findings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagRow {
    pub track_id: String,
    pub title: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<i32>,
}

/// The tags a song's sound suggests, next to what it has (IPC
/// `DoctorSuggestedTags`). Shown only; nothing is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedTags {
    pub track_id: String,
    /// What the file says now.
    pub title: String,
    pub artist: String,
    pub album: String,
    /// What its sound matched on MusicBrainz.
    pub new_title: String,
    pub new_artist: String,
    pub new_album: Option<String>,
    pub recording_mbid: String,
    /// 0 to 100.
    pub score: u8,
}

/// One thing worth fixing (IPC `DoctorFinding`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Stable: the kind and the items it's about, so Ignore sticks.
    pub id: String,
    pub kind: Kind,
    pub artist_id: String,
    pub artist: String,
    pub album_id: String,
    pub album: String,
    pub thumb: Option<String>,
    /// A short headline.
    pub title: String,
    /// What's wrong and what the fix does, in a sentence or two.
    pub text: String,
    /// What it's based on, for "Why?".
    pub evidence: Vec<String>,
    pub fix: Fix,
    /// Songs it's about.
    #[serde(default)]
    pub tracks: Vec<String>,
    #[serde(default)]
    pub others: Vec<OtherAlbum>,
    /// Each song's tags (album tag and album artist findings).
    #[serde(default)]
    pub table: Vec<TagRow>,
    /// The tags its sound suggests (sounds-like findings).
    #[serde(default)]
    pub suggest: Option<SuggestedTags>,
}

/// A finding the user chose to ignore (sync store).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ignored {
    pub kind: Kind,
    /// What it said, for the Ignored list when it no longer shows up.
    #[serde(default)]
    pub title: String,
    /// When, unix seconds.
    #[serde(default)]
    pub at: i64,
}

/// Which checks run (Settings, each on by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Checks {
    pub duplicates: bool,
    pub covers: bool,
    pub album_tags: bool,
    pub split_albums: bool,
    pub album_artist: bool,
    pub low_quality: bool,
    pub unmatched: bool,
    /// Songs identified by sound as another recording than their tags say.
    pub sounds_like: bool,
}

impl Default for Checks {
    fn default() -> Self {
        Self {
            duplicates: true,
            covers: true,
            album_tags: true,
            split_albums: true,
            album_artist: true,
            low_quality: true,
            unmatched: true,
            sounds_like: true,
        }
    }
}

impl Checks {
    pub fn on(&self, k: Kind) -> bool {
        match k {
            Kind::Duplicate => self.duplicates,
            Kind::Cover => self.covers,
            Kind::AlbumTags => self.album_tags,
            Kind::SplitAlbum => self.split_albums,
            Kind::AlbumArtist => self.album_artist,
            Kind::LowQuality => self.low_quality,
            Kind::Unmatched => self.unmatched,
            Kind::SoundsLike => self.sounds_like,
        }
    }
}

/// What Settings says (pushed by `settings::publish`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Run library doctor.
    pub on: bool,
    pub checks: Checks,
    /// Covers smaller than this on their longest side are low-res.
    pub cover_px: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { on: true, checks: Checks::default(), cover_px: DEFAULT_COVER_PX }
    }
}

static OPTIONS: LazyLock<RwLock<Options>> = LazyLock::new(|| RwLock::new(Options::default()));
static WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);
static RUNNING: AtomicBool = AtomicBool::new(false);
static CHECKED_AT: AtomicI64 = AtomicI64::new(0);

pub fn options() -> Options {
    *OPTIONS.read()
}

/// Set from the settings; a change wakes the check.
pub fn set_options(o: Options) {
    let changed = {
        let mut w = OPTIONS.write();
        let changed = *w != o;
        *w = o;
        changed
    };
    if changed {
        poke();
    }
}

/// Ask for a check soon.
pub fn poke() {
    WAKE.notify_one();
}

/// A check is going now.
pub fn running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// When the last check finished (unix seconds), if one has.
pub fn checked_at() -> Option<i64> {
    Some(CHECKED_AT.load(Ordering::Relaxed)).filter(|t| *t > 0)
}

fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Startup work (the first sync, Home) goes first.
const START_DELAY: Duration = Duration::from_secs(120);
/// How often it looks for changes while nothing pokes it.
const LOOK_EVERY: Duration = Duration::from_secs(60);
/// The library has to be quiet this long before a check (a sync or the tag
/// scan still writing).
const SETTLE: Duration = Duration::from_secs(20);
/// Pause between artists, so the check never hogs the database.
const ARTIST_GAP: Duration = Duration::from_millis(15);
/// Covers sized per round, and the pause between them.
const COVERS_PER_ROUND: usize = 60;
const COVER_GAP: Duration = Duration::from_millis(500);

/// Start the background check for the life of the app.
pub fn spawn(app: tauri::AppHandle, db: DbHandle, parts: Arc<dyn PartSource>, reader: Reader) {
    tauri::async_runtime::spawn(follow_store(app.clone()));
    tauri::async_runtime::spawn(watch(app, db, parts, reader));
}

/// The sync store, once the app has set it up.
async fn sync_store(app: &tauri::AppHandle) -> crate::sync::SyncStore {
    use tauri::Manager;
    loop {
        if let Some(s) = app.try_state::<crate::sync::commands::SyncService>() {
            return s.store.clone();
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Marks change what's released (check again); ignores change the count.
async fn follow_store(app: tauri::AppHandle) {
    let store = sync_store(&app).await;
    let mut rx = store.subscribe();
    loop {
        let names = match rx.recv().await {
            Ok(c) => c.namespaces,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                vec![crate::sync::store::RELEASE_MARKS.name().to_string(), crate::sync::store::DOCTOR_IGNORED.name().to_string()]
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };
        if names.iter().any(|n| n == crate::sync::store::RELEASE_MARKS.name()) {
            poke();
        }
        if names.iter().any(|n| n == crate::sync::store::DOCTOR_IGNORED.name()) {
            commands::emit_changed(&app).await;
        }
    }
}

async fn watch(app: tauri::AppHandle, db: DbHandle, parts: Arc<dyn PartSource>, reader: Reader) {
    tokio::time::sleep(START_DELAY).await;
    let mut seen: Option<store::Stamp> = None;
    loop {
        let poked = tokio::time::timeout(LOOK_EVERY, WAKE.notified()).await.is_ok();
        let opts = options();
        if !opts.on {
            continue;
        }
        let Ok(stamp) = db.read_async(store::stamp).await else { continue };
        if !poked && seen.as_ref() == Some(&stamp) {
            continue;
        }
        // Wait for the library to settle: a sync or the tag scan may still be going.
        let mut stamp = stamp;
        loop {
            tokio::time::sleep(SETTLE).await;
            match db.read_async(store::stamp).await {
                Ok(s) if s == stamp => break,
                Ok(s) => stamp = s,
                Err(_) => break,
            }
        }
        seen = Some(stamp);
        RUNNING.store(true, Ordering::Relaxed);
        commands::emit_changed(&app).await;
        if let Err(e) = run_round(&db, &*parts, &reader).await {
            tracing::warn!("library doctor: {e}");
        }
        RUNNING.store(false, Ordering::Relaxed);
        CHECKED_AT.store(now_s(), Ordering::Relaxed);
        commands::emit_changed(&app).await;
    }
}

/// One round: every album artist whose albums changed, then a few covers.
pub async fn run_round(db: &DbHandle, parts: &dyn PartSource, reader: &Reader) -> crate::error::AppResult<usize> {
    let artists = db.read_async(store::artist_ids).await?;
    let mut changed = 0;
    for id in artists {
        if !options().on {
            return Ok(changed);
        }
        let a = id.clone();
        let fresh = db.read_async(move |c| checks::check_if_changed(c, &a)).await?;
        if let Some((sig, findings)) = fresh {
            db.write_async(move |c| store::replace_artist(c, &id, &sig, &findings)).await?;
            changed += 1;
            tokio::time::sleep(ARTIST_GAP).await;
        }
    }
    db.write_async(store::prune).await?;
    if options().checks.covers {
        covers::probe_round(db, parts, reader, COVERS_PER_ROUND, COVER_GAP).await?;
    }
    Ok(changed)
}
