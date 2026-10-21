//! Writing to the device. Every file goes to a hidden partial file first and
//! is renamed into place once it's whole and flushed, so pulling the device
//! out mid-copy leaves no half song under a real name. Failures are sorted
//! into the few a person can act on: the device is full, it's
//! write-protected, Platter isn't allowed to write there, or it was removed.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// The most Platter reads back from a device (its manifest).
const MAX_READ: u64 = 16 * 1024 * 1024;

/// How the device is written (a temp folder in tests, with faults).
pub trait DeviceIo: Send + Sync {
    /// Copies a local file onto the device at `to`. `stop` is checked
    /// between chunks (an Interrupted error when it says stop).
    fn copy_in(&self, from: &Path, to: &Path, stop: &dyn Fn() -> bool, progress: &mut dyn FnMut(u64)) -> io::Result<u64>;
    /// Writes small content (the manifest, playlists) the same safe way.
    fn write_bytes(&self, to: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Adds to the end of a small file (the manifest's journal) and flushes
    /// it to the device.
    fn append(&self, to: &Path, bytes: &[u8]) -> io::Result<()>;
    fn read(&self, p: &Path) -> io::Result<Vec<u8>>;
    fn size(&self, p: &Path) -> Option<u64>;
    fn remove(&self, p: &Path) -> io::Result<()>;
    /// Removes `dir` and its parents while they're empty, up to `top`.
    fn prune(&self, dir: &Path, top: &Path);
    fn is_dir(&self, p: &Path) -> bool;
}

pub struct StdIo;

const CHUNK: usize = 1 << 20;

/// The ending of the hidden partial files.
pub const PART_SUFFIX: &str = ".platter-part";

fn part_of(to: &Path) -> PathBuf {
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    to.with_file_name(format!(".{name}{PART_SUFFIX}"))
}

/// Opens the partial file empty. A link someone left under its name is
/// removed rather than written through, so a write can't land outside the
/// device's music folder; a leftover half file is emptied (removing it can
/// wait on Windows while something else has it open).
fn fresh_part(part: &Path) -> io::Result<File> {
    match fs::symlink_metadata(part) {
        Ok(m) if m.is_file() => fs::OpenOptions::new().write(true).truncate(true).open(part),
        Ok(m) if !m.is_dir() => {
            fs::remove_file(part)?;
            fs::OpenOptions::new().write(true).create_new(true).open(part)
        }
        _ => fs::OpenOptions::new().write(true).create_new(true).open(part),
    }
}

/// Refuses a link where a file Platter adds to should be.
fn not_a_link(p: &Path) -> io::Result<()> {
    match fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => Err(io::Error::new(io::ErrorKind::InvalidInput, "a link, not a file")),
        _ => Ok(()),
    }
}

fn finish(part: &Path, to: &Path, f: File) -> io::Result<()> {
    f.sync_all()?;
    drop(f);
    fs::rename(part, to)
}

impl DeviceIo for StdIo {
    fn copy_in(&self, from: &Path, to: &Path, stop: &dyn Fn() -> bool, progress: &mut dyn FnMut(u64)) -> io::Result<u64> {
        if let Some(dir) = to.parent() {
            fs::create_dir_all(dir)?;
        }
        let part = part_of(to);
        let mut run = || -> io::Result<u64> {
            let mut src = File::open(from)?;
            let mut out = fresh_part(&part)?;
            let mut buf = vec![0u8; CHUNK];
            let mut n = 0u64;
            loop {
                if stop() {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
                }
                let got = src.read(&mut buf)?;
                if got == 0 {
                    break;
                }
                out.write_all(&buf[..got])?;
                n += got as u64;
                progress(n);
            }
            finish(&part, to, out)?;
            Ok(n)
        };
        let r = run();
        if r.is_err() {
            let _ = fs::remove_file(&part);
        }
        r
    }

    fn write_bytes(&self, to: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(dir) = to.parent() {
            fs::create_dir_all(dir)?;
        }
        let part = part_of(to);
        let r = (|| {
            let mut f = fresh_part(&part)?;
            f.write_all(bytes)?;
            finish(&part, to, f)
        })();
        if r.is_err() {
            let _ = fs::remove_file(&part);
        }
        r
    }

    fn append(&self, to: &Path, bytes: &[u8]) -> io::Result<()> {
        not_a_link(to)?;
        let mut f = fs::OpenOptions::new().create(true).append(true).open(to)?;
        f.write_all(bytes)?;
        f.sync_all()
    }

    fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        // Only small regular files (the manifest) are read back from a device.
        let m = fs::symlink_metadata(p)?;
        if !m.is_file() || m.len() > MAX_READ {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a small regular file"));
        }
        fs::read(p)
    }

    fn size(&self, p: &Path) -> Option<u64> {
        fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| m.len())
    }

    fn remove(&self, p: &Path) -> io::Result<()> {
        match fs::remove_file(p) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }

    fn prune(&self, dir: &Path, top: &Path) {
        let mut d = dir.to_path_buf();
        while d.starts_with(top) && d != top {
            // remove_dir only removes an empty folder.
            if fs::remove_dir(&d).is_err() {
                break;
            }
            if !d.pop() {
                break;
            }
        }
    }

    fn is_dir(&self, p: &Path) -> bool {
        p.is_dir()
    }
}

