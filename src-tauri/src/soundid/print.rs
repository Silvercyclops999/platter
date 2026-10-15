//! A song's audio fingerprint, made on this computer from the first two
//! minutes of its file with `rusty-chromaprint` (a Rust port of Chromaprint,
//! the algorithm AcoustID uses), in the compressed, base64 form AcoustID
//! takes.

use std::io::Cursor;

use base64::Engine;
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};

use crate::audio::decode::{DecodeError, TrackDecoder};

/// How much audio goes into a fingerprint (what AcoustID's own tools use).
pub const PRINT_SECONDS: u32 = 120;
/// Less audio than this makes no useful fingerprint.
pub const MIN_SECONDS: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintError {
    /// The file couldn't be decoded (format, or too little of it read).
    Decode(String),
    /// Too short to fingerprint.
    TooShort,
}

impl std::fmt::Display for PrintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrintError::Decode(m) => write!(f, "couldn't decode: {m}"),
            PrintError::TooShort => f.write_str("too short to fingerprint"),
        }
    }
}

/// Chromaprint's default algorithm, the one AcoustID's database uses.
pub fn config() -> Configuration {
    Configuration::preset_test2()
}

fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// The raw fingerprint of up to [`PRINT_SECONDS`] of the audio in `bytes`
/// (the start of a file, or all of it), and how many seconds went in.
/// A file cut short ends where the bytes do.
pub fn raw(bytes: Vec<u8>, hint: Option<&str>) -> Result<(Vec<u32>, u32), PrintError> {
    let mut dec = TrackDecoder::open(Box::new(Cursor::new(bytes)), hint).map_err(|e| PrintError::Decode(e.to_string()))?;
    let (rate, channels) = (dec.rate, dec.channels);
    let cfg = config();
    let mut printer = Fingerprinter::new(&cfg);
    printer.start(rate, channels as u32).map_err(|e| PrintError::Decode(format!("{e:?}")))?;
    let want = PRINT_SECONDS as u64 * rate as u64 * channels as u64;
    let mut fed: u64 = 0;
    let mut buf: Vec<i16> = Vec::new();
    while fed < want {
        let block = match dec.next_block() {
            Ok(Some(b)) => b,
            Ok(None) => break,
            // The bytes end mid-packet: what was decoded is enough.
            Err(DecodeError::Io(_) | DecodeError::Corrupt(_)) if fed >= MIN_SECONDS as u64 * rate as u64 * channels as u64 => break,
            Err(e) => return Err(PrintError::Decode(e.to_string())),
        };
        let take = block.len().min((want - fed) as usize);
        let take = take - take % channels;
        buf.clear();
        buf.extend(block[..take].iter().map(|x| to_i16(*x)));
        printer.consume(&buf);
        fed += take as u64;
    }
    let secs = (fed / (rate as u64 * channels as u64)) as u32;
    if secs < MIN_SECONDS {
        return Err(PrintError::TooShort);
    }
    printer.finish();
    Ok((printer.fingerprint().to_vec(), secs))
}

/// A raw fingerprint compressed and encoded the way AcoustID takes it
/// (URL-safe base64, no padding).
pub fn encode(raw: &[u32]) -> String {
    let cfg = config();
    let bytes = FingerprintCompressor::from(&cfg).compress(raw);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// [`raw`] then [`encode`].
pub fn fingerprint(bytes: Vec<u8>, hint: Option<&str>) -> Result<String, PrintError> {
    let (fp, _) = raw(bytes, hint)?;
    if fp.is_empty() {
        return Err(PrintError::TooShort);
    }
    Ok(encode(&fp))
}

/// How many bytes to read for [`PRINT_SECONDS`] of a file: from its
/// bitrate, with room for tags and cover art ahead of the audio, else a
/// share of its size, else a cap.
pub fn bytes_wanted(bitrate_kbps: Option<u32>, duration_ms: u64, size: Option<u64>) -> u64 {
    const HEAD_ROOM: u64 = 1024 * 1024;
    const CAP: u64 = 40 * 1024 * 1024;
    let secs = PRINT_SECONDS as u64 + 5;
    let by_rate = bitrate_kbps.filter(|b| *b > 0).map(|b| b as u64 * 1000 / 8 * secs + HEAD_ROOM);
    let by_size = size.filter(|s| *s > 0).map(|s| {
        if duration_ms == 0 {
            s
        } else {
            (s as u128 * (secs * 1000) as u128 / duration_ms as u128) as u64 + HEAD_ROOM
        }
    });
    let n = by_rate.or(by_size).unwrap_or(CAP).min(CAP);
    match size {
        Some(s) if s > 0 => n.min(s),
        _ => n,
    }
}
