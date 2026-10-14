//! Playback engine: one continuous output stream fed by a queue of
//! per-track *segments*.
//!
//! ```text
//!  loader thread (track A) ──push──▶ [ segment A | segment B ] ──render──▶ output
//!  loader thread (track B) ──push──▶                ▲
//!                                   decode-ahead: B is opened and decoded
//!                                   as soon as A has finished decoding
//! ```
//!
//! The mixer pulls samples from the front segment and moves to the next one
//! in the same output buffer, so album transitions have no gap. Each segment
//! knows its track, so the position clock always reports the *audible*
//! track. Phase 2 crossfade/loudness slot in as a per-segment gain/DSP stage
//! in [`Core::render`] without changing this structure.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::cache::{DiskCache, PrecacheHost, cache_key};
use super::convert::Converter;
use super::decode::{DecodeError, TrackDecoder, extension_hint};
use super::gate::{Priority, PriorityCell, TransferGate};
use super::http::{HttpConfig, HttpSource, HttpStats};
use super::output::{AudioOutput, OutputCallback, OutputEvent, OutputFormat, OutputState, Renderer};
use super::source::{StreamSource, TrackResolver};
use crate::error::{AppError, AppResult};
use crate::model::Track;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Decoded audio kept ahead of the play position (all segments).
    pub buffer_ahead_ms: u64,
    /// Pause/resume gain ramp.
    pub ramp_ms: u64,
    /// Fade-out/in applied around seeks and skips (0 disables).
    pub declick_ms: u64,
    pub http: HttpConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            buffer_ahead_ms: 12_000,
            ramp_ms: 10,
            declick_ms: 5,
            http: HttpConfig::default(),
        }
    }
}

/// Engine → controller notifications.
#[derive(Debug)]
pub enum EngineEvent {
    /// The segment's first sample just played (track became audible).
    Started { key: u64, tag: String },
    /// The segment played to its end.
    Ended { key: u64, tag: String, listened_ms: u64 },
    /// The track could not be played (after the transcode fallback).
    Failed { key: u64, tag: String, track_id: String, error: AppError },
    /// Nothing left to play.
    Drained,
    /// Output underran while playing (true) / recovered (false).
    Buffering(bool),
    /// The output device went away; the engine has paused. `fell_back_from`
    /// names a chosen device; the output now targets the system default.
    DeviceLost {
        why: String,
        fell_back_from: Option<String>,
    },
    /// The output stream broke but can be reopened (call
    /// [`Engine::reopen_output`]); playback state is untouched.
    OutputReset(String),
    /// The output moved to another device by itself.
    OutputChanged,
}

/// What the loader reports about where the audio came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Cache,
    Stream,
    Transcode,
}

struct LoaderCtl {
    cancel: AtomicBool,
    seek: Mutex<Option<(u64, u64)>>, // (generation, ms)
    /// Transfer-gate priority of this segment's stream.
    prio: PriorityCell,
}

impl LoaderCtl {
    fn new(prio: Priority) -> Arc<Self> {
        Arc::new(Self {
            cancel: AtomicBool::new(false),
            seek: Mutex::new(None),
            prio: prio.cell(),
        })
    }
}

struct Segment {
    key: u64,
    tag: String,
    track: Track,
    samples: VecDeque<f32>,
    /// Track frame (output rate) of the first sample after the last seek.
    start_frame: u64,
    /// Frames consumed since `start_frame`.
    consumed: u64,
    /// Frames actually rendered (for listened time), across seeks.
    listened: u64,
    done: bool,
    started: bool,
    generation: u64,
    ctl: Arc<LoaderCtl>,
    stats: Option<HttpStats>,
    origin: Option<Origin>,
    /// The A-B loop on this track, if any.
    ab: Option<AbLoop>,
    /// Replaying the loop's saved audio. While it runs, `samples` hold the
    /// audio from `Replay::resume_frame` on.
    replay: Option<Replay>,
}

/// Most samples an A-B loop keeps for gapless replay (about a minute of
/// 48 kHz stereo). Longer loops wrap by re-decoding from A.
const MAX_LOOP_SAMPLES: usize = 6_000_000;

/// An A-B loop: when playback reaches B it goes back to A. Once a whole
/// pass from A to B has played, its audio is kept and replayed from memory,
/// so later passes have no gap.
struct AbLoop {
    a_ms: u64,
    b_ms: u64,
    a: u64,
    b: u64,
    clip: Vec<f32>,
    building: bool,
    ready: Option<Arc<Vec<f32>>>,
}

impl AbLoop {
    fn new(a_ms: u64, b_ms: u64, m: &Mix) -> Self {
        Self {
            a_ms,
            b_ms,
            a: m.ms_to_frames(a_ms),
            b: m.ms_to_frames(b_ms),
            clip: Vec::new(),
            building: false,
            ready: None,
        }
    }
}

struct Replay {
    clip: Arc<Vec<f32>>,
    at: usize,
    resume_frame: u64,
}

/// What reaching B does.
enum Wrap {
    /// Play the saved A..B audio (gapless).
    Replay(Arc<Vec<f32>>),
    /// Re-decode from A.
    Reload,
}

impl Segment {
    fn new(key: u64, tag: String, track: Track, start_frame: u64, prio: Priority) -> Self {
        Self {
            key,
            tag,
            track,
            samples: VecDeque::new(),
            start_frame,
            consumed: 0,
            listened: 0,
            done: false,
            started: false,
            generation: 0,
            ctl: LoaderCtl::new(prio),
            stats: None,
            origin: None,
            ab: None,
            replay: None,
        }
    }

