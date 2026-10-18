//! Each source's part of an album's sleeve notes, from the library cache:
//! the files' tags, what Plex gave the library, and the cached MusicBrainz
//! and Discogs releases. Nothing here goes to the network.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};

use super::{Fact, LibSong, Part, RawCredit, Source, TrackPart};
use crate::artistinfo::cache;
use crate::artistinfo::fold_name;
use crate::artistinfo::mbdetails::{LibTrack, MbRelease, match_tracks};
use crate::artistinfo::service::album_key;
use crate::error::AppResult;
use crate::tags::provenance::FileRelease;

/// Cache row (in `artist_info`, keyed `album:<id>`) for a Discogs release.
pub const DISCOGS_ROW: &str = "discogs";
/// Cache row (keyed `track:<id>`) for the credits Plex keeps on a song.
pub const PLEX_ROW: &str = "plexCredits";

pub fn track_key(track_id: &str) -> String {
    format!("track:{track_id}")
}

/// What the notes need about an album from the library.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AlbumBasics {
    pub title: String,
    pub artist: String,
    pub year: Option<i32>,
    /// Plex's release date.
    pub released: Option<String>,
    /// Plex's label.
    pub studio: Option<String>,
    /// ARCHIVE_EDITION, the album's most common.
    pub edition: Option<String>,
    /// ARCHIVE_GROUP: editions of one album share it.
    pub group: Option<String>,
    pub status: Option<FileRelease>,
    pub songs: Vec<LibSong>,
    /// Part key per song id (for its file tags).
    pub parts: HashMap<String, String>,
}

pub fn basics(c: &Connection, album_id: &str) -> AppResult<Option<AlbumBasics>> {
    let row = c
        .query_row(
            "SELECT al.title, al.artist_title, al.year, al.released, al.studio, at.edition, at.grp, at.release
             FROM albums al LEFT JOIN album_tags at ON at.album_id = al.id WHERE al.id = ?1",
            [album_id],
            |r| {
                Ok(AlbumBasics {
                    title: r.get(0)?,
                    artist: r.get(1)?,
                    year: r.get(2)?,
                    released: r.get(3)?,
                    studio: r.get(4)?,
                    edition: r.get(5)?,
                    group: r.get(6)?,
                    status: r.get::<_, Option<String>>(7)?.as_deref().and_then(FileRelease::from_key),
                    ..Default::default()
                })
            },
        )
        .optional()?;
    let Some(mut b) = row else { return Ok(None) };
    let mut st = c.prepare_cached(
        "SELECT id, title, disc_number, track_number, part_key FROM tracks WHERE album_id = ?1 ORDER BY disc_number, track_number",
    )?;
    let rows = st.query_map([album_id], |r| {
        Ok((LibSong { id: r.get(0)?, title: r.get(1)?, disc: r.get(2)?, number: r.get(3)? }, r.get::<_, String>(4)?))
    })?;
    for row in rows {
        let (song, part) = row?;
        b.parts.insert(song.id.clone(), part);
        b.songs.push(song);
    }
    Ok(Some(b))
}

/// "a" when every date is the same, else "earliest/latest".
pub fn span<'a>(dates: impl Iterator<Item = &'a str>) -> Option<String> {
    let all: Vec<&str> = dates.map(str::trim).filter(|d| !d.is_empty()).collect();
    let (first, last) = (all.iter().min()?, all.iter().max()?);
    // Each may itself be a span.
    let first = first.split('/').next().unwrap_or(first);
    let last = last.rsplit('/').next().unwrap_or(last);
    Some(if first == last { first.to_owned() } else { format!("{first}/{last}") })
}

fn most_common(values: Vec<String>) -> Option<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for v in values {
        match counts.iter_mut().find(|(x, _)| fold_name(x) == fold_name(&v)) {
            Some((_, n)) => *n += 1,
            None => counts.push((v, 1)),
        }
    }
    // Ties go to the first seen (disc and track order).
    let best = counts.iter().map(|(_, n)| *n).max()?;
    counts.into_iter().find(|(_, n)| *n == best).map(|(v, _)| v)
}

