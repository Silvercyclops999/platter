//! The swap: better copies from the staging folder go into the library,
//! one file at a time, where the files are. Lidarr is never involved.
//!
//! For each file:
//!
//! 1. the new copy is copied next to the old file under a hidden name
//! 2. the old file's tags are carried onto it and read back (`tags::carry`)
//! 3. the old file moves to the backup folder, checked byte for byte
//! 4. the new copy takes the old file's name (its extension may change)
//!
//! Everything is checked before the first change: the share mapping covers
//! every file, the share answers, no file is in use, the backup folder
//! takes a test file, and no new name is taken. Any failure on the way puts
//! every file of the swap back as it was, so the library is never left half
//! swapped and an old file is never deleted. Undo puts the old files back
//! exactly (the same bytes) and keeps the new copies in the backup folder.
//!
//! The files are reached through a [`SwapHost`]: this computer through the
//! share mapping now; a home device running next to the files can be
//! another host later, with the same steps.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppResult;
use crate::musicshare::ShareMap;
use crate::tags::FileTags;
use crate::tags::carry::AudioInfo;

/// File operations, so tests can fake a share that fails.
pub trait ShareFs: Send + Sync {
    fn exists(&self, p: &Path) -> bool;
    fn is_dir(&self, p: &Path) -> bool;
    fn list(&self, dir: &Path) -> io::Result<Vec<PathBuf>>;
    fn copy(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove(&self, p: &Path) -> io::Result<()>;
    fn create_dir_all(&self, p: &Path) -> io::Result<()>;
    fn hash(&self, p: &Path) -> io::Result<String>;
    /// Fails when another program has the file open for writing or locked.
    fn check_unlocked(&self, p: &Path) -> io::Result<()>;
    /// Writes and removes a small file in `dir`.
    fn probe_writable(&self, dir: &Path) -> io::Result<()>;
}

/// Reading and carrying tags.
pub trait TagIo: Send + Sync {
    /// The tags Platter reads (ARCHIVE_*, ISRC, recording id...).
    fn read(&self, p: &Path) -> AppResult<FileTags>;
    fn audio(&self, p: &Path) -> AppResult<AudioInfo>;
    /// Put `old`'s tags on `new` and prove nothing was lost.
    fn carry(&self, old: &Path, new: &Path) -> AppResult<()>;
}

/// Where the swap runs and how it reaches the files.
pub trait SwapHost: Send + Sync {
    /// "this computer", for messages.
    fn label(&self) -> String;
    /// A music file (Plex's path) on this host; None without a mapping.
    fn music_path(&self, plex_path: &str) -> Option<PathBuf>;
    /// A staging file (Lidarr's path) on this host.
    fn staging_path(&self, lidarr_path: &str) -> Option<PathBuf>;
    /// The folder replaced files go in.
    fn backup_root(&self) -> PathBuf;
    fn fs(&self) -> &dyn ShareFs;
    fn tags(&self) -> &dyn TagIo;
}

// ---------------------------------------------------------------- this computer

pub struct LocalFs;

impl ShareFs for LocalFs {
    fn exists(&self, p: &Path) -> bool {
        p.exists()
    }

    fn is_dir(&self, p: &Path) -> bool {
        p.is_dir()
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect();
        v.sort();
        Ok(v)
    }

    fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::copy(from, to)?;
        std::fs::File::open(to)?.sync_all()
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn remove(&self, p: &Path) -> io::Result<()> {
        std::fs::remove_file(p)
    }

    fn create_dir_all(&self, p: &Path) -> io::Result<()> {
        std::fs::create_dir_all(p)
    }

    fn hash(&self, p: &Path) -> io::Result<String> {
        let mut f = std::fs::File::open(p)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    fn check_unlocked(&self, p: &Path) -> io::Result<()> {
        let f = std::fs::OpenOptions::new().read(true).write(true).open(p)?;
        f.try_lock().map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "in use"))?;
        f.unlock()
    }

    fn probe_writable(&self, dir: &Path) -> io::Result<()> {
        let p = dir.join(".platter-write-test");
        std::fs::write(&p, b"ok")?;
        std::fs::remove_file(&p)
    }
}

pub struct LoftyTags;

