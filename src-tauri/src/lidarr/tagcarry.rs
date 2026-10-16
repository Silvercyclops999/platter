//! Carrying one file's tags onto another: an album upgrade puts the old
//! file's tags (every text tag, `ARCHIVE_*` and other custom ones included,
//! and its embedded pictures) onto the better copy, keeping only the new
//! copy's technical tags (encoder, ReplayGain), which describe its audio.
//!
//! Tags move between formats (MP3 to FLAC is the usual case) through one
//! neutral form: upper case Vorbis-style names. Custom tags travel as
//! themselves (a TXXX frame, a Vorbis comment, an MP4 freeform atom). Tags
//! only one format can hold (an ID3 rating frame, say) can't move to
//! another; then nothing is written and the copy fails with their names.
//!
//! After writing, the copy is read back and compared with the original:
//! anything missing or changed fails the copy, so a swap never goes ahead
//! on a file that lost a tag.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType};
use lofty::flac::FlacFile;
use lofty::id3::v2::{Frame, Id3v2Tag};
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OggPictureStorage, OpusFile, VorbisFile};
use lofty::picture::Picture;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, ItemValue, SplitTag, Tag, TagExt, TagItem, TagType};

use crate::error::{AppError, AppResult};

/// How a file keeps its tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// FLAC, Ogg Vorbis, Opus.
    Vorbis,
    /// MP3 (and AAC in ADTS).
    Id3,
    /// M4A / ALAC.
    Mp4,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Vorbis => "FLAC",
            Kind::Id3 => "MP3",
            Kind::Mp4 => "M4A",
        }
    }
}

/// A file's tags in the neutral form.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bag {
    /// Upper case name, value; a name may repeat.
    pub pairs: Vec<(String, String)>,
    pub pictures: Vec<Picture>,
    /// Frames or atoms only this format can hold.
    pub uncarried: Vec<String>,
}

/// The format's own tag, for a same-format copy that keeps everything.
enum Raw {
    Vorbis,
    Id3(Box<Id3v2Tag>),
    Mp4(Box<Ilst>),
}

/// The codec and quality a file's audio has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInfo {
    /// "flac", "mp3", "aac", "alac", "vorbis", "opus".
    pub codec: String,
    pub kbps: Option<u32>,
    pub bit_depth: Option<u32>,
    pub duration_ms: Option<u64>,
}

fn copy_err(why: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("The tags couldn't be copied: {why}."))
}

/// Tags that describe the encoding rather than the song: the new copy keeps its own.
pub fn technical(key: &str) -> bool {
    let k = key.trim().to_ascii_uppercase();
    matches!(k.as_str(), "ENCODER" | "ENCODERSETTINGS" | "ENCODER_SETTINGS" | "ENCODING" | "ITUNSMPB" | "ITUNNORM" | "ENCODERSOFTWARE")
        || k.starts_with("REPLAYGAIN_")
        || k.starts_with("R128_")
}

fn file_type(path: &Path) -> AppResult<FileType> {
    let cant = |e: &dyn std::fmt::Display| copy_err(format!("Platter couldn't open {} ({e})", show(path)));
    Probe::open(path)
        .map_err(|e| cant(&e))?
        .guess_file_type()
        .map_err(|e| cant(&e))?
        .file_type()
        .ok_or_else(|| copy_err(format!("{} isn't an audio file Platter knows", show(path))))
}

fn kind_of(t: FileType) -> AppResult<Kind> {
    match t {
        FileType::Flac | FileType::Vorbis | FileType::Opus => Ok(Kind::Vorbis),
        FileType::Mpeg | FileType::Aac => Ok(Kind::Id3),
        FileType::Mp4 => Ok(Kind::Mp4),
        other => Err(copy_err(format!("Platter can't carry tags in {other:?} files"))),
    }
}

fn show(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn open(path: &Path) -> AppResult<File> {
    File::open(path).map_err(|e| copy_err(format!("Platter couldn't open {} ({e})", show(path))))
}

fn parse_err<E: std::fmt::Display>(path: &Path) -> impl Fn(E) -> AppError + '_ {
    move |e| copy_err(format!("Platter couldn't read {} ({e})", show(path)))
}

/// A generic key's neutral name.
fn neutral(key: ItemKey) -> String {
    key.map_key(TagType::VorbisComments).map(str::to_ascii_uppercase).unwrap_or_else(|| format!("{key:?}").to_ascii_uppercase())
}

/// The generic key a neutral name maps to, when there is one.
fn generic(name: &str) -> Option<ItemKey> {
    ItemKey::from_key(TagType::VorbisComments, name)
}

