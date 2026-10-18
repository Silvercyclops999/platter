//! Signal path: what happens to the sound on its way from the file to the
//! output device, as a read-only snapshot (docs/IPC.md `signal_path` and
//! `audio://signal-path`).
//!
//! Nothing here runs on the output thread. The engine records the source
//! format when a decoder opens and the device format when the output opens
//! (both off the render path, in a small record the render callback never
//! touches); the controller adds what it already holds (the track, sound
//! settings, volume, the output state it last sent) and builds this.

use serde::Serialize;

use super::dsp::{self, Crossfade, DspSettings, Leveling, TrackAudio};
use super::output::{Exclusive, OutputFormat, OutputState};
use crate::model::Track;

pub const EV_SIGNAL_PATH: &str = "audio://signal-path";

/// Where the playing audio is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceFrom {
    /// Streamed from the server.
    Server,
    /// The audio cache on this computer.
    Cache,
    /// A download on this computer.
    Download,
}

/// What the engine learned when a track's decoder opened.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceInfo {
    pub from: SourceFrom,
    /// The server sent a transcode, not the file itself.
    pub transcoded: bool,
    /// ...because the file couldn't be played directly.
    pub fallback: bool,
    /// The format decoded ("flac", "mp3"), when known.
    pub codec: Option<String>,
    pub rate: u32,
    pub channels: usize,
    /// Loudness data, once known.
    pub audio: Option<TrackAudio>,
}

/// The file or stream going in.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStage {
    /// Upper case: "FLAC", "MP3".
    pub codec: Option<String>,
    pub lossless: bool,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    /// kbps, for the file itself (unknown for a transcode).
    pub bitrate: Option<u32>,
    pub transcoded: bool,
    pub from: Option<SourceFrom>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoudnessStage {
    /// Leveling is on (and not bypassed).
    pub on: bool,
    /// "smart", "track", "album" or "off".
    pub mode: &'static str,
    /// The leveling gain for this song, without the preamp.
    pub gain_db: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EqStage {
    pub eq: bool,
    pub correction: bool,
    /// The correction profile's own preamp.
    pub correction_preamp_db: Option<f32>,
    /// "Volume before the equalizer".
    pub preamp_db: f32,
    pub limiter: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResampleStage {
    pub from_rate: u32,
    pub to_rate: u32,
    pub from_channels: u32,
    pub to_channels: u32,
    /// Plain words for the converter in use.
    pub quality: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlendStage {
    /// Automix blends this song into the next.
    pub automix: bool,
    /// "smart", "fixed" or "off".
    pub crossfade: &'static str,
    pub crossfade_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputStage {
    pub device: Option<String>,
    /// "shared", "exclusive" or "fallback" (asked for exclusive, playing shared).
    pub mode: &'static str,
    /// Why it's playing shared when exclusive was asked for.
    pub reason: Option<String>,
    /// The stream's rate and channels, while it's open.
    pub rate: Option<u32>,
    pub channels: Option<u32>,
    /// Bits per sample on an exclusive stream.
    pub bits: Option<u16>,
    /// Another device on the network (casting).
    pub network: bool,
    /// Linear slider 0..1 (0 when muted).
    pub volume: f32,
    pub volume_bypassed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VerdictKind {
    Idle,
    BitPerfect,
    Lossless,
    Lossy,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub kind: VerdictKind,
    pub text: String,
}

/// "Why isn't this bit-perfect?": one plain line, and the Settings row that changes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhyNot {
    pub text: String,
    pub setting: Option<&'static str>,
}

/// `signal_path` result and `audio://signal-path` payload.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalPath {
    pub track_id: Option<String>,
    pub source: Option<SourceStage>,
    /// What the decoder hands on ("32-bit float").
    pub decode: Option<&'static str>,
    pub loudness: LoudnessStage,
    pub eq: EqStage,
    pub resample: Option<ResampleStage>,
    pub blend: BlendStage,
    pub output: OutputStage,
    pub verdict: Verdict,
    pub why: Option<WhyNot>,
}

/// What the snapshot is built from.
pub struct Inputs<'a> {
    pub track: Option<&'a Track>,
    pub source: Option<&'a SourceInfo>,
    /// The sound settings the controller runs (before bit-perfect bypasses them).
    pub dsp: DspSettings,
    /// An album is playing in album order (Smart leveling uses album gain).
    pub album_in_order: bool,
    pub output: Option<&'a OutputState>,
    /// The open stream's format (None while closed).
    pub format: Option<OutputFormat>,
    pub volume: f32,
    pub muted: bool,
    /// Automix blends into the next song.
    pub automix: bool,
}

const LOSSLESS: [&str; 10] = ["flac", "alac", "wav", "aiff", "aif", "pcm", "ape", "wv", "wavpack", "dsd"];

fn lossless(codec: Option<&str>) -> bool {
    codec.is_some_and(|c| LOSSLESS.contains(&c.to_ascii_lowercase().as_str()))
}

/// "48 kHz", "44.1 kHz".
pub fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{} kHz", rate / 1000)
    } else {
        format!("{:.1} kHz", rate as f32 / 1000.0)
    }
}

