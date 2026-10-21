//! CarPlay: Platter on the car's screen.
//!
//! The car only gets Apple's templates: a tab bar of lists, and the shared
//! Now Playing screen (fed by the same session as the lock screen). This
//! module builds the lists from the library cache; `ios.rs` turns them into
//! templates and plays what is picked.

use std::collections::HashSet;

use rusqlite::Connection;

use crate::audio::events::PlayerStatus;
use crate::audio::queue::{QueueView, Repeat};
use crate::audio::source::{ContextKind, PlayContext};
use crate::db::queries;
use crate::downloads::store::{self as dl_store, ItemKind};
use crate::db::types::{AlbumSort, ArtistSort, HomeItem};
use crate::error::AppResult;
use crate::plex::types::{Health, HealthStatus};

/// Most rows one CarPlay list shows; cars allow 100 to 500, and a long list
/// is hard to use while driving anyway.
pub const LIST_MAX: usize = 100;
/// Rows every car shows: some cut any list to 12 while driving, and the
/// fewest a list may be given room for.
pub const LIST_SHORT: usize = 12;
/// Most releases an artist's list pages through: far more than anyone has.
const RELEASES_MAX: usize = 2000;

/// One row on the car's screen.
#[derive(Debug, Clone, PartialEq)]
pub struct CarItem {
    pub title: String,
    pub subtitle: Option<String>,
    /// The server's image path, fetched through the art cache.
    pub thumb: Option<String>,
    pub action: CarAction,
}

/// What a row does when picked.
#[derive(Debug, Clone, PartialEq)]
pub enum CarAction {
    /// Start this album, playlist, artist or mix from the top.
    Play(PlayContext),
    /// Start it shuffled.
    Shuffle(PlayContext),
    /// Play it from this song: a song in an opened list (its track id, and
    /// its place in the list, so a song that's in it twice starts at the
    /// right copy).
    PlayFrom { context: PlayContext, track_id: String, index: usize },
    /// An opened list's own Play (or Shuffle) row: plays like
    /// [`CarAction::Play`] but is never the row marked as playing (the song
    /// playing is).
    Start { context: PlayContext, shuffle: bool },
    /// Shuffle every song: the songs are drawn when it's picked.
    ShuffleLibrary,
    /// Open another list.
    Open(CarList),
    /// Show another page of this list in place of the one showing, so a
    /// long list never goes deeper than the car allows.
    Turn { list: CarList, page: usize },
    /// Play this queue entry (its item id) now: a row in Up next.
    Jump(String),
    /// A message, not a choice: picking it does nothing.
    Info,
}

/// The lists the car can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CarList {
    RecentlyPlayed,
    RecentlyAdded,
    Playlists,
    Albums,
    Artists,
    /// One artist: Popular, then everything they put out, newest first, a
    /// page at a time when it's longer than the car shows.
    Artist { id: String, name: String },
    /// An album, playlist, mix or Liked songs, opened: Play and Shuffle,
    /// then its songs.
    Songs { kind: ContextKind, id: String, title: String },
}

impl CarList {
    pub fn title(&self) -> &str {
        match self {
            CarList::RecentlyPlayed => "Recently played",
            CarList::RecentlyAdded => "Recently added",
            CarList::Playlists => "Playlists",
            CarList::Albums => "Albums",
            CarList::Artists => "Artists",
            CarList::Artist { name, .. } => name,
            CarList::Songs { title, .. } => title,
        }
    }
}

/// Opens an album, playlist, mix or Liked songs to its songs.
fn songs(kind: ContextKind, id: &str, title: &str) -> CarAction {
    CarAction::Open(CarList::Songs { kind, id: id.to_string(), title: title.to_string() })
}

fn play(kind: ContextKind, id: &str) -> CarAction {
    CarAction::Play(PlayContext {
        kind,
        id: Some(id.to_string()),
        track_ids: None,
    })
}

fn open(list: CarList) -> CarItem {
    CarItem {
        title: list.title().to_string(),
        subtitle: None,
        thumb: None,
        action: CarAction::Open(list),
    }
}

/// Most covers one row of covers shows (CarPlay's image row holds 8).
pub const COVERS_MAX: usize = 8;
/// Songs "Shuffle my library" starts with; the queue tops up from there.
pub const SHUFFLE_MAX: usize = 500;
/// Mixes on Home.
pub const MIXES_MAX: usize = 12;

