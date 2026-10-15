//! Eras: the ARCHIVE_ERA tag groups an artist's songs, released and
//! unreleased, into periods. Era cards on the artist page, the era filter
//! and the era page come from here, as does the album page's era banner.
//!
//! ARCHIVE_DATE is read in the formats docs/TAGGING.md pins: `YYYY`,
//! `YYYY-MM` and `YYYY-MM-DD`. Anything else is ignored.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::db::queries::{albums_by_ids, tracks_by_ids};
use crate::db::store::{ARTIST_COLS, row_artist};
use crate::error::{AppError, AppResult};
use crate::identity::fold;
use crate::model::{Album, Artist, Track};

/// An ARCHIVE_DATE: a year, maybe a month, maybe a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArchiveDate {
    pub year: i32,
    pub month: Option<u8>,
    pub day: Option<u8>,
}

impl ArchiveDate {
    /// Sorts by year, then month, then day (missing parts first).
    pub fn key(&self) -> i32 {
        self.year * 10_000 + self.month.unwrap_or(0) as i32 * 100 + self.day.unwrap_or(0) as i32
    }

    /// As written in the pinned form.
    pub fn text(&self) -> String {
        match (self.month, self.day) {
            (Some(m), Some(d)) => format!("{:04}-{m:02}-{d:02}", self.year),
            (Some(m), None) => format!("{:04}-{m:02}", self.year),
            _ => format!("{:04}", self.year),
        }
    }
}

/// `YYYY`, `YYYY-MM` or `YYYY-MM-DD` (surrounding spaces allowed). None for
/// anything else, including impossible months and days.
pub fn parse_date(s: &str) -> Option<ArchiveDate> {
    let s = s.trim();
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() > 3 {
        return None;
    }
    let num = |p: &str, len: usize| (p.len() == len && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse::<u32>().ok()).flatten();
    let year = num(parts[0], 4)? as i32;
    if !(1000..=9999).contains(&year) {
        return None;
    }
    let month = match parts.get(1) {
        Some(&p) =>Some(num(p, 2).filter(|m| (1..=12).contains(m))? as u8),
        None => None,
    };
    let day = match parts.get(2) {
        Some(&p) => {
            let d = num(p, 2)?;
            let max = match month? {
                2 => {
                    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
                    if leap { 29 } else { 28 }
                }
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            if !(1..=max).contains(&d) {
                return None;
            }
            Some(d as u8)
        }
        None => None,
    };
    Some(ArchiveDate { year, month, day })
}

/// One era of an artist.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Era {
    /// As tagged (the first spelling seen).
    pub name: String,
    /// Earliest and latest date: ARCHIVE_DATE when any song has one, else
    /// the albums' years.
    pub start: Option<String>,
    pub end: Option<String>,
    pub track_count: u32,
    pub album_count: u32,
    /// The cover of the era's most played album.
    pub cover: Option<String>,
    pub cover_album_id: Option<String>,
    /// For the era filter: the era's albums and songs.
    pub album_ids: Vec<String>,
    pub track_ids: Vec<String>,
}

struct Acc {
    name: String,
    tracks: Vec<String>,
    albums: Vec<String>,
    dates: Vec<ArchiveDate>,
    years: Vec<i32>,
    /// (plays, album id, thumb) of each album.
    album_plays: HashMap<String, (i64, Option<String>)>,
}

/// An artist's eras, oldest first (eras with no date or year last, by name).
pub fn artist_eras(c: &Connection, artist_id: &str, show_edits: bool) -> AppResult<Vec<Era>> {
    let mut st = c.prepare_cached(
        "SELECT t.id, t.album_id, f.era, f.date, al.year, coalesce(al.view_count, 0), al.thumb, f.flags
         FROM tracks t
         JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'
         LEFT JOIN albums al ON al.id = t.album_id
         WHERE t.artist_id = ?1 AND trim(coalesce(f.era, '')) <> ''
         ORDER BY t.album_id, t.disc_number, t.track_number",
    )?;
    let mut accs: Vec<Acc> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut rows = st.query([artist_id])?;
    while let Some(r) = rows.next()? {
        let flags: Option<String> = r.get(7)?;
        if !show_edits && crate::tags::is_edit(flags.as_deref()) {
            continue;
        }
        let era: String = r.get::<_, String>(2)?.trim().to_string();
        let key = fold::fold_text(&era);
        let i = *at.entry(key).or_insert_with(|| {
            accs.push(Acc { name: era.clone(), tracks: Vec::new(), albums: Vec::new(), dates: Vec::new(), years: Vec::new(), album_plays: HashMap::new() });
            accs.len() - 1
        });
        let a = &mut accs[i];
        let track: String = r.get(0)?;
        let album: String = r.get(1)?;
        a.tracks.push(track);
        if !a.albums.contains(&album) {
            a.albums.push(album.clone());
            if let Some(y) = r.get::<_, Option<i32>>(4)? {
                a.years.push(y);
            }
            a.album_plays.insert(album, (r.get(5)?, r.get(6)?));
        }
        if let Some(d) = r.get::<_, Option<String>>(3)?.as_deref().and_then(parse_date) {
            a.dates.push(d);
        }
    }
    let mut eras: Vec<(Option<i32>, Era)> = accs
        .into_iter()
        .map(|a| {
            let (start, end, key) = match (a.dates.iter().min(), a.dates.iter().max()) {
                (Some(lo), Some(hi)) => (Some(lo.text()), Some(hi.text()), Some(lo.key())),
                _ => match (a.years.iter().min(), a.years.iter().max()) {
                    (Some(lo), Some(hi)) => (Some(lo.to_string()), Some(hi.to_string()), Some(lo * 10_000)),
                    _ => (None, None, None),
                },
            };
            let cover = a
                .albums
                .iter()
                .map(|id| (id, &a.album_plays[id]))
                .max_by(|x, y| x.1.0.cmp(&y.1.0).then_with(|| y.0.cmp(x.0)))
                .map(|(id, (_, thumb))| (id.clone(), thumb.clone()));
            let era = Era {
                name: a.name,
                start,
                end,
                track_count: a.tracks.len() as u32,
                album_count: a.albums.len() as u32,
                cover: cover.as_ref().and_then(|c| c.1.clone()),
                cover_album_id: cover.map(|c| c.0),
                album_ids: a.albums,
                track_ids: a.tracks,
            };
            (key, era)
        })
        .collect();
    eras.sort_by(|a, b| match (a.0, b.0) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.1.name.cmp(&b.1.name)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()),
    });
    Ok(eras.into_iter().map(|e| e.1).collect())
}

