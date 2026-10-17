//! Cover art for the OS: copied from the art cache to a small local file the
//! OS can read. The OS never gets a server URL (those carry the token).

use std::path::{Path, PathBuf};

use crate::db::ArtCache;
use crate::error::AppResult;

/// Largest picture side Platter decodes; covers are far smaller.
pub const MAX_DECODE_SIDE: u32 = 8192;
/// Most memory one picture's decode may take.
pub const MAX_DECODE_ALLOC: u64 = 128 * 1024 * 1024;

/// Decode a picture from a file's tags or a server, with limits on its size
/// and memory, so a crafted picture can't take all memory (or panic the
/// decoder and end the app).
pub fn decode_limited(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_SIDE);
    limits.max_image_height = Some(MAX_DECODE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let run = || {
        let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
        r.limits(limits);
        r.decode().map_err(|e| e.to_string())
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err("the picture looks damaged".into()))
}

/// Size requested from the art cache for the OS controls.
pub const COVER_PX: u32 = 512;
const PREFIX: &str = "cover-";

fn ext_for(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "jpg",
    }
}

fn name_for(thumb: &str, ext: &str) -> String {
    // FNV-1a: a stable name per image, so a repeat play reuses the file.
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in thumb.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{PREFIX}{h:016x}.{ext}")
}

/// The cover for `thumb` as a local file in `dir`, fetched through the art
/// cache. Older cover files in `dir` are removed.
pub async fn cover_file(art: &ArtCache, dir: &Path, thumb: &str) -> AppResult<PathBuf> {
    let img = art.get_or_fetch(thumb, COVER_PX, COVER_PX).await?;
    let path = dir.join(name_for(thumb, ext_for(&img.content_type)));
    let dir = dir.to_path_buf();
    let out = path.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        if !out.exists() {
            let tmp = out.with_extension("tmp");
            std::fs::write(&tmp, &img.bytes)?;
            std::fs::rename(&tmp, &out)?;
        }
        for e in std::fs::read_dir(&dir)?.flatten() {
            let p = e.path();
            let ours = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(PREFIX));
            if ours && p != out {
                let _ = std::fs::remove_file(p);
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| crate::error::AppError::Internal(e.to_string()))??;
    Ok(path)
}

/// The form each OS wants for a local cover file. Windows takes `file://`
/// followed by the plain path; macOS takes a proper file URL.
pub fn cover_uri(path: &Path) -> String {
    if cfg!(target_os = "windows") {
        format!("file://{}", path.display())
    } else {
        url::Url::from_file_path(path)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ArtFetcher;
    use crate::db::source::ArtBytes;
    use crate::error::AppError;

    #[test]
    fn huge_or_broken_pictures_are_refused() {
        let png = |w: u32, h: u32| {
            let mut out = Vec::new();
            image::DynamicImage::ImageRgb8(image::RgbImage::new(w, h)).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
            out
        };
        assert!(decode_limited(&png(MAX_DECODE_SIDE + 1, 1)).is_err());
        assert_eq!(decode_limited(&png(64, 32)).unwrap().width(), 64);
        assert!(decode_limited(b"\x89PNG\r\n\x1a\nnot really").is_err());
    }
    use std::sync::Arc;

    struct Png;
    #[async_trait::async_trait]
    impl ArtFetcher for Png {
        async fn fetch(&self, path: &str, _: u32, _: u32) -> AppResult<ArtBytes> {
            if path.contains("missing") {
                return Err(AppError::NotFound("art".into()));
            }
            Ok(ArtBytes { bytes: b"\x89PNG....".to_vec().into(), content_type: "image/png".into() })
        }
    }

    #[tokio::test]
    async fn cover_is_a_local_file_never_a_network_url() {
        let dir = tempfile::tempdir().unwrap();
        let art = ArtCache::open(dir.path().join("art"), 1 << 30, Arc::new(Png)).unwrap();
        let out = dir.path().join("now-playing");
        let a = cover_file(&art, &out, "/library/metadata/1/thumb/9").await.unwrap();
        assert!(a.starts_with(&out));
        assert_eq!(a.extension().unwrap(), "png");
        assert_eq!(std::fs::read(&a).unwrap(), b"\x89PNG....");
        let uri = cover_uri(&a);
        assert!(uri.starts_with("file://"), "{uri}");
        assert!(!uri.contains("http") && !uri.contains("X-Plex-Token") && !uri.contains("/library/"), "{uri}");

        // The next track replaces the old file.
        let b = cover_file(&art, &out, "/library/metadata/2/thumb/9").await.unwrap();
        assert_ne!(a, b);
        assert!(!a.exists() && b.exists());
        // Same art again reuses the same name.
        assert_eq!(cover_file(&art, &out, "/library/metadata/2/thumb/9").await.unwrap(), b);
        // No art: an error, no file.
        assert!(cover_file(&art, &out, "/missing").await.is_err());
    }
}