/// A titled group of rows.
#[derive(Debug, Clone, PartialEq)]
pub struct CarSection {
    pub header: Option<String>,
    pub rows: Vec<CarRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CarRow {
    Item(CarItem),
    /// One row of covers under a title; each cover plays its item.
    Covers { title: String, items: Vec<CarItem> },
}

/// The Home tab, as you'd use it in a car: shuffle everything or play your
/// likes in one tap, then what you played lately as covers, then your
/// mixes, then the longer lists.
pub fn home_sections(c: &Connection) -> AppResult<Vec<CarSection>> {
    let mut first = Vec::new();
    if has_songs(c)? {
        first.push(CarRow::Item(shuffle_library()));
    }
    first.push(CarRow::Item(liked_item()));
    let mut out = vec![CarSection { header: None, rows: first }];
    let recent: Vec<CarItem> = queries::recently_played(c, COVERS_MAX * 2)?
        .into_iter()
        .filter(|h| !matches!(h, HomeItem::Artist { .. }))
        .map(home_item)
        .take(COVERS_MAX)
        .collect();
    if !recent.is_empty() {
        out.push(CarSection {
            header: None,
            rows: vec![CarRow::Covers { title: "Jump back in".into(), items: recent }],
        });
    }
    let mixes = mixes(c)?;
    if !mixes.is_empty() {
        out.push(CarSection { header: Some("Made for you".into()), rows: mixes.into_iter().map(CarRow::Item).collect() });
    }
    out.push(CarSection {
        header: Some("More".into()),
        rows: vec![CarRow::Item(open(CarList::RecentlyPlayed)), CarRow::Item(open(CarList::RecentlyAdded))],
    });
    Ok(out)
}

/// The rows of [`home_sections`], flattened (covers become plain rows).
pub fn home(c: &Connection) -> AppResult<Vec<CarItem>> {
    Ok(home_sections(c)?
        .into_iter()
        .flat_map(|s| s.rows)
        .flat_map(|r| match r {
            CarRow::Item(i) => vec![i],
            CarRow::Covers { items, .. } => items,
        })
        .collect())
}

fn liked_item() -> CarItem {
    CarItem {
        title: "Liked songs".to_string(),
        subtitle: None,
        thumb: None,
        action: songs(ContextKind::Liked, "liked", "Liked songs"),
    }
}

fn has_songs(c: &Connection) -> AppResult<bool> {
    Ok(c.query_row("SELECT EXISTS (SELECT 1 FROM tracks)", [], |r| r.get(0))?)
}

fn shuffle_library() -> CarItem {
    CarItem { title: "Shuffle my library".into(), subtitle: None, thumb: None, action: CarAction::ShuffleLibrary }
}

/// What "Shuffle my library" plays: a fresh random draw of every song (at
/// most [`SHUFFLE_MAX`]), for the player to shuffle and top up.
pub fn shuffle_library_context(c: &Connection) -> AppResult<PlayContext> {
    let mut st = c.prepare_cached("SELECT id FROM tracks ORDER BY random() LIMIT ?1")?;
    let ids = st.query_map([SHUFFLE_MAX as i64], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(ids) })
}

/// The made-for-you mixes (Daily Mixes, the daylist, Discover...), in their
/// shelf order, each played in one tap.
fn mixes(c: &Connection) -> AppResult<Vec<CarItem>> {
    Ok(crate::mixes::store::all(c)?
        .into_iter()
        .take(MIXES_MAX)
        .map(|(m, _, _)| CarItem {
            thumb: m.art.clone().or_else(|| m.covers.first().cloned()),
            subtitle: (!m.description.is_empty()).then(|| m.description.clone()),
            action: songs(ContextKind::Mix, &m.id, &m.title),
            title: m.title,
        })
        .collect())
}

/// The Library tab.
pub fn library() -> Vec<CarItem> {
    vec![
        liked_item(),
        open(CarList::Playlists),
        open(CarList::Albums),
        open(CarList::Artists),
    ]
}

/// One of the lists a row opens, at most [`LIST_MAX`] long. `liked_min` is
/// the rating that counts as liked (Settings).
pub fn list(c: &Connection, which: &CarList, liked_min: f64) -> AppResult<Vec<CarItem>> {
    page(c, which, liked_min, 0, LIST_MAX)
}

/// A page of a list, for a car that shows `room` rows. Only an artist's
/// list has pages; the others are the same on every page.
pub fn page(c: &Connection, which: &CarList, liked_min: f64, page: usize, room: usize) -> AppResult<Vec<CarItem>> {
    Ok(match which {
        CarList::RecentlyPlayed => queries::recently_played(c, LIST_MAX)?.into_iter().map(home_item).collect(),
        CarList::RecentlyAdded => queries::recently_added(c, LIST_MAX)?.into_iter().map(album_item).collect(),
        CarList::Playlists => queries::home_playlists(c, LIST_MAX)?.into_iter().map(playlist_item).collect(),
        CarList::Albums => {
            let albums = every(|off| Ok(queries::albums(c, AlbumSort::Name, off, PAGE_READ, None)?.items))?;
            let rows = albums.into_iter().map(|a| (letter(&a.sort_title), album_item(a))).collect();
            a_to_z(which, rows, ("album", "albums"), page, room.clamp(LIST_SHORT, LIST_MAX))
        }
        CarList::Artists => {
            let artists = every(|off| Ok(queries::artists(c, ArtistSort::Name, off, PAGE_READ)?.items))?;
            let rows = artists.into_iter().map(|a| (letter(&a.sort_title), artist_item(a))).collect();
            a_to_z(which, rows, ("artist", "artists"), page, room.clamp(LIST_SHORT, LIST_MAX))
        }
        CarList::Artist { id, .. } => artist(c, which, id, page, room.clamp(LIST_SHORT, LIST_MAX))?,
        CarList::Songs { kind, id, .. } => songs_list(c, *kind, id, liked_min)?,
    })
}