    /// Track frame of the next sample to play.
    fn pos(&self) -> u64 {
        self.start_frame + self.consumed
    }

    /// Track frame where `samples` begin.
    fn samples_start(&self) -> u64 {
        self.replay.as_ref().map_or(self.pos(), |r| r.resume_frame)
    }

    /// Stops a replay; playback continues from the decoded samples.
    fn end_replay(&mut self) {
        if let Some(r) = self.replay.take() {
            self.start_frame = r.resume_frame;
            self.consumed = 0;
        }
    }

    /// Whether the A-B loop wraps before the next frame, and how.
    fn wrap_due(&self) -> Option<Wrap> {
        let l = self.ab.as_ref()?;
        if self.replay.as_ref().is_some_and(|r| r.at < r.clip.len()) {
            return None;
        }
        let pos = self.pos();
        // B past the decoded end of the track counts as the end.
        let at_end = self.done && self.samples.is_empty();
        if pos < l.b && !at_end {
            return None;
        }
        match &l.ready {
            Some(clip) if pos == l.b => Some(Wrap::Replay(clip.clone())),
            _ => Some(Wrap::Reload),
        }
    }

    /// Keeps the frame just played if it continues an A..B pass.
    fn record(&mut self, frame: &[f32]) {
        let pos = self.pos().saturating_sub(1);
        let Some(l) = self.ab.as_mut() else { return };
        if l.ready.is_some() {
            return;
        }
        let ch = frame.len().max(1);
        if pos == l.a {
            l.clip.clear();
            let want = (l.b - l.a) as usize * ch;
            l.building = want <= MAX_LOOP_SAMPLES;
            if l.building {
                l.clip.reserve_exact(want);
            }
        }
        if !l.building {
            return;
        }
        if pos != l.a + (l.clip.len() / ch) as u64 || pos >= l.b {
            l.building = false;
            l.clip = Vec::new();
            return;
        }
        l.clip.extend_from_slice(frame);
        if pos + 1 == l.b {
            l.building = false;
            l.ready = Some(Arc::new(std::mem::take(&mut l.clip)));
        }
    }
}

/// A resampler left running at the end of a track so the next track (same
/// source format) continues through it without edge transients.
struct Handoff {
    prev_key: u64,
    conv: Converter,
    fmt: OutputFormat,
}

struct Pending {
    key: u64,
    tag: String,
    track: Track,
}

struct Mix {
    format: OutputFormat,
    output_open: bool,
    segments: VecDeque<Segment>,
    next: Option<Pending>,
    playing: bool,
    gain: f32,
    volume_target: f32,
    volume: f32,
    tail: VecDeque<f32>,
    fade_in: usize,
    underrun: bool,
    drained_sent: bool,
    handoff: Option<Handoff>,
}

impl Mix {
    fn ch(&self) -> usize {
        self.format.channels as usize
    }
    fn ms_to_frames(&self, ms: u64) -> u64 {
        ms * self.format.sample_rate as u64 / 1000
    }
    fn frames_to_ms(&self, f: u64) -> u64 {
        let rate = self.format.sample_rate.max(1) as u64;
        (f * 1000 + rate / 2) / rate
    }
    fn buffered_frames(&self) -> usize {
        let ch = self.ch();
        self.segments.iter().map(|s| s.samples.len() / ch).sum()
    }
    fn seg_mut(&mut self, key: u64) -> Option<&mut Segment> {
        self.segments.iter_mut().find(|s| s.key == key)
    }
    fn declick_frames(&self, cfg: &EngineConfig) -> usize {
        self.ms_to_frames(cfg.declick_ms) as usize
    }

    /// Moves the next few ms of audible audio into a faded-out tail so a
    /// flush/seek doesn't click.
    fn make_tail(&mut self, cfg: &EngineConfig) {
        let n = self.declick_frames(cfg);
        if n == 0 || !self.playing || self.gain <= 0.0 {
            return;
        }
        let ch = self.ch();
        let Some(front) = self.segments.front() else { return };
        let avail = (front.samples.len() / ch).min(n);
        let mut tail = VecDeque::with_capacity(avail * ch);
        for f in 0..avail {
            let g = 1.0 - (f + 1) as f32 / (avail + 1) as f32;
            for c in 0..ch {
                tail.push_back(front.samples[f * ch + c] * g);
            }
        }
        self.tail = tail;
        self.fade_in = n;
    }

    fn cancel_all(&mut self) {
        for s in self.segments.drain(..) {
            s.ctl.cancel.store(true, Ordering::SeqCst);
        }
        self.handoff = None;
    }

    /// Appends samples to a segment regardless of its state (the delayed
    /// tail of a continued resampler). False if the segment is gone.
    fn append_to(&mut self, key: u64, data: &[f32]) -> bool {
        match self.seg_mut(key) {
            Some(s) => {
                s.samples.extend(data.iter().copied());
                true
            }
            None => false,
        }
    }

    /// Re-decodes the front segment from `target` (frames) / `ms`. The
    /// preloaded next is dropped and re-chained after it.
    fn reload_front(&mut self, target: u64, ms: u64) {
        let mut rechain: Option<Pending> = None;
        while self.segments.len() > 1 {
            let s = self.segments.pop_back().expect("len > 1");
            s.ctl.cancel.store(true, Ordering::SeqCst);
            if self.next.is_none() {
                rechain = Some(Pending {
                    key: s.key,
                    tag: s.tag,
                    track: s.track,
                });
            }
        }
        if rechain.is_some() {
            self.next = rechain;
        }
        // The front re-decodes; a parked resampler for it is stale.
        self.handoff = None;
        let Some(front) = self.segments.front_mut() else { return };
        front.replay = None;
        front.samples.clear();
        front.generation += 1;
        front.start_frame = target;
        front.consumed = 0;
        front.done = false;
        *front.ctl.seek.lock() = Some((front.generation, ms));
        self.drained_sent = false;
    }

