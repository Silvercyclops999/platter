//! Local crash reports. A panic writes a plain text file to
//! `<app data>/crash-reports/`. Nothing is ever sent anywhere; a future
//! "send report" button would ask first. On relaunch the player restores the
//! queue and position from `queue.json` (written a moment after every
//! change), so a crash shows nothing to the user.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Reports kept; older ones are deleted.
pub const KEEP: usize = 10;

pub fn reports_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("crash-reports")
}

/// Web addresses lose their query (`?...`), where keys and ids ride.
fn strip_queries(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let end_of_url = rest[i..].find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>')).map_or(rest.len(), |e| i + e);
        let url = &rest[..end_of_url];
        out.push_str(url.split_once('?').map_or(url, |(a, _)| a));
        rest = &rest[end_of_url..];
    }
    out.push_str(rest);
    out
}

/// The report text. No tokens or library data: only the panic message,
/// where it happened, the version and a backtrace, passed through the log
/// file's redaction (tokens, server names, addresses, home folders) with
/// web addresses cut at their query.
pub fn report_text(message: &str, location: Option<&str>, backtrace: &str) -> String {
    crate::platform::logs::redact(&strip_queries(&report_raw(message, location, backtrace)))
}

fn report_raw(message: &str, location: Option<&str>, backtrace: &str) -> String {
    format!(
        "Platter {} crashed\nos: {} {}\nwhere: {}\nmessage: {}\n\nbacktrace:\n{}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        location.unwrap_or("unknown"),
        message,
        backtrace
    )
}

/// Write a report and prune old ones. Returns the file written.
pub fn write_report(dir: &Path, text: &str, now_ms: u128) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("crash-{now_ms}.txt"));
    std::fs::write(&path, text)?;
    let mut all: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    all.sort();
    let extra = all.len().saturating_sub(KEEP);
    for old in &all[..extra] {
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

/// Install the panic hook (keeps the default hook's stderr output too).
pub fn install(data_dir: &Path) {
    let dir = reports_dir(data_dir);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line()));
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = write_report(&dir, &report_text(&message, location.as_deref(), &bt), now);
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_carry_no_keys_addresses_or_names() {
        let text = report_text(
            "request to http://10.20.30.40:32400/library/parts/1?X-Plex-Token=abc123&download=1 failed",
            Some(r"C:\Users\listener\src\x.rs:1"),
            "frames",
        );
        for leak in ["abc123", "10.20.30.40", "download=1", "listener"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
        assert!(text.contains("/library/parts/1 failed"), "{text}");
    }

    #[test]
    fn writes_report_and_keeps_the_last_few() {
        let d = tempfile::tempdir().unwrap();
        let dir = reports_dir(d.path());
        let text = report_text("index out of bounds", Some("src/x.rs:1"), "frames");
        assert!(text.contains("index out of bounds"));
        assert!(text.contains("src/x.rs:1"));
        for i in 0..(KEEP as u128 + 3) {
            write_report(&dir, &text, 1_000 + i).unwrap();
        }
        let n = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(n, KEEP);
        assert!(!dir.join("crash-1000.txt").exists());
        assert!(dir.join(format!("crash-{}.txt", 1_000 + KEEP as u128 + 2)).exists());
    }
}