/// Rows read from the library at a time.
const PAGE_READ: u32 = 500;

/// Every row a paged read gives, `read` taking the offset.
fn every<T>(mut read: impl FnMut(u32) -> AppResult<Vec<T>>) -> AppResult<Vec<T>> {
    let mut out = Vec::new();
    loop {
        let got = read(out.len() as u32)?;
        let done = got.len() < PAGE_READ as usize;
        out.extend(got);
        if done {
            return Ok(out);
        }
    }
}

/// The letter a name files under: its first letter without accents, or #
/// for numbers, symbols and other scripts.
fn letter(sort_title: &str) -> char {
    use unicode_normalization::UnicodeNormalization;
    match sort_title.nfd().find(|c| c.is_alphanumeric()).map(|c| c.to_ascii_uppercase()) {
        Some(c) if c.is_ascii_uppercase() => c,
        _ => '#',
    }
}

/// A run of letters on an A to Z list, and where its rows are.
#[derive(Debug)]
struct Letters {
    first: char,
    /// The last letter in the run other than #.
    last: Option<char>,
    hash: bool,
    start: usize,
    end: usize,
}

impl Letters {
    fn label(&self) -> String {
        let letters = match self.last {
            Some(last) if last == self.first => last.to_string(),
            Some(last) => format!("{} to {last}", self.first),
            None => return "#".to_string(),
        };
        if self.hash { format!("{letters} and #") } else { letters }
    }
}

/// Rows grouped into at most `room` runs of letters, each as even as whole
/// letters allow; one run per letter when there's room.
fn letter_runs(letters: &[char], room: usize) -> Vec<Letters> {
    let mut buckets: Vec<(char, usize, usize)> = Vec::new();
    for (i, l) in letters.iter().enumerate() {
        match buckets.last_mut() {
            Some((b, _, end)) if b == l => *end = i + 1,
            _ => buckets.push((*l, i, i + 1)),
        }
    }
    let target = if buckets.len() <= room { 1 } else { letters.len().div_ceil(room) };
    let mut runs: Vec<Letters> = Vec::new();
    for (l, start, end) in buckets {
        let open = runs.last().is_some_and(|r| r.end - r.start < target);
        let run = match runs.last_mut() {
            Some(r) if open => r,
            _ => {
                runs.push(Letters { first: l, last: None, hash: false, start, end });
                runs.last_mut().expect("just pushed")
            }
        };
        run.end = end;
        if l == '#' {
            run.hash = true;
        } else {
            run.last = Some(l);
        }
    }
    runs
}

/// Every album or artist, A to Z (numbers and symbols under # at the end).
/// When they don't all fit the car, the first page is a row per run of
/// letters, and each run has its own pages (All letters first, More last);
/// every page turns in place.
fn a_to_z(which: &CarList, mut rows: Vec<(char, CarItem)>, noun: (&str, &str), page: usize, room: usize) -> Vec<CarItem> {
    // Stable: A to Z as the library sorts names, # after Z.
    rows.sort_by_key(|(l, _)| if *l == '#' { 26 } else { *l as u8 - b'A' });
    if rows.len() <= room {
        return rows.into_iter().map(|(_, r)| r).collect();
    }
    let letters: Vec<char> = rows.iter().map(|(l, _)| *l).collect();
    let runs = letter_runs(&letters, room);
    let turn = |page| CarAction::Turn { list: which.clone(), page };
    let count = |n: usize| if n == 1 { format!("1 {}", noun.0) } else { format!("{n} {}", noun.1) };
    // Every page after the first: its run, and its rows.
    let mut pages: Vec<(usize, usize, usize)> = Vec::new();
    let mut firsts = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        firsts.push(pages.len() + 1);
        let starts = page_starts(run.end - run.start, room);
        for (k, s) in starts.iter().enumerate() {
            let e = starts.get(k + 1).copied().unwrap_or(run.end - run.start);
            pages.push((i, run.start + s, run.start + e));
        }
    }
    let Some(&(run, start, end)) = page.checked_sub(1).and_then(|p| pages.get(p)) else {
        return runs
            .iter()
            .zip(firsts)
            .map(|(r, first)| CarItem { title: r.label(), subtitle: Some(count(r.end - r.start)), thumb: None, action: turn(first) })
            .collect();
    };
    let run = &runs[run];
    let more = run.end - end;
    let mut out = vec![CarItem { title: "All letters".to_string(), subtitle: None, thumb: None, action: turn(0) }];
    out.extend(rows.drain(start..end).map(|(_, r)| r));
    if more > 0 {
        let subtitle = format!("{more} more {} in {}", if more == 1 { noun.0 } else { noun.1 }, run.label());
        out.push(CarItem { title: "More".to_string(), subtitle: Some(subtitle), thumb: None, action: turn(page + 1) });
    }
    out
}

