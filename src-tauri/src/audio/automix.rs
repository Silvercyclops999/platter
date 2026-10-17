//! Automix: DJ-style transitions between songs.
//!
//! The player decides where Automix applies (the setting, the context, the
//! per-playlist Mix switch) and asks [`plan`] how to go from one song to
//! the next with what the analysis knows about both
//! ([`crate::analysis::features::Analysis`]). A plan is either
//! beat-matched (both songs have a steady, confidently found beat within
//! about 6% of each other's tempo once octaves are allowed for: the
//! outgoing song is sped up or slowed down to the incoming one's tempo and
//! the two meet on a bar line) or a plain equal-power fade as long as the
//! crossfade setting (6 s when it's off). Its length follows how energetic
//! the songs are and the transition style setting, and never takes more
//! than 30% of the shorter song. Clashing keys get a shorter, simpler blend.
//!
//! The engine ([`super::engine`]) runs the plan: the next song takes over
//! (title, art, the play report) at the blend's midpoint.

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::analysis::features::{Analysis, Grid};
use crate::model::Track;

/// The most a blend can take of the shorter song.
pub const MAX_SHARE: f64 = 0.30;
/// The longest blend (ms). Also what the engine keeps decoded for one.
pub const MAX_BLEND_MS: f64 = 24_000.0;
/// Shortest blend worth doing (ms).
const MIN_BLEND_MS: f64 = 1_000.0;
/// Fade length when the crossfade setting is off or smart (ms).
pub const DEFAULT_FADE_MS: u64 = 6_000;
/// Tempo matching changes the outgoing song's speed by at most this much.
pub const MAX_RATE_CHANGE: f64 = 0.06;
/// Beat grids below this confidence aren't matched.
pub const BEAT_CONF: f32 = 0.5;
/// Bar starts below this confidence: songs meet on a beat instead.
const DOWNBEAT_CONF: f32 = 0.35;
/// Keys below this confidence don't count as clashing.
const KEY_CONF: f32 = 0.3;
/// Songs shorter than this aren't mixed (ms).
const MIN_SONG_MS: u64 = 30_000;

/// "Where" in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reach {
    /// Playlists with Mix on, mixes, radio and what plays after a list ends.
    #[default]
    PlaylistsAndMixes,
    /// Everything but albums.
    AllButAlbums,
}

/// "Transition style" in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Feel {
    /// Longer, gentler blends.
    #[default]
    Smooth,
    /// Shorter blends that swap on the bar.
    Energetic,
}

/// One transition's style, picked per gap on a mixed playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MixStyle {
    /// Platter picks.
    #[default]
    Auto,
    /// Straight on, no blend.
    None,
    /// Both songs cross evenly over the whole blend.
    Blend,
    /// The next song comes up first, then this one goes.
    Lift,
    /// The next song waits underneath and takes over on the last bar.
    Drop,
    /// This song echoes out while the next one comes in.
    Echo,
}

/// How the two songs' volumes move during a blend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Curve {
    Blend,
    Lift,
    Drop,
    Echo,
}

/// The Automix settings the player needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutomixOptions {
    pub on: bool,
    pub reach: Reach,
    pub feel: Feel,
    /// The crossfade setting's length (ms; 0 when it isn't a fixed fade).
    pub fade_ms: u64,
}

impl Default for AutomixOptions {
    fn default() -> Self {
        Self { on: true, reach: Reach::default(), feel: Feel::default(), fade_ms: 0 }
    }
}

/// A playlist with Mix on, and the styles picked for its gaps (keyed
/// [`gap_key`]). Kept in the sync file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MixedPlaylist {
    #[serde(default)]
    pub styles: BTreeMap<String, MixStyle>,
}

impl MixedPlaylist {
    pub fn style(&self, from: &str, to: &str) -> MixStyle {
        self.styles.get(&gap_key(from, to)).copied().unwrap_or_default()
    }
}

/// The key of the gap between two songs.
pub fn gap_key(from: &str, to: &str) -> String {
    format!("{from}>{to}")
}

