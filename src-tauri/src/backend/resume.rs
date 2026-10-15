//! Resume on another device: servers that keep a play queue (Subsonic's
//! saved queue) get this computer's queue as it changes, so another app on
//! the same account can pick it up; and a queue another app left there can
//! be picked up here. Not a handoff: nothing tells the other app to stop.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::State;
use tokio::sync::broadcast;

use super::{Backends, Caps};
use crate::audio::events::{PlayerState, TrackingEvent};
use crate::audio::player::PlayerHandle;
use crate::audio::queue::Repeat;
use crate::error::{AppError, AppResult};

/// Songs kept in a saved queue at most.
pub const MAX_SAVED: usize = 500;

/// A queue saved on a server, to offer in the player.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeOffer {
    pub tag: String,
    pub server_name: String,
    pub track_count: usize,
    /// The song that was playing.
    pub current_id: String,
    pub position_ms: u64,
    /// The app that saved it.
    pub changed_by: Option<String>,
    /// When, unix seconds.
    pub changed_at: Option<i64>,
}

/// The playing song and what comes after it (the user's queue, then the
/// context), with where the song is.
pub fn queue_of(state: &PlayerState) -> Option<(Vec<String>, u64)> {
    let current = state.queue.current.as_ref()?;
    let ids: Vec<String> = std::iter::once(current.track.id.clone())
        .chain(state.queue.user_queue.iter().map(|i| i.track.id.clone()))
        .chain(state.queue.next_up.iter().map(|i| i.track.id.clone()))
        .take(MAX_SAVED)
        .collect();
    Some((ids, state.position_ms))
}

/// Keep this queue on the playing song's server, when it keeps queues.
/// True when it was saved.
pub async fn save(backends: &Backends, ids: &[String], position_ms: u64) -> AppResult<bool> {
    let Some(first) = ids.first() else { return Ok(false) };
    let server = backends.for_id(first)?;
    if !server.caps().has(Caps::RESUME_QUEUE) {
        return Ok(false);
    }
    let Some(r) = server.resume() else { return Ok(false) };
    let tag = crate::sid::tag_of(first).map(str::to_string);
    let mine: Vec<String> = ids.iter().filter(|i| crate::sid::tag_of(i).map(str::to_string) == tag).cloned().collect();
    r.save(&mine, 0, position_ms).await?;
    Ok(true)
}

/// Every queue saved on a server in use.
pub async fn offers(backends: &Backends) -> Vec<ResumeOffer> {
    let mut out = Vec::new();
    for s in backends.all() {
        if !s.caps().has(Caps::RESUME_QUEUE) {
            continue;
        }
        let (Some(r), Some(tag)) = (s.resume(), s.tag().await) else { continue };
        match r.load().await {
            Ok(Some(q)) => out.push(ResumeOffer {
                tag,
                server_name: s.name().await.unwrap_or_default(),
                track_count: q.track_ids.len(),
                current_id: q.track_ids.get(q.current).cloned().unwrap_or_default(),
                position_ms: q.position_ms,
                changed_by: q.changed_by,
                changed_at: q.changed_at,
            }),
            Ok(None) => {}
            Err(e) => tracing::debug!("saved queue not read: {}", e.kind()),
        }
    }
    out
}

/// Save the queue as playback moves on: each new song and each pause.
pub fn spawn_saver(backends: Backends, player: PlayerHandle, mut events: broadcast::Receiver<TrackingEvent>, on: Arc<AtomicBool>) {
    tauri::async_runtime::spawn(async move {
        loop {
            let ev = match events.recv().await {
                Ok(e) => e,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            };
            if !matches!(ev, TrackingEvent::TrackStarted { .. } | TrackingEvent::Paused { .. }) || !on.load(Ordering::Relaxed) {
                continue;
            }
            let Ok(state) = player.state().await else { continue };
            let Some((ids, pos)) = queue_of(&state) else { continue };
            if let Err(e) = save(&backends, &ids, pos).await {
                tracing::debug!("queue not saved on the server: {}", e.kind());
            }
        }
    });
}

#[tauri::command]
pub async fn resume_queues(backends: State<'_, Backends>) -> AppResult<Vec<ResumeOffer>> {
    Ok(offers(&backends).await)
}

/// Play the queue saved on this server, from where it was.
#[tauri::command]
pub async fn resume_queue_play(backends: State<'_, Backends>, player: State<'_, PlayerHandle>, tag: String) -> AppResult<()> {
    let server = backends.server(&tag).ok_or_else(|| AppError::NotFound("That server isn't in Platter any more.".into()))?;
    let r = server.resume().ok_or_else(|| AppError::Invalid("That server doesn't keep a queue.".into()))?;
    let q = r.load().await?.ok_or_else(|| AppError::NotFound("There's no saved queue on that server.".into()))?;
    let ids = q.track_ids[q.current.min(q.track_ids.len().saturating_sub(1))..].to_vec();
    player.adopt_queue(None, ids, Vec::new(), false, Repeat::Off, q.position_ms, true).await
}

/// Save the queue now (the saver does this as playback moves on).
#[tauri::command]
pub async fn resume_queue_save(backends: State<'_, Backends>, player: State<'_, PlayerHandle>) -> AppResult<bool> {
    let state = player.state().await?;
    let Some((ids, pos)) = queue_of(&state) else { return Ok(false) };
    save(&backends, &ids, pos).await
}
