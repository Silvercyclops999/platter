//! One file's tags through `lofty`, field by field (docs/TAGGING.md).
//!
//! FLAC and Ogg keep Vorbis comments, MP3 an ID3v2 tag (written in the
//! version the file already has: v2.3 stays v2.3, everything else is v2.4),
//! MP4 an `ilst` with freeform `----:com.apple.iTunes:*` items for what has
//! no atom of its own. Only the fields asked for change; every other tag,
//! picture and frame is written back as it was read, and in the same place
//! in the tag. A field a file keeps under another name than the usual one
//! (TOTALTRACKS, an EXPLICIT tag, a `RELEASETYPE` TXXX) is written there.

use std::borrow::Cow;
use std::fs::File;
use std::path::Path;
use std::str::FromStr;

use lofty::TextEncoding;
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType};
use lofty::flac::FlacFile;
use lofty::id3::v2::{ExtendedTextFrame, Frame, FrameId, Id3v2Tag, Id3v2Version, TextInformationFrame};
use lofty::mp4::{AdvisoryRating, Atom, AtomData, AtomIdent, DataType, Ilst, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OpusFile, VorbisFile};
use lofty::tag::{Accessor, TagExt};
use lofty::tag::items::Timestamp;

use super::Values;
use super::fields::Field;
use crate::error::{AppError, AppResult};

/// What a read needs besides the file: the custom status tag's name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ctx {
    /// Upper case, None when Settings has none.
    pub custom_key: Option<String>,
    /// The album writes track and disc totals as `n/total` in TRACKNUMBER
    /// and DISCNUMBER (Vorbis files that carry no total yet follow it).
    pub slash_numbers: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    Flac,
    OggVorbis,
    Opus,
    Mp3,
    Mp4,
}

enum Inner {
    Flac(Box<FlacFile>),
    Vorbis(VorbisComments),
    Id3 { tag: Id3v2Tag, v23: bool },
    Mp4(Ilst),
}

/// One file's tags, read and ready to change.
pub struct Doc {
    inner: Inner,
    pub format: Format,
}

fn io_err(path: &Path, e: impl std::fmt::Display) -> AppError {
    AppError::Io(format!("{}: {e}", path.display()))
}

const ITUNES: &str = "com.apple.iTunes";

impl Doc {
    /// Reads a file's tags (not its audio).
    pub fn read(path: &Path) -> AppResult<Doc> {
        let probe = lofty::probe::Probe::open(path).map_err(|e| io_err(path, e))?;
        let probe = probe.guess_file_type().map_err(|e| io_err(path, e))?;
        let ty = probe.file_type();
        let opts = ParseOptions::new().read_properties(false);
        // Vorbis comments as written ("3/12" stays in TRACKNUMBER). ID3 keeps
        // lofty's conversions, which turn v2.3 dates back on writing.
        let raw = ParseOptions::new().read_properties(false).implicit_conversions(false);
        let mut f = File::open(path).map_err(|e| io_err(path, e))?;
        let bad = |e: lofty::error::FileParseError| AppError::Decode(format!("{}: {e}", path.display()));
        Ok(match ty {
            Some(FileType::Flac) => {
                let mut ff = FlacFile::read_from(&mut f, raw).map_err(bad)?;
                if ff.vorbis_comments().is_none() {
                    ff.set_vorbis_comments(VorbisComments::new());
                }
                Doc { inner: Inner::Flac(Box::new(ff)), format: Format::Flac }
            }
            Some(FileType::Vorbis) => {
                let vf = VorbisFile::read_from(&mut f, raw).map_err(bad)?;
                Doc { inner: Inner::Vorbis(vf.vorbis_comments().clone()), format: Format::OggVorbis }
            }
            Some(FileType::Opus) => {
                let of = OpusFile::read_from(&mut f, raw).map_err(bad)?;
                Doc { inner: Inner::Vorbis(of.vorbis_comments().clone()), format: Format::Opus }
            }
            Some(FileType::Mpeg) => {
                let mf = MpegFile::read_from(&mut f, opts).map_err(bad)?;
                let (tag, v23) = match mf.id3v2() {
                    Some(t) => (t.clone(), t.original_version() == Id3v2Version::V3),
                    None => (Id3v2Tag::new(), false),
                };
                Doc { inner: Inner::Id3 { tag, v23 }, format: Format::Mp3 }
            }
            Some(FileType::Mp4) => {
                let mf = Mp4File::read_from(&mut f, opts).map_err(bad)?;
                Doc { inner: Inner::Mp4(mf.ilst().cloned().unwrap_or_default()), format: Format::Mp4 }
            }
            _ => return Err(AppError::Invalid(format!("{}: not a FLAC, MP3, M4A or Ogg file", path.display()))),
        })
    }

