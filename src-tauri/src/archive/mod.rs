//! The archive, read side: what's released and why, the user's marks, each
//! song's own status, other versions of a song, artist page sections and
//! eras. Nothing here writes to Plex or to files.
//!
//! - [`status`]   each album's release status with every piece of evidence
//!   behind it (`album_status`), for "Why?" and everything that sorts by it
//! - [`marks`]    the user's marks and custom sections, in the sync store
//! - [`versions`] songs grouped with their other versions (`versions`), each
//!   song's own status, the versions panel and search grouping
//! - [`sections`] artist page sections by release type and status, plus the
//!   user's own
//! - [`lineage`]  every version of a song on one timeline, by date
//! - [`eras`]     ARCHIVE_ERA and ARCHIVE_DATE: era cards, the era page and
//!   the album page's era banner
//! - [`commands`] the Tauri commands
//!
//! The release rule itself lives in `db::release`, and song folding in
//! `identity::fold`: this module only uses them.

pub mod commands;
pub mod eras;
pub mod lineage;
pub mod marks;
pub mod sections;
pub mod status;
pub mod versions;

#[cfg(test)]
mod tests;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

/// An album moved to a section of the user's own (the sync store).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomSection {
    pub name: String,
    /// The album's artist, so an artist's sections list without a lookup.
    pub artist_id: String,
}

/// Rebuild every album's status, then every song's versions and status.
/// After a library sync and when a setting that feeds the rule changes.
pub fn refresh_all(c: &Connection) -> AppResult<()> {
    status::refresh(c, None)?;
    versions::rebuild(c)?;
    Ok(())
}

/// The same for these album artists only (file tags read, a mark set).
pub fn refresh_artists(c: &Connection, artist_ids: &[String]) -> AppResult<()> {
    if artist_ids.is_empty() {
        return Ok(());
    }
    status::refresh(c, Some(artist_ids))?;
    versions::refresh_artists(c, artist_ids)?;
    Ok(())
}

/// [`refresh_artists`] for the album artists of these songs.
pub fn refresh_for_tracks(c: &Connection, track_ids: &[String]) -> AppResult<()> {
    let ids = artists_of(c, "SELECT DISTINCT artist_id FROM tracks WHERE id IN (SELECT value FROM json_each(?1))", track_ids)?;
    refresh_artists(c, &ids)
}

/// [`refresh_artists`] for the artists of these albums.
pub fn refresh_for_albums(c: &Connection, album_ids: &[String]) -> AppResult<()> {
    let ids = artists_of(c, "SELECT DISTINCT artist_id FROM albums WHERE id IN (SELECT value FROM json_each(?1))", album_ids)?;
    refresh_artists(c, &ids)
}

fn artists_of(c: &Connection, sql: &str, ids: &[String]) -> AppResult<Vec<String>> {
    let list = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
    let mut st = c.prepare_cached(sql)?;
    let out = st.query_map([list], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Rebuild everything in the background, on the writer (a setting that
/// feeds the rule changed).
pub fn refresh_soon(db: &crate::db::DbHandle) {
    let db = db.clone();
    std::thread::spawn(move || {
        if let Err(e) = db.write(|c| in_tx(c, refresh_all)) {
            tracing::warn!("archive refresh failed: {e}");
        }
    });
}

/// Run `f` in one transaction.
pub fn in_tx(c: &mut Connection, f: impl FnOnce(&Connection) -> AppResult<()>) -> AppResult<()> {
    let tx = c.transaction()?;
    f(&tx)?;
    tx.commit()?;
    Ok(())
}