    /// The front reached its loop's B: go back to A.
    fn wrap_front(&mut self, wrap: Wrap, cfg: &EngineConfig) {
        self.make_tail(cfg);
        let Some(front) = self.segments.front_mut() else { return };
        let Some((a, a_ms)) = front.ab.as_ref().map(|l| (l.a, l.a_ms)) else { return };
        match wrap {
            Wrap::Replay(clip) => {
                let resume_frame = front.samples_start();
                front.replay = Some(Replay { clip, at: 0, resume_frame });
                front.start_frame = a;
                front.consumed = 0;
            }
            Wrap::Reload => {
                front.end_replay();
                self.reload_front(a, a_ms);
            }
        }
    }

    /// The output rate changed: loop points move to the new rate and saved
    /// loop audio is dropped.
    fn rescale_loops(&mut self) {
        let rate = self.format.sample_rate as u64;
        for s in self.segments.iter_mut() {
            s.end_replay();
            if let Some(l) = s.ab.as_mut() {
                l.a = l.a_ms * rate / 1000;
                l.b = l.b_ms * rate / 1000;
                l.clip = Vec::new();
                l.building = false;
                l.ready = None;
            }
        }
    }

    /// No next track after all: flush the parked resampler into its track.
    fn flush_handoff(&mut self) {
        if let Some(mut h) = self.handoff.take() {
            let mut out = Vec::new();
            h.conv.finish(&mut out);
            self.append_to(h.prev_key, &out);
        }
    }
}

/// Shared between engine, loaders and the output thread.
pub(crate) struct Core {
    mix: Mutex<Mix>,
    cv: Condvar,
    events: UnboundedSender<EngineEvent>,
    cfg: EngineConfig,
}

impl Renderer for Core {
    fn render(&self, out: &mut [f32]) -> usize {
        let mut events: Vec<EngineEvent> = Vec::new();
        let mut m = self.mix.lock();
        let ch = m.ch();
        let frames = out.len() / ch;
        let ramp_step = 1.0 / m.ms_to_frames(self.cfg.ramp_ms).max(1) as f32;
        let fade_len = m.declick_frames(&self.cfg).max(1);
        let mut filled = 0;
        let mut consumed_any = false;
        let mut underrun = false;
        let mut f = 0;
        while f < frames {
            let o = &mut out[f * ch..(f + 1) * ch];
            // Volume smoothing (~20 ms) avoids zipper noise.
            m.volume += (m.volume_target - m.volume) * 0.002;
            if !m.tail.is_empty() {
                let v = m.volume * m.gain;
                for s in o.iter_mut() {
                    *s = m.tail.pop_front().unwrap_or(0.0) * v;
                }
                filled += 1;
                f += 1;
                continue;
            }
            if !m.playing && m.gain <= 0.0 {
                break;
            }
            // A-B loop: at B, go back to A.
            if let Some(w) = m.segments.front().and_then(Segment::wrap_due) {
                m.wrap_front(w, &self.cfg);
                consumed_any = true;
                if !m.tail.is_empty() {
                    continue;
                }
            }
            // Find audio in the front segment, retiring finished ones.
            let mut got = false;
            while let Some(front) = m.segments.front_mut() {
                if let Some(r) = front.replay.as_mut() {
                    if r.at + ch <= r.clip.len() {
                        o.copy_from_slice(&r.clip[r.at..r.at + ch]);
                        r.at += ch;
                        front.consumed += 1;
                        front.listened += 1;
                        front.record(o);
                        got = true;
                        break;
                    }
                    front.end_replay();
                }
                if front.samples.len() >= ch {
                    if !front.started {
                        front.started = true;
                        events.push(EngineEvent::Started {
                            key: front.key,
                            tag: front.tag.clone(),
                        });
                    }
                    for s in o.iter_mut() {
                        *s = front.samples.pop_front().unwrap_or(0.0);
                    }
                    front.consumed += 1;
                    front.listened += 1;
                    front.record(o);
                    got = true;
                    break;
                }
                if front.done {
                    let seg = m.segments.pop_front().expect("front exists");
                    seg.ctl.cancel.store(true, Ordering::SeqCst);
                    if !seg.started {
                        events.push(EngineEvent::Started {
                            key: seg.key,
                            tag: seg.tag.clone(),
                        });
                    }
                    // The next track is now the audible one: its stream
                    // gets top transfer priority.
                    if let Some(next) = m.segments.front() {
                        next.ctl.prio.store(Priority::Current as u8, Ordering::SeqCst);
                    }
                    let listened_ms = m.frames_to_ms(seg.listened);
                    events.push(EngineEvent::Ended {
                        key: seg.key,
                        tag: seg.tag,
                        listened_ms,
                    });
                    consumed_any = true;
                    continue;
                }
                // Front isn't decoded far enough yet.
                underrun = true;
                break;
            }
            if !got {
                if m.segments.is_empty() && m.next.is_none() && !m.drained_sent {
                    m.drained_sent = true;
                    events.push(EngineEvent::Drained);
                }
                break;
            }
            consumed_any = true;
            // Gain: pause ramp, post-seek fade-in, volume.
            if m.playing {
                m.gain = (m.gain + ramp_step).min(1.0);
            } else {
                m.gain = (m.gain - ramp_step).max(0.0);
            }
            let mut g = m.gain * m.volume;
            if m.fade_in > 0 {
                g *= 1.0 - m.fade_in as f32 / (fade_len + 1) as f32;
                m.fade_in -= 1;
            }
            for s in o.iter_mut() {
                *s *= g;
            }
            filled += 1;
            f += 1;
        }
        for s in out[f * ch..].iter_mut() {
            *s = 0.0;
        }
        if underrun != m.underrun && m.playing {
            m.underrun = underrun;
            events.push(EngineEvent::Buffering(underrun));
        }
        drop(m);
        if consumed_any {
            self.cv.notify_all();
        }
        for e in events {
            let _ = self.events.send(e);
        }
        filled
    }
}

