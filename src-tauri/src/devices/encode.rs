//! Making a song's file for the device from its original: a copy when the
//! original is already in the wanted codec, otherwise decoded (the player's
//! own decoder) and encoded here. FLAC with flacenc, MP3 with LAME (320
//! kbps constant, or V0), Opus with libopus at about 160 kbps in an Ogg
//! file. Then the tags and a small cover go on.
//!
//! Runs on one plain thread per song (the caller keeps it to two at a time
//! and lowers the thread's priority).

use std::fs::File;
use std::io::Write;
use std::path::Path;

use flacenc::component::BitRepr;
use flacenc::error::Verify;
use flacenc::source::{Fill, Source};

use super::DeviceFormat;
use super::tags::{self, Kind};
use crate::audio::convert::Converter;
use crate::audio::decode::TrackDecoder;
use crate::model::Track;

/// What to do with the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Make {
    /// Copy it unchanged (Original).
    AsIs,
    /// Copy it and redo its tags and cover (already in the wanted codec).
    Retag,
    /// Decode and encode.
    Convert,
}

fn codec_of(t: &Track, src_ext: &str) -> String {
    let c = t.codec.as_deref().unwrap_or_default().to_ascii_lowercase();
    match c.as_str() {
        "mp3" | "mpeg" => "mp3".into(),
        "" => match src_ext {
            "flac" => "flac".into(),
            "mp3" => "mp3".into(),
            "opus" => "opus".into(),
            _ => String::new(),
        },
        _ => c,
    }
}

pub fn make_of(format: DeviceFormat, t: &Track, src_ext: &str) -> Make {
    match format.codec() {
        None => Make::AsIs,
        Some(c) if codec_of(t, src_ext) == c => Make::Retag,
        Some(_) => Make::Convert,
    }
}

/// Makes `dest` from the original at `src`. `art`: album art from the
/// server, for files without their own picture. `stop` is checked as it goes.
pub fn make(src: &Path, src_ext: &str, dest: &Path, format: DeviceFormat, t: &Track, art: Option<&[u8]>, stop: &(dyn Fn() -> bool + Sync)) -> Result<(), String> {
    let how = make_of(format, t, src_ext);
    if how != Make::Convert {
        std::fs::copy(src, dest).map_err(|e| format!("couldn't copy the file ({e})"))?;
        if how == Make::AsIs {
            return Ok(());
        }
    } else {
        let hint = crate::audio::decode::extension_hint(t.codec.as_deref(), t.container.as_deref()).unwrap_or_else(|| src_ext.to_string());
        let bits = t.bit_depth.unwrap_or(16);
        match format {
            DeviceFormat::Flac => flac(src, &hint, dest, bits, stop)?,
            DeviceFormat::Mp3Cbr320 | DeviceFormat::Mp3V0 => mp3(src, &hint, dest, format == DeviceFormat::Mp3V0, stop)?,
            DeviceFormat::Opus => opus(src, &hint, dest, stop)?,
            DeviceFormat::Original => unreachable!("Original is copied"),
        }
    }
    let kind = if matches!(format, DeviceFormat::Mp3Cbr320 | DeviceFormat::Mp3V0) { Kind::Id3 } else { Kind::Vorbis };
    let (pairs, pictures) = tags::pairs_for(src, t);
    let cover = tags::cover(&pictures, art, super::COVER_PX);
    tags::write(dest, kind, &pairs, cover)
}

// ------------------------------------------------------------------ decoding

/// Decoded audio converted to the rate and channels an encoder wants.
struct Pcm {
    dec: TrackDecoder,
    conv: Converter,
    ended: bool,
    pub rate: u32,
    pub channels: usize,
    pub source_rate: u32,
}

impl Pcm {
    fn open(src: &Path, hint: &str, rate: impl Fn(u32) -> u32, channels: impl Fn(usize) -> usize) -> Result<Self, String> {
        let f = File::open(src).map_err(|e| format!("couldn't open the song ({e})"))?;
        let dec = TrackDecoder::open(Box::new(f), Some(hint)).map_err(|e| format!("couldn't read the song ({e})"))?;
        let (out_rate, out_ch) = (rate(dec.rate), channels(dec.channels));
        Ok(Self {
            conv: Converter::new(dec.rate, dec.channels, out_rate, out_ch),
            source_rate: dec.rate,
            dec,
            ended: false,
            rate: out_rate,
            channels: out_ch,
        })
    }