/// An artist's list: Popular (which plays on through the albums, as the
/// artist page's play does), then every release, newest first. When that's
/// more than the car shows, the first page ends in Older releases and the
/// next ones start with Newer releases; both turn the page in place.
fn artist(c: &Connection, which: &CarList, id: &str, page: usize, room: usize) -> AppResult<Vec<CarItem>> {
    let mut albums = queries::artist_albums(c, id, RELEASES_MAX)?;
    sort_newest_first(&mut albums);
    let starts = page_starts(albums.len(), room);
    let page = page.min(starts.len() - 1);
    let start = starts[page];
    let end = starts.get(page + 1).copied().unwrap_or(albums.len());
    let turn = |page| CarAction::Turn { list: which.clone(), page };
    let head = if page == 0 {
        CarItem { title: "Popular".to_string(), subtitle: None, thumb: None, action: play(ContextKind::Artist, id) }
    } else {
        CarItem { title: "Newer releases".to_string(), subtitle: None, thumb: None, action: turn(page - 1) }
    };
    let older = albums.len() - end;
    let mut out = vec![head];
    // On the artist's own list the name says nothing new: the year and the
    // kind of release do.
    out.extend(albums.drain(start..end).map(artist_album_item));
    if older > 0 {
        let more = if older == 1 { "1 more".to_string() } else { format!("{older} more") };
        out.push(CarItem { title: "Older releases".to_string(), subtitle: Some(more), thumb: None, action: turn(page + 1) });
    }
    Ok(out)
}

/// Newest first, as a discography is; demos and unreleased music after
/// everything that came out, then undated releases last in each part.
fn sort_newest_first(albums: &mut [crate::model::Album]) {
    use crate::model::ReleaseSection;
    albums.sort_by_key(|a| {
        let archive = matches!(a.section, ReleaseSection::Demos | ReleaseSection::Unreleased);
        (archive, a.year.is_none(), std::cmp::Reverse(a.year), std::cmp::Reverse(a.added_at))
    });
}

/// Where each page of an artist's `total` releases starts, for a car that
/// shows `room` rows: every page has its first row (Popular or Newer
/// releases), and each page but the last ends in Older releases.
fn page_starts(total: usize, room: usize) -> Vec<usize> {
    let mut starts = vec![0];
    let mut at = 0;
    loop {
        // All that's left fits with just the first row.
        if total - at < room {
            return starts;
        }
        at += room - 2;
        starts.push(at);
    }
}

/// The Downloaded tab: everything kept on the phone, newest first, so the
/// car has music with no signal.
pub fn downloaded(c: &Connection) -> AppResult<Vec<CarItem>> {
    use crate::downloads::store::{self, ItemKind};
    let mut rows = store::items(c)?;
    rows.reverse();
    let albums: Vec<String> = rows.iter().filter(|r| r.key.kind == ItemKind::Album).map(|r| r.key.id.clone()).collect();
    let albums = queries::albums_by_ids(c, &albums)?;
    Ok(rows
        .into_iter()
        .take(LIST_MAX)
        .map(|r| match r.key.kind {
            ItemKind::Album => match albums.iter().find(|a| a.id == r.key.id) {
                Some(a) => album_item(a.clone()),
                None => CarItem { action: songs(ContextKind::Album, &r.key.id, &r.title), title: r.title, subtitle: None, thumb: None },
            },
            ItemKind::Playlist => CarItem {
                action: songs(ContextKind::Playlist, &r.key.id, &r.title),
                title: r.title,
                subtitle: Some("Playlist".into()),
                thumb: None,
            },
            ItemKind::Liked => CarItem {
                title: "Liked songs".into(),
                subtitle: None,
                thumb: None,
                action: songs(ContextKind::Liked, "liked", "Liked songs"),
            },
            ItemKind::Mix => CarItem {
                title: r.title,
                subtitle: Some("Mix".into()),
                thumb: None,
                action: CarAction::Play(PlayContext {
                    kind: ContextKind::Tracks,
                    id: None,
                    track_ids: store::item_tracks(c, &r.key).ok(),
                }),
            },
        })
        .collect())
}

/// What's playing, for the Radio tab's first stations.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Playing {
    pub track_id: String,
    pub title: String,
    pub artist_id: String,
    pub artist: String,
}

/// The Radio tab: one-tap stations, Platter's own (nothing from outside).
pub fn radio(c: &Connection, playing: Option<&Playing>) -> AppResult<Vec<CarItem>> {
    let mut out = Vec::new();
    if let Some(p) = playing {
        out.push(CarItem {
            title: format!("Radio from {}", p.title),
            subtitle: Some(p.artist.clone()),
            thumb: None,
            action: play(ContextKind::Radio, &format!("track:{}", p.track_id)),
        });
        if !p.artist_id.is_empty() {
            out.push(CarItem {
                title: format!("{} radio", p.artist),
                subtitle: None,
                thumb: None,
                action: play(ContextKind::Radio, &format!("artist:{}", p.artist_id)),
            });
        }
    }
    let pick = |sql: &str| -> AppResult<Option<String>> {
        Ok(c.query_row(sql, [], |r| r.get::<_, String>(0)).ok())
    };
    if let Some(id) = pick("SELECT id FROM albums ORDER BY random() LIMIT 1")? {
        out.extend(queries::albums_by_ids(c, &[id])?.into_iter().map(|a| CarItem {
            title: "A random album".into(),
            subtitle: Some(format!("{} by {}", a.title, a.artist_title)),
            thumb: a.thumb.clone(),
            action: play(ContextKind::Album, &a.id),
        }));
    }
    // Played before, but not in the last 90 days (or never, when nothing is old).
    let forgotten = pick(
        "SELECT id FROM albums WHERE view_count > 0 AND last_viewed_at < strftime('%s','now') - 90 * 86400
         ORDER BY random() LIMIT 1",
    )?;
    if let Some(id) = forgotten {
        out.extend(queries::albums_by_ids(c, &[id])?.into_iter().map(|a| CarItem {
            title: "One you haven't played in a while".into(),
            subtitle: Some(format!("{} by {}", a.title, a.artist_title)),
            thumb: a.thumb.clone(),
            action: play(ContextKind::Album, &a.id),
        }));
    }
    Ok(out)
}

