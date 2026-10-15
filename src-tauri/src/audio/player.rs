//! Player controller: owns the engine and the queue, runs as one tokio task
//! (an actor) and exposes the async command API behind the playback/queue
//! commands in docs/IPC.md.
//!
//! Emits `player://state`, `player://position` (4×/s while playing) and
//! `player://error` through an [`EventSink`], plus [`TrackingEvent`]s on a
//! broadcast channel for play tracking.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::Instant;

use super::cache::{DiskCache, PrecacheHost, Precacher, cache_key};
use super::engine::{Engine, EngineConfig, EngineDeps, EngineEvent};
use super::gate::TransferGate;
use super::events::{
    AbLoop, ErrorPayload, EventSink, OutputLost, PlayerState, PlayerStatus, PositionPayload, Problem,
    SKIPPED_MESSAGE, TrackingEvent, perceptual_gain,
};
use super::output::{AudioOutput, OutputState};
use super::persist::{self, Persisted};
use super::queue::{AddAt, ContextInfo, ItemSource, Queue, QueueItem, Repeat, ShuffleStyle};
use super::dsp::DspSettings;
use super::source::{ContextKind, MoreRequest, PlayContext, TrackLookup, TrackResolver};
use crate::error::{AppError, AppResult};
use crate::model::Track;

/// Play refused because nothing asked for is stored on this computer.
pub const OFFLINE_NOTHING_PLAYS: &str =
    "These songs aren't on this computer, so they can't play until Platter can reach your server.";

/// Where to start among the playable tracks, and how many are left out. A
/// start song that can't play moves to the next playable one after it (or
/// the first one, wrapping round).
pub fn playable_part(tracks: &[Track], start: Option<&str>, ok: &HashSet<String>) -> (Option<String>, usize) {
    let left_out = tracks.iter().filter(|t| !ok.contains(&t.id)).count();
    let start = start.map(|id| {
        if ok.contains(id) {
            return Some(id.to_string());
        }
        let at = tracks.iter().position(|t| t.id == id).unwrap_or(0);
        tracks[at..]
            .iter()
            .chain(&tracks[..at])
            .find(|t| ok.contains(&t.id))
            .map(|t| t.id.clone())
    });
    (start.flatten(), left_out)
}

/// "Previous" restarts the track instead at or past this point.
pub const PREVIOUS_RESTART_MS: u64 = 3_000;
/// The shortest A-B loop.
pub const MIN_LOOP_MS: u64 = 1_000;
/// Songs a radio refill or a keep-playing batch asks for.
pub const REFILL_BATCH: usize = 20;
/// A radio refills when fewer songs than this are left.
pub const RADIO_LOW: usize = 5;
/// Smart shuffle puts a similar song after every this many.
pub const SMART_EVERY: usize = 3;
/// Songs at the end of a context that "similar music" is like.
const SIMILAR_SEEDS: usize = 10;
const PROBLEMS_CAP: usize = 200;

#[derive(Debug, Clone)]
pub struct PlayerConfig {
    /// Queue/position file (e.g. `<app data>/queue.json`). `None` = don't persist.
    pub persist_path: Option<PathBuf>,
    /// Disk pre-cache directory. `None` = no pre-cache.
    pub cache_dir: Option<PathBuf>,
    pub cache_cap_bytes: u64,
    /// How many upcoming tracks to pre-cache.
    pub precache_count: usize,
    pub persist_debounce: Duration,
    pub position_interval: Duration,
    pub engine: EngineConfig,
    /// Fixed shuffle seed (tests).
    pub seed: Option<u64>,
    /// Starting shuffle style (from settings).
    pub shuffle_style: ShuffleStyle,
    /// Contexts resume where they were left (from settings).
    pub resume_contexts: bool,
    /// Crossfade, leveling, EQ and limiter (from settings).
    pub dsp: DspSettings,
    /// Downloaded songs: played first, and never pre-cached.
    pub local_files: Option<super::source::LocalFiles>,
    /// When the queue ends, keep playing similar music (from settings).
    pub autoplay: bool,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            persist_path: None,
            cache_dir: None,
            cache_cap_bytes: 2 * 1024 * 1024 * 1024,
            precache_count: 5,
            persist_debounce: Duration::from_millis(500),
            position_interval: Duration::from_millis(250),
            engine: EngineConfig::default(),
            seed: None,
            shuffle_style: ShuffleStyle::default(),
            resume_contexts: true,
            dsp: DspSettings::default(),
            local_files: None,
            autoplay: true,
        }
    }
}

type Reply<T> = oneshot::Sender<AppResult<T>>;

enum Cmd {
    SetContext {
        info: ContextInfo,
        tracks: Vec<Track>,
        start_track_id: Option<String>,
        shuffle: Option<bool>,
        reply: Reply<()>,
    },
    Toggle(Reply<()>),
    Pause(Reply<()>),
    Resume(Reply<()>),
    Next(Reply<()>),
    Previous(Reply<()>),
    Seek(u64, Reply<()>),
    SetVolume(f32, Reply<()>),
    SetMuted(bool, Reply<()>),
    SetShuffle(bool, Reply<()>),
    SetContextShuffle(String, bool, Reply<()>),
    SetShuffleStyle(ShuffleStyle, Reply<()>),
    SetResumeContexts(bool, Reply<()>),
    SetPrecacheCount(usize, Reply<()>),
    SetDsp(DspSettings, Reply<()>),
    SetRepeat(Repeat, Reply<()>),
    SetLoop(Option<AbLoop>, Reply<()>),
    QueueAdd(Vec<Track>, AddAt, Reply<()>),
    QueueRemove(String, Reply<()>),
    QueueMove(String, usize, Reply<()>),
    QueueClear(Reply<()>),
    QueueJump(String, Reply<()>),
    Output(oneshot::Sender<OutputState>),
    SelectOutput(Option<String>, Reply<OutputState>),
    State(oneshot::Sender<PlayerState>),
    Flush(oneshot::Sender<()>),
    /// Like SetContext, but starts at `start_ms` and plays or stays paused.
    SetContextAt {
        info: ContextInfo,
        tracks: Vec<Track>,
        start_track_id: Option<String>,
        start_ms: u64,
        play: bool,
        reply: Reply<()>,
    },
    /// Like QueueJump, but starts at a position and plays or stays paused.
    JumpAt(String, u64, bool, Reply<()>),
    /// Keep playing similar music when the queue ends.
    SetAutoplay(bool, Reply<()>),
    /// Smart shuffle on (shuffle on too) or off.
    SetSmartShuffle(bool, Reply<()>),
    /// Crossfade on for the playing context only.
    SetSmoothTransitions(bool, Reply<()>),
    /// Songs from a refill (`TrackLookup::more`) for the context `key` names.
    Extend {
        key: String,
        tracks: Vec<Track>,
        source: ItemSource,
    },
    /// Take over a queue from another device (see `Queue::adopt`).
    Adopt {
        info: ContextInfo,
        tracks: Vec<Track>,
        user: Vec<Track>,
        shuffle: bool,
        repeat: Repeat,
        start_ms: u64,
        play: bool,
        reply: Reply<()>,
    },
}

