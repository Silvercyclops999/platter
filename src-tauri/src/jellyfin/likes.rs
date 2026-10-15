//! When each favourite was first seen. Jellyfin keeps no date for a
//! favourite, and Liked Songs sorts newest first, so the date a like was
//! made here (or first seen in a sync) is kept on this computer, one small
//! file per server.

use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Seen {
    at: i64,
}

pub struct LikeDates {
    file: Option<PathBuf>,
    map: Mutex<HashMap<String, Seen>>,
}

impl LikeDates {
    /// Kept in `file` (read now); None keeps them in memory only.
    pub fn open(file: Option<PathBuf>) -> Self {
        let map = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { file, map: Mutex::new(map) }
    }

    /// The date for a liked song, recording `now` the first time.
    pub fn first_seen(&self, id: &str, now: i64) -> i64 {
        let mut m = self.map.lock();
        m.entry(id.to_string()).or_insert(Seen { at: now }).at
    }

    pub fn get(&self, id: &str) -> Option<i64> {
        self.map.lock().get(id).map(|s| s.at)
    }

    /// A like made now.
    pub fn liked(&self, id: &str, now: i64) {
        self.map.lock().insert(id.to_string(), Seen { at: now });
        self.save();
    }

    pub fn unliked(&self, id: &str) {
        if self.map.lock().remove(id).is_some() {
            self.save();
        }
    }

    /// After a full list of a library's favourites: forget the songs of
    /// that library (`in_library`) that aren't liked any more.
    pub fn keep_only(&self, liked: &std::collections::HashSet<String>, in_library: impl Fn(&str) -> bool) {
        self.map.lock().retain(|id, _| liked.contains(id) || !in_library(id));
        self.save();
    }

    pub fn save(&self) {
        let Some(file) = &self.file else { return };
        let bytes = match serde_json::to_vec(&*self.map.lock()) {
            Ok(b) => b,
            Err(_) => return,
        };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, file)).is_err() {
            tracing::debug!("like dates not saved");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_stay_and_go_with_the_like() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x.likes.json");
        let l = LikeDates::open(Some(f.clone()));
        assert_eq!(l.first_seen("t:1", 100), 100);
        assert_eq!(l.first_seen("t:1", 200), 100, "the first sighting stays");
        l.liked("t:2", 300);
        l.save();
        let again = LikeDates::open(Some(f));
        assert_eq!((again.get("t:1"), again.get("t:2")), (Some(100), Some(300)));
        again.unliked("t:2");
        assert_eq!(again.get("t:2"), None);
        let liked: std::collections::HashSet<String> = ["t:9".to_string()].into();
        again.first_seen("other:5", 1);
        again.keep_only(&liked, |id| id.starts_with("t:"));
        assert_eq!((again.get("t:1"), again.get("other:5")), (None, Some(1)));
    }
}