fn album_item(a: crate::model::Album) -> CarItem {
    CarItem {
        subtitle: Some(a.artist_title.clone()),
        thumb: a.thumb.clone(),
        action: songs(ContextKind::Album, &a.id, &a.title),
        title: a.title,
    }
}

/// The centred square to crop a `w` x `h` picture to, as (x, y, side) in
/// whole pixels; None when it's square already (or empty).
pub fn square_crop(w: usize, h: usize) -> Option<(usize, usize, usize)> {
    if w == h || w == 0 || h == 0 {
        return None;
    }
    let side = w.min(h);
    Some(((w - side) / 2, (h - side) / 2, side))
}

/// "2011", "EP · 2019", "Single".
pub fn release_line(year: Option<i32>, format: Option<&str>) -> Option<String> {
    let kind = match format {
        Some("ep") => Some("EP"),
        Some("single") => Some("Single"),
        Some("compilation") => Some("Compilation"),
        _ => None,
    };
    match (kind, year) {
        (Some(k), Some(y)) => Some(format!("{k} · {y}")),
        (Some(k), None) => Some(k.to_string()),
        (None, Some(y)) => Some(y.to_string()),
        (None, None) => None,
    }
}

fn artist_album_item(a: crate::model::Album) -> CarItem {
    CarItem {
        subtitle: release_line(a.year, a.format.as_deref()),
        thumb: a.thumb.clone(),
        action: songs(ContextKind::Album, &a.id, &a.title),
        title: a.title,
    }
}

fn playlist_item(p: crate::model::Playlist) -> CarItem {
    let count = if p.track_count == 1 { "1 song".to_string() } else { format!("{} songs", p.track_count) };
    CarItem {
        subtitle: Some(count),
        thumb: p.thumb.clone(),
        action: songs(ContextKind::Playlist, &p.id, &p.title),
        title: p.title,
    }
}

fn artist_item(a: crate::model::Artist) -> CarItem {
    CarItem {
        subtitle: None,
        thumb: a.thumb.clone(),
        action: CarAction::Open(CarList::Artist { id: a.id, name: a.title.clone() }),
        title: a.title,
    }
}

fn home_item(h: HomeItem) -> CarItem {
    match h {
        HomeItem::Album { album } => album_item(album),
        HomeItem::Artist { artist } => artist_item(artist),
        HomeItem::Playlist { playlist } => playlist_item(playlist),
    }
}

/// An opened album, playlist, mix or Liked songs: Play and Shuffle first,
/// then each song, which plays it from there.
fn songs_list(c: &Connection, kind: ContextKind, id: &str, liked_min: f64) -> AppResult<Vec<CarItem>> {
    let tracks = match kind {
        ContextKind::Album => queries::album_tracks(c, id)?,
        ContextKind::Playlist => {
            let ids: Vec<String> = crate::db::playlists::items(c, id)?.into_iter().map(|i| i.track_id).collect();
            let found: std::collections::HashMap<String, crate::model::Track> =
                queries::tracks_by_ids(c, &ids)?.into_iter().map(|t| (t.id.clone(), t)).collect();
            ids.iter().filter_map(|t| found.get(t).cloned()).collect()
        }
        ContextKind::Mix => crate::mixes::store::tracks(c, id)?.0,
        ContextKind::Liked => crate::db::likes::liked_tracks(c, liked_min)?,
        ContextKind::Artist | ContextKind::Tracks | ContextKind::Search | ContextKind::Radio => Vec::new(),
    };
    let context = || PlayContext { kind, id: Some(id.to_string()), track_ids: None };
    let total = tracks.len();
    let count = if total == 1 { "1 song".to_string() } else { format!("{total} songs") };
    let mut out = vec![
        CarItem {
            title: "Play".into(),
            subtitle: Some(count),
            thumb: None,
            action: CarAction::Start { context: context(), shuffle: false },
        },
        CarItem { title: "Shuffle".into(), subtitle: None, thumb: None, action: CarAction::Start { context: context(), shuffle: true } },
    ];
    // Room for the songs, and a last row saying how many more there are.
    let room = if total > LIST_MAX - 2 { LIST_MAX - 3 } else { LIST_MAX - 2 };
    // An album's songs share its cover; other lists show each song's.
    let own_covers = kind != ContextKind::Album;
    out.extend(tracks.into_iter().take(room).enumerate().map(|(index, t)| CarItem {
        subtitle: Some(t.track_artist.clone().unwrap_or_else(|| t.artist_title.clone())),
        thumb: if own_covers { t.thumb.clone() } else { None },
        action: CarAction::PlayFrom { context: context(), track_id: t.id.clone(), index },
        title: t.title,
    }));
    if total > room {
        out.push(info(&format!("Showing {room} of {total} songs"), Some("Play or Shuffle plays them all")));
    }
    Ok(out)
}