/// Everything a loader thread needs.
struct Env {
    core: Arc<Core>,
    resolver: Arc<dyn TrackResolver>,
    cache: Option<Arc<DiskCache>>,
    client: reqwest::Client,
    rt: Handle,
    gate: Arc<TransferGate>,
}

pub struct EngineDeps {
    pub resolver: Arc<dyn TrackResolver>,
    pub cache: Option<Arc<DiskCache>>,
    pub client: reqwest::Client,
    pub rt: Handle,
    pub output: Box<dyn AudioOutput>,
    pub config: EngineConfig,
    /// Shared with the pre-cacher: one HTTP body transfer at a time.
    pub gate: Arc<TransferGate>,
}

impl PrecacheHost for Core {
    fn precache_allowed(&self) -> bool {
        let m = self.mix.lock();
        // A queued next track that hasn't started loading is fine: the
        // pre-cacher fetches it itself (and the loader then finds it on disk).
        m.segments.iter().take(2).all(|s| match s.origin {
            Some(Origin::Cache) => true,
            Some(_) => s.stats.as_ref().is_some_and(|st| st.get().idle),
            None => false,
        })
    }

    fn is_streaming(&self, track: &Track) -> bool {
        self.mix
            .lock()
            .segments
            .iter()
            .any(|s| s.track.id == track.id && s.origin != Some(Origin::Cache))
    }
}

/// Position of the audible track.
#[derive(Debug, Clone, PartialEq)]
pub struct EnginePosition {
    pub key: u64,
    pub tag: String,
    pub position_ms: u64,
    pub buffered_ms: u64,
    pub listened_ms: u64,
    pub started: bool,
}

pub struct Engine {
    env: Arc<Env>,
    output: Mutex<Box<dyn AudioOutput>>,
    next_key: AtomicU64,
}

impl Engine {
    pub fn new(deps: EngineDeps) -> (Self, UnboundedReceiver<EngineEvent>) {
        let (tx, rx) = unbounded_channel();
        let core = Arc::new(Core {
            mix: Mutex::new(Mix {
                format: OutputFormat {
                    sample_rate: 48_000,
                    channels: 2,
                },
                output_open: false,
                segments: VecDeque::new(),
                next: None,
                playing: false,
                gain: 0.0,
                volume_target: 1.0,
                volume: 1.0,
                tail: VecDeque::new(),
                fade_in: 0,
                underrun: false,
                drained_sent: true,
                handoff: None,
            }),
            cv: Condvar::new(),
            events: tx,
            cfg: deps.config,
        });
        let env = Arc::new(Env {
            core,
            resolver: deps.resolver,
            cache: deps.cache,
            client: deps.client,
            rt: deps.rt,
            gate: deps.gate,
        });
        (
            Self {
                env,
                output: Mutex::new(deps.output),
                next_key: AtomicU64::new(1),
            },
            rx,
        )
    }

    /// For the pre-cacher: lets it wait until the playing/next streams are
    /// buffered and skip tracks the engine is already streaming.
    pub fn precache_host(&self) -> Arc<dyn PrecacheHost> {
        self.env.core.clone()
    }

    fn core(&self) -> &Core {
        &self.env.core
    }

    pub fn format(&self) -> OutputFormat {
        self.core().mix.lock().format
    }

    /// Opens the output if it isn't. Returns true if the format changed.
    fn ensure_output(&self) -> AppResult<bool> {
        if self.core().mix.lock().output_open {
            return Ok(false);
        }
        let weak = Arc::downgrade(&self.env.core);
        let cb: OutputCallback = Arc::new(move |ev| {
            let Some(core) = weak.upgrade() else { return };
            let ev = match ev {
                OutputEvent::DeviceLost { why, fell_back_from } => {
                    let mut m = core.mix.lock();
                    m.playing = false;
                    m.gain = 0.0;
                    m.output_open = false;
                    EngineEvent::DeviceLost { why, fell_back_from }
                }
                OutputEvent::Reset(why) => EngineEvent::OutputReset(why),
                OutputEvent::Rerouted => EngineEvent::OutputChanged,
            };
            let _ = core.events.send(ev);
        });
        let renderer: Arc<dyn Renderer> = self.env.core.clone();
        let fmt = self.output.lock().open(renderer, cb)?;
        let mut m = self.core().mix.lock();
        let changed = m.format != fmt;
        m.format = fmt;
        m.output_open = true;
        if changed {
            m.rescale_loops();
        }
        Ok(changed)
    }

    /// Releases the device (after it was lost, or on shutdown).
    pub fn close_output(&self) {
        self.output.lock().close();
        self.core().mix.lock().output_open = false;
    }

    /// The output devices, the choice and the device in use.
    pub fn output_state(&self) -> OutputState {
        let out = self.output.lock();
        OutputState::build(out.devices(), out.selected(), out.opened())
    }

    /// Chooses the output device (None follows the system default). An open
    /// stream moves there at once, keeping position and play state.
    pub fn select_output(&self, id: Option<String>) -> AppResult<()> {
        self.output.lock().select(id);
        if self.core().mix.lock().output_open {
            self.reopen_output()?;
        }
        Ok(())
    }