    /// "2.3" or "2.4" for an MP3.
    pub fn id3_version(&self) -> Option<&'static str> {
        match &self.inner {
            Inner::Id3 { v23, .. } => Some(if *v23 { "2.3" } else { "2.4" }),
            _ => None,
        }
    }

    fn vorbis(&self) -> Option<&VorbisComments> {
        match &self.inner {
            Inner::Flac(f) => f.vorbis_comments(),
            Inner::Vorbis(v) => Some(v),
            _ => None,
        }
    }

    fn vorbis_mut(&mut self) -> Option<&mut VorbisComments> {
        match &mut self.inner {
            Inner::Flac(f) => f.vorbis_comments_mut(),
            Inner::Vorbis(v) => Some(v),
            _ => None,
        }
    }

    /// Every field the file has a value for.
    pub fn values(&self, ctx: &Ctx) -> Values {
        Field::ALL.iter().filter_map(|f| self.get(*f, ctx).filter(|v| !v.trim().is_empty()).map(|v| (*f, v))).collect()
    }

    /// Every tag name the file carries, upper case (Vorbis keys, ID3 frame
    /// ids with TXXX descriptions as `TXXX:NAME`, MP4 atoms with freeform
    /// names as `----:NAME`).
    pub fn keys(&self) -> Vec<String> {
        let mut out: Vec<String> = match &self.inner {
            Inner::Flac(_) | Inner::Vorbis(_) => {
                self.vorbis().map(|v| v.items().map(|(k, _)| k.to_ascii_uppercase()).collect()).unwrap_or_default()
            }
            Inner::Id3 { tag, .. } => tag
                .into_iter()
                .map(|fr| match fr {
                    Frame::UserText(t) => format!("TXXX:{}", t.description.to_ascii_uppercase()),
                    other => other.id_str().to_string(),
                })
                .collect(),
            Inner::Mp4(ilst) => ilst
                .into_iter()
                .map(|a| match a.ident() {
                    AtomIdent::Fourcc(c) => String::from_utf8_lossy(c).to_string(),
                    AtomIdent::Freeform { name, .. } => format!("----:{}", name.to_ascii_uppercase()),
                })
                .collect(),
        };
        out.dedup();
        out
    }

    pub fn get(&self, f: Field, ctx: &Ctx) -> Option<String> {
        match &self.inner {
            Inner::Flac(_) | Inner::Vorbis(_) => self.vorbis().and_then(|v| vorbis_get(v, f, ctx)),
            Inner::Id3 { tag, .. } => id3_get(tag, f, ctx),
            Inner::Mp4(ilst) => mp4_get(ilst, f, ctx),
        }
    }

    /// Sets a field; an empty value removes it. `v` is already cleaned
    /// ([`super::fields::clean_value`]).
    pub fn set(&mut self, f: Field, v: &str, ctx: &Ctx) -> AppResult<()> {
        if f == Field::CustomStatus && ctx.custom_key.is_none() {
            return Err(AppError::Invalid("No custom release status tag is set in Settings.".into()));
        }
        match &mut self.inner {
            Inner::Flac(_) | Inner::Vorbis(_) => {
                if let Some(vc) = self.vorbis_mut() {
                    vorbis_set(vc, f, v, ctx);
                }
                Ok(())
            }
            Inner::Id3 { tag, v23 } => id3_set(tag, *v23, f, v, ctx),
            Inner::Mp4(ilst) => {
                mp4_set(ilst, f, v, ctx);
                Ok(())
            }
        }
    }

    /// Writes the tags into the file at `path` (a copy of the original:
    /// see `write`). Only the tag this format keeps is written.
    pub fn save(&self, path: &Path) -> AppResult<()> {
        let mut file = std::fs::OpenOptions::new().read(true).write(true).open(path).map_err(|e| io_err(path, e))?;
        let opts = WriteOptions::new();
        let r = match &self.inner {
            // The whole FLAC file, so its pictures are written back too.
            Inner::Flac(ff) => ff.save_to(&mut file, opts),
            Inner::Vorbis(vc) => vc.save_to(&mut file, opts),
            Inner::Id3 { tag, v23 } => {
                let mut o = opts;
                tag.save_to(&mut file, o.use_id3v23(*v23))
            }
            Inner::Mp4(ilst) => ilst.save_to(&mut file, opts),
        };
        r.map_err(|e| io_err(path, e))?;
        file.sync_all().map_err(|e| io_err(path, e))
    }
}

