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

/// Removes what mustn't be in a shareable log: `X-Plex-Token`, `api_key`,
/// `apiKey` and `Token="..."` values, a Subsonic sign-in's `t`, `s` and `p`
/// parameters, `*.plex.direct` host names and IPv4 addresses.
pub fn redact(line: &str) -> String {
    let line = redact_quoted_token(line);
    let line = redact_header_values(&line);
    let line = redact_home_folders(&line);
    let mut out = String::with_capacity(line.len());
    for word in split_keep(&line) {
        out.push_str(&redact_word(word));
    }
    out
}

/// Header-form secrets (`X-Api-Key: abc`, `X-Emby-Token=abc`,
/// `Authorization: Bearer abc`): the value goes.
fn redact_header_values(line: &str) -> String {
    const NAMES: [&str; 6] = ["x-api-key", "x-emby-token", "x-mediabrowser-token", "x-plex-token", "authorization", "x-emby-authorization"];
    let mut out = line.to_string();
    for name in NAMES {
        let mut from = 0;
        loop {
            let lower = out.to_ascii_lowercase();
            let Some(i) = lower[from..].find(name).map(|i| i + from) else { break };
            let mut j = i + name.len();
            let rest = out.as_bytes();
            // `name: value`, `name = value` or `"name": "value"`.
            while j < rest.len() && matches!(rest[j], b'"' | b'\'') {
                j += 1;
            }
            if j >= rest.len() || !matches!(rest[j], b':' | b'=') {
                from = i + name.len();
                continue;
            }
            j += 1;
            while j < rest.len() && matches!(rest[j], b' ' | b'"' | b'\'') {
                j += 1;
            }
            // A scheme word (Bearer, MediaBrowser) stays; its value goes.
            let word_end = |k: usize| out[k..].find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '&')).map_or(out.len(), |e| k + e);
            let first = word_end(j);
            let start = if name == "authorization" && out[j..first].chars().all(|c| c.is_ascii_alphabetic()) && first < out.len() && out[first..].starts_with(' ') {
                first + 1
            } else {
                j
            };
            let end = word_end(start);
            if end > start && !out[start..end].starts_with('<') {
                out.replace_range(start..end, "<token>");
            }
            from = start + 1;
        }
    }
    out
}

/// A home folder in a path names the person: `C:\Users\<user>\...`,
/// `/Users/<user>/...`, `/home/<user>/...`.
fn redact_home_folders(line: &str) -> String {
    let mut out = line.to_string();
    for marker in ["\\users\\", "/users/", "/home/"] {
        let mut from = 0;
        loop {
            let lower = out.to_ascii_lowercase();
            let Some(i) = lower[from..].find(marker).map(|i| i + from) else { break };
            let start = i + marker.len();
            // Windows user folders can hold spaces ("Adam Smith").
            let stops: &[char] = if marker.starts_with('\\') { &['\\', '/', '"', '\''] } else { &['/', ' ', '"', '\''] };
            let end = out[start..].find(stops).map_or(out.len(), |e| start + e);
            if end > start && &out[start..end] != "<user>" && !out[start..end].eq_ignore_ascii_case("public") {
                out.replace_range(start..end, "<user>");
            }
            from = start + 1;
        }
    }
    out
}

/// Hosts whose names say nothing about the user, kept for reading logs.
const PUBLIC_HOSTS: [&str; 12] = [
    "plex.tv",
    "ntfy.sh",
    "github.com",
    "githubusercontent.com",
    "musicbrainz.org",
    "listenbrainz.org",
    "last.fm",
    "audioscrobbler.com",
    "discogs.com",
    "wikipedia.org",
    "wikidata.org",
    "coverartarchive.org",
];

/// `scheme://host[:port]`: a host that isn't a well-known public service
/// becomes `<host>` (a server's own name says where it is).
fn redact_url_host(w: &str) -> String {
    let Some(i) = w.find("://") else { return w.to_string() };
    let start = i + 3;
    let end = w[start..].find(['/', '?', '#']).map_or(w.len(), |e| start + e);
    let authority = &w[start..end];
    let host_end = if authority.starts_with('[') { authority.find(']').map_or(authority.len(), |e| e + 1) } else { authority.rfind(':').unwrap_or(authority.len()) };
    let host = authority[..host_end].rsplit('@').next().unwrap_or("").to_ascii_lowercase();
    if host.is_empty() || host.starts_with('<') || PUBLIC_HOSTS.iter().any(|p| host == *p || host.ends_with(&format!(".{p}"))) {
        return w.to_string();
    }
    format!("{}<host>{}", &w[..start], &authority[host_end..]) + &w[end..]
}

