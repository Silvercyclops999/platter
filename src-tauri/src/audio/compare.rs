//! Compare: two versions of a song played against each other. Both whole
//! files go into the audio cache first, one after the other through the
//! transfer gate (some servers allow one file transfer at a time), so a
//! switch is a local open and seek to the same moment in the other version.
//! When the lengths differ by more than [`APPROXIMATE_OVER`], the moment is
//! mapped by proportion and the UI says it's approximate. While comparing,
//! loudness leveling is on (track gain) when the setting asks for it and
//! the server has loudness data for both, else off; the user's own sound
//! settings come back when Compare closes.
//!
//! Compare never disturbs the user's listening: the first switch sets the
//! whole play state aside ([`PlayerHandle::hold`]) and closing puts it back
//! exactly ([`PlayerHandle::unhold`]). Nothing played in between is
//! reported, logged, scrobbled or saved as a resume point.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use super::dsp::Leveling;
use super::events::{AbLoop, PlayerStatus};
use super::player::PlayerHandle;
use super::source::{ContextKind, PlayContext};
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::settings::SettingsState;

/// Lengths further apart than this share (of the longer one) map by
/// proportion.
pub const APPROXIMATE_OVER: f64 = 0.05;

pub const EV_PROGRESS: &str = "compare://progress";

/// Whether two lengths are far enough apart to map by proportion.
pub fn approximate(a_ms: u64, b_ms: u64) -> bool {
    let long = a_ms.max(b_ms);
    if long == 0 {
        return false;
    }
    (a_ms.abs_diff(b_ms) as f64 / long as f64) > APPROXIMATE_OVER
}

/// The same moment in the other version: the same time when the lengths
/// are close, else the same share of the song. Never past the end.
pub fn map_position(pos_ms: u64, from_ms: u64, to_ms: u64) -> u64 {
    let end = to_ms.saturating_sub(1);
    if from_ms == 0 || to_ms == 0 {
        return 0;
    }
    let at = if approximate(from_ms, to_ms) {
        (pos_ms.min(from_ms) as f64 * to_ms as f64 / from_ms as f64).round() as u64
    } else {
        pos_ms
    };
    at.min(end)
}

/// A loop region carried into the other version.
pub fn map_loop(ab: AbLoop, from_ms: u64, to_ms: u64) -> AbLoop {
    let a_ms = map_position(ab.a_ms, from_ms, to_ms);
    let b_ms = map_position(ab.b_ms, from_ms, to_ms).max(a_ms + 1);
    AbLoop { a_ms, b_ms }
}

/// Which of the two versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    A,
    B,
}

/// `compare://progress`: how far preparing is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub side: Side,
    /// 0 to 1 for that version's file.
    pub fraction: f32,
}

/// What `compare_prepare` found.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareReady {
    pub a_id: String,
    pub b_id: String,
    /// Lengths differ by more than 5%: switches map by proportion.
    pub approximate: bool,
    /// Both play at matched loudness.
    pub level_matched: bool,
    /// The server has loudness data for both.
    pub level_available: bool,
}

/// Where Compare is after a switch.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareNow {
    pub side: Side,
    pub position_ms: u64,
    pub ab_loop: Option<AbLoop>,
}

struct Session {
    a: Track,
    b: Track,
    side: Option<Side>,
    /// The loop region in A's time.
    ab: Option<AbLoop>,
}

impl Session {
    fn track(&self, s: Side) -> &Track {
        match s {
            Side::A => &self.a,
            Side::B => &self.b,
        }
    }
    fn len(&self, s: Side) -> u64 {
        self.track(s).duration_ms
    }
    /// The loop in `s`'s time.
    fn loop_for(&self, s: Side) -> Option<AbLoop> {
        self.ab.map(|ab| match s {
            Side::A => ab,
            Side::B => map_loop(ab, self.len(Side::A), self.len(Side::B)),
        })
    }
}

/// The open Compare, if any (managed state).
#[derive(Default)]
pub struct CompareState {
    session: Mutex<Option<Session>>,
    /// Bumped by each prepare and by closing, so an older prepare stops.
    generation: Arc<AtomicU64>,
}

