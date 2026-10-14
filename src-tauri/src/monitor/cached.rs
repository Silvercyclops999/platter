//! Which tracks are on this computer, so the UI can dim the rest while only
//! cached music can play (server offline, files unavailable, signed out by
//! the server). Read from the audio pre-cache directory.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use tauri::State;

use crate::error::AppResult;

/// Managed state: the audio cache directory.
pub struct AudioCacheDir(pub PathBuf);

/// Track id from a cache file name (`<id>-<hash>.audio`, see
/// `audio::cache::cache_key`).
pub fn track_id_of(file_name: &str) -> Option<&str> {
    let stem = file_name.strip_suffix(".audio")?;
    let (id, hash) = stem.rsplit_once('-')?;
    (!id.is_empty() && hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(id)
}

pub fn cached_ids(dir: &Path) -> Vec<String> {
    let mut out = BTreeSet::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if let Some(name) = e.file_name().to_str()
            && let Some(id) = track_id_of(name)
        {
            out.insert(id.to_string());
        }
    }
    out.into_iter().collect()
}

/// `cached_tracks` → ids of tracks that can play without the server.
#[tauri::command]
pub async fn cached_tracks(dir: State<'_, AudioCacheDir>) -> AppResult<Vec<String>> {
    let d = dir.0.clone();
    Ok(tokio::task::spawn_blocking(move || cached_ids(&d))
        .await
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::cache::{DiskCache, cache_key};
    use crate::audio::testutil::track as t;

    fn track(id: &str, part: &str) -> crate::model::Track {
        t(id, "A", part)
    }

    #[test]
    fn lists_ids_the_player_cache_wrote() {
        // Written through the player's own cache, so a naming change there
        // breaks this test rather than silently un-dimming everything.
        let d = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(d.path(), 1 << 30).unwrap();
        for (id, part) in [("101", "/library/parts/1/2/file.flac"), ("202", "/library/parts/3/4/file.mp3")] {
            cache.insert_bytes(&cache_key(&track(id, part)), b"x").unwrap();
        }
        std::fs::write(d.path().join("303-0000000000000000.part"), b"x").unwrap();
        assert_eq!(cached_ids(d.path()), vec!["101".to_string(), "202".to_string()]);
    }

    #[test]
    fn ignores_strange_names() {
        assert_eq!(track_id_of("12-0123456789abcdef.audio"), Some("12"));
        assert_eq!(track_id_of("12.audio"), None);
        assert_eq!(track_id_of("-0123456789abcdef.audio"), None);
        assert_eq!(track_id_of("12-xyz.audio"), None);
        assert!(cached_ids(Path::new("does/not/exist")).is_empty());
    }
}
