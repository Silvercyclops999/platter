//! The app's side of tapping a song the library doesn't have
//! (`lidarr::tap`): the song plays from its finished download as a song of
//! its own ("arriving"), then moves to the library's copy at the same spot.
//!
//! An arriving song has an id of its own (`arriving:` and the tap's id). The
//! player finds it through [`ArrivingLookup`] (wrapped around the library
//! lookup) and plays its file through [`ArrivingFiles`] (wrapped around the
//! downloads), so the engine needs nothing new. It isn't reported to the
//! server; the library's copy is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::audio::player::PlayerHandle;
use crate::audio::source::{ContextKind, LocalAudio, MoreRequest, PlayContext, ResolvedContext, TrackLookup};
use crate::db::DbHandle;
use crate::error::AppResult;
use crate::lidarr::tap::{LibraryTrack, Tap, TapHost};
use crate::model::Track;

pub const PREFIX: &str = "arriving:";
/// The tap event (`lidarr::tap::Tap`).
pub const EVENT_TAP: &str = "lidarr://tap";
/// Asks for the library to catch up at most this often while a tap waits.
const REFRESH_EVERY: Duration = Duration::from_secs(6);

/// Whether a track id is an arriving song's.
pub fn is_arriving(id: &str) -> bool {
    id.starts_with(PREFIX)
}

/// Arriving songs and their files.
#[derive(Clone, Default)]
pub struct Arriving(Arc<Mutex<HashMap<String, (Track, PathBuf)>>>);

impl Arriving {
    pub fn add(&self, tap: &Tap, file: &Path) -> Track {
        let t = track_for(tap, file);
        self.0.lock().insert(t.id.clone(), (t.clone(), file.to_path_buf()));
        t
    }

    pub fn get(&self, id: &str) -> Option<(Track, PathBuf)> {
        self.0.lock().get(id).cloned()
    }
}

/// The song as the player shows it while it plays from the download.
fn track_for(tap: &Tap, file: &Path) -> Track {
    let ext = file.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    Track {
        id: format!("{PREFIX}{}", tap.id),
        title: tap.title.clone(),
        album_id: String::new(),
        album_title: tap.album.clone(),
        artist_id: String::new(),
        artist_title: tap.artist.clone(),
        track_artist: None,
        track_number: 0,
        disc_number: 1,
        duration_ms: tap.duration_ms.unwrap_or(0),
        thumb: tap.thumb.clone(),
        part_key: String::new(),
        codec: ext.clone(),
        container: ext,
        bitrate: None,
        bit_depth: None,
        sample_rate: None,
        user_rating: None,
        view_count: 0,
        last_viewed_at: None,
        rating_count: None,
        rated_at: None,
        genres: Vec::new(),
        mbid: None,
        file: None,
        other_versions: None,
        song_status: None,
        file_release: None,
    }
}

/// The downloads, plus arriving songs' files.
pub struct ArrivingFiles {
    pub inner: Arc<dyn LocalAudio>,
    pub arriving: Arriving,
}

impl LocalAudio for ArrivingFiles {
    fn path(&self, track: &Track) -> Option<PathBuf> {
        if is_arriving(&track.id) {
            return self.arriving.get(&track.id).map(|(_, p)| p).filter(|p| p.is_file());
        }
        self.inner.path(track)
    }

    fn ids(&self) -> Vec<String> {
        self.inner.ids()
    }
}

/// The library lookup, plus arriving songs.
pub struct ArrivingLookup {
    pub inner: Arc<dyn TrackLookup>,
    pub arriving: Arriving,
}

#[async_trait]
impl TrackLookup for ArrivingLookup {
    async fn tracks(&self, ids: &[String]) -> AppResult<Vec<Track>> {
        if !ids.iter().any(|i| is_arriving(i)) {
            return self.inner.tracks(ids).await;
        }
        let rest: Vec<String> = ids.iter().filter(|i| !is_arriving(i)).cloned().collect();
        let found = if rest.is_empty() { Vec::new() } else { self.inner.tracks(&rest).await? };
        // In the order asked.
        Ok(ids
            .iter()
            .filter_map(|i| if is_arriving(i) { self.arriving.get(i).map(|(t, _)| t) } else { found.iter().find(|t| &t.id == i).cloned() })
            .collect())
    }