/// Every mixed playlist, by id.
pub type Mixed = HashMap<String, MixedPlaylist>;

/// What the analysis knows, for the player (the database behind it).
#[async_trait]
pub trait MixInfoSource: Send + Sync {
    /// Analyses for these songs (missing: not analysed yet).
    async fn infos(&self, ids: &[String]) -> HashMap<String, Analysis>;
    /// These songs are about to be mixed: analyse them first.
    fn want(&self, _ids: &[String]) {}
}

/// Why two songs can't be mixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Unmixable {
    /// Songs of one album, one after the other in album order.
    Album,
    /// The first runs straight into the second.
    Continuous,
    /// Podcasts and audiobooks.
    Spoken,
    Short,
}

impl Unmixable {
    pub fn text(self) -> &'static str {
        match self {
            Unmixable::Album => "Songs from one album, in order",
            Unmixable::Continuous => "Runs straight into the next song",
            Unmixable::Spoken => "Spoken word isn't mixed",
            Unmixable::Short => "Too short to mix",
        }
    }
}

/// Same album, and `b` is the track right after `a` in album order.
pub fn album_in_order(a: &Track, b: &Track) -> bool {
    !a.album_id.is_empty()
        && a.album_id == b.album_id
        && ((b.disc_number == a.disc_number && b.track_number == a.track_number + 1)
            || (b.disc_number == a.disc_number + 1 && b.track_number <= 1))
}

fn spoken(t: &Track) -> bool {
    t.genres.iter().any(|g| {
        let g = g.to_lowercase();
        g.contains("podcast") || g.contains("audiobook") || g.contains("spoken")
    })
}

fn duration(t: &Track, i: Option<&Analysis>) -> u64 {
    i.map(|i| i.duration_ms).filter(|d| *d > 0).unwrap_or(t.duration_ms)
}

/// Why `a` can't blend into `b`, if it can't.
pub fn unmixable(a: &Track, ai: Option<&Analysis>, b: &Track, bi: Option<&Analysis>) -> Option<Unmixable> {
    if spoken(a) || spoken(b) {
        return Some(Unmixable::Spoken);
    }
    if album_in_order(a, b) {
        return Some(Unmixable::Album);
    }
    if !a.album_id.is_empty() && a.album_id == b.album_id && ai.is_some_and(|i| i.ends_hot) && bi.is_some_and(|i| i.starts_hot) {
        return Some(Unmixable::Continuous);
    }
    let (da, db) = (duration(a, ai), duration(b, bi));
    if (da > 0 && da < MIN_SONG_MS) || (db > 0 && db < MIN_SONG_MS) {
        return Some(Unmixable::Short);
    }
    None
}

/// Place on the circle of fifths (keys 0 to 11 major, 12 to 23 minor);
/// a minor key sits with its relative major.
fn fifths(key: u8) -> i32 {
    let tonic = if key < 12 { key as i32 } else { (key as i32 - 12 + 3) % 12 };
    (tonic * 7) % 12
}

/// Keys that sound right together: the same, the relative major or minor,
/// or a fifth apart in the same mode.
pub fn keys_fit(a: u8, b: u8) -> bool {
    let (fa, fb) = (fifths(a), fifths(b));
    let d = (fa - fb).rem_euclid(12).min((fb - fa).rem_euclid(12));
    if (a < 12) == (b < 12) { d <= 1 } else { d == 0 }
}

/// Whether two songs' keys are known well enough and clash.
pub fn keys_clash(ai: Option<&Analysis>, bi: Option<&Analysis>) -> bool {
    match (ai, bi) {
        (Some(a), Some(b)) => match (a.key, b.key) {
            (Some(ka), Some(kb)) if a.key_conf >= KEY_CONF && b.key_conf >= KEY_CONF => !keys_fit(ka, kb),
            _ => false,
        },
        _ => false,
    }
}

