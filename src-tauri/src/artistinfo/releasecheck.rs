//! Release status check on MusicBrainz for albums Plex couldn't match.
//!
//! An album with a `local://` guid and no release type or version tag is
//! only probably unreleased: Plex misses some well-known albums. After each
//! sync, in the background, each such album's artist and title are looked
//! up on MusicBrainz (through the shared one-a-second limiter, a few albums
//! a minute). A sure release group match with an official release of about
//! the same length (counting the songs tagged released, else disc 1, so a
//! disc of extras doesn't hide the match) means Plex missed the match: the album counts as
//! released and its ids give Discord a Cover Art Archive cover. No match
//! makes "probably unreleased" stronger. Answers live in `album_mb_check`
//! and feed the one unreleased rule (`db::release`).
//!
//! Albums tagged as demos or with any ARCHIVE_VERSION are never looked up,
//! nor albums whose release status tags (`tags::provenance`) rate them
//! unreleased, leaked or alternate. An album those tags rate official is
//! still looked up, for its Cover Art Archive cover.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::musicbrainz::{FoundRelease, MusicBrainz};
use super::service::ArtistInfoService;
use super::{FetchError, InfoSource};
use crate::db::queries::now_s;
use crate::db::release::{self, ReleaseStatus};
use crate::error::{AppError, AppResult};

const DAY: i64 = 24 * 3600;
/// A found release is checked again after a month...
pub const FOUND_TTL: i64 = 30 * DAY;
/// ...and an album MusicBrainz didn't have after a week.
pub const MISS_TTL: i64 = 7 * DAY;
/// Pause after each album looked up, so page lookups go first.
pub const CHECK_GAP: Duration = Duration::from_secs(10);

/// One stored answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRow {
    pub found: bool,
    pub release_group_id: Option<String>,
    pub release_id: Option<String>,
    pub checked_at: i64,
    pub expires_at: i64,
}

pub fn get(c: &Connection, album_id: &str) -> AppResult<Option<CheckRow>> {
    Ok(c.query_row(
        "SELECT found, release_group_id, release_id, checked_at, expires_at FROM album_mb_check WHERE album_id = ?1",
        params![album_id],
        |r| {
            Ok(CheckRow {
                found: r.get(0)?,
                release_group_id: r.get(1)?,
                release_id: r.get(2)?,
                checked_at: r.get(3)?,
                expires_at: r.get(4)?,
            })
        },
    )
    .optional()?)
}

/// Store an answer: found (with its ids, kept [`FOUND_TTL`]) or not
/// ([`MISS_TTL`]).
pub fn put(c: &Connection, album_id: &str, found: Option<&FoundRelease>, now: i64) -> AppResult<()> {
    let ttl = if found.is_some() { FOUND_TTL } else { MISS_TTL };
    c.execute(
        "INSERT INTO album_mb_check (album_id, found, release_group_id, release_id, checked_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (album_id) DO UPDATE SET found = excluded.found, release_group_id = excluded.release_group_id,
             release_id = excluded.release_id, checked_at = excluded.checked_at, expires_at = excluded.expires_at",
        params![
            album_id,
            found.is_some(),
            found.map(|f| f.release_group_id.as_str()),
            found.map(|f| f.release_id.as_str()),
            now,
            now + ttl
        ],
    )?;
    Ok(())
}

/// The MusicBrainz ids the check found for an album, for its cover:
/// (release group, release). None unless found and the answers count.
pub fn found_ids(c: &Connection, album_id: &str) -> AppResult<Option<(String, Option<String>)>> {
    if !release::use_musicbrainz() {
        return Ok(None);
    }
    Ok(get(c, album_id)?.filter(|r| r.found).and_then(|r| r.release_group_id.map(|g| (g, r.release_id))))
}

/// An album to look up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub album_id: String,
    pub artist: String,
    pub title: String,
    /// The songs compared with an official release's length
    /// ([`crate::tags::provenance::compare_count`]).
    pub track_count: usize,
}

/// No release type, no demo, no version tag on the album or any of its files.
fn untagged(format: Option<&str>, subformats: &[String], version: Option<&str>, file_versions: bool) -> bool {
    let blank = |s: Option<&str>| s.is_none_or(|s| s.trim().is_empty());
    blank(format) && subformats.iter().all(|s| s.trim().is_empty()) && blank(version) && !file_versions
}

/// Albums Plex couldn't match, untagged, with no answer or an expired one.
/// Newest first.
pub fn candidates(c: &Connection, now: i64) -> AppResult<Vec<Candidate>> {
    let mut st = c.prepare(
        "SELECT al.id, al.artist_title, al.title, al.track_count, al.format, al.subformats, at.version,
                (SELECT count(*) FROM tracks t WHERE t.album_id = al.id),
                EXISTS (SELECT 1 FROM tracks t JOIN file_tags ft ON ft.part_key = t.part_key
                        WHERE t.album_id = al.id AND trim(coalesce(ft.version, '')) != ''),
                (SELECT count(*) FROM tracks t JOIN file_tags ft ON ft.part_key = t.part_key AND ft.status = 'ok'
                        WHERE t.album_id = al.id AND ft.release = 'released'),
                (SELECT count(*) FROM tracks t WHERE t.album_id = al.id AND coalesce(t.disc_number, 0) <= 1),
                at.release
         FROM albums al
         LEFT JOIN album_tags at ON at.album_id = al.id
         LEFT JOIN album_mb_check mc ON mc.album_id = al.id
         WHERE al.matched = 0 AND (mc.album_id IS NULL OR mc.expires_at <= ?1)
         ORDER BY al.added_at DESC",
    )?;
    let rows = st.query_map([now], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, bool>(8)?,
            r.get::<_, i64>(9)?,
            r.get::<_, i64>(10)?,
            r.get::<_, Option<String>>(11)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (album_id, artist, title, count, format, subs, version, tracks, file_versions, released, disc_one, status) = row?;
        let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
        if !untagged(format.as_deref(), &subs, version.as_deref(), file_versions) {
            continue;
        }
        if status.as_deref().is_some_and(|s| s != crate::tags::provenance::FileRelease::Released.key()) {
            continue;
        }
        let (Some(artist), Some(title)) = (artist.filter(|a| !a.trim().is_empty()), title.filter(|t| !t.trim().is_empty()))
        else {
            continue;
        };
        let all = if tracks > 0 { tracks } else { count.unwrap_or(0) };
        let n = |x: i64| x.max(0) as usize;
        let track_count = crate::tags::provenance::compare_count(n(released), n(disc_one), n(all));
        out.push(Candidate { album_id, artist, title, track_count });
    }
    Ok(out)
}

