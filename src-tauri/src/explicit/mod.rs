//! Explicit marks for songs whose files don't say.
//!
//! Plex sends no explicit flag for music and most files carry none, so the
//! E had almost nothing to go on. In the background, each album whose files
//! leave a song unsettled is looked up on Deezer's public catalogue (no key):
//! one album search, then that album's songs, each explicit, clean or
//! unknown. Answers live in `explicit_found` (one row per song) and
//! `explicit_check` (one per album, with when to ask again). Files are never
//! written. A file's own tag always wins ([`TRACK_SQL`]); an album is
//! explicit when any of its songs is.
//!
//! Albums the one unreleased rule (`db::release`) counts as unreleased are
//! never looked up, nor albums whose every song already has a file answer.
//! The info sources' shared client refuses while Go offline is on, and the
//! setting can turn it off.

pub mod deezer;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::artistinfo::http::InfoHttp;
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::{AppError, AppResult};

use deezer::{DzAlbum, LocalTrack};

const DAY: i64 = 24 * 3600;
/// A matched album is asked again after three months...
pub const FOUND_TTL: i64 = 90 * DAY;
/// ...and one Deezer didn't have after two weeks.
pub const MISS_TTL: i64 = 14 * DAY;
/// Pause after each album: two requests every 1.5 s, far under Deezer's
/// published 50 per 5 s.
pub const GAP: Duration = Duration::from_millis(1500);
/// Badges are read again at most this often while a round finds marks.
const CHANGED_EVERY: Duration = Duration::from_secs(20);

#[cfg(not(test))]
static ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
#[cfg(test)]
thread_local! {
    static ON: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// "Find explicit songs online" (and not offline).
pub fn enabled() -> bool {
    #[cfg(not(test))]
    return ON.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    return ON.with(|c| c.get());
}

/// Set from the settings.
pub fn set_enabled(on: bool) {
    #[cfg(not(test))]
    ON.store(on, std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    ON.with(|c| c.set(on));
}

/// SQL for whether the song aliased `t` is explicit (1), clean (0) or
/// unknown (NULL): its file's tag, else what Deezer said.
pub const TRACK_SQL: &str = "coalesce(
    (SELECT f.explicit FROM file_tags f WHERE f.part_key = t.part_key AND f.status = 'ok'),
    (SELECT x.explicit FROM explicit_found x WHERE x.track_id = t.id))";

/// Whether a song is explicit, by its file and then Deezer.
pub fn track_explicit(c: &Connection, track_id: &str) -> AppResult<bool> {
    let v: Option<Option<i64>> = c
        .prepare_cached(&format!("SELECT {TRACK_SQL} FROM tracks t WHERE t.id = ?1"))?
        .query_row([track_id], |r| r.get(0))
        .optional()?;
    Ok(v.flatten() == Some(1))
}

/// Whether any song on an album is explicit.
pub fn album_explicit(c: &Connection, album_id: &str) -> AppResult<bool> {
    Ok(c.prepare_cached(&format!("SELECT EXISTS (SELECT 1 FROM tracks t WHERE t.album_id = ?1 AND {TRACK_SQL} = 1)"))?
        .query_row([album_id], |r| r.get(0))?)
}

/// Songs and albums explicit only by Deezer's answer (their files say
/// nothing), for the badge index.
pub fn found_marks(c: &Connection) -> AppResult<(HashSet<String>, HashSet<String>)> {
    let mut st = c.prepare_cached(
        "SELECT t.id, t.album_id FROM explicit_found x JOIN tracks t ON t.id = x.track_id
         WHERE x.explicit = 1 AND NOT EXISTS (
             SELECT 1 FROM file_tags f WHERE f.part_key = t.part_key AND f.status = 'ok' AND f.explicit IS NOT NULL)",
    )?;
    let mut tracks = HashSet::new();
    let mut albums = HashSet::new();
    for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (t, a) = row?;
        tracks.insert(t);
        albums.insert(a);
    }
    Ok((tracks, albums))
}

/// An album to look up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub album_id: String,
    pub artist: String,
    pub title: String,
}

