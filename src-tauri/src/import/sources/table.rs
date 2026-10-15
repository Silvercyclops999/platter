//! Any CSV or M3U list: transfer tool exports and lists people make.
//!
//! CSV: the delimiter (comma, semicolon, tab or bar) is found from the
//! header line, quotes follow the usual rules (doubled quotes, line breaks
//! inside quotes), and columns are found by their names: artist, title,
//! album, length, ISRC and playlist, under the names exports commonly use.
//! Rows are grouped into playlists by the playlist column, else the file
//! is one playlist named after it. A playlist named like a liked list
//! ("Liked Songs", "Favorites", "Loved tracks") becomes the liked songs.
//!
//! M3U: `#EXTINF:<seconds>,Artist - Title` lines, else "Artist - Title"
//! taken from each entry's file name. `#PLAYLIST:` names the list.

use super::Parser;
use crate::error::{AppError, AppResult};
use crate::import::files::{InputFile, decode_text};
use crate::import::{Parsed, PlaylistIn, SongIn};

pub struct Table;

impl Parser for Table {
    fn parse(&self, files: &[InputFile]) -> AppResult<Parsed> {
        let mut out = Parsed::default();
        let mut usable = 0;
        let mut first_err = None;
        for f in files {
            let text = decode_text(&f.bytes);
            let ext = f.ext();
            let res = if matches!(ext.as_str(), "m3u" | "m3u8") || text.trim_start().starts_with("#EXTM3U") {
                Ok(parse_m3u(&text, &f.stem()))
            } else if matches!(ext.as_str(), "csv" | "tsv" | "txt") {
                parse_csv(&text, &f.stem())
            } else {
                out.skipped.unknown_files += 1;
                continue;
            };
            match res {
                Ok(p) => {
                    usable += 1;
                    out.absorb(p);
                }
                Err(e) => {
                    out.skipped.unknown_files += 1;
                    first_err.get_or_insert(e);
                }
            }
        }
        if usable == 0 {
            return Err(first_err.unwrap_or_else(|| AppError::Invalid("No CSV or M3U files were found there.".into())));
        }
        Ok(out)
    }
}

/// Names a liked list goes by, folded to letters and digits.
const LIKED_NAMES: &[&str] = &[
    "likedsongs",
    "liked",
    "likes",
    "favorites",
    "favourites",
    "favoritesongs",
    "favouritesongs",
    "myfavorites",
    "lovedtracks",
    "loved",
    "favoritetracks",
    "mylikes",
    "likedmusic",
];

/// Lower case letters and digits only: "Track Name" -> "trackname".
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub fn is_liked_name(name: &str) -> bool {
    LIKED_NAMES.contains(&squash(name).as_str())
}

/// Puts rows into playlists (or the liked list) in the order they came.
fn group(rows: Vec<(Option<String>, SongIn)>, fallback: &str, out: &mut Parsed) {
    let mut lists: Vec<PlaylistIn> = Vec::new();
    for (name, song) in rows {
        let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| fallback.to_string());
        if is_liked_name(&name) {
            out.liked.get_or_insert_with(Vec::new).push(song);
            continue;
        }
        match lists.iter_mut().find(|p| p.name == name) {
            Some(p) => p.songs.push(song),
            None => lists.push(PlaylistIn { name, songs: vec![song] }),
        }
    }
    out.playlists.extend(lists);
}

// ------------------------------------------------------------------ CSV

