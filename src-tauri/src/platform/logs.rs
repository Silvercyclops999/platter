//! The log file: what the core logs at info level and above also goes to
//! `platter.log` in the app's log folder, so a user can attach it to an
//! issue (Help, Show Logs on macOS). Nothing is sent anywhere.
//!
//! Logging starts before the app knows its folders, so lines go nowhere
//! until [`open`] is called. A file over [`MAX_BYTES`] at startup becomes
//! `platter.old.log` (one kept).
//!
//! People attach this file to public issues, so every line is passed through
//! [`redact`] first: Plex tokens, plex.direct server names and IP addresses
//! never reach the file, whatever an error message carried.

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
            Some(f) => {
                let line = redact(&String::from_utf8_lossy(buf));
                f.lock().write_all(line.as_bytes())?;
                Ok(buf.len())
            }
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

/// Removes what mustn't be in a shareable log: `X-Plex-Token`, `api_key`
/// and `Token="..."` values, `*.plex.direct` host names and IPv4 addresses.
pub fn redact(line: &str) -> String {
    let line = redact_quoted_token(line);
    let mut out = String::with_capacity(line.len());
    for word in split_keep(&line) {
        out.push_str(&redact_word(word));
    }
    out
}

/// Splits into runs of URL-ish characters and everything else, keeping both.
fn split_keep(s: &str) -> Vec<&str> {
    let urlish = |c: char| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'*+,;=%".contains(c);
    let mut parts = Vec::new();
    let mut start = 0;
    let mut cur: Option<bool> = None;
    for (i, c) in s.char_indices() {
        let k = urlish(c);
        if cur.is_some_and(|x| x != k) {
            parts.push(&s[start..i]);
            start = i;
        }
        cur = Some(k);
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

/// A `Token="..."` field (a Jellyfin authorization header) keeps its name only.
fn redact_quoted_token(line: &str) -> String {
    const KEY: &str = "token=\"";
    let mut out = line.to_string();
    let mut from = 0;
    while let Some(i) = out[from..].to_ascii_lowercase().find(KEY).map(|i| i + from) {
        let start = i + KEY.len();
        let end = out[start..].find('"').map_or(out.len(), |e| start + e);
        out.replace_range(start..end, "<token>");
        from = start + "<token>".len();
    }
    out
}

fn redact_word(w: &str) -> String {
    let mut w = w.to_string();
    // Tokens and keys in a query string, any case.
    for key in ["x-plex-token=", "api_key=", "apikey=", "accesstoken="] {
        let lower = w.to_ascii_lowercase();
        if let Some(i) = lower.find(key) {
            let from = i + key.len();
            let end = w[from..].find(['&', '#']).map_or(w.len(), |e| from + e);
            w.replace_range(from..end, "<token>");
        }
    }
    // plex.direct host names (they hold the server's address and id).
    while let Some(i) = w.to_ascii_lowercase().find(".plex.direct") {
        let start = w[..i].rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.')).map_or(0, |p| p + 1);
        w.replace_range(start..i + ".plex.direct".len(), "<server>");
    }
    redact_ipv4(&w)
}

fn redact_ipv4(w: &str) -> String {
    let b = w.as_bytes();
    let mut out = String::with_capacity(w.len());
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b'.')) {
            let mut j = i;
            let mut groups = 0;
            loop {
                let s = j;
                while j < b.len() && b[j].is_ascii_digit() && j - s < 3 {
                    j += 1;
                }
                if j == s {
                    break;
                }
                groups += 1;
                if groups < 4 && j < b.len() && b[j] == b'.' && j + 1 < b.len() && b[j + 1].is_ascii_digit() {
                    j += 1;
                } else {
                    break;
                }
            }
            if groups == 4 && !(j < b.len() && b[j].is_ascii_digit()) {
                out.push_str("<ip>");
                i = j;
                continue;
            }
        }
        let c = w[i..].chars().next().unwrap_or_default();
        out.push(c);
        i += c.len_utf8().max(1);
    }
    out
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
    fn tokens_servers_and_addresses_never_reach_the_file() {
        let line = "GET https://10-0-0-5.example.plex.direct:32400/library?X-Plex-Token=abcDEF123&x=1 failed, local 10.0.0.5, version 1.2.3";
        let r = super::redact(line);
        assert!(!r.contains("abcDEF123") && !r.contains("plex.direct") && !r.contains("10.0.0.5"), "{r}");
        assert!(r.contains("https://<server>:32400/library?X-Plex-Token=<token>&x=1"), "{r}");
        assert!(r.contains("local <ip>") && r.contains("version 1.2.3"), "{r}");
        let jf = super::redact("auth MediaBrowser Client=\"Platter\", Token=\"jf-secret\" and /socket?api_key=k2&deviceId=d");
        assert!(!jf.contains("jf-secret") && !jf.contains("k2"), "{jf}");
        assert!(jf.contains("Token=\"<token>\"") && jf.contains("api_key=<token>&deviceId=d"), "{jf}");
        assert_eq!(super::redact("Café ok
"), "Café ok
");
    }

    #[test]
    fn lines_before_the_file_opens_go_nowhere() {
        // FILE may be set by another test in this process; either way a
        // write reports the whole line taken.
        assert_eq!(LogFile.write(b"early\n").unwrap(), 6);
    }
}