/// Cloneable handle; register it with `tauri::Builder::manage`.
#[derive(Clone)]
pub struct PlayerHandle {
    tx: mpsc::UnboundedSender<Cmd>,
    lookup: Arc<dyn TrackLookup>,
    tracking: broadcast::Sender<TrackingEvent>,
    problems: Arc<Mutex<Vec<Problem>>>,
    outputs: broadcast::Sender<OutputState>,
    /// The one-transfer-at-a-time gate, shared with background readers.
    gate: Arc<TransferGate>,
    /// Whether playback's own transfers are done for now.
    host: Arc<dyn PrecacheHost>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl PlayerHandle {
    /// Builds the engine and starts the controller task on `rt`.
    /// Restores the persisted queue/position (paused) if there is one.
    pub fn spawn(
        rt: Handle,
        resolver: Arc<dyn TrackResolver>,
        lookup: Arc<dyn TrackLookup>,
        output: Box<dyn AudioOutput>,
        sink: Arc<dyn EventSink>,
        config: PlayerConfig,
    ) -> Self {
        let client = super::http::http_client();
        let cache = config.cache_dir.as_ref().and_then(|d| {
            DiskCache::new(d, config.cache_cap_bytes)
                .map(Arc::new)
                .map_err(|e| tracing::warn!("pre-cache disabled: {e}"))
                .ok()
        });
        // One HTTP body transfer at a time across streams and pre-caching:
        // some servers drop the older transfer when a second one starts.
        let gate = TransferGate::new();
        let (engine, engine_rx) = Engine::new(EngineDeps {
            resolver: resolver.clone(),
            cache: cache.clone(),
            local: config.local_files.clone(),
            client: client.clone(),
            rt: rt.clone(),
            output,
            config: config.engine.clone(),
            gate: gate.clone(),
        });
        let host = engine.precache_host();
        let precacher = cache.as_ref().map(|c| {
            Precacher::spawn(
                &rt,
                c.clone(),
                resolver.clone(),
                client.clone(),
                gate.clone(),
                Some(engine.precache_host()),
            )
        });
        let (tx, rx) = mpsc::unbounded_channel();
        let (tracking, _) = broadcast::channel(256);
        let problems = Arc::new(Mutex::new(Vec::new()));

        let mut queue = match config.seed {
            Some(s) => Queue::with_seed(s),
            None => Queue::new(),
        };
        let mut restore_pos = 0;
        let mut volume = 1.0;
        let mut muted = false;
        if let Some(p) = config.persist_path.as_deref().and_then(persist::load) {
            queue = p.queue;
            queue.adopt_legacy_shuffle();
            if let Some(s) = config.seed {
                queue.reseed(s);
            }
            restore_pos = p.position_ms;
            volume = p.volume.clamp(0.0, 1.0);
            muted = p.muted;
        }
        queue.set_shuffle_style(config.shuffle_style);
        queue.set_resume_contexts(config.resume_contexts);
        let status = if queue.current().is_some() {
            PlayerStatus::Paused
        } else {
            PlayerStatus::Idle
        };
        engine.set_gain_now(if muted { 0.0 } else { perceptual_gain(volume) });
        engine.set_dsp(config.dsp);
        let (outputs, _) = broadcast::channel(16);

        let actor = Actor {
            lookup: lookup.clone(),
            self_tx: tx.downgrade(),
            refilling: None,
            refill_dead: None,
            smart: false,
            dsp: config.dsp,
            transitions_for: None,
            engine,
            queue,
            status,
            volume,
            muted,
            cur_key: None,
            cur_started: false,
            cur_ended: false,
            next_key: None,
            restore_pos,
            skip_ids: HashSet::new(),
            failures_in_row: 0,
            sink,
            tracking: tracking.clone(),
            problems: problems.clone(),
            precacher,
            cache,
            config,
            dirty: false,
            persist_at: None,
            ticks: 0,
            ab: None,
            outputs: outputs.clone(),
        };
        rt.spawn(actor.run(rx, engine_rx));
        Self {
            tx,
            lookup,
            tracking,
            problems,
            outputs,
            gate,
            host,
        }
    }

    /// The transfer gate, for background readers that must queue behind
    /// playback (the file tag scan).
    pub fn transfer_gate(&self) -> Arc<TransferGate> {
        self.gate.clone()
    }

    /// Tells background readers when playback's own transfers are done.
    pub fn transfer_host(&self) -> Arc<dyn PrecacheHost> {
        self.host.clone()
    }

    /// Applies crossfade, leveling, EQ and limiter settings (ramped).
    pub async fn set_dsp(&self, dsp: DspSettings) -> AppResult<()> {
        self.call(|r| Cmd::SetDsp(dsp, r)).await
    }

    /// Output device changes (the same states `output://changed` carries).
    pub fn subscribe_output(&self) -> broadcast::Receiver<OutputState> {
        self.outputs.subscribe()
    }

    async fn call<T>(&self, make: impl FnOnce(Reply<T>) -> Cmd) -> AppResult<T> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(make(tx))
            .map_err(|_| AppError::Internal("player stopped".into()))?;
        rx.await
            .map_err(|_| AppError::Internal("player stopped".into()))?
    }

    /// `play_context`: resolves the context and starts it.
    pub async fn play_context(
        &self,
        context: PlayContext,
        start_track_id: Option<String>,
        shuffle: Option<bool>,
    ) -> AppResult<()> {
        let resolved = self.lookup.context(&context).await?;
        if resolved.tracks.is_empty() {
            return Err(AppError::Invalid("nothing to play".into()));
        }
        let (tracks, start_track_id) = self.keep_playable(resolved.tracks, start_track_id).await?;
        let tracks = if shuffle == Some(true) { self.skip_hidden(tracks, start_track_id.as_deref()) } else { tracks };
        let info = ContextInfo {
            kind: context.kind,
            id: context.id.clone(),
            title: resolved.title,
        };
        self.call(|reply| Cmd::SetContext {
            info,
            tracks,
            start_track_id,
            shuffle,
            reply,
        })
        .await
    }

