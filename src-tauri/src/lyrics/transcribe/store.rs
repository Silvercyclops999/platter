//! Where transcriptions go. Found words are saved in `user_lyrics` as
//! enhanced LRC (line and word times) with `auto` set to the model, so the
//! lyrics editor opens them like any other lyrics (see lyrics::user). Songs
//! transcribed with no words found go in `lyrics_auto_empty`, so the batch
//! doesn't redo them.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;
use crate::lyrics::user;

/// Save the result. An empty body records "no words found". Returns false
/// when the user's own lyrics were there and were kept.
pub fn save(c: &Connection, track_id: &str, body: &str, model: &str, now: i64) -> AppResult<bool> {
    if body.trim().is_empty() {
        c.execute(
            "INSERT INTO lyrics_auto_empty (track_id, model, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (track_id) DO UPDATE SET model = excluded.model, created_at = excluded.created_at",
            params![track_id, model, now],
        )?;
        return Ok(true);
    }
    c.execute("DELETE FROM lyrics_auto_empty WHERE track_id = ?1", params![track_id])?;
    user::put_auto(c, track_id, body, model, now)
}

/// Whether the song needs no transcribing: it has lyrics in the app
/// (written, edited or transcribed) or was transcribed with no words found.
pub fn done(c: &Connection, track_id: &str) -> AppResult<bool> {
    Ok(c
        .query_row(
            "SELECT 1 WHERE EXISTS (SELECT 1 FROM user_lyrics WHERE track_id = ?1)
                OR EXISTS (SELECT 1 FROM lyrics_auto_empty WHERE track_id = ?1)",
            params![track_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Whether the song was transcribed and no words came out.
pub fn found_nothing(c: &Connection, track_id: &str) -> AppResult<bool> {
    Ok(c.query_row("SELECT 1 FROM lyrics_auto_empty WHERE track_id = ?1", params![track_id], |_| Ok(())).optional()?.is_some())
}

/// Forget a transcription (not lyrics the user wrote or edited).
pub fn forget(c: &Connection, track_id: &str) -> AppResult<bool> {
    let a = c.execute("DELETE FROM user_lyrics WHERE track_id = ?1 AND auto IS NOT NULL", params![track_id])?;
    let b = c.execute("DELETE FROM lyrics_auto_empty WHERE track_id = ?1", params![track_id])?;
    Ok(a + b > 0)
}

/// The album's tracks in play order.
pub fn album_tracks(c: &Connection, album_id: &str) -> AppResult<Vec<String>> {
    let mut st = c.prepare("SELECT id FROM tracks WHERE album_id = ?1 ORDER BY disc_number, track_number, id")?;
    let ids = st.query_map(params![album_id], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(ids)
}

/// Unreleased tracks (by the one unreleased rule: demo release types, or an
/// unreleased ARCHIVE_VERSION read from the file) that still need doing.
pub fn unreleased_todo(c: &Connection) -> AppResult<Vec<String>> {
    let mut st = c.prepare(
        "SELECT t.id, al.format, al.subformats, ft.version FROM tracks t
         LEFT JOIN albums al ON al.id = t.album_id
         LEFT JOIN file_tags ft ON ft.part_key = t.part_key
         WHERE NOT EXISTS (SELECT 1 FROM user_lyrics u WHERE u.track_id = t.id)
           AND NOT EXISTS (SELECT 1 FROM lyrics_auto_empty e WHERE e.track_id = t.id)
         ORDER BY t.artist_title, t.album_title, t.disc_number, t.track_number",
    )?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, format, subs, version) = row?;
        let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
        if crate::db::release::is_unreleased(format.as_deref(), &subs, version.as_deref()) {
            out.push(id);
        }
    }
    Ok(out)
}
