//! "Not enough space": watch free space on the disk that holds the caches.
//! When it runs low, trim the caches first (oldest audio first, then shrink
//! the artwork cache); only if that isn't enough, show the banner. The art
//! cache cap goes back to normal once space recovers.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::db::ArtCache;
use crate::plex::PlexState;

pub const CHECK_EVERY: Duration = Duration::from_secs(60);
/// Below this much free space, start trimming.
pub const LOW: u64 = 512 * 1024 * 1024;
/// Trim until this much is free (some headroom so it doesn't flap).
pub const TARGET: u64 = 1024 * 1024 * 1024;
/// Files touched this recently are playing or about to: never trim them.
const KEEP_RECENT: Duration = Duration::from_secs(10 * 60);

/// What one check decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskAction {
    Fine,
    /// Free this many bytes from the caches.
    Trim(u64),
}

pub fn plan(available: u64) -> DiskAction {
    if available >= LOW {
        DiskAction::Fine
    } else {
        DiskAction::Trim(TARGET - available)
    }
}

/// Delete the least recently used `*.audio` files in the audio cache until
/// `want` bytes are freed. Returns the bytes freed.
pub fn trim_audio(dir: &Path, want: u64, now: SystemTime) -> u64 {
    let mut files: Vec<(PathBuf, u64, SystemTime)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "audio"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((e.path(), m.len(), m.modified().ok()?))
        })
        .filter(|(_, _, t)| now.duration_since(*t).unwrap_or_default() > KEEP_RECENT)
        .collect();
    files.sort_by_key(|f| f.2);
    let mut freed = 0;
    for (path, len, _) in files {
        if freed >= want {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            freed += len;
        }
    }
    freed
}

fn available(dir: &Path) -> Option<u64> {
    fs4::available_space(dir).ok()
}

/// Background loop.
pub fn spawn(plex: PlexState, audio_dir: PathBuf, art: ArtCache, art_cap: u64) {
    tauri::async_runtime::spawn(async move {
        loop {
            let dir = audio_dir.clone();
            let art2 = art.clone();
            let low = tokio::task::spawn_blocking(move || {
                let avail = available(&dir)?;
                match plan(avail) {
                    DiskAction::Fine => {
                        if avail >= TARGET && art2.cap() < art_cap {
                            let _ = art2.set_cap(art_cap);
                        }
                        Some(false)
                    }
                    DiskAction::Trim(want) => {
                        let mut freed = trim_audio(&dir, want, SystemTime::now());
                        if freed < want {
                            let size = art2.size();
                            let _ = art2.set_cap(size.saturating_sub(want - freed));
                            freed += size.saturating_sub(art2.size());
                        }
                        tracing::warn!("low disk space: trimmed {freed} bytes of cache");
                        let after = available(&dir).unwrap_or(avail + freed);
                        Some(after < LOW)
                    }
                }
            })
            .await
            .ok()
            .flatten();
            if let Some(low) = low {
                plex.set_low_disk(low);
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn plan_trims_only_when_low() {
        assert_eq!(plan(10 * 1024 * 1024 * 1024), DiskAction::Fine);
        assert_eq!(plan(LOW), DiskAction::Fine);
        assert_eq!(plan(100 * 1024 * 1024), DiskAction::Trim(TARGET - 100 * 1024 * 1024));
        assert_eq!(plan(0), DiskAction::Trim(TARGET));
    }

    fn file(dir: &Path, name: &str, len: usize, age: Duration) {
        let p = dir.join(name);
        fs::write(&p, vec![0u8; len]).unwrap();
        let f = fs::File::options().append(true).open(&p).unwrap();
        f.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn disk_full_trims_oldest_audio_first_and_keeps_recent() {
        let d = tempfile::tempdir().unwrap();
        let hour = Duration::from_secs(3600);
        file(d.path(), "old.audio", 1000, hour * 5);
        file(d.path(), "mid.audio", 1000, hour * 3);
        file(d.path(), "new.audio", 1000, hour);
        file(d.path(), "playing.audio", 1000, Duration::from_secs(5));
        file(d.path(), "note.txt", 1000, hour * 9);
        let freed = trim_audio(d.path(), 1500, SystemTime::now());
        assert_eq!(freed, 2000);
        assert!(!d.path().join("old.audio").exists());
        assert!(!d.path().join("mid.audio").exists());
        assert!(d.path().join("new.audio").exists());
        assert!(d.path().join("playing.audio").exists());
        assert!(d.path().join("note.txt").exists());
        // asking for more than there is frees what it can, never the current track
        assert_eq!(trim_audio(d.path(), u64::MAX, SystemTime::now()), 1000);
        assert!(d.path().join("playing.audio").exists());
    }

    #[test]
    fn free_space_is_readable() {
        let d = tempfile::tempdir().unwrap();
        assert!(available(d.path()).is_some());
    }
}
