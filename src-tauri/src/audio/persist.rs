//! Queue + position persistence ("Queue and position persist across
//! restarts"). Written atomically (temp file + rename) so a crash mid-write
//! never leaves a corrupt file.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::queue::Queue;

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Persisted {
    pub version: u32,
    pub queue: Queue,
    pub position_ms: u64,
    pub volume: f32,
    pub muted: bool,
}

pub fn save(path: &Path, p: &Persisted) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec(p).map_err(io::Error::other)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

/// `None` if missing, unreadable, or from an incompatible version.
pub fn load(path: &Path) -> Option<Persisted> {
    let bytes = fs::read(path).ok()?;
    // Ids saved before they carried their server get its tag (crate::sid).
    let mut v: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("ignoring unreadable queue file: {e}");
            return None;
        }
    };
    crate::sid::resolve_json(&mut v);
    match serde_json::from_value::<Persisted>(v) {
        Ok(p) if p.version == VERSION => Some(p),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!("ignoring unreadable queue file: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::queue::{AddAt, ContextInfo, Repeat};
    use crate::audio::source::ContextKind;
    use crate::audio::testutil::track;

    #[test]
    fn round_trip_with_position() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("queue.json");
        let mut q = Queue::with_seed(9);
        q.set_context(
            ContextInfo {
                kind: ContextKind::Playlist,
                id: Some("pl".into()),
                title: Some("Mix".into()),
            },
            (0..5).map(|i| track(&format!("t{i}"), "A", "/p")).collect(),
            Some(2),
            Some(false),
        );
        q.note_position(12_000);
        q.set_context_shuffle("other", true);
        q.add(vec![track("u", "B", "/p")], AddAt::End);
        q.set_repeat(Repeat::All);
        let p = Persisted {
            version: VERSION,
            queue: q.clone(),
            position_ms: 61_234,
            volume: 0.7,
            muted: true,
        };
        save(&path, &p).unwrap();
        let r = load(&path).unwrap();
        assert_eq!(r.position_ms, 61_234);
        assert_eq!(r.volume, 0.7);
        assert!(r.muted);
        assert_eq!(r.queue.view(), q.view());
        assert_eq!(r.queue.repeat(), Repeat::All);
        // Resume points and shuffle preferences are saved with the queue.
        let rp = r.queue.resume_point("pl").unwrap();
        assert_eq!((rp.track_id.as_str(), rp.position_ms), ("t2", 12_000));
        assert!(r.queue.shuffle_pref("other"));
        assert!(!r.queue.shuffle_pref("pl"));
    }

    #[test]
    fn missing_corrupt_or_old_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("q.json");
        assert!(load(&path).is_none());
        fs::write(&path, b"{not json").unwrap();
        assert!(load(&path).is_none());
        let old = Persisted {
            version: 0,
            queue: Queue::new(),
            position_ms: 0,
            volume: 1.0,
            muted: false,
        };
        save(&path, &old).unwrap();
        assert!(load(&path).is_none());
    }
}