/// How well `b` follows `a`, 0 to 1: tempo, key and energy. Unknowns
/// count as middling.
pub fn fit(ai: Option<&Analysis>, bi: Option<&Analysis>) -> f32 {
    let (Some(a), Some(b)) = (ai, bi) else { return 0.5 };
    let tempo = match (a.bpm, b.bpm) {
        (Some(x), Some(y)) if a.beat_conf >= BEAT_CONF && b.beat_conf >= BEAT_CONF => {
            let r = [0.5f32, 1.0, 2.0].iter().map(|m| (x / (y * m) - 1.0).abs()).fold(f32::MAX, f32::min);
            (1.0 - r / 0.12).max(0.0)
        }
        _ => 0.5,
    };
    let key = match (a.key, b.key) {
        (Some(x), Some(y)) if a.key_conf >= KEY_CONF && b.key_conf >= KEY_CONF => {
            if keys_fit(x, y) {
                1.0
            } else {
                0.0
            }
        }
        _ => 0.5,
    };
    let energy = 1.0 - (a.energy - b.energy).abs();
    0.45 * tempo + 0.35 * key + 0.2 * energy
}

/// How the engine runs one blend. Times in ms: `a_start_ms` in the
/// outgoing song, `b_start_ms` in the incoming one, the rest in output
/// time from the blend's start.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlendPlan {
    /// Where the blend starts in the outgoing song. None: when what's left
    /// of it is `len_ms` long.
    pub a_start_ms: Option<f64>,
    pub len_ms: f64,
    /// Where the incoming song starts (its lead-in is skipped).
    pub b_start_ms: f64,
    /// The outgoing song's speed once the tempo is matched...
    pub rate: f64,
    /// ...reached this long into the blend (from 1 at its start).
    pub ramp_ms: f64,
    pub curve: Curve,
    /// One beat during the blend, for the curve's timing.
    pub beat_ms: f64,
    /// The songs meet on the beat.
    pub beatmatched: bool,
}

impl BlendPlan {
    /// The outgoing song's speed `u` ms into the blend.
    pub fn rate_at(&self, u: f64) -> f64 {
        if self.ramp_ms <= 0.0 || u >= self.ramp_ms {
            self.rate
        } else {
            1.0 + (self.rate - 1.0) * u / self.ramp_ms
        }
    }

    /// How far the outgoing song has played `u` ms into the blend (ms).
    pub fn a_advance(&self, u: f64) -> f64 {
        a_advance(self.rate, self.ramp_ms, u)
    }

    /// The handover: the next song becomes the one playing.
    pub fn midpoint_ms(&self) -> f64 {
        self.len_ms / 2.0
    }
}

fn a_advance(rate: f64, ramp: f64, u: f64) -> f64 {
    if ramp <= 0.0 {
        return rate * u;
    }
    if u <= ramp {
        u + (rate - 1.0) * u * u / (2.0 * ramp)
    } else {
        ramp * (1.0 + rate) / 2.0 + rate * (u - ramp)
    }
}

/// Volumes `t` ms into a blend of `len` ms (outgoing, incoming), and how
/// much of the outgoing song feeds the echo.
pub fn gains(curve: Curve, t: f64, len: f64, beat: f64) -> (f32, f32) {
    let len = len.max(1.0);
    let t = t.clamp(0.0, len);
    let up = |x: f64| ((x.clamp(0.0, 1.0)) * std::f64::consts::FRAC_PI_2).sin() as f32;
    let down = |x: f64| ((x.clamp(0.0, 1.0)) * std::f64::consts::FRAC_PI_2).cos() as f32;
    let beat = beat.clamp(50.0, len / 2.0);
    match curve {
        Curve::Blend => (down(t / len), up(t / len)),
        Curve::Lift => {
            let h = len / 2.0;
            (down((t - h) / h), up(t / h))
        }
        Curve::Drop => {
            let cut = len - beat;
            if t < cut {
                (1.0, 0.4 * up(t / cut))
            } else {
                let x = (t - cut) / beat;
                (down(x), 0.4 + 0.6 * up(x))
            }
        }
        Curve::Echo => {
            let h = len / 2.0;
            let stop = (h - beat).max(0.0);
            (down((t - stop) / beat), up(t / len))
        }
    }
}

