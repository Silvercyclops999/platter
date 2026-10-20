//! CarPlay: Platter on the car's screen.
//!
//! The car only gets Apple's templates: a tab bar of lists, and the shared
//! Now Playing screen (fed by the same session as the lock screen). This
//! module builds the lists from the library cache; `ios.rs` turns them into
//! templates and plays what is picked.

use rusqlite::Connection;

use crate::audio::queue::{QueueView, Repeat};
use crate::audio::source::{ContextKind, PlayContext};
use crate::db::queries;
use crate::db::types::{AlbumSort, ArtistSort, HomeItem};
use crate::error::AppResult;
use crate::plex::types::{Health, HealthStatus};

/// Most rows one CarPlay list shows; cars allow 100 to 500, and a long list
/// is hard to use while driving anyway.
pub const LIST_MAX: usize = 100;

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
    /// Shuffle every song: the songs are drawn when it's picked.
    ShuffleLibrary,
    /// Open another list.
    Open(CarList),
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
    /// One artist: Popular, then their albums.
    Artist { id: String, name: String },
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
        }
    }
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
    CarItem { title: "Liked songs".to_string(), subtitle: None, thumb: None, action: play(ContextKind::Liked, "liked") }
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
            action: play(ContextKind::Mix, &m.id),
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

/// One of the lists a row opens, at most [`LIST_MAX`] long.
pub fn list(c: &Connection, which: &CarList) -> AppResult<Vec<CarItem>> {
    Ok(match which {
        CarList::RecentlyPlayed => queries::recently_played(c, LIST_MAX)?.into_iter().map(home_item).collect(),
        CarList::RecentlyAdded => queries::recently_added(c, LIST_MAX)?.into_iter().map(album_item).collect(),
        CarList::Playlists => queries::home_playlists(c, LIST_MAX)?.into_iter().map(playlist_item).collect(),
        CarList::Albums => queries::albums(c, AlbumSort::Recent, 0, LIST_MAX as u32, None)?
            .items
            .into_iter()
            .map(album_item)
            .collect(),
        CarList::Artists => queries::artists(c, ArtistSort::Plays, 0, LIST_MAX as u32)?
            .items
            .into_iter()
            .map(artist_item)
            .collect(),
        CarList::Artist { id, .. } => artist(c, id)?,
    })
}

/// An artist's list: Popular (which plays on through the albums, as the
/// artist page's play does), then each album.
fn artist(c: &Connection, id: &str) -> AppResult<Vec<CarItem>> {
    let popular = CarItem {
        title: "Popular".to_string(),
        subtitle: None,
        thumb: None,
        action: play(ContextKind::Artist, id),
    };
    let albums = queries::artist_albums(c, id, LIST_MAX - 1)?;
    // On the artist's own list the name says nothing new: the year and the
    // kind of release do.
    Ok(std::iter::once(popular).chain(albums.into_iter().map(artist_album_item)).collect())
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
                None => CarItem { title: r.title, subtitle: None, thumb: None, action: play(ContextKind::Album, &r.key.id) },
            },
            ItemKind::Playlist => CarItem {
                title: r.title,
                subtitle: Some("Playlist".into()),
                thumb: None,
                action: play(ContextKind::Playlist, &r.key.id),
            },
            ItemKind::Liked => CarItem {
                title: "Liked songs".into(),
                subtitle: None,
                thumb: None,
                action: play(ContextKind::Liked, "liked"),
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
        action: play(ContextKind::Album, &a.id),
        title: a.title,
    }
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
        action: play(ContextKind::Album, &a.id),
        title: a.title,
    }
}

fn playlist_item(p: crate::model::Playlist) -> CarItem {
    let songs = if p.track_count == 1 { "1 song".to_string() } else { format!("{} songs", p.track_count) };
    CarItem {
        subtitle: Some(songs),
        thumb: p.thumb.clone(),
        action: play(ContextKind::Playlist, &p.id),
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

/// What's playing, as far as marking rows goes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NowKey {
    /// The context the queue plays (album, playlist, mix...), with its id.
    pub context: Option<(ContextKind, String)>,
    /// The playing song's album.
    pub album_id: Option<String>,
}

/// Whether a row plays what's playing now, to show it as playing: the same
/// album, playlist, mix or Liked songs, or the album the playing song is on.
pub fn is_playing(item: &CarItem, now: &NowKey) -> bool {
    let (CarAction::Play(ctx) | CarAction::Shuffle(ctx)) = &item.action else { return false };
    let Some(id) = &ctx.id else { return false };
    if ctx.track_ids.is_some() {
        return false;
    }
    if now.context.as_ref().is_some_and(|(k, cid)| *k == ctx.kind && cid == id) {
        return true;
    }
    ctx.kind == ContextKind::Album && now.album_id.as_deref() == Some(id.as_str())
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

pub mod siri;

#[cfg(target_os = "ios")]
pub mod ios;
#[cfg(target_os = "ios")]
pub mod siri_ios;

#[cfg(test)]
mod tests;
