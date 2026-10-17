//! Tags on converted files: every text tag the original has, in the new
//! format's own kind of tag (Vorbis comments for FLAC and Opus, ID3v2.3 for
//! MP3, which old players read best), the song's basics from the library
//! when the file lacks them, and one front cover made small enough for
//! small players.

use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use lofty::config::WriteOptions;
use lofty::id3::v2::Id3v2Tag;
use lofty::ogg::OggPictureStorage;
use lofty::ogg::tag::VorbisComments;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{ItemKey, ItemValue, Tag, TagExt, TagItem, TagType};

use crate::lidarr::tagcarry;
use crate::model::Track;

/// What kind of tag the file takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Vorbis,
    Id3,
}

/// Tags that describe the old encoding rather than the song.
fn encoding_tag(k: &str) -> bool {
    matches!(k, "ENCODER" | "ENCODERSETTINGS" | "ENCODER_SETTINGS" | "ENCODING" | "ITUNSMPB" | "ITUNNORM" | "ENCODERSOFTWARE")
}

/// The source's tags plus the library's basics where it has none.
pub fn pairs_for(src: &Path, t: &Track) -> (Vec<(String, String)>, Vec<Picture>) {
    let bag = tagcarry::read(src).unwrap_or_default();
    let mut pairs: Vec<(String, String)> = bag.pairs.into_iter().filter(|(k, _)| !encoding_tag(k)).collect();
    let has = |pairs: &[(String, String)], k: &str| pairs.iter().any(|(n, v)| n == k && !v.trim().is_empty());
    let artist = t.track_artist.clone().filter(|a| !a.trim().is_empty()).unwrap_or_else(|| t.artist_title.clone());
    let mut fill = vec![
        ("TITLE", t.title.clone()),
        ("ARTIST", artist),
        ("ALBUM", t.album_title.clone()),
        ("ALBUMARTIST", t.artist_title.clone()),
    ];
    if t.track_number > 0 {
        fill.push(("TRACKNUMBER", t.track_number.to_string()));
    }
    if t.disc_number > 0 {
        fill.push(("DISCNUMBER", t.disc_number.to_string()));
    }
    for (k, v) in fill {
        if !v.trim().is_empty() && !has(&pairs, k) {
            pairs.push((k.to_string(), v));
        }
    }
    (pairs, bag.pictures)
}

/// The front cover (or the first picture), resized to at most `px` and
/// stored as a JPEG. `fallback` (album art from the server) when the file
/// has no picture.
pub fn cover(pictures: &[Picture], fallback: Option<&[u8]>, px: u32) -> Option<Picture> {
    let data = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())
        .map(|p| p.data())
        .or(fallback)?;
    let small = shrink(data, px)?;
    Some(Picture::unchecked(small).pic_type(PictureType::CoverFront).mime_type(MimeType::Jpeg).build())
}

/// A JPEG no bigger than `px` on its longest side.
pub fn shrink(data: &[u8], px: u32) -> Option<Vec<u8>> {
    let img = crate::platform::art::decode_limited(data).ok()?;
    let img = if img.width().max(img.height()) > px { img.resize(px, px, image::imageops::FilterType::Triangle) } else { img };
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut out, 85)).ok()?;
    Some(out)
}

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

/// Writes the tags onto `dest`, replacing whatever tags it has.
pub fn write(dest: &Path, kind: Kind, pairs: &[(String, String)], cover: Option<Picture>) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| write_unguarded(dest, kind, pairs, cover)))
        .unwrap_or_else(|_| Err("the tags couldn't be written (the file looks damaged)".into()))
}

fn write_unguarded(dest: &Path, kind: Kind, pairs: &[(String, String)], cover: Option<Picture>) -> Result<(), String> {
    fn err(e: impl std::fmt::Display) -> String {
        format!("the tags couldn't be written ({e})")
    }
    match kind {
        Kind::Vorbis => {
            let mut vc = VorbisComments::new();
            for (k, v) in pairs {
                vc.push(k.clone(), v.clone());
            }
            if let Some(p) = cover {
                vc.insert_picture(p, None).map_err(err)?;
            }
            vc.save_to_path(dest, WriteOptions::default()).map_err(err)
        }
        Kind::Id3 => {
            let mut t = Tag::new(TagType::Id3v2);
            let mut custom = Vec::new();
            for (k, vs) in grouped(pairs) {
                match ItemKey::from_key(TagType::VorbisComments, &k).filter(|g| g.map_key(TagType::Id3v2).is_some()) {
                    Some(g) => {
                        for v in vs {
                            t.push_unchecked(TagItem::new(g, ItemValue::Text(v)));
                        }
                    }
                    None => custom.push((k, vs)),
                }
            }
            if let Some(p) = cover {
                t.push_picture(p);
            }
            let mut id3 = Id3v2Tag::from(t);
            for (k, vs) in custom {
                id3.insert_user_text(k, vs.join("\0"));
            }
            id3.save_to_path(dest, WriteOptions::new().use_id3v23(true)).map_err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn covers_shrink_to_a_small_jpeg() {
        let big = png(1400, 1000);
        let p = cover(&[], Some(&big), 500).unwrap();
        assert_eq!(p.pic_type(), PictureType::CoverFront);
        let img = image::load_from_memory(p.data()).unwrap();
        assert_eq!((img.width(), img.height()), (500, 357));
        assert_eq!(image::guess_format(p.data()).unwrap(), image::ImageFormat::Jpeg);
        // A small one keeps its size.
        let small = cover(&[], Some(&png(300, 300)), 500).unwrap();
        assert_eq!(image::load_from_memory(small.data()).unwrap().width(), 300);
        assert!(cover(&[], None, 500).is_none());
        assert!(cover(&[], Some(b"not an image"), 500).is_none());
    }
}