// ------------------------------------------------------------------ Vorbis

fn vorbis_all(v: &VorbisComments, key: &str) -> Option<String> {
    let all: Vec<&str> = v.get_all(key).map(str::trim).filter(|s| !s.is_empty()).collect();
    (!all.is_empty()).then(|| all.join("; "))
}

/// A key's value, replacing every value it had, in the place the first
/// one was (or at the end). Empty removes it.
fn vorbis_put(v: &mut VorbisComments, key: &str, value: &str) {
    let items: Vec<(String, String)> = v.take_items().collect();
    let mut done = false;
    let mut out = Vec::with_capacity(items.len() + 1);
    for (k, old) in items {
        if k.eq_ignore_ascii_case(key) {
            if !done && !value.is_empty() {
                out.push((k, value.to_string()));
            }
            done = true;
        } else {
            out.push((k, old));
        }
    }
    if !done && !value.is_empty() {
        out.push((key.to_string(), value.to_string()));
    }
    for (k, val) in out {
        v.push(k, val);
    }
}

fn has(v: &VorbisComments, key: &str) -> bool {
    v.get(key).is_some()
}

/// `n/total` split.
fn split_slash(s: &str) -> (String, Option<String>) {
    match s.split_once('/') {
        Some((a, b)) => (a.trim().to_string(), Some(b.trim().to_string()).filter(|b| !b.is_empty())),
        None => (s.trim().to_string(), None),
    }
}

/// Number and total keys for track or disc.
fn number_keys(disc: bool) -> (&'static str, [&'static str; 2]) {
    if disc { ("DISCNUMBER", ["DISCTOTAL", "TOTALDISCS"]) } else { ("TRACKNUMBER", ["TRACKTOTAL", "TOTALTRACKS"]) }
}