    /// Closes and reopens the output (new device, or a reset stream). Play
    /// state is untouched; if the device format differs, the audible track
    /// re-decodes from where it was.
    pub fn reopen_output(&self) -> AppResult<()> {
        let pos = self.position();
        self.close_output();
        let changed = self.ensure_output()?;
        if changed && let Some(p) = pos {
            self.seek_inner(p.position_ms, true);
        }
        Ok(())
    }

    fn alloc_key(&self) -> u64 {
        self.next_key.fetch_add(1, Ordering::SeqCst)
    }

    /// Flushes everything and starts `track` at `start_ms`. Returns the key.
    pub fn load(&self, track: Track, tag: String, start_ms: u64, play: bool) -> AppResult<u64> {
        if let Err(e) = self.ensure_output() {
            tracing::warn!("audio output unavailable: {e}");
            if play {
                return Err(e);
            }
        }
        let key = self.alloc_key();
        let ctl;
        {
            let mut m = self.core().mix.lock();
            m.make_tail(&self.core().cfg);
            m.cancel_all();
            m.next = None;
            let start_frame = m.ms_to_frames(start_ms);
            let seg = Segment::new(key, tag, track.clone(), start_frame, Priority::Current);
            ctl = seg.ctl.clone();
            m.segments.push_back(seg);
            m.playing = play;
            if !play {
                m.gain = 0.0;
                m.tail.clear();
            }
            m.drained_sent = false;
            m.underrun = false;
        }
        self.core().cv.notify_all();
        spawn_loader(self.env.clone(), key, track, start_ms, ctl);
        Ok(key)
    }

    /// Sets the track to play gaplessly after the current one. Ignored if the
    /// audible segment is no longer `current_key` (a transition raced us).
    /// Returns the key the next segment will have.
    pub fn set_next(&self, next: Option<(Track, String)>, current_key: u64) -> Option<u64> {
        let mut spawn: Option<(u64, Track, Arc<LoaderCtl>)> = None;
        let result;
        {
            let mut m = self.core().mix.lock();
            if m.segments.front().map(|s| s.key) != Some(current_key) {
                return None;
            }
            match next {
                None => {
                    while m.segments.len() > 1 {
                        let s = m.segments.pop_back().expect("len > 1");
                        s.ctl.cancel.store(true, Ordering::SeqCst);
                    }
                    m.next = None;
                    m.flush_handoff();
                    result = None;
                }
                Some((track, tag)) => {
                    // Already loading the right one?
                    if let Some(second) = m.segments.get(1)
                        && second.tag == tag
                        && second.track.id == track.id
                    {
                        return Some(second.key);
                    }
                    if let Some(p) = &m.next
                        && p.tag == tag
                        && p.track.id == track.id
                    {
                        return Some(p.key);
                    }
                    while m.segments.len() > 1 {
                        let s = m.segments.pop_back().expect("len > 1");
                        s.ctl.cancel.store(true, Ordering::SeqCst);
                    }
                    let key = self.alloc_key();
                    m.next = Some(Pending { key, tag, track });
                    m.drained_sent = false;
                    result = Some(key);
                    if m.segments.front().is_some_and(|s| s.done) {
                        spawn = take_pending(&mut m);
                    }
                }
            }
        }
        self.core().cv.notify_all();
        if let Some((key, track, ctl)) = spawn {
            spawn_loader(self.env.clone(), key, track, 0, ctl);
        }
        result
    }

    /// Starts/resumes output. Returns Ok(true) if the device format changed
    /// and the current track was reloaded at its position.
    pub fn play(&self) -> AppResult<bool> {
        let changed = self.ensure_output()?;
        if changed {
            // Buffered audio is in the old format: re-decode from here.
            if let Some(p) = self.position() {
                self.seek_inner(p.position_ms, true);
            }
        }
        let mut m = self.core().mix.lock();
        m.playing = true;
        Ok(changed)
    }

    pub fn pause(&self) {
        self.core().mix.lock().playing = false;
    }

    pub fn is_playing(&self) -> bool {
        self.core().mix.lock().playing
    }

    /// Flushes everything; nothing is audible afterwards.
    pub fn stop(&self) {
        let mut m = self.core().mix.lock();
        m.make_tail(&self.core().cfg);
        m.cancel_all();
        m.next = None;
        m.playing = false;
        m.drained_sent = true;
        drop(m);
        self.core().cv.notify_all();
    }

    /// Linear amplitude (the perceptual curve is applied by the caller).
    pub fn set_gain(&self, gain: f32) {
        self.core().mix.lock().volume_target = gain.clamp(0.0, 1.0);
    }

    /// Sets the gain instantly (no smoothing). For tests and startup.
    pub fn set_gain_now(&self, gain: f32) {
        let mut m = self.core().mix.lock();
        m.volume_target = gain.clamp(0.0, 1.0);
        m.volume = m.volume_target;
    }

    /// Seeks the audible track. Within the decoded buffer this is instant;
    /// otherwise the loader seeks its (already open) source.
    pub fn seek(&self, ms: u64) -> bool {
        self.seek_inner(ms, false)
    }

