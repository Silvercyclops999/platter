//! The log file: what the core logs at info level and above also goes to
//! `platter.log` in the app's log folder, so a user can attach it to an
//! issue (Help, Show Logs on macOS). Nothing is sent anywhere.
//!
//! Logging starts before the app knows its folders, so lines go nowhere
//! until [`open`] is called. A file over [`MAX_BYTES`] at startup becomes
//! `platter.old.log` (one kept).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Runtime};

pub const FILE_NAME: &str = "platter.log";
pub const OLD_FILE_NAME: &str = "platter.old.log";
/// Bigger than this at startup: start a new file.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;

static FILE: OnceLock<Mutex<File>> = OnceLock::new();

/// The writer the file layer logs through.
#[derive(Clone, Copy, Default)]
pub struct LogFile;

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match FILE.get() {
            Some(f) => f.lock().write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match FILE.get() {
            Some(f) => f.lock().flush(),
            None => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogFile {
    type Writer = LogFile;
    fn make_writer(&'a self) -> Self::Writer {
        LogFile
    }
}

/// Opens (or starts) the log file in `dir`, moving a big one aside first.
pub fn open_file(dir: &Path) -> io::Result<(PathBuf, File)> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(FILE_NAME);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        std::fs::rename(&path, dir.join(OLD_FILE_NAME))?;
    }
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    Ok((path, file))
}

/// The folder the log file lives in.
pub fn dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_log_dir().ok()
}

/// Starts writing the log file (once).
pub fn open<R: Runtime>(app: &AppHandle<R>) {
    let Some(dir) = dir(app) else { return };
    match open_file(&dir) {
        Ok((_, file)) => {
            let _ = FILE.set(Mutex::new(file));
        }
        Err(e) => tracing::warn!("log file: {e}"),
    }
}

/// Shows the log folder in Finder / Explorer.
pub fn show<R: Runtime>(app: &AppHandle<R>) {
    use tauri_plugin_opener::OpenerExt;
    let Some(dir) = dir(app) else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Err(e) = app.opener().open_path(dir.to_string_lossy(), None::<&str>) {
        tracing::warn!("show logs: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_big_log_is_moved_aside_and_a_small_one_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("logs");
        let (path, mut f) = open_file(&dir).unwrap();
        assert_eq!(path, dir.join(FILE_NAME));
        f.write_all(b"first\n").unwrap();
        drop(f);
        let (_, mut f) = open_file(&dir).unwrap();
        f.write_all(b"second\n").unwrap();
        drop(f);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        assert!(!dir.join(OLD_FILE_NAME).exists());

        std::fs::write(&path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        let (_, mut f) = open_file(&dir).unwrap();
        f.write_all(b"fresh\n").unwrap();
        drop(f);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh\n");
        assert_eq!(std::fs::metadata(dir.join(OLD_FILE_NAME)).unwrap().len(), MAX_BYTES + 1);
    }

    #[test]
    fn lines_before_the_file_opens_go_nowhere() {
        // FILE may be set by another test in this process; either way a
        // write reports the whole line taken.
        assert_eq!(LogFile.write(b"early\n").unwrap(), 6);
    }
}