/// Splits CSV text into rows of fields.
pub fn split_csv(text: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.trim().is_empty() => {
                field.clear();
                quoted = true;
            }
            c if c == delim => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                any = false;
            }
            c => field.push(c),
        }
    }
    if any {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

/// The delimiter that splits the header line into the most fields
/// (outside quotes).
pub fn detect_delimiter(text: &str) -> char {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut best = (',', 0);
    for d in [',', ';', '\t', '|'] {
        let mut n = 0;
        let mut quoted = false;
        for c in first.chars() {
            if c == '"' {
                quoted = !quoted;
            } else if c == d && !quoted {
                n += 1;
            }
        }
        if n > best.1 {
            best = (d, n);
        }
    }
    best.0
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    pub artist: Option<usize>,
    /// The artist column lists several names ("Artist Name(s)").
    pub artist_many: bool,
    pub album_artist: Option<usize>,
    pub title: Option<usize>,
    pub album: Option<usize>,
    pub duration: Option<usize>,
    /// The length column is in milliseconds by its name.
    pub duration_ms: bool,
    pub isrc: Option<usize>,
    pub playlist: Option<usize>,
}

const TITLE: &[&str] = &["trackname", "title", "track", "song", "songname", "songtitle", "tracktitle", "name", "trackname1"];
const ARTIST: &[&str] = &["artistname", "artist", "artists", "artistnames", "artistnames1", "performer", "performers", "creator", "trackartist"];
const ALBUM_ARTIST: &[&str] = &["albumartist", "albumartistname", "albumartistnames"];
const ALBUM: &[&str] = &["album", "albumname", "albumtitle", "release", "releasename", "record"];
const DURATION: &[&str] = &["duration", "durationms", "length", "time", "tracklength", "trackduration", "trackdurationms", "durationseconds", "lengthms", "durations", "playtime"];
const ISRC: &[&str] = &["isrc", "trackisrc", "isrccode"];
const PLAYLIST: &[&str] = &["playlist", "playlistname", "playlisttitle", "list", "listname"];

/// Finds the columns by their header names.
pub fn detect_columns(header: &[String]) -> Columns {
    let names: Vec<String> = header.iter().map(|h| squash(h)).collect();
    let find = |want: &[&str], taken: &[Option<usize>]| -> Option<usize> {
        // Exact names first, in the order listed.
        for w in want {
            if let Some(i) = names.iter().position(|n| n == w)
                && !taken.contains(&Some(i))
            {
                return Some(i);
            }
        }
        None
    };
    let mut c = Columns { isrc: find(ISRC, &[]), ..Default::default() };
    c.playlist = find(PLAYLIST, &[c.isrc]);
    c.album_artist = find(ALBUM_ARTIST, &[c.isrc, c.playlist]);
    c.artist = find(ARTIST, &[c.isrc, c.playlist, c.album_artist]);
    c.album = find(ALBUM, &[c.isrc, c.playlist, c.album_artist, c.artist]);
    c.title = find(TITLE, &[c.isrc, c.playlist, c.album_artist, c.artist, c.album]);
    c.duration = find(DURATION, &[c.isrc, c.playlist, c.album_artist, c.artist, c.album, c.title]);
    // Looser: a name that contains the word ("Song Title (Service)").
    let taken = |c: &Columns| [c.isrc, c.playlist, c.album_artist, c.artist, c.album, c.title, c.duration];
    let contains = |word: &str, not: &[&str], taken: [Option<usize>; 7]| {
        names
            .iter()
            .enumerate()
            .find(|(i, n)| n.contains(word) && !not.iter().any(|x| n.contains(x)) && !taken.contains(&Some(*i)))
            .map(|(i, _)| i)
    };
    if c.isrc.is_none() {
        c.isrc = contains("isrc", &[], taken(&c));
    }
    if c.artist.is_none() {
        c.artist = contains("artist", &["album", "id", "url", "uri", "genre"], taken(&c));
    }
    if c.title.is_none() {
        c.title = contains("title", &["album", "playlist", "id"], taken(&c)).or_else(|| contains("track", &["id", "url", "uri", "number", "no", "count", "artist", "album"], taken(&c)));
    }
    if c.album.is_none() {
        c.album = contains("album", &["artist", "id", "url", "uri", "type", "date", "image"], taken(&c));
    }
    if c.duration.is_none() {
        c.duration = contains("duration", &[], taken(&c));
    }
    if c.playlist.is_none() {
        c.playlist = contains("playlist", &["id", "url", "uri", "description"], taken(&c));
    }
    if c.artist.is_none() {
        c.artist = c.album_artist;
    }
    if let Some(i) = c.duration {
        c.duration_ms = names[i].ends_with("ms") || names[i].contains("millis");
    }
    if let Some(i) = c.artist {
        c.artist_many = names[i].contains("artists") || names[i].contains("names");
    }
    c
}

/// A length: "3:45", "1:02:03", or a number (milliseconds when the column
/// says so or the number is large, else seconds).
pub fn parse_duration(s: &str, ms_column: bool) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.contains(':') {
        let mut total = 0u64;
        for part in s.split(':') {
            let v: f64 = part.trim().parse().ok()?;
            total = total * 60 + v as u64;
        }
        return (total > 0).then_some(total * 1000);
    }
    let v: f64 = s.replace(',', ".").parse().ok()?;
    if v <= 0.0 {
        return None;
    }
    if ms_column || v >= 10_000.0 { Some(v as u64) } else { Some((v * 1000.0) as u64) }
}

