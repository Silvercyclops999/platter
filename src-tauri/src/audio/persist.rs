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

/// The same file as [`Persisted`], written from borrowed parts so saving
/// doesn't copy the queue.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedRef<'a> {
    pub version: u32,
    pub queue: &'a Queue,
    pub position_ms: u64,
    pub volume: f32,
    pub muted: bool,
}

pub fn to_bytes(p: &PersistedRef<'_>) -> io::Result<Vec<u8>> {
    serde_json::to_vec(p).map_err(io::Error::other)
}

pub fn save(path: &Path, p: &Persisted) -> io::Result<()> {
    let json = serde_json::to_vec(p).map_err(io::Error::other)?;
    save_bytes(path, &json)
}

/// Writes an encoded queue file atomically.
pub fn save_bytes(path: &Path, json: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

enum Job {
    Write(std::path::PathBuf, Vec<u8>),
    /// Answered once everything sent before it is on disk.
    Ack(Box<dyn FnOnce() + Send>),
}

/// Writes queue files on their own thread, so a long queue doesn't hold up
/// the player. Saves that pile up while one is written collapse into the
/// newest.
pub struct Writer {
    tx: std::sync::mpsc::Sender<Job>,
}

impl Writer {
    pub fn spawn() -> Option<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("queue-save".into())
            .spawn(move || {
                let write = |p: &Path, b: &[u8]| {
                    if let Err(e) = save_bytes(p, b) {
                        tracing::warn!("could not save queue: {e}");
                    }
                };
                while let Ok(mut job) = rx.recv() {
                    loop {
                        match job {
                            Job::Write(p, b) => match rx.try_recv() {
                                Ok(Job::Write(p2, b2)) => job = Job::Write(p2, b2),
                                Ok(next) => {
                                    write(&p, &b);
                                    job = next;
                                }
                                Err(_) => {
                                    write(&p, &b);
                                    break;
                                }
                            },
                            Job::Ack(done) => {
                                done();
                                break;
                            }
                        }
                    }
                }
            })
            .ok()?;
        Some(Self { tx })
    }

    pub fn write(&self, path: std::path::PathBuf, bytes: Vec<u8>) {
        let _ = self.tx.send(Job::Write(path, bytes));
    }

    /// Resolves once every earlier write is done.
    pub fn ack(&self, done: tokio::sync::oneshot::Sender<()>) {
        self.after(Box::new(move || {
            let _ = done.send(());
        }));
    }

    /// Waits (blocking) until every earlier write is done.
    pub fn wait(&self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.after(Box::new(move || {
            let _ = tx.send(());
        }));
        let _ = rx.recv();
    }

    fn after(&self, f: Box<dyn FnOnce() + Send>) {
        if let Err(std::sync::mpsc::SendError(Job::Ack(f))) = self.tx.send(Job::Ack(f)) {
            f();
        }
    }
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
        Ok(mut p) if p.version == VERSION => {
            if p.queue.repair() {
                tracing::warn!("queue file didn't hold together; repaired it");
            }
            if !p.volume.is_finite() {
                p.volume = 1.0;
            }
            Some(p)
        }
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
    fn a_damaged_queue_file_is_repaired_instead_of_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("q.json");
        let mut q = Queue::with_seed(1);
        q.set_context(
            ContextInfo { kind: ContextKind::Album, id: Some("al".into()), title: None },
            (0..4).map(|i| track(&format!("t{i}"), "A", "/p")).collect(),
            Some(1),
            Some(false),
        );
        let p = Persisted { version: VERSION, queue: q, position_ms: 5, volume: 0.5, muted: false };
        let mut v = serde_json::to_value(&p).unwrap();
        for (order, pos) in [(serde_json::json!([0, 7, 2, 3]), 1), (serde_json::json!([0, 1, 1]), 1), (serde_json::json!([3, 2, 1, 0]), 99)] {
            v["queue"]["order"] = order;
            v["queue"]["pos"] = pos.into();
            v["volume"] = serde_json::Value::Null;
            fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
            // A null volume isn't readable: the file is ignored.
            assert!(load(&path).is_none());
            v["volume"] = 0.5.into();
            fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
            let mut r = load(&path).unwrap();
            // Everything that walks the queue stays in range.
            let view = r.queue.view();
            assert!(view.next_up.len() <= 4);
            r.queue.upcoming(10);
            r.queue.note_position(0);
            while r.queue.next_manual().is_some() {}
            r.queue.previous();
        }
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
