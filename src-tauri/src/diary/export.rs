//! The diary as a plain text journal or a CSV table.
//!
//! Dates are local to this computer: [`local_times`] asks SQLite, which
//! knows the time zone and daylight saving, so the formatting here stays pure.

use rusqlite::Connection;
use serde::Deserialize;

use super::DiaryItem;
use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Text,
    Csv,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Text => "txt",
            ExportFormat::Csv => "csv",
        }
    }
}

/// A local date and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

impl LocalTime {
    fn month_name(&self) -> &'static str {
        MONTHS[(self.month.clamp(1, 12) - 1) as usize]
    }
    fn iso_date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
    fn hm(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }
}

/// Local times for unix ms values, in this computer's time zone.
pub fn local_times(c: &Connection, at: &[i64]) -> AppResult<Vec<LocalTime>> {
    let mut st = c.prepare_cached("SELECT strftime('%Y %m %d %H %M', ?1 / 1000, 'unixepoch', 'localtime')")?;
    let mut out = Vec::with_capacity(at.len());
    for ms in at {
        let s: String = st.query_row([ms], |r| r.get(0))?;
        let n: Vec<i64> = s.split(' ').filter_map(|p| p.parse().ok()).collect();
        let g = |i: usize| n.get(i).copied().unwrap_or(0);
        out.push(LocalTime { year: g(0) as i32, month: g(1) as u32, day: g(2) as u32, hour: g(3) as u32, minute: g(4) as u32 });
    }
    Ok(out)
}

/// "4.5/5", or empty with no rating.
fn rating_text(r: Option<f32>) -> String {
    match r {
        Some(r) if r.fract() == 0.0 => format!("{}/5", r as u32),
        Some(r) => format!("{r:.1}/5"),
        None => String::new(),
    }
}

/// Where it was heard: "On Den PC, headphones (WH-1000XM5)".
pub fn where_text(item: &DiaryItem) -> String {
    let e = &item.entry;
    let out = match (&e.output, item.listened_on) {
        // "Headphones (WH-1000XM5)" already says so.
        (Some(o), Some(k)) if o.to_lowercase().contains(k.label().trim_end_matches('s')) => Some(o.clone()),
        (Some(o), Some(k)) => Some(format!("{} ({o})", k.label())),
        (Some(o), None) => Some(o.clone()),
        (None, _) => None,
    };
    match (&e.device, out) {
        (Some(d), Some(o)) => format!("On {d}, {o}"),
        (Some(d), None) => format!("On {d}"),
        (None, Some(o)) => format!("On {o}"),
        (None, None) => String::new(),
    }
}

/// How much was played: "Played 9 of 10 songs, 41 min".
pub fn played_text(item: &DiaryItem) -> String {
    let e = &item.entry;
    if !e.auto {
        return "Logged by hand".into();
    }
    let mut s = if e.songs_played >= e.songs_total {
        format!("Played all {} songs", e.songs_total)
    } else {
        format!("Played {} of {} songs", e.songs_played, e.songs_total)
    };
    if e.minutes > 0 {
        s.push_str(&format!(", {} min", e.minutes));
    }
    s
}

/// The diary as a journal: newest month first, one block per entry.
/// `items` are newest first, each with its local time; `today` heads it.
pub fn to_text(items: &[(DiaryItem, LocalTime)], today: LocalTime) -> String {
    let mut out = String::new();
    out.push_str("Listening diary\n");
    let n = items.len();
    out.push_str(&format!(
        "Exported from Platter on {} {} {}. {} {}.\n",
        today.day,
        today.month_name(),
        today.year,
        n,
        if n == 1 { "entry" } else { "entries" }
    ));
    let mut month: Option<(i32, u32)> = None;
    for (item, t) in items {
        let e = &item.entry;
        if month != Some((t.year, t.month)) {
            month = Some((t.year, t.month));
            let head = format!("{} {}", t.month_name(), t.year);
            out.push_str(&format!("\n{head}\n{}\n", "-".repeat(head.chars().count())));
        }
        let mut title = format!("{}, {}", e.title, e.artist);
        if let Some(y) = e.year {
            title.push_str(&format!(" ({y})"));
        }
        let rating = rating_text(e.rating);
        out.push_str(&format!("\n{} {:>2}  {title}", &t.month_name()[..3], t.day));
        if !rating.is_empty() {
            out.push_str(&format!("  {rating}"));
        }
        out.push('\n');
        let mut line = played_text(item);
        let place = where_text(item);
        if !place.is_empty() {
            line.push_str(&format!(". {place}"));
        }
        out.push_str(&format!("        {line}.\n"));
        for l in e.note.lines() {
            out.push_str(&format!("        {l}\n"));
        }
    }
    out
}

/// A CSV field: quoted when needed, and a leading = + - @ is kept as text
/// so a spreadsheet never runs it as a formula.
fn csv_field(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_owned() };
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

pub const CSV_HEADER: &str =
    "date,time,album,artist,year,rating,songs_played,songs_total,minutes,device,output,listened_on,source,note";

/// The diary as a table, one row per entry, newest first. Starts with a
/// byte order mark so spreadsheets read accents right.
pub fn to_csv(items: &[(DiaryItem, LocalTime)]) -> String {
    let mut out = String::from("\u{feff}");
    out.push_str(CSV_HEADER);
    out.push_str("\r\n");
    for (item, t) in items {
        let e = &item.entry;
        let row = [
            t.iso_date(),
            t.hm(),
            csv_field(&e.title),
            csv_field(&e.artist),
            e.year.map(|y| y.to_string()).unwrap_or_default(),
            e.rating.map(|r| if r.fract() == 0.0 { format!("{}", r as u32) } else { format!("{r:.1}") }).unwrap_or_default(),
            if e.auto { e.songs_played.to_string() } else { String::new() },
            if e.auto { e.songs_total.to_string() } else { String::new() },
            if e.auto { e.minutes.to_string() } else { String::new() },
            csv_field(e.device.as_deref().unwrap_or("")),
            csv_field(e.output.as_deref().unwrap_or("")),
            item.listened_on.map(|k| k.label()).unwrap_or("").to_owned(),
            if e.auto { "played".into() } else { "logged".into() },
            csv_field(&e.note),
        ];
        out.push_str(&row.join(","));
        out.push_str("\r\n");
    }
    out
}
