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
    save_timed(c, track_id, body, model, now, super::engine::TIMING_VERSION)
}

/// [`save`] for a result made with the given timing version.
pub fn save_timed(c: &Connection, track_id: &str, body: &str, model: &str, now: i64, timing: i64) -> AppResult<bool> {
    if body.trim().is_empty() {
        c.execute(
            "INSERT INTO lyrics_auto_empty (track_id, model, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (track_id) DO UPDATE SET model = excluded.model, created_at = excluded.created_at",
            params![track_id, model, now],
        )?;
        return Ok(true);
    }
    c.execute("DELETE FROM lyrics_auto_empty WHERE track_id = ?1", params![track_id])?;
    user::put_auto_timed(c, track_id, body, model, now, timing)
}

/// How a transcription was heard: the whole mix with nothing to go on, the
/// mix with the title hint and unsure lines left out, or the voice alone.
/// [`HEARD_VOICE_EARLY`] is the voice before 0.2.38: separating it failed
/// on every song until 0.2.37, and a song whose redo failed then was noted
/// as done anyway, so every song noted that way is heard once more.
pub const HEARD_OLD: i64 = 1;
pub const HEARD_MIX: i64 = 2;
pub const HEARD_VOICE_EARLY: i64 = 3;
pub const HEARD_VOICE: i64 = 4;

/// Note how the song's transcription (or its "no words found") was heard.
pub fn set_heard(c: &Connection, track_id: &str, heard: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO lyrics_auto_heard (track_id, heard) VALUES (?1, ?2)
         ON CONFLICT (track_id) DO UPDATE SET heard = excluded.heard",
        params![track_id, heard],
    )?;
    Ok(())
}

/// How the song's transcription was heard here, if noted.
pub fn heard(c: &Connection, track_id: &str) -> AppResult<Option<i64>> {
    Ok(c.query_row("SELECT heard FROM lyrics_auto_heard WHERE track_id = ?1", params![track_id], |r| r.get(0)).optional()?)
}

