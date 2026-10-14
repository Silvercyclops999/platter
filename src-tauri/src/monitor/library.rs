//! What a library sync found, turned into what the user sees:
//! no music library → "No music library found." banner (cleared by the next
//! sync that finds one); library replaced → "This library changed." toast
//! (the cache was already rebuilt by the sync).

use std::sync::Arc;

use crate::db::DbEvents;
use crate::db::types::{SyncOutcome, SyncStatus};
use crate::plex::PlexState;

pub const LIBRARY_CHANGED: &str = "This library changed.";

/// Wraps the app's db event sink and reports sync outcomes to health.
pub struct SyncHealth {
    pub inner: Arc<dyn DbEvents>,
    pub plex: PlexState,
}

impl DbEvents for SyncHealth {
    fn sync_progress(&self, status: &SyncStatus) {
        self.inner.sync_progress(status);
    }

    fn library_changed(&self) {
        self.inner.library_changed();
    }

    fn sync_outcome(&self, outcome: &SyncOutcome) {
        match outcome {
            SyncOutcome::NoMusicLibrary => self.plex.set_no_music_library(true),
            SyncOutcome::Cancelled => {}
            SyncOutcome::LibraryReplaced { .. } => {
                self.plex.set_no_music_library(false);
                self.plex.toast("info", LIBRARY_CHANGED);
            }
            SyncOutcome::Full { .. } | SyncOutcome::Incremental { .. } => {
                self.plex.set_no_music_library(false)
            }
        }
        self.inner.sync_outcome(outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::discovery::RaceConfig;
    use crate::plex::keychain::MemoryStore;
    use crate::plex::state::EventSink;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct Rec(Mutex<Vec<(String, serde_json::Value)>>);
    impl EventSink for Rec {
        fn emit(&self, e: &str, v: serde_json::Value) {
            self.0.lock().push((e.into(), v));
        }
    }

    struct NoDb;
    impl DbEvents for NoDb {
        fn sync_progress(&self, _: &SyncStatus) {}
        fn library_changed(&self) {}
    }

    fn rig() -> (SyncHealth, Arc<Rec>, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let plex = PlexState::with_parts(
            d.path().to_path_buf(),
            Arc::new(MemoryStore::new()),
            "http://127.0.0.1:9",
            RaceConfig::default(),
        )
        .unwrap();
        let rec = Arc::new(Rec::default());
        plex.set_event_sink(rec.clone());
        (SyncHealth { inner: Arc::new(NoDb), plex }, rec, d)
    }

    #[test]
    fn no_music_library_sets_and_clears_the_flag() {
        let (s, _rec, _d) = rig();
        s.sync_outcome(&SyncOutcome::NoMusicLibrary);
        assert!(s.plex.no_music_library());
        // a cancelled run says nothing either way
        s.sync_outcome(&SyncOutcome::Cancelled);
        assert!(s.plex.no_music_library());
        // the user added a library; the next sync finds it
        s.sync_outcome(&SyncOutcome::Full { artists: 1, albums: 1, tracks: 1, deleted: 0 });
        assert!(!s.plex.no_music_library());
    }

    #[test]
    fn library_replaced_tells_the_user() {
        let (s, rec, _d) = rig();
        s.sync_outcome(&SyncOutcome::LibraryReplaced { artists: 1, albums: 1, tracks: 1 });
        let toasts: Vec<String> = rec
            .0
            .lock()
            .iter()
            .filter(|(e, _)| e == "toast")
            .map(|(_, v)| v["message"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(toasts, vec![LIBRARY_CHANGED.to_string()]);
    }
}
