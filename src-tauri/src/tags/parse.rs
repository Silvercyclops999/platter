//! Tag parsers that work on the start of a file (or a few pieces of it).
//!
//! The scan reads the first 64 KB of a file through Plex. Most tags live
//! there. When they don't (a big cover picture before the tags, or an MP4
//! whose `moov` atom sits after the audio), the parser says which bytes it
//! needs next ([`Step::Need`]) and is run again once they're added. Formats:
//! FLAC (Vorbis comments, with or without a leading ID3 tag), ID3v2.2 to 2.4
//! (MP3), MP4 atoms (M4A, ALAC) and Ogg (Vorbis, Opus, FLAC).

use super::FileTags;

/// First read of every file.
pub const FIRST_READ: usize = 64 * 1024;
/// Reads that follow a jump (the next atom or frame header).
pub const MORE_READ: usize = 64 * 1024;
/// Largest single piece read for one tag block, frame or atom.
pub const MAX_BLOCK: usize = 1024 * 1024;

/// Pieces of one file read so far.
#[derive(Debug, Default, Clone)]
pub struct Sparse {
    chunks: Vec<(u64, Vec<u8>)>,
    /// File size, when the server said (Content-Range).
    pub total: Option<u64>,
    /// Plex's container name ("flac", "mp3", "mp4", "ogg"), when known.
    pub container: Option<String>,
}

impl Sparse {
    pub fn new(first: Vec<u8>, total: Option<u64>) -> Self {
        Self { chunks: vec![(0, first)], total, container: None }
    }

    pub fn add(&mut self, offset: u64, bytes: Vec<u8>) {
        self.chunks.push((offset, bytes));
    }

    /// Bytes held, all pieces together.
    pub fn held(&self) -> usize {
        self.chunks.iter().map(|c| c.1.len()).sum()
    }

    /// `len` bytes at `offset`, when one piece holds them all.
    pub fn get(&self, offset: u64, len: usize) -> Option<&[u8]> {
        self.chunks.iter().find_map(|(start, b)| {
            let end = start + b.len() as u64;
            (offset >= *start && offset + len as u64 <= end).then(|| {
                let i = (offset - start) as usize;
                &b[i..i + len]
            })
        })
    }
}

/// What a parse pass found.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // one per file read, never stored
pub enum Step {
    Done(FileTags),
    /// Read at least `len` bytes at `offset`, then parse again.
    Need { offset: u64, len: usize },
}

/// Why a pass stopped early.
enum Miss {
    Need(u64, usize),
    /// Past the end of the file, or the data makes no sense: keep what was found.
    Stop,
}

type R<T> = Result<T, Miss>;

/// `len` bytes at `offset` from the pieces, or a request for them.
fn need(s: &Sparse, offset: u64, len: usize) -> R<&[u8]> {
    if let Some(total) = s.total
        && offset + len as u64 > total
    {
        return Err(Miss::Stop);
    }
    s.get(offset, len).ok_or(Miss::Need(offset, len))
}

/// Parse whatever the pieces hold. With `last` set, missing bytes end the
/// pass with what was found so far instead of asking for more.
pub fn parse(s: &Sparse, last: bool) -> Step {
    let mut tags = FileTags::for_read();
    let r = parse_into(s, &mut tags);
    match r {
        Err(Miss::Need(offset, len)) if !last => Step::Need { offset, len },
        _ => Step::Done(tags),
    }
}

fn parse_into(s: &Sparse, tags: &mut FileTags) -> R<()> {
    let head = need(s, 0, 12)?;
    if &head[..4] == b"fLaC" {
        return flac(s, 0, tags);
    }
    if &head[..3] == b"ID3" {
        let end = id3(s, 0, tags)?;
        // Some FLAC files carry an ID3 tag in front. Only read past the tag
        // for one Plex calls FLAC: for an MP3 it would be a wasted read.
        let flac_file = s.container.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("flac"));
        return match need(s, end, 4) {
            Ok(b) if b == b"fLaC" => flac(s, end, tags),
            Err(Miss::Need(o, l)) if flac_file => Err(Miss::Need(o, l)),
            _ => Ok(()),
        };
    }
    if &head[..4] == b"OggS" {
        return ogg(s, tags);
    }
    if &head[4..8] == b"ftyp" {
        return mp4(s, tags);
    }
    Ok(())
}

