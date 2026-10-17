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
//! track. Each segment carries its loudness leveling gain; near the end of
//! a track the next one can fade in over it (the outgoing segment moves to
//! `Mix::fading` and the next becomes the front). The mixed audio then runs
//! through the EQ and limiter ([`super::dsp`]).
//!
//! Automix blends ([`super::automix`]) run the same way, with a plan per
//! pair of segments: the outgoing song can change speed to match the
//! incoming one's tempo, the volumes follow the plan's curve, and the
//! incoming song only becomes the audible track (Started / Ended) at the
//! blend's midpoint. Until then the position is the outgoing song's.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::automix::{self, BlendPlan, Curve, Echo};
use super::cache::{DiskCache, PrecacheHost, cache_key};
use super::convert::Converter;
use super::decode::{DecodeError, TrackDecoder, extension_hint};
use super::dsp::{self, Chain, Crossfade, DspSettings, SilenceMeter, TrackAudio, level_gain};
use super::gate::{Priority, PriorityCell, TransferGate};
use super::http::{HttpConfig, HttpSource, HttpStats};
use super::devguard::GuardedOutput;
use super::output::{OutputCallback, OutputEvent, OutputFormat, OutputState, Renderer};
use super::source::{LocalFiles, StreamSource, TrackResolver};
use crate::error::{AppError, AppResult};
use crate::model::Track;

/// How long a loader waits for a track's loudness data.
const INFO_WAIT: Duration = Duration::from_millis(1500);
/// How long the loudness data of a song that isn't playing yet may take;
/// its level is set when it comes.
const LATE_INFO_WAIT: Duration = Duration::from_secs(20);
/// The next song is opened (connection, format) this long before the
/// playing one's decoded audio runs out, so a slow link has time.
const PREOPEN_MS: u64 = 45_000;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Decoded audio kept ahead of the play position (all segments).
    pub buffer_ahead_ms: u64,
    /// Pause/resume gain ramp.
    pub ramp_ms: u64,
    /// Fade-out/in applied around seeks and skips (0 disables).
    pub declick_ms: u64,
    pub http: HttpConfig,
    /// Longest the player waits on the audio device (open, close, list)
    /// before giving up on it for now.
    pub device_timeout: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            buffer_ahead_ms: 12_000,
            ramp_ms: 10,
            declick_ms: 5,
            http: HttpConfig::default(),
            device_timeout: Duration::from_secs(8),
        }
    }
}