/// Removes the partial files a pulled-out drive or a crash left below
/// `top`. Only files with the partial ending go, and links and junctions
/// are never followed. Returns how many went.
pub fn sweep_parts(top: &Path) -> usize {
    fn walk(dir: &Path, depth: usize, n: &mut usize) {
        if depth > 8 {
            return;
        }
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            // file_type() doesn't follow links: a link is never a dir here.
            let Ok(t) = e.file_type() else { continue };
            let p = e.path();
            if t.is_symlink() {
                continue;
            }
            if t.is_dir() {
                walk(&p, depth + 1, n);
            } else if t.is_file() && e.file_name().to_string_lossy().ends_with(PART_SUFFIX) {
                match fs::remove_file(&p) {
                    Ok(()) => *n += 1,
                    Err(err) => tracing::warn!(path = %p.display(), "device sync: couldn't remove a partial file ({err})"),
                }
            }
        }
    }
    let mut n = 0;
    if fs::symlink_metadata(top).is_ok_and(|m| m.is_dir()) {
        walk(top, 0, &mut n);
    }
    n
}

/// Why writing to the device stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Full,
    WriteProtected,
    /// The system refused: permissions, a read-only folder, a link.
    AccessDenied,
    Removed,
    Other(String),
}

impl Failure {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::WriteProtected => "writeProtected",
            Self::AccessDenied => "accessDenied",
            Self::Removed => "removed",
            Self::Other(_) => "other",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Full => "The device is full. Free some space on it, set a smaller size limit, or choose less, then sync again.".into(),
            Self::WriteProtected => {
                "The device is write-protected. Slide its lock switch off, or make sure it isn't read-only, then sync again.".into()
            }
            Self::AccessDenied => {
                "Platter isn't allowed to write to the music folder on the device. Check that the folder isn't read-only and that you can change it, then sync again.".into()
            }
            Self::Removed => "The device was removed before the sync finished. Plug it back in and sync again; the songs already copied stay.".into(),
            Self::Other(m) => format!("Couldn't write to the device: {m}"),
        }
    }
}