fn vorbis_get(v: &VorbisComments, f: Field, ctx: &Ctx) -> Option<String> {
    match f {
        Field::TrackNumber | Field::DiscNumber | Field::TrackTotal | Field::DiscTotal => {
            let disc = matches!(f, Field::DiscNumber | Field::DiscTotal);
            let (num_key, total_keys) = number_keys(disc);
            let (n, t) = v.get(num_key).map(split_slash).unwrap_or_default();
            if matches!(f, Field::TrackNumber | Field::DiscNumber) {
                return Some(n).filter(|n| !n.is_empty());
            }
            total_keys.iter().find_map(|k| v.get(k).map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).or(t)
        }
        Field::Explicit => {
            let adv = v.get("ITUNESADVISORY").map(str::trim);
            match adv {
                Some("1" | "4") => Some("explicit".into()),
                Some("2") => Some("clean".into()),
                _ => match v.get("EXPLICIT").map(|s| s.trim().to_ascii_lowercase()).as_deref() {
                    Some("1" | "true" | "yes" | "explicit") => Some("explicit".into()),
                    Some("0" | "false" | "no" | "clean") => Some("clean".into()),
                    _ => None,
                },
            }
        }
        Field::CustomStatus => ctx.custom_key.as_deref().and_then(|k| vorbis_all(v, k)),
        Field::ReleaseType => vorbis_all(v, "RELEASETYPE").or_else(|| vorbis_all(v, "MUSICBRAINZ_ALBUMTYPE")),
        Field::PicardStatus => vorbis_all(v, "RELEASESTATUS").or_else(|| vorbis_all(v, "MUSICBRAINZ_ALBUMSTATUS")),
        Field::AlbumArtist => {
            vorbis_all(v, "ALBUMARTIST").or_else(|| vorbis_all(v, "ALBUM ARTIST")).or_else(|| vorbis_all(v, "ALBUM_ARTIST"))
        }
        Field::Year => vorbis_all(v, "DATE").or_else(|| vorbis_all(v, "YEAR")),
        _ => f.vorbis_key().and_then(|k| vorbis_all(v, k)),
    }
}

fn vorbis_set(v: &mut VorbisComments, f: Field, val: &str, ctx: &Ctx) {
    match f {
        Field::TrackNumber | Field::DiscNumber => {
            let (num_key, total_keys) = number_keys(f == Field::DiscNumber);
            let (_, total) = v.get(num_key).map(split_slash).unwrap_or_default();
            let separate = total_keys.iter().any(|k| has(v, k));
            let out = match total {
                // The file keeps the total in the number tag: keep it there.
                Some(t) if !val.is_empty() && !separate => format!("{val}/{t}"),
                _ => val.to_string(),
            };
            vorbis_put(v, num_key, &out);
        }
        Field::TrackTotal | Field::DiscTotal => {
            let (num_key, total_keys) = number_keys(f == Field::DiscTotal);
            let num = v.get(num_key).map(split_slash);
            let separate = total_keys.iter().find(|k| has(v, k)).copied();
            match (separate, num) {
                (Some(k), _) => vorbis_put(v, k, val),
                (None, Some((n, Some(_)))) if !n.is_empty() => {
                    let out = if val.is_empty() { n } else { format!("{n}/{val}") };
                    vorbis_put(v, num_key, &out);
                }
                (None, Some((n, None))) if ctx.slash_numbers && !n.is_empty() && !val.is_empty() => {
                    vorbis_put(v, num_key, &format!("{n}/{val}"))
                }
                _ => vorbis_put(v, total_keys[0], val),
            }
        }
        Field::Explicit => {
            let explicit = val == "explicit";
            let uses_explicit = has(v, "EXPLICIT");
            if uses_explicit {
                vorbis_put(v, "EXPLICIT", if val.is_empty() { "" } else if explicit { "1" } else { "0" });
            }
            if !uses_explicit || has(v, "ITUNESADVISORY") {
                vorbis_put(v, "ITUNESADVISORY", if val.is_empty() { "" } else if explicit { "1" } else { "2" });
            }
        }
        Field::CustomStatus => {
            if let Some(k) = &ctx.custom_key {
                vorbis_put(v, k, val);
            }
        }
        _ => {
            // A file that keeps the field under another name keeps it there.
            let alts: &[&str] = match f {
                Field::ReleaseType => &["RELEASETYPE", "MUSICBRAINZ_ALBUMTYPE"],
                Field::PicardStatus => &["RELEASESTATUS", "MUSICBRAINZ_ALBUMSTATUS"],
                Field::AlbumArtist => &["ALBUMARTIST", "ALBUM ARTIST", "ALBUM_ARTIST"],
                Field::Year => &["DATE", "YEAR"],
                _ => &[],
            };
            let key = alts.iter().find(|k| has(v, k)).copied().or(f.vorbis_key());
            if let Some(k) = key {
                vorbis_put(v, k, val);
            }
        }
    }
}