/// IPv6 addresses (`fe80::1`, `[2001:db8::5]:32400`) become `<ip>`.
fn redact_ipv6(w: &str) -> String {
    let mut out = String::with_capacity(w.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        let bare = run.trim_matches(|c| c == '[' || c == ']');
        if bare.matches(':').count() >= 2 && bare.parse::<std::net::Ipv6Addr>().is_ok() {
            out.push_str("<ip>");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in w.chars() {
        if c.is_ascii_hexdigit() || c == ':' || c == '.' || c == '%' {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
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
    for key in ["x-plex-token=", "api_key=", "apikey=", "accesstoken=", "access_token=", "x-emby-token=", "authtoken="] {
        let lower = w.to_ascii_lowercase();
        if let Some(i) = lower.find(key) {
            let from = i + key.len();
            let end = w[from..].find(['&', '#']).map_or(w.len(), |e| from + e);
            if end > from {
                w.replace_range(from..end, "<token>");
            }
        }
    }
    // A Subsonic sign-in: the user, the password token, its salt, or a
    // password, as query parameters of their own (`?u=`, `&t=`, `&s=`, `&p=`).
    for key in ["u=", "t=", "s=", "p="] {
        let mut from = 0;
        loop {
            let lower = w.to_ascii_lowercase();
            let Some(i) = lower[from..].find(key).map(|i| i + from) else { break };
            let at_param = i > 0 && matches!(w.as_bytes()[i - 1], b'?' | b'&');
            let start = i + key.len();
            if !at_param {
                from = start;
                continue;
            }
            let end = w[start..].find(['&', '#']).map_or(w.len(), |e| start + e);
            if end == start {
                from = start;
                continue;
            }
            w.replace_range(start..end, "<token>");
            from = start + "<token>".len();
        }
    }
    // plex.direct host names (they hold the server's address and id).
    while let Some(i) = w.to_ascii_lowercase().find(".plex.direct") {
        let start = w[..i].rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.')).map_or(0, |p| p + 1);
        w.replace_range(start..i + ".plex.direct".len(), "<server>");
    }
    redact_ipv6(&redact_ipv4(&redact_url_host(&w)))
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
        let os = super::redact("GET /rest/stream?id=s1&format=raw&u=me&t=abcd1234ef&s=c19b2d&v=1.16.1 x?apiKey=KEY9&p=pw1&f=json");
        for secret in ["abcd1234ef", "c19b2d", "KEY9", "pw1"] {
            assert!(!os.contains(secret), "{os}");
        }
        assert!(os.contains("format=raw&u=<token>&t=<token>&s=<token>&v=1.16.1") && os.contains("apiKey=<token>&p=<token>&f=json"), "{os}");
        assert_eq!(super::redact("start=5&last=2 sets=1"), "start=5&last=2 sets=1", "only whole parameter names");
        assert_eq!(super::redact("Café ok
"), "Café ok
");
    }

    #[test]
    fn the_gaps_found_later_are_closed_too() {
        let r = super::redact("peer fe80::1c2d:3e4f, server [2001:db8::5]:32400, at 12:30:45 v2.0.1");
        assert!(!r.contains("fe80") && !r.contains("2001:db8"), "{r}");
        assert!(r.contains("at 12:30:45 v2.0.1"), "{r}");
        let h = super::redact("GET https://music.myhouse.example:4533/rest/ping and http://jellyfin.local/x then https://plex.tv/api/v2/user");
        assert!(!h.contains("myhouse") && !h.contains("jellyfin.local"), "{h}");
        assert!(h.contains("https://<host>:4533/rest/ping") && h.contains("https://plex.tv/api/v2/user"), "{h}");
        let hd = super::redact(r#"headers X-Emby-Token: em-secret, X-Api-Key=lk-secret, "X-Plex-Token": "px-secret", Authorization: Bearer au-secret"#);
        for secret in ["em-secret", "lk-secret", "px-secret", "au-secret"] {
            assert!(!hd.contains(secret), "{hd}");
        }
        assert!(hd.contains("Bearer <token>"), "{hd}");
        let p = super::redact(r"couldn't open C:\Users\Adam Smith\Music\a.flac or /Users/adam/Music or /home/sam/x");
        assert!(!p.contains("Adam") && !p.contains("Smith") && !p.contains("adam") && !p.contains("sam/"), "{p}");
        assert!(p.contains(r"C:\Users\<user>\Music"), "{p}");
    }

    #[test]
    fn lines_before_the_file_opens_go_nowhere() {
        // FILE may be set by another test in this process; either way a
        // write reports the whole line taken.
        assert_eq!(LogFile.write(b"early\n").unwrap(), 6);
    }
}