/// Albums with a song no file settles, released by the one rule, never
/// asked or due again. Newest first.
pub fn candidates(c: &Connection, now: i64) -> AppResult<Vec<Candidate>> {
    let mut st = c.prepare(
        "SELECT al.id, al.artist_title, al.title, al.format, al.subformats, coalesce(at.version, at.release),
                al.studio, al.matched, mc.found
         FROM albums al
         LEFT JOIN album_tags at ON at.album_id = al.id
         LEFT JOIN album_mb_check mc ON mc.album_id = al.id
         LEFT JOIN explicit_check x ON x.album_id = al.id
         WHERE (x.album_id IS NULL OR x.expires_at <= ?1)
           AND EXISTS (SELECT 1 FROM tracks t LEFT JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'
                       WHERE t.album_id = al.id AND f.explicit IS NULL)
         ORDER BY al.added_at DESC",
    )?;
    let rows = st.query_map([now], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<bool>>(7)?,
            r.get::<_, Option<bool>>(8)?,
        ))
    })?;
    let mb = crate::db::release::use_musicbrainz();
    let mut out = Vec::new();
    for row in rows {
        let (album_id, artist, title, format, subs, version, studio, matched, found) = row?;
        let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
        let found = if mb { found } else { None };
        if crate::db::release::album_unreleased(&album_id, format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref(), found)
        {
            continue;
        }
        let (Some(artist), Some(title)) = (artist.filter(|a| !a.trim().is_empty()), title.filter(|t| !t.trim().is_empty()))
        else {
            continue;
        };
        out.push(Candidate { album_id, artist, title });
    }
    Ok(out)
}