/// Puts both versions' files on this computer (one at a time) and opens
/// Compare. `compare://progress` reports as it goes. Leveling follows the
/// "Compare level matching" setting while Compare is open.
#[tauri::command]
pub async fn compare_prepare(
    app: AppHandle,
    player: State<'_, PlayerHandle>,
    settings: State<'_, SettingsState>,
    compare: State<'_, CompareState>,
    a_id: String,
    b_id: String,
) -> AppResult<CompareReady> {
    if a_id == b_id {
        return Err(AppError::Invalid("Pick two different versions.".into()));
    }
    let run = compare.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let tracks = player.lookup_tracks(&[a_id.clone(), b_id.clone()]).await?;
    let (Some(a), Some(b)) = (tracks.iter().find(|t| t.id == a_id).cloned(), tracks.iter().find(|t| t.id == b_id).cloned()) else {
        return Err(AppError::NotFound("Song".into()));
    };
    let counter = compare.generation.clone();
    let keep = move || counter.load(Ordering::SeqCst) == run;
    let emit = |i: usize, fraction: f32| {
        let side = if i == 0 { Side::A } else { Side::B };
        let _ = app.emit(EV_PROGRESS, Progress { side, fraction });
    };
    let fetched = player.fetch_whole(&[a.clone(), b.clone()], &emit, &keep).await;
    if let Err(e) = fetched {
        if keep() {
            // A failed prepare leaves nothing behind: the listening comes back.
            *compare.session.lock() = None;
            player.release_whole();
            player.unhold().await?;
            player.set_dsp(settings.get().await?.dsp()).await?;
        }
        return Err(e);
    }
    if !keep() {
        return Err(AppError::Invalid("Compare was closed.".into()));
    }
    let s = settings.get().await?;
    let (la, lb) = (player.loudness(&a).await, player.loudness(&b).await);
    let level_available = la.and_then(|x| x.gain_db).is_some() && lb.and_then(|x| x.gain_db).is_some();
    let level_matched = s.compare_level_match && level_available;
    let mut dsp = s.dsp();
    dsp.leveling = if level_matched { Leveling::Track } else { Leveling::Off };
    player.set_dsp(dsp).await?;
    let ready = CompareReady {
        a_id: a.id.clone(),
        b_id: b.id.clone(),
        approximate: approximate(a.duration_ms, b.duration_ms),
        level_matched,
        level_available,
    };
    *compare.session.lock() = Some(Session { a, b, side: None, ab: None });
    Ok(ready)
}

/// Plays `side` at the moment the other one is at (the start when Compare
/// hasn't played yet), keeping play or pause unless `play` says.
#[tauri::command]
pub async fn compare_switch(
    player: State<'_, PlayerHandle>,
    compare: State<'_, CompareState>,
    side: Side,
    play: Option<bool>,
) -> AppResult<CompareNow> {
    let (from, to_track, from_len, to_len, ab) = {
        let g = compare.session.lock();
        let s = g.as_ref().ok_or_else(|| AppError::Invalid("Compare isn't open.".into()))?;
        (s.side.map(|f| (f, s.track(f).id.clone())), s.track(side).clone(), s.side.map(|f| s.len(f)), s.len(side), s.loop_for(side))
    };
    // The first time Compare plays, the user's listening is set aside.
    player.hold().await?;
    let st = player.state().await?;
    let playing_from = from.is_some_and(|(_, id)| st.track.as_ref().is_some_and(|t| t.id == id));
    let position_ms = match from_len {
        Some(len) if playing_from => map_position(st.position_ms, len, to_len),
        _ => 0,
    };
    let position_ms = match ab {
        Some(l) if position_ms < l.a_ms || position_ms >= l.b_ms => l.a_ms,
        _ => position_ms,
    };
    let play = play.unwrap_or(!playing_from || st.status == PlayerStatus::Playing);
    let ctx = PlayContext {
        kind: ContextKind::Tracks,
        id: None,
        track_ids: Some(vec![to_track.id.clone()]),
    };
    player.play_context_at(ctx, Some(to_track.id.clone()), position_ms, play).await?;
    if ab.is_some() {
        player.set_ab_loop(ab).await?;
    }
    if let Some(s) = compare.session.lock().as_mut() {
        s.side = Some(side);
    }
    Ok(CompareNow { side, position_ms, ab_loop: ab })
}

