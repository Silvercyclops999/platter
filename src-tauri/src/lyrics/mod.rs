//! Lyrics for the lyrics view and the full-window view (docs/IPC.md
//! `lyrics_get`).
//!
//! Sources in order: the Plex server's lyrics stream (a sidecar .lrc or .txt
//! file, or Plex's own lyrics), then lyrics the user wrote in the app, then
//! LRCLIB when its setting is on. The user's own lyrics go first instead
//! when they asked for that (for the song, or for every song in Settings).
//! LRCLIB answers are cached in SQLite (found: 30 days, none: 7 days); Plex
//! isn't cached here (the server keeps its own).
//!
//! Layout:
//! - [`lrclib`]   the LRCLIB requests and match rules (pure)
//! - [`cache`]    the `lyrics_cache` table
//! - [`user`]     the `user_lyrics` table (the lyrics editor's saves)
//! - [`live`]     whether a track is a live version (borrowed timing)
//! - [`commands`] the Tauri commands and the Plex side
//!
//! Any source being down is silent: the view says there are no lyrics.

pub mod cache;
pub mod commands;
pub mod live;
pub mod lrclib;
pub mod transcribe;
pub mod user;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};

use crate::artistinfo::http::InfoHttp;
use crate::artistinfo::limit::Limiter;
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::{AppError, AppResult};
use crate::plex::details::{Lyrics, LyricsSource, parse_lyrics_text};
use crate::share::matching::exact_key;
use lrclib::Query;

/// LRCLIB has no published limit; stay gentle.
pub const LRCLIB_GAP: Duration = Duration::from_millis(500);

pub const CREDIT: &str = "Lyrics from LRCLIB";
pub const CREDIT_MATCHED: &str = "Lyrics from LRCLIB, matched by title";

/// The server side: a track's lyrics from its server, if it has some.
#[async_trait]
pub trait ServerLyrics: Send + Sync {
    async fn lyrics(&self, track_id: &str) -> AppResult<Option<Lyrics>>;
}

/// The name the server side had when Plex was the only server.
pub use ServerLyrics as PlexLyrics;

/// What LRCLIB is asked about a library track: the track artist (or the
/// album artist), title, album and length.
fn track_query(c: &Connection, id: &str) -> AppResult<Option<Query>> {
    Ok(c.query_row(
        "SELECT title, album_title, artist_title, track_artist, duration_ms FROM tracks WHERE id = ?1",
        params![id],
        |r| {
            let album_artist: String = r.get(2)?;
            let track_artist: Option<String> = r.get(3)?;
            Ok(Query {
                title: r.get(0)?,
                album: r.get(1)?,
                artist: track_artist.filter(|a| !a.trim().is_empty()).unwrap_or(album_artist),
                duration_ms: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        },
    )
    .optional()?)
}

fn cache_key(q: &Query) -> String {
    format!("lrclib|{}|{}|{}|{}", exact_key(&q.artist), exact_key(&q.title), exact_key(&q.album), q.seconds())
}

/// Cached or fetched LRCLIB text to lyrics.
fn to_lyrics(body: &str, matched: bool) -> Option<Lyrics> {
    let mut l = parse_lyrics_text(body);
    if l.lines.is_empty() {
        return None;
    }
    l.source = LyricsSource::Lrclib;
    l.credit = Some(if matched { CREDIT_MATCHED } else { CREDIT }.to_owned());
    Some(l)
}

/// Sources found by artist and title rather than read from this file: their
/// timing belongs to whichever version they were written against.
pub fn borrows_timing(source: LyricsSource) -> bool {
    matches!(source, LyricsSource::Plex | LyricsSource::Lrclib)
}

/// `lyrics_get`'s answer: the lyrics, plus whether their timing comes from
/// another version of the song (a live take shown with studio lyrics).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    #[serde(flatten)]
    pub lyrics: Lyrics,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub borrowed_timing: bool,
}

/// Lyrics from Plex, then LRCLIB. Cheap to clone.
#[derive(Clone)]
pub struct LyricsService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DbHandle,
    http: Arc<dyn InfoHttp>,
    plex: Arc<dyn PlexLyrics>,
    limiter: Limiter,
    /// "Lyrics from LRCLIB when Plex has none" (Settings).
    lrclib_on: Arc<AtomicBool>,
    /// "Prefer my own lyrics over Plex's" (Settings).
    prefer_own: Arc<AtomicBool>,
    /// "Show auto-transcribed lyrics" (Settings).
    show_auto: Arc<AtomicBool>,
    /// "Show borrowed timing on live versions" (Settings).
    live_timing: Arc<AtomicBool>,
}

