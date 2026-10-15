//! The import sources and their parsers.
//!
//! A source is what the user turns on in Settings (each has an id, kept in
//! the `importSources` setting). Each source reads its files with a
//! [`Parser`]. Parsers are tolerant: fields they don't know are ignored,
//! and they accept the other names a field has gone by.

pub mod streaming_export;
pub mod table;

use super::Parsed;
use super::files::InputFile;
use crate::error::AppResult;

/// Turns the files of one pick into playlists, likes, albums and plays.
pub trait Parser: Send + Sync {
    fn parse(&self, files: &[InputFile]) -> AppResult<Parsed>;
}

/// An import source the user can turn on.
#[derive(Clone, Copy)]
pub struct Source {
    /// Kept in settings and in the import tables.
    pub id: &'static str,
    pub parser: &'static dyn Parser,
}

static STREAMING: streaming_export::StreamingExport = streaming_export::StreamingExport;
static TABLE: table::Table = table::Table;

/// Every source, in the order Settings lists them. Their names and help
/// text are in the UI's strings table.
pub const SOURCES: &[Source] = &[
    // A streaming service's account data and extended listening history.
    Source { id: "streaming", parser: &STREAMING },
    // Files from a transfer tool (other services).
    Source { id: "transfer", parser: &TABLE },
    // Any CSV or M3U list.
    Source { id: "lists", parser: &TABLE },
];

pub fn source(id: &str) -> Option<&'static Source> {
    SOURCES.iter().find(|s| s.id == id)
}

// ------------------------------------------------------------------ times

/// Days since 1970-01-01 for a calendar date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix ms from the time stamps history files use: "2023-04-05T06:07:08Z",
/// with fractions or an offset, or "2023-04-05 06:07" (taken as UTC).
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 16 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    if b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b' ') || b[13] != b':' {
        return None;
    }
    let (y, mo, d, h, mi) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?);
    let mut sec = 0;
    let mut i = 16;
    if b.get(16) == Some(&b':') {
        sec = num(17..19)?;
        i = 19;
    }
    let mut ms = 0;
    if b.get(i) == Some(&b'.') {
        let digits: String = s[i + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
        i += 1 + digits.len();
        let mut frac = digits;
        frac.truncate(3);
        while frac.len() < 3 {
            frac.push('0');
        }
        ms = frac.parse().unwrap_or(0);
    }
    let mut offset_min = 0;
    match b.get(i) {
        None | Some(b'Z') | Some(b'z') => {}
        Some(b'+') | Some(b'-') => {
            let sign = if b[i] == b'-' { -1 } else { 1 };
            let oh: i64 = s.get(i + 1..i + 3)?.parse().ok()?;
            let rest = s.get(i + 3..).unwrap_or("").trim_start_matches(':');
            let om: i64 = rest.get(..2).and_then(|x| x.parse().ok()).unwrap_or(0);
            offset_min = sign * (oh * 60 + om);
        }
        _ => {}
    }
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    Some((((days * 24 + h) * 60 + mi - offset_min) * 60 + sec) * 1000 + ms)
}
