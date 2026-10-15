//! Lyrics the user wrote travel through the store (the `lyrics` namespace).
//! The `user_lyrics` table stays what the lyrics views read; this keeps the
//! two in step. Lyrics transcribed on this computer stay on it until the
//! user edits and saves them.

use super::store::{LYRICS, SyncStore, UserLyricsEntry};
use crate::db::DbHandle;
use crate::error::AppResult;
use crate::lyrics::user;

/// First run with the store: the user's own lyrics go in with the time they
/// were saved.
pub fn seed(store: &SyncStore, db: &DbHandle) -> AppResult<()> {
    let ids = db.read(user::ids)?;
    for id in ids {
        let Some(own) = db.read(|c| user::get(c, &id))? else { continue };
        if own.auto.is_some() {
            continue;
        }
        let ts = (own.updated_at.max(1) as u64) * 1000;
        store.seed(&LYRICS, &id, &UserLyricsEntry { text: own.text, prefer: own.prefer }, ts)?;
    }
    Ok(())
}

/// After the user saves lyrics here.
pub fn saved(store: &SyncStore, track_id: &str, text: &str, prefer: bool) -> AppResult<()> {
    store.put(&LYRICS, track_id, &UserLyricsEntry { text: text.to_string(), prefer })
}

/// After the user deletes lyrics here.
pub fn deleted(store: &SyncStore, track_id: &str) -> AppResult<()> {
    store.remove(&LYRICS, track_id).map(|_| ())
}

/// Bring the table in line with the store (after the sync file brought
/// changes). Returns how many songs changed.
pub fn apply(store: &SyncStore, db: &DbHandle) -> AppResult<usize> {
    let mut n = 0;
    let now = (store.now() / 1000) as i64;
    for (id, e) in store.list(&LYRICS)? {
        if !user::valid_track_id(&id) {
            continue;
        }
        let cur = db.read(|c| user::get(c, &id))?;
        if cur.as_ref().is_some_and(|c| c.text == e.text && c.prefer == e.prefer && c.auto.is_none()) {
            continue;
        }
        let Ok(text) = user::clean(&e.text) else { continue };
        db.write(|c| user::put(c, &id, &text, e.prefer, now))?;
        n += 1;
    }
    for id in store.deleted(&LYRICS)? {
        let mine = db.read(|c| user::get(c, &id))?;
        if mine.is_some_and(|m| m.auto.is_none()) {
            db.write(|c| user::delete(c, &id))?;
            n += 1;
        }
    }
    Ok(n)
}
