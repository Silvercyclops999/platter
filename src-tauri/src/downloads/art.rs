//! Album art kept beside downloaded songs, so covers still show when the
//! artwork cache has let them go and the server can't be reached.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use super::Downloads;
use crate::db::ArtFetcher;
use crate::db::artcache::sniff_content_type;
use crate::db::source::ArtBytes;
use crate::error::AppResult;

/// Folder inside the downloads folder.
pub const ART_DIR: &str = "art";
/// The size kept (big enough for the album page header).
const SIZE: u32 = 640;

fn name(thumb: &str) -> String {
    // FNV-1a of the Plex art path (as kept before paths carried their server).
    let mut h: u64 = 0xcbf29ce484222325;
    for b in crate::sid::storage_form(thumb).as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}.img")
}

pub fn path(dir: &Path, thumb: &str) -> PathBuf {
    dir.join(ART_DIR).join(name(thumb))
}

/// Fetches and keeps the cover once.
pub async fn keep(fetcher: &dyn ArtFetcher, dir: &Path, thumb: &str) {
    let p = path(dir, thumb);
    if p.is_file() {
        return;
    }
    match fetcher.fetch(thumb, SIZE, SIZE).await {
        Ok(img) => {
            let _ = tokio::fs::create_dir_all(dir.join(ART_DIR)).await;
            if let Err(e) = tokio::fs::write(&p, &img.bytes).await {
                tracing::debug!("could not keep cover: {e}");
            }
        }
        Err(e) => tracing::debug!("no cover kept: {e}"),
    }
}

/// The art fetcher for the artwork cache: the usual one first, then a cover
/// kept with the downloads.
pub struct WithDownloads {
    pub inner: Arc<dyn ArtFetcher>,
    pub downloads: Downloads,
}

#[async_trait]
impl ArtFetcher for WithDownloads {
    async fn fetch(&self, art_path: &str, width: u32, height: u32) -> AppResult<ArtBytes> {
        match self.inner.fetch(art_path, width, height).await {
            Ok(b) => Ok(b),
            Err(e) => {
                let p = path(&self.downloads.dir(), art_path);
                match tokio::fs::read(&p).await {
                    Ok(bytes) => Ok(ArtBytes {
                        content_type: sniff_content_type(&bytes).to_string(),
                        bytes: bytes.into(),
                    }),
                    Err(_) => Err(e),
                }
            }
        }
    }
}