/// Songs transcribed (or found to have no words) heard worse than `below`,
/// the last played first. Lyrics the user wrote or edited aren't among
/// them, nor songs no longer in the library. A transcription from another
/// device is noted as it was heard there (an older build's as the old way).
pub fn redo_due(c: &Connection, below: i64, limit: usize) -> AppResult<Vec<String>> {
    let mut st = c.prepare(
        "SELECT h.track_id FROM lyrics_auto_heard h JOIN tracks t ON t.id = h.track_id
         WHERE h.heard < ?1
           AND NOT EXISTS (SELECT 1 FROM user_lyrics u WHERE u.track_id = h.track_id AND u.auto IS NULL)
           AND (EXISTS (SELECT 1 FROM user_lyrics u WHERE u.track_id = h.track_id AND u.auto IS NOT NULL)
                OR EXISTS (SELECT 1 FROM lyrics_auto_empty e WHERE e.track_id = h.track_id))
         ORDER BY t.last_viewed_at IS NULL, t.last_viewed_at DESC, h.track_id LIMIT ?2",
    )?;
    let rows = st.query_map(params![below, limit as i64], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
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
    if a > 0 {
        crate::lyricsearch::forget_or_log(c, track_id);
    }
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
/// unreleased ARCHIVE_VERSION read from the file, or an album Plex couldn't
/// match) that still need doing.
pub fn unreleased_todo(c: &Connection) -> AppResult<Vec<String>> {
    let mut st = c.prepare(
        "SELECT t.id, al.format, al.subformats, ft.version, al.studio, al.matched, mc.found, t.album_id FROM tracks t
         LEFT JOIN albums al ON al.id = t.album_id
         LEFT JOIN album_mb_check mc ON mc.album_id = t.album_id
         LEFT JOIN file_tags ft ON ft.part_key = t.part_key
         WHERE NOT EXISTS (SELECT 1 FROM user_lyrics u WHERE u.track_id = t.id)
           AND NOT EXISTS (SELECT 1 FROM lyrics_auto_empty e WHERE e.track_id = t.id)
         ORDER BY t.artist_title, t.album_title, t.disc_number, t.track_number",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<bool>>(5)?,
            r.get::<_, Option<bool>>(6)?,
            r.get::<_, String>(7)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, format, subs, version, studio, matched, mb, album) = row?;
        let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
        if crate::db::release::track_unreleased(&id, &album, format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref(), mb) {
            out.push(id);
        }
    }
    Ok(out)
}

/// Songs transcribed with no words found: (track, model, unix seconds).
pub fn empties(c: &Connection) -> AppResult<Vec<(String, String, i64)>> {
    let mut st = c.prepare("SELECT track_id, model, created_at FROM lyrics_auto_empty ORDER BY track_id")?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The model a "no words found" result came from.
pub fn empty_model(c: &Connection, track_id: &str) -> AppResult<Option<(String, i64)>> {
    Ok(c
        .query_row("SELECT model, created_at FROM lyrics_auto_empty WHERE track_id = ?1", params![track_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?)
}

/// The song's file version (its part key), when the song is in the library.
pub fn part_key(c: &Connection, track_id: &str) -> AppResult<Option<String>> {
    Ok(c
        .query_row("SELECT part_key FROM tracks WHERE id = ?1", params![track_id], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten()
        .filter(|p| !p.is_empty()))
}

/// Whether the song counts as unreleased (the rule [`unreleased_todo`] uses).
pub fn is_unreleased(c: &Connection, track_id: &str) -> AppResult<bool> {
    let row = c
        .query_row(
            "SELECT al.format, al.subformats, ft.version, al.studio, al.matched, mc.found, t.album_id FROM tracks t
             LEFT JOIN albums al ON al.id = t.album_id
             LEFT JOIN album_mb_check mc ON mc.album_id = t.album_id
             LEFT JOIN file_tags ft ON ft.part_key = t.part_key
             WHERE t.id = ?1",
            params![track_id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<bool>>(4)?,
                    r.get::<_, Option<bool>>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?;
    let Some((format, subs, version, studio, matched, mb, album)) = row else { return Ok(false) };
    let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
    Ok(crate::db::release::track_unreleased(track_id, &album, format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref(), mb))
}

/// The song's title, when it's in the library.
pub fn title(c: &Connection, track_id: &str) -> AppResult<Option<String>> {
    Ok(c.query_row("SELECT title FROM tracks WHERE id = ?1", params![track_id], |r| r.get(0)).optional()?)
}

/// The song's title and artist (the track artist, else the album's), when
/// it's in the library. "Various Artists" names nobody.
pub fn title_artist(c: &Connection, track_id: &str) -> AppResult<Option<(String, Option<String>)>> {
    Ok(c.query_row(
        "SELECT title, coalesce(nullif(trim(track_artist), ''), artist_title) FROM tracks WHERE id = ?1",
        params![track_id],
        |r| {
            let artist: String = r.get(1)?;
            Ok((r.get(0)?, (!crate::lyrics::various_artists(&artist)).then_some(artist)))
        },
    )
    .optional()?)
}

/// A transcribed song was looked at for official synced lyrics now.
pub fn official_checked(c: &Connection, track_id: &str, now: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO lyrics_official_check (track_id, checked_at) VALUES (?1, ?2)
         ON CONFLICT (track_id) DO UPDATE SET checked_at = excluded.checked_at",
        params![track_id, now],
    )?;
    Ok(())
}

/// Transcribed songs not looked at for official synced lyrics since
/// `before` (unix seconds), oldest first, up to `limit`. Songs the user set
/// to show their transcription are left alone.
pub fn official_due(c: &Connection, before: i64, limit: usize) -> AppResult<Vec<String>> {
    let mut st = c.prepare(
        "SELECT u.track_id FROM user_lyrics u
         LEFT JOIN lyrics_official_check o ON o.track_id = u.track_id
         WHERE u.auto IS NOT NULL AND COALESCE(o.checked_at, 0) < ?1
           AND NOT EXISTS (SELECT 1 FROM lyrics_choice ch WHERE ch.track_id = u.track_id AND ch.pick = 'auto')
         ORDER BY COALESCE(o.checked_at, 0), u.track_id LIMIT ?2",
    )?;
    let rows = st.query_map(params![before, limit as i64], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}
