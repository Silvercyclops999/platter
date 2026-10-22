//! What the app and the helper process say to each other over the helper's
//! stdin and stdout. Every frame is a marker byte, a kind byte, a 32-bit
//! little-endian length and the body: a JSON message, or a song's samples
//! as 32-bit little-endian floats right after the request that needs them.

use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::super::engine::Step;
use super::super::lines::Segment;
use super::super::model::ModelKind;

/// Bumped when the messages change; the app and its helper are always the
/// same binary, so a mismatch means something else answered.
pub const VERSION: u32 = 4;

const MARK: u8 = 0xA7;
const JSON: u8 = b'J';
const SAMPLES: u8 = b'S';
/// The longest frame read (an hour of audio is about 230 MB).
const MAX_FRAME: usize = 512 * 1024 * 1024;

/// App to helper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Request {
    /// Followed by a samples frame of `samples` floats (16 kHz mono).
    #[serde(rename_all = "camelCase")]
    Transcribe {
        model: PathBuf,
        kind: ModelKind,
        threads: usize,
        language: String,
        #[serde(default)]
        prompt: String,
        /// See [`Hints::plain_times`](crate::lyrics::transcribe::engine::Hints::plain_times).
        #[serde(default)]
        plain_times: bool,
        samples: usize,
        lowest: bool,
    },
    /// Followed by a samples frame of `frames` stereo frames (interleaved,
    /// 44.1 kHz): separate the singing with the voice model at `weights`.
    #[serde(rename_all = "camelCase")]
    Separate { weights: PathBuf, frames: usize, lowest: bool },
    /// Stop the run in progress (ignored when nothing runs).
    Cancel,
    /// Free the loaded model.
    Unload,
}

/// Helper to app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Reply {
    /// First, once the helper has started.
    Hello { version: u32 },
    /// The run moved on: loading the model, or the percent listened to.
    Progress { step: Step },
    Done { segments: Vec<Segment> },
    /// The singing, in a samples frame of `samples` floats (16 kHz mono)
    /// right after.
    Vocals { samples: usize },
    Failed { message: String },
    Cancelled,
}

fn frame<W: Write>(w: &mut W, kind: u8, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too long"))?;
    w.write_all(&[MARK, kind])?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// Reads one frame of the given kind. None at a clean end of the stream.
fn read_frame<R: Read>(r: &mut R, kind: u8) -> io::Result<Option<Vec<u8>>> {
    let mut head = [0u8; 6];
    match r.read_exact(&mut head[..1]) {
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        res => res?,
    }
    r.read_exact(&mut head[1..])?;
    if head[0] != MARK || head[1] != kind {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a frame"));
    }
    let len = u32::from_le_bytes([head[2], head[3], head[4], head[5]]) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too long"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

pub fn write_msg<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    frame(w, JSON, &serde_json::to_vec(msg)?)
}

/// None at a clean end of the stream.
pub fn read_msg<R: Read, T: DeserializeOwned>(r: &mut R) -> io::Result<Option<T>> {
    match read_frame(r, JSON)? {
        Some(body) => Ok(Some(serde_json::from_slice(&body)?)),
        None => Ok(None),
    }
}

pub fn write_samples<W: Write>(w: &mut W, pcm: &[f32]) -> io::Result<()> {
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    frame(w, SAMPLES, &bytes)
}

pub fn read_samples<R: Read>(r: &mut R, n: usize) -> io::Result<Vec<f32>> {
    let body = read_frame(r, SAMPLES)?.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
    if body.len() != n * 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "wrong number of samples"));
    }
    Ok(body.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::transcribe::lines::Word;

    #[test]
    fn messages_and_samples_round_trip() {
        let mut buf = Vec::new();
        let req = Request::Transcribe {
            model: PathBuf::from("models/small.bin"),
            kind: ModelKind::Small,
            threads: 2,
            language: "en".into(),
            prompt: "Song by Band.".into(),
            plain_times: true,
            samples: 3,
            lowest: true,
        };
        write_msg(&mut buf, &req).unwrap();
        write_samples(&mut buf, &[0.5, -1.0, 0.25]).unwrap();
        let seg = Segment { words: vec![Word { start_ms: 10, end_ms: 20, text: " hi".into() }], sure: Some(0.75) };
        write_msg(&mut buf, &Reply::Done { segments: vec![seg.clone()] }).unwrap();
        write_msg(&mut buf, &Reply::Progress { step: Step::Loading }).unwrap();
        write_msg(&mut buf, &Reply::Progress { step: Step::Percent(40) }).unwrap();
        let mut r = &buf[..];
        assert_eq!(read_msg::<_, Request>(&mut r).unwrap(), Some(req));
        assert_eq!(read_samples(&mut r, 3).unwrap(), vec![0.5, -1.0, 0.25]);
        assert_eq!(read_msg::<_, Reply>(&mut r).unwrap(), Some(Reply::Done { segments: vec![seg] }));
        assert_eq!(read_msg::<_, Reply>(&mut r).unwrap(), Some(Reply::Progress { step: Step::Loading }));
        assert_eq!(read_msg::<_, Reply>(&mut r).unwrap(), Some(Reply::Progress { step: Step::Percent(40) }));
        assert_eq!(read_msg::<_, Reply>(&mut r).unwrap(), None, "a clean end");
    }

    #[test]
    fn a_separation_and_its_vocals_round_trip() {
        let mut buf = Vec::new();
        let req = Request::Separate { weights: PathBuf::from("models/htdemucs.safetensors"), frames: 2, lowest: false };
        write_msg(&mut buf, &req).unwrap();
        write_samples(&mut buf, &[0.1, 0.2, 0.3, 0.4]).unwrap();
        write_msg(&mut buf, &Reply::Vocals { samples: 1 }).unwrap();
        write_samples(&mut buf, &[0.5]).unwrap();
        let mut r = &buf[..];
        assert_eq!(read_msg::<_, Request>(&mut r).unwrap(), Some(req));
        assert_eq!(read_samples(&mut r, 4).unwrap(), vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(read_msg::<_, Reply>(&mut r).unwrap(), Some(Reply::Vocals { samples: 1 }));
        assert_eq!(read_samples(&mut r, 1).unwrap(), vec![0.5]);
    }

    #[test]
    fn stray_output_and_cut_frames_are_errors() {
        let mut r = &b"whisper_init: loading model\n"[..];
        assert!(read_msg::<_, Reply>(&mut r).is_err());
        let mut buf = Vec::new();
        write_msg(&mut buf, &Reply::Cancelled).unwrap();
        buf.truncate(buf.len() - 2);
        assert!(read_msg::<_, Reply>(&mut &buf[..]).is_err());
        let mut buf = Vec::new();
        write_samples(&mut buf, &[1.0, 2.0]).unwrap();
        assert!(read_samples(&mut &buf[..], 3).is_err(), "the count must match");
    }
}
