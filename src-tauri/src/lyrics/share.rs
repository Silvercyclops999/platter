//! Sharing a transcription to LRCLIB, one song at a time (docs/IPC.md
//! "Sharing lyrics"). Offered in the lyrics view only for a song showing
//! lyrics transcribed on one of the user's computers, after the user
//! reviewed them (opened them in the lyrics editor, or played the song
//! through with them showing), when the title, artist, album and length are
//! known, and not for unreleased songs unless Settings allows it. The user
//! sees exactly what is sent and confirms every time; a song shared once
//! isn't offered again (on any device). See [`super::publish`] for LRCLIB.
//!
//! `lyrics_share` keeps, per song, when the user reviewed the lyrics and
//! when they shared them.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use tauri::State;

use super::publish::{Payload, PublishError, Publisher, lrc_for_lrclib};
use super::transcribe::store as tstore;
use super::user;
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::{AppError, AppResult};

/// Why the offer isn't there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WhyNot {
    /// "Offer to share transcriptions publicly" is off.
    Off,
    /// The song isn't showing a transcription.
    NotTranscribed,
    /// The user hasn't checked the lyrics yet.
    NotReviewed,
    /// Title, artist, album or length is missing.
    MissingInfo,
    /// An unreleased song, and Settings doesn't allow those.
    Unreleased,
    /// Already shared.
    Shared,
}

/// `lyrics_share_info`: whether to offer sharing, and exactly what would go.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareInfo {
    pub offer: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why_not: Option<WhyNot>,
    /// The song counts as unreleased (sharing publishes its title).
    pub unreleased: bool,
    /// What LRCLIB would get (when the lyrics and details are there).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Payload>,
}

/// The settings the offer follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub offer: bool,
    pub unreleased: bool,
}

/// After the user reviewed the song's lyrics.
pub fn mark_reviewed(c: &Connection, track_id: &str, now: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO lyrics_share (track_id, reviewed_at) VALUES (?1, ?2)
         ON CONFLICT (track_id) DO UPDATE SET reviewed_at = excluded.reviewed_at",
        params![track_id, now],
    )?;
    Ok(())
}

/// After the song's lyrics were shared.
pub fn mark_shared(c: &Connection, track_id: &str, now: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO lyrics_share (track_id, shared_at) VALUES (?1, ?2)
         ON CONFLICT (track_id) DO UPDATE SET shared_at = excluded.shared_at",
        params![track_id, now],
    )?;
    Ok(())
}