/// What's playing, as far as marking rows goes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NowKey {
    /// The context the queue plays (album, playlist, mix...), with its id.
    pub context: Option<(ContextKind, String)>,
    /// The playing song's album.
    pub album_id: Option<String>,
    /// The playing song.
    pub track_id: Option<String>,
}

/// The playing song's album, opened to its songs: what Now Playing's album
/// button shows. None when nothing plays or the album isn't in the library.
pub fn now_album(c: &Connection, now: &NowKey) -> AppResult<Option<CarList>> {
    let Some(id) = &now.album_id else { return Ok(None) };
    Ok(queries::albums_by_ids(c, std::slice::from_ref(id))?
        .into_iter()
        .next()
        .map(|a| CarList::Songs { kind: ContextKind::Album, id: a.id, title: a.title }))
}

/// Whether a row plays what's playing now, to show it as playing: the same
/// album, playlist, mix or Liked songs, or the album the playing song is on.
pub fn is_playing(item: &CarItem, now: &NowKey) -> bool {
    let (kind, id) = match &item.action {
        // A song in an opened list: that song, in that list.
        CarAction::PlayFrom { context, track_id, .. } => {
            let same_list = match (&context.id, &now.context) {
                (Some(id), Some((k, cid))) => *k == context.kind && cid == id,
                _ => false,
            };
            return same_list && now.track_id.as_deref() == Some(track_id.as_str());
        }
        CarAction::Open(CarList::Songs { kind, id, .. }) => (*kind, id),
        CarAction::Play(ctx) | CarAction::Shuffle(ctx) if ctx.track_ids.is_none() => match &ctx.id {
            Some(id) => (ctx.kind, id),
            None => return false,
        },
        _ => return false,
    };
    if now.context.as_ref().is_some_and(|(k, cid)| *k == kind && cid == id) {
        return true;
    }
    kind == ContextKind::Album && now.album_id.as_deref() == Some(id.as_str())
}

/// Up next on the car: the songs queued by hand, then the rest of what's
/// playing, at most [`LIST_MAX`]. Picking one plays it.
pub fn up_next(q: &QueueView) -> Vec<CarItem> {
    q.user_queue
        .iter()
        .chain(&q.next_up)
        .take(LIST_MAX)
        .map(|i| CarItem {
            title: i.track.title.clone(),
            subtitle: Some(i.track.track_artist.clone().unwrap_or_else(|| i.track.artist_title.clone())),
            thumb: i.track.thumb.clone(),
            action: CarAction::Jump(i.item_id.clone()),
        })
        .collect()
}

/// The car's repeat button steps Off, All, One, and back to Off.
pub fn next_repeat(r: Repeat) -> Repeat {
    match r {
        Repeat::Off => Repeat::All,
        Repeat::All => Repeat::One,
        Repeat::One => Repeat::Off,
    }
}

/// Whether the car can show the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// No account and no other server: nothing to show yet.
    SignedOut,
    /// Signed in, but the server is out of reach: only downloads play.
    Unreachable,
    Online,
}

/// How things stand, from the Plex account's health and whether another
/// server (Jellyfin, Navidrome) is set up. With another server, a Plex
/// account that isn't signed in doesn't count.
pub fn reach(plex: &Health, other_servers: bool) -> Reach {
    match plex.status {
        HealthStatus::NoAuth if !plex.cached_only => {
            if other_servers { Reach::Online } else { Reach::SignedOut }
        }
        _ if plex.cached_only => Reach::Unreachable,
        _ => Reach::Online,
    }
}

/// The car's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home,
    Library,
    Downloaded,
    Radio,
}

pub const SIGN_IN: &str = "Open Platter on your phone to sign in";
pub const UNREACHABLE: &str = "Can't reach your server";

fn info(title: &str, subtitle: Option<&str>) -> CarItem {
    CarItem { title: title.to_string(), subtitle: subtitle.map(str::to_string), thumb: None, action: CarAction::Info }
}

/// A tab's rows for how things stand. Signed out, every tab says to sign in
/// on the phone; with the server out of reach, Downloaded still lists what's
/// kept and the others say so. `rows` reads the tab only when it shows.
pub fn tab_rows(tab: Tab, reach: Reach, rows: impl FnOnce() -> Vec<CarItem>) -> Vec<CarItem> {
    match (reach, tab) {
        (Reach::SignedOut, _) => vec![info(SIGN_IN, None)],
        (Reach::Unreachable, Tab::Downloaded) | (Reach::Online, _) => rows(),
        (Reach::Unreachable, _) => vec![info(UNREACHABLE, Some("Downloaded music still plays"))],
    }
}

