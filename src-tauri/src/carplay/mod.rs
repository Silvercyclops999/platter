//! CarPlay: Platter on the car's screen (dev/docs/specs/phase-5/02-carplay.md).
//!
//! The car only gets Apple's templates: a tab bar of lists, and the shared
//! Now Playing screen (fed by the same session as the lock screen). This
//! module builds the lists from the library cache; `ios.rs` turns them into
//! templates and plays what is picked.

use rusqlite::Connection;

use crate::audio::source::{ContextKind, PlayContext};
use crate::db::queries;
use crate::db::types::{AlbumSort, ArtistSort, HomeItem};
use crate::error::AppResult;

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
    /// Open another list.
    Open(CarList),
}

/// The lists the car can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarList {
    RecentlyPlayed,
    RecentlyAdded,
    Playlists,
    Albums,
    Artists,
}

impl CarList {
    pub fn title(self) -> &'static str {
        match self {
            CarList::RecentlyPlayed => "Recently played",
            CarList::RecentlyAdded => "Recently added",
            CarList::Playlists => "Playlists",
            CarList::Albums => "Albums",
            CarList::Artists => "Artists",
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

/// The Home tab: what was played lately, then the shelves to dig into.
pub fn home(c: &Connection) -> AppResult<Vec<CarItem>> {
    let mut out: Vec<CarItem> = queries::recently_played(c, 8)?.into_iter().map(home_item).collect();
    out.push(open(CarList::RecentlyPlayed));
    out.push(open(CarList::RecentlyAdded));
    Ok(out)
}

/// The Library tab.
pub fn library() -> Vec<CarItem> {
    let liked = CarItem {
        title: "Liked songs".to_string(),
        subtitle: None,
        thumb: None,
        action: play(ContextKind::Liked, "liked"),
    };
    vec![
        liked,
        open(CarList::Playlists),
        open(CarList::Albums),
        open(CarList::Artists),
    ]
}

/// One of the lists a row opens, at most [`LIST_MAX`] long.
pub fn list(c: &Connection, which: CarList) -> AppResult<Vec<CarItem>> {
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
    })
}

fn album_item(a: crate::model::Album) -> CarItem {
    CarItem {
        subtitle: Some(a.artist_title.clone()),
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
        action: play(ContextKind::Artist, &a.id),
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

#[cfg(target_os = "ios")]
pub mod ios;

#[cfg(test)]
mod tests;