    /// The next stretch of interleaved samples (-1 to 1), None at the end.
    fn next(&mut self, stop: &(dyn Fn() -> bool + Sync)) -> Result<Option<Vec<f32>>, String> {
        loop {
            if stop() {
                return Err(STOPPED.into());
            }
            if self.ended {
                return Ok(None);
            }
            let mut out = Vec::new();
            match self.dec.next_block().map_err(|e| format!("couldn't read the song ({e})"))? {
                Some(block) => self.conv.push(block, &mut out),
                None => {
                    self.ended = true;
                    self.conv.finish(&mut out);
                }
            }
            if !out.is_empty() {
                return Ok(Some(out));
            }
        }
    }
}

pub const STOPPED: &str = "stopped";

fn write_file(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = File::create(dest).map_err(|e| format!("couldn't write the file ({e})"))?;
    f.write_all(bytes).map_err(|e| format!("couldn't write the file ({e})"))
}

// ---------------------------------------------------------------------- FLAC

struct FlacSource<'a> {
    pcm: Pcm,
    bits: usize,
    pending: Vec<i32>,
    stop: &'a (dyn Fn() -> bool + Sync),
    err: Option<String>,
}

impl Source for FlacSource<'_> {
    fn channels(&self) -> usize {
        self.pcm.channels
    }
    fn bits_per_sample(&self) -> usize {
        self.bits
    }
    fn sample_rate(&self) -> usize {
        self.pcm.rate as usize
    }
    fn read_samples<F: Fill>(&mut self, block_size: usize, dest: &mut F) -> Result<usize, flacenc::error::SourceError> {
        let ch = self.pcm.channels;
        let scale = ((1i64 << (self.bits - 1)) - 1) as f32;
        while self.err.is_none() && self.pending.len() < block_size * ch {
            match self.pcm.next(self.stop) {
                Ok(Some(block)) => self.pending.extend(block.iter().map(|s| (s.clamp(-1.0, 1.0) * scale).round() as i32)),
                Ok(None) => break,
                // Ends the stream here; the caller sees the error.
                Err(e) => self.err = Some(e),
            }
        }
        if self.err.is_some() {
            return Ok(0);
        }
        let n = (self.pending.len() / ch).min(block_size);
        if n == 0 {
            return Ok(0);
        }
        dest.fill_interleaved(&self.pending[..n * ch])?;
        self.pending.drain(..n * ch);
        Ok(n)
    }
}

fn flac(src: &Path, hint: &str, dest: &Path, bits: u32, stop: &(dyn Fn() -> bool + Sync)) -> Result<(), String> {
    let pcm = Pcm::open(src, hint, |r| r, |c| c.min(8))?;
    let bits = bits.clamp(16, 24) as usize;
    let mut source = FlacSource { pcm, bits, pending: Vec::new(), stop, err: None };
    let config = flacenc::config::Encoder::default().into_verified().map_err(|(_, e)| format!("FLAC settings ({e:?})"))?;
    let block = config.block_size;
    let mut stream = flacenc::encode_with_fixed_block_size(&config, &mut source, block).map_err(|e| format!("couldn't make the FLAC file ({e:?})"))?;
    if let Some(e) = source.err {
        return Err(e);
    }
    // A fixed block size stream: the shorter last block doesn't count as
    // the smallest (decoders read a smaller minimum as variable blocks).
    if stream.frame_count() > 1 {
        let _ = stream.stream_info_mut().set_block_sizes(block, block);
    }
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| format!("couldn't make the FLAC file ({e:?})"))?;
    write_file(dest, sink.as_slice())
}

// ----------------------------------------------------------------------- MP3

/// MP3 takes 32, 44.1 or 48 kHz: anything else goes to the nearer family.
fn mp3_rate(r: u32) -> u32 {
    match r {
        32_000 | 44_100 | 48_000 => r,
        r if r.is_multiple_of(11_025) => 44_100,
        _ => 48_000,
    }
}

