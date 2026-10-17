//! Writing to the device. Every file goes to a hidden partial file first and
//! is renamed into place once it's whole and flushed, so pulling the device
//! out mid-copy leaves no half song under a real name. Failures are sorted
//! into the few a person can act on: the device is full, it's
//! write-protected, or it was removed.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// How the device is written (a temp folder in tests, with faults).
pub trait DeviceIo: Send + Sync {
    /// Copies a local file onto the device at `to`. `stop` is checked
    /// between chunks (an Interrupted error when it says stop).
    fn copy_in(&self, from: &Path, to: &Path, stop: &dyn Fn() -> bool, progress: &mut dyn FnMut(u64)) -> io::Result<u64>;
    /// Writes small content (the manifest, playlists) the same safe way.
    fn write_bytes(&self, to: &Path, bytes: &[u8]) -> io::Result<()>;
    fn read(&self, p: &Path) -> io::Result<Vec<u8>>;
    fn size(&self, p: &Path) -> Option<u64>;
    fn remove(&self, p: &Path) -> io::Result<()>;
    /// Removes `dir` and its parents while they're empty, up to `top`.
    fn prune(&self, dir: &Path, top: &Path);
    fn is_dir(&self, p: &Path) -> bool;
}

pub struct StdIo;

const CHUNK: usize = 1 << 20;

fn part_of(to: &Path) -> PathBuf {
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    to.with_file_name(format!(".{name}.platter-part"))
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
            let mut out = File::create(&part)?;
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
            let mut f = File::create(&part)?;
            f.write_all(bytes)?;
            finish(&part, to, f)
        })();
        if r.is_err() {
            let _ = fs::remove_file(&part);
        }
        r
    }

    fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
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

/// Why writing to the device stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Full,
    WriteProtected,
    Removed,
    Other(String),
}

impl Failure {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::WriteProtected => "writeProtected",
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
    match e.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded | io::ErrorKind::FileTooLarge => return Failure::Full,
        io::ErrorKind::ReadOnlyFilesystem | io::ErrorKind::PermissionDenied => return Failure::WriteProtected,
        _ => {}
    }
    if let Some(code) = e.raw_os_error() {
        #[cfg(windows)]
        match code {
            // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
            39 | 112 => return Failure::Full,
            // ERROR_WRITE_PROTECT, ERROR_ACCESS_DENIED
            19 | 5 => return Failure::WriteProtected,
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
            // ENODEV, ENXIO, EIO
            19 | 6 | 5 => return Failure::Removed,
            _ => {}
        }
        let _ = code;
    }
    Failure::Other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_sort_into_what_people_can_act_on() {
        assert_eq!(classify(&io::Error::from(io::ErrorKind::StorageFull), true), Failure::Full);
        assert_eq!(classify(&io::Error::from(io::ErrorKind::ReadOnlyFilesystem), true), Failure::WriteProtected);
        assert_eq!(classify(&io::Error::from(io::ErrorKind::PermissionDenied), true), Failure::WriteProtected);
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
        }
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