    pub async fn toggle(&self) -> AppResult<()> {
        self.call(Cmd::Toggle).await
    }
    pub async fn pause(&self) -> AppResult<()> {
        self.call(Cmd::Pause).await
    }
    pub async fn resume(&self) -> AppResult<()> {
        self.call(Cmd::Resume).await
    }
    pub async fn next(&self) -> AppResult<()> {
        self.call(Cmd::Next).await
    }
    pub async fn previous(&self) -> AppResult<()> {
        self.call(Cmd::Previous).await
    }
    pub async fn seek(&self, position_ms: u64) -> AppResult<()> {
        self.call(|r| Cmd::Seek(position_ms, r)).await
    }
    pub async fn set_volume(&self, volume: f32) -> AppResult<()> {
        self.call(|r| Cmd::SetVolume(volume, r)).await
    }
    pub async fn set_muted(&self, muted: bool) -> AppResult<()> {
        self.call(|r| Cmd::SetMuted(muted, r)).await
    }
    pub async fn set_shuffle(&self, shuffle: bool) -> AppResult<()> {
        self.call(|r| Cmd::SetShuffle(shuffle, r)).await
    }
    /// Shuffle preference for a context by id: applies now if it's
    /// playing, otherwise the next time it plays.
    pub async fn set_context_shuffle(&self, context_id: String, shuffle: bool) -> AppResult<()> {
        self.call(|r| Cmd::SetContextShuffle(context_id, shuffle, r)).await
    }
    pub async fn set_shuffle_style(&self, style: ShuffleStyle) -> AppResult<()> {
        self.call(|r| Cmd::SetShuffleStyle(style, r)).await
    }
    /// Whether albums and playlists resume where they were left.
    pub async fn set_resume_contexts(&self, on: bool) -> AppResult<()> {
        self.call(|r| Cmd::SetResumeContexts(on, r)).await
    }
    /// How many upcoming tracks to keep pre-cached (at least 1).
    /// Keep playing similar music when the queue ends (or stop).
    pub async fn set_autoplay(&self, on: bool) -> AppResult<()> {
        self.call(|r| Cmd::SetAutoplay(on, r)).await
    }

    /// Smart shuffle: shuffle with similar songs from the library mixed in.
    pub async fn set_smart_shuffle(&self, on: bool) -> AppResult<()> {
        self.call(|r| Cmd::SetSmartShuffle(on, r)).await
    }

    /// Crossfade for the playing context only (off again when it changes).
    pub async fn set_smooth_transitions(&self, on: bool) -> AppResult<()> {
        self.call(|r| Cmd::SetSmoothTransitions(on, r)).await
    }

    pub async fn set_precache_count(&self, n: usize) -> AppResult<()> {
        self.call(|r| Cmd::SetPrecacheCount(n, r)).await
    }
    pub async fn set_repeat(&self, repeat: Repeat) -> AppResult<()> {
        self.call(|r| Cmd::SetRepeat(repeat, r)).await
    }
    /// Sets the A-B loop on the playing track (points in either order, at
    /// least [`MIN_LOOP_MS`] apart), or clears it with None.
    pub async fn set_ab_loop(&self, ab: Option<AbLoop>) -> AppResult<()> {
        self.call(|r| Cmd::SetLoop(ab, r)).await
    }
    /// `queue_add`: looks the tracks up, then queues them.
    pub async fn queue_add(&self, track_ids: Vec<String>, at: AddAt) -> AppResult<()> {
        let tracks = self.lookup.tracks(&track_ids).await?;
        if tracks.is_empty() {
            return Err(AppError::NotFound("tracks".into()));
        }
        let (tracks, _) = self.keep_playable(tracks, None).await?;
        self.call(|r| Cmd::QueueAdd(tracks, at, r)).await
    }

    /// While only music on this computer can play, leaves the rest out (and
    /// tells the lookup how many, for its notice). A start song that can't
    /// play gives way to the next one that can.
    /// Shuffle play leaves out hidden songs and artists, except the song
    /// that was picked. Everything hidden: the context plays as it is.
    fn skip_hidden(&self, tracks: Vec<Track>, start: Option<&str>) -> Vec<Track> {
        let Some(hidden) = self.lookup.hidden().filter(|h| !h.is_empty()) else { return tracks };
        let keep = |t: &Track| Some(t.id.as_str()) == start || !hidden.hides(&t.id, &t.artist_id);
        if !tracks.iter().any(keep) {
            return tracks;
        }
        tracks.into_iter().filter(keep).collect()
    }

    async fn keep_playable(
        &self,
        tracks: Vec<Track>,
        start: Option<String>,
    ) -> AppResult<(Vec<Track>, Option<String>)> {
        let Some(ok) = self.lookup.playable_now().await else {
            return Ok((tracks, start));
        };
        let (start, left_out) = playable_part(&tracks, start.as_deref(), &ok);
        let kept: Vec<Track> = tracks.into_iter().filter(|t| ok.contains(&t.id)).collect();
        if kept.is_empty() {
            return Err(AppError::Invalid(OFFLINE_NOTHING_PLAYS.into()));
        }
        if left_out > 0 {
            self.lookup.left_out_offline(left_out);
        }
        Ok((kept, start))
    }
    pub async fn queue_remove(&self, item_id: String) -> AppResult<()> {
        self.call(|r| Cmd::QueueRemove(item_id, r)).await
    }
    pub async fn queue_move(&self, item_id: String, to_index: usize) -> AppResult<()> {
        self.call(|r| Cmd::QueueMove(item_id, to_index, r)).await
    }
    pub async fn queue_clear(&self) -> AppResult<()> {
        self.call(Cmd::QueueClear).await
    }
    pub async fn queue_jump(&self, item_id: String) -> AppResult<()> {
        self.call(|r| Cmd::QueueJump(item_id, r)).await
    }
    pub async fn state(&self) -> AppResult<PlayerState> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::State(tx))
            .map_err(|_| AppError::Internal("player stopped".into()))?;
        rx.await
            .map_err(|_| AppError::Internal("player stopped".into()))
    }
    /// Writes the queue file now (call on app exit).
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Cmd::Flush(tx)).is_ok() {
            let _ = rx.await;
        }
    }
    /// Starts a context at `start_track_id` and `start_ms`, playing or
    /// paused (moving playback back from another device).
    pub async fn play_context_at(
        &self,
        context: PlayContext,
        start_track_id: Option<String>,
        start_ms: u64,
        play: bool,
    ) -> AppResult<()> {
        let resolved = self.lookup.context(&context).await?;
        if resolved.tracks.is_empty() {
            return Err(AppError::Invalid("nothing to play".into()));
        }
        let (tracks, start_track_id) = self.keep_playable(resolved.tracks, start_track_id).await?;
        let info = ContextInfo {
            kind: context.kind,
            id: context.id.clone(),
            title: resolved.title,
        };
        self.call(|reply| Cmd::SetContextAt {
            info,
            tracks,
            start_track_id,
            start_ms,
            play,
            reply,
        })
        .await
    }
    /// Takes over a queue from another Platter: `ids` is the playing song
    /// then what comes next, `user` the user queue, started at `start_ms`.
    #[allow(clippy::too_many_arguments)]
    pub async fn adopt_queue(
        &self,
        context: Option<ContextInfo>,
        ids: Vec<String>,
        user: Vec<String>,
        shuffle: bool,
        repeat: Repeat,
        start_ms: u64,
        play: bool,
    ) -> AppResult<()> {
        let tracks = self.lookup.tracks(&ids).await?;
        if tracks.is_empty() {
            return Err(AppError::NotFound("tracks".into()));
        }
        let user = if user.is_empty() { Vec::new() } else { self.lookup.tracks(&user).await? };
        let info = context.unwrap_or(ContextInfo {
            kind: ContextKind::Tracks,
            id: None,
            title: None,
        });
        self.call(|reply| Cmd::Adopt { info, tracks, user, shuffle, repeat, start_ms, play, reply })
            .await
    }
    /// Jumps to a queue entry at `position_ms`, playing or paused.
    pub async fn queue_jump_at(&self, item_id: String, position_ms: u64, play: bool) -> AppResult<()> {
        self.call(|r| Cmd::JumpAt(item_id, position_ms, play, r)).await
    }
    /// Looks tracks up by id (cache first), in the order given.
    pub async fn lookup_tracks(&self, ids: &[String]) -> AppResult<Vec<Track>> {
        self.lookup.tracks(ids).await
    }
    /// Output devices, the choice and the device in use.
    pub async fn output_state(&self) -> AppResult<OutputState> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Output(tx))
            .map_err(|_| AppError::Internal("player stopped".into()))?;
        rx.await
            .map_err(|_| AppError::Internal("player stopped".into()))
    }
    /// Plays on `device_id` (None: the system default), keeping position
    /// and play state.
    pub async fn select_output(&self, device_id: Option<String>) -> AppResult<OutputState> {
        self.call(|r| Cmd::SelectOutput(device_id, r)).await
    }
    /// Play-tracking events.
    pub fn subscribe_tracking(&self) -> broadcast::Receiver<TrackingEvent> {
        self.tracking.subscribe()
    }
    /// Tracks that couldn't be played this session, newest last.
    pub fn problems(&self) -> Vec<Problem> {
        self.problems.lock().clone()
    }
}