/// What the album page shows (IPC `AlbumReleaseStatus`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumReleaseStatus {
    pub id: String,
    pub status: ReleaseStatus,
    /// Plain sentences for "Why?".
    pub reasons: Vec<String>,
}

/// An album's release status by the one rule.
pub fn album_status(c: &Connection, album_id: &str) -> AppResult<Option<AlbumReleaseStatus>> {
    let row = c
        .query_row(
            "SELECT al.format, al.subformats, coalesce(at.version, at.release), al.studio, al.matched, mc.found FROM albums al
             LEFT JOIN album_tags at ON at.album_id = al.id
             LEFT JOIN album_mb_check mc ON mc.album_id = al.id WHERE al.id = ?1",
            params![album_id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<bool>>(4)?,
                    r.get::<_, Option<bool>>(5)?,
                ))
            },
        )
        .optional()?;
    Ok(row.map(|(format, subs, version, studio, matched, mb)| {
        let subs: Vec<String> = serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default();
        let (status, reasons) = release::release_status(format.as_deref(), &subs, matched, studio.as_deref(), version.as_deref(), mb);
        AlbumReleaseStatus { id: album_id.to_owned(), status, reasons }
    }))
}

/// Whether the check may ask MusicBrainz now: MusicBrainz on (and not
/// offline) and the check's setting on.
pub fn enabled(svc: &ArtistInfoService) -> bool {
    svc.usable(InfoSource::Musicbrainz) && release::use_musicbrainz()
}

/// Look one album up and store the answer. Returns whether the album's
/// answer changed between found and not found (or is new and found).
pub async fn check_one(svc: &ArtistInfoService, cand: &Candidate, now: i64) -> Result<bool, FetchError> {
    let http = svc.http();
    let limiter = svc.musicbrainz_limiter();
    let mb = MusicBrainz { http: http.as_ref(), limiter: &limiter };
    let found = mb.release_check(&cand.artist, &cand.title, cand.track_count).await?;
    let id = cand.album_id.clone();
    let changed = svc
        .db()
        .write_async(move |c| {
            let before = get(c, &id)?.map(|r| r.found);
            put(c, &id, found.as_ref(), now)?;
            let after = found.is_some();
            if after && before != Some(true) {
                // A cover uploaded while it counted as unreleased gives way
                // to Cover Art Archive.
                c.execute("DELETE FROM discord_art WHERE album_id = ?1", params![id])?;
            }
            let changed = before.map_or(after, |b| b != after);
            if changed {
                crate::archive::refresh_for_albums(c, std::slice::from_ref(&id))?;
            }
            Ok(changed)
        })
        .await
        .map_err(|e: AppError| FetchError::Unavailable(e.to_string()))?;
    if changed && let Some(ev) = svc.events() {
        ev.release_status_changed(&cand.album_id);
    }
    Ok(changed)
}

/// Check every album that's due, pausing `gap` after each. Stops at the
/// first failure (offline, MusicBrainz busy): the rest wait for the next
/// round. Returns how many were looked up.
pub async fn run_round(svc: &ArtistInfoService, gap: Duration) -> usize {
    let now = now_s();
    let due = match svc.db().read_async(move |c| candidates(c, now)).await {
        Ok(d) => d,
        Err(e) => {
            tracing::debug!("release check candidates: {e}");
            return 0;
        }
    };
    let mut n = 0;
    for cand in due {
        if !enabled(svc) {
            break;
        }
        match check_one(svc, &cand, now_s()).await {
            Ok(_) => n += 1,
            Err(e) => {
                tracing::debug!("release check for {}: {e:?}", cand.album_id);
                break;
            }
        }
        tokio::time::sleep(gap).await;
    }
    n
}

/// Check after each library sync, and every few hours for answers that
/// expired. Runs for the life of the app.
pub async fn watch(svc: ArtistInfoService) {
    // Startup work (the first sync, Home, page lookups) goes first.
    tokio::time::sleep(Duration::from_secs(90)).await;
    let mut seen: Option<Option<i64>> = None;
    let mut last_round = 0i64;
    loop {
        let synced = svc
            .db()
            .read_async(|c| Ok(crate::db::store::sync_state(c)?.and_then(|s| s.last_sync_at)))
            .await
            .unwrap_or(None);
        let now = now_s();
        if enabled(&svc) && (seen != Some(synced) || now - last_round > 6 * 3600) {
            seen = Some(synced);
            last_round = now;
            let n = run_round(&svc, CHECK_GAP).await;
            if n > 0 {
                tracing::debug!(n, "release check round");
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