/// The switches lyrics read from Settings.
#[derive(Clone)]
pub struct Switches {
    pub lrclib: Arc<AtomicBool>,
    pub prefer_own: Arc<AtomicBool>,
    pub show_auto: Arc<AtomicBool>,
    pub live_timing: Arc<AtomicBool>,
}

impl Default for Switches {
    fn default() -> Self {
        Self { lrclib: Default::default(), prefer_own: Default::default(), show_auto: Arc::new(AtomicBool::new(true)), live_timing: Default::default() }
    }
}

impl LyricsService {
    pub fn new(db: DbHandle, http: Arc<dyn InfoHttp>, plex: Arc<dyn PlexLyrics>, switches: Switches) -> Self {
        Self::with_gap(db, http, plex, switches, LRCLIB_GAP)
    }

    pub fn with_gap(db: DbHandle, http: Arc<dyn InfoHttp>, plex: Arc<dyn PlexLyrics>, switches: Switches, gap: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                http,
                plex,
                limiter: Limiter::with_gap(gap),
                lrclib_on: switches.lrclib,
                prefer_own: switches.prefer_own,
                show_auto: switches.show_auto,
                live_timing: switches.live_timing,
            }),
        }
    }

    /// The track's lyrics, or None when no source has them (or a source is
    /// down: that's silent).
    pub async fn get(&self, track_id: &str) -> AppResult<Option<Lyrics>> {
        let own = self.own(track_id).await?;
        // Transcriptions never go ahead of Plex's, and hide when turned off.
        let own = own.filter(|o| o.auto.is_none() || self.inner.show_auto.load(Ordering::Relaxed));
        let own_first = own.as_ref().is_some_and(|o| o.auto.is_none())
            && (own.as_ref().is_some_and(|o| o.prefer) || self.inner.prefer_own.load(Ordering::Relaxed));
        let own = own.and_then(|o| o.lyrics());
        if own_first && own.is_some() {
            return Ok(own);
        }
        match self.inner.plex.lyrics(track_id).await {
            Ok(Some(l)) => return Ok(Some(l)),
            Ok(None) => {}
            Err(e) if e.kind() == "invalid" => return Err(e),
            Err(e) => tracing::debug!(kind = e.kind(), "plex lyrics unavailable"),
        }
        if own.is_some() {
            return Ok(own);
        }
        if !self.inner.lrclib_on.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let id = track_id.to_owned();
        let Some(q) = self.inner.db.read_async(move |c| track_query(c, &id)).await? else {
            return Ok(None);
        };
        if q.title.trim().is_empty() || q.artist.trim().is_empty() {
            return Ok(None);
        }
        self.lrclib(&q).await
    }

    /// The lyrics as the lyrics view shows them (docs/IPC.md `lyrics_get`).
    /// On a live version, lyrics found by artist and title (Plex's licensed
    /// lyrics, LRCLIB) carry another version's timing: they come back
    /// unsynced and marked `borrowed_timing`, unless "Show borrowed timing
    /// on live versions" is on (then they keep their times, still marked).
    /// Lyrics tied to the file (a sidecar file, the user's own, a
    /// transcription) keep their timing, and a transcription is shown ahead
    /// of borrowed timing.
    pub async fn shown(&self, track_id: &str) -> AppResult<Option<Shown>> {
        let Some(mut lyrics) = self.get(track_id).await? else {
            return Ok(None);
        };
        if !lyrics.synced || !borrows_timing(lyrics.source) {
            return Ok(Some(Shown { lyrics, borrowed_timing: false }));
        }
        let id = track_id.to_owned();
        let live = self
            .inner
            .db
            .read_async(move |c| live::track_is_live(c, &id))
            .await
            .unwrap_or_else(|e| {
                tracing::debug!(kind = e.kind(), "live check failed");
                false
            });
        if !live {
            return Ok(Some(Shown { lyrics, borrowed_timing: false }));
        }
        if !self.inner.live_timing.load(Ordering::Relaxed) {
            // A transcription of this very recording (the lyrics view offers
            // one for these) beats words timed to another version.
            if self.inner.show_auto.load(Ordering::Relaxed)
                && let Ok(Some(own)) = self.own(track_id).await
                && own.auto.is_some()
                && let Some(t) = own.lyrics().filter(|t| t.synced)
            {
                return Ok(Some(Shown { lyrics: t, borrowed_timing: false }));
            }
            lyrics.synced = false;
            lyrics.timed.clear();
        }
        Ok(Some(Shown { lyrics, borrowed_timing: true }))
    }

    /// Whether a source other than transcription has lyrics for the track
    /// (asked before transcribing an album or the background batch).
    pub async fn has_other(&self, track_id: &str) -> bool {
        if let Ok(Some(o)) = self.own(track_id).await {
            return o.auto.is_none();
        }
        // No row of its own, so whatever get finds is Plex's or LRCLIB's.
        matches!(self.get(track_id).await, Ok(Some(_)))
    }

    /// The track's own lyrics, as saved by the editor.
    pub async fn own(&self, track_id: &str) -> AppResult<Option<user::OwnLyrics>> {
        if !user::valid_track_id(track_id) {
            return Ok(None);
        }
        let id = track_id.to_owned();
        self.inner.db.read_async(move |c| user::get(c, &id)).await
    }

    /// Save the track's own lyrics (LRC or plain lines).
    pub async fn save_own(&self, track_id: &str, text: &str, prefer: bool) -> AppResult<()> {
        if !user::valid_track_id(track_id) {
            return Err(AppError::Invalid("track id".into()));
        }
        let text = user::clean(text)?;
        let id = track_id.to_owned();
        let now = now_s();
        self.inner.db.write_async(move |c| user::put(c, &id, &text, prefer, now)).await
    }

    /// Remove the track's own lyrics. True when there were some.
    pub async fn delete_own(&self, track_id: &str) -> AppResult<bool> {
        if !user::valid_track_id(track_id) {
            return Ok(false);
        }
        let id = track_id.to_owned();
        self.inner.db.write_async(move |c| user::delete(c, &id)).await
    }

    /// Every track with its own lyrics.
    pub async fn own_ids(&self) -> AppResult<Vec<String>> {
        self.inner.db.read_async(user::ids).await
    }

    async fn lrclib(&self, q: &Query) -> AppResult<Option<Lyrics>> {
        let key = cache_key(q);
        let now = now_s();
        let k = key.clone();
        if let Some(row) = self.inner.db.read_async(move |c| cache::get(c, &k)).await?
            && row.expires_at > now
        {
            return Ok(row.body.as_deref().and_then(|b| to_lyrics(b, row.matched)));
        }
        let Some((body, matched)) = self.fetch(q).await else {
            // Unreachable or busy: not remembered, asked again next time.
            return Ok(None);
        };
        let stored = body.clone();
        self.inner
            .db
            .write_async(move |c| {
                cache::prune(c, now)?;
                cache::put(c, &key, stored.as_deref(), matched, now)
            })
            .await?;
        Ok(body.as_deref().and_then(|b| to_lyrics(b, matched)))
    }

    /// Ask LRCLIB: the exact lookup, then a title search. Some((text, matched
    /// by title)) is an answer (text None: none found); None means LRCLIB
    /// couldn't be asked.
    async fn fetch(&self, q: &Query) -> Option<(Option<String>, bool)> {
        let http = self.inner.http.as_ref();
        let limiter = &self.inner.limiter;
        let resp = limiter
            .get(http, &lrclib::get_url(q), &[])
            .await
            .map_err(|e| tracing::debug!(kind = e.kind(), "lyrics lookup failed"))
            .ok()?;
        match resp.status {
            200 => match lrclib::parse_get(&resp.body) {
                Ok(Some(text)) => return Some((Some(text), false)),
                // Instrumental or empty: nothing to search for.
                Ok(None) => return Some((None, false)),
                Err(e) => {
                    tracing::debug!(kind = e.kind(), "lyrics reply unreadable");
                    return None;
                }
            },
            404 => {}
            s => {
                tracing::debug!(status = s, "lyrics lookup answered with an error");
                return None;
            }
        }
        let resp = limiter
            .get(http, &lrclib::search_url(q), &[])
            .await
            .map_err(|e| tracing::debug!(kind = e.kind(), "lyrics search failed"))
            .ok()?;
        if !resp.ok() {
            tracing::debug!(status = resp.status, "lyrics search answered with an error");
            return None;
        }
        match lrclib::best_search_hit(q, &resp.body) {
            Ok(text) => Some((text.clone(), text.is_some())),
            Err(e) => {
                tracing::debug!(kind = e.kind(), "lyrics search reply unreadable");
                None
            }
        }
    }
}
