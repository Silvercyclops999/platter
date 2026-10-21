//! Playlist edits in the local cache and the queue of edits waiting to
//! reach Plex (the `playlists` module replays it).
//!
//! Every edit is applied here first, so the page updates at once and works
//! offline. Rows are named by Plex's playlistItemID; rows Plex hasn't given
//! an id yet (just added, or cached before ids were kept) carry a local
//! `local:…` id until the next refresh from Plex.
//!
//! A playlist made here stays local only (`local_only = 1`) until it has a
//! song: Plex creates playlists from a list of items. Its single queued
//! Create op sends the playlist as it is at that moment, so edits to a
//! local-only playlist aren't queued one by one. Once created, the local id
//! maps to the Plex id in `playlist_aliases` and both keep working.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::model::Playlist;

use super::source::PlaylistEntry;
use super::store::{PLAYLIST_COLS, row_playlist, set_playlist_items, upsert_playlist};

pub const LOCAL_PREFIX: &str = "local:";
/// Longest name and description the edit dialog allows.
pub const TITLE_MAX: usize = 100;
pub const SUMMARY_MAX: usize = 300;

/// A new local id for a playlist or a row.
pub fn local_item_id() -> String {
    format!("{LOCAL_PREFIX}{}", uuid::Uuid::new_v4().simple())
}

pub fn is_local(id: &str) -> bool {
    id.starts_with(LOCAL_PREFIX)
}

fn now_s() -> i64 {
    super::queries::now_s()
}

// ---------------------------------------------------------------- reads

/// The id to use now: a local id that has been created in Plex maps to its
/// Plex id.
pub fn resolve_id(c: &Connection, id: &str) -> AppResult<String> {
    if !is_local(id) {
        return Ok(id.to_string());
    }
    let mut st = c.prepare_cached("SELECT plex_id FROM playlist_aliases WHERE local_id = ?1")?;
    Ok(st.query_row([id], |r| r.get(0)).optional()?.unwrap_or_else(|| id.to_string()))
}

/// One row of a playlist as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub item_id: String,
    pub track_id: String,
}