// ------------------------------------------------------------------ Vorbis comments

fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn u32be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// A Vorbis comment block. Stops quietly at a truncated entry.
pub fn vorbis_comment(b: &[u8], tags: &mut FileTags) {
    let mut i = 0usize;
    let take = |i: &mut usize, n: usize| -> Option<&[u8]> {
        let out = b.get(*i..i.checked_add(n)?)?;
        *i += n;
        Some(out)
    };
    let Some(vlen) = take(&mut i, 4).map(u32le) else { return };
    if take(&mut i, vlen as usize).is_none() {
        return;
    }
    let Some(count) = take(&mut i, 4).map(u32le) else { return };
    for _ in 0..count.min(10_000) {
        let Some(len) = take(&mut i, 4).map(u32le) else { return };
        let Some(entry) = take(&mut i, len as usize) else { return };
        let text = String::from_utf8_lossy(entry);
        if let Some((k, v)) = text.split_once('=') {
            tags.set(k, v);
        }
    }
}

// ------------------------------------------------------------------ FLAC

fn flac(s: &Sparse, base: u64, tags: &mut FileTags) -> R<()> {
    let mut off = base + 4;
    for _ in 0..64 {
        let h = need(s, off, 4)?;
        let last = h[0] & 0x80 != 0;
        let kind = h[0] & 0x7f;
        let len = u32::from_be_bytes([0, h[1], h[2], h[3]]) as usize;
        if kind == 4 {
            let body = need(s, off + 4, len.min(MAX_BLOCK))?;
            vorbis_comment(body, tags);
            return Ok(());
        }
        if last || kind == 127 {
            return Ok(());
        }
        off += 4 + len as u64;
    }
    Ok(())
}

// ------------------------------------------------------------------ ID3v2

fn synchsafe(b: &[u8]) -> u32 {
    b.iter().take(4).fold(0u32, |acc, x| (acc << 7) | (*x as u32 & 0x7f))
}

/// Undo ID3 unsynchronisation (0xFF 0x00 → 0xFF).
fn unsync(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut prev = 0u8;
    for &x in b {
        if !(prev == 0xff && x == 0) {
            out.push(x);
        }
        prev = x;
    }
    out
}

/// Parses an ID3v2 tag at `base`; returns where it ends.
fn id3(s: &Sparse, base: u64, tags: &mut FileTags) -> R<u64> {
    let h = need(s, base, 10)?;
    let ver = h[3];
    let flags = h[5];
    let size = synchsafe(&h[6..10]) as u64;
    let footer = if ver == 4 && flags & 0x10 != 0 { 10 } else { 0 };
    let end = base + 10 + size + footer;
    if !(2..=4).contains(&ver) {
        return Ok(end);
    }
    let body = base + 10;
    // Whole-tag unsynchronisation (2.2 and 2.3): read it all, then undo it.
    if flags & 0x80 != 0 && ver < 4 {
        let raw = need(s, body, (size as usize).min(MAX_BLOCK))?;
        let clean = unsync(raw);
        let mut mem = Sparse::new(clean, None);
        mem.total = Some(mem.held() as u64);
        let _ = frames(&mem, 0, mem.held() as u64, ver, flags & 0x40 != 0, false, tags);
        return Ok(end);
    }
    frames(s, body, base + 10 + size, ver, flags & 0x40 != 0, true, tags)?;
    Ok(end)
}