fn leveling_name(l: Leveling) -> &'static str {
    match l {
        Leveling::Smart => "smart",
        Leveling::Track => "track",
        Leveling::Album => "album",
        Leveling::Off => "off",
    }
}

/// Builds the snapshot. Pure: no locks, no I/O.
pub fn build(i: &Inputs) -> SignalPath {
    let exclusive = i.output.map(|o| o.exclusive.clone()).unwrap_or_default();
    let bit_perfect_out = exclusive.is_on();
    // Bit-perfect output bypasses every sound setting.
    let dsp = if bit_perfect_out { DspSettings::untouched() } else { i.dsp };
    let device = i.output.and_then(|o| o.current.as_ref());
    let network = device.is_some_and(|d| d.id.starts_with(super::netsink::NET_PREFIX));
    let (mode, reason, bits) = match &exclusive {
        Exclusive::Off => ("shared", None, None),
        Exclusive::On { bits, .. } => ("exclusive", None, Some(*bits)),
        Exclusive::Fallback { reason } => ("fallback", Some(reason.clone()), None),
    };
    let output = OutputStage {
        device: device.map(|d| d.name.clone()),
        mode,
        reason,
        rate: i.format.map(|f| f.sample_rate),
        channels: i.format.map(|f| f.channels as u32),
        bits,
        network,
        volume: if i.muted { 0.0 } else { i.volume.clamp(0.0, 1.0) },
        volume_bypassed: bit_perfect_out,
    };

    let mut level_off = dsp;
    level_off.preamp_db = 0.0;
    let audio = i.source.and_then(|s| s.audio);
    let gain_db = (dsp.leveling != Leveling::Off && i.track.is_some()).then(|| {
        let (g, _) = dsp::level_gain(audio.as_ref(), &level_off, i.album_in_order);
        if g == 1.0 { 0.0 } else { 20.0 * g.log10() }
    });
    let loudness = LoudnessStage { on: dsp.leveling != Leveling::Off, mode: leveling_name(dsp.leveling), gain_db };
    let eq = EqStage {
        eq: dsp.eq.is_some(),
        correction: dsp.correction.is_some(),
        correction_preamp_db: dsp.correction.map(|c| c.preamp_db),
        preamp_db: dsp.preamp_db,
        limiter: dsp.limiter,
    };
    let (crossfade, crossfade_ms) = match dsp.crossfade {
        Crossfade::Off => ("off", None),
        Crossfade::Smart => ("smart", None),
        Crossfade::Fixed { ms } => ("fixed", Some(ms)),
    };
    let blend = BlendStage { automix: i.automix && !bit_perfect_out, crossfade, crossfade_ms };

    let Some(track) = i.track else {
        return SignalPath {
            track_id: None,
            source: None,
            decode: None,
            loudness,
            eq,
            resample: None,
            blend,
            output,
            verdict: Verdict { kind: VerdictKind::Idle, text: "Nothing playing".into() },
            why: None,
        };
    };

    let s = i.source;
    let transcoded = s.is_some_and(|s| s.transcoded);
    // A transcode or a download in another format isn't the server's file.
    let codec = s.and_then(|s| s.codec.clone()).or_else(|| track.codec.clone());
    let same_file = !transcoded && codec.as_deref().map(str::to_ascii_lowercase) == track.codec.as_deref().map(str::to_ascii_lowercase);
    let is_lossless = lossless(codec.as_deref());
    let source = SourceStage {
        codec: codec.as_deref().map(str::to_ascii_uppercase),
        lossless: is_lossless,
        bit_depth: if same_file && is_lossless { track.bit_depth } else { None },
        sample_rate: s.map(|s| s.rate).or(track.sample_rate),
        channels: s.map(|s| s.channels as u32),
        bitrate: if same_file { track.bitrate } else { None },
        transcoded,
        from: s.map(|s| s.from),
    };
    let resample = match (s, i.format) {
        (Some(s), Some(f)) if s.rate != f.sample_rate || s.channels != f.channels as usize => Some(ResampleStage {
            from_rate: s.rate,
            to_rate: f.sample_rate,
            from_channels: s.channels as u32,
            to_channels: f.channels as u32,
            quality: if s.rate != f.sample_rate { "High quality (FFT)" } else { "Channel mix only" },
        }),
        _ => None,
    };

    let fallback = s.is_some_and(|s| s.fallback);
    let (verdict, why) = verdict(&source, &dsp, &output, &exclusive, resample.as_ref(), loudness.gain_db, fallback);
    SignalPath {
        track_id: Some(track.id.clone()),
        source: Some(source),
        decode: Some("32-bit float"),
        loudness,
        eq,
        resample,
        blend,
        output,
        verdict,
        why,
    }
}