/// The first of several artists in one field ("A, B" or "A; B").
fn first_artist(s: &str) -> &str {
    s.split([';', ',']).next().unwrap_or(s).trim()
}

pub fn parse_csv(text: &str, file_stem: &str) -> AppResult<Parsed> {
    let delim = detect_delimiter(text);
    let mut rows = split_csv(text, delim).into_iter();
    let Some(header) = rows.next() else {
        return Err(AppError::Invalid("That file is empty.".into()));
    };
    let cols = detect_columns(&header);
    let (Some(ti), Some(ai)) = (cols.title, cols.artist) else {
        return Err(AppError::Invalid(
            "Couldn't find artist and title columns in that file. It needs a header row naming them.".into(),
        ));
    };
    let mut out = Parsed::default();
    let mut grouped = Vec::new();
    let cell = |r: &[String], i: Option<usize>| i.and_then(|i| r.get(i)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    for r in rows {
        let title = cell(&r, Some(ti));
        let artist = cell(&r, Some(ai)).or_else(|| cell(&r, cols.album_artist));
        let (Some(title), Some(artist)) = (title, artist) else {
            out.skipped.incomplete_rows += 1;
            continue;
        };
        let artist = if cols.artist_many { first_artist(&artist).to_string() } else { artist };
        let song = SongIn {
            artist,
            title,
            album: cell(&r, cols.album),
            duration_ms: cell(&r, cols.duration).and_then(|d| parse_duration(&d, cols.duration_ms)),
            isrc: cell(&r, cols.isrc).and_then(|v| crate::tags::clean_isrc(&v)),
            service_id: None,
        };
        grouped.push((cell(&r, cols.playlist), song));
    }
    group(grouped, file_stem, &mut out);
    Ok(out)
}

// ------------------------------------------------------------------ M3U

/// "Artist - Title" split at the first " - ".
fn artist_title(s: &str) -> Option<(String, String)> {
    let (a, t) = s.split_once(" - ")?;
    let (a, t) = (a.trim(), t.trim());
    (!a.is_empty() && !t.is_empty()).then(|| (a.to_string(), t.to_string()))
}

pub fn parse_m3u(text: &str, file_stem: &str) -> Parsed {
    let mut out = Parsed::default();
    let mut name = None;
    let mut pending: Option<(Option<u64>, String)> = None;
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.eq_ignore_ascii_case("#EXTM3U") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#PLAYLIST:") {
            name = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (secs, label) = rest.split_once(',').unwrap_or((rest, ""));
            // "#EXTINF:215 tvg-id=..., label": the number comes first.
            let secs: Option<u64> = secs.split_whitespace().next().and_then(|s| s.parse::<i64>().ok()).filter(|s| *s > 0).map(|s| s as u64 * 1000);
            pending = Some((secs, label.trim().to_string()));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        // An entry: a path or address.
        let (secs, label) = pending.take().unwrap_or((None, String::new()));
        let from_label = artist_title(&label);
        let from_path = || {
            let base = line.rsplit(['/', '\\']).next().unwrap_or(line);
            let base = percent_encoding::percent_decode_str(base).decode_utf8_lossy().into_owned();
            let stem = base.rsplit_once('.').map_or(base.as_str(), |(s, _)| s).to_string();
            // "01 - Artist - Title" and "01. Title" both happen; only take
            // "Artist - Title" when there are exactly two parts after a
            // leading track number.
            let stem = stem.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches(['.', ' ', '-']).trim().to_string();
            artist_title(&stem)
        };
        match from_label.or_else(from_path) {
            Some((artist, title)) => rows.push((
                None,
                SongIn { artist, title, album: None, duration_ms: secs, isrc: None, service_id: None },
            )),
            None => out.skipped.incomplete_rows += 1,
        }
    }
    group(rows, name.as_deref().unwrap_or(file_stem), &mut out);
    out
}