// ------------------------------------------------------------------ ID3v2

fn fid(id: &'static str) -> FrameId<'static> {
    FrameId::Valid(Cow::Borrowed(id))
}

fn id3_text(tag: &Id3v2Tag, id: &'static str) -> Option<String> {
    let all: Vec<&str> = tag.get_texts(&fid(id))?.map(str::trim).filter(|s| !s.is_empty()).collect();
    (!all.is_empty()).then(|| all.join("; "))
}

fn enc(v23: bool) -> TextEncoding {
    // ID3v2.3 has no UTF-8.
    if v23 { TextEncoding::UTF16 } else { TextEncoding::UTF8 }
}

fn id3_put_text(tag: &mut Id3v2Tag, v23: bool, id: &'static str, val: &str) {
    if val.is_empty() {
        let _ = tag.remove(&fid(id)).count();
    } else {
        tag.insert(Frame::Text(TextInformationFrame::new(fid(id), enc(v23), val.to_string())));
    }
}

/// The TXXX description the file already uses for `name` (any case).
fn txxx_name(tag: &Id3v2Tag, name: &str) -> Option<String> {
    tag.into_iter().find_map(|fr| match fr {
        Frame::UserText(t) if t.description.eq_ignore_ascii_case(name) => Some(t.description.to_string()),
        _ => None,
    })
}

fn txxx_get(tag: &Id3v2Tag, name: &str) -> Option<String> {
    let d = txxx_name(tag, name)?;
    tag.get_user_text(&d).map(|s| s.replace('\0', "; ").trim().to_string()).filter(|s| !s.is_empty())
}

fn txxx_put(tag: &mut Id3v2Tag, v23: bool, name: &str, val: &str) {
    let d = txxx_name(tag, name);
    if val.is_empty() {
        if let Some(d) = d {
            tag.remove_user_text(&d);
        }
        return;
    }
    let d = d.unwrap_or_else(|| name.to_string());
    tag.insert(Frame::UserText(ExtendedTextFrame::new(enc(v23), d, val.to_string())));
}

/// TXXX names a field may be kept under, the usual one first.
fn txxx_names(f: Field, ctx: &Ctx) -> Vec<String> {
    match f {
        Field::ReleaseType => vec!["MusicBrainz Album Type".into(), "RELEASETYPE".into()],
        Field::PicardStatus => vec!["MusicBrainz Album Status".into(), "RELEASESTATUS".into()],
        Field::CustomStatus => ctx.custom_key.iter().cloned().collect(),
        _ => f.vorbis_key().map(|k| vec![k.to_string()]).unwrap_or_default(),
    }
}

fn id3_frame(f: Field) -> Option<&'static str> {
    Some(match f {
        Field::AlbumArtist => "TPE2",
        Field::Album => "TALB",
        Field::Genre => "TCON",
        Field::Title => "TIT2",
        Field::Artist => "TPE1",
        Field::Isrc => "TSRC",
        _ => return None,
    })
}

fn id3_get(tag: &Id3v2Tag, f: Field, ctx: &Ctx) -> Option<String> {
    if let Some(id) = id3_frame(f) {
        return id3_text(tag, id);
    }
    match f {
        Field::Year => tag.date().map(|t| t.to_string()),
        Field::TrackNumber | Field::TrackTotal | Field::DiscNumber | Field::DiscTotal => {
            let id = if matches!(f, Field::DiscNumber | Field::DiscTotal) { "TPOS" } else { "TRCK" };
            let (n, t) = split_slash(&id3_text(tag, id)?);
            if matches!(f, Field::TrackNumber | Field::DiscNumber) { Some(n).filter(|n| !n.is_empty()) } else { t }
        }
        Field::Explicit => match txxx_get(tag, "ITUNESADVISORY").as_deref().map(str::trim) {
            Some("1" | "4") => Some("explicit".into()),
            Some("2") => Some("clean".into()),
            _ => match txxx_get(tag, "EXPLICIT").map(|s| s.to_ascii_lowercase()).as_deref() {
                Some("1" | "true" | "yes" | "explicit") => Some("explicit".into()),
                Some("0" | "false" | "no" | "clean") => Some("clean".into()),
                _ => None,
            },
        },
        _ => txxx_names(f, ctx).iter().find_map(|n| txxx_get(tag, n)),
    }
}