/// Sets the loop region, in the time of the version playing now (null
/// clears it). It carries over on every switch.
#[tauri::command]
pub async fn compare_loop(player: State<'_, PlayerHandle>, compare: State<'_, CompareState>, ab: Option<AbLoop>) -> AppResult<Option<AbLoop>> {
    if let Some(l) = ab
        && l.b_ms <= l.a_ms
    {
        return Err(AppError::Invalid("The loop has to end after it starts.".into()));
    }
    let now = {
        let mut g = compare.session.lock();
        let s = g.as_mut().ok_or_else(|| AppError::Invalid("Compare isn't open.".into()))?;
        let side = s.side.unwrap_or(Side::A);
        s.ab = ab.map(|l| match side {
            Side::A => l,
            Side::B => map_loop(l, s.len(Side::B), s.len(Side::A)),
        });
        s.loop_for(side)
    };
    player.set_ab_loop(now).await?;
    Ok(now)
}

/// Closes Compare: the files go back to the cache's normal turnover, the
/// user's own sound settings come back, and so does their listening, just
/// as it was before Compare first played.
#[tauri::command]
pub async fn compare_end(player: State<'_, PlayerHandle>, settings: State<'_, SettingsState>, compare: State<'_, CompareState>) -> AppResult<()> {
    compare.generation.fetch_add(1, Ordering::SeqCst);
    compare.session.lock().take();
    player.release_whole();
    // The queue, position, loop and play or pause as they were.
    player.unhold().await?;
    player.set_dsp(settings.get().await?.dsp()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_lengths_keep_the_same_time() {
        assert!(!approximate(200_000, 205_000));
        assert_eq!(map_position(60_000, 200_000, 205_000), 60_000);
        // Never past the other version's end.
        assert_eq!(map_position(204_000, 205_000, 200_000), 199_999);
    }

    #[test]
    fn far_lengths_map_by_proportion() {
        assert!(approximate(200_000, 100_000));
        assert!(approximate(100_000, 106_000));
        assert_eq!(map_position(50_000, 200_000, 100_000), 25_000);
        assert_eq!(map_position(25_000, 100_000, 200_000), 50_000);
        assert_eq!(map_position(0, 100_000, 200_000), 0);
    }

    #[test]
    fn exactly_five_percent_is_not_approximate() {
        assert!(!approximate(100_000, 95_000));
        assert!(approximate(100_000, 94_999));
    }

    #[test]
    fn empty_lengths_are_safe() {
        assert!(!approximate(0, 0));
        assert_eq!(map_position(10_000, 0, 100_000), 0);
        assert_eq!(map_position(10_000, 100_000, 0), 0);
    }

    #[test]
    fn loops_carry_over_and_stay_a_region() {
        let l = map_loop(AbLoop { a_ms: 20_000, b_ms: 40_000 }, 200_000, 100_000);
        assert_eq!((l.a_ms, l.b_ms), (10_000, 20_000));
        let tight = map_loop(AbLoop { a_ms: 99_999, b_ms: 100_000 }, 100_000, 100_000);
        assert!(tight.b_ms > tight.a_ms);
    }

    #[test]
    fn session_maps_the_loop_into_b() {
        let t = |id: &str, ms: u64| Track { id: id.into(), duration_ms: ms, ..crate::audio::testutil::track(id, "a", "/p") };
        let s = Session { a: t("a", 200_000), b: t("b", 100_000), side: None, ab: Some(AbLoop { a_ms: 40_000, b_ms: 80_000 }) };
        assert_eq!(s.loop_for(Side::A), Some(AbLoop { a_ms: 40_000, b_ms: 80_000 }));
        assert_eq!(s.loop_for(Side::B), Some(AbLoop { a_ms: 20_000, b_ms: 40_000 }));
    }
}