    async fn context(&self, ctx: &PlayContext) -> AppResult<ResolvedContext> {
        let ids = ctx.track_ids.as_deref().unwrap_or_default();
        if ctx.kind == ContextKind::Tracks && ids.iter().any(|i| is_arriving(i)) {
            return Ok(ResolvedContext { tracks: self.tracks(ids).await?, title: None });
        }
        self.inner.context(ctx).await
    }

    async fn playable_now(&self) -> Option<std::collections::HashSet<String>> {
        let mut ok = self.inner.playable_now().await?;
        ok.extend(self.arriving.0.lock().keys().cloned());
        Some(ok)
    }

    fn left_out_offline(&self, n: usize) {
        self.inner.left_out_offline(n)
    }

    async fn more(&self, req: &MoreRequest) -> AppResult<Vec<Track>> {
        self.inner.more(req).await
    }

    fn hidden(&self) -> Option<Arc<crate::hidden::HiddenSet>> {
        self.inner.hidden()
    }
}

/// Plays taps on this device and tells the window.
pub struct AppTapHost {
    pub app: tauri::AppHandle,
    pub player: PlayerHandle,
    pub arriving: Arriving,
    pub db: DbHandle,
    pub last_refresh: Mutex<Option<Instant>>,
}

#[async_trait]
impl TapHost for AppTapHost {
    fn changed(&self, tap: &Tap) {
        use tauri::Emitter;
        let _ = self.app.emit(EVENT_TAP, tap);
    }

    async fn play_file(&self, tap: &Tap, file: &Path) -> AppResult<()> {
        let t = self.arriving.add(tap, file);
        let ctx = PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(vec![t.id.clone()]) };
        self.player.play_context_from(ctx, Some(t.id), None, Some(false)).await
    }

    async fn play_library(&self, tap: &Tap, track: &LibraryTrack, from_download: bool) -> AppResult<bool> {
        let album = PlayContext { kind: ContextKind::Album, id: Some(track.album_id.clone()), track_ids: None };
        if from_download {
            // Only while the download's copy is still the one playing (the
            // user may have moved on): the same spot, playing or paused as it was.
            let st = self.player.state().await?;
            let id = format!("{PREFIX}{}", tap.id);
            if st.track.as_ref().map(|t| t.id.as_str()) != Some(id.as_str()) {
                return Ok(false);
            }
            let playing = st.status == crate::audio::events::PlayerStatus::Playing;
            self.player.play_context_at(album, Some(track.track_id.clone()), st.position_ms, playing).await?;
            return Ok(true);
        }
        self.player.play_context_from(album, Some(track.track_id.clone()), None, Some(false)).await?;
        Ok(true)
    }

    fn refresh_library(&self) {
        let mut last = self.last_refresh.lock();
        if last.is_some_and(|l| l.elapsed() < REFRESH_EVERY) || self.db.sync_running() {
            return;
        }
        *last = Some(Instant::now());
        if let Err(e) = self.db.start_sync(false) {
            tracing::debug!("tap: library catch-up didn't start: {e}");
        }
    }

    fn flag_missing(&self, tap: &Tap) {
        tracing::warn!(title = %tap.title, album = %tap.album, "Plex hasn't shown a song Lidarr imported");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoFiles;
    impl LocalAudio for NoFiles {
        fn path(&self, _t: &Track) -> Option<PathBuf> {
            None
        }
        fn ids(&self) -> Vec<String> {
            Vec::new()
        }
    }

    fn tap() -> Tap {
        crate::lidarr::tap::tests_support::tap("t1", "Dawn", "Northlight", "Second Wind")
    }

    #[test]
    fn an_arriving_song_plays_its_file_and_nothing_else_changes() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("02 - Dawn.flac");
        std::fs::write(&f, b"fLaC").unwrap();
        let a = Arriving::default();
        let t = a.add(&tap(), &f);
        assert_eq!(t.id, "arriving:t1");
        assert!(is_arriving(&t.id) && !is_arriving("123"));
        assert_eq!((t.title.as_str(), t.artist_title.as_str(), t.album_title.as_str(), t.codec.as_deref()), ("Dawn", "Northlight", "Second Wind", Some("flac")));
        let files = ArrivingFiles { inner: Arc::new(NoFiles), arriving: a.clone() };
        assert_eq!(files.path(&t), Some(f.clone()));
        std::fs::remove_file(&f).unwrap();
        assert_eq!(files.path(&t), None, "moved away by the import");
    }
}