fn from_generic(t: &Tag, bag: &mut Bag) {
    for item in t.items() {
        match item.value() {
            ItemValue::Text(v) | ItemValue::Locator(v) => bag.pairs.push((neutral(item.key()), v.clone())),
            ItemValue::Binary(_) => bag.uncarried.push(neutral(item.key())),
        }
    }
    bag.pictures.extend(t.pictures().iter().cloned());
}

fn read_raw(path: &Path) -> AppResult<(Kind, Bag, Raw)> {
    let t = file_type(path)?;
    let kind = kind_of(t)?;
    let opts = ParseOptions::new();
    let mut f = open(path)?;
    let mut bag = Bag::default();
    match t {
        FileType::Flac | FileType::Vorbis | FileType::Opus => {
            let (vc, extra): (Option<VorbisComments>, Vec<Picture>) = match t {
                FileType::Flac => {
                    let file = FlacFile::read_from(&mut f, opts).map_err(parse_err(path))?;
                    if file.id3v2().is_some_and(|i| !i.is_empty()) {
                        bag.uncarried.push("ID3".into());
                    }
                    (file.vorbis_comments().cloned(), file.pictures().iter().map(|(p, _)| p.clone()).collect())
                }
                FileType::Vorbis => (Some(VorbisFile::read_from(&mut f, opts).map_err(parse_err(path))?.vorbis_comments().clone()), vec![]),
                _ => (Some(OpusFile::read_from(&mut f, opts).map_err(parse_err(path))?.vorbis_comments().clone()), vec![]),
            };
            if let Some(vc) = vc {
                bag.pairs.extend(vc.items().map(|(k, v)| (k.to_ascii_uppercase(), v.to_string())));
                bag.pictures.extend(vc.pictures().iter().map(|(p, _)| p.clone()));
            }
            bag.pictures.extend(extra);
            Ok((kind, bag, Raw::Vorbis))
        }
        FileType::Mpeg | FileType::Aac => {
            let id3 = match t {
                FileType::Mpeg => {
                    let file = MpegFile::read_from(&mut f, opts).map_err(parse_err(path))?;
                    if file.ape().is_some_and(|a| !a.is_empty()) {
                        bag.uncarried.push("APE".into());
                    }
                    file.id3v2().cloned()
                }
                _ => lofty::aac::AacFile::read_from(&mut f, opts).map_err(parse_err(path))?.id3v2().cloned(),
            }
            .unwrap_or_default();
            let (rest, g) = id3.clone().split_tag();
            from_generic(&g, &mut bag);
            for frame in &*rest {
                match frame {
                    Frame::UserText(x) if !x.description.is_empty() => {
                        for v in x.content.split('\0') {
                            bag.pairs.push((x.description.to_ascii_uppercase(), v.to_string()));
                        }
                    }
                    other => bag.uncarried.push(other.id_str().to_string()),
                }
            }
            Ok((kind, bag, Raw::Id3(Box::new(id3))))
        }
        _ => {
            let file = Mp4File::read_from(&mut f, opts).map_err(parse_err(path))?;
            let ilst = file.ilst().cloned().unwrap_or_default();
            let (rest, g) = ilst.clone().split_tag();
            from_generic(&g, &mut bag);
            for atom in &*rest {
                match atom.ident() {
                    AtomIdent::Freeform { name, .. } => {
                        for d in atom.data() {
                            match d {
                                AtomData::UTF8(v) | AtomData::UTF16(v) => bag.pairs.push((name.to_ascii_uppercase(), v.clone())),
                                _ => bag.uncarried.push(format!("----:{name}")),
                            }
                        }
                    }
                    AtomIdent::Fourcc(f) => bag.uncarried.push(String::from_utf8_lossy(&f[..]).into_owned()),
                }
            }
            Ok((kind, bag, Raw::Mp4(Box::new(ilst))))
        }
    }
}

/// A file's tags in the neutral form.
pub fn read(path: &Path) -> AppResult<Bag> {
    read_raw(path).map(|(_, b, _)| b)
}

/// The audio's codec, bit rate and bit depth.
pub fn audio(path: &Path) -> AppResult<AudioInfo> {
    let t = file_type(path)?;
    let tf = lofty::read_from_path(path).map_err(parse_err(path))?;
    let p = tf.properties();
    let bit_depth = p.bit_depth().map(u32::from);
    let codec = match t {
        FileType::Flac => "flac",
        FileType::Mpeg => "mp3",
        FileType::Aac => "aac",
        FileType::Mp4 if bit_depth.is_some() => "alac",
        FileType::Mp4 => "aac",
        FileType::Vorbis => "vorbis",
        FileType::Opus => "opus",
        FileType::Wav => "wav",
        FileType::Aiff => "aiff",
        _ => "",
    };
    let ms = p.duration().as_millis() as u64;
    Ok(AudioInfo { codec: codec.into(), kbps: p.audio_bitrate().filter(|k| *k > 0), bit_depth, duration_ms: (ms > 0).then_some(ms) })
}