/// The files' tags: credits per song, release facts as most files have
/// them, and the archive's date (when recorded) and source.
pub fn tag_part(c: &Connection, b: &AlbumBasics) -> AppResult<Part> {
    let mut p = Part { edition: b.edition.clone(), ..Default::default() };
    let mut facts: [Vec<String>; 5] = Default::default();
    let mut origins = Vec::new();
    let mut st = c.prepare_cached("SELECT date, source FROM file_tags WHERE part_key = ?1 AND status = 'ok'")?;
    for song in &b.songs {
        let Some(key) = b.parts.get(&song.id) else { continue };
        let s = crate::tags::store::sleeve(c, key)?;
        let (date, source): (Option<String>, Option<String>) =
            st.query_row([key], |r| Ok((r.get(0)?, r.get(1)?))).optional()?.unwrap_or_default();
        for (slot, v) in facts.iter_mut().zip([&s.label, &s.catalogue, &s.barcode, &s.country, &s.original_date]) {
            if let Some(v) = v {
                slot.push(v.clone());
            }
        }
        origins.extend(source);
        let credits: Vec<RawCredit> = s.credits.iter().map(|x| RawCredit::new(&x.name, &x.role, x.detail.as_deref())).collect();
        if !credits.is_empty() || date.is_some() {
            p.tracks.insert(song.id.clone(), TrackPart { credits, recorded: date });
        }
    }
    let [label, catalogue, barcode, country, original] = facts;
    p.label = most_common(label);
    p.catalogue = most_common(catalogue);
    p.barcode = most_common(barcode);
    p.country = most_common(country);
    p.released = most_common(original);
    p.origin = most_common(origins);
    p.recorded = span(p.tracks.values().filter_map(|t| t.recorded.as_deref()));
    Ok(p)
}

/// One credit Plex keeps on a song.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlexCredit {
    pub name: String,
    /// "composer", "writer", "lyricist" or "producer".
    pub roles: Vec<String>,
}

/// The song credits Plex gave, as cached, and whether they're due again.
pub fn plex_cached(c: &Connection, track_id: &str, now: i64) -> AppResult<(Option<Vec<PlexCredit>>, bool)> {
    let row = cache::get(c, &track_key(track_id), PLEX_ROW)?;
    let due = row.as_ref().is_none_or(|r| r.stale(now));
    Ok((row.and_then(|r| r.parse::<Vec<PlexCredit>>()), due))
}

/// What Plex gave the library: the release date and label, and the song
/// credits fetched so far.
pub fn plex_part(c: &Connection, b: &AlbumBasics, now: i64) -> AppResult<Part> {
    let mut p = Part { released: b.released.clone(), label: b.studio.clone(), ..Default::default() };
    for song in &b.songs {
        if let (Some(credits), _) = plex_cached(c, &song.id, now)? {
            let raw: Vec<RawCredit> = credits.iter().flat_map(|x| x.roles.iter().map(|r| RawCredit::new(&x.name, r, None))).collect();
            if !raw.is_empty() {
                p.tracks.insert(song.id.clone(), TrackPart { credits: raw, recorded: None });
            }
        }
    }
    Ok(p)
}

/// The cached MusicBrainz release for an album, if any.
pub fn cached_mb(c: &Connection, album_id: &str) -> AppResult<Option<MbRelease>> {
    let row = cache::get(c, &album_key(album_id), crate::artistinfo::InfoSource::Musicbrainz.as_str())?;
    Ok(row.and_then(|r| r.parse::<MbRelease>()))
}

/// Media formats as a line: "CD", "2×CD", "CD + DVD".
pub fn format_line(formats: &[String]) -> Option<String> {
    let mut runs: Vec<(String, usize)> = Vec::new();
    for f in formats.iter().map(|f| f.trim()).filter(|f| !f.is_empty()) {
        match runs.last_mut() {
            Some((x, n)) if x == f => *n += 1,
            _ => runs.push((f.to_owned(), 1)),
        }
    }
    if runs.is_empty() {
        return None;
    }
    Some(runs.into_iter().map(|(f, n)| if n > 1 { format!("{n}×{f}") } else { f }).collect::<Vec<_>>().join(" + "))
}

fn mb_credits(cs: &[crate::artistinfo::mbdetails::MbCredit]) -> Vec<RawCredit> {
    let mut out = Vec::new();
    for c in cs {
        for role in &c.roles {
            if role == "performer" && !c.instruments.is_empty() {
                out.extend(c.instruments.iter().map(|i| RawCredit::new(&c.name, role, Some(i))));
            } else {
                out.push(RawCredit::new(&c.name, role, None));
            }
        }
    }
    out
}