/// The echo on an outgoing song (`Curve::Echo`): what's fed in during the
/// beat before the dry sound stops repeats every three quarters of a beat,
/// fading, until the blend ends.
pub struct Echo {
    buf: Vec<f32>,
    at: usize,
    ch: usize,
}

impl Echo {
    pub const FEEDBACK: f32 = 0.5;

    pub fn new(delay_frames: usize, ch: usize) -> Self {
        Self { buf: vec![0.0; delay_frames.max(1) * ch.max(1)], at: 0, ch: ch.max(1) }
    }

    /// One frame: `input` goes in (scaled by `feed`), the echo is added to
    /// `out` (scaled by `wet`).
    pub fn frame(&mut self, input: &[f32], feed: f32, wet: f32, out: &mut [f32]) {
        let ch = self.ch;
        for c in 0..ch.min(out.len()).min(input.len()) {
            let i = self.at * ch + c;
            let delayed = self.buf[i];
            out[c] += delayed * wet;
            self.buf[i] = input[c] * feed + delayed * Self::FEEDBACK;
        }
        self.at = (self.at + 1) % (self.buf.len() / ch);
    }

    /// Feed and wet levels `t` ms into the blend.
    pub fn levels(t: f64, len: f64, beat: f64) -> (f32, f32) {
        let h = len / 2.0;
        let beat = beat.clamp(50.0, h.max(50.0));
        let feed = if t >= h - 2.0 * beat && t < h { 1.0 } else { 0.0 };
        let wet = if t < h { 1.0 } else { (((t - h) / h.max(1.0)).clamp(0.0, 1.0) * std::f64::consts::FRAC_PI_2).cos() as f32 };
        (feed, wet)
    }
}

/// What a plan is made from.
#[derive(Debug, Clone, Copy)]
pub struct PlanOptions {
    pub feel: Feel,
    /// The crossfade setting's length (0: use [`DEFAULT_FADE_MS`]).
    pub fade_ms: u64,
    pub style: MixStyle,
}

fn curve_for(style: MixStyle, auto: Curve) -> Curve {
    match style {
        MixStyle::Blend => Curve::Blend,
        MixStyle::Lift => Curve::Lift,
        MixStyle::Drop => Curve::Drop,
        MixStyle::Echo => Curve::Echo,
        MixStyle::Auto | MixStyle::None => auto,
    }
}

/// How to go from `a` into `b`. None: no blend (an unmixable pair, the "No
/// transition" style, or songs too short for one).
pub fn plan(a: &Track, ai: Option<&Analysis>, b: &Track, bi: Option<&Analysis>, o: &PlanOptions) -> Option<BlendPlan> {
    if o.style == MixStyle::None || unmixable(a, ai, b, bi).is_some() {
        return None;
    }
    let (da, db) = (duration(a, ai) as f64, duration(b, bi) as f64);
    let short = if da > 0.0 && db > 0.0 { da.min(db) } else { da.max(db) };
    let cap = if short > 0.0 { (MAX_SHARE * short).min(MAX_BLEND_MS) } else { MAX_BLEND_MS };
    if cap < MIN_BLEND_MS {
        return None;
    }
    let energy = match (ai, bi) {
        (Some(a), Some(b)) => (a.energy + b.energy) / 2.0,
        (Some(x), None) | (None, Some(x)) => x.energy,
        _ => 0.5,
    };
    let clash = keys_clash(ai, bi);
    if let (Some(ai), Some(bi)) = (ai, bi)
        && let Some(p) = beatmatch(ai, bi, energy, clash, cap, o)
    {
        return Some(p);
    }
    // A plain equal-power fade.
    let base = if o.fade_ms > 0 { o.fade_ms } else { DEFAULT_FADE_MS } as f64;
    let mut len = match o.feel {
        Feel::Smooth => base * (1.3 - 0.6 * energy as f64),
        Feel::Energetic => base * 0.6,
    };
    if clash {
        len *= 0.5;
    }
    let len = len.clamp(MIN_BLEND_MS, cap);
    let a_start_ms = ai.filter(|i| i.cue_out_ms > 0).map(|i| i.cue_out_ms as f64 - len).filter(|s| *s > 0.0);
    Some(BlendPlan {
        a_start_ms,
        len_ms: len,
        b_start_ms: bi.map_or(0.0, |i| i.cue_in_ms as f64),
        rate: 1.0,
        ramp_ms: 0.0,
        curve: curve_for(o.style, Curve::Blend),
        beat_ms: 500.0,
        beatmatched: false,
    })
}