struct Actor {
    /// Asked for more songs when a radio or a context runs low.
    lookup: Arc<dyn TrackLookup>,
    /// Refills report back here (weak, so dropping the handles still stops the actor).
    self_tx: mpsc::WeakUnboundedSender<Cmd>,
    /// The refill in flight, by context key.
    refilling: Option<String>,
    /// A context state a refill found nothing for (not asked again).
    refill_dead: Option<String>,
    /// Smart shuffle is on (acts while the context is shuffled).
    smart: bool,
    /// The sound settings from Settings (smooth transitions lay over them).
    dsp: DspSettings,
    /// The context smooth transitions are on for.
    transitions_for: Option<String>,
    engine: Engine,
    queue: Queue,
    status: PlayerStatus,
    volume: f32,
    muted: bool,
    /// Engine key of the current queue entry (None = nothing loaded).
    cur_key: Option<u64>,
    cur_started: bool,
    cur_ended: bool,
    next_key: Option<u64>,
    /// Position to start from when nothing is loaded (restore / ended).
    restore_pos: u64,
    /// Queue entries that failed to load this context (skipped).
    skip_ids: HashSet<String>,
    failures_in_row: usize,
    sink: Arc<dyn EventSink>,
    tracking: broadcast::Sender<TrackingEvent>,
    problems: Arc<Mutex<Vec<Problem>>>,
    precacher: Option<Arc<Precacher>>,
    cache: Option<Arc<DiskCache>>,
    config: PlayerConfig,
    dirty: bool,
    persist_at: Option<Instant>,
    ticks: u64,
    /// The A-B loop and the queue entry it belongs to.
    ab: Option<(String, AbLoop)>,
    outputs: broadcast::Sender<OutputState>,
}