/// (reviewed, shared) here.
fn state(c: &Connection, track_id: &str) -> AppResult<(bool, bool)> {
    Ok(c
        .query_row("SELECT reviewed_at IS NOT NULL, shared_at IS NOT NULL FROM lyrics_share WHERE track_id = ?1", params![track_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?
        .unwrap_or((false, false)))
}

/// What LRCLIB would get for the song: its details and the transcription
/// (synced lines without word times, and the plain words). None when the
/// song isn't showing a transcription.
fn payload(c: &Connection, track_id: &str) -> AppResult<Option<(Payload, bool)>> {
    let Some(own) = user::get(c, track_id)? else { return Ok(None) };
    if own.auto.is_none() {
        return Ok(None);
    }
    let Some(q) = super::track_query(c, track_id)? else { return Ok(None) };
    let (synced, plain) = lrc_for_lrclib(&own.text);
    let p = Payload {
        track_name: q.title.trim().to_owned(),
        artist_name: q.artist.trim().to_owned(),
        album_name: q.album.trim().to_owned(),
        duration: q.seconds() as f64,
        plain_lyrics: plain,
        synced_lyrics: synced,
    };
    let complete = !p.track_name.is_empty() && !p.artist_name.is_empty() && !p.album_name.is_empty() && q.duration_ms > 0;
    Ok(Some((p, complete)))
}

/// Whether to offer sharing the song's lyrics. `shared_elsewhere`: another
/// device shared them.
pub fn info(c: &Connection, track_id: &str, o: Options, shared_elsewhere: bool) -> AppResult<ShareInfo> {
    let unreleased = tstore::is_unreleased(c, track_id)?;
    let (reviewed, shared) = state(c, track_id)?;
    let found = payload(c, track_id)?;
    let why_not = if !o.offer {
        Some(WhyNot::Off)
    } else if shared || shared_elsewhere {
        Some(WhyNot::Shared)
    } else if found.is_none() {
        Some(WhyNot::NotTranscribed)
    } else if unreleased && !o.unreleased {
        Some(WhyNot::Unreleased)
    } else if found.as_ref().is_some_and(|(_, complete)| !complete) {
        Some(WhyNot::MissingInfo)
    } else if !reviewed {
        Some(WhyNot::NotReviewed)
    } else {
        None
    };
    let payload = if why_not.is_none() { found.map(|(p, _)| p) } else { None };
    Ok(ShareInfo { offer: why_not.is_none(), why_not, unreleased, payload })
}

/// In words, for a share that can't go ahead.
pub fn why_not_text(w: WhyNot) -> &'static str {
    match w {
        WhyNot::Off => "Sharing transcriptions is off in Settings.",
        WhyNot::NotTranscribed => "Only lyrics transcribed on your computers can be shared.",
        WhyNot::NotReviewed => "Check these lyrics first: open them in the lyrics editor or play the song through with them showing.",
        WhyNot::MissingInfo => "This song needs a title, artist, album and length before its lyrics can be shared.",
        WhyNot::Unreleased => "Sharing lyrics of unreleased songs is off in Settings.",
        WhyNot::Shared => "These lyrics were already shared.",
    }
}

/// Shares in progress, so they can be cancelled. Cheap to clone.
#[derive(Clone, Default)]
pub struct LyricsShare {
    publisher: Publisher,
    running: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl LyricsShare {
    pub fn new(publisher: Publisher) -> Self {
        Self { publisher, running: Default::default() }
    }

    /// Check the offer again, then publish. On success the song remembers it
    /// was shared; on failure nothing changes.
    pub async fn share(&self, db: &DbHandle, track_id: &str, o: Options, shared_elsewhere: bool) -> AppResult<()> {
        let info = {
            let id = track_id.to_owned();
            db.read_async(move |c| info(c, &id, o, shared_elsewhere)).await?
        };
        let Some(p) = info.payload.filter(|_| info.offer) else {
            return Err(AppError::Invalid(why_not_text(info.why_not.unwrap_or(WhyNot::NotTranscribed)).into()));
        };
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut r = self.running.lock();
            if r.contains_key(track_id) {
                return Err(AppError::Invalid("These lyrics are already being shared.".into()));
            }
            r.insert(track_id.to_owned(), cancel.clone());
        }
        let res = self.publisher.publish(&p, cancel).await;
        self.running.lock().remove(track_id);
        match res {
            Ok(()) => {
                let id = track_id.to_owned();
                db.write_async(move |c| mark_shared(c, &id, now_s())).await
            }
            Err(e @ PublishError::Offline) => Err(AppError::Network(e.message())),
            Err(e) => Err(AppError::Invalid(e.message())),
        }
    }

    pub fn cancel(&self, track_id: &str) {
        if let Some(c) = self.running.lock().get(track_id) {
            c.store(true, Ordering::Relaxed);
        }
    }
}

async fn options(settings: &crate::settings::SettingsState) -> AppResult<(Options, bool)> {
    let s = settings.get().await?;
    Ok((Options { offer: s.share_transcriptions, unreleased: s.share_transcriptions_unreleased }, s.go_offline))
}

async fn shared_elsewhere(sync: &crate::sync::commands::SyncService, track_id: &str) -> bool {
    let id = track_id.to_owned();
    sync.store.run(move |s| crate::sync::transcripts::was_shared(s, &id)).await.unwrap_or(false)
}

fn check_id(track_id: &str) -> AppResult<()> {
    if user::valid_track_id(track_id) { Ok(()) } else { Err(AppError::Invalid("track id".into())) }
}

#[tauri::command]
pub async fn lyrics_share_info(
    db: State<'_, DbHandle>,
    settings: State<'_, crate::settings::SettingsState>,
    sync: State<'_, crate::sync::commands::SyncService>,
    track_id: String,
) -> AppResult<ShareInfo> {
    check_id(&track_id)?;
    let (o, _) = options(&settings).await?;
    let elsewhere = shared_elsewhere(&sync, &track_id).await;
    db.read_async(move |c| info(c, &track_id, o, elsewhere)).await
}

/// The user reviewed the song's lyrics (opened the editor, or played the
/// song through with them showing).
#[tauri::command]
pub async fn lyrics_share_reviewed(db: State<'_, DbHandle>, track_id: String) -> AppResult<()> {
    check_id(&track_id)?;
    db.write_async(move |c| mark_reviewed(c, &track_id, now_s())).await
}

/// Publish the song's lyrics to LRCLIB, after the user confirmed.
#[tauri::command]
pub async fn lyrics_share_publish(
    share: State<'_, LyricsShare>,
    db: State<'_, DbHandle>,
    settings: State<'_, crate::settings::SettingsState>,
    sync: State<'_, crate::sync::commands::SyncService>,
    track_id: String,
) -> AppResult<()> {
    check_id(&track_id)?;
    let (o, offline) = options(&settings).await?;
    if offline {
        return Err(AppError::Network("Go offline is on. Turn it off to share lyrics.".into()));
    }
    let elsewhere = shared_elsewhere(&sync, &track_id).await;
    share.share(&db, &track_id, o, elsewhere).await?;
    let id = track_id.clone();
    if let Err(e) = sync.store.run(move |s| crate::sync::transcripts::shared(s, &id, now_s())).await {
        tracing::warn!("shared lyrics not noted in the sync store: {e}");
    }
    Ok(())
}

#[tauri::command]
pub async fn lyrics_share_cancel(share: State<'_, LyricsShare>, track_id: String) -> AppResult<()> {
    share.cancel(&track_id);
    Ok(())
}