fn beatmatch(ai: &Analysis, bi: &Analysis, energy: f32, clash: bool, cap: f64, o: &PlanOptions) -> Option<BlendPlan> {
    let (ga, gb): (Grid, Grid) = (ai.outro?, bi.intro?);
    if ga.conf < BEAT_CONF || gb.conf < BEAT_CONF || ai.cue_out_ms == 0 {
        return None;
    }
    // The incoming song's beat, doubled or halved to sit nearest the
    // outgoing one's.
    let (out_beat, rate) = [0.5, 1.0, 2.0]
        .iter()
        .map(|m| {
            let ob = gb.beat_ms * m;
            (ob, ga.beat_ms / ob)
        })
        .min_by(|x, y| (x.1 - 1.0).abs().total_cmp(&(y.1 - 1.0).abs()))?;
    if (rate - 1.0).abs() > MAX_RATE_CHANGE {
        return None;
    }
    let bars = ga.downbeat_conf >= DOWNBEAT_CONF && gb.downbeat_conf >= DOWNBEAT_CONF;
    let bar_out = 4.0 * out_beat;
    let mut n = match (o.feel, energy < 0.45) {
        (Feel::Smooth, true) => 16.0,
        (Feel::Smooth, false) | (Feel::Energetic, true) => 8.0,
        (Feel::Energetic, false) => 4.0,
    };
    if clash {
        n = f64::min(n, 2.0);
    }
    while n * bar_out > cap && n > 1.0 {
        n /= 2.0;
    }
    let len = n * bar_out;
    if len > cap || len < MIN_BLEND_MS {
        return None;
    }
    // Where the incoming song starts, and its first bar (or beat) after that.
    let b_start = bi.cue_in_ms as f64;
    let first = if bars {
        gb.bar_at_or_after(b_start)
    } else {
        let k = ((b_start - gb.downbeat_ms) / gb.beat_ms).ceil();
        gb.downbeat_ms + k * gb.beat_ms
    };
    let u_b = first - b_start;
    if u_b >= len / 2.0 {
        return None;
    }
    // Speed reaches the new tempo as the incoming song's first bar plays.
    let ramp = u_b.max(0.0);
    let at_b = a_advance(rate, ramp, u_b);
    // The outgoing song's bar (or beat) that meets it: the latest one that
    // still leaves the whole blend inside its music.
    let ideal = ai.cue_out_ms as f64 - a_advance(rate, ramp, len) + at_b;
    let meet = if bars {
        ga.bar_at_or_before(ideal)
    } else {
        let k = ((ideal - ga.downbeat_ms) / ga.beat_ms).floor();
        ga.downbeat_ms + k * ga.beat_ms
    };
    let a_start = meet - at_b;
    if a_start <= 0.0 {
        return None;
    }
    let auto = if clash {
        Curve::Blend
    } else {
        match o.feel {
            Feel::Smooth => Curve::Lift,
            Feel::Energetic if bi.energy + 0.05 >= ai.energy => Curve::Drop,
            Feel::Energetic => Curve::Lift,
        }
    };
    Some(BlendPlan {
        a_start_ms: Some(a_start),
        len_ms: len,
        b_start_ms: b_start,
        rate,
        ramp_ms: ramp,
        curve: curve_for(o.style, auto),
        beat_ms: out_beat,
        beatmatched: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, album: &str, n: u32, secs: u64) -> Track {
        let mut t = crate::audio::testutil::track(id, "artist", "/p");
        t.album_id = album.into();
        t.track_number = n;
        t.duration_ms = secs * 1000;
        t
    }

    fn info(bpm: f64, secs: u64, key: Option<u8>, energy: f32) -> Analysis {
        let beat = 60_000.0 / bpm;
        let dur = secs * 1000;
        let grid = |from: f64| Grid { downbeat_ms: from, beat_ms: beat, conf: 0.9, downbeat_conf: 0.8 };
        Analysis {
            duration_ms: dur,
            bpm: Some(bpm as f32),
            beat_conf: 0.9,
            intro: Some(grid(250.0)),
            outro: Some(grid(250.0 + 4.0 * beat * ((dur as f64 - 60_000.0) / (4.0 * beat)).floor())),
            key,
            key_conf: 0.8,
            energy,
            loudness_db: -12.0,
            cue_in_ms: 100,
            cue_out_ms: dur - 500,
            intro_end_ms: 8_000,
            outro_start_ms: dur - 10_000,
            starts_hot: false,
            ends_hot: false,
            sections: Vec::new(),
            features: Vec::new(),
        }
    }

    fn opts() -> PlanOptions {
        PlanOptions { feel: Feel::Smooth, fade_ms: 0, style: MixStyle::Auto }
    }

    #[test]
    fn beatmatched_blends_meet_on_a_bar_and_match_the_tempo() {
        let (a, b) = (track("a", "x", 1, 240), track("b", "y", 1, 200));
        let (ai, bi) = (info(126.0, 240, Some(21), 0.6), info(124.0, 200, Some(0), 0.6));
        let p = plan(&a, Some(&ai), &b, Some(&bi), &opts()).unwrap();
        assert!(p.beatmatched);
        assert!((p.rate - 124.0 / 126.0).abs() < 1e-9, "{}", p.rate);
        assert!((p.rate - 1.0).abs() <= MAX_RATE_CHANGE);
        // Where the incoming song's first bar plays, the outgoing song is on one of its bars.
        let gb = bi.intro.unwrap();
        let u_b = gb.bar_at_or_after(p.b_start_ms) - p.b_start_ms;
        let a_pos = p.a_start_ms.unwrap() + p.a_advance(u_b);
        let ga = ai.outro.unwrap();
        let off = (a_pos - ga.downbeat_ms).rem_euclid(4.0 * ga.beat_ms);
        assert!(off.min(4.0 * ga.beat_ms - off) < 1e-6, "off the bar by {off} ms");
        // After the ramp both beats are the same length.
        assert!((ga.beat_ms / p.rate - 60_000.0 / 124.0).abs() < 1e-6);
        // The blend stays inside the outgoing song's music, a whole number of bars.
        assert!(p.a_start_ms.unwrap() + p.a_advance(p.len_ms) <= ai.cue_out_ms as f64 + 1e-6);
        let bars = p.len_ms / (4.0 * p.beat_ms);
        assert!((bars - bars.round()).abs() < 1e-9 && bars >= 1.0, "{bars}");
        assert_eq!(p.curve, Curve::Lift, "keys fit: A minor into C major");
    }

    #[test]
    fn no_more_than_thirty_percent_of_the_shorter_song() {
        let (a, b) = (track("a", "x", 1, 300), track("b", "y", 1, 40));
        for (ai, bi) in [(None, None), (Some(info(100.0, 300, None, 0.2)), Some(info(100.0, 40, None, 0.2)))] {
            let p = plan(&a, ai.as_ref(), &b, bi.as_ref(), &PlanOptions { fade_ms: 12_000, ..opts() }).unwrap();
            assert!(p.len_ms <= 0.3 * 40_000.0 + 1e-6, "{p:?}");
        }
        // Long songs: never more than the longest blend.
        let (a, b) = (track("a", "x", 1, 900), track("b", "y", 1, 900));
        let (ai, bi) = (info(70.0, 900, None, 0.1), info(70.0, 900, None, 0.1));
        let p = plan(&a, Some(&ai), &b, Some(&bi), &opts()).unwrap();
        assert!(p.len_ms <= MAX_BLEND_MS, "{}", p.len_ms);
    }

    #[test]
    fn unsure_beats_or_far_tempos_fall_back_to_a_fade() {
        let (a, b) = (track("a", "x", 1, 240), track("b", "y", 1, 240));
        let mut ai = info(120.0, 240, None, 0.5);
        let bi = info(121.0, 240, None, 0.5);
        ai.outro.as_mut().unwrap().conf = 0.2;
        let p = plan(&a, Some(&ai), &b, Some(&bi), &opts()).unwrap();
        assert!(!p.beatmatched && p.rate == 1.0 && p.curve == Curve::Blend);
        // Fade length follows the crossfade setting, ending where the music does.
        let p = plan(&a, Some(&ai), &b, Some(&bi), &PlanOptions { fade_ms: 8_000, ..opts() }).unwrap();
        assert!((p.len_ms - 8_000.0).abs() < 1.0, "{}", p.len_ms);
        assert_eq!(p.a_start_ms, Some(ai.cue_out_ms as f64 - p.len_ms));
        // 140 against 100 BPM is too far apart to match.
        let (ai, bi) = (info(140.0, 240, None, 0.5), info(100.0, 240, None, 0.5));
        assert!(!plan(&a, Some(&ai), &b, Some(&bi), &opts()).unwrap().beatmatched);
        // Half or double time matches.
        let (ai, bi) = (info(140.0, 240, None, 0.5), info(71.0, 240, None, 0.5));
        assert!(plan(&a, Some(&ai), &b, Some(&bi), &opts()).unwrap().beatmatched);
        // Nothing known: a fade from the end of the file.
        let p = plan(&a, None, &b, None, &opts()).unwrap();
        assert!(p.a_start_ms.is_none() && (p.len_ms - DEFAULT_FADE_MS as f64 * 1.0).abs() < 1.0);
    }

    #[test]
    fn clashing_keys_get_a_short_simple_blend() {
        let (a, b) = (track("a", "x", 1, 240), track("b", "y", 1, 240));
        let (ai, bi) = (info(120.0, 240, Some(0), 0.3), info(120.0, 240, Some(6), 0.3));
        assert!(keys_clash(Some(&ai), Some(&bi)));
        let p = plan(&a, Some(&ai), &b, Some(&bi), &opts()).unwrap();
        assert!(p.len_ms <= 2.0 * 4.0 * 500.0 + 1e-6 && p.curve == Curve::Blend, "{p:?}");
        let fit_ok = plan(&a, Some(&ai), &b, Some(&info(120.0, 240, Some(7), 0.3)), &opts()).unwrap();
        assert!(fit_ok.len_ms > p.len_ms);
    }

    #[test]
    fn never_within_an_album_in_order_spoken_word_or_runs_on() {
        let (a, b) = (track("a", "al", 3, 200), track("b", "al", 4, 200));
        assert_eq!(unmixable(&a, None, &b, None), Some(Unmixable::Album));
        assert!(plan(&a, None, &b, None, &opts()).is_none());
        // Next disc, first track.
        let mut c = track("c", "al", 1, 200);
        c.disc_number = 2;
        let mut last = track("l", "al", 12, 200);
        last.disc_number = 1;
        assert_eq!(unmixable(&last, None, &c, None), Some(Unmixable::Album));
        // Same album out of order is fine, unless the songs run into each other.
        let d = track("d", "al", 7, 200);
        assert_eq!(unmixable(&a, None, &d, None), None);
        let (mut ai, mut di) = (info(120.0, 200, None, 0.5), info(120.0, 200, None, 0.5));
        ai.ends_hot = true;
        di.starts_hot = true;
        assert_eq!(unmixable(&a, Some(&ai), &d, Some(&di)), Some(Unmixable::Continuous));
        let mut p = track("p", "pod", 1, 2000);
        p.genres = vec!["Podcast".into()];
        assert_eq!(unmixable(&d, None, &p, None), Some(Unmixable::Spoken));
        assert_eq!(unmixable(&d, None, &track("s", "z", 1, 20), None), Some(Unmixable::Short));
        // "No transition" picked for the gap.
        let o = PlanOptions { style: MixStyle::None, ..opts() };
        assert!(plan(&a, None, &d, None, &o).is_none());
    }

    #[test]
    fn energetic_is_shorter_and_drops() {
        let (a, b) = (track("a", "x", 1, 240), track("b", "y", 1, 240));
        let (ai, bi) = (info(160.0, 240, None, 0.8), info(160.0, 240, None, 0.85));
        let smooth = plan(&a, Some(&ai), &b, Some(&bi), &opts()).unwrap();
        let e = plan(&a, Some(&ai), &b, Some(&bi), &PlanOptions { feel: Feel::Energetic, ..opts() }).unwrap();
        assert!(e.len_ms < smooth.len_ms);
        assert_eq!(e.curve, Curve::Drop);
        // A picked style wins over Auto.
        let echo = plan(&a, Some(&ai), &b, Some(&bi), &PlanOptions { style: MixStyle::Echo, ..opts() }).unwrap();
        assert_eq!(echo.curve, Curve::Echo);
        // Calmer songs blend for longer.
        let (ci, di) = (info(160.0, 240, None, 0.2), info(160.0, 240, None, 0.2));
        assert!(plan(&a, Some(&ci), &b, Some(&di), &opts()).unwrap().len_ms > smooth.len_ms);
    }

    #[test]
    fn curves_start_on_the_outgoing_song_and_end_on_the_incoming_one() {
        for c in [Curve::Blend, Curve::Lift, Curve::Drop, Curve::Echo] {
            let (a0, b0) = gains(c, 0.0, 8000.0, 500.0);
            let (a1, b1) = gains(c, 8000.0, 8000.0, 500.0);
            assert!(a0 > 0.99 && b0 < 0.01, "{c:?} start");
            assert!(a1 < 0.01 && b1 > 0.99, "{c:?} end");
        }
        // Equal power in the middle of a plain blend.
        let (a, b) = gains(Curve::Blend, 4000.0, 8000.0, 500.0);
        assert!(((a * a + b * b) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn keys_fit_on_the_circle_of_fifths() {
        assert!(keys_fit(0, 0) && keys_fit(0, 7) && keys_fit(0, 5), "C, G, F major");
        assert!(keys_fit(0, 21), "C major and A minor");
        assert!(keys_fit(21, 16), "A minor and E minor");
        assert!(!keys_fit(0, 6), "C and F sharp");
        assert!(!keys_fit(0, 12 + 9 + 2), "C major and B minor");
    }

    #[test]
    fn fit_prefers_close_tempo_and_friendly_keys() {
        let a = info(120.0, 200, Some(0), 0.5);
        let near = info(121.0, 200, Some(7), 0.5);
        let far = info(90.0, 200, Some(6), 0.9);
        assert!(fit(Some(&a), Some(&near)) > fit(Some(&a), Some(&far)));
        assert_eq!(fit(None, Some(&a)), 0.5);
    }

    #[test]
    fn the_echo_repeats_and_fades() {
        let mut e = Echo::new(4, 1);
        let mut outs = Vec::new();
        for i in 0..16 {
            let mut o = [0.0f32];
            e.frame(&[if i == 0 { 1.0 } else { 0.0 }], 1.0, 1.0, &mut o);
            outs.push(o[0]);
        }
        assert_eq!(outs[4], 1.0);
        assert_eq!(outs[8], 0.5);
        assert_eq!(outs[12], 0.25);
        let (feed, wet) = Echo::levels(7000.0, 8000.0, 500.0);
        assert_eq!(feed, 0.0);
        assert!(wet < 0.5);
    }
}
