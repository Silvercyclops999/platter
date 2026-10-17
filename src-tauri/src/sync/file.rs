//! The sync file: `Platter/sync-<account tag>.json` in a folder the user
//! picks (one their cloud app already syncs). One file per Plex account;
//! the tag is a hash of the account id, so the name says nothing about it.
//!
//! Writes are atomic (a temp file beside it, then a rename), so a cloud
//! client never uploads half a file. When two devices write before the cloud
//! catches up, cloud clients keep both as "conflicted copies"
//! (`sync-<tag> (conflicted copy).json`, `sync-<tag>-PC-NAME.json`,
//! `sync-<tag> (1).json`); those are merged in and removed.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::store::{Entry, EntryMap};
use crate::error::{AppError, AppResult};

/// The format this build writes. A file with a higher one was written by a
/// newer Platter and is left alone.
pub const FORMAT: u32 = 1;
/// The folder Platter keeps its file in, inside the chosen folder.
pub const SUBFOLDER: &str = "Platter";
/// Tombstones are kept this long so every device hears about the deletion.
pub const TOMBSTONE_MS: u64 = 90 * 24 * 60 * 60 * 1000;

/// When a device last wrote the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSeen {
    pub name: String,
    pub last_seen: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFileData {
    #[serde(default)]
    pub format: u32,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub devices: BTreeMap<String, DeviceSeen>,
    #[serde(default)]
    pub entries: EntryMap,
}

/// Why a file couldn't be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// Not JSON, or not a sync file.
    Unreadable,
    /// Written by a newer Platter.
    Newer,
    /// The folder or file couldn't be read at all.
    Io(String),
}

pub fn file_name(tag: &str) -> String {
    format!("sync-{tag}.json")
}

pub fn dir(folder: &Path) -> PathBuf {
    folder.join(SUBFOLDER)
}

/// Read a sync file; Ok(None) when there's none yet.
pub fn read(path: &Path) -> Result<Option<SyncFileData>, ReadError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ReadError::Io(e.to_string())),
    };
    let data: SyncFileData = serde_json::from_slice(&bytes).map_err(|_| ReadError::Unreadable)?;
    if data.app != "Platter" {
        return Err(ReadError::Unreadable);
    }
    if data.format > FORMAT {
        return Err(ReadError::Newer);
    }
    Ok(Some(data))
}

/// Write the file whole: a temp file beside it, flushed, then renamed over.
pub fn write_atomic(path: &Path, data: &SyncFileData) -> AppResult<()> {
    let json = serde_json::to_vec_pretty(data).map_err(|e| AppError::Internal(e.to_string()))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("sync.json");
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::Io(format!("couldn't write the sync file: {e}")));
    }
    Ok(())
}

/// Copies a cloud client made of the file beside it: names starting with
/// `sync-<tag>` and ending `.json`, other than the file itself.
pub fn conflicted_copies(dir: &Path, tag: &str) -> Vec<PathBuf> {
    let main = file_name(tag);
    let stem = format!("sync-{tag}");
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let n = e.file_name().to_str()?.to_string();
            let copy = n != main
                && n.starts_with(&stem)
                && n.to_ascii_lowercase().ends_with(".json")
                && !n.contains(".broken")
                // The next character isn't part of a longer tag.
                && n[stem.len()..].chars().next().is_some_and(|c| !c.is_ascii_hexdigit());
            copy.then(|| e.path())
        })
        .collect();
    out.sort();
    out
}

/// Merge `other` into `base` where its entries win. Returns whether `base` changed.
pub fn merge_into(base: &mut EntryMap, other: &EntryMap) -> bool {
    let mut changed = false;
    for (ns, entries) in other {
        let mine = base.entry(ns.clone()).or_default();
        for (key, e) in entries {
            let take = mine.get(key).is_none_or(|m: &Entry| e.wins_over(m));
            if take && mine.get(key) != Some(e) {
                mine.insert(key.clone(), e.clone());
                changed = true;
            }
        }
    }
    base.retain(|_, m| !m.is_empty());
    changed
}

/// A device not seen for this long no longer holds tombstones back (it
/// was most likely retired).
pub const FORGET_DEVICE_MS: u64 = 365 * 24 * 60 * 60 * 1000;

/// The time before which tombstones may go: older than [`TOMBSTONE_MS`],
/// and older than the last time every other device wrote the file (a device
/// merges the file before it writes, so it has seen them). Devices not
/// seen for [`FORGET_DEVICE_MS`] don't count. Otherwise a device away for
/// a long time would bring deleted things back.
pub fn prune_before(devices: &BTreeMap<String, DeviceSeen>, me: &str, now: u64) -> u64 {
    let forgotten = now.saturating_sub(FORGET_DEVICE_MS);
    devices
        .iter()
        .filter(|(id, d)| id.as_str() != me && d.last_seen >= forgotten)
        .map(|(_, d)| d.last_seen)
        .fold(now.saturating_sub(TOMBSTONE_MS), u64::min)
}

/// Drop tombstones older than `before` (from [`prune_before`]).
pub fn prune(map: &mut EntryMap, before: u64) {
    for m in map.values_mut() {
        m.retain(|_, e| !(e.is_tombstone() && e.ts < before));
    }
    map.retain(|_, m| !m.is_empty());
}

/// Move an unreadable file aside (kept, never deleted) so a new one can start.
pub fn set_aside(path: &Path, now: u64) -> AppResult<PathBuf> {
    let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("sync");
    let to = path.with_file_name(format!("{name}.broken-{now}.json"));
    std::fs::rename(path, &to).map_err(|e| AppError::Io(format!("couldn't move the old sync file: {e}")))?;
    Ok(to)
}