/// A car that drops and comes back within this many seconds is the same
/// drive (a loose cable, a car that restarts its screen): nothing changes.
pub const RECONNECT_GRACE_S: u64 = 30;

/// How things stand when a car connects.
#[derive(Debug, Clone, PartialEq)]
pub struct Connect {
    /// "Resume when the car connects".
    pub resume_on: bool,
    pub reach: Reach,
    pub status: PlayerStatus,
    /// The song the player would play on: (id, kept on the phone).
    pub track: Option<(String, bool)>,
    /// Seconds since the car last left, if it left during this run (see
    /// [`since_left`]).
    pub since_disconnect_s: Option<u64>,
}

/// Seconds since the car left, from the monotonic clock and the wall clock.
/// Apple's monotonic clock stops while the phone sleeps, so a short drive
/// and a night asleep can read as seconds; the wall clock counts the sleep.
/// The longer of the two wins, and a wall clock that went backwards counts
/// as long ago: a missed resume is worse than a resume after a blink.
pub fn since_left(mono_s: u64, wall_s: Option<u64>) -> u64 {
    wall_s.map_or(u64::MAX, |w| w.max(mono_s))
}

/// What to do when a car connects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConnect {
    Nothing,
    /// Play on from where the player stopped.
    Resume,
}

/// Plays on where you left off when the car connects, if the setting is
/// on. Never when it's already playing, when nothing was playing, when the
/// car only blinked, or when the song can't play (out of reach and not
/// kept on the phone). Nothing new is started: no queue, no sound.
pub fn on_connect(c: &Connect) -> OnConnect {
    if !c.resume_on || c.since_disconnect_s.is_some_and(|s| s < RECONNECT_GRACE_S) {
        return OnConnect::Nothing;
    }
    if !matches!(c.status, PlayerStatus::Paused | PlayerStatus::Idle) {
        return OnConnect::Nothing;
    }
    match (&c.track, c.reach) {
        (None, _) | (_, Reach::SignedOut) => OnConnect::Nothing,
        (Some((_, kept)), Reach::Unreachable) if !kept => OnConnect::Nothing,
        _ => OnConnect::Resume,
    }
}

/// What's kept on the phone: items with at least one finished song, and
/// every finished song.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Kept {
    pub items: HashSet<(ItemKind, String)>,
    pub tracks: HashSet<String>,
}

impl Kept {
    pub fn read(c: &Connection) -> AppResult<Self> {
        let items = dl_store::all_item_counts(c)?
            .into_iter()
            .filter(|(_, n)| n.done > 0)
            .filter_map(|((kind, id), _)| Some((ItemKind::parse(&kind).ok()?, id)))
            .collect();
        let tracks = dl_store::done_files(c)?.into_iter().map(|(t, _)| t).collect();
        Ok(Self { items, tracks })
    }

    fn item(&self, kind: ItemKind, id: &str) -> bool {
        self.items.contains(&(kind, id.to_string()))
    }

    /// Whether picking this row plays something with no signal. Rows that
    /// don't play (lists, messages, Up next) always count.
    pub fn plays(&self, action: &CarAction) -> bool {
        let (kind, id) = match action {
            CarAction::Play(ctx) | CarAction::Shuffle(ctx) => {
                if let Some(ids) = &ctx.track_ids {
                    return ids.iter().any(|t| self.tracks.contains(t));
                }
                (ctx.kind, ctx.id.as_deref().unwrap_or_default())
            }
            // An album, playlist or mix to open: as playing it would.
            CarAction::Open(CarList::Songs { kind, id, .. }) => (*kind, id.as_str()),
            // A song in an opened list: that song.
            CarAction::PlayFrom { track_id, .. } => return self.tracks.contains(track_id),
            CarAction::Start { context, .. } => (context.kind, context.id.as_deref().unwrap_or_default()),
            CarAction::ShuffleLibrary => return !self.tracks.is_empty(),
            CarAction::Open(_) | CarAction::Turn { .. } | CarAction::Jump(_) | CarAction::Info => return true,
        };
        match kind {
            ContextKind::Album => self.item(ItemKind::Album, id),
            ContextKind::Playlist => self.item(ItemKind::Playlist, id),
            ContextKind::Liked => self.item(ItemKind::Liked, ""),
            ContextKind::Mix => self.item(ItemKind::Mix, id),
            // Popular and radio are drawn from the server.
            ContextKind::Artist | ContextKind::Radio | ContextKind::Tracks | ContextKind::Search => false,
        }
    }
}

/// The subtitle of a row that can't play with no signal.
pub const NEEDS_CONNECTION: &str = "Needs a connection";

/// A list's rows with no signal. With "Show downloads only when offline",
/// rows that can't play go; without it they stay, say so, and do nothing.
pub fn offline_rows(items: Vec<CarItem>, kept: &Kept, only_downloads: bool) -> Vec<CarItem> {
    items
        .into_iter()
        .filter_map(|i| {
            if kept.plays(&i.action) {
                Some(i)
            } else if only_downloads {
                None
            } else {
                Some(CarItem { subtitle: Some(NEEDS_CONNECTION.into()), action: CarAction::Info, ..i })
            }
        })
        .collect()
}