fn id3_set(tag: &mut Id3v2Tag, v23: bool, f: Field, val: &str, ctx: &Ctx) -> AppResult<()> {
    if let Some(id) = id3_frame(f) {
        id3_put_text(tag, v23, id, val);
        return Ok(());
    }
    match f {
        Field::Year => {
            if val.is_empty() {
                tag.remove_date();
            } else {
                let t = Timestamp::from_str(val).map_err(|e| AppError::Invalid(format!("Year: {e}")))?;
                tag.set_date(t);
            }
        }
        Field::TrackNumber | Field::TrackTotal | Field::DiscNumber | Field::DiscTotal => {
            let disc = matches!(f, Field::DiscNumber | Field::DiscTotal);
            let id = if disc { "TPOS" } else { "TRCK" };
            let (n, t) = id3_text(tag, id).map(|s| split_slash(&s)).unwrap_or_default();
            let (n, t) = if matches!(f, Field::TrackNumber | Field::DiscNumber) {
                (val.to_string(), t)
            } else {
                (n, Some(val.to_string()).filter(|v| !v.is_empty()))
            };
            let out = match (n.is_empty(), t) {
                (true, _) => String::new(),
                (false, Some(t)) => format!("{n}/{t}"),
                (false, None) => n,
            };
            id3_put_text(tag, v23, id, &out);
        }
        Field::Explicit => {
            let explicit = val == "explicit";
            let uses_explicit = txxx_name(tag, "EXPLICIT").is_some();
            if uses_explicit {
                txxx_put(tag, v23, "EXPLICIT", if val.is_empty() { "" } else if explicit { "1" } else { "0" });
            }
            if !uses_explicit || txxx_name(tag, "ITUNESADVISORY").is_some() {
                txxx_put(tag, v23, "ITUNESADVISORY", if val.is_empty() { "" } else if explicit { "1" } else { "2" });
            }
        }
        _ => {
            let names = txxx_names(f, ctx);
            // Every name the file uses for it, else the usual one.
            let used: Vec<String> = names.iter().filter(|n| txxx_name(tag, n).is_some()).cloned().collect();
            let targets = if used.is_empty() { names.into_iter().take(1).collect() } else { used };
            for n in targets {
                txxx_put(tag, v23, &n, val);
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ MP4

fn fourcc(f: Field) -> Option<[u8; 4]> {
    Some(match f {
        Field::AlbumArtist => *b"aART",
        Field::Album => *b"\xa9alb",
        Field::Year => *b"\xa9day",
        Field::Genre => *b"\xa9gen",
        Field::Title => *b"\xa9nam",
        Field::Artist => *b"\xa9ART",
        _ => return None,
    })
}

fn atom_text(a: &Atom<'_>) -> Option<String> {
    let all: Vec<String> = a
        .data()
        .filter_map(|d| match d {
            AtomData::UTF8(s) | AtomData::UTF16(s) => Some(s.trim().to_string()),
            _ => None,
        })
        .filter(|s| !s.is_empty())
        .collect();
    (!all.is_empty()).then(|| all.join("; "))
}

/// The freeform name the file already uses for `name` (any case).
fn freeform_name(ilst: &Ilst, name: &str) -> Option<(String, String)> {
    ilst.into_iter().find_map(|a| match a.ident() {
        AtomIdent::Freeform { mean, name: n } if n.eq_ignore_ascii_case(name) => Some((mean.to_string(), n.to_string())),
        _ => None,
    })
}

fn freeform_get(ilst: &Ilst, name: &str) -> Option<String> {
    let (mean, n) = freeform_name(ilst, name)?;
    ilst.get(&AtomIdent::Freeform { mean: Cow::Owned(mean), name: Cow::Owned(n) }).and_then(atom_text)
}

fn freeform_put(ilst: &mut Ilst, name: &str, val: &str) {
    let (mean, n) = freeform_name(ilst, name).unwrap_or_else(|| (ITUNES.to_string(), name.to_string()));
    let ident = AtomIdent::Freeform { mean: Cow::Owned(mean), name: Cow::Owned(n) };
    if val.is_empty() {
        let _ = ilst.remove(&ident).count();
    } else {
        ilst.replace_atom(Atom::new(ident, AtomData::UTF8(val.to_string())));
    }
}

fn mp4_get(ilst: &Ilst, f: Field, ctx: &Ctx) -> Option<String> {
    if let Some(c) = fourcc(f) {
        return ilst.get(&AtomIdent::Fourcc(c)).and_then(atom_text);
    }
    match f {
        Field::TrackNumber => ilst.track().map(|n| n.to_string()),
        Field::TrackTotal => ilst.track_total().map(|n| n.to_string()),
        Field::DiscNumber => ilst.disk().map(|n| n.to_string()),
        Field::DiscTotal => ilst.disk_total().map(|n| n.to_string()),
        Field::Explicit => match ilst.advisory_rating() {
            Some(AdvisoryRating::Explicit) => Some("explicit".into()),
            Some(AdvisoryRating::Clean) => Some("clean".into()),
            _ => None,
        },
        _ => txxx_names(f, ctx).iter().find_map(|n| freeform_get(ilst, n)),
    }
}

fn mp4_set(ilst: &mut Ilst, f: Field, val: &str, ctx: &Ctx) {
    if let Some(c) = fourcc(f) {
        let ident = AtomIdent::Fourcc(c);
        if val.is_empty() {
            let _ = ilst.remove(&ident).count();
        } else {
            ilst.replace_atom(Atom::new(ident, AtomData::UTF8(val.to_string())));
        }
        if f == Field::Genre {
            // A numbered genre would sit beside the text one.
            let _ = ilst.remove(&AtomIdent::Fourcc(*b"gnre")).count();
        }
        return;
    }
    let num = val.parse::<u32>().ok();
    match (f, num) {
        (Field::TrackNumber, Some(n)) => ilst.set_track(n),
        (Field::TrackNumber, None) => ilst.remove_track(),
        (Field::TrackTotal, Some(n)) => ilst.set_track_total(n),
        (Field::TrackTotal, None) => ilst.remove_track_total(),
        (Field::DiscNumber, Some(n)) => ilst.set_disk(n),
        (Field::DiscNumber, None) => ilst.remove_disk(),
        (Field::DiscTotal, Some(n)) => ilst.set_disk_total(n),
        (Field::DiscTotal, None) => ilst.remove_disk_total(),
        (Field::Explicit, _) => match val {
            // One byte, as iTunes writes it (lofty's own setter writes four).
            "explicit" | "clean" => {
                let r = if val == "explicit" { AdvisoryRating::Explicit } else { AdvisoryRating::Clean };
                let data = AtomData::Unknown { code: DataType::BeSignedInteger, data: vec![r.as_u8()] };
                ilst.replace_atom(Atom::new(AtomIdent::Fourcc(*b"rtng"), data));
            }
            _ => {
                let _ = ilst.remove(&AtomIdent::Fourcc(*b"rtng")).count();
            }
        },
        _ => {
            let names = txxx_names(f, ctx);
            let used: Vec<String> = names.iter().filter(|n| freeform_name(ilst, n).is_some()).cloned().collect();
            let targets = if used.is_empty() { names.into_iter().take(1).collect() } else { used };
            for n in targets {
                freeform_put(ilst, &n, val);
            }
        }
    }
}
