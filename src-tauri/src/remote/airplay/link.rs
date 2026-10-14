//! Playing on an AirPlay receiver: the local player keeps the queue and
//! decodes, the output moves to the receiver, and controls go to the local
//! player plus a flush so the receiver drops what it has buffered.
//!
//! What's heard runs a fixed delay behind the engine, so the position shown
//! is the engine's minus that delay. Right after a seek or resume nothing is
//! heard for the delay; the position holds at the seek point meanwhile. When
//! the engine moves on to the next song by itself, the old one is still
//! playing at the receiver for the delay, so it stays shown that long.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use super::AirPlayTarget;
use crate::audio::events::PlayerStatus;
use crate::audio::netsink::NetRoute;
use crate::error::{AppError, AppResult};
use crate::remote::companion::Timeline;
use crate::remote::lan::LanLink;
use crate::remote::service::LocalPlayer;
use crate::remote::types::{RemoteCommand, repeat_code};

/// What the computer had before the move, put back on the way out.
struct Before {
    output: Option<String>,
    volume: f32,
}

#[derive(Default)]
struct View {
    /// The song shown, with its length.
    shown: Option<(String, u64)>,
    /// Hold the position at least here until the heard position catches up.
    floor: Option<u64>,
    /// The next song change came from a control (flushed), so show it at once.
    jumped: bool,
}

pub struct AirPlayLink {
    target: Arc<AirPlayTarget>,
    local: Arc<dyn LocalPlayer>,
    route: NetRoute,
    before: Mutex<Option<Before>>,
    view: Mutex<View>,
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
}

impl AirPlayLink {
    /// Moves the computer's sound to the receiver. Fails (and leaves
    /// everything as it was) when the receiver doesn't answer, wants a
    /// password, or refuses this sender.
    pub async fn start(target: Arc<AirPlayTarget>, local: Arc<dyn LocalPlayer>, route: NetRoute) -> AppResult<Self> {
        let t = target.clone();
        blocking(move || t.probe()).await?;
        let st = local.state().await?;
        let before = Before {
            output: local.output_selected().await.unwrap_or(None),
            volume: st.volume,
        };
        target.set_volume(st.volume).ok();
        route.set(Some(target.clone()));
        if let Err(e) = local.select_output(Some(target.device.output_id())).await {
            route.set(None);
            let _ = local.select_output(before.output.clone()).await;
            return Err(e);
        }
        if let Err(e) = local.set_volume(1.0).await {
            tracing::debug!("full volume for AirPlay failed: {e}");
        }
        let link = Self {
            target,
            local,
            route,
            before: Mutex::new(Some(before)),
            view: Mutex::default(),
        };
        link.view.lock().shown = st.track.map(|t| (t.id, t.duration_ms));
        // Nothing is heard for the delay: hold where the move happened.
        link.view.lock().floor = Some(st.position_ms);
        Ok(link)
    }

    fn delay_ms(&self) -> u64 {
        self.target.delay().as_millis() as u64
    }

    async fn flush(&self) {
        if let Some(s) = self.target.session() {
            let _ = blocking(move || s.flush()).await;
        }
    }

    async fn ours(&self) -> bool {
        let id = self.target.device.output_id();
        self.route.get().is_some_and(|t| t.device().id == id)
            && self.local.output_selected().await.ok().flatten().as_deref() == Some(id.as_str())
    }

    async fn restore(&self) {
        let Some(b) = self.before.lock().take() else { return };
        if self.ours().await
            && let Err(e) = self.local.select_output(b.output).await
        {
            tracing::warn!("couldn't move the sound back to this computer: {e}");
        }
        self.route.set(None);
        let _ = self.local.set_volume(b.volume).await;
    }
}