impl TagIo for LoftyTags {
    fn read(&self, p: &Path) -> AppResult<FileTags> {
        crate::tags::read_local(p).map_err(|e| crate::error::AppError::Invalid(format!("Platter couldn't read the tags of {} ({e}).", name(p))))
    }

    fn audio(&self, p: &Path) -> AppResult<AudioInfo> {
        crate::tags::carry::audio(p)
    }

    fn carry(&self, old: &Path, new: &Path) -> AppResult<()> {
        crate::tags::carry::carry(old, new)
    }
}

/// This computer, through the music share mapping.
pub struct DesktopHost {
    pub map: ShareMap,
    /// Lidarr's path for the staging folder (the setting).
    pub lidarr_staging: String,
    /// Platter's own folder, for backups when none is set.
    pub fallback_backup: PathBuf,
    pub fs: Box<dyn ShareFs>,
    pub tags: Box<dyn TagIo>,
}

impl SwapHost for DesktopHost {
    fn label(&self) -> String {
        "this computer".into()
    }

    fn music_path(&self, plex_path: &str) -> Option<PathBuf> {
        self.map.music_path(plex_path)
    }

    fn staging_path(&self, lidarr_path: &str) -> Option<PathBuf> {
        self.map.staging_path(&self.lidarr_staging, lidarr_path)
    }

    fn backup_root(&self) -> PathBuf {
        crate::musicshare::backup_dir(&self.map, &self.fallback_backup)
    }

    fn fs(&self) -> &dyn ShareFs {
        self.fs.as_ref()
    }

    fn tags(&self) -> &dyn TagIo {
        self.tags.as_ref()
    }
}

// ---------------------------------------------------------------- the swap

/// Why a swap (or its undo) stopped. Every one leaves the old files where
/// they were.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Failure {
    /// No share mapping covers the files.
    NoMapping,
    /// The share (or a folder on it) can't be reached.
    ShareUnreachable { path: String },
    /// A file isn't where it was when the offer was made.
    Changed { name: String },
    StagingMissing { name: String },
    FileLocked { name: String },
    NameTaken { name: String },
    TagCopyFailed { name: String, why: String },
    BackupNotWritable { path: String },
    /// A copy or rename on the share failed part way.
    MoveFailed { name: String },
    RescanFailed,
    /// Plex can't be reached (or no music library covers the folder) before the swap.
    PlexUnreachable,
    /// Undo: the kept file is gone from the backup folder.
    BackupMissing { name: String },
}

impl Failure {
    pub fn message(&self) -> String {
        match self {
            Failure::NoMapping => {
                "Platter doesn't know where this computer reaches your music share. Set it in Settings > Integrations > Lidarr, then try again. Nothing was changed.".into()
            }
            Failure::ShareUnreachable { path } => format!("Platter can't reach {path}. Check the share is connected, then try again. Nothing was changed."),
            Failure::Changed { name } => format!("{name} isn't where it was. Nothing was changed; look at the album again."),
            Failure::StagingMissing { name } => format!("The better copy of {name} isn't in the staging folder any more. Nothing was changed."),
            Failure::FileLocked { name } => format!("{name} is in use by another program. Close it, then try again. Nothing was changed."),
            Failure::NameTaken { name } => format!("There's already a file called {name} in that folder, so Platter left the album as it is."),
            Failure::TagCopyFailed { name, why } => format!("{why} The swap stopped at {name} and every file is back as it was."),
            Failure::BackupNotWritable { path } => format!("Platter can't write to the backup folder {path}, so it changed nothing. Pick another backup folder in Settings."),
            Failure::MoveFailed { name } => format!("Platter couldn't put the new copy of {name} in place, so every file is back as it was."),
            Failure::RescanFailed => "Plex didn't take the request to rescan the folder, so Platter put the old files back. Try again when Plex is reachable.".into(),
            Failure::PlexUnreachable => "Platter can't reach Plex to rescan the folder afterwards, so it changed nothing. Try again when Plex is reachable.".into(),
            Failure::BackupMissing { name } => format!("The kept copy of {name} isn't in the backup folder any more, so Platter can't put it back."),
        }
    }
}