    fn seek_inner(&self, ms: u64, force_reload: bool) -> bool {
        {
            let mut m = self.core().mix.lock();
            let cfg = self.core().cfg.clone();
            let ch = m.ch();
            let target = m.ms_to_frames(ms);
            let Some(front) = m.segments.front_mut() else { return false };
            // A replay's audio isn't in `samples`: continue from those.
            front.end_replay();
            let pos = front.pos();
            let buffered = (front.samples.len() / ch) as u64;
            if !force_reload && target >= pos && target <= pos + buffered {
                m.make_tail(&cfg);
                let front = m.segments.front_mut().expect("front");
                let skip = (target - pos) as usize;
                front.samples.drain(..skip * ch);
                front.consumed += skip as u64;
                drop(m);
                self.core().cv.notify_all();
                return true;
            }
            m.make_tail(&cfg);
            m.reload_front(target, ms);
        }
        self.core().cv.notify_all();
        true
    }

    /// Where the audible (or about-to-be-audible) track is.
    pub fn position(&self) -> Option<EnginePosition> {
        let m = self.core().mix.lock();
        let ch = m.ch();
        let front = m.segments.front()?;
        let pos = front.pos();
        let decoded_end = front.samples_start() + (front.samples.len() / ch) as u64;
        let mut buffered_ms = if front.done {
            front.track.duration_ms
        } else {
            m.frames_to_ms(decoded_end)
        };
        if front.origin == Some(Origin::Cache) {
            buffered_ms = front.track.duration_ms;
        } else if let Some(st) = &front.stats {
            let s = st.get();
            if let Some(total) = s.total.filter(|t| *t > 0) {
                let frac = s.buffered_end as f64 / total as f64;
                buffered_ms = buffered_ms.max((frac * front.track.duration_ms as f64) as u64);
            }
        }
        Some(EnginePosition {
            key: front.key,
            tag: front.tag.clone(),
            position_ms: m.frames_to_ms(pos),
            buffered_ms: if front.track.duration_ms > 0 {
                buffered_ms.min(front.track.duration_ms)
            } else {
                buffered_ms
            },
            listened_ms: m.frames_to_ms(front.listened),
            started: front.started,
        })
    }

    /// Sets or clears the A-B loop (ms, A before B) on the audible track.
    /// False if `key` isn't the audible track.
    pub fn set_loop(&self, key: u64, ab: Option<(u64, u64)>) -> bool {
        let mut m = self.core().mix.lock();
        let lp = ab.map(|(a, b)| AbLoop::new(a, b, &m));
        match m.segments.front_mut() {
            Some(front) if front.key == key => {
                // A running replay is the track's own audio, so it plays on.
                front.ab = lp;
                true
            }
            _ => false,
        }
    }

    /// Key of the preloaded next segment (if it exists yet).
    pub fn next_key(&self) -> Option<u64> {
        let m = self.core().mix.lock();
        m.segments
            .get(1)
            .map(|s| s.key)
            .or_else(|| m.next.as_ref().map(|p| p.key))
    }

    /// Where the audible track's audio comes from (cache, stream, transcode).
    pub fn origin(&self) -> Option<Origin> {
        self.core().mix.lock().segments.front().and_then(|s| s.origin)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.core().mix.lock().cancel_all();
        self.core().cv.notify_all();
        self.output.lock().close();
    }
}

fn take_pending(m: &mut Mix) -> Option<(u64, Track, Arc<LoaderCtl>)> {
    let p = m.next.take()?;
    let seg = Segment::new(p.key, p.tag, p.track.clone(), 0, Priority::Next);
    let ctl = seg.ctl.clone();
    m.segments.push_back(seg);
    Some((p.key, p.track, ctl))
}

fn spawn_loader(env: Arc<Env>, key: u64, track: Track, start_ms: u64, ctl: Arc<LoaderCtl>) {
    let _ = std::thread::Builder::new()
        .name(format!("decode-{key}"))
        .spawn(move || run_loader(env, key, track, start_ms, ctl));
}

enum OpenFail {
    /// Don't bother with the transcoder (missing file, auth).
    Fatal(AppError),
    /// Try the transcoder.
    Retry(AppError),
}

struct Opened {
    dec: TrackDecoder,
    stats: Option<HttpStats>,
    origin: Origin,
    /// Bytes to store in the disk cache once fully fetched.
    cache_from: Option<HttpStats>,
}

fn open_from(
    env: &Env,
    track: &Track,
    src: StreamSource,
    origin: Origin,
    prio: &PriorityCell,
) -> Result<Opened, OpenFail> {
    let hint = src
        .codec_hint
        .clone()
        .or_else(|| extension_hint(track.codec.as_deref(), track.container.as_deref()));
    let http = HttpSource::open(
        env.rt.clone(),
        env.client.clone(),
        src,
        env.core.cfg.http.clone(),
        env.gate.clone(),
        prio.clone(),
    )
    .map_err(|e| match e {
        AppError::NotFound(_) | AppError::Auth(_) => OpenFail::Fatal(e),
        other => OpenFail::Retry(other),
    })?;
    let stats = http.stats_handle();
    let dec = TrackDecoder::open(Box::new(http), hint.as_deref()).map_err(|e| match e {
        DecodeError::Io(m) => OpenFail::Retry(AppError::Network(m)),
        other => OpenFail::Retry(AppError::Decode(other.to_string())),
    })?;
    Ok(Opened {
        dec,
        stats: Some(stats.clone()),
        origin,
        cache_from: (origin == Origin::Stream).then_some(stats),
    })
}