#[async_trait]
impl LanLink for AirPlayLink {
    async fn poll(&self) -> AppResult<Option<Timeline>> {
        if !self.ours().await {
            return Ok(None);
        }
        let st = self.local.state().await?;
        let delay = self.delay_ms();
        let Some(track) = st.track.as_ref() else {
            return Ok(Some(Timeline {
                state: "paused".into(),
                time_ms: 0,
                duration_ms: 0,
                rating_key: None,
                play_queue_id: None,
                play_queue_item_id: None,
                volume: Some((self.target.volume() * 100.0).round() as i64),
                shuffle: st.shuffle,
                repeat: repeat_code(st.repeat) as i64,
            }));
        };
        let state = match st.status {
            PlayerStatus::Playing => "playing",
            PlayerStatus::Loading | PlayerStatus::Buffering => "buffering",
            _ => "paused",
        };
        let mut v = self.view.lock();
        let (key, time_ms, duration_ms) = match v.shown.clone() {
            // The engine moved on by itself; the receiver is still playing the
            // end of the last song.
            Some((prev, prev_len)) if prev != track.id && !v.jumped && st.position_ms < delay && st.status == PlayerStatus::Playing => {
                (prev, prev_len.saturating_sub(delay - st.position_ms), prev_len)
            }
            _ => {
                if v.shown.as_ref().is_none_or(|(id, _)| *id != track.id) {
                    v.shown = Some((track.id.clone(), track.duration_ms));
                    v.jumped = false;
                    v.floor = None;
                }
                let heard = st.position_ms.saturating_sub(delay);
                let t = if state == "paused" {
                    // Pausing here put the engine back on what was heard.
                    st.position_ms
                } else {
                    match v.floor {
                        Some(f) if heard < f => f,
                        _ => {
                            v.floor = None;
                            heard
                        }
                    }
                };
                (track.id.clone(), t, track.duration_ms)
            }
        };
        Ok(Some(Timeline {
            state: state.into(),
            time_ms,
            duration_ms,
            rating_key: Some(key),
            play_queue_id: None,
            play_queue_item_id: None,
            volume: Some((self.target.volume() * 100.0).round() as i64),
            shuffle: st.shuffle,
            repeat: repeat_code(st.repeat) as i64,
        }))
    }

    async fn command(&self, cmd: &RemoteCommand) -> AppResult<()> {
        let local = &self.local;
        match cmd {
            RemoteCommand::Play => {
                let st = local.state().await?;
                local.resume().await?;
                self.view.lock().floor = Some(st.position_ms);
            }
            RemoteCommand::Pause => {
                let st = local.state().await?;
                local.pause().await?;
                self.flush().await;
                // The engine ran ahead; carry on later from what was heard.
                let heard = st.position_ms.saturating_sub(self.delay_ms());
                local.seek(heard).await?;
                self.view.lock().floor = Some(heard);
            }
            RemoteCommand::Seek { position_ms } => {
                local.seek(*position_ms).await?;
                self.flush().await;
                self.view.lock().floor = Some(*position_ms);
            }
            RemoteCommand::Next | RemoteCommand::Previous => {
                if matches!(cmd, RemoteCommand::Next) {
                    local.next().await?;
                } else {
                    local.previous().await?;
                }
                self.flush().await;
                let mut v = self.view.lock();
                v.jumped = true;
                v.shown = None;
                v.floor = None;
            }
            RemoteCommand::Volume { volume } => {
                let (t, v) = (self.target.clone(), *volume);
                blocking(move || t.set_volume(v)).await?;
            }
            RemoteCommand::Shuffle { on } => local.set_shuffle(*on).await?,
            RemoteCommand::Repeat { mode } => local.set_repeat(*mode).await?,
        }
        Ok(())
    }

    async fn leave(&self, _stop: bool) {
        self.restore().await;
    }

    fn plays_here(&self) -> bool {
        true
    }

    async fn jumped(&self) {
        self.flush().await;
        let mut v = self.view.lock();
        v.jumped = true;
        v.shown = None;
        v.floor = None;
    }
}