/// Walks frames from `start` to `end`, reading only the ones wanted.
fn frames(s: &Sparse, start: u64, end: u64, ver: u8, ext: bool, frame_unsync: bool, tags: &mut FileTags) -> R<()> {
    let mut off = start;
    if ext && ver >= 3 {
        let e = need(s, off, 4)?;
        off += if ver == 3 { 4 + u32be(e) as u64 } else { synchsafe(e) as u64 };
    }
    let hdr: u64 = if ver == 2 { 6 } else { 10 };
    while off + hdr <= end {
        let h = need(s, off, hdr as usize)?;
        if h[0] == 0 {
            break; // padding
        }
        let (id, size, fflags): (&[u8], u64, u16) = match ver {
            2 => (&h[..3], u32::from_be_bytes([0, h[3], h[4], h[5]]) as u64, 0),
            3 => (&h[..4], u32be(&h[4..8]) as u64, u16::from_be_bytes([h[8], h[9]])),
            _ => (&h[..4], synchsafe(&h[4..8]) as u64, u16::from_be_bytes([h[8], h[9]])),
        };
        if size == 0 || off + hdr + size > end {
            break;
        }
        let kind = match id {
            b"TXXX" | b"TXX" => Some(Frame::UserText),
            b"TSRC" | b"TRC" => Some(Frame::Isrc),
            b"UFID" | b"UFI" => Some(Frame::Ufid),
            b"TALB" | b"TAL" => Some(Frame::Text("ALBUM")),
            b"TPE2" | b"TP2" => Some(Frame::Text("ALBUMARTIST")),
            b"TDRC" | b"TYER" | b"TYE" => Some(Frame::Text("YEAR")),
            _ => None,
        };
        if let Some(kind) = kind {
            let compressed_or_encrypted = match ver {
                3 => fflags & 0x00c0 != 0,
                4 => fflags & 0x000c != 0,
                _ => false,
            };
            if !compressed_or_encrypted {
                let data = need(s, off + hdr, (size as usize).min(MAX_BLOCK))?;
                let mut data = std::borrow::Cow::Borrowed(data);
                if ver == 4 && frame_unsync && fflags & 0x0002 != 0 {
                    data = std::borrow::Cow::Owned(unsync(&data));
                }
                let d: &[u8] = if ver == 4 && fflags & 0x0001 != 0 { data.get(4..).unwrap_or(&[]) } else { &data };
                match kind {
                    Frame::UserText => txxx(d, tags),
                    Frame::Isrc => text_frame(d, "ISRC", tags),
                    Frame::Ufid => ufid(d, tags),
                    Frame::Text(key) => text_frame(d, key, tags),
                }
            }
        }
        off += hdr + size;
    }
    Ok(())
}

/// Decodes ID3 text in encoding `enc` (0 Latin-1, 1 UTF-16 with BOM, 2 UTF-16BE, 3 UTF-8).
fn decode(enc: u8, b: &[u8]) -> String {
    match enc {
        1 | 2 => {
            let (be, body) = match b {
                [0xfe, 0xff, rest @ ..] => (true, rest),
                [0xff, 0xfe, rest @ ..] => (false, rest),
                _ => (enc == 2, b),
            };
            let units: Vec<u16> = body
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| if be { u16::from_be_bytes(*c) } else { u16::from_le_bytes(*c) })
                .collect();
            String::from_utf16_lossy(&units)
        }
        3 => String::from_utf8_lossy(b).into_owned(),
        _ => b.iter().map(|&c| c as char).collect(),
    }
}

/// Splits ID3 text on its terminator (one zero byte, or two aligned for UTF-16).
fn split_terminated(enc: u8, b: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    if enc == 1 || enc == 2 {
        let mut start = 0;
        let mut i = 0;
        while i + 1 < b.len() {
            if b[i] == 0 && b[i + 1] == 0 {
                out.push(&b[start..i]);
                start = i + 2;
            }
            i += 2;
        }
        out.push(&b[start.min(b.len())..]);
    } else {
        out.extend(b.split(|&x| x == 0));
    }
    out
}