fn open_track(
    env: &Env,
    track: &Track,
    transcode: bool,
    prio: &PriorityCell,
) -> Result<Opened, OpenFail> {
    if transcode {
        let src = env
            .rt
            .block_on(env.resolver.transcode(track))
            .map_err(OpenFail::Fatal)?;
        return open_from(env, track, src, Origin::Transcode, prio);
    }
    if let Some(cache) = &env.cache
        && let Some(path) = cache.get(&cache_key(track))
        && let Ok(file) = std::fs::File::open(&path)
    {
        let hint = extension_hint(track.codec.as_deref(), track.container.as_deref());
        match TrackDecoder::open(Box::new(file), hint.as_deref()) {
            Ok(dec) => {
                return Ok(Opened {
                    dec,
                    stats: None,
                    origin: Origin::Cache,
                    cache_from: None,
                });
            }
            Err(e) => tracing::debug!("cached copy of {} unusable: {e}", track.id),
        }
    }
    let src = env
        .rt
        .block_on(env.resolver.stream(track))
        .map_err(|e| match e {
            AppError::NotFound(_) | AppError::Auth(_) => OpenFail::Fatal(e),
            other => OpenFail::Retry(other),
        })?;
    open_from(env, track, src, Origin::Stream, prio)
}

/// Push result for a decoded block.
enum Push {
    Ok,
    /// Seek happened (generation changed) or segment gone.
    Stale,
    Cancelled,
}

fn push_samples(core: &Core, key: u64, generation: u64, ctl: &LoaderCtl, data: &[f32]) -> Push {
    let mut m = core.mix.lock();
    loop {
        if ctl.cancel.load(Ordering::SeqCst) {
            return Push::Cancelled;
        }
        let cap = m.ms_to_frames(core.cfg.buffer_ahead_ms) as usize;
        let buffered = m.buffered_frames();
        let Some(seg) = m.seg_mut(key) else { return Push::Cancelled };
        if seg.generation != generation {
            return Push::Stale;
        }
        if buffered < cap || seg.samples.is_empty() {
            seg.samples.extend(data.iter().copied());
            return Push::Ok;
        }
        core.cv.wait_for(&mut m, Duration::from_millis(50));
    }
}

/// Decoding finished for this generation; chain the next track.
fn mark_done(env: &Arc<Env>, key: u64, generation: u64) {
    let spawn = {
        let mut m = env.core.mix.lock();
        let is_last = m.segments.back().map(|s| s.key) == Some(key);
        match m.seg_mut(key) {
            Some(seg) if seg.generation == generation => seg.done = true,
            _ => return,
        }
        if is_last { take_pending(&mut m) } else { None }
    };
    if let Some((k, t, ctl)) = spawn {
        spawn_loader(env.clone(), k, t, 0, ctl);
    }
}

fn fail(env: &Env, key: u64, track: &Track, error: AppError) {
    let tag = {
        let mut m = env.core.mix.lock();
        let Some(idx) = m.segments.iter().position(|s| s.key == key) else { return };
        let seg = m.segments.remove(idx).expect("index valid");
        seg.tag
    };
    env.core.cv.notify_all();
    let _ = env.core.events.send(EngineEvent::Failed {
        key,
        tag,
        track_id: track.id.clone(),
        error,
    });
}

