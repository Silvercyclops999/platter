//! symphonia probe + decode to interleaved f32, with sample-accurate seek.
//! Encoder delay/padding are trimmed by symphonia's gapless mode (on by
//! default in 0.6), so the decoded length is exactly the playable length.

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase, Timestamp};

/// Why a track couldn't be decoded.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    /// Format or codec not supported (→ try the transcoder).
    Unsupported(String),
    /// I/O failed underneath (network gone, file missing).
    Io(String),
    /// Data is corrupt beyond recovery.
    Corrupt(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Unsupported(m) => write!(f, "unsupported: {m}"),
            DecodeError::Io(m) => write!(f, "io: {m}"),
            DecodeError::Corrupt(m) => write!(f, "corrupt: {m}"),
        }
    }
}

fn map_err(e: SymError) -> DecodeError {
    match e {
        SymError::IoError(e) => DecodeError::Io(e.to_string()),
        SymError::Unsupported(m) => DecodeError::Unsupported(m.to_string()),
        other => DecodeError::Corrupt(other.to_string()),
    }
}

/// Maps a Plex codec/container name to a probe extension hint.
pub fn extension_hint(codec: Option<&str>, container: Option<&str>) -> Option<String> {
    let pick = |s: &str| -> Option<&'static str> {
        Some(match s.to_ascii_lowercase().as_str() {
            "flac" => "flac",
            "mp3" | "mpeg" => "mp3",
            "aac" | "alac" | "mp4" | "m4a" | "mov" => "m4a",
            "ogg" | "vorbis" | "opus" | "oga" => "ogg",
            "wav" | "pcm" | "wave" => "wav",
            "aiff" | "aif" => "aiff",
            "mka" | "mkv" | "webm" => "mkv",
            "caf" => "caf",
            _ => return None,
        })
    };
    container
        .and_then(pick)
        .or_else(|| codec.and_then(pick))
        .map(str::to_string)
}

pub struct TrackDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    pub rate: u32,
    pub channels: usize,
    /// Playable frames, if the container says.
    pub total_frames: Option<u64>,
    buf: Vec<f32>,
    /// After a seek: drop decoded frames before this timestamp.
    skip_until: Option<Timestamp>,
    errors_in_row: u32,
    /// The first decoded block, read at open to learn the real format.
    primed: Option<Vec<f32>>,
    /// The stream ended while priming.
    primed_end: bool,
}

impl TrackDecoder {
    pub fn open(src: Box<dyn MediaSource>, hint: Option<&str>) -> Result<Self, DecodeError> {
        let mss = MediaSourceStream::new(src, Default::default());
        let mut h = Hint::new();
        if let Some(ext) = hint {
            h.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(&h, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(map_err)?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| DecodeError::Unsupported("no audio track".into()))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| DecodeError::Unsupported("no audio codec parameters".into()))?
            .clone();
        let track_id = track.id;
        let time_base = track.time_base;
        let total_frames = track.num_frames;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(map_err)?;
        let rate = params.sample_rate.unwrap_or(0);
        let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(0);
        if rate == 0 || channels == 0 {
            return Err(DecodeError::Unsupported(
                "stream does not declare sample rate/channels".into(),
            ));
        }
        let mut d = Self {
            format,
            decoder,
            track_id,
            time_base,
            rate,
            channels,
            total_frames,
            buf: Vec::new(),
            skip_until: None,
            errors_in_row: 0,
            primed: None,
            primed_end: false,
        };
        d.prime()?;
        Ok(d)
    }

    /// Decodes the first block now: the container's rate and channel count
    /// can be wrong (an AAC file whose header gives half the real rate), and
    /// everything downstream is set up from these.
    fn prime(&mut self) -> Result<(), DecodeError> {
        let declared = self.rate;
        match self.decode_next(true)? {
            true => self.primed = Some(std::mem::take(&mut self.buf)),
            false => self.primed_end = true,
        }
        if self.rate != declared
            && let Some(n) = self.total_frames.as_mut()
        {
            *n = *n * self.rate as u64 / declared.max(1) as u64;
        }
        Ok(())
    }

    /// Next block of interleaved f32 at the source rate; `None` at the end.
    pub fn next_block(&mut self) -> Result<Option<&[f32]>, DecodeError> {
        if let Some(b) = self.primed.take() {
            self.buf = b;
            return Ok(Some(&self.buf));
        }
        if std::mem::take(&mut self.primed_end) {
            return Ok(None);
        }
        Ok(self.decode_next(false)?.then_some(&self.buf[..]))
    }

    /// Decodes the next block into `buf`; false at the end. `first` takes
    /// the format from the decoded audio.
    fn decode_next(&mut self, first: bool) -> Result<bool, DecodeError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                Err(SymError::ResetRequired) => return Ok(false),
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                Err(e) => return Err(map_err(e)),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let pts = packet.pts;
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    self.errors_in_row = 0;
                    let frames = decoded.frames();
                    if frames == 0 {
                        continue;
                    }
                    let ch = decoded.spec().channels().count();
                    if ch == 0 {
                        continue;
                    }
                    if first && decoded.spec().rate() > 0 {
                        self.rate = decoded.spec().rate();
                        self.channels = ch;
                    }
                    decoded.copy_to_vec_interleaved(&mut self.buf);
                    if ch != self.channels {
                        // Channel layout changed mid-stream: remix to the declared one.
                        let mut tmp = Vec::new();
                        super::convert::mix_channels(&self.buf, ch, self.channels, &mut tmp);
                        self.buf = tmp;
                    }
                    if let Some(until) = self.skip_until {
                        let drop_frames = self.frames_between(pts, until);
                        if drop_frames >= frames as u64 {
                            continue;
                        }
                        self.skip_until = None;
                        let n = (drop_frames as usize * self.channels).min(self.buf.len());
                        self.buf.drain(..n);
                    }
                    return Ok(true);
                }
                Err(SymError::DecodeError(m)) => {
                    // A bad packet: skip it, unless the stream is hopeless.
                    self.errors_in_row += 1;
                    if self.errors_in_row > 32 {
                        return Err(DecodeError::Corrupt(m.to_string()));
                    }
                }
                Err(SymError::IoError(e)) => return Err(DecodeError::Io(e.to_string())),
                Err(e) => return Err(map_err(e)),
            }
        }
    }

    /// Frames from `from` up to `to` (0 if `to` <= `from`).
    fn frames_between(&self, from: Timestamp, to: Timestamp) -> u64 {
        let d = to.get() - from.get();
        if d <= 0 {
            return 0;
        }
        match self.time_base {
            Some(tb) => {
                let num = d as u128 * tb.numer.get() as u128 * self.rate as u128;
                (num / tb.denom.get() as u128) as u64
            }
            None => d as u64,
        }
    }

    /// Seeks so the next decoded sample is at `ms` (sample-accurate).
    /// Returns the frame index (at the source rate) playback resumes from.
    pub fn seek_ms(&mut self, ms: u64) -> Result<u64, DecodeError> {
        let time = Time::try_from_nanos_u128(ms as u128 * 1_000_000)
            .ok_or_else(|| DecodeError::Corrupt("bad seek time".into()))?;
        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(map_err)?;
        self.decoder.reset();
        self.primed = None;
        self.primed_end = false;
        self.skip_until = (seeked.required_ts > seeked.actual_ts).then_some(seeked.required_ts);
        // Frame index we'll resume at.
        let frame = (ms as u128 * self.rate as u128 / 1000) as u64;
        Ok(frame)
    }
}