/// One file to swap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub track_id: String,
    /// The old file as Plex sees it.
    pub plex_path: String,
    /// The new copy as Lidarr sees it.
    pub lidarr_path: String,
}

/// One file swapped: enough to undo it exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Done {
    pub track_id: String,
    pub plex_path: String,
    /// Where the old file was (and goes back to on undo).
    pub original: String,
    /// The new copy, under the old file's name.
    pub placed: String,
    /// The old file now.
    pub backup: String,
    pub old_hash: String,
    pub new_hash: String,
}

pub fn name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn shown(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The new copy's name in the old file's place: the old name with the new
/// copy's extension.
pub fn final_path(old: &Path, new_src: &Path) -> PathBuf {
    match new_src.extension() {
        Some(e) => old.with_extension(e),
        None => old.to_path_buf(),
    }
}

fn temp_path(final_: &Path) -> PathBuf {
    final_.with_file_name(format!(".{}.platter-upgrade", name(final_)))
}

/// Move a file and prove the bytes arrived: a rename where it can, else a
/// copy, a hash check and only then the removal of the original.
fn move_checked(fs: &dyn ShareFs, from: &Path, to: &Path, hash: &str) -> Result<(), MoveErr> {
    if let Some(dir) = to.parent() {
        fs.create_dir_all(dir).map_err(|_| MoveErr::Target)?;
    }
    if fs.rename(from, to).is_ok() {
        return if fs.hash(to).map(|h| h == hash).unwrap_or(false) { Ok(()) } else { Err(MoveErr::Target) };
    }
    fs.copy(from, to).map_err(|_| MoveErr::Target)?;
    if !fs.hash(to).map(|h| h == hash).unwrap_or(false) {
        let _ = fs.remove(to);
        return Err(MoveErr::Target);
    }
    if fs.remove(from).is_err() {
        let _ = fs.remove(to);
        return Err(MoveErr::Source);
    }
    Ok(())
}

enum MoveErr {
    /// The destination couldn't take it.
    Target,
    /// The original couldn't be removed (in use).
    Source,
}

/// A folder name for one swap's backups.
pub fn backup_folder(root: &Path, artist: &str, album: &str, swap_id: &str) -> PathBuf {
    let safe = |s: &str| -> String {
        s.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect::<String>().trim().trim_matches('.').chars().take(80).collect()
    };
    let short: String = swap_id.chars().filter(char::is_ascii_alphanumeric).take(8).collect();
    root.join(format!("{} - {} ({short})", safe(artist), safe(album)))
}

/// Put back what a swap did so far, newest first.
fn roll_back(fs: &dyn ShareFs, done: &[Done]) {
    for d in done.iter().rev() {
        let (placed, original, backup) = (PathBuf::from(&d.placed), PathBuf::from(&d.original), PathBuf::from(&d.backup));
        if placed != original || fs.exists(&placed) {
            let _ = fs.remove(&placed);
        }
        let _ = move_checked(fs, &backup, &original, &d.old_hash);
    }
}

/// Swap `items` in. All of them, or none.
pub fn run(host: &dyn SwapHost, backup_dir: &Path, items: &[Planned]) -> Result<Vec<Done>, Failure> {
    let fs = host.fs();
    // Where everything is.
    let mut resolved = Vec::new();
    for it in items {
        let old = host.music_path(&it.plex_path).ok_or(Failure::NoMapping)?;
        let new_src = host.staging_path(&it.lidarr_path).ok_or(Failure::NoMapping)?;
        resolved.push((it, old, new_src));
    }
    // Every check before the first change.
    for (_, old, new_src) in &resolved {
        let folder = old.parent().map(Path::to_path_buf).unwrap_or_default();
        if !fs.is_dir(&folder) {
            return Err(Failure::ShareUnreachable { path: shown(&folder) });
        }
        if !fs.exists(old) {
            return Err(Failure::Changed { name: name(old) });
        }
        if !fs.exists(new_src) {
            return Err(Failure::StagingMissing { name: name(old) });
        }
        if fs.check_unlocked(old).is_err() {
            return Err(Failure::FileLocked { name: name(old) });
        }
        let final_ = final_path(old, new_src);
        if final_ != *old && fs.exists(&final_) {
            return Err(Failure::NameTaken { name: name(&final_) });
        }
    }
    let backup_bad = || Failure::BackupNotWritable { path: shown(backup_dir) };
    fs.create_dir_all(backup_dir).map_err(|_| backup_bad())?;
    fs.probe_writable(backup_dir).map_err(|_| backup_bad())?;

    let mut done: Vec<Done> = Vec::new();
    for (it, old, new_src) in &resolved {
        let final_ = final_path(old, new_src);
        let tmp = temp_path(&final_);
        let fail = |f: Failure, done: &[Done]| {
            let _ = fs.remove(&tmp);
            roll_back(fs, done);
            Err(f)
        };
        if fs.copy(new_src, &tmp).is_err() {
            return fail(Failure::MoveFailed { name: name(old) }, &done);
        }
        if let Err(e) = host.tags().carry(old, &tmp) {
            return fail(Failure::TagCopyFailed { name: name(old), why: e.to_string() }, &done);
        }
        let Ok(old_hash) = fs.hash(old) else { return fail(Failure::ShareUnreachable { path: shown(old) }, &done) };
        let parent = old.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let backup = backup_dir.join(parent).join(name(old));
        if fs.exists(&backup) {
            return fail(backup_bad(), &done);
        }
        match move_checked(fs, old, &backup, &old_hash) {
            Ok(()) => {}
            Err(MoveErr::Target) => return fail(backup_bad(), &done),
            Err(MoveErr::Source) => return fail(Failure::FileLocked { name: name(old) }, &done),
        }
        let record = |new_hash: String| Done {
            track_id: it.track_id.clone(),
            plex_path: it.plex_path.clone(),
            original: shown(old),
            placed: shown(&final_),
            backup: shown(&backup),
            old_hash: old_hash.clone(),
            new_hash,
        };
        if fs.rename(&tmp, &final_).is_err() {
            let _ = move_checked(fs, &backup, old, &old_hash);
            return fail(Failure::MoveFailed { name: name(old) }, &done);
        }
        let new_hash = fs.hash(&final_).unwrap_or_default();
        done.push(record(new_hash));
    }
    Ok(done)
}

/// Put the old files back, byte for byte. The new copies move to the
/// backup folder's "Replaced" folder (nothing is deleted).
pub fn undo(host: &dyn SwapHost, done: &[Done]) -> Result<(), Failure> {
    let fs = host.fs();
    for d in done {
        let (placed, original, backup) = (PathBuf::from(&d.placed), PathBuf::from(&d.original), PathBuf::from(&d.backup));
        let folder = original.parent().map(Path::to_path_buf).unwrap_or_default();
        if !fs.is_dir(&folder) {
            return Err(Failure::ShareUnreachable { path: shown(&folder) });
        }
        if !fs.exists(&backup) || !fs.hash(&backup).is_ok_and(|h| h == d.old_hash) {
            return Err(Failure::BackupMissing { name: name(&original) });
        }
        if fs.exists(&placed) && fs.check_unlocked(&placed).is_err() {
            return Err(Failure::FileLocked { name: name(&placed) });
        }
        if placed != original && fs.exists(&original) {
            return Err(Failure::NameTaken { name: name(&original) });
        }
    }
    for d in done.iter().rev() {
        let (placed, original, backup) = (PathBuf::from(&d.placed), PathBuf::from(&d.original), PathBuf::from(&d.backup));
        if fs.exists(&placed) {
            let dir = backup.parent().and_then(Path::parent).unwrap_or(Path::new("")).join("Replaced");
            let mut keep = dir.join(name(&placed));
            let mut n = 2;
            while fs.exists(&keep) {
                keep = dir.join(format!("{n} {}", name(&placed)));
                n += 1;
            }
            let h = fs.hash(&placed).map_err(|_| Failure::ShareUnreachable { path: shown(&placed) })?;
            if move_checked(fs, &placed, &keep, &h).is_err() {
                return Err(Failure::MoveFailed { name: name(&original) });
            }
        }
        if move_checked(fs, &backup, &original, &d.old_hash).is_err() {
            return Err(Failure::MoveFailed { name: name(&original) });
        }
    }
    Ok(())
}