fn mp3(src: &Path, hint: &str, dest: &Path, vbr: bool, stop: &(dyn Fn() -> bool + Sync)) -> Result<(), String> {
    use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, InterleavedPcm, Mode, MonoPcm, Quality, VbrMode};
    let mut pcm = Pcm::open(src, hint, mp3_rate, |c| c.min(2))?;
    let e = |what: &'static str| move |x: mp3lame_encoder::BuildError| format!("MP3 {what} ({x:?})");
    let mut b = Builder::new().ok_or("the MP3 encoder couldn't start")?;
    b.set_num_channels(pcm.channels as u8).map_err(e("channels"))?;
    b.set_sample_rate(pcm.rate).map_err(e("rate"))?;
    b.set_mode(if pcm.channels == 1 { Mode::Mono } else { Mode::JointStereo }).map_err(e("mode"))?;
    b.set_quality(Quality::NearBest).map_err(e("quality"))?;
    if vbr {
        b.set_vbr_mode(VbrMode::Mtrh).map_err(e("VBR"))?;
        b.set_vbr_quality(Quality::Best).map_err(e("VBR quality"))?;
    } else {
        b.set_vbr_mode(VbrMode::Off).map_err(e("VBR"))?;
        b.set_brate(Bitrate::Kbps320).map_err(e("bit rate"))?;
    }
    b.set_to_write_vbr_tag(true).map_err(e("tag"))?;
    let mut enc = b.build().map_err(e("settings"))?;
    let mut out: Vec<u8> = Vec::new();
    let ee = |x: mp3lame_encoder::EncodeError| format!("couldn't make the MP3 file ({x:?})");
    while let Some(block) = pcm.next(stop)? {
        let frames = block.len() / pcm.channels;
        out.reserve(mp3lame_encoder::max_required_buffer_size(frames));
        if pcm.channels == 1 {
            enc.encode_to_vec(MonoPcm(&block[..]), &mut out).map_err(ee)?;
        } else {
            enc.encode_to_vec(InterleavedPcm(&block[..]), &mut out).map_err(ee)?;
        }
    }
    out.reserve(7200);
    enc.flush_to_vec::<FlushNoGap>(&mut out).map_err(ee)?;
    // The LAME tag (length and gapless info) replaces the first frame.
    let mut tag = Vec::with_capacity(enc.lame_tag_size());
    if let Some(n) = enc.lame_tag_encode_to_vec(&mut tag)
        && n.get() <= out.len()
    {
        out[..n.get()].copy_from_slice(&tag[..n.get()]);
    }
    write_file(dest, &out)
}

// ---------------------------------------------------------------------- Opus

pub const OPUS_KBPS: i32 = 160;
const OPUS_RATE: u32 = 48_000;
/// 20 ms at 48 kHz.
const OPUS_FRAME: usize = 960;

struct OpusEnc(*mut unsafe_libopus::OpusEncoder);

impl Drop for OpusEnc {
    fn drop(&mut self) {
        // SAFETY: made by opus_encoder_create and freed once here.
        unsafe { unsafe_libopus::opus_encoder_destroy(self.0) }
    }
}

fn opus_head(channels: u8, pre_skip: u16, rate: u32) -> Vec<u8> {
    let mut h = b"OpusHead".to_vec();
    h.push(1);
    h.push(channels);
    h.extend_from_slice(&pre_skip.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&0i16.to_le_bytes());
    h.push(0);
    h
}

fn opus_tags() -> Vec<u8> {
    let vendor = b"Platter";
    let mut t = b"OpusTags".to_vec();
    t.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    t.extend_from_slice(vendor);
    t.extend_from_slice(&0u32.to_le_bytes());
    t
}