/// Home's sections with no signal, as [`offline_rows`]. A row of covers
/// keeps only covers that play (they have no room to say why not); a
/// section left empty goes.
pub fn offline_sections(sections: Vec<CarSection>, kept: &Kept, only_downloads: bool) -> Vec<CarSection> {
    sections
        .into_iter()
        .filter_map(|s| {
            let rows: Vec<CarRow> = s
                .rows
                .into_iter()
                .filter_map(|r| match r {
                    CarRow::Item(i) => offline_rows(vec![i], kept, only_downloads).pop().map(CarRow::Item),
                    CarRow::Covers { title, items } => {
                        let items: Vec<CarItem> = items.into_iter().filter(|i| kept.plays(&i.action)).collect();
                        (!items.is_empty()).then_some(CarRow::Covers { title, items })
                    }
                })
                .collect();
            (!rows.is_empty()).then_some(CarSection { header: s.header, rows })
        })
        .collect()
}

/// A tab's rows with the server out of reach: what plays, or the message
/// when nothing does.
pub fn offline_tab(items: Vec<CarItem>, kept: &Kept, only_downloads: bool) -> Vec<CarItem> {
    let rows = offline_rows(items, kept, only_downloads);
    if rows.iter().all(|r| r.action == CarAction::Info) {
        return vec![info(UNREACHABLE, Some("Downloaded music still plays"))];
    }
    rows
}

/// "Shuffle my library" with no signal: a random draw of the kept songs.
pub fn shuffle_downloads_context(c: &Connection) -> AppResult<PlayContext> {
    let mut st = c.prepare_cached(
        "SELECT track_id FROM download_files WHERE state = 'done' AND file IS NOT NULL ORDER BY random() LIMIT ?1",
    )?;
    let ids = st.query_map([SHUFFLE_MAX as i64], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(ids) })
}

/// Rows the car marks explicit (`CPListItem.explicitContent`): explicit
/// songs, albums with an explicit song, and Up next rows playing one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExplicitRows {
    tracks: HashSet<String>,
    albums: HashSet<String>,
    /// Up next rows, by queue item id.
    queue: HashSet<String>,
}

impl ExplicitRows {
    /// Whether a row gets the car's explicit mark.
    pub fn marks(&self, item: &CarItem) -> bool {
        match &item.action {
            CarAction::PlayFrom { track_id, .. } => self.tracks.contains(track_id),
            CarAction::Jump(item_id) => self.queue.contains(item_id),
            CarAction::Open(CarList::Songs { kind: ContextKind::Album, id, .. }) => self.albums.contains(id),
            CarAction::Play(ctx) | CarAction::Shuffle(ctx) if ctx.kind == ContextKind::Album => {
                ctx.id.as_ref().is_some_and(|id| self.albums.contains(id))
            }
            _ => false,
        }
    }

    /// Adds another list's marks.
    pub fn extend(&mut self, other: ExplicitRows) {
        self.tracks.extend(other.tracks);
        self.albums.extend(other.albums);
        self.queue.extend(other.queue);
    }
}

/// The explicit marks for these rows (and rows of covers in sections).
pub fn explicit_rows<'a>(c: &Connection, items: impl IntoIterator<Item = &'a CarItem>) -> AppResult<ExplicitRows> {
    let mut out = ExplicitRows::default();
    for item in items {
        match &item.action {
            CarAction::PlayFrom { track_id, .. } => {
                if crate::explicit::track_explicit(c, track_id)? {
                    out.tracks.insert(track_id.clone());
                }
            }
            CarAction::Open(CarList::Songs { kind: ContextKind::Album, id, .. }) => {
                if crate::explicit::album_explicit(c, id)? {
                    out.albums.insert(id.clone());
                }
            }
            CarAction::Play(ctx) | CarAction::Shuffle(ctx) if ctx.kind == ContextKind::Album => {
                if let Some(id) = &ctx.id
                    && crate::explicit::album_explicit(c, id)?
                {
                    out.albums.insert(id.clone());
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The explicit marks for a tab of sections.
pub fn explicit_sections(c: &Connection, sections: &[CarSection]) -> AppResult<ExplicitRows> {
    let items = sections.iter().flat_map(|s| s.rows.iter()).filter_map(|r| match r {
        CarRow::Item(i) => Some(i),
        CarRow::Covers { .. } => None,
    });
    explicit_rows(c, items)
}

/// The explicit marks for Up next ([`up_next`]).
pub fn explicit_queue(c: &Connection, q: &QueueView) -> AppResult<ExplicitRows> {
    let mut out = ExplicitRows::default();
    for i in q.user_queue.iter().chain(&q.next_up).take(LIST_MAX) {
        if crate::explicit::track_explicit(c, &i.track.id)? {
            out.queue.insert(i.item_id.clone());
        }
    }
    Ok(out)
}

pub mod siri;

#[cfg(target_os = "ios")]
pub mod ios;
#[cfg(target_os = "ios")]
pub mod siri_ios;

#[cfg(test)]
mod tests;