/// One album or build on the era page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EraGroup {
    pub album: Album,
    /// The latest ARCHIVE_DATE among its songs in the era.
    pub date: Option<String>,
    pub edition: Option<String>,
    pub tracks: Vec<Track>,
}

/// The era page (`/artist/:id/era/:name`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EraPage {
    pub artist: Artist,
    pub era: Era,
    /// Newest build first.
    pub groups: Vec<EraGroup>,
}

/// The era named `name` (any case) of an artist, its songs by album.
pub fn era_page(c: &Connection, artist_id: &str, name: &str, show_edits: bool) -> AppResult<EraPage> {
    let artist = c
        .query_row(&format!("SELECT {ARTIST_COLS} FROM artists ar WHERE ar.id = ?1"), [artist_id], row_artist)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("artist {artist_id}")))?;
    let want = fold::fold_text(name.trim());
    let era = artist_eras(c, artist_id, show_edits)?
        .into_iter()
        .find(|e| fold::fold_text(&e.name) == want)
        .ok_or_else(|| AppError::NotFound(format!("era {name}")))?;
    let tracks = tracks_by_ids(c, &era.track_ids)?;
    let albums = albums_by_ids(c, &era.album_ids)?;
    let mut date_of = c.prepare_cached(
        "SELECT f.date FROM tracks t JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok' WHERE t.id = ?1",
    )?;
    let mut edition_of = c.prepare_cached("SELECT edition FROM album_tags WHERE album_id = ?1")?;
    let mut groups: Vec<(Option<i32>, EraGroup)> = Vec::new();
    for album in albums {
        let mut list: Vec<Track> = tracks.iter().filter(|t| t.album_id == album.id).cloned().collect();
        list.sort_by_key(|t| (t.disc_number, t.track_number));
        let mut latest: Option<ArchiveDate> = None;
        for t in &list {
            let d: Option<String> = date_of.query_row([&t.id], |r| r.get(0)).optional()?.flatten();
            if let Some(d) = d.as_deref().and_then(parse_date) {
                latest = Some(latest.map_or(d, |l| l.max(d)));
            }
        }
        let edition: Option<String> = edition_of.query_row([&album.id], |r| r.get(0)).optional()?.flatten();
        let key = latest.map(|d| d.key()).or(album.year.map(|y| y * 10_000));
        groups.push((key, EraGroup { date: latest.map(|d| d.text()), edition, tracks: list, album }));
    }
    groups.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| b.1.album.added_at.cmp(&a.1.album.added_at)).then_with(|| a.1.album.id.cmp(&b.1.album.id))
    });
    Ok(EraPage { artist, era, groups: groups.into_iter().map(|g| g.1).collect() })
}

/// The album page's era banner for an unreleased build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumArchive {
    pub era: Option<String>,
    pub date: Option<String>,
    pub edition: Option<String>,
    pub source: Option<String>,
}

/// The most common era, the latest date and the most common source among
/// an album's songs, with its edition. None when the album isn't unreleased
/// or nothing is tagged.
pub fn album_archive(c: &Connection, album_id: &str) -> AppResult<Option<AlbumArchive>> {
    let unreleased = super::status::album_status(c, album_id)?.is_some_and(|s| s.status.is_unreleased());
    if !unreleased {
        return Ok(None);
    }
    let mut st = c.prepare_cached(
        "SELECT f.era, f.date, f.source FROM tracks t JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'
         WHERE t.album_id = ?1",
    )?;
    let mut eras: Vec<(String, usize)> = Vec::new();
    let mut sources: Vec<(String, usize)> = Vec::new();
    let mut latest: Option<ArchiveDate> = None;
    let bump = |list: &mut Vec<(String, usize)>, v: Option<String>| {
        if let Some(v) = v.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
            match list.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&v)) {
                Some(e) => e.1 += 1,
                None => list.push((v, 1)),
            }
        }
    };
    let mut rows = st.query([album_id])?;
    while let Some(r) = rows.next()? {
        bump(&mut eras, r.get(0)?);
        if let Some(d) = r.get::<_, Option<String>>(1)?.as_deref().and_then(parse_date) {
            latest = Some(latest.map_or(d, |l| l.max(d)));
        }
        bump(&mut sources, r.get(2)?);
    }
    let top = |list: Vec<(String, usize)>| list.into_iter().max_by_key(|x| x.1).map(|x| x.0);
    let edition: Option<String> = c
        .query_row("SELECT edition FROM album_tags WHERE album_id = ?1", [album_id], |r| r.get(0))
        .optional()?
        .flatten();
    let out = AlbumArchive { era: top(eras), date: latest.map(|d| d.text()), edition, source: top(sources) };
    Ok((out.era.is_some() || out.date.is_some() || out.edition.is_some() || out.source.is_some()).then_some(out))
}