pub fn items(c: &Connection, playlist_id: &str) -> AppResult<Vec<Item>> {
    let mut st = c.prepare_cached(
        "SELECT item_id, track_id FROM playlist_items WHERE playlist_id = ?1 ORDER BY position",
    )?;
    let rows = st
        .query_map([playlist_id], |r| {
            Ok(Item { item_id: r.get::<_, Option<String>>(0)?.unwrap_or_default(), track_id: r.get(1)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn playlist(c: &Connection, id: &str) -> AppResult<Option<Playlist>> {
    Ok(c
        .query_row(&format!("SELECT {PLAYLIST_COLS} FROM playlists p WHERE p.id = ?1"), [id], row_playlist)
        .optional()?)
}

pub fn is_local_only(c: &Connection, id: &str) -> AppResult<bool> {
    Ok(c
        .query_row("SELECT local_only FROM playlists WHERE id = ?1", [id], |r| r.get::<_, bool>(0))
        .optional()?
        .unwrap_or(false))
}

/// The playlist to edit: resolves the id and refuses smart playlists.
pub fn editable(c: &Connection, id: &str) -> AppResult<Playlist> {
    let id = resolve_id(c, id)?;
    let p = playlist(c, &id)?.ok_or_else(|| AppError::NotFound(format!("playlist {id}")))?;
    if p.smart {
        return Err(AppError::Invalid("Smart playlists can't be changed here.".into()));
    }
    Ok(p)
}

/// Which of `track_ids` are already in the playlist (each once, in the
/// order asked).
pub fn duplicates(c: &Connection, id: &str, track_ids: &[String]) -> AppResult<Vec<String>> {
    let id = resolve_id(c, id)?;
    let have: std::collections::HashSet<String> = items(c, &id)?.into_iter().map(|i| i.track_id).collect();
    let mut out: Vec<String> = Vec::new();
    for t in track_ids {
        if have.contains(t) && !out.contains(t) {
            out.push(t.clone());
        }
    }
    Ok(out)
}

/// The name for a new playlist from the Settings template. `#N` becomes the
/// next number no playlist uses yet ("My playlist #3"); a template without
/// `#N` is used as it is.
pub fn next_name(c: &Connection, template: &str) -> AppResult<String> {
    let template = template.trim();
    let template = if template.is_empty() { DEFAULT_NAME } else { template };
    let Some((before, after)) = template.split_once("#N") else {
        return Ok(template.to_string());
    };
    let mut st = c.prepare_cached("SELECT title FROM playlists")?;
    let titles: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let used: std::collections::HashSet<u32> = titles
        .iter()
        .filter_map(|t| t.strip_prefix(&format!("{before}#"))?.strip_suffix(after)?.parse().ok())
        .collect();
    let n = (1..).find(|n| !used.contains(n)).unwrap_or(1);
    Ok(format!("{before}#{n}{after}"))
}

pub const DEFAULT_NAME: &str = "My playlist #N";

// ---------------------------------------------------------------- local edits

fn write_items(c: &Connection, playlist_id: &str, items: &[Item]) -> AppResult<()> {
    let entries: Vec<PlaylistEntry> = items
        .iter()
        .map(|i| PlaylistEntry { track_id: i.track_id.clone(), item_id: Some(i.item_id.clone()) })
        .collect();
    set_playlist_items(c, playlist_id, &entries)?;
    touch(c, playlist_id)
}

/// Recount songs and length and mark the playlist changed now.
fn touch(c: &Connection, playlist_id: &str) -> AppResult<()> {
    c.execute(
        "UPDATE playlists SET
           track_count = (SELECT count(*) FROM playlist_items WHERE playlist_id = ?1),
           duration_ms = (SELECT coalesce(sum(t.duration_ms), 0) FROM playlist_items pi
                          JOIN tracks t ON t.id = pi.track_id WHERE pi.playlist_id = ?1),
           updated_at = ?2
         WHERE id = ?1",
        params![playlist_id, now_s()],
    )?;
    Ok(())
}

/// A new playlist that only exists here until it has a song.
pub fn create_local(c: &Connection, title: &str) -> AppResult<Playlist> {
    let id = local_item_id();
    c.execute(
        "INSERT INTO playlists (id, title, smart, track_count, duration_ms, local_only, updated_at)
         VALUES (?1, ?2, 0, 0, 0, 1, ?3)",
        params![id, title, now_s()],
    )?;
    playlist(c, &id)?.ok_or_else(|| AppError::Internal("playlist vanished".into()))
}

/// Where added songs go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    End,
    Top,
    After(String),
}

/// Add songs; returns the new rows' (local) ids in order.
pub fn add(c: &Connection, playlist_id: &str, track_ids: &[String], place: &Place) -> AppResult<Vec<String>> {
    let mut list = items(c, playlist_id)?;
    let new: Vec<Item> = track_ids
        .iter()
        .map(|t| Item { item_id: local_item_id(), track_id: t.clone() })
        .collect();
    let at = match place {
        Place::End => list.len(),
        Place::Top => 0,
        Place::After(after) => {
            let after = resolve_item(c, after)?;
            list.iter().position(|i| i.item_id == after).map(|p| p + 1).unwrap_or(list.len())
        }
    };
    let ids = new.iter().map(|i| i.item_id.clone()).collect();
    list.splice(at..at, new);
    write_items(c, playlist_id, &list)?;
    Ok(ids)
}

/// A removed row and where it was (for undo and for naming it to Plex).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemRef {
    pub item_id: String,
    pub track_id: String,
    pub index: usize,
}

/// Remove rows by id; unknown ids are ignored.
pub fn remove(c: &Connection, playlist_id: &str, item_ids: &[String]) -> AppResult<Vec<ItemRef>> {
    let ids: Vec<String> = item_ids.iter().map(|i| resolve_item(c, i)).collect::<AppResult<_>>()?;
    let list = items(c, playlist_id)?;
    let mut removed = Vec::new();
    let mut kept = Vec::with_capacity(list.len());
    for (index, it) in list.into_iter().enumerate() {
        if ids.contains(&it.item_id) {
            removed.push(ItemRef { item_id: it.item_id, track_id: it.track_id, index });
        } else {
            kept.push(it);
        }
    }
    if !removed.is_empty() {
        write_items(c, playlist_id, &kept)?;
    }
    Ok(removed)
}

/// One step for Plex: put `item` right after `after` (None: at the top).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub item: ItemRef,
    pub after: Option<ItemRef>,
}

/// Move rows (kept in their current order) to just after `after` (None:
/// the top). Returns the Plex moves that give the same order.
pub fn move_items(c: &Connection, playlist_id: &str, item_ids: &[String], after: Option<&str>) -> AppResult<Vec<Move>> {
    let ids: Vec<String> = item_ids.iter().map(|i| resolve_item(c, i)).collect::<AppResult<_>>()?;
    let after = after.map(|a| resolve_item(c, a)).transpose()?;
    if after.as_ref().is_some_and(|a| ids.contains(a)) {
        return Err(AppError::Invalid("Can't move songs after one of themselves.".into()));
    }
    let list = items(c, playlist_id)?;
    let original: Vec<Item> = list.clone();
    let (moving, mut rest): (Vec<Item>, Vec<Item>) = list.into_iter().partition(|i| ids.contains(&i.item_id));
    if moving.is_empty() {
        return Ok(Vec::new());
    }
    let at = match &after {
        None => 0,
        Some(a) => rest
            .iter()
            .position(|i| &i.item_id == a)
            .map(|p| p + 1)
            .ok_or_else(|| AppError::NotFound("playlist item".into()))?,
    };
    let new_order: Vec<Item> = {
        let tail = rest.split_off(at);
        rest.into_iter().chain(moving.iter().cloned()).chain(tail).collect()
    };
    if new_order == original {
        return Ok(Vec::new());
    }
    // Plex moves one row at a time: each moved row goes after the row
    // before it in the new order. Hints use the order before the move.
    let hint = |id: &str| {
        original.iter().position(|i| i.item_id == id).map(|index| ItemRef {
            item_id: id.to_string(),
            track_id: original[index].track_id.clone(),
            index,
        })
    };
    let mut moves = Vec::new();
    for m in &moving {
        let pos = new_order.iter().position(|i| i.item_id == m.item_id).unwrap_or(0);
        let prev = pos.checked_sub(1).and_then(|p| hint(&new_order[p].item_id));
        if let Some(item) = hint(&m.item_id) {
            moves.push(Move { item, after: prev });
        }
    }
    write_items(c, playlist_id, &new_order)?;
    Ok(moves)
}

pub fn update_details(c: &Connection, playlist_id: &str, title: Option<&str>, summary: Option<&str>) -> AppResult<()> {
    if let Some(t) = title {
        c.execute("UPDATE playlists SET title = ?2 WHERE id = ?1", params![playlist_id, t])?;
    }
    if let Some(s) = summary {
        let s = if s.is_empty() { None } else { Some(s) };
        c.execute("UPDATE playlists SET summary = ?2 WHERE id = ?1", params![playlist_id, s])?;
    }
    c.execute("UPDATE playlists SET updated_at = ?2 WHERE id = ?1", params![playlist_id, now_s()])?;
    Ok(())
}

pub fn delete(c: &Connection, playlist_id: &str) -> AppResult<()> {
    c.execute("DELETE FROM playlists WHERE id = ?1", [playlist_id])?;
    Ok(())
}

/// A local row id that has since been given a Plex id maps to it.
fn resolve_item(c: &Connection, id: &str) -> AppResult<String> {
    resolve_id(c, id)
}

// ---------------------------------------------------------------- from Plex

/// Replace the cached playlist with what Plex has.
pub fn apply_remote(c: &Connection, p: &Playlist, entries: &[PlaylistEntry]) -> AppResult<()> {
    upsert_playlist(c, p)?;
    set_playlist_items(c, &p.id, entries)
}

/// The playlist made here now exists in Plex as `plex_id`: move its rows
/// and queued edits over and remember the mapping.
pub fn adopt(c: &Connection, local_id: &str, plex_id: &str) -> AppResult<()> {
    if local_id != plex_id {
        // A sync may already have cached the new playlist; ours wins.
        c.execute("DELETE FROM playlists WHERE id = ?1", [plex_id])?;
        c.execute(
            "INSERT INTO playlists (id, title, smart, track_count, duration_ms, thumb, summary, updated_at, local_only)
             SELECT ?2, title, smart, track_count, duration_ms, thumb, summary, updated_at, 0
             FROM playlists WHERE id = ?1",
            params![local_id, plex_id],
        )?;
        c.execute("UPDATE playlist_items SET playlist_id = ?2 WHERE playlist_id = ?1", params![local_id, plex_id])?;
        c.execute("DELETE FROM playlists WHERE id = ?1", [local_id])?;
        c.execute("UPDATE playlist_ops SET playlist_id = ?2 WHERE playlist_id = ?1", params![local_id, plex_id])?;
    }
    map_id(c, local_id, plex_id)
}

/// Remember that a local id (playlist or row) is now `plex_id` and rewrite
/// queued edits that name it.
pub fn map_id(c: &Connection, local_id: &str, plex_id: &str) -> AppResult<()> {
    c.execute(
        "INSERT OR REPLACE INTO playlist_aliases (local_id, plex_id) VALUES (?1, ?2)",
        params![local_id, plex_id],
    )?;
    c.execute("UPDATE playlist_items SET item_id = ?2 WHERE item_id = ?1", params![local_id, plex_id])?;
    let rows: Vec<(i64, String)> = {
        let mut st = c.prepare_cached("SELECT seq, op FROM playlist_ops WHERE instr(op, ?1) > 0")?;
        st.query_map([local_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
    };
    for (seq, json) in rows {
        let quoted_from = format!("\"{local_id}\"");
        let quoted_to = format!("\"{plex_id}\"");
        c.execute(
            "UPDATE playlist_ops SET op = ?2 WHERE seq = ?1",
            params![seq, json.replace(&quoted_from, &quoted_to)],
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------- queue

/// One edit for Plex. Rows are named by item id plus where they were, so a
/// row whose Plex id isn't known here can still be found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Op {
    /// Create the local-only playlist in Plex as it is when this replays.
    Create,
    /// A create sent at `at` (unix seconds) without a clear answer: it may
    /// have landed, so the next try looks for it (by the title and songs it
    /// went with) before making another.
    #[serde(rename_all = "camelCase")]
    CreateSent { at: i64, title: String, track_ids: Vec<String> },
    /// Append songs; `item_ids` are the local ids the new rows got here.
    #[serde(rename_all = "camelCase")]
    Add {
        track_ids: Vec<String>,
        item_ids: Vec<String>,
        /// Sent once already without a clear answer: it may have landed.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        sent: bool,
    },
    Remove { item: ItemRef },
    Move { item: ItemRef, after: Option<ItemRef> },
    Update { title: Option<String>, summary: Option<String> },
    #[serde(rename_all = "camelCase")]
    Poster { bytes: Vec<u8>, content_type: String },
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedOp {
    pub seq: i64,
    pub playlist_id: String,
    pub op: Op,
    pub attempts: u32,
}

pub fn enqueue(c: &Connection, playlist_id: &str, op: &Op) -> AppResult<()> {
    let json = serde_json::to_string(op).map_err(|e| AppError::Internal(e.to_string()))?;
    c.execute(
        "INSERT INTO playlist_ops (playlist_id, op, created_at) VALUES (?1, ?2, ?3)",
        params![playlist_id, json, now_s()],
    )?;
    Ok(())
}

/// Queued edits, oldest first. Ones that no longer parse are dropped.
pub fn queued(c: &Connection) -> AppResult<Vec<QueuedOp>> {
    let mut st = c.prepare_cached("SELECT seq, playlist_id, op, attempts FROM playlist_ops ORDER BY seq")?;
    let rows: Vec<(i64, String, String, u32)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(seq, playlist_id, json, attempts)| match serde_json::from_str(&json) {
            Ok(op) => Some(QueuedOp { seq, playlist_id, op, attempts }),
            Err(e) => {
                tracing::warn!("dropping unreadable playlist edit {seq}: {e}");
                None
            }
        })
        .collect())
}

pub fn has_queued(c: &Connection, playlist_id: &str) -> AppResult<bool> {
    Ok(c
        .query_row("SELECT 1 FROM playlist_ops WHERE playlist_id = ?1 LIMIT 1", [playlist_id], |_| Ok(()))
        .optional()?
        .is_some())
}

pub fn drop_op(c: &Connection, seq: i64) -> AppResult<()> {
    c.execute("DELETE FROM playlist_ops WHERE seq = ?1", [seq])?;
    Ok(())
}

/// Drop every queued edit for a playlist (Plex refused one, or it's gone).
pub fn drop_ops_for(c: &Connection, playlist_id: &str) -> AppResult<()> {
    c.execute("DELETE FROM playlist_ops WHERE playlist_id = ?1", [playlist_id])?;
    Ok(())
}

/// Drop a created playlist's queued edits other than a cover: the create
/// sent everything else.
pub fn drop_ops_after_create(c: &Connection, playlist_id: &str) -> AppResult<()> {
    let seqs: Vec<i64> = queued(c)?
        .into_iter()
        .filter(|q| q.playlist_id == playlist_id && !matches!(q.op, Op::Poster { .. }))
        .map(|q| q.seq)
        .collect();
    for s in seqs {
        drop_op(c, s)?;
    }
    Ok(())
}

/// Marks a queued add or create as sent, before it goes out: if no clear
/// answer comes back, the next try looks before sending again.
pub fn mark_sent(c: &Connection, seq: i64) -> AppResult<()> {
    let row: Option<(String, String)> =
        c.query_row("SELECT playlist_id, op FROM playlist_ops WHERE seq = ?1", [seq], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((pid, json)) = row else { return Ok(()) };
    let mut op: Op = serde_json::from_str(&json).map_err(|e| AppError::Internal(e.to_string()))?;
    match &mut op {
        Op::Add { sent, .. } => *sent = true,
        Op::Create => {
            let title = playlist(c, &pid)?.map(|p| p.title).unwrap_or_default();
            let track_ids = items(c, &pid)?.into_iter().map(|i| i.track_id).collect();
            op = Op::CreateSent { at: now_s(), title, track_ids };
        }
        _ => {}
    }
    let json = serde_json::to_string(&op).map_err(|e| AppError::Internal(e.to_string()))?;
    c.execute("UPDATE playlist_ops SET op = ?2 WHERE seq = ?1", params![seq, json])?;
    Ok(())
}

pub fn bump_attempts(c: &Connection, seq: i64) -> AppResult<u32> {
    c.execute("UPDATE playlist_ops SET attempts = attempts + 1 WHERE seq = ?1", [seq])?;
    Ok(c.query_row("SELECT attempts FROM playlist_ops WHERE seq = ?1", [seq], |r| r.get(0)).optional()?.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbHandle;

    fn db() -> (DbHandle, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("t.db")).unwrap();
        db.write(|c| {
            for (id, ms) in [("t1", 1000), ("t2", 2000), ("t3", 3000), ("t4", 4000)] {
                c.execute(
                    "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number,
                       disc_number, duration_ms, part_key) VALUES (?1, ?1, 'a', 'A', 'r', 'R', 1, 1, ?2, '/p')",
                    params![id, ms],
                )?;
            }
            Ok(())
        })
        .unwrap();
        (db, dir)
    }

    fn tracks_of(c: &Connection, id: &str) -> Vec<String> {
        items(c, id).unwrap().into_iter().map(|i| i.track_id).collect()
    }

    #[test]
    fn next_name_takes_the_first_free_number() {
        let (db, _d) = db();
        db.write(|c| {
            assert_eq!(next_name(c, DEFAULT_NAME)?, "My playlist #1");
            create_local(c, "My playlist #1")?;
            create_local(c, "My playlist #3")?;
            assert_eq!(next_name(c, DEFAULT_NAME)?, "My playlist #2");
            create_local(c, "My playlist #2")?;
            assert_eq!(next_name(c, DEFAULT_NAME)?, "My playlist #4");
            assert_eq!(next_name(c, "Mix #N (new)")?, "Mix #1 (new)");
            assert_eq!(next_name(c, "Untitled")?, "Untitled");
            assert_eq!(next_name(c, "  ")?, "My playlist #4");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn add_appends_or_inserts_and_counts() {
        let (db, _d) = db();
        db.write(|c| {
            let p = create_local(c, "P")?;
            let first = add(c, &p.id, &["t1".into(), "t2".into()], &Place::End)?;
            add(c, &p.id, &["t3".into()], &Place::End)?;
            assert_eq!(tracks_of(c, &p.id), ["t1", "t2", "t3"]);
            add(c, &p.id, &["t4".into()], &Place::After(first[0].clone()))?;
            assert_eq!(tracks_of(c, &p.id), ["t1", "t4", "t2", "t3"]);
            add(c, &p.id, &["t2".into()], &Place::Top)?;
            assert_eq!(tracks_of(c, &p.id), ["t2", "t1", "t4", "t2", "t3"]);
            let p = playlist(c, &p.id)?.unwrap();
            assert_eq!((p.track_count, p.duration_ms), (5, 12_000));
            assert!(p.updated_at.is_some());
            assert_eq!(duplicates(c, &p.id, &["t2".into(), "t9".into(), "t2".into()])?, ["t2"]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn remove_reports_where_rows_were() {
        let (db, _d) = db();
        db.write(|c| {
            let p = create_local(c, "P")?;
            let ids = add(c, &p.id, &["t1".into(), "t2".into(), "t3".into()], &Place::End)?;
            let gone = remove(c, &p.id, &[ids[2].clone(), ids[0].clone(), "nope".into()])?;
            assert_eq!(gone.iter().map(|g| (g.track_id.as_str(), g.index)).collect::<Vec<_>>(), [("t1", 0), ("t3", 2)]);
            assert_eq!(tracks_of(c, &p.id), ["t2"]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn move_gives_plex_steps_that_rebuild_the_order() {
        let (db, _d) = db();
        db.write(|c| {
            let p = create_local(c, "P")?;
            let ids = add(c, &p.id, &["t1".into(), "t2".into(), "t3".into(), "t4".into()], &Place::End)?;
            // Last to the top.
            let moves = move_items(c, &p.id, &[ids[3].clone()], None)?;
            assert_eq!(tracks_of(c, &p.id), ["t4", "t1", "t2", "t3"]);
            assert_eq!(moves.len(), 1);
            assert_eq!(moves[0].after, None);
            // Two rows after t2, keeping their order (t4, t1).
            let moves = move_items(c, &p.id, &[ids[0].clone(), ids[3].clone()], Some(&ids[1]))?;
            assert_eq!(tracks_of(c, &p.id), ["t2", "t4", "t1", "t3"]);
            let steps: Vec<(String, Option<String>)> =
                moves.iter().map(|m| (m.item.track_id.clone(), m.after.as_ref().map(|a| a.track_id.clone()))).collect();
            assert_eq!(steps, [("t4".into(), Some("t2".into())), ("t1".into(), Some("t4".into()))]);
            // A no-op move asks Plex for nothing.
            assert!(move_items(c, &p.id, &[ids[1].clone()], None)?.is_empty());
            assert!(move_items(c, &p.id, &[ids[1].clone()], Some(&ids[1])).is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn adopt_moves_rows_and_ops_and_keeps_the_old_id_working() {
        let (db, _d) = db();
        db.write(|c| {
            let p = create_local(c, "P")?;
            let ids = add(c, &p.id, &["t1".into()], &Place::End)?;
            enqueue(c, &p.id, &Op::Create)?;
            enqueue(
                c,
                &p.id,
                &Op::Remove { item: ItemRef { item_id: ids[0].clone(), track_id: "t1".into(), index: 0 } },
            )?;
            adopt(c, &p.id, "900")?;
            map_id(c, &ids[0], "77")?;
            assert_eq!(resolve_id(c, &p.id)?, "900");
            assert!(!is_local_only(c, "900")?);
            assert_eq!(items(c, "900")?, [Item { item_id: "77".into(), track_id: "t1".into() }]);
            let q = queued(c)?;
            assert!(q.iter().all(|q| q.playlist_id == "900"));
            assert!(matches!(&q[1].op, Op::Remove { item } if item.item_id == "77"));
            assert_eq!(crate::db::queries::playlist_page(c, &p.id)?.playlist.id, "900");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn smart_playlists_are_not_editable() {
        let (db, _d) = db();
        db.write(|c| {
            c.execute("INSERT INTO playlists (id, title, smart) VALUES ('s', 'Smart', 1)", [])?;
            assert!(matches!(editable(c, "s"), Err(AppError::Invalid(_))));
            assert!(matches!(editable(c, "missing"), Err(AppError::NotFound(_))));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn ops_round_trip_through_the_queue() {
        let (db, _d) = db();
        db.write(|c| {
            enqueue(c, "1", &Op::Add { track_ids: vec!["t1".into()], item_ids: vec!["local:x".into()], sent: false })?;
            enqueue(c, "1", &Op::Update { title: Some("N".into()), summary: None })?;
            enqueue(c, "2", &Op::Delete)?;
            let q = queued(c)?;
            assert_eq!(q.len(), 3);
            assert!(has_queued(c, "1")?);
            assert_eq!(bump_attempts(c, q[0].seq)?, 1);
            drop_ops_for(c, "1")?;
            assert_eq!(queued(c)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }
}