fn run_loader(env: Arc<Env>, key: u64, track: Track, start_ms: u64, ctl: Arc<LoaderCtl>) {
    let core = env.core.clone();
    // 1. Open: cache → original stream → transcoder.
    let mut transcoded = false;
    let mut opened = match open_track(&env, &track, false, &ctl.prio) {
        Ok(o) => o,
        Err(OpenFail::Fatal(e)) => return fail(&env, key, &track, e),
        Err(OpenFail::Retry(e)) => {
            tracing::info!("track {} not playable directly ({e}); trying transcode", track.id);
            transcoded = true;
            match open_track(&env, &track, true, &ctl.prio) {
                Ok(o) => o,
                Err(OpenFail::Fatal(e) | OpenFail::Retry(e)) => return fail(&env, key, &track, e),
            }
        }
    };
    if ctl.cancel.load(Ordering::SeqCst) {
        return;
    }
    let mut generation = 0u64;
    let mut seek_to = (start_ms > 0).then_some(start_ms);
    let mut out_fmt;
    {
        let mut m = core.mix.lock();
        out_fmt = m.format;
        if let Some(seg) = m.seg_mut(key) {
            seg.stats = opened.stats.clone();
            seg.origin = Some(opened.origin);
            generation = seg.generation;
            // A seek may have been requested while we were opening.
            if let Some((g, ms)) = seg.ctl.seek.lock().take() {
                generation = g;
                seek_to = Some(ms);
            }
        }
    }
    let mut conv = Converter::new(
        opened.dec.rate,
        opened.dec.channels,
        out_fmt.sample_rate,
        out_fmt.channels as usize,
    );
    // Continue the previous track's resampler if it was parked for us, so
    // the join has no filter edge transients. Its first output frames are
    // the previous track's delayed tail and are routed back to it.
    let mut owe_prev: Option<(u64, usize)> = None;
    {
        let mut m = core.mix.lock();
        // The parked resampler belongs to the segment just before us (which
        // may already have finished playing and been retired).
        let idx = m.segments.iter().position(|s| s.key == key);
        let prev_key = idx
            .and_then(|i| i.checked_sub(1))
            .map(|i| m.segments[i].key);
        let for_us = m.handoff.as_ref().is_some_and(|h| {
            Some(h.prev_key) == prev_key
                || (idx == Some(0) && !m.segments.iter().any(|s| s.key == h.prev_key))
        });
        if for_us {
            let mut h = m.handoff.take().expect("checked above");
            if seek_to.is_none()
                && h.fmt == out_fmt
                && h.conv.input_format() == (opened.dec.rate, opened.dec.channels)
            {
                let owed = h.conv.continue_track() as usize;
                conv = h.conv;
                owe_prev = Some((h.prev_key, owed));
            } else {
                let mut tail = Vec::new();
                h.conv.finish(&mut tail);
                m.append_to(h.prev_key, &tail);
            }
        }
    }
    let mut out: Vec<f32> = Vec::with_capacity(16 * 1024);
    let mut pushed_any = false;
    let mut cached = false;

    'decode: loop {
        if ctl.cancel.load(Ordering::SeqCst) {
            return;
        }
        if let Some(ms) = seek_to.take() {
            // Device format may have changed since the converter was built.
            let fmt = core.mix.lock().format;
            if fmt != out_fmt {
                out_fmt = fmt;
            }
            conv = Converter::new(
                opened.dec.rate,
                opened.dec.channels,
                out_fmt.sample_rate,
                out_fmt.channels as usize,
            );
            if let Err(e) = opened.dec.seek_ms(ms) {
                tracing::warn!("seek failed on {}: {e}", track.id);
                // Reopen from scratch and try once more.
                match open_track(&env, &track, transcoded, &ctl.prio) {
                    Ok(o) => {
                        opened = o;
                        if opened.dec.seek_ms(ms).is_err() {
                            mark_done(&env, key, generation);
                            continue 'decode;
                        }
                    }
                    Err(OpenFail::Fatal(e) | OpenFail::Retry(e)) => {
                        return fail(&env, key, &track, e);
                    }
                }
            }
        }
        match opened.dec.next_block() {
            Ok(Some(block)) => {
                out.clear();
                conv.push(block, &mut out);
                let mut slice: &[f32] = &out;
                if let Some((pk, n)) = owe_prev.as_mut()
                    && *n > 0
                {
                    let ch = out_fmt.channels as usize;
                    let take = (*n).min(slice.len() / ch);
                    if core.mix.lock().append_to(*pk, &slice[..take * ch]) {
                        slice = &slice[take * ch..];
                        *n -= take;
                    } else {
                        *n = 0; // previous track already gone: keep them here
                    }
                }
                if slice.is_empty() {
                    continue;
                }
                match push_samples(&core, key, generation, &ctl, slice) {
                    Push::Ok => pushed_any = true,
                    Push::Cancelled => return,
                    Push::Stale => {
                        if let Some((g, ms)) = ctl.seek.lock().take() {
                            generation = g;
                            seek_to = Some(ms);
                        }
                    }
                }
            }
            Ok(None) => {
                out.clear();
                // If a next track is queued, park the resampler for it
                // instead of flushing (see `Handoff`).
                let parked = !conv.is_passthrough() && {
                    let mut m = core.mix.lock();
                    let ok = m.next.is_some()
                        && m.handoff.is_none()
                        && m.segments.back().map(|s| (s.key, s.generation)) == Some((key, generation));
                    if ok {
                        let fresh = Converter::new(
                            opened.dec.rate,
                            opened.dec.channels,
                            out_fmt.sample_rate,
                            out_fmt.channels as usize,
                        );
                        m.handoff = Some(Handoff {
                            prev_key: key,
                            conv: std::mem::replace(&mut conv, fresh),
                            fmt: out_fmt,
                        });
                    }
                    ok
                };
                if !parked {
                    conv.finish(&mut out);
                }
                if !out.is_empty() {
                    match push_samples(&core, key, generation, &ctl, &out) {
                        Push::Cancelled => return,
                        Push::Stale => {
                            if let Some((g, ms)) = ctl.seek.lock().take() {
                                generation = g;
                                seek_to = Some(ms);
                            }
                            continue 'decode;
                        }
                        Push::Ok => {}
                    }
                }
                mark_done(&env, key, generation);
                // Save a fully streamed file to the disk cache.
                if !cached
                    && let (Some(cache), Some(st)) = (&env.cache, &opened.cache_from)
                    && let Some(bytes) = st.completed_bytes()
                {
                    cached = true;
                    if let Err(e) = cache.insert_bytes(&cache_key(&track), &bytes) {
                        tracing::debug!("could not cache {}: {e}", track.id);
                    }
                }
                // Idle until a seek or cancel.
                loop {
                    if ctl.cancel.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Some((g, ms)) = ctl.seek.lock().take() {
                        generation = g;
                        seek_to = Some(ms);
                        continue 'decode;
                    }
                    let mut m = core.mix.lock();
                    core.cv.wait_for(&mut m, Duration::from_millis(50));
                }
            }
            Err(e) => {
                if !pushed_any && !transcoded && !matches!(e, DecodeError::Io(_)) {
                    tracing::info!("decode of {} failed ({e}); trying transcode", track.id);
                    transcoded = true;
                    match open_track(&env, &track, true, &ctl.prio) {
                        Ok(o) => {
                            opened = o;
                            conv = Converter::new(
                                opened.dec.rate,
                                opened.dec.channels,
                                out_fmt.sample_rate,
                                out_fmt.channels as usize,
                            );
                            if start_ms > 0 {
                                seek_to = Some(start_ms);
                            }
                            continue;
                        }
                        Err(OpenFail::Fatal(e) | OpenFail::Retry(e)) => {
                            return fail(&env, key, &track, e);
                        }
                    }
                }
                if !pushed_any {
                    let err = match e {
                        DecodeError::Io(m) => AppError::Network(m),
                        other => AppError::Decode(other.to_string()),
                    };
                    return fail(&env, key, &track, err);
                }
                // Mid-track failure: play what we have, then move on.
                tracing::warn!("track {} ended early: {e}", track.id);
                mark_done(&env, key, generation);
                loop {
                    if ctl.cancel.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Some((g, ms)) = ctl.seek.lock().take() {
                        generation = g;
                        seek_to = Some(ms);
                        continue 'decode;
                    }
                    let mut m = core.mix.lock();
                    core.cv.wait_for(&mut m, Duration::from_millis(50));
                }
            }
        }
    }
}