impl Actor {
    async fn run(
        mut self,
        mut rx: mpsc::UnboundedReceiver<Cmd>,
        mut engine_rx: mpsc::UnboundedReceiver<EngineEvent>,
    ) {
        let mut tick = tokio::time::interval(self.config.position_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        self.refresh_precache();
        loop {
            let persist_at = self.persist_at;
            tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(cmd) => self.handle(cmd),
                    None => {
                        self.persist_now();
                        return;
                    }
                },
                Some(ev) = engine_rx.recv() => self.on_engine(ev),
                _ = tick.tick() => self.on_tick(),
                _ = async {
                    match persist_at {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending::<()>().await,
                    }
                } => self.persist_now(),
            }
        }
    }

    // ------------------------------------------------------------ helpers

    /// Also records the current context's resume point, since everything
    /// that marks dirty (track change, pause, seek, the periodic save while
    /// playing) is a moment it should be updated.
    fn mark_dirty(&mut self) {
        self.queue.note_position(self.position_ms());
        if self.config.persist_path.is_some() {
            self.dirty = true;
            self.persist_at = Some(Instant::now() + self.config.persist_debounce);
        }
    }

    fn persist_now(&mut self) {
        self.persist_at = None;
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let Some(path) = self.config.persist_path.clone() else { return };
        let p = Persisted {
            version: persist::VERSION,
            queue: self.queue.clone(),
            position_ms: self.position_ms(),
            volume: self.volume,
            muted: self.muted,
        };
        if let Err(e) = persist::save(&path, &p) {
            tracing::warn!("could not save queue: {e}");
        }
    }

    fn position_ms(&self) -> u64 {
        match (self.cur_key, self.engine.position()) {
            (Some(k), Some(p)) if p.key == k => p.position_ms,
            _ => self.restore_pos,
        }
    }

    fn current_track(&self) -> Option<&Track> {
        self.queue.current().map(|c| &c.track)
    }

    fn snapshot(&self) -> PlayerState {
        let track = self.current_track().cloned();
        PlayerState {
            status: self.status,
            duration_ms: track.as_ref().map_or(0, |t| t.duration_ms),
            track,
            position_ms: self.position_ms(),
            volume: self.volume,
            muted: self.muted,
            shuffle: self.queue.shuffle(),
            shuffled_contexts: self.queue.shuffled_contexts(),
            repeat: self.queue.repeat(),
            queue: self.queue.view(),
            ab_loop: self.current_loop(),
            smart_shuffle: self.smart && self.queue.shuffle(),
            smooth_transitions: self.transitions_for.is_some(),
        }
    }

    /// The loop, if it belongs to the current entry.
    fn current_loop(&self) -> Option<AbLoop> {
        match (&self.ab, self.queue.current()) {
            (Some((id, l)), Some(c)) if &c.item_id == id => Some(*l),
            _ => None,
        }
    }

    fn clear_loop(&mut self) {
        if self.ab.take().is_some()
            && let Some(k) = self.cur_key
        {
            self.engine.set_loop(k, None);
        }
    }

    fn set_loop(&mut self, ab: Option<AbLoop>) -> AppResult<()> {
        let Some(ab) = ab else {
            self.clear_loop();
            return Ok(());
        };
        let Some(item) = self.queue.current() else {
            return Err(AppError::Invalid("nothing is playing".into()));
        };
        let dur = item.track.duration_ms;
        let (mut a, mut b) = (ab.a_ms.min(ab.b_ms), ab.a_ms.max(ab.b_ms));
        if dur > 0 {
            a = a.min(dur);
            b = b.min(dur);
        }
        if b - a < MIN_LOOP_MS {
            return Err(AppError::Invalid("B must be at least 1 s after A".into()));
        }
        let l = AbLoop { a_ms: a, b_ms: b };
        self.ab = Some((item.item_id.clone(), l));
        if let Some(k) = self.cur_key {
            self.engine.set_loop(k, Some((a, b)));
        }
        Ok(())
    }

    fn emit_state(&self) {
        self.sink.state(&self.snapshot());
    }

    fn emit_position(&self) {
        let (pos, buffered) = match (self.cur_key, self.engine.position()) {
            (Some(k), Some(p)) if p.key == k => (p.position_ms, p.buffered_ms),
            _ => (self.restore_pos, 0),
        };
        self.sink.position(&PositionPayload {
            position_ms: pos,
            duration_ms: self.current_track().map_or(0, |t| t.duration_ms),
            buffered_ms: buffered,
        });
    }

    fn track_event(&self, e: TrackingEvent) {
        let _ = self.tracking.send(e);
    }

    fn apply_gain(&self) {
        self.engine.set_gain(if self.muted {
            0.0
        } else {
            perceptual_gain(self.volume)
        });
    }

    /// The next entry to preload: auto order, skipping failed entries.
    fn pick_next(&self) -> Option<QueueItem> {
        if self.queue.repeat() == Repeat::One {
            return self.queue.current().cloned();
        }
        let n = self.queue.view().user_queue.len() + self.queue.view().next_up.len() + 1;
        self.queue
            .upcoming(n.max(1))
            .into_iter()
            .find(|i| !self.skip_ids.contains(&i.item_id))
    }

    /// Tells the engine which album is playing in album order (leveling's
    /// Smart mode uses album gain for it).
    fn sync_album_context(&self) {
        let album = self
            .queue
            .context()
            .filter(|c| c.kind == ContextKind::Album && !self.queue.shuffle())
            .and_then(|c| c.id.clone());
        self.engine.set_album_context(album);
    }

    fn emit_output(&self, state: &OutputState) {
        self.sink.output(state);
        let _ = self.outputs.send(state.clone());
    }

    /// The playing context, by kind and id.
    fn context_key(&self) -> Option<String> {
        self.queue.context().map(|c| format!("{:?}|{}", c.kind, c.id.as_deref().unwrap_or("")))
    }

    /// The sound settings, with crossfade on while smooth transitions are.
    fn apply_dsp(&self) {
        let mut dsp = self.dsp;
        if self.transitions_for.is_some() && dsp.crossfade == crate::audio::dsp::Crossfade::Off {
            dsp.crossfade = crate::audio::dsp::Crossfade::Smart;
        }
        self.engine.set_dsp(dsp);
    }

    /// Smooth transitions end when another context starts.
    fn check_transitions(&mut self) {
        if self.transitions_for.is_some() && self.transitions_for != self.context_key() {
            self.transitions_for = None;
            self.apply_dsp();
        }
    }

    /// Names the context and how far it reaches, so a refill that comes back
    /// late (the context changed or grew meanwhile) is dropped.
    fn refill_key(&self) -> Option<String> {
        let c = self.queue.context()?;
        Some(format!("{:?}|{}|{}", c.kind, c.id.as_deref().unwrap_or(""), self.queue.context_len()))
    }

    /// Radio running low, or the last song of a context playing with "keep
    /// playing similar music" on: ask for more in the background.
    fn maybe_refill(&mut self) {
        let Some(ctx) = self.queue.context().cloned() else { return };
        let Some(key) = self.refill_key() else { return };
        if self.refilling.is_some() || self.refill_dead.as_deref() == Some(key.as_str()) {
            return;
        }
        let remaining = self.queue.remaining();
        let radio = ctx.kind == ContextKind::Radio;
        let smart_kinds = matches!(ctx.kind, ContextKind::Album | ContextKind::Playlist | ContextKind::Liked | ContextKind::Artist);
        if self.smart && self.queue.shuffle() && smart_kinds && self.queue.smart_upcoming() == 0 && remaining >= SMART_EVERY {
            let Some(tx) = self.self_tx.upgrade() else { return };
            self.refilling = Some(key.clone());
            let req = MoreRequest {
                kind: ctx.kind,
                id: ctx.id.clone(),
                seed_ids: self.queue.context_sample(SIMILAR_SEEDS),
                avoid: self.queue.known_ids(),
                n: (remaining / SMART_EVERY).clamp(1, REFILL_BATCH),
                smart: true,
            };
            let lookup = self.lookup.clone();
            tokio::spawn(async move {
                let tracks = lookup.more(&req).await.unwrap_or_default();
                let _ = tx.send(Cmd::Extend { key, tracks, source: ItemSource::Smart });
            });
            return;
        }
        let due = if radio {
            remaining < RADIO_LOW
        } else {
            self.config.autoplay && self.queue.repeat() == Repeat::Off && remaining == 0 && self.queue.current().is_some()
        };
        if !due {
            return;
        }
        let Some(tx) = self.self_tx.upgrade() else { return };
        self.refilling = Some(key.clone());
        let req = MoreRequest {
            kind: ctx.kind,
            id: ctx.id.clone(),
            seed_ids: self.queue.played_context_ids(SIMILAR_SEEDS),
            avoid: self.queue.known_ids(),
            n: REFILL_BATCH,
            smart: false,
        };
        let source = if radio { ItemSource::Context } else { ItemSource::Autoplay };
        let lookup = self.lookup.clone();
        tokio::spawn(async move {
            let tracks = match lookup.more(&req).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::debug!("no more songs for {:?}: {e}", req.kind);
                    Vec::new()
                }
            };
            let _ = tx.send(Cmd::Extend { key, tracks, source });
        });
    }

    fn update_next(&mut self) {
        self.maybe_refill();
        self.sync_album_context();
        if let Some(cur) = self.cur_key {
            let next = self.pick_next().map(|i| (i.track, i.item_id));
            self.next_key = self.engine.set_next(next, cur);
        }
        self.refresh_precache();
    }

    fn refresh_precache(&self) {
        let n = self.config.precache_count;
        let mut want: Vec<Track> = self.current_track().cloned().into_iter().collect();
        want.extend(
            self.queue
                .upcoming(n + self.skip_ids.len())
                .into_iter()
                .filter(|i| !self.skip_ids.contains(&i.item_id))
                .take(n)
                .map(|i| i.track),
        );
        if let Some(c) = &self.cache {
            c.set_pinned(want.iter().map(cache_key));
        }
        if let Some(p) = &self.precacher {
            // Downloaded songs are on this computer already.
            if let Some(local) = &self.config.local_files {
                want.retain(|t| local.0.path(t).is_none());
            }
            p.set_wanted(want);
        }
    }

    /// Reports the current track as skipped (if it was audible).
    fn end_current_as_skip(&mut self) {
        if self.cur_started
            && !self.cur_ended
            && let Some(item) = self.queue.current().cloned()
        {
            let p = self.engine.position();
            let (pos, listened) = p.map_or((0, 0), |p| (p.position_ms, p.listened_ms));
            self.track_event(TrackingEvent::TrackEnded {
                track: item.track,
                item_id: item.item_id,
                listened_ms: listened,
                completed: false,
                skipped_at_ms: Some(pos),
            });
        }
        self.cur_ended = true;
    }

    /// Loads the queue's current entry into the engine.
    fn start_current(&mut self, play: bool, start_ms: u64) -> AppResult<()> {
        let Some(item) = self.queue.current().cloned() else {
            self.engine.stop();
            self.cur_key = None;
            self.status = PlayerStatus::Idle;
            return Ok(());
        };
        self.cur_started = false;
        self.cur_ended = false;
        self.next_key = None;
        self.restore_pos = start_ms;
        self.sync_album_context();
        match self
            .engine
            .load(item.track.clone(), item.item_id.clone(), start_ms, play)
        {
            Ok(key) => {
                self.cur_key = Some(key);
                // A loop survives reloading its own track, nothing else.
                match self.current_loop() {
                    Some(l) => {
                        self.engine.set_loop(key, Some((l.a_ms, l.b_ms)));
                    }
                    None => self.ab = None,
                }
                self.status = if play {
                    PlayerStatus::Loading
                } else {
                    PlayerStatus::Paused
                };
                self.update_next();
                self.mark_dirty();
                Ok(())
            }
            Err(e) => {
                self.cur_key = None;
                self.status = PlayerStatus::Error;
                self.sink.error(&ErrorPayload {
                    track_id: Some(item.track.id.clone()),
                    kind: e.kind().into(),
                    message: e.to_string(),
                });
                Err(e)
            }
        }
    }

    fn is_active(&self) -> bool {
        matches!(
            self.status,
            PlayerStatus::Playing | PlayerStatus::Loading | PlayerStatus::Buffering
        )
    }

    // ------------------------------------------------------------ commands

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::State(tx) => {
                let _ = tx.send(self.snapshot());
                return;
            }
            Cmd::Output(tx) => {
                let _ = tx.send(self.engine.output_state());
                return;
            }
            Cmd::SelectOutput(id, reply) => {
                let r = self.select_output(id);
                let _ = reply.send(r);
            }
            Cmd::Flush(tx) => {
                self.dirty = true;
                self.persist_now();
                let _ = tx.send(());
                return;
            }
            Cmd::SetContext {
                info,
                tracks,
                start_track_id,
                shuffle,
                reply,
            } => {
                // Remember where the outgoing context was left.
                self.queue.note_position(self.position_ms());
                self.end_current_as_skip();
                let start = start_track_id
                    .as_ref()
                    .and_then(|id| tracks.iter().position(|t| &t.id == id));
                self.skip_ids.clear();
                self.failures_in_row = 0;
                let at = self
                    .queue
                    .set_context_at(info, tracks, start, shuffle)
                    .map_or(0, |(_, ms)| ms);
                let r = self.start_current(true, at);
                let _ = reply.send(r);
            }
            Cmd::Toggle(reply) => {
                let r = if self.is_active() {
                    self.do_pause()
                } else {
                    self.do_resume()
                };
                let _ = reply.send(r);
            }
            Cmd::Pause(reply) => {
                let _ = reply.send(self.do_pause());
            }
            Cmd::Resume(reply) => {
                let _ = reply.send(self.do_resume());
            }
            Cmd::Next(reply) => {
                self.end_current_as_skip();
                self.failures_in_row = 0;
                let r = match self.queue.next_manual() {
                    Some(_) => self.start_current(true, 0),
                    None => {
                        self.end_of_queue();
                        Ok(())
                    }
                };
                let _ = reply.send(r);
            }
            Cmd::Previous(reply) => {
                // At or past 3 s: restart (repeat-one is kept). Earlier: the
                // previous context position, or a restart on the first one.
                let r = if self.position_ms() >= PREVIOUS_RESTART_MS {
                    self.do_seek(0)
                } else if self.queue.has_previous() {
                    self.end_current_as_skip();
                    self.failures_in_row = 0;
                    self.queue.previous();
                    self.start_current(true, 0)
                } else {
                    self.queue.previous(); // drops repeat-one
                    self.update_next();
                    self.do_seek(0)
                };
                let _ = reply.send(r);
            }
            Cmd::Seek(ms, reply) => {
                let _ = reply.send(self.do_seek(ms));
            }
            Cmd::SetVolume(v, reply) => {
                self.volume = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
                self.apply_gain();
                self.mark_dirty();
                let _ = reply.send(Ok(()));
            }
            Cmd::SetMuted(m, reply) => {
                self.muted = m;
                self.apply_gain();
                self.mark_dirty();
                let _ = reply.send(Ok(()));
            }
            Cmd::SetSmartShuffle(on, reply) => {
                self.smart = on;
                if on && !self.queue.shuffle() {
                    self.queue.set_shuffle(true);
                }
                if !on {
                    self.queue.drop_smart_upcoming();
                }
                self.refill_dead = None;
                self.update_next();
                self.mark_dirty();
                let _ = reply.send(Ok(()));
            }
            Cmd::SetSmoothTransitions(on, reply) => {
                self.transitions_for = if on { self.context_key() } else { None };
                self.apply_dsp();
                let _ = reply.send(Ok(()));
            }
            Cmd::SetShuffle(s, reply) => {
                if !s {
                    self.smart = false;
                    self.queue.drop_smart_upcoming();
                }
                self.queue.set_shuffle(s);
                self.update_next();
                self.mark_dirty();
                let _ = reply.send(Ok(()));
            }
            Cmd::SetContextShuffle(id, s, reply) => {
                if !s {
                    self.smart = false;
                    self.queue.drop_smart_upcoming();
                }
                self.queue.set_context_shuffle(&id, s);
                self.update_next();
                self.mark_dirty();
                let _ = reply.send(Ok(()));
            }
            Cmd::SetShuffleStyle(style, reply) => {
                self.queue.set_shuffle_style(style);
                let _ = reply.send(Ok(()));
            }
            Cmd::SetResumeContexts(on, reply) => {
                self.queue.set_resume_contexts(on);
                let _ = reply.send(Ok(()));
            }
            Cmd::SetAutoplay(on, reply) => {
                self.config.autoplay = on;
                self.refill_dead = None;
                self.update_next();
                let _ = reply.send(Ok(()));
            }
            Cmd::Extend { key, tracks, source } => {
                if self.refilling.as_deref() == Some(key.as_str()) {
                    self.refilling = None;
                }
                // The context changed while the songs were found.
                if self.refill_key().as_deref() != Some(key.as_str()) {
                    return;
                }
                if tracks.is_empty() {
                    self.refill_dead = Some(key);
                    return;
                }
                if source == ItemSource::Smart {
                    if self.smart && self.queue.shuffle() {
                        self.queue.insert_smart(tracks, SMART_EVERY);
                    }
                } else {
                    self.queue.extend_context(tracks, source);
                }
                self.update_next();
                self.mark_dirty();
            }
            Cmd::SetPrecacheCount(n, reply) => {
                self.config.precache_count = n.max(1);
                self.refresh_precache();
                let _ = reply.send(Ok(()));
            }
            Cmd::SetDsp(dsp, reply) => {
                self.dsp = dsp;
                self.apply_dsp();
                let _ = reply.send(Ok(()));
                return;
            }
            Cmd::SetRepeat(r, reply) => {
                self.queue.set_repeat(r);
                self.update_next();
                self.mark_dirty();
                let _ = reply.send(Ok(()));
            }
            Cmd::SetLoop(ab, reply) => {
                let _ = reply.send(self.set_loop(ab));
            }
            Cmd::QueueAdd(tracks, at, reply) => {
                self.queue.add(tracks, at);
                if self.queue.current().is_none() {
                    // Nothing loaded: the first queued track becomes current.
                    self.queue.advance();
                    self.status = PlayerStatus::Paused;
                    self.restore_pos = 0;
                }
                self.update_next();
                self.mark_dirty();
                let _ = reply.send(Ok(()));
            }
            Cmd::QueueRemove(id, reply) => {
                let r = self.queue.remove(&id);
                self.update_next();
                self.mark_dirty();
                let _ = reply.send(r);
            }
            Cmd::QueueMove(id, to, reply) => {
                let r = self.queue.move_item(&id, to);
                self.update_next();
                self.mark_dirty();
                let _ = reply.send(r);
            }
            Cmd::QueueClear(reply) => {
                self.queue.clear_user();
                self.update_next();
                self.mark_dirty();
                let _ = reply.send(Ok(()));
            }
            Cmd::QueueJump(id, reply) => {
                if self.queue.find(&id).is_none() {
                    let _ = reply.send(Err(AppError::NotFound(format!("queue item {id}"))));
                    return;
                }
                self.end_current_as_skip();
                self.failures_in_row = 0;
                let r = self
                    .queue
                    .jump(&id)
                    .and_then(|_| self.start_current(true, 0));
                let _ = reply.send(r);
            }
            Cmd::SetContextAt {
                info,
                tracks,
                start_track_id,
                start_ms,
                play,
                reply,
            } => {
                self.queue.note_position(self.position_ms());
                self.end_current_as_skip();
                let start = start_track_id
                    .as_ref()
                    .and_then(|id| tracks.iter().position(|t| &t.id == id));
                self.skip_ids.clear();
                self.failures_in_row = 0;
                let _ = self.queue.set_context_at(info, tracks, start, None);
                let r = self.start_current(play, start_ms);
                let _ = reply.send(r);
            }
            Cmd::Adopt {
                info,
                tracks,
                user,
                shuffle,
                repeat,
                start_ms,
                play,
                reply,
            } => {
                self.queue.note_position(self.position_ms());
                self.end_current_as_skip();
                self.skip_ids.clear();
                self.failures_in_row = 0;
                let _ = self.queue.adopt(info, tracks, user, shuffle, repeat);
                let r = self.start_current(play, start_ms);
                self.update_next();
                self.mark_dirty();
                let _ = reply.send(r);
            }
            Cmd::JumpAt(id, ms, play, reply) => {
                if self.queue.find(&id).is_none() {
                    let _ = reply.send(Err(AppError::NotFound(format!("queue item {id}"))));
                    return;
                }
                self.end_current_as_skip();
                self.failures_in_row = 0;
                let r = self
                    .queue
                    .jump(&id)
                    .and_then(|_| self.start_current(play, ms));
                let _ = reply.send(r);
            }
        }
        self.check_transitions();
        self.emit_state();
    }

    /// The output is gone or wouldn't open: pause where we are.
    fn pause_for_output(&mut self) {
        self.engine.pause();
        if self.is_active() {
            self.status = PlayerStatus::Paused;
            if let Some(t) = self.current_track() {
                self.track_event(TrackingEvent::Paused {
                    track_id: t.id.clone(),
                    position_ms: self.position_ms(),
                });
            }
            self.mark_dirty();
        }
    }

    fn select_output(&mut self, id: Option<String>) -> AppResult<OutputState> {
        if let Some(id) = &id
            && !self.engine.output_state().devices.iter().any(|d| &d.id == id)
        {
            return Err(AppError::NotFound(format!("output device {id}")));
        }
        let r = self.engine.select_output(id);
        if let Err(e) = &r {
            self.pause_for_output();
            self.sink.error(&ErrorPayload {
                track_id: None,
                kind: e.kind().into(),
                message: e.to_string(),
            });
        }
        let state = self.engine.output_state();
        self.emit_output(&state);
        r.map(|_| state)
    }

    fn do_pause(&mut self) -> AppResult<()> {
        if !self.is_active() {
            return Ok(());
        }
        self.engine.pause();
        self.status = PlayerStatus::Paused;
        if let Some(t) = self.current_track() {
            self.track_event(TrackingEvent::Paused {
                track_id: t.id.clone(),
                position_ms: self.position_ms(),
            });
        }
        self.mark_dirty();
        Ok(())
    }

    fn do_resume(&mut self) -> AppResult<()> {
        if self.queue.current().is_none() {
            return Ok(());
        }
        if self.cur_key.is_none() || self.engine.position().is_none() {
            // Nothing loaded (fresh start / restored / ended): load here.
            let pos = self.restore_pos;
            return self.start_current(true, pos);
        }
        match self.engine.play() {
            Ok(_) => {
                self.status = if self.cur_started {
                    PlayerStatus::Playing
                } else {
                    PlayerStatus::Loading
                };
                if self.cur_started
                    && let Some(t) = self.current_track()
                {
                    self.track_event(TrackingEvent::Resumed {
                        track_id: t.id.clone(),
                        position_ms: self.position_ms(),
                    });
                }
                Ok(())
            }
            Err(e) => {
                self.status = PlayerStatus::Error;
                self.sink.error(&ErrorPayload {
                    track_id: self.current_track().map(|t| t.id.clone()),
                    kind: e.kind().into(),
                    message: e.to_string(),
                });
                Err(e)
            }
        }
    }

    fn do_seek(&mut self, ms: u64) -> AppResult<()> {
        let dur = self.current_track().map_or(0, |t| t.duration_ms);
        let ms = if dur > 0 { ms.min(dur) } else { ms };
        // Seeking outside the loop's span clears it.
        if let Some(l) = self.current_loop()
            && (ms < l.a_ms || ms > l.b_ms)
        {
            self.clear_loop();
        }
        let from = self.position_ms();
        if self.cur_key.is_some() && self.engine.seek(ms) {
            if let Some(t) = self.current_track() {
                self.track_event(TrackingEvent::Seeked {
                    track_id: t.id.clone(),
                    from_ms: from,
                    to_ms: ms,
                });
            }
        } else {
            self.restore_pos = ms;
        }
        self.emit_position();
        self.mark_dirty();
        Ok(())
    }

    /// Nothing more to play. With repeat off the context rewinds to its
    /// first track, paused at 0:00, with the rest listed as upcoming.
    fn end_of_queue(&mut self) {
        self.ab = None;
        self.engine.stop();
        self.cur_key = None;
        self.next_key = None;
        self.restore_pos = 0;
        if self.queue.repeat() == Repeat::Off {
            self.queue.rewind();
        }
        self.status = if self.queue.current().is_some() {
            PlayerStatus::Paused
        } else {
            PlayerStatus::Idle
        };
        self.mark_dirty();
    }

    // ------------------------------------------------------------ engine

    fn on_engine(&mut self, ev: EngineEvent) {
        match ev {
            EngineEvent::Started { key, tag } => {
                if Some(key) == self.cur_key {
                    self.on_track_audible();
                } else if Some(key) == self.next_key {
                    // Gapless transition.
                    if !self.queue.advance_auto_to(&tag) {
                        tracing::warn!("queue lost track of entry {tag}");
                    }
                    self.cur_key = Some(key);
                    self.next_key = None;
                    self.cur_ended = false;
                    self.ab = None;
                    self.on_track_audible();
                    self.update_next();
                } else {
                    return;
                }
            }
            EngineEvent::Ended { key, listened_ms, .. } => {
                if Some(key) != self.cur_key || self.cur_ended {
                    return;
                }
                self.cur_ended = true;
                if let Some(item) = self.queue.current().cloned() {
                    self.track_event(TrackingEvent::TrackEnded {
                        track: item.track,
                        item_id: item.item_id,
                        listened_ms,
                        completed: true,
                        skipped_at_ms: None,
                    });
                }
                return;
            }
            EngineEvent::Drained => {
                // Stale if something has been loaded since.
                if self.cur_key.is_none() || self.engine.position().is_some() {
                    return;
                }
                self.continue_after_gap();
            }
            EngineEvent::Failed {
                key,
                tag,
                track_id,
                error,
            } => {
                self.record_problem(&tag, &track_id, &error);
                self.sink.error(&ErrorPayload {
                    track_id: Some(track_id),
                    kind: error.kind().into(),
                    message: SKIPPED_MESSAGE.into(),
                });
                self.skip_ids.insert(tag.clone());
                self.failures_in_row += 1;
                let limit = self.queue.view().next_up.len() + self.queue.view().user_queue.len() + 1;
                if Some(key) == self.cur_key {
                    if self.failures_in_row > limit {
                        self.end_of_queue();
                    } else {
                        // Skip the broken current entry (even with repeat-one).
                        self.cur_ended = true;
                        let n = self.queue.view().user_queue.len()
                            + self.queue.view().next_up.len()
                            + 1;
                        let target = self
                            .queue
                            .upcoming(n)
                            .into_iter()
                            .find(|i| !self.skip_ids.contains(&i.item_id));
                        match target {
                            Some(t) if self.queue.advance_auto_to(&t.item_id) => {
                                let _ = self.start_current(true, 0);
                            }
                            _ => self.end_of_queue(),
                        }
                    }
                } else if Some(key) == self.next_key {
                    self.next_key = None;
                    if self.engine.position().is_some() {
                        self.update_next();
                    } else {
                        // The current track already finished: move on now.
                        self.continue_after_gap();
                    }
                } else {
                    return;
                }
            }
            EngineEvent::Buffering(b) => {
                if matches!(self.status, PlayerStatus::Playing | PlayerStatus::Buffering) {
                    self.status = if b {
                        PlayerStatus::Buffering
                    } else {
                        PlayerStatus::Playing
                    };
                } else {
                    return;
                }
            }
            EngineEvent::DeviceLost { why, fell_back_from } => {
                tracing::info!("audio device lost: {why}");
                self.engine.close_output();
                self.pause_for_output();
                self.emit_output(&self.engine.output_state());
                if let Some(device) = fell_back_from {
                    self.sink.output_lost(&OutputLost { device });
                }
            }
            EngineEvent::OutputReset(why) => {
                tracing::info!("audio output reset: {why}");
                if let Err(e) = self.engine.reopen_output() {
                    tracing::warn!("audio output did not reopen: {e}");
                    self.pause_for_output();
                }
                self.emit_output(&self.engine.output_state());
            }
            EngineEvent::OutputChanged => {
                self.emit_output(&self.engine.output_state());
                return;
            }
        }
        self.emit_state();
    }

    /// The engine ran dry (end of queue, or the preloaded next wasn't
    /// there): start the next playable entry, or stop at the end.
    fn continue_after_gap(&mut self) {
        let Some(next) = self.pick_next() else {
            self.end_of_queue();
            return;
        };
        if self.queue.advance_auto_to(&next.item_id) {
            let _ = self.start_current(true, 0);
        } else {
            self.end_of_queue();
        }
    }

    fn on_track_audible(&mut self) {
        self.cur_started = true;
        self.failures_in_row = 0;
        if self.status == PlayerStatus::Loading || self.engine.is_playing() {
            self.status = PlayerStatus::Playing;
        }
        if let Some(item) = self.queue.current().cloned() {
            self.track_event(TrackingEvent::TrackStarted {
                track: item.track,
                item_id: item.item_id,
                at: now_ms(),
                context: self.queue.context().cloned(),
            });
        }
        self.mark_dirty();
        self.emit_position();
    }

    fn record_problem(&self, _item_id: &str, track_id: &str, error: &AppError) {
        let (title, artist) = self
            .queue
            .find(_item_id)
            .map(|i| (i.track.title.clone(), i.track.artist_title.clone()))
            .unwrap_or_default();
        let mut p = self.problems.lock();
        p.push(Problem {
            track_id: track_id.into(),
            title,
            artist,
            kind: error.kind().into(),
            message: error.to_string(),
            at: now_ms(),
        });
        let len = p.len();
        if len > PROBLEMS_CAP {
            p.drain(..len - PROBLEMS_CAP);
        }
    }

    fn on_tick(&mut self) {
        if !matches!(self.status, PlayerStatus::Playing | PlayerStatus::Buffering) {
            return;
        }
        self.emit_position();
        self.ticks += 1;
        let per_sec = (1000 / self.config.position_interval.as_millis().max(1) as u64).max(1);
        if self.ticks.is_multiple_of(per_sec)
            && let Some(t) = self.current_track()
        {
            self.track_event(TrackingEvent::Progress {
                track_id: t.id.clone(),
                position_ms: self.position_ms(),
                duration_ms: t.duration_ms,
            });
        }
        // Save the position every ~5 s while playing.
        if self.ticks.is_multiple_of(per_sec * 5) {
            self.mark_dirty();
        }
    }
}