/// Sorts a write error. `root_present`: the device's top folder is still
/// there (when it isn't, the device was pulled out).
pub fn classify(e: &io::Error, root_present: bool) -> Failure {
    if !root_present {
        return Failure::Removed;
    }
    // The system's own code first: Windows files a write-protected device
    // and a refused folder under different codes but similar error kinds.
    if let Some(code) = e.raw_os_error() {
        #[cfg(windows)]
        match code {
            // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
            39 | 112 => return Failure::Full,
            // ERROR_WRITE_PROTECT
            19 => return Failure::WriteProtected,
            // ERROR_ACCESS_DENIED
            5 => return Failure::AccessDenied,
            // ERROR_NOT_READY, ERROR_DEV_NOT_EXIST, ERROR_DEVICE_NOT_CONNECTED,
            // ERROR_DEVICE_REMOVED, ERROR_NO_MEDIA_IN_DRIVE
            21 | 55 | 1167 | 1617 | 1112 => return Failure::Removed,
            _ => {}
        }
        #[cfg(unix)]
        match code {
            // ENOSPC, EDQUOT
            28 | 122 | 69 => return Failure::Full,
            // EROFS
            30 => return Failure::WriteProtected,
            // EPERM, EACCES
            1 | 13 => return Failure::AccessDenied,
            // ENODEV, ENXIO, EIO
            19 | 6 | 5 => return Failure::Removed,
            _ => {}
        }
        let _ = code;
    }
    match e.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded | io::ErrorKind::FileTooLarge => Failure::Full,
        io::ErrorKind::ReadOnlyFilesystem => Failure::WriteProtected,
        io::ErrorKind::PermissionDenied => Failure::AccessDenied,
        _ => Failure::Other(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_sort_into_what_people_can_act_on() {
        assert_eq!(classify(&io::Error::from(io::ErrorKind::StorageFull), true), Failure::Full);
        assert_eq!(classify(&io::Error::from(io::ErrorKind::ReadOnlyFilesystem), true), Failure::WriteProtected);
        assert_eq!(classify(&io::Error::from(io::ErrorKind::PermissionDenied), true), Failure::AccessDenied);
        assert_eq!(classify(&io::Error::from(io::ErrorKind::NotFound), false), Failure::Removed);
        assert!(matches!(classify(&io::Error::other("odd"), true), Failure::Other(_)));
        #[cfg(windows)]
        {
            assert_eq!(classify(&io::Error::from_raw_os_error(112), true), Failure::Full);
            assert_eq!(classify(&io::Error::from_raw_os_error(19), true), Failure::WriteProtected);
            assert_eq!(classify(&io::Error::from_raw_os_error(21), true), Failure::Removed);
        }
        #[cfg(unix)]
        {
            assert_eq!(classify(&io::Error::from_raw_os_error(28), true), Failure::Full);
            assert_eq!(classify(&io::Error::from_raw_os_error(30), true), Failure::WriteProtected);
            assert_eq!(classify(&io::Error::from_raw_os_error(13), true), Failure::AccessDenied);
        }
    }

    #[cfg(windows)]
    #[test]
    fn access_denied_is_not_called_write_protected() {
        let f = classify(&io::Error::from_raw_os_error(5), true);
        assert_eq!(f, Failure::AccessDenied);
        assert_eq!(f.kind(), "accessDenied");
        assert!(!f.message().contains("write-protected"), "{}", f.message());
        assert_eq!(classify(&io::Error::from_raw_os_error(19), true).kind(), "writeProtected");
    }

    #[test]
    fn copies_land_whole_and_stopped_copies_leave_nothing() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src.bin");
        fs::write(&src, vec![7u8; CHUNK * 2 + 5]).unwrap();
        let to = d.path().join("dev/A/B/song.flac");
        let mut seen = 0;
        let n = StdIo.copy_in(&src, &to, &|| false, &mut |b| seen = b).unwrap();
        assert_eq!(n, (CHUNK * 2 + 5) as u64);
        assert_eq!(seen, n);
        assert_eq!(fs::read(&to).unwrap().len() as u64, n);
        assert!(!part_of(&to).exists());

        let to2 = d.path().join("dev/A/B/other.flac");
        let e = StdIo.copy_in(&src, &to2, &|| true, &mut |_| {}).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        assert!(!to2.exists() && !part_of(&to2).exists());
    }

    #[test]
    fn appends_add_to_the_end() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("j");
        StdIo.append(&p, b"a\n").unwrap();
        StdIo.append(&p, b"b\n").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"a\nb\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_left_under_a_partial_name_is_not_written_through() {
        let d = tempfile::tempdir().unwrap();
        let outside = d.path().join("outside.txt");
        fs::write(&outside, b"keep me").unwrap();
        let src = d.path().join("src.bin");
        fs::write(&src, b"song").unwrap();
        let dev = d.path().join("dev");
        fs::create_dir_all(&dev).unwrap();

        let to = dev.join("song.flac");
        std::os::unix::fs::symlink(&outside, part_of(&to)).unwrap();
        StdIo.copy_in(&src, &to, &|| false, &mut |_| {}).unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"song");

        let manifest = dev.join(".platter-sync.json");
        std::os::unix::fs::symlink(&outside, part_of(&manifest)).unwrap();
        StdIo.write_bytes(&manifest, b"{}").unwrap();
        assert_eq!(fs::read(&manifest).unwrap(), b"{}");

        // A leftover half file is replaced, not added to.
        fs::write(part_of(&to), b"half of an old copy").unwrap();
        StdIo.copy_in(&src, &to, &|| false, &mut |_| {}).unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"song");

        let journal = dev.join("journal");
        std::os::unix::fs::symlink(&outside, &journal).unwrap();
        assert!(StdIo.append(&journal, b"x\n").is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"keep me");
    }

    #[test]
    fn leftover_partial_files_are_swept_and_nothing_else() {
        let d = tempfile::tempdir().unwrap();
        let top = d.path().join("Music");
        fs::create_dir_all(top.join("A/B")).unwrap();
        let part = top.join("A/B/.01 Song.flac.platter-part");
        let manifest_part = top.join(".platter-sync.json.platter-part");
        fs::write(&part, b"half").unwrap();
        fs::write(&manifest_part, b"half").unwrap();
        let other = top.join("A/B/notes.platter-partial");
        let song = top.join("A/B/01 Song.flac");
        fs::write(&other, b"x").unwrap();
        fs::write(&song, b"x").unwrap();
        // A partial file outside the music folder, reachable through a link.
        let outside = d.path().join("Elsewhere");
        fs::create_dir_all(&outside).unwrap();
        let far = outside.join(".x.flac.platter-part");
        fs::write(&far, b"x").unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, top.join("link")).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&outside, top.join("link")).is_ok();

        assert_eq!(sweep_parts(&top), 2);
        assert!(!part.exists() && !manifest_part.exists());
        assert!(other.exists() && song.exists());
        if linked {
            assert!(far.exists(), "links are not followed");
        }
        assert_eq!(sweep_parts(&d.path().join("missing")), 0);
    }

    #[test]
    fn pruning_stops_at_the_music_folder_and_full_folders() {
        let d = tempfile::tempdir().unwrap();
        let top = d.path().join("Music");
        fs::create_dir_all(top.join("A/B")).unwrap();
        fs::create_dir_all(top.join("C/D")).unwrap();
        fs::write(top.join("C/keep.txt"), b"x").unwrap();
        StdIo.prune(&top.join("A/B"), &top);
        assert!(!top.join("A").exists());
        StdIo.prune(&top.join("C/D"), &top);
        assert!(top.join("C/keep.txt").exists() && !top.join("C/D").exists());
        assert!(top.exists());
    }
}
