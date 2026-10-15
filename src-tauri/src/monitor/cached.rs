//! Which tracks are on this computer, so the UI can dim the rest while only
//! cached music can play (server offline, files unavailable, signed out by
//! the server). Read from the audio pre-cache directory and the downloads.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::db::DbHandle;
use crate::db::store::{ALBUM_COLS, row_album};
use crate::error::AppResult;
use crate::model::Album;

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
            out.insert(crate::sid::resolve(&crate::sid::from_file_part(id)));
        }
    }
    out.into_iter().collect()
}

/// `cached_tracks` → ids of tracks that can play without the server.
/// Pre-cached and downloaded ids together, sorted.
async fn on_this_computer(dir: &AudioCacheDir, downloads: &crate::downloads::Downloads) -> Vec<String> {
    let d = dir.0.clone();
    let mut ids: BTreeSet<String> = tokio::task::spawn_blocking(move || cached_ids(&d))
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    ids.extend(downloads.downloaded_ids());
    ids.into_iter().collect()
}

#[tauri::command]
pub async fn cached_tracks(
    dir: State<'_, AudioCacheDir>,
    downloads: State<'_, crate::downloads::Downloads>,
) -> AppResult<Vec<String>> {
    Ok(on_this_computer(&dir, &downloads).await)
}

/// What Home can still offer while only cached music plays: the albums,
/// artists and playlists with at least one song on this computer.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedLibrary {
    /// Most recently played first, then by title.
    pub albums: Vec<Album>,
    pub artist_ids: Vec<String>,
    pub playlist_ids: Vec<String>,
}

pub fn cached_library_of(c: &Connection, track_ids: &[String]) -> AppResult<CachedLibrary> {
    let mut album_ids = BTreeSet::new();
    let mut artist_ids = BTreeSet::new();
    let mut playlist_ids = BTreeSet::new();
    let mut track = c.prepare_cached("SELECT album_id, artist_id FROM tracks WHERE id = ?1")?;
    let mut lists = c.prepare_cached("SELECT DISTINCT playlist_id FROM playlist_items WHERE track_id = ?1")?;
    for id in track_ids {
        if let Some((album, artist)) = track
            .query_row([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .optional()?
        {
            album_ids.insert(album);
            artist_ids.insert(artist);
        }
        for p in lists.query_map([id], |r| r.get::<_, String>(0))? {
            playlist_ids.insert(p?);
        }
    }
    let mut st = c.prepare_cached(&format!("SELECT {ALBUM_COLS} FROM albums al WHERE al.id = ?1"))?;
    let mut albums = Vec::with_capacity(album_ids.len());
    for id in &album_ids {
        if let Some(a) = st.query_row([id], row_album).optional()? {
            albums.push(a);
        }
    }
    albums.sort_by(|a, b| {
        b.last_viewed_at
            .unwrap_or(0)
            .cmp(&a.last_viewed_at.unwrap_or(0))
            .then_with(|| a.sort_title.to_lowercase().cmp(&b.sort_title.to_lowercase()))
    });
    Ok(CachedLibrary {
        albums,
        artist_ids: artist_ids.into_iter().collect(),
        playlist_ids: playlist_ids.into_iter().collect(),
    })
}

/// `cached_library` → the albums, artists and playlists that can still play
/// without the server.
#[tauri::command]
pub async fn cached_library(
    dir: State<'_, AudioCacheDir>,
    downloads: State<'_, crate::downloads::Downloads>,
    db: State<'_, DbHandle>,
) -> AppResult<CachedLibrary> {
    let ids = on_this_computer(&dir, &downloads).await;
    db.read_async(move |c| cached_library_of(c, &ids)).await
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
    fn cached_library_lists_what_still_plays() {
        use crate::db::fake;
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("t.db")).unwrap();
        let ar = fake::artist("ar", "Artist");
        let other = fake::artist("ar2", "Other");
        let a1 = fake::album("al1", "Kept", &ar, 2001, None, &[]);
        let a2 = fake::album("al2", "Gone", &other, 2002, None, &[]);
        let tracks = [fake::track("t1", "One", &a1, 1, 1), fake::track("t2", "Two", &a2, 1, 1)];
        db.write(|c| {
            let tx = c.transaction()?;
            crate::db::store::upsert_albums(&tx, &[a1.clone(), a2.clone()], 1)?;
            crate::db::store::upsert_tracks(&tx, &tracks, 1)?;
            tx.execute("INSERT INTO playlists (id, title) VALUES ('p1', 'Mix'), ('p2', 'Other mix')", [])?;
            tx.execute(
                "INSERT INTO playlist_items (playlist_id, position, track_id) VALUES ('p1', 0, 't1'), ('p2', 0, 't2')",
                [],
            )?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let lib = db.read(|c| cached_library_of(c, &["t1".into(), "missing".into()])).unwrap();
        assert_eq!(lib.albums.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["al1"]);
        assert_eq!(lib.artist_ids, vec!["ar".to_string()]);
        assert_eq!(lib.playlist_ids, vec!["p1".to_string()]);
        assert_eq!(db.read(|c| cached_library_of(c, &[])).unwrap(), CachedLibrary::default());
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