/// A MusicBrainz release as a source part, its songs matched to the library's.
pub fn mb_part(r: &MbRelease, songs: &[LibSong]) -> Part {
    let lib: Vec<LibTrack> = songs.iter().map(|s| LibTrack { id: s.id.clone(), title: s.title.clone(), disc: s.disc, number: s.number }).collect();
    let ids = match_tracks(&r.tracks, &lib);
    let mut p = Part {
        released: r.first_release_date.clone().or_else(|| r.date.clone()),
        label: r.label.clone(),
        catalogue: r.catalogue.clone(),
        country: r.country.clone(),
        barcode: r.barcode.clone(),
        format: format_line(&r.formats),
        edition: r.disambiguation.clone(),
        notes: r.annotation.clone(),
        credits: mb_credits(&r.credits),
        link: Some(format!("https://musicbrainz.org/release/{}", r.release_id)),
        ..Default::default()
    };
    p.edition_date = r.date.clone().filter(|d| Some(d) != p.released.as_ref());
    p.studios = r.places.iter().map(|x| (x.role.clone(), x.name.clone())).collect();
    for (t, id) in r.tracks.iter().zip(ids) {
        for pl in &t.places {
            if !p.studios.iter().any(|(role, name)| *role == pl.role && *name == pl.name) {
                p.studios.push((pl.role.clone(), pl.name.clone()));
            }
        }
        let Some(id) = id else { continue };
        let credits = mb_credits(&t.credits);
        if !credits.is_empty() || t.recorded.is_some() {
            p.tracks.insert(id, TrackPart { credits, recorded: t.recorded.clone() });
        }
    }
    p.recorded = span(p.tracks.values().filter_map(|t| t.recorded.as_deref()));
    p
}

/// Library artist ids for credited names, by folded name.
pub fn library_ids(c: &Connection, names: &[String]) -> AppResult<HashMap<String, String>> {
    let mut out = HashMap::new();
    let mut st = c.prepare_cached("SELECT id FROM artists WHERE title = ?1 COLLATE NOCASE LIMIT 1")?;
    let mut seen = HashSet::new();
    for n in names {
        let key = fold_name(n);
        if !seen.insert(key.clone()) {
            continue;
        }
        if let Some(id) = st.query_row([n], |r| r.get::<_, String>(0)).optional()? {
            out.insert(key, id);
        }
    }
    Ok(out)
}

/// "A", "A and B", "A, B and C", "A, B, C and 2 more".
fn list(names: &[String]) -> String {
    const SHOWN: usize = 4;
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        n if n <= SHOWN => format!("{} and {}", names[..n - 1].join(", "), names[n - 1]),
        n => format!("{} and {} more", names[..SHOWN - 1].join(", "), n - (SHOWN - 1)),
    }
}

/// How this edition differs from the library's other editions of the album
/// (albums sharing its ARCHIVE_GROUP): songs it has that they don't, and
/// songs it leaves out.
pub fn edition_notes(c: &Connection, album_id: &str, b: &AlbumBasics) -> AppResult<Vec<Fact>> {
    use crate::share::matching::title_key;
    let Some(group) = &b.group else { return Ok(vec![]) };
    let mut st = c.prepare_cached(
        "SELECT at.album_id, coalesce(at.edition, al.title), al.year FROM album_tags at JOIN albums al ON al.id = at.album_id
         WHERE at.grp = ?1 AND at.album_id != ?2 ORDER BY al.year, al.title",
    )?;
    let others: Vec<(String, String, Option<i32>)> =
        st.query_map(rusqlite::params![group, album_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
    let mut titles = c.prepare_cached("SELECT title FROM tracks WHERE album_id = ?1 ORDER BY disc_number, track_number")?;
    let mine: Vec<(String, String)> = b.songs.iter().map(|s| (title_key(&s.title), s.title.clone())).collect();
    let mine_keys: HashSet<&str> = mine.iter().map(|(k, _)| k.as_str()).collect();
    let mut out = Vec::new();
    for (id, name, _) in others {
        let theirs: Vec<(String, String)> =
            titles.query_map([&id], |r| r.get::<_, String>(0))?.filter_map(Result::ok).map(|t| (title_key(&t), t)).collect();
        let their_keys: HashSet<&str> = theirs.iter().map(|(k, _)| k.as_str()).collect();
        let extra: Vec<String> = mine.iter().filter(|(k, _)| !their_keys.contains(k.as_str())).map(|(_, t)| t.clone()).collect();
        let missing: Vec<String> = theirs.iter().filter(|(k, _)| !mine_keys.contains(k.as_str())).map(|(_, t)| t.clone()).collect();
        let songs = |n: usize| if n == 1 { "1 song".to_owned() } else { format!("{n} songs") };
        let text = match (extra.is_empty(), missing.is_empty()) {
            (true, true) => format!("Same songs as {name}"),
            (false, true) => format!("{} more than {name}: {}", songs(extra.len()), list(&extra)),
            (true, false) => format!("Leaves out {} from {name}: {}", songs(missing.len()), list(&missing)),
            (false, false) => format!(
                "Next to {name}: adds {} ({}), leaves out {} ({})",
                songs(extra.len()),
                list(&extra),
                songs(missing.len()),
                list(&missing)
            ),
        };
        out.push(Fact { value: text, sources: vec![Source::Tags] });
    }
    Ok(out)
}