fn label(src: &SourceStage) -> String {
    let codec = src.codec.clone().unwrap_or_else(|| "Unknown format".into());
    match src.bitrate {
        Some(b) if !src.lossless => format!("{codec} {b}"),
        _ => codec,
    }
}

fn verdict(
    src: &SourceStage,
    dsp: &DspSettings,
    out: &OutputStage,
    exclusive: &Exclusive,
    resample: Option<&ResampleStage>,
    level_db: Option<f32>,
    fallback: bool,
) -> (Verdict, Option<WhyNot>) {
    if !src.lossless {
        let (text, why) = if fallback {
            (
                format!("Lossy stream ({})", label(src)),
                WhyNot { text: "The file couldn't be played directly, so your server converted it.".into(), setting: None },
            )
        } else if src.transcoded {
            (
                format!("Lossy stream ({})", label(src)),
                WhyNot {
                    text: "Your server is sending a smaller MP3 instead of the file. Set streaming quality to Original.".into(),
                    setting: Some("qualityHome"),
                },
            )
        } else {
            (
                format!("Lossy source ({})", label(src)),
                WhyNot { text: "The file itself is lossy, so no setting can make it bit-perfect.".into(), setting: None },
            )
        };
        return (Verdict { kind: VerdictKind::Lossy, text }, Some(why));
    }
    let resampled_to = resample.filter(|r| r.from_rate != r.to_rate).map(|r| r.to_rate);
    if exclusive.is_on() && resample.is_none() {
        return (Verdict { kind: VerdictKind::BitPerfect, text: "Bit-perfect".into() }, None);
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(r) = resampled_to {
        parts.push(format!("resampled to {}", khz(r)));
    }
    let mut applied: Vec<&str> = Vec::new();
    if out.volume < 1.0 || level_db.is_some_and(|d| d.abs() >= 0.05) || dsp.preamp_db != 0.0 {
        applied.push("volume");
    }
    if dsp.eq.is_some() {
        applied.push("EQ");
    }
    if dsp.correction.is_some() {
        applied.push("headphone correction");
    }
    if !applied.is_empty() {
        let list = match applied.len() {
            1 => applied[0].to_string(),
            n => format!("{} and {}", applied[..n - 1].join(", "), applied[n - 1]),
        };
        parts.push(format!("{list} applied"));
    }
    if parts.is_empty() {
        parts.push("shared with other apps".into());
    }
    let text = format!("Lossless, {}", parts.join(", "));
    let why = match exclusive {
        Exclusive::Fallback { reason } => WhyNot {
            text: format!("Bit-perfect output is playing shared for now: {reason}"),
            setting: Some("bitPerfectOutputs"),
        },
        _ if out.network => WhyNot {
            text: "Music goes to another device over the network, which plays it its own way.".into(),
            setting: None,
        },
        Exclusive::On { .. } => WhyNot {
            text: "The device couldn't open at the song's own format, so it's converted.".into(),
            setting: Some("bitPerfectOutputs"),
        },
        Exclusive::Off => WhyNot {
            text: "The system mixer, volume and sound settings all touch the sound. Turn on Bit-perfect output for this device.".into(),
            setting: Some("bitPerfectOutputs"),
        },
    };
    (Verdict { kind: VerdictKind::Lossless, text }, Some(why))
}