/// Engine → controller notifications.
#[derive(Debug)]
pub enum EngineEvent {
    /// The segment's first sample just played (track became audible).
    Started { key: u64, tag: String },
    /// The segment played to its end, or to where its audio ran out when
    /// `early` says the rest couldn't be fetched or decoded.
    Ended { key: u64, tag: String, listened_ms: u64, early: Option<EndedEarly> },
    /// The track could not be played (after the transcode fallback).
    /// `at_ms` is where it was meant to start.
    Failed { key: u64, tag: String, track_id: String, error: AppError, at_ms: u64 },
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

/// A track whose audio stopped before its end.
#[derive(Debug)]
pub struct EndedEarly {
    pub error: AppError,
    /// Where the audio ran out.
    pub at_ms: u64,
}

/// Whether an error means the server can't be reached right now (the
/// network, or the sign-in), rather than something wrong with one file.
pub fn unreachable(e: &AppError) -> bool {
    matches!(e, AppError::Network(_) | AppError::Auth(_))
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
    /// Loudness data and remembered silence (from the resolver).
    audio: Option<TrackAudio>,
    /// Quiet lead-in / run-out measured while decoding (output frames).
    head: Option<u64>,
    tail: Option<u64>,
    /// Leveling gain (linear), ramping towards `lg_target`.
    lg: f32,
    lg_target: f32,
    lg_step: f32,
    /// Highest level this track reaches after its target gain.
    peak: f32,
    /// Whether to crossfade into the next track has been decided.
    xf_decided: bool,
    /// The Automix blend into the next segment (by its key).
    blend: Option<(u64, BlendPlan)>,
    /// Its buffers and names, made when it was planned so starting it on
    /// the output thread allocates nothing.
    blend_kit: Option<BlendKit>,
    /// Whether that blend was started or given up (a seek resets it).
    bl_decided: bool,
    /// Decoding stopped before the end of the track (reported with its end).
    early: Option<AppError>,
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
            audio: None,
            head: None,
            tail: None,
            lg: 1.0,
            lg_target: 1.0,
            lg_step: 0.0,
            peak: dsp::UNKNOWN_PEAK,
            xf_decided: false,
            blend: None,
            blend_kit: None,
            bl_decided: false,
            early: None,
        }
    }

    /// Sets the leveling gain: at once, or ramped over `frames`.
    fn set_gain(&mut self, g: f32, peak: f32, frames: usize) {
        self.lg_target = g;
        self.peak = peak;
        if frames == 0 || self.lg == g {
            self.lg = g;
            self.lg_step = 0.0;
        } else {
            self.lg_step = (g - self.lg) / frames as f32;
        }
    }

    /// Applies (and advances) the leveling gain on one frame.
    #[inline]
    fn apply_gain(&mut self, o: &mut [f32]) {
        if self.lg != 1.0 {
            for s in o.iter_mut() {
                *s *= self.lg;
            }
        }
        if self.lg != self.lg_target {
            self.lg += self.lg_step;
            let past = if self.lg_step > 0.0 {
                self.lg >= self.lg_target
            } else {
                self.lg <= self.lg_target
            };
            if past || self.lg_step == 0.0 {
                self.lg = self.lg_target;
            }
        }
    }

    /// Quiet lead-in in frames: measured, else remembered.
    fn head_frames(&self, rate: u64) -> Option<u64> {
        self.head
            .or_else(|| self.audio.and_then(|a| a.head_ms).map(|ms| ms * rate / 1000))
    }

    fn tail_frames(&self, rate: u64) -> Option<u64> {
        self.tail
            .or_else(|| self.audio.and_then(|a| a.tail_ms).map(|ms| ms * rate / 1000))
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

/// The outgoing track during a crossfade: its last `len` frames play under
/// the start of the new front segment.
struct Fading {
    seg: Segment,
    len: usize,
    pos: usize,
    /// The outgoing track fades over the last `a_fade` frames of the
    /// overlap; the incoming one over its first `b_fade`.
    a_fade: usize,
    b_fade: usize,
    /// An Automix blend (the fades above are unused then).
    blend: Option<Blend>,
}

/// What a blend needs, made ahead on the engine's caller thread.
struct BlendKit {
    rate: u32,
    ch: usize,
    beat_ms: f64,
    echo: Option<Echo>,
    frame: Vec<f32>,
    a_tag: String,
    b_tag: String,
}

impl BlendKit {
    fn new(plan: &BlendPlan, rate: u32, ch: usize, a_tag: String, b_tag: String) -> Self {
        let per_ms = rate as f64 / 1000.0;
        let echo = (plan.curve == Curve::Echo).then(|| Echo::new((0.75 * plan.beat_ms * per_ms) as usize, ch));
        Self { rate, ch, beat_ms: plan.beat_ms, echo, frame: vec![0.0; ch], a_tag, b_tag }
    }

    fn fits(&self, plan: &BlendPlan, rate: u32, ch: usize, b_tag: &str) -> bool {
        self.rate == rate
            && self.ch == ch
            && self.beat_ms == plan.beat_ms
            && self.echo.is_some() == (plan.curve == Curve::Echo)
            && self.b_tag == b_tag
    }
}

/// A running Automix blend.
struct Blend {
    plan: BlendPlan,
    /// Output frames per ms.
    per_ms: f64,
    /// How far into the outgoing segment's next frame playback is (its
    /// speed can differ from 1).
    frac: f64,
    /// Frame of the blend where the incoming song takes over; None once it has.
    handover: Option<usize>,
    /// The incoming segment.
    b_key: u64,
    /// For the events at the handover (taken then).
    a_tag: String,
    b_tag: String,
    echo: Option<Echo>,
    /// A skip mid-blend: from these gains to (0, 1) over `n` frames from `at`.
    hurry: Option<(usize, usize, f32, f32)>,
    /// One frame of the outgoing song.
    frame: Vec<f32>,
}

/// How long a skip during a blend takes to clear the outgoing song.
const HURRY_MS: f64 = 40.0;

impl Fading {
    /// Volumes now (outgoing, incoming).
    fn gains(&self) -> (f32, f32) {
        match &self.blend {
            Some(b) => {
                if let Some((at, n, ga, gb)) = b.hurry {
                    let x = ((self.pos.saturating_sub(at)) as f32 / n.max(1) as f32).min(1.0);
                    return (ga * (1.0 - x), gb + (1.0 - gb) * x);
                }
                let t = self.pos as f64 / b.per_ms;
                let len = self.len as f64 / b.per_ms;
                automix::gains(b.plan.curve, t, len, b.plan.beat_ms)
            }
            None => (self.a_gain(), self.b_gain()),
        }
    }
    fn a_gain(&self) -> f32 {
        let start = self.len - self.a_fade;
        if self.pos < start {
            1.0
        } else {
            dsp::fade_out(self.pos - start, self.a_fade)
        }
    }
    fn b_gain(&self) -> f32 {
        if self.pos < self.b_fade {
            dsp::fade_in(self.pos, self.b_fade)
        } else {
            1.0
        }
    }
    /// A blend whose incoming song hasn't taken over yet: the outgoing one
    /// is still the playing track.
    fn handing_over(&self) -> bool {
        self.blend.as_ref().is_some_and(|b| b.handover.is_some())
    }
}

use automix::album_in_order;

/// What fetching the next frame from the segments found.
enum Fetch {
    Got,
    /// The front segment isn't decoded far enough yet.
    Underrun,
    /// No segments left.
    Empty,
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
    /// Level while the stream runs dry: it fades out over the last few ms
    /// before an underrun and back in after, instead of cutting.
    starve_gain: f32,
    drained_sent: bool,
    handoff: Option<Handoff>,
    /// Crossfade, leveling, EQ and limiter settings.
    dsp: DspSettings,
    /// EQ and limiter.
    chain: Chain,
    /// The outgoing track while a crossfade runs.
    fading: Option<Fading>,
    /// The album playing in album order, if any (leveling picks album gain).
    album_ctx: Option<String>,
    /// Loudest the EQ curve gets (linear), for the limiter's skip test.
    eq_max: f32,
    /// The settings asked for; `dsp` is what runs (nothing while
    /// bit-perfect, no correction during an A/B comparison).
    user_dsp: DspSettings,
    /// The output is exclusive at the music's own rate: samples go out
    /// untouched (no volume, EQ, leveling, crossfade or limiter).
    bit_perfect: bool,
    /// Headphone correction is bypassed for an A/B comparison.
    corr_bypass: bool,
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
    /// A running crossfade ends here too (its outgoing track is mixed into
    /// the tail).
    fn make_tail(&mut self, cfg: &EngineConfig) {
        let n = self.declick_frames(cfg);
        if n == 0 || !self.playing || self.gain <= 0.0 {
            self.drop_fading();
            return;
        }
        let ch = self.ch();
        let Some(front) = self.segments.front() else {
            self.drop_fading();
            return;
        };
        let mut avail = (front.samples.len() / ch).min(n);
        let (ga, gb, lga) = match &self.fading {
            Some(fd) => {
                avail = avail.min(fd.seg.samples.len() / ch);
                let (ga, gb) = fd.gains();
                (ga, gb, fd.seg.lg)
            }
            None => (0.0, 1.0, 0.0),
        };
        // The last tail's buffer is reused: no allocating while the
        // output thread waits for the lock.
        let mut tail = std::mem::take(&mut self.tail);
        tail.clear();
        tail.reserve(avail * ch);
        for f in 0..avail {
            let g = 1.0 - (f + 1) as f32 / (avail + 1) as f32;
            for c in 0..ch {
                let mut v = front.samples[f * ch + c] * front.lg * gb;
                if let Some(fd) = &self.fading {
                    v += fd.seg.samples[f * ch + c] * lga * ga;
                }
                tail.push_back(v * g);
            }
        }
        self.tail = tail;
        self.fade_in = n;
        self.drop_fading();
    }

    /// What actually runs, given the user's settings and the output.
    fn effective_dsp(&self) -> DspSettings {
        if self.bit_perfect {
            return DspSettings::untouched();
        }
        let mut d = self.user_dsp;
        if self.corr_bypass {
            d.correction = None;
        }
        d
    }

    /// Applies crossfade, leveling, correction, EQ and limiter settings.
    /// `live_ok`: changes may ramp or dip if audio is flowing.
    fn apply_dsp(&mut self, live_ok: bool) {
        let s = self.effective_dsp();
        let old = self.dsp;
        self.dsp = s;
        if old.leveling != s.leveling || old.preamp_db != s.preamp_db {
            self.update_gains(true);
        }
        let live = live_ok && self.output_open && self.playing;
        if old.eq != s.eq {
            let target = s.eq.unwrap_or([0.0; dsp::EQ_BANDS]);
            let ramp = if live { self.chain.ramp_frames() } else { 0 };
            self.chain.eq.set_target(target, ramp);
        }
        if old.correction != s.correction || self.chain.correction() != s.correction {
            self.chain.set_correction(s.correction, live && !self.segments.is_empty());
        }
        if old.limiter != s.limiter {
            self.chain.set_limiter(s.limiter, live && !self.segments.is_empty());
        }
        self.refresh_eq_max(Some(old));
    }

    /// The loudest the correction and EQ can make anything (linear), while
    /// either changes taking the louder of before and after.
    fn refresh_eq_max(&mut self, old: Option<DspSettings>) {
        let rate = self.format.sample_rate;
        let eq = |b: Option<[f32; dsp::EQ_BANDS]>| b.map_or(1.0, |b| dsp::eq_max_gain(&b, rate));
        let corr = |c: Option<dsp::Correction>| c.map_or(1.0, |c| c.max_gain(rate).max(1.0));
        let cur = self.chain.eq.current();
        let mut e = eq(self.dsp.eq).max(dsp::eq_max_gain(&cur, rate));
        let mut c = corr(self.dsp.correction);
        if let Some(o) = old {
            e = e.max(eq(o.eq));
            c = c.max(corr(o.correction));
        }
        self.eq_max = e * c;
    }

    fn drop_fading(&mut self) {
        if let Some(fd) = self.fading.take() {
            fd.seg.ctl.cancel.store(true, Ordering::SeqCst);
        }
    }

    fn cancel_all(&mut self) {
        for s in self.segments.drain(..) {
            s.ctl.cancel.store(true, Ordering::SeqCst);
        }
        self.drop_fading();
        self.handoff = None;
    }

    /// Recomputes leveling gains: on every segment (a setting changed;
    /// audible ones ramp), or only on those not audible yet (the context
    /// changed, which applies from the next track).
    fn update_gains(&mut self, all: bool) {
        let ramp = self.chain.ramp_frames();
        let dsp = self.dsp;
        let ctx = self.album_ctx.as_deref();
        let fading = self.fading.as_mut().map(|f| &mut f.seg);
        for s in self.segments.iter_mut().chain(fading) {
            if !all && s.started {
                continue;
            }
            let album = ctx.is_some_and(|c| c == s.track.album_id);
            let (g, peak) = level_gain(s.audio.as_ref(), &dsp, album);
            s.set_gain(g, peak, if s.started { ramp } else { 0 });
        }
    }

    /// Whether the mix can reach the limiter's ceiling (if not, its gain
    /// work is skipped).
    fn can_clip(&self) -> bool {
        let level = |s: &Segment| s.peak / s.lg_target.max(1e-9) * s.lg.max(s.lg_target);
        let mut sum: f32 = self.segments.front().map_or(0.0, level);
        if let Some(fd) = &self.fading {
            sum += level(&fd.seg);
        }
        let tail = self.tail.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        sum.max(tail) * self.eq_max > dsp::LIMIT_CEILING
    }

    /// How many frames the front and the next segment should overlap.
    fn overlap_frames(&self, a: &Segment, b: &Segment) -> u64 {
        let rate = self.format.sample_rate as u64;
        // A fixed fade takes at most half of either track; a smart overlap
        // (quiet parts only) at most the whole of either.
        let part = |t: &Track, div: u64| if t.duration_ms > 0 { t.duration_ms * rate / (1000 * div) } else { u64::MAX };
        let cap = |frames: u64, div: u64| frames.min(part(&a.track, div)).min(part(&b.track, div));
        if self.dsp.album_gapless && album_in_order(&a.track, &b.track) {
            return 0;
        }
        match self.dsp.crossfade {
            Crossfade::Off => 0,
            Crossfade::Fixed { ms } => cap(ms * rate / 1000, 2),
            Crossfade::Smart => {
                let tail = a.tail_frames(rate).unwrap_or(0);
                let head = b.head_frames(rate).unwrap_or(0);
                cap((tail + head).min(dsp::SMART_OVERLAP_CAP_MS * rate / 1000), 1)
            }
        }
    }

    /// Whether a crossfade into the next segment starts at this frame. It's
    /// decided once per track, when its remaining audio first fits the
    /// overlap: if the next track isn't decoded that far yet, the change is
    /// gapless instead.
    fn crossfade_due(&mut self) -> bool {
        if self.dsp.crossfade == Crossfade::Off || self.fading.is_some() || self.segments.len() < 2 {
            return false;
        }
        let ch = self.ch();
        let (a, b) = (&self.segments[0], &self.segments[1]);
        if !a.done || !a.started || a.xf_decided || a.replay.is_some() || a.ab.is_some() {
            return false;
        }
        // An Automix blend that's still to come goes first.
        if !a.bl_decided && a.blend.as_ref().is_some_and(|(k, _)| *k == b.key) {
            return false;
        }
        let rem = (a.samples.len() / ch) as u64;
        if rem == 0 || rem > self.overlap_frames(a, b) {
            return false;
        }
        let ready = (b.samples.len() / ch) as u64 >= rem && b.replay.is_none();
        self.segments[0].xf_decided = true;
        ready
    }

    /// The decoded audio a blend needs, beyond the usual buffer (ms).
    fn blend_reserve_ms(&self) -> u64 {
        self.segments
            .iter()
            .filter_map(|s| s.blend.as_ref())
            .map(|(_, p)| (p.len_ms * p.rate.max(1.0) + p.b_start_ms) as u64 + 1_000)
            .max()
            .unwrap_or(0)
    }

    fn ms_to_frames_f(&self, ms: f64) -> u64 {
        (ms.max(0.0) * self.format.sample_rate as f64 / 1000.0).round() as u64
    }

    /// Whether the Automix blend into the next segment starts at this frame.
    /// Decided once: when the outgoing song reaches the plan's start with
    /// itself decoded to the end and the incoming one decoded far enough.
    /// Otherwise (late, not decoded yet, a loop on) no blend this time, and
    /// the crossfade setting applies.
    fn blend_due(&mut self) -> Option<BlendPlan> {
        if self.bit_perfect || self.fading.is_some() || self.segments.len() < 2 {
            return None;
        }
        let ch = self.ch();
        let (a, b) = (&self.segments[0], &self.segments[1]);
        let (k, plan) = a.blend.as_ref()?;
        if *k != b.key || a.bl_decided || !a.started || a.replay.is_some() || b.replay.is_some() {
            return None;
        }
        if a.ab.is_some() {
            self.segments[0].bl_decided = true;
            return None;
        }
        let plan = *plan;
        let len = self.ms_to_frames_f(plan.len_ms);
        let pos = a.pos();
        let start = match plan.a_start_ms {
            Some(ms) => self.ms_to_frames_f(ms),
            None => {
                let rem = (a.samples.len() / ch) as u64;
                if !a.done || rem == 0 || rem > len {
                    return None;
                }
                pos
            }
        };
        if pos < start {
            return None;
        }
        // From here it's now or never.
        self.segments[0].bl_decided = true;
        let a = &self.segments[0];
        let b = &self.segments[1];
        // A beat-matched blend that starts late would miss the bar.
        let late = pos - start > if plan.beatmatched { 2 } else { self.ms_to_frames(500) };
        let need_b = self.ms_to_frames_f(plan.b_start_ms) + len;
        let ready = a.done && a.samples.len() / ch > 1 && ((b.samples.len() / ch) as u64 >= need_b || b.done);
        if late || !ready {
            return None;
        }
        Some(plan)
    }

    /// Starts an Automix blend: the front segment moves to `fading`, the
    /// next one starts (past its lead-in) underneath it.
    fn start_blend(&mut self, plan: BlendPlan) {
        let ch = self.ch();
        let rate = self.format.sample_rate as f64;
        let per_ms = rate / 1000.0;
        let skip = self.ms_to_frames_f(plan.b_start_ms) as usize;
        let Some(mut a) = self.segments.pop_front() else { return };
        let Some(b) = self.segments.front_mut() else {
            self.segments.push_front(a);
            return;
        };
        let skip = skip.min((b.samples.len() / ch).saturating_sub(1));
        b.samples.drain(..skip * ch);
        b.consumed += skip as u64;
        b.ctl.prio.store(Priority::Current as u8, Ordering::SeqCst);
        // Audible now, but it only becomes the playing track at the midpoint.
        b.started = true;
        let b_key = b.key;
        // Made when the blend was planned; built here only if the output
        // changed since.
        let kit = match a.blend_kit.take() {
            Some(k) if k.fits(&plan, self.format.sample_rate, ch, &b.tag) => k,
            _ => BlendKit::new(&plan, self.format.sample_rate, ch, a.tag.clone(), b.tag.clone()),
        };
        // As long as planned, if the outgoing song has the audio for it.
        let have = (a.samples.len() / ch).saturating_sub(1) as f64 / per_ms;
        let mut len_ms = plan.len_ms;
        if plan.a_advance(len_ms) > have {
            let (mut lo, mut hi) = (0.0, len_ms);
            for _ in 0..40 {
                let mid = (lo + hi) / 2.0;
                if plan.a_advance(mid) > have { hi = mid } else { lo = mid }
            }
            len_ms = lo;
        }
        let len = ((len_ms * per_ms) as usize).max(1);
        self.fading = Some(Fading {
            seg: a,
            len,
            pos: 0,
            a_fade: 0,
            b_fade: 0,
            blend: Some(Blend {
                plan,
                per_ms,
                frac: 0.0,
                handover: Some(len / 2),
                b_key,
                a_tag: kit.a_tag,
                b_tag: kit.b_tag,
                echo: kit.echo,
                hurry: None,
                frame: kit.frame,
            }),
        });
    }

    /// The incoming song of a blend takes over: the outgoing one has ended.
    fn hand_over(fd: &mut Fading, frame_ms: impl Fn(u64) -> u64, events: &mut Vec<EngineEvent>) {
        let Some(b) = fd.blend.as_mut() else { return };
        if b.handover.take().is_none() {
            return;
        }
        let at_ms = frame_ms(fd.seg.pos());
        events.push(EngineEvent::Ended {
            key: fd.seg.key,
            tag: std::mem::take(&mut b.a_tag),
            listened_ms: frame_ms(fd.seg.listened),
            early: fd.seg.early.take().map(|error| EndedEarly { error, at_ms }),
        });
        events.push(EngineEvent::Started {
            key: b.b_key,
            tag: std::mem::take(&mut b.b_tag),
        });
    }

    /// A blend that hasn't reached its midpoint goes back to the outgoing
    /// song alone (a seek in it, or a different next song): the incoming
    /// one waits to be loaded again from its start. Returns a loader to
    /// start when the outgoing song is already decoded to its end.
    fn unblend(&mut self) -> Option<(u64, Track, Arc<LoaderCtl>)> {
        if !self.fading.as_ref().is_some_and(Fading::handing_over) {
            return None;
        }
        let fd = self.fading.take().expect("checked");
        let mut a = fd.seg;
        a.bl_decided = true;
        let mut rechain: Option<Pending> = None;
        while let Some(s) = self.segments.pop_back() {
            s.ctl.cancel.store(true, Ordering::SeqCst);
            rechain = Some(Pending { key: s.key, tag: s.tag, track: s.track });
        }
        if self.next.is_none() {
            self.next = rechain;
        }
        self.handoff = None;
        a.ctl.prio.store(Priority::Current as u8, Ordering::SeqCst);
        let done = a.done;
        self.segments.push_front(a);
        if done { take_pending(self) } else { None }
    }

    /// Moves the front segment to `fading` and makes the next one audible.
    fn start_crossfade(&mut self, events: &mut Vec<EngineEvent>) {
        let ch = self.ch();
        let rate = self.format.sample_rate as u64;
        let Some(mut a) = self.segments.pop_front() else { return };
        let len = a.samples.len() / ch;
        let head = self.segments.front().and_then(|b| b.head_frames(rate)).unwrap_or(0);
        let (a_fade, b_fade) = match self.dsp.crossfade {
            Crossfade::Fixed { .. } => (len, len),
            _ => (
                a.tail_frames(rate).unwrap_or(0).min(len as u64) as usize,
                head.min(len as u64) as usize,
            ),
        };
        let at_ms = self.frames_to_ms(a.pos() + len as u64);
        events.push(EngineEvent::Ended {
            key: a.key,
            tag: a.tag.clone(),
            listened_ms: self.frames_to_ms(a.listened + len as u64),
            early: a.early.take().map(|error| EndedEarly { error, at_ms }),
        });
        if let Some(b) = self.segments.front_mut() {
            b.ctl.prio.store(Priority::Current as u8, Ordering::SeqCst);
            if !b.started {
                b.started = true;
                events.push(EngineEvent::Started {
                    key: b.key,
                    tag: b.tag.clone(),
                });
            }
        }
        self.fading = Some(Fading {
            seg: a,
            len,
            pos: 0,
            a_fade,
            b_fade,
            blend: None,
        });
    }

    /// Mixes the outgoing track's next frame of a blend under `o`.
    fn mix_blend(&mut self, o: &mut [f32], events: &mut Vec<EngineEvent>) {
        let ch = o.len();
        let rate = self.format.sample_rate.max(1) as u64;
        let frame_ms = |f: u64| (f * 1000 + rate / 2) / rate;
        let Some(fd) = self.fading.as_mut() else { return };
        let (ga, gb) = fd.gains();
        let pos = fd.pos;
        let len = fd.len;
        let short = fd.seg.samples.len() < ch || fd.blend.as_ref().is_none_or(|b| b.frame.len() != ch);
        if short {
            Self::hand_over(fd, frame_ms, events);
            self.drop_fading();
            return;
        }
        let Some(bl) = fd.blend.as_mut() else { return };
        let s = &fd.seg.samples;
        // Between two decoded frames when the speed isn't 1.
        let frac = bl.frac as f32;
        let lga = fd.seg.lg;
        for c in 0..ch {
            let x0 = s[c];
            let x1 = if s.len() >= 2 * ch { s[ch + c] } else { x0 };
            bl.frame[c] = (x0 + (x1 - x0) * frac) * lga;
        }
        for (v, a) in o.iter_mut().zip(&bl.frame) {
            *v = *v * gb + a * ga;
        }
        let t = pos as f64 / bl.per_ms;
        if let Some(e) = bl.echo.as_mut() {
            let (feed, mut wet) = Echo::levels(t, len as f64 / bl.per_ms, bl.plan.beat_ms);
            if bl.hurry.is_some() {
                wet *= ga;
            }
            e.frame(&bl.frame, feed, wet, o);
        }
        bl.frac += bl.plan.rate_at(t);
        while bl.frac >= 1.0 && fd.seg.samples.len() >= ch {
            fd.seg.samples.drain(..ch);
            fd.seg.consumed += 1;
            fd.seg.listened += 1;
            bl.frac -= 1.0;
        }
        fd.pos += 1;
        let hurried = bl.hurry.is_some_and(|(at, n, _, _)| fd.pos >= at + n);
        if bl.handover.is_some_and(|h| fd.pos >= h) {
            Self::hand_over(fd, frame_ms, events);
        }
        if fd.pos >= fd.len || fd.seg.samples.len() < ch || hurried {
            Self::hand_over(fd, frame_ms, events);
            self.drop_fading();
        }
    }

    /// Mixes the outgoing track's next frame under `o` (the front's frame).
    #[inline]
    fn mix_fading(&mut self, o: &mut [f32], events: &mut Vec<EngineEvent>) {
        let ch = o.len();
        if self.fading.as_ref().is_some_and(|f| f.blend.is_some()) {
            self.mix_blend(o, events);
            return;
        }
        let Some(fd) = self.fading.as_mut() else { return };
        let (ga, gb) = (fd.a_gain(), fd.b_gain());
        let lga = fd.seg.lg;
        for s in o.iter_mut() {
            let x = fd.seg.samples.pop_front().unwrap_or(0.0);
            *s = *s * gb + x * lga * ga;
        }
        fd.pos += 1;
        if fd.pos >= fd.len || fd.seg.samples.len() < ch {
            self.drop_fading();
        }
    }

    /// Takes the next frame of the front segment into `o` (leveling gain
    /// applied), retiring finished segments on the way.
    #[inline]
    fn next_frame(&mut self, o: &mut [f32], events: &mut Vec<EngineEvent>) -> Fetch {
        let ch = self.ch();
        loop {
            let Some(front) = self.segments.front_mut() else { return Fetch::Empty };
            if let Some(r) = front.replay.as_mut() {
                if r.at + ch <= r.clip.len() {
                    o.copy_from_slice(&r.clip[r.at..r.at + ch]);
                    r.at += ch;
                    front.consumed += 1;
                    front.listened += 1;
                    front.record(o);
                    front.apply_gain(o);
                    return Fetch::Got;
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
                front.apply_gain(o);
                return Fetch::Got;
            }
            if front.done {
                let mut seg = self.segments.pop_front().expect("front exists");
                seg.ctl.cancel.store(true, Ordering::SeqCst);
                if !seg.started {
                    events.push(EngineEvent::Started {
                        key: seg.key,
                        tag: seg.tag.clone(),
                    });
                }
                // The next track is now the audible one: its stream
                // gets top transfer priority.
                if let Some(next) = self.segments.front() {
                    next.ctl.prio.store(Priority::Current as u8, Ordering::SeqCst);
                }
                let listened_ms = self.frames_to_ms(seg.listened);
                let at_ms = self.frames_to_ms(seg.pos());
                events.push(EngineEvent::Ended {
                    key: seg.key,
                    tag: seg.tag,
                    listened_ms,
                    early: seg.early.take().map(|error| EndedEarly { error, at_ms }),
                });
                continue;
            }
            return Fetch::Underrun;
        }
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
        front.bl_decided = false;
        front.generation += 1;
        front.start_frame = target;
        front.consumed = 0;
        front.done = false;
        front.early = None;
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

    /// A segment's loudness data arrived: set its gain (ramped if it's
    /// already audible).
    fn set_audio(&mut self, key: u64, info: Option<TrackAudio>) {
        let ramp = self.chain.ramp_frames();
        let dsp = self.dsp;
        let ctx = self.album_ctx.clone();
        let Some(s) = self.seg_mut(key) else { return };
        s.audio = info;
        let album = ctx.is_some_and(|c| c == s.track.album_id);
        let (g, peak) = level_gain(info.as_ref(), &dsp, album);
        s.set_gain(g, peak, if s.started { ramp } else { 0 });
    }

    /// Measured silence for a segment (output frames).
    fn set_silence(&mut self, key: u64, generation: u64, head: Option<u64>, tail: Option<u64>) {
        if let Some(s) = self.seg_mut(key)
            && s.generation == generation
        {
            if head.is_some() {
                s.head = head;
            }
            if tail.is_some() {
                s.tail = tail;
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
        let mut guard = self.mix.lock();
        let m = &mut *guard;
        let ch = m.ch();
        let frames = out.len() / ch;
        let ramp_step = 1.0 / m.ms_to_frames(self.cfg.ramp_ms).max(1) as f32;
        let fade_len = m.declick_frames(&self.cfg).max(1);
        let declick = self.cfg.declick_ms > 0;
        let quiet = !m.can_clip();
        let mut filled = 0;
        let mut consumed_any = false;
        let mut underrun = false;
        let mut f = 0;
        while f < frames {
            let o = &mut out[f * ch..(f + 1) * ch];
            // Volume smoothing (~20 ms) avoids zipper noise.
            m.volume += (m.volume_target - m.volume) * 0.002;
            let from_tail = !m.tail.is_empty();
            let mut flush = false;
            if from_tail {
                for s in o.iter_mut() {
                    *s = m.tail.pop_front().unwrap_or(0.0);
                }
            } else {
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
                if let Some(plan) = m.blend_due() {
                    m.start_blend(plan);
                } else if m.crossfade_due() {
                    m.start_crossfade(&mut events);
                }
                consumed_any = true;
                // About to run dry mid-song (not at its end): fade out.
                let starving = declick
                    && m.segments.front().is_some_and(|s| {
                        !s.done && s.replay.is_none() && s.samples.len() / ch <= fade_len
                    });
                match m.next_frame(o, &mut events) {
                    Fetch::Got if declick && !m.bit_perfect => {
                        let step = 1.0 / fade_len as f32;
                        m.starve_gain = if starving {
                            (m.starve_gain - step).max(0.0)
                        } else {
                            (m.starve_gain + step).min(1.0)
                        };
                        if m.starve_gain < 1.0 {
                            let g = m.starve_gain;
                            for s in o.iter_mut() {
                                *s *= g;
                            }
                        }
                    }
                    Fetch::Got => {}
                    Fetch::Underrun => {
                        underrun = true;
                        break;
                    }
                    Fetch::Empty if m.fading.is_some() => o.fill(0.0),
                    Fetch::Empty if !m.bit_perfect && m.chain.pending() => flush = true,
                    Fetch::Empty => {
                        if m.next.is_none() && !m.drained_sent {
                            m.drained_sent = true;
                            events.push(EngineEvent::Drained);
                        }
                        break;
                    }
                }
                if !flush {
                    m.mix_fading(o, &mut events);
                    // Post-seek fade-in.
                    if m.fade_in > 0 {
                        let g = 1.0 - m.fade_in as f32 / (fade_len + 1) as f32;
                        for s in o.iter_mut() {
                            *s *= g;
                        }
                        m.fade_in -= 1;
                    }
                }
            }
            if m.bit_perfect {
                // Untouched: nothing in the chain runs.
            } else if flush {
                m.chain.flush(o);
            } else {
                m.chain.process(o, quiet);
            }
            // Pause ramp and volume, after the chain so audio held in the
            // limiter's delay ramps with everything else.
            if !from_tail {
                if m.playing {
                    m.gain = (m.gain + ramp_step).min(1.0);
                } else {
                    m.gain = (m.gain - ramp_step).max(0.0);
                }
            }
            // Bit-perfect keeps mute but not the volume level.
            let vol = if m.bit_perfect { if m.volume_target > 0.0 { 1.0 } else { 0.0 } } else { m.volume };
            let g = m.gain * vol;
            if g != 1.0 {
                for s in o.iter_mut() {
                    *s *= g;
                }
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
        drop(guard);
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
    local: Option<LocalFiles>,
    client: reqwest::Client,
    rt: Handle,
    gate: Arc<TransferGate>,
}

pub struct EngineDeps {
    pub resolver: Arc<dyn TrackResolver>,
    pub cache: Option<Arc<DiskCache>>,
    /// Downloaded songs, played before the cache and the server.
    pub local: Option<LocalFiles>,
    pub client: reqwest::Client,
    pub rt: Handle,
    pub output: Box<dyn super::output::AudioOutput>,
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
    output: GuardedOutput,
    next_key: AtomicU64,
    /// The output stream is held still while nothing plays.
    output_idle: AtomicBool,
}

impl Engine {
    pub fn new(deps: EngineDeps) -> (Self, UnboundedReceiver<EngineEvent>) {
        let (tx, rx) = unbounded_channel();
        let device_timeout = deps.config.device_timeout;
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
                starve_gain: 1.0,
                drained_sent: true,
                handoff: None,
                dsp: DspSettings::default(),
                chain: Chain::new(48_000, 2),
                fading: None,
                album_ctx: None,
                eq_max: 1.0,
                user_dsp: DspSettings::default(),
                bit_perfect: false,
                corr_bypass: false,
            }),
            cv: Condvar::new(),
            events: tx,
            cfg: deps.config,
        });
        let env = Arc::new(Env {
            core,
            resolver: deps.resolver,
            cache: deps.cache,
            local: deps.local,
            client: deps.client,
            rt: deps.rt,
            gate: deps.gate,
        });
        (
            Self {
                env,
                output: GuardedOutput::new(deps.output, device_timeout),
                next_key: AtomicU64::new(1),
                output_idle: AtomicBool::new(false),
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
            if self.output_idle.swap(false, Ordering::SeqCst) {
                self.output.quick(|o| o.set_idle(false));
            }
            return Ok(false);
        }
        self.output_idle.store(false, Ordering::SeqCst);
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
        // An open that comes back after we stopped waiting is closed again,
        // so no stream runs that the engine doesn't know about.
        let gave_up = Arc::new(AtomicBool::new(false));
        let late = gave_up.clone();
        let opened = self.output.slow("opening", move |o| {
            let r = o.open(renderer, cb).map(|fmt| (fmt, o.exclusive()));
            if late.load(Ordering::SeqCst) && r.is_ok() {
                o.close();
            }
            r
        });
        let Some(opened) = opened else {
            gave_up.store(true, Ordering::SeqCst);
            return Err(AppError::Io("The audio device isn't responding.".into()));
        };
        let (fmt, exclusive) = opened?;
        let bit_perfect = exclusive.is_on();
        let mut m = self.core().mix.lock();
        let changed = m.format != fmt;
        m.format = fmt;
        m.output_open = true;
        if changed {
            m.rescale_loops();
            m.chain.configure(fmt.sample_rate, fmt.channels as usize);
            m.refresh_eq_max(None);
        }
        let mode_changed = m.bit_perfect != bit_perfect;
        if mode_changed {
            m.bit_perfect = bit_perfect;
            m.apply_dsp(false);
        }
        drop(m);
        // The device picker shows how the device is held.
        if mode_changed || exclusive != super::output::Exclusive::Off {
            let _ = self.core().events.send(EngineEvent::OutputChanged);
        }
        Ok(changed)
    }

    /// Applies crossfade, leveling, EQ and limiter settings. Gain and EQ
    /// changes ramp; the limiter switches with a short dip.
    pub fn set_dsp(&self, s: DspSettings) {
        let mut m = self.core().mix.lock();
        m.user_dsp = s;
        m.apply_dsp(true);
    }

    /// The settings running now (see [`Engine::bit_perfect`]).
    pub fn dsp(&self) -> DspSettings {
        self.core().mix.lock().dsp
    }

    /// Headphone correction off (true) or back on, to compare.
    pub fn set_correction_bypass(&self, bypass: bool) {
        let mut m = self.core().mix.lock();
        if m.corr_bypass != bypass {
            m.corr_bypass = bypass;
            m.apply_dsp(true);
        }
    }

    pub fn correction_bypassed(&self) -> bool {
        self.core().mix.lock().corr_bypass
    }

    /// Whether samples go out untouched (an exclusive output is open).
    pub fn bit_perfect(&self) -> bool {
        self.core().mix.lock().bit_perfect
    }

    /// Devices (by id) that play bit-perfect. An open stream reopens so a
    /// change applies at once, keeping position and play state.
    pub fn set_exclusive_devices(&self, ids: Vec<String>) -> AppResult<()> {
        let rate = self.core().mix.lock().segments.front().and_then(|s| s.track.sample_rate);
        let (was, now) = self
            .output
            .slow("checking bit-perfect", move |out| {
                let was = out.exclusive_wanted();
                out.set_exclusive_devices(ids);
                let now = out.exclusive_wanted();
                out.set_preferred_rate(if now { rate } else { None });
                (was, now)
            })
            .ok_or_else(|| AppError::Io("The audio device isn't responding.".into()))?;
        let open = self.core().mix.lock().output_open;
        if was != now && open {
            self.reopen_output()?;
        }
        Ok(())
    }

    /// Bit-perfect plays each song at its own rate: before loading one at
    /// another rate, the output closes so it reopens at the new one.
    fn match_rate(&self, track: &Track) {
        let rate = track.sample_rate.filter(|r| (8_000..=768_000).contains(r));
        let wanted = self.output.slow("checking bit-perfect", move |out| {
            let wanted = out.exclusive_wanted();
            if wanted {
                out.set_preferred_rate(rate);
            }
            wanted
        });
        if wanted != Some(true) {
            return;
        }
        let reopen = {
            let m = self.core().mix.lock();
            m.output_open && rate.is_some_and(|r| r != m.format.sample_rate)
        };
        if reopen {
            self.core().mix.lock().tail.clear();
            self.close_output();
        }
    }

    /// The album playing in album order (None: anything else). Leveling's
    /// Smart mode uses album gain for its tracks, from the next track on.
    pub fn set_album_context(&self, album_id: Option<String>) {
        let mut m = self.core().mix.lock();
        if m.album_ctx != album_id {
            m.album_ctx = album_id;
            m.update_gains(false);
        }
    }

    /// Leveling gain of the audible track (linear target), for tests.
    #[cfg(test)]
    pub(crate) fn front_gain(&self) -> Option<f32> {
        self.core().mix.lock().segments.front().map(|s| s.lg_target)
    }

    /// How many segments are loaded and whether all of them are decoded to
    /// the end, so tests can wait for decoding instead of for time.
    #[cfg(test)]
    pub(crate) fn decoded(&self) -> (usize, bool) {
        let m = self.core().mix.lock();
        (m.segments.len(), m.segments.iter().all(|s| s.done))
    }

    /// Paused and the pause ramp has run out, so nothing more is rendered.
    #[cfg(test)]
    pub(crate) fn ramped_down(&self) -> bool {
        let m = self.core().mix.lock();
        !m.playing && m.gain <= 0.0
    }

    /// Releases the device (after it was lost, or on shutdown).
    pub fn close_output(&self) {
        self.output.slow("closing", |o| o.close());
        self.core().mix.lock().output_open = false;
    }

    /// The output devices, the choice and the device in use.
    pub fn output_state(&self) -> OutputState {
        let devices = self.output.devices();
        self.output
            .quick(|out| {
                let mut s = OutputState::build(devices.clone(), out.selected(), out.opened());
                s.exclusive = out.exclusive();
                s
            })
            .unwrap_or_else(|| OutputState::build(devices, None, None))
    }

    /// Chooses the output device (None follows the system default). An open
    /// stream moves there at once, keeping position and play state.
    pub fn select_output(&self, id: Option<String>) -> AppResult<()> {
        if self.output.quick(|o| o.select(id)).is_none() {
            return Err(AppError::Io("The audio device isn't responding.".into()));
        }
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
        self.match_rate(&track);
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
            m.starve_gain = 1.0;
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
            // Mid-blend, before the handover, the outgoing song is still the
            // current one. The same next song carries on; another one ends
            // the blend.
            if m.fading.as_ref().is_some_and(|f| f.handing_over() && f.seg.key == current_key) {
                let same = match (&next, m.segments.front()) {
                    (Some((t, tag)), Some(b)) => &b.tag == tag && b.track.id == t.id,
                    _ => false,
                };
                if same {
                    return m.segments.front().map(|b| b.key);
                }
                spawn = m.unblend();
            }
            if m.segments.front().map(|s| s.key) != Some(current_key) {
                drop(m);
                if let Some((key, track, ctl)) = spawn {
                    spawn_loader(self.env.clone(), key, track, 0, ctl);
                }
                return None;
            }
            // Bit-perfect: a song at another rate can't join gaplessly; it
            // loads on its own when this one ends, at its own rate.
            let rate = m.format.sample_rate;
            let next = next.filter(|(t, _)| !m.bit_perfect || t.sample_rate.is_none_or(|r| r == rate));
            match next {
                None => {
                    while m.segments.len() > 1 {
                        let s = m.segments.pop_back().expect("len > 1");
                        s.ctl.cancel.store(true, Ordering::SeqCst);
                    }
                    m.next = None;
                    m.flush_handoff();
                    spawn = None;
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
                    spawn = None;
                    let key = self.alloc_key();
                    m.next = Some(Pending { key, tag, track });
                    m.drained_sent = false;
                    result = Some(key);
                    if spawn.is_none() && m.segments.front().is_some_and(|s| s.done) {
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

    /// Holds the open output still while nothing plays, so the device
    /// callback stops waking the app. Does nothing while playing; `play`
    /// and `load` start it again.
    pub fn idle_output(&self) {
        if self.core().mix.lock().playing || self.output_idle.swap(true, Ordering::SeqCst) {
            return;
        }
        self.output.quick(|o| o.set_idle(true));
    }

    /// Whether the output is held still (or there's nothing to hold).
    pub fn output_is_idle(&self) -> bool {
        self.output_idle.load(Ordering::SeqCst)
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
        let mut respawn = None;
        let r = self.seek_locked(ms, force_reload, &mut respawn);
        // A reload chains the next song again by itself.
        if let Some((key, track, ctl)) = respawn.filter(|r| !r.2.cancel.load(Ordering::SeqCst)) {
            spawn_loader(self.env.clone(), key, track, 0, ctl);
        }
        r
    }

    fn seek_locked(&self, ms: u64, force_reload: bool, respawn: &mut Option<(u64, Track, Arc<LoaderCtl>)>) -> bool {
        {
            let mut m = self.core().mix.lock();
            // Seeking cancels a blend. Before its midpoint the seek is in the
            // outgoing song, which plays on alone.
            *respawn = m.unblend();
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
        let front = match &m.fading {
            Some(f) if f.handing_over() => &f.seg,
            _ => m.segments.front()?,
        };
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

    /// Counts `ms` as already listened on segment `key`: a play carried on
    /// after the song had to be loaded again.
    pub fn credit_listened(&self, key: u64, ms: u64) {
        let mut m = self.core().mix.lock();
        let frames = m.ms_to_frames(ms);
        if let Some(s) = m.seg_mut(key) {
            s.listened += frames;
        }
    }

    /// Key of the preloaded next segment (if it exists yet).
    pub fn next_key(&self) -> Option<u64> {
        let m = self.core().mix.lock();
        if m.fading.as_ref().is_some_and(Fading::handing_over) {
            return m.segments.front().map(|s| s.key);
        }
        m.segments
            .get(1)
            .map(|s| s.key)
            .or_else(|| m.next.as_ref().map(|p| p.key))
    }

    /// Plans the Automix blend from segment `key` into `next` (None: no
    /// blend, the crossfade setting applies). Ignored once that blend began.
    pub fn set_blend(&self, key: u64, next: u64, plan: Option<BlendPlan>) {
        let mut m = self.core().mix.lock();
        let (rate, ch) = (m.format.sample_rate, m.ch());
        let b_tag = m
            .segments
            .iter()
            .find(|s| s.key == next)
            .map(|s| s.tag.clone())
            .or_else(|| m.next.as_ref().filter(|p| p.key == next).map(|p| p.tag.clone()));
        if let Some(s) = m.seg_mut(key) {
            s.blend = plan.map(|p| (next, p));
            s.blend_kit = match (plan, b_tag) {
                (Some(p), Some(b_tag)) => Some(BlendKit::new(&p, rate, ch, s.tag.clone(), b_tag)),
                _ => None,
            };
        }
    }

    /// The blend planned out of segment `key`, if any (tests and the UI).
    pub fn blend_of(&self, key: u64) -> Option<(u64, BlendPlan)> {
        let m = self.core().mix.lock();
        m.segments.iter().find(|s| s.key == key).and_then(|s| s.blend)
    }

    /// A blend is running (before or after its midpoint).
    pub fn blending(&self) -> bool {
        self.core().mix.lock().fading.as_ref().is_some_and(|f| f.blend.is_some())
    }

    /// Next during a blend from segment `key`, before its midpoint: the
    /// incoming song takes over now at full volume, the outgoing one clears
    /// within a few ms. False when no such blend is running.
    pub fn skip_blend(&self, key: u64) -> bool {
        let mut events = Vec::new();
        {
            let mut m = self.core().mix.lock();
            let rate = m.format.sample_rate.max(1) as u64;
            let frame_ms = |f: u64| (f * 1000 + rate / 2) / rate;
            let Some(fd) = m.fading.as_mut() else { return false };
            if !fd.handing_over() || fd.seg.key != key {
                return false;
            }
            let (ga, gb) = fd.gains();
            let pos = fd.pos;
            if let Some(b) = fd.blend.as_mut() {
                let n = (HURRY_MS * b.per_ms) as usize;
                b.hurry = Some((pos, n.max(1), ga, gb));
            }
            Mix::hand_over(fd, frame_ms, &mut events);
        }
        for e in events {
            let _ = self.core().events.send(e);
        }
        true
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
        self.output.slow("closing", |o| o.close());
    }
}

/// A converter from the decoder's format to the output's.
fn converter(dec: &TrackDecoder, fmt: OutputFormat) -> AppResult<Converter> {
    Converter::new(dec.rate, dec.channels, fmt.sample_rate, fmt.channels as usize).map_err(AppError::Decode)
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
    // A downloaded copy first; its format can differ from the server's file
    // (an MP3 download), so the file name gives the hint.
    if let Some(local) = &env.local
        && let Some(path) = local.0.path(track)
        && let Ok(file) = std::fs::File::open(&path)
    {
        let hint = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|e| extension_hint(None, Some(e)))
            .or_else(|| extension_hint(track.codec.as_deref(), track.container.as_deref()));
        match TrackDecoder::open(Box::new(file), hint.as_deref()) {
            Ok(dec) => {
                return Ok(Opened {
                    dec,
                    stats: None,
                    origin: Origin::Cache,
                    cache_from: None,
                });
            }
            Err(e) => tracing::debug!("downloaded copy of {} unusable: {e}", track.id),
        }
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

/// Longest a decode thread sleeps between checks while it has nothing to
/// do (buffer full, or the file fully decoded). Seeks, cancels and the
/// output taking samples all wake it at once; this only bounds a missed
/// wake, so a paused player isn't woken 20 times a second per track.
pub(crate) const DECODER_IDLE_WAIT: Duration = Duration::from_secs(1);

/// A fully decoded segment waits here for a seek (returns its generation
/// and position) or a cancel (returns None). The flags are read under the
/// mix lock, which is where they are set, so a wake can't slip between
/// the check and the wait.
fn idle_until_seek(core: &Core, ctl: &LoaderCtl) -> Option<(u64, u64)> {
    let mut m = core.mix.lock();
    loop {
        if ctl.cancel.load(Ordering::SeqCst) {
            return None;
        }
        if let Some(seek) = ctl.seek.lock().take() {
            return Some(seek);
        }
        core.cv.wait_for(&mut m, DECODER_IDLE_WAIT);
    }
}

/// Push result for a decoded block.
enum Push {
    Ok,
    /// Seek happened (generation changed) or segment gone.
    Stale,
    Cancelled,
}

fn push_samples(env: &Arc<Env>, key: u64, generation: u64, ctl: &LoaderCtl, data: &[f32]) -> Push {
    let core = &env.core;
    let mut m = core.mix.lock();
    loop {
        if ctl.cancel.load(Ordering::SeqCst) {
            return Push::Cancelled;
        }
        // Crossfades need the next track's head decoded while the overlap
        // of the current one is still buffered.
        let overlap = m.dsp.max_overlap_ms().max(m.blend_reserve_ms());
        let cap = m.ms_to_frames(core.cfg.buffer_ahead_ms + 2 * overlap) as usize;
        let buffered = m.buffered_frames();
        let Some(seg) = m.seg_mut(key) else { return Push::Cancelled };
        if seg.generation != generation {
            return Push::Stale;
        }
        if buffered < cap || seg.samples.is_empty() {
            seg.samples.extend(data.iter().copied());
            let early = open_next_early(&mut m, key);
            drop(m);
            if let Some((k, t, ctl)) = early {
                spawn_loader(env.clone(), k, t, 0, ctl);
            }
            return Push::Ok;
        }
        core.cv.wait_for(&mut m, DECODER_IDLE_WAIT);
    }
}

/// The last segment's decoded audio reaches within [`PREOPEN_MS`] of its
/// end: the next song's loader starts now. It opens its stream, then waits
/// for this one to finish decoding before it decodes (see
/// [`wait_for_prev`]).
fn open_next_early(m: &mut Mix, key: u64) -> Option<(u64, Track, Arc<LoaderCtl>)> {
    if m.next.is_none() || m.segments.back().map(|s| s.key) != Some(key) {
        return None;
    }
    let ch = m.ch() as u64;
    let seg = m.segments.back()?;
    if seg.track.duration_ms == 0 {
        return None;
    }
    let decoded_end = seg.samples_start() + seg.samples.len() as u64 / ch.max(1);
    let end = m.ms_to_frames(seg.track.duration_ms);
    if decoded_end + m.ms_to_frames(PREOPEN_MS) < end {
        return None;
    }
    take_pending(m)
}

/// A song opened ahead waits here until the one before it is decoded to its
/// end (so a parked resampler can carry on into it). False if cancelled.
fn wait_for_prev(core: &Core, key: u64, ctl: &LoaderCtl) -> bool {
    let mut m = core.mix.lock();
    loop {
        if ctl.cancel.load(Ordering::SeqCst) {
            return false;
        }
        let Some(idx) = m.segments.iter().position(|s| s.key == key) else { return false };
        if idx == 0 || m.segments[idx - 1].done {
            return true;
        }
        core.cv.wait_for(&mut m, DECODER_IDLE_WAIT);
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
    // A next song opened ahead may be waiting for this.
    env.core.cv.notify_all();
    if let Some((k, t, ctl)) = spawn {
        spawn_loader(env.clone(), k, t, 0, ctl);
    }
}

/// Decoding stopped part way: what's buffered plays, and its end carries
/// the error. When the server is out of reach the next song isn't chained
/// on, so playback stops where the audio ran out.
fn end_early(env: &Arc<Env>, key: u64, generation: u64, error: AppError) {
    let stop_here = unreachable(&error);
    {
        let mut m = env.core.mix.lock();
        match m.seg_mut(key) {
            Some(seg) if seg.generation == generation => seg.early = Some(error),
            _ => return,
        }
        if stop_here {
            if let Some(seg) = m.seg_mut(key) {
                seg.done = true;
            }
            // A next song opened ahead doesn't follow on either.
            while m.segments.back().is_some_and(|s| s.key != key) {
                if let Some(s) = m.segments.pop_back() {
                    s.ctl.cancel.store(true, Ordering::SeqCst);
                }
            }
            m.next = None;
            m.flush_handoff();
        }
    }
    if stop_here {
        env.core.cv.notify_all();
    } else {
        mark_done(env, key, generation);
    }
}

/// Drops a segment that couldn't load and reports it. The event is sent
/// under the mix lock: once the segment is gone the output can run dry and
/// report `Drained`, and the player must see the failure first or it would
/// pick the broken entry again.
fn fail(env: &Env, key: u64, track: &Track, error: AppError) {
    {
        let mut m = env.core.mix.lock();
        let Some(idx) = m.segments.iter().position(|s| s.key == key) else { return };
        let seg = m.segments.remove(idx).expect("index valid");
        let at_ms = m.frames_to_ms(seg.start_frame);
        let _ = env.core.events.send(EngineEvent::Failed {
            key,
            tag: seg.tag,
            track_id: track.id.clone(),
            error,
            at_ms,
        });
    }
    env.core.cv.notify_all();
    #[cfg(test)]
    hold_after_fail(&env.core, &track.id);
}

/// Track ids whose failure holds the loader (after the segment is gone)
/// until the output has run dry, so tests get the worst ordering.
#[cfg(test)]
pub(crate) static HOLD_AFTER_FAIL: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[cfg(test)]
fn hold_after_fail(core: &Core, track_id: &str) {
    if !HOLD_AFTER_FAIL.lock().iter().any(|id| id == track_id) {
        return;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !core.mix.lock().drained_sent && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn run_loader(env: Arc<Env>, key: u64, track: Track, start_ms: u64, ctl: Arc<LoaderCtl>) {
    let core = env.core.clone();
    // Loudness data comes while the stream opens.
    let info_task = {
        let resolver = env.resolver.clone();
        let t = track.clone();
        env.rt.spawn(async move { resolver.audio_info(&t).await })
    };
    // 1. Open: cache → original stream → transcoder.
    let mut transcoded = false;
    let mut opened = match open_track(&env, &track, false, &ctl.prio) {
        Ok(o) => o,
        Err(OpenFail::Fatal(e)) => return fail(&env, key, &track, e),
        // Out of reach: the transcoder is on the same server.
        Err(OpenFail::Retry(e)) if unreachable(&e) => return fail(&env, key, &track, e),
        Err(OpenFail::Retry(e)) => {
            tracing::info!("track {} not playable directly ({e}); trying transcode", track.id);
            transcoded = true;
            match open_track(&env, &track, true, &ctl.prio) {
                Ok(o) => o,
                Err(OpenFail::Fatal(e) | OpenFail::Retry(e)) => return fail(&env, key, &track, e),
            }
        }
    };
    if ctl.cancel.load(Ordering::SeqCst) || !wait_for_prev(&core, key, &ctl) {
        return;
    }
    // Its gain must be known before the track is heard; without an answer
    // in time it plays with the preamp only. A song that isn't playing yet
    // doesn't wait: its level is set when the answer comes.
    let front = core.mix.lock().segments.front().is_some_and(|s| s.key == key);
    let mut late_info = None;
    let info = if front || info_task.is_finished() {
        env.rt
            .block_on(async { tokio::time::timeout(INFO_WAIT, info_task).await })
            .ok()
            .and_then(|r| r.ok())
            .flatten()
    } else {
        late_info = Some(info_task);
        None
    };
    let mut generation = 0u64;
    let mut seek_to = (start_ms > 0).then_some(start_ms);
    let mut out_fmt;
    {
        let mut m = core.mix.lock();
        out_fmt = m.format;
        m.set_audio(key, info);
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
    if let Some(task) = late_info {
        let core = core.clone();
        env.rt.spawn(async move {
            if let Ok(Ok(Some(info))) = tokio::time::timeout(LATE_INFO_WAIT, task).await {
                core.mix.lock().set_audio(key, Some(info));
            }
        });
    }
    let mut conv = match converter(&opened.dec, out_fmt) {
        Ok(c) => c,
        Err(e) => return fail(&env, key, &track, e),
    };
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
    // Quiet lead-in / run-out, for smart crossfades.
    let mut meter = SilenceMeter::new(out_fmt.sample_rate, out_fmt.channels as usize, 0);
    let mut head_known = false;

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
            let base = ms * out_fmt.sample_rate as u64 / 1000;
            meter = SilenceMeter::new(out_fmt.sample_rate, out_fmt.channels as usize, base);
            head_known = false;
            conv = match converter(&opened.dec, out_fmt) {
                Ok(c) => c,
                Err(e) => return fail(&env, key, &track, e),
            };
            if let Err(e) = opened.dec.seek_ms(ms) {
                tracing::warn!("seek failed on {}: {e}", track.id);
                // Reopen from scratch and try once more.
                match open_track(&env, &track, transcoded, &ctl.prio) {
                    Ok(o) => {
                        opened = o;
                        if opened.dec.seek_ms(ms).is_err() {
                            // Can't get there: the song ends here (not
                            // played on from its start under the new
                            // position).
                            end_early(&env, key, generation, AppError::Decode("couldn't seek in this file".into()));
                            match idle_until_seek(&core, &ctl) {
                                Some((g, ms)) => {
                                    generation = g;
                                    seek_to = Some(ms);
                                    continue 'decode;
                                }
                                None => return,
                            }
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
                match push_samples(&env, key, generation, &ctl, slice) {
                    Push::Ok => {
                        pushed_any = true;
                        meter.push(slice);
                        if !head_known
                            && let Some(h) = meter.head(dsp::SMART_OVERLAP_CAP_MS * out_fmt.sample_rate as u64 / 1000)
                        {
                            head_known = true;
                            core.mix.lock().set_silence(key, generation, Some(h), None);
                        }
                    }
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
                // The replacement is built before taking the lock the
                // output thread needs.
                let fresh = (!conv.is_passthrough()).then(|| converter(&opened.dec, out_fmt).ok()).flatten();
                let parked = fresh.is_some_and(|fresh| {
                    let mut m = core.mix.lock();
                    // A next song follows: queued, or opened ahead and
                    // waiting for this one.
                    let idx = m.segments.iter().position(|s| s.key == key);
                    let mine = idx.is_some_and(|i| m.segments[i].generation == generation);
                    let last = idx.is_some_and(|i| i + 1 == m.segments.len());
                    let ok = mine && m.handoff.is_none() && (!last || m.next.is_some());
                    if ok {
                        m.handoff = Some(Handoff {
                            prev_key: key,
                            conv: std::mem::replace(&mut conv, fresh),
                            fmt: out_fmt,
                        });
                    }
                    ok
                });
                if !parked {
                    conv.finish(&mut out);
                }
                if !out.is_empty() {
                    match push_samples(&env, key, generation, &ctl, &out) {
                        Push::Cancelled => return,
                        Push::Stale => {
                            if let Some((g, ms)) = ctl.seek.lock().take() {
                                generation = g;
                                seek_to = Some(ms);
                            }
                            continue 'decode;
                        }
                        Push::Ok => meter.push(&out),
                    }
                }
                meter.finish();
                let (head, tail) = (meter.head(0), meter.tail());
                core.mix.lock().set_silence(key, generation, head, tail);
                if meter.whole()
                    && let (Some(h), Some(t)) = (head, tail)
                {
                    let rate = out_fmt.sample_rate.max(1) as u64;
                    env.resolver.save_silence(&track.id, h * 1000 / rate, t * 1000 / rate);
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
                if let Some((g, ms)) = idle_until_seek(&core, &ctl) {
                    generation = g;
                    seek_to = Some(ms);
                    continue 'decode;
                }
                return;
            }
            Err(e) => {
                if !pushed_any && !transcoded && !matches!(e, DecodeError::Io(_)) {
                    tracing::info!("decode of {} failed ({e}); trying transcode", track.id);
                    transcoded = true;
                    match open_track(&env, &track, true, &ctl.prio) {
                        Ok(o) => {
                            opened = o;
                            conv = match converter(&opened.dec, out_fmt) {
                                Ok(c) => c,
                                Err(e) => return fail(&env, key, &track, e),
                            };
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
                // Mid-track failure: play what we have. A file that's bad
                // from here moves on to the next song; a lost connection
                // stops there, so the song isn't lost with it.
                tracing::warn!("track {} ended early: {e}", track.id);
                let err = match e {
                    DecodeError::Io(m) => AppError::Network(m),
                    other => AppError::Decode(other.to_string()),
                };
                end_early(&env, key, generation, err);
                if let Some((g, ms)) = idle_until_seek(&core, &ctl) {
                    generation = g;
                    seek_to = Some(ms);
                    continue 'decode;
                }
                return;
            }
        }
    }
}