/// Pairs grouped by name, in first-seen order.
fn grouped(pairs: &[(String, String)]) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for (k, v) in pairs {
        match out.iter_mut().find(|(n, _)| n == k) {
            Some((_, vs)) => vs.push(v.clone()),
            None => out.push((k.clone(), vec![v.clone()])),
        }
    }
    out
}

fn generic_tag(ty: TagType, pairs: &[(String, String)], pictures: &[Picture]) -> (Tag, Vec<(String, Vec<String>)>) {
    let mut t = Tag::new(ty);
    let mut custom = Vec::new();
    for (k, vs) in grouped(pairs) {
        match generic(&k).filter(|g| g.map_key(ty).is_some()) {
            Some(g) => {
                for v in vs {
                    t.push_unchecked(TagItem::new(g, ItemValue::Text(v)));
                }
            }
            None => custom.push((k, vs)),
        }
    }
    for p in pictures {
        t.push_picture(p.clone());
    }
    (t, custom)
}

fn save(tag: &impl TagExt, path: &Path) -> AppResult<()> {
    tag.save_to_path(path, WriteOptions::default()).map_err(|e| copy_err(format!("Platter couldn't write {} ({e})", show(path))))
}

/// Remove every tag Platter doesn't write itself (ID3v1, APE, an ID3v2
/// tag in front of a FLAC file).
fn strip_extra(path: &Path, kind: Kind) {
    let extra: &[TagType] = match kind {
        Kind::Vorbis => &[TagType::Id3v2],
        Kind::Id3 => &[TagType::Id3v1, TagType::Ape],
        Kind::Mp4 => &[],
    };
    for t in extra {
        let _ = t.remove_from_path(path, WriteOptions::default());
    }
}

fn write(path: &Path, kind: Kind, pairs: &[(String, String)], pictures: &[Picture], same: Option<Raw>) -> AppResult<()> {
    strip_extra(path, kind);
    match (kind, same) {
        (Kind::Vorbis, _) => {
            let mut vc = VorbisComments::new();
            for (k, v) in pairs {
                vc.push(k.clone(), v.clone());
            }
            for p in pictures {
                vc.insert_picture(p.clone(), None).map_err(|e| copy_err(format!("a picture couldn't be copied ({e})")))?;
            }
            save(&vc, path)
        }
        (Kind::Id3, same) => {
            let tech: Vec<(String, String)> = pairs.iter().filter(|(k, _)| technical(k)).cloned().collect();
            let mut id3 = match same {
                // Same format: the old tag as it is, all its frames, minus its technical ones.
                Some(Raw::Id3(old)) => {
                    let mut old = *old;
                    old.retain(|f| !frame_technical(f));
                    old
                }
                _ => {
                    let (t, custom) = generic_tag(TagType::Id3v2, pairs, pictures);
                    let mut id3 = Id3v2Tag::from(t);
                    for (k, vs) in custom.into_iter().filter(|(k, _)| !technical(k)) {
                        id3.insert_user_text(k, vs.join("\0"));
                    }
                    id3
                }
            };
            let (t, custom) = generic_tag(TagType::Id3v2, &tech, &[]);
            for f in Id3v2Tag::from(t) {
                id3.insert(f);
            }
            for (k, vs) in custom {
                id3.insert_user_text(k, vs.join("\0"));
            }
            save(&id3, path)
        }
        (Kind::Mp4, same) => {
            let tech: Vec<(String, String)> = pairs.iter().filter(|(k, _)| technical(k)).cloned().collect();
            let mut ilst = match same {
                Some(Raw::Mp4(old)) => {
                    let mut old = *old;
                    old.retain(|a| !atom_technical(a));
                    old
                }
                _ => {
                    let (t, custom) = generic_tag(TagType::Mp4Ilst, pairs, pictures);
                    let mut ilst = Ilst::from(t);
                    for (k, vs) in custom.into_iter().filter(|(k, _)| !technical(k)) {
                        add_freeform(&mut ilst, &k, vs);
                    }
                    ilst
                }
            };
            let (t, custom) = generic_tag(TagType::Mp4Ilst, &tech, &[]);
            for a in Ilst::from(t) {
                ilst.insert(a);
            }
            for (k, vs) in custom {
                add_freeform(&mut ilst, &k, vs);
            }
            save(&ilst, path)
        }
    }
}