/// An album's songs, for matching.
pub fn local_tracks(c: &Connection, album_id: &str) -> AppResult<Vec<LocalTrack>> {
    let mut st = c.prepare_cached(
        "SELECT id, title, coalesce(disc_number, 1), coalesce(track_number, 0), coalesce(duration_ms, 0)
         FROM tracks WHERE album_id = ?1 ORDER BY disc_number, track_number",
    )?;
    let rows = st.query_map([album_id], |r| {
        Ok(LocalTrack {
            id: r.get(0)?,
            title: r.get(1)?,
            disc: r.get::<_, i64>(2)?.max(0) as u32,
            number: r.get::<_, i64>(3)?.max(0) as u32,
            duration_ms: r.get::<_, i64>(4)?.max(0) as u64,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Store an album's answer: its songs' marks when matched (kept
/// [`FOUND_TTL`]), or none ([`MISS_TTL`]). Returns whether any song's
/// explicit mark changed.
pub fn put(c: &mut Connection, album_id: &str, found: Option<&[(String, bool)]>, now: i64) -> AppResult<bool> {
    let tx = c.transaction()?;
    let before: HashSet<String> = {
        let mut st = tx.prepare_cached("SELECT track_id FROM explicit_found WHERE album_id = ?1 AND explicit = 1")?;
        st.query_map([album_id], |r| r.get(0))?.collect::<Result<_, _>>()?
    };
    tx.execute("DELETE FROM explicit_found WHERE album_id = ?1", [album_id])?;
    {
        let mut ins = tx.prepare_cached(
            "INSERT OR REPLACE INTO explicit_found (track_id, album_id, explicit) VALUES (?1, ?2, ?3)",
        )?;
        for (id, e) in found.unwrap_or_default() {
            ins.execute(params![id, album_id, *e as i64])?;
        }
    }
    let ttl = if found.is_some() { FOUND_TTL } else { MISS_TTL };
    tx.execute(
        "INSERT INTO explicit_check (album_id, found, checked_at, expires_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (album_id) DO UPDATE SET found = excluded.found, checked_at = excluded.checked_at,
             expires_at = excluded.expires_at",
        params![album_id, found.is_some(), now, now + ttl],
    )?;
    tx.commit()?;
    let after: HashSet<String> = found.unwrap_or_default().iter().filter(|(_, e)| *e).map(|(id, _)| id.clone()).collect();
    Ok(before != after)
}

/// Why a lookup stopped. The round ends; the album is asked again next round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    /// Offline, Deezer down, busy or answering an error.
    Unavailable(String),
}

async fn get(http: &dyn InfoHttp, url: &str) -> Result<Value, LookupError> {
    let r = http.get(url, &[]).await.map_err(|e| LookupError::Unavailable(e.to_string()))?;
    if !r.ok() {
        return Err(LookupError::Unavailable(format!("Deezer answered {}", r.status)));
    }
    let v: Value = r.json().map_err(|e: AppError| LookupError::Unavailable(e.to_string()))?;
    match deezer::error(&v) {
        Some(deezer::DzError::Busy) => Err(LookupError::Unavailable("Deezer is busy right now".into())),
        // An error about this request only: nothing found, so one album
        // can't hold up the rest.
        Some(deezer::DzError::NoData) => Ok(serde_json::json!({ "data": [] })),
        None => Ok(v),
    }
}

/// Finds an album on Deezer and its songs' marks. Ok(None): no sure match.
pub async fn look_up(
    http: &dyn InfoHttp,
    base: &str,
    cand: &Candidate,
    local: &[LocalTrack],
) -> Result<Option<Vec<(String, bool)>>, LookupError> {
    let pick = |found: &[DzAlbum]| deezer::pick(&cand.title, &cand.artist, local.len(), found).map(|a| a.id);
    let mut id = pick(&deezer::albums(&get(http, &deezer::search_url(base, &cand.artist, &cand.title)).await?));
    if id.is_none() {
        let loose = crate::identity::fold::fold_album(&cand.title);
        id = pick(&deezer::albums(&get(http, &deezer::loose_url(base, &cand.artist, &loose)).await?));
    }
    let Some(id) = id else { return Ok(None) };
    let songs = deezer::tracks(&get(http, &deezer::tracks_url(base, id)).await?);
    let marks = deezer::match_tracks(local, &songs);
    Ok((!marks.is_empty()).then_some(marks))
}

/// Look every due album up, pausing `gap` after each. Stops at the first
/// failure (offline, Deezer busy): the rest wait for the next round.
/// `changed` is called when songs gained or lost a mark (at most every
/// [`CHANGED_EVERY`], and once at the end). Returns how many were asked.
pub async fn run_round(db: &DbHandle, http: &dyn InfoHttp, base: &str, gap: Duration, changed: &(dyn Fn() + Send + Sync)) -> usize {
    let now = now_s();
    let due = match db.read_async(move |c| candidates(c, now)).await {
        Ok(d) => d,
        Err(e) => {
            tracing::debug!("explicit lookup candidates: {e}");
            return 0;
        }
    };
    let mut n = 0;
    let mut pending = false;
    let mut told = std::time::Instant::now();
    for cand in due {
        if !enabled() {
            break;
        }
        let id = cand.album_id.clone();
        let local = match db.read_async(move |c| local_tracks(c, &id)).await {
            Ok(l) => l,
            Err(e) => {
                tracing::debug!("explicit lookup songs: {e}");
                break;
            }
        };
        let answer = match look_up(http, base, &cand, &local).await {
            Ok(a) => a,
            Err(LookupError::Unavailable(e)) => {
                tracing::debug!("explicit lookup for {}: {e}", cand.album_id);
                break;
            }
        };
        let id = cand.album_id.clone();
        match db.write_async(move |c| put(c, &id, answer.as_deref(), now_s())).await {
            Ok(true) => pending = true,
            Ok(false) => {}
            Err(e) => {
                tracing::debug!("explicit lookup not stored: {e}");
                break;
            }
        }
        n += 1;
        if pending && told.elapsed() >= CHANGED_EVERY {
            changed();
            pending = false;
            told = std::time::Instant::now();
        }
        tokio::time::sleep(gap).await;
    }
    if pending {
        changed();
    }
    n
}

/// Rounds after each library sync and every few hours, for the life of
/// the app.
pub async fn watch(db: DbHandle, http: Arc<dyn InfoHttp>, changed: Arc<dyn Fn() + Send + Sync>) {
    // Startup work (the first sync, Home, page lookups) goes first.
    tokio::time::sleep(Duration::from_secs(60)).await;
    let mut seen: Option<Option<i64>> = None;
    let mut last_round = 0i64;
    loop {
        let synced = db
            .read_async(|c| Ok(crate::db::store::sync_state(c)?.and_then(|s| s.last_sync_at)))
            .await
            .unwrap_or(None);
        let now = now_s();
        if enabled() && (seen != Some(synced) || now - last_round > 6 * 3600) {
            seen = Some(synced);
            last_round = now;
            let n = run_round(&db, http.as_ref(), deezer::API, GAP, changed.as_ref()).await;
            if n > 0 {
                tracing::debug!(n, "explicit lookup round");
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
