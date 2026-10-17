//! A song's audio for the recognizer: the whole file (from the pre-cache
//! when it's there, else read from the server behind playback), decoded with
//! the player's own decoder and converted to 16 kHz mono.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::audio::StreamSource;
use crate::audio::convert::Converter;
use crate::audio::decode::{DecodeError, TrackDecoder};
use crate::tags::fetch::{FetchError, Reader};

/// What the recognizer wants.
pub const RATE: u32 = 16_000;
/// Read size per request. Each one holds the transfer gate only briefly, so
/// playback never waits long.
pub const CHUNK: usize = 1024 * 1024;
/// Longest file read (a very long live recording at most).
pub const MAX_FILE: u64 = 1024 * 1024 * 1024;
/// Longest audio transcribed: an hour.
pub const MAX_SECONDS: usize = 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    /// The server can't be read now (try later).
    Unreachable(String),
    /// This file can't be read or decoded.
    Unreadable(String),
    Cancelled,
}

/// The whole file through the player's transfer gate at the lowest priority,
/// in 1 MB range reads. Each read waits while playback is downloading and
/// gives way the moment playback wants the gate.
pub async fn read_whole(reader: &Reader, src: &StreamSource, cancel: &AtomicBool) -> Result<Vec<u8>, AudioError> {
    read_whole_up_to(reader, src, cancel, MAX_FILE).await?.ok_or_else(|| AudioError::Unreadable("file too big".into()))
}

/// [`read_whole`], or Ok(None) as soon as the file turns out bigger than
/// `max` bytes.
pub async fn read_whole_up_to(reader: &Reader, src: &StreamSource, cancel: &AtomicBool, max: u64) -> Result<Option<Vec<u8>>, AudioError> {
    let map = |e: FetchError| match e {
        FetchError::Transient(m) => AudioError::Unreachable(m),
        FetchError::File(m) => AudioError::Unreadable(m),
        FetchError::RangeRefused => AudioError::Unreadable("the server doesn't allow partial reads".into()),
    };
    let max = max.min(MAX_FILE);
    let mut out: Vec<u8> = Vec::new();
    let mut total: Option<u64> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AudioError::Cancelled);
        }
        let got = reader.range(src, out.len() as u64, CHUNK).await.map_err(map)?;
        if total.is_none() {
            total = got.total;
            if let Some(t) = total {
                if t > max {
                    return Ok(None);
                }
                out.reserve(t as usize);
            }
        }
        let n = got.bytes.len();
        out.extend_from_slice(&got.bytes);
        let finished = got.whole || n == 0 || total.is_some_and(|t| out.len() as u64 >= t) || (total.is_none() && n < CHUNK);
        if out.len() as u64 > max {
            return Ok(None);
        }
        if finished {
            return Ok(Some(out));
        }
    }
}

/// Decode a file to 16 kHz mono f32.
pub fn to_16k_mono(bytes: Vec<u8>, hint: Option<&str>, cancel: &AtomicBool) -> Result<Vec<f32>, AudioError> {
    let bad = |e: DecodeError| AudioError::Unreadable(e.to_string());
    let mut dec = TrackDecoder::open(Box::new(Cursor::new(bytes)), hint).map_err(bad)?;
    let mut conv = Converter::new(dec.rate, dec.channels, RATE, 1);
    let mut out = Vec::new();
    let cap = MAX_SECONDS * RATE as usize;
    while let Some(block) = dec.next_block().map_err(bad)? {
        conv.push(block, &mut out);
        if out.len() >= cap {
            out.truncate(cap);
            return Ok(out);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(AudioError::Cancelled);
        }
    }
    conv.finish(&mut out);
    Ok(out)
}
