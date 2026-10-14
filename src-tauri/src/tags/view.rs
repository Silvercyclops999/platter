//! Shapes library pages by what the file tags say (Settings: group album
//! editions, show fan and AI edits). Applied to query results, so the
//! queries themselves stay as they are.

use rusqlite::Connection;

use crate::db::types::{AlbumPage, ArtistPage, HomeData, HomeItem};
use crate::error::AppResult;

use super::TagView;
use super::store::{drop_edits, hidden_albums};

/// Artist page: editions behind their group's face and all-edit albums
/// leave the sections; a section left empty goes.
pub fn artist_page(c: &Connection, page: &mut ArtistPage, view: TagView) -> AppResult<()> {
    let hidden = hidden_albums(c, view)?;
    if hidden.is_empty() {
        return Ok(());
    }
    for s in &mut page.sections {
        s.albums.retain(|a| !hidden.contains(&a.id));
    }
    page.sections.retain(|s| !s.albums.is_empty());
    Ok(())
}

/// Album page: fan and AI edits leave the track list (unless shown), and
/// "More by" leaves out hidden editions and this album's own editions.
pub fn album_page(c: &Connection, page: &mut AlbumPage, view: TagView) -> AppResult<()> {
    drop_edits(c, &mut page.tracks, view)?;
    page.total_duration_ms = page.tracks.iter().map(|t| t.duration_ms).sum();
    let hidden = hidden_albums(c, view)?;
    let group: Option<String> = c
        .query_row("SELECT grp FROM album_tags WHERE album_id = ?1", [&page.album.id], |r| r.get(0))
        .unwrap_or(None);
    let mut same_group = std::collections::HashSet::new();
    if let Some(g) = group.filter(|_| view.group_editions) {
        let mut st = c.prepare_cached("SELECT album_id FROM album_tags WHERE grp = ?1")?;
        for id in st.query_map([g], |r| r.get::<_, String>(0))? {
            same_group.insert(id?);
        }
    }
    page.more_by_artist.retain(|a| !hidden.contains(&a.id) && !same_group.contains(&a.id));
    Ok(())
}

/// Home: the album shelves leave out hidden editions. Items you played stay.
pub fn home(c: &Connection, data: &mut HomeData, view: TagView) -> AppResult<()> {
    let hidden = hidden_albums(c, view)?;
    if hidden.is_empty() {
        return Ok(());
    }
    data.recently_added.retain(|a| !hidden.contains(&a.id));
    data.jump_back_in.retain(|a| !hidden.contains(&a.id));
    Ok(())
}

/// A Home "Show all" page, same rule as [`home`] for album shelves.
pub fn home_items(c: &Connection, items: &mut Vec<HomeItem>, view: TagView, albums_only_shelf: bool) -> AppResult<()> {
    if !albums_only_shelf {
        return Ok(());
    }
    let hidden = hidden_albums(c, view)?;
    items.retain(|i| !matches!(i, HomeItem::Album { album } if hidden.contains(&album.id)));
    Ok(())
}
