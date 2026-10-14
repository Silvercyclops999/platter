//! Similar artists for the artist page.
//!
//! Plex only returns `Similar` in artist detail (`/library/metadata/{id}`),
//! not on the list items sync pages through, so fetching it during sync would
//! cost one call per artist. Instead it's fetched when an artist page opens,
//! stored in `artists.similar` with `similar_fetched_at`, and reused for
//! [`SIMILAR_TTL_S`]. The page shows what's cached at once; a missing or stale
//! entry refreshes in the background and `DbEvents::artist_changed` tells the
//! page to pick up the new list.

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;

use super::DbHandle;
use super::queries::now_s;
use super::source::{LibrarySource, SimilarArtist};

/// How long fetched similar artists are reused before a refresh.
pub const SIMILAR_TTL_S: i64 = 7 * 24 * 3600;

/// Whether the artist's similar list is missing or older than the TTL.
/// Unknown artists never need one.
pub fn is_stale(c: &Connection, id: &str, now: i64) -> AppResult<bool> {
    let at: Option<Option<i64>> = c
        .query_row("SELECT similar_fetched_at FROM artists WHERE id = ?1", [id], |r| r.get(0))
        .optional()?;
    Ok(match at {
        None => false,
        Some(None) => true,
        Some(Some(t)) => now - t >= SIMILAR_TTL_S,
    })
}

/// Store a fetched list (empty lists too, so they aren't refetched daily).
pub fn store(c: &Connection, id: &str, list: &[SimilarArtist], now: i64) -> AppResult<()> {
    let json = serde_json::to_string(list).unwrap_or_else(|_| "[]".into());
    c.execute(
        "UPDATE artists SET similar = ?2, similar_fetched_at = ?3 WHERE id = ?1",
        params![id, json, now],
    )?;
    Ok(())
}

/// Fetch and store the artist's similar artists if they're stale. Returns
/// whether a fetch ran and the stored list changed. Errors (offline server)
/// leave the cache as it was; the next page open tries again.
pub async fn refresh_if_stale(db: &DbHandle, source: &dyn LibrarySource, id: &str, now: i64) -> AppResult<bool> {
    let key = id.to_string();
    if !db.read_async(move |c| is_stale(c, &key, now)).await? {
        return Ok(false);
    }
    // One call per artist at a time.
    if !db.begin_similar_fetch(id) {
        return Ok(false);
    }
    let result = async {
        let list = source.artist_similar(id).await?;
        let key = id.to_string();
        db.write_async(move |c| {
            let before: String =
                c.query_row("SELECT similar FROM artists WHERE id = ?1", [&key], |r| r.get(0))?;
            store(c, &key, &list, now)?;
            let after = serde_json::to_string(&list).unwrap_or_default();
            Ok(before != after)
        })
        .await
    }
    .await;
    db.end_similar_fetch(id);
    let changed = result?;
    if changed && let Some(ev) = db.events() {
        ev.artist_changed(id);
    }
    Ok(changed)
}

/// [`refresh_if_stale`] on a background task, for the `artist_page` command.
/// Does nothing without a library source (signed out, tests).
pub fn spawn_refresh_if_stale(db: DbHandle, id: String) {
    let Some(source) = db.source() else { return };
    spawn_with(db, source, id);
}

fn spawn_with(db: DbHandle, source: Arc<dyn LibrarySource>, id: String) {
    tokio::spawn(async move {
        if let Err(e) = refresh_if_stale(&db, source.as_ref(), &id, now_s()).await {
            tracing::debug!("similar artists for {id}: {e}");
        }
    });
}