/// ID3 frames the scan reads.
#[derive(Clone, Copy)]
enum Frame {
    /// TXXX: a named value (the ARCHIVE_* tags and the like).
    UserText,
    /// TSRC: the ISRC.
    Isrc,
    /// UFID: an id owned by a database; MusicBrainz keeps its recording id here.
    Ufid,
    /// A standard text frame kept under this name (album, album artist, year).
    Text(&'static str),
}

/// A standard text frame (encoding byte, then one or more values) stored as `key`.
fn text_frame(d: &[u8], key: &str, tags: &mut FileTags) {
    let Some((&enc, rest)) = d.split_first() else { return };
    for v in split_terminated(enc, rest) {
        let v = decode(enc, v);
        if !v.trim().is_empty() {
            tags.set(key, &v);
        }
    }
}

/// A unique file id: owner (Latin-1, zero-terminated), then up to 64 bytes.
/// Only MusicBrainz's is kept, as the recording id.
fn ufid(d: &[u8], tags: &mut FileTags) {
    let Some(end) = d.iter().position(|&b| b == 0) else { return };
    let owner = &d[..end];
    if owner.eq_ignore_ascii_case(b"http://musicbrainz.org") || owner.eq_ignore_ascii_case(b"https://musicbrainz.org") {
        let id = String::from_utf8_lossy(&d[end + 1..]);
        tags.set("MUSICBRAINZ_TRACKID", &id);
    }
}

/// A user text frame: description, then one or more values.
fn txxx(d: &[u8], tags: &mut FileTags) {
    let Some((&enc, rest)) = d.split_first() else { return };
    let parts = split_terminated(enc, rest);
    let Some((desc, values)) = parts.split_first() else { return };
    let key = decode(enc, desc);
    for v in values {
        let v = decode(enc, v);
        if !v.trim().is_empty() {
            tags.set(&key, &v);
        }
    }
}

// ------------------------------------------------------------------ MP4

/// An atom: where its body starts and where it ends.
struct Atom {
    kind: [u8; 4],
    body: u64,
    end: u64,
}

fn atom_at(s: &Sparse, off: u64, limit: u64) -> R<Atom> {
    let h = need(s, off, 8)?;
    let size = u32be(&h[..4]) as u64;
    let kind = [h[4], h[5], h[6], h[7]];
    let (body, end) = match size {
        0 => (off + 8, limit),
        1 => {
            let l = need(s, off + 8, 8)?;
            (off + 16, off + u64::from_be_bytes(l.try_into().unwrap_or([0; 8])))
        }
        n => (off + 8, off + n),
    };
    if end < body || end > limit {
        return Err(Miss::Stop);
    }
    Ok(Atom { kind, body, end })
}

fn find_child(s: &Sparse, start: u64, end: u64, kind: &[u8; 4]) -> R<Option<Atom>> {
    let mut off = start;
    while off + 8 <= end {
        let a = atom_at(s, off, end)?;
        if &a.kind == kind {
            return Ok(Some(a));
        }
        if a.end <= off {
            break;
        }
        off = a.end;
    }
    Ok(None)
}

fn mp4(s: &Sparse, tags: &mut FileTags) -> R<()> {
    let limit = s.total.unwrap_or(u64::MAX);
    let Some(moov) = find_child(s, 0, limit, b"moov")? else { return Ok(()) };
    let Some(udta) = find_child(s, moov.body, moov.end, b"udta")? else { return Ok(()) };
    let Some(meta) = find_child(s, udta.body, udta.end, b"meta")? else { return Ok(()) };
    // `meta` is a full atom (4 bytes of version and flags) in iTunes files,
    // a plain one in QuickTime files: plain when a child header comes next.
    let peek = need(s, meta.body, 8)?;
    let start = if &peek[4..8] == b"hdlr" || &peek[4..8] == b"ilst" { meta.body } else { meta.body + 4 };
    let Some(ilst) = find_child(s, start, meta.end, b"ilst")? else { return Ok(()) };
    let mut off = ilst.body;
    while off + 8 <= ilst.end {
        let item = atom_at(s, off, ilst.end)?;
        match &item.kind {
            b"rtng" => {
                if let Some(d) = find_child(s, item.body, item.end, b"data")? {
                    let v = need(s, d.body, ((d.end - d.body) as usize).min(64))?;
                    match v.get(8) {
                        Some(1) | Some(4) => tags.set_explicit(Some(true)),
                        Some(2) => tags.set_explicit(Some(false)),
                        _ => {}
                    }
                }
            }
            // Album, album artist and date: plain text in a data child.
            b"\xa9alb" | b"aART" | b"\xa9day" => {
                if let Some(d) = find_child(s, item.body, item.end, b"data")? {
                    let v = need(s, d.body, ((d.end - d.body) as usize).min(1024))?;
                    let value = String::from_utf8_lossy(v.get(8..).unwrap_or(&[])).into_owned();
                    let key = match &item.kind {
                        b"aART" => "ALBUMARTIST",
                        b"\xa9day" => "YEAR",
                        _ => "ALBUM",
                    };
                    tags.set(key, &value);
                }
            }
            b"----" => {
                let name = find_child(s, item.body, item.end, b"name")?;
                let data = find_child(s, item.body, item.end, b"data")?;
                if let (Some(n), Some(d)) = (name, data) {
                    let n = need(s, n.body, ((n.end - n.body) as usize).min(256))?;
                    let key = String::from_utf8_lossy(n.get(4..).unwrap_or(&[])).into_owned();
                    if tags.keeps(&key) {
                        let v = need(s, d.body, ((d.end - d.body) as usize).min(MAX_BLOCK))?;
                        let value = String::from_utf8_lossy(v.get(8..).unwrap_or(&[])).into_owned();
                        tags.set(&key, &value);
                    }
                }
            }
            _ => {}
        }
        if item.end <= off {
            break;
        }
        off = item.end;
    }
    Ok(())
}

// ------------------------------------------------------------------ Ogg

#[derive(Clone, Copy, PartialEq)]
enum OggKind {
    Vorbis,
    Opus,
    Flac,
    Other,
}

fn ogg_comment(kind: OggKind, p: &[u8], tags: &mut FileTags) {
    match kind {
        OggKind::Vorbis if p.starts_with(b"\x03vorbis") => vorbis_comment(&p[7..], tags),
        OggKind::Opus if p.starts_with(b"OpusTags") => vorbis_comment(&p[8..], tags),
        // Ogg FLAC: the second packet is a metadata block (4-byte header).
        OggKind::Flac if p.first().is_some_and(|b| b & 0x7f == 4) => vorbis_comment(p.get(4..).unwrap_or(&[]), tags),
        _ => {}
    }
}

fn ogg(s: &Sparse, tags: &mut FileTags) -> R<()> {
    let mut off = 0u64;
    let mut packet: Vec<u8> = Vec::new();
    let mut count = 0;
    let mut kind = OggKind::Other;
    let r = (|| -> R<()> {
        for _ in 0..512 {
            let h = need(s, off, 27)?;
            if &h[..4] != b"OggS" {
                return Err(Miss::Stop);
            }
            let nseg = h[26] as usize;
            let table = need(s, off + 27, nseg)?.to_vec();
            let body_len: usize = table.iter().map(|x| *x as usize).sum();
            let body = need(s, off + 27 + nseg as u64, body_len)?;
            let mut at = 0;
            for lace in table {
                packet.extend_from_slice(&body[at..at + lace as usize]);
                at += lace as usize;
                if lace < 255 {
                    count += 1;
                    if count == 1 {
                        kind = if packet.starts_with(b"\x01vorbis") {
                            OggKind::Vorbis
                        } else if packet.starts_with(b"OpusHead") {
                            OggKind::Opus
                        } else if packet.starts_with(b"\x7fFLAC") {
                            OggKind::Flac
                        } else {
                            OggKind::Other
                        };
                        if kind == OggKind::Other {
                            return Ok(());
                        }
                        packet.clear();
                    } else {
                        return Ok(());
                    }
                }
            }
            if packet.len() > MAX_BLOCK {
                return Ok(());
            }
            off += 27 + nseg as u64 + body_len as u64;
        }
        Ok(())
    })();
    // A complete comment packet, or as much of it as was read.
    if count >= 1 && !packet.is_empty() {
        ogg_comment(kind, &packet, tags);
    }
    r
}