fn add_freeform(ilst: &mut Ilst, name: &str, vs: Vec<String>) {
    let ident = AtomIdent::Freeform { mean: "com.apple.iTunes".into(), name: name.to_string().into() };
    if let Some(a) = Atom::from_collection(ident, vs.into_iter().map(AtomData::UTF8).collect()) {
        ilst.insert(a);
    }
}

fn frame_technical(f: &Frame<'_>) -> bool {
    match f {
        Frame::UserText(x) => technical(&x.description),
        other => matches!(other.id_str(), "TSSE" | "RVA2" | "RVAD"),
    }
}

fn atom_technical(a: &Atom<'_>) -> bool {
    match a.ident() {
        AtomIdent::Freeform { name, .. } => technical(name),
        AtomIdent::Fourcc(f) => f == b"\xa9too",
    }
}

fn count(pairs: &[(String, String)]) -> HashMap<(String, String), usize> {
    let mut m = HashMap::new();
    for (k, v) in pairs.iter().filter(|(k, _)| !technical(k)) {
        *m.entry((k.trim().to_ascii_uppercase(), v.trim().to_string())).or_insert(0) += 1;
    }
    m
}

/// Put `old`'s tags onto `new` (in place), keeping `new`'s technical tags,
/// then read `new` back to prove nothing was lost.
pub fn carry(old: &Path, new: &Path) -> AppResult<()> {
    let (old_kind, old_bag, old_raw) = read_raw(old)?;
    let (new_kind, new_bag, _) = read_raw(new)?;
    let same = old_kind == new_kind;
    if !same && !old_bag.uncarried.is_empty() {
        let mut names = old_bag.uncarried.clone();
        names.sort();
        names.dedup();
        return Err(copy_err(format!(
            "{} has tags only {} files can hold ({}), so they can't move to {}",
            show(old),
            old_kind.name(),
            names.join(", "),
            new_kind.name()
        )));
    }
    let mut pairs: Vec<(String, String)> = old_bag.pairs.iter().filter(|(k, _)| !technical(k)).cloned().collect();
    pairs.extend(new_bag.pairs.iter().filter(|(k, _)| technical(k)).cloned());
    write(new, new_kind, &pairs, &old_bag.pictures, same.then_some(old_raw))?;

    // Read back and compare.
    let after = read(new)?;
    let (want, got) = (count(&old_bag.pairs), count(&after.pairs));
    let mut missing: Vec<String> = want.iter().filter(|(k, n)| got.get(*k).copied().unwrap_or(0) < **n).map(|((k, _), _)| k.clone()).collect();
    missing.extend(got.iter().filter(|(k, n)| want.get(*k).copied().unwrap_or(0) < **n).map(|((k, _), _)| k.clone()));
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        return Err(copy_err(format!("{} came out different on the new copy", missing.join(", "))));
    }
    let pics = |b: &Bag| {
        let mut v: Vec<(u8, Vec<u8>)> = b.pictures.iter().map(|p| (p.pic_type().as_u8(), p.data().to_vec())).collect();
        v.sort();
        v
    };
    if pics(&old_bag) != pics(&after) {
        return Err(copy_err("the cover art came out different on the new copy"));
    }
    if same && old_bag.uncarried.len() != after.uncarried.len() {
        return Err(copy_err("some of its tags came out different on the new copy"));
    }
    Ok(())
}

/// Set one custom text tag (the provenance note) on a file, keeping every
/// other tag and picture, then read it back.
pub fn set_text(path: &Path, key: &str, value: &str) -> AppResult<()> {
    let key = key.trim().to_ascii_uppercase();
    let (kind, bag, raw) = read_raw(path)?;
    let mut pairs: Vec<(String, String)> = bag.pairs.into_iter().filter(|(k, _)| *k != key).collect();
    pairs.push((key.clone(), value.to_string()));
    let same = match (kind, raw) {
        (Kind::Id3, Raw::Id3(mut t)) => {
            t.remove_user_text(&key);
            t.insert_user_text(key.clone(), value.to_string());
            save(&*t, path)?;
            None
        }
        (Kind::Mp4, Raw::Mp4(mut t)) => {
            t.retain(|a| !matches!(a.ident(), AtomIdent::Freeform { name, .. } if name.eq_ignore_ascii_case(&key)));
            add_freeform(&mut t, &key, vec![value.to_string()]);
            save(&*t, path)?;
            None
        }
        (k, r) => Some((k, r)),
    };
    if let Some((k, _)) = same {
        write(path, k, &pairs, &bag.pictures, None)?;
    }
    let after = read(path)?;
    if !after.pairs.iter().any(|(k, v)| *k == key && v == value) {
        return Err(copy_err(format!("{key} didn't take on {}", show(path))));
    }
    Ok(())
}