fn opus(src: &Path, hint: &str, dest: &Path, stop: &(dyn Fn() -> bool + Sync)) -> Result<(), String> {
    use ogg::writing::{PacketWriteEndInfo, PacketWriter};
    use unsafe_libopus::{OPUS_APPLICATION_AUDIO, OPUS_GET_LOOKAHEAD_REQUEST, OPUS_OK, OPUS_SET_BITRATE_REQUEST, opus_encoder_ctl};

    let mut pcm = Pcm::open(src, hint, |_| OPUS_RATE, |c| c.min(2))?;
    let ch = pcm.channels;
    let mut err = 0i32;
    // SAFETY: plain constructor; the pointer is checked and owned by OpusEnc.
    let enc = OpusEnc(unsafe { unsafe_libopus::opus_encoder_create(OPUS_RATE as i32, ch as i32, OPUS_APPLICATION_AUDIO, &mut err) });
    if enc.0.is_null() || err != OPUS_OK {
        return Err(format!("the Opus encoder couldn't start ({err})"));
    }
    let bitrate = if ch == 1 { OPUS_KBPS / 2 } else { OPUS_KBPS } * 1000;
    let mut lookahead = 0i32;
    // SAFETY: valid encoder; requests with the argument types they take.
    unsafe {
        opus_encoder_ctl!(enc.0, OPUS_SET_BITRATE_REQUEST, bitrate);
        opus_encoder_ctl!(enc.0, OPUS_GET_LOOKAHEAD_REQUEST, &mut lookahead);
    }
    let pre_skip = lookahead.max(0) as u64;

    let file = File::create(dest).map_err(|e| format!("couldn't write the file ({e})"))?;
    let mut w = PacketWriter::new(std::io::BufWriter::new(file));
    let serial: u32 = rand::random();
    let io_err = |e: std::io::Error| format!("couldn't write the file ({e})");
    w.write_packet(opus_head(ch as u8, pre_skip as u16, pcm.source_rate), serial, PacketWriteEndInfo::EndPage, 0).map_err(io_err)?;
    w.write_packet(opus_tags(), serial, PacketWriteEndInfo::EndPage, 0).map_err(io_err)?;

    let mut pending: Vec<f32> = Vec::new();
    let mut fed: u64 = 0;
    let mut sent: u64 = 0;
    let mut packet = vec![0u8; 4000];
    let mut held: Option<Vec<u8>> = None;
    let mut ended = false;
    loop {
        while !ended && pending.len() < OPUS_FRAME * ch {
            match pcm.next(stop)? {
                Some(b) => {
                    fed += (b.len() / ch) as u64;
                    pending.extend(b);
                }
                None => ended = true,
            }
        }
        if pending.is_empty() {
            break;
        }
        if pending.len() < OPUS_FRAME * ch {
            pending.resize(OPUS_FRAME * ch, 0.0);
        }
        // SAFETY: `pending` holds a whole frame and `packet` is its stated size.
        let n = unsafe {
            unsafe_libopus::opus_encode_float(enc.0, pending.as_ptr(), OPUS_FRAME as i32, packet.as_mut_ptr(), packet.len() as i32)
        };
        if n < 0 {
            return Err(format!("couldn't make the Opus file ({n})"));
        }
        pending.drain(..OPUS_FRAME * ch);
        // Each packet goes out once the next is known, so the last one can
        // end the stream with the true length.
        if let Some(p) = held.take() {
            w.write_packet(p, serial, PacketWriteEndInfo::NormalPacket, pre_skip + sent).map_err(io_err)?;
        }
        sent += OPUS_FRAME as u64;
        held = Some(packet[..n as usize].to_vec());
    }
    let last = held.unwrap_or_default();
    w.write_packet(last, serial, PacketWriteEndInfo::EndStream, pre_skip + fed.min(sent)).map_err(io_err)?;
    w.into_inner().flush().map_err(io_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp3_rates_snap_to_ones_it_takes() {
        assert_eq!(mp3_rate(44_100), 44_100);
        assert_eq!(mp3_rate(88_200), 44_100);
        assert_eq!(mp3_rate(96_000), 48_000);
        assert_eq!(mp3_rate(22_050), 44_100);
    }

    #[test]
    fn the_opus_header_is_the_standard_layout() {
        let h = opus_head(2, 312, 44_100);
        assert_eq!(&h[..8], b"OpusHead");
        assert_eq!(h.len(), 19);
        assert_eq!(h[9], 2);
        assert_eq!(u16::from_le_bytes([h[10], h[11]]), 312);
        assert_eq!(u32::from_le_bytes([h[12], h[13], h[14], h[15]]), 44_100);
    }

    fn sine_wav(path: &Path, frames: u32) {
        let spec = hound::WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..frames {
            let v = ((i as f32 * 0.05).sin() * 9000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
    }

    fn frames_in(path: &Path, hint: &str) -> usize {
        let mut dec = TrackDecoder::open(Box::new(File::open(path).unwrap()), Some(hint)).unwrap();
        let mut n = 0;
        loop {
            match dec.next_block() {
                Ok(Some(b)) => n += b.len() / dec.channels,
                Ok(None) => return n,
                Err(e) => panic!("after {n} frames: {e}"),
            }
        }
    }

    #[test]
    fn flac_files_decode_whole() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("a.wav");
        sine_wav(&src, 13_230);
        let out = d.path().join("a.flac");
        flac(&src, "wav", &out, 16, &|| false).unwrap();
        assert_eq!(frames_in(&out, "flac"), 13_230);
    }
}
