//! In-memory [`LibrarySource`] for tests.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::error::{AppError, AppResult};
use crate::model::{Album, Artist, Playlist, Track};

use super::release::release_section;
use super::source::{Changes, HistoryEntry, ItemKind, LibrarySource, MusicSection, SimilarArtist};

#[derive(Default, Clone)]
pub struct FakeLibrary {
    pub sections: Vec<MusicSection>,
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<(Playlist, Vec<String>)>,
    pub history: Vec<HistoryEntry>,
    /// id → updatedAt (unix s); missing = 0.
    pub updated: HashMap<String, i64>,
    /// Playlists whose items the server can't list (answers 500).
    pub broken_playlists: Vec<String>,
    /// Artist id -> `Similar` from artist detail.
    pub similar: HashMap<String, Vec<SimilarArtist>>,
}

#[derive(Default)]
pub struct FakeSource {
    pub lib: Mutex<FakeLibrary>,
    pub page_calls: Mutex<Vec<(ItemKind, u64, u64)>>,
    pub playlist_item_calls: Mutex<Vec<String>>,
    pub fail_tracks_at: Mutex<Option<u64>>,
    /// Artist ids asked for similar artists, in order.
    pub similar_calls: Mutex<Vec<String>>,
    /// Make `artist_similar` fail (server offline).
    pub fail_similar: Mutex<bool>,
}

impl FakeSource {
    pub fn new(lib: FakeLibrary) -> Self {
        Self { lib: Mutex::new(lib), ..Default::default() }
    }
}

fn page<T: Clone>(v: &[T], start: u64, size: u64) -> Vec<T> {
    v.iter().skip(start as usize).take(size as usize).cloned().collect()
}

#[async_trait]
impl LibrarySource for FakeSource {
    async fn music_sections(&self) -> AppResult<Vec<MusicSection>> {
        Ok(self.lib.lock().sections.clone())
    }

    async fn count(&self, _s: &str, kind: ItemKind) -> AppResult<u64> {
        let l = self.lib.lock();
        Ok(match kind {
            ItemKind::Artist => l.artists.len(),
            ItemKind::Album => l.albums.len(),
            ItemKind::Track => l.tracks.len(),
        } as u64)
    }

    async fn artists(&self, _s: &str, start: u64, size: u64) -> AppResult<Vec<Artist>> {
        self.page_calls.lock().push((ItemKind::Artist, start, size));
        Ok(page(&self.lib.lock().artists, start, size))
    }

    async fn albums(&self, _s: &str, start: u64, size: u64) -> AppResult<Vec<Album>> {
        self.page_calls.lock().push((ItemKind::Album, start, size));
        Ok(page(&self.lib.lock().albums, start, size))
    }

    async fn tracks(&self, _s: &str, start: u64, size: u64) -> AppResult<Vec<Track>> {
        self.page_calls.lock().push((ItemKind::Track, start, size));
        if *self.fail_tracks_at.lock() == Some(start) {
            return Err(AppError::Network("connection reset".into()));
        }
        Ok(page(&self.lib.lock().tracks, start, size))
    }

    async fn changed_since(&self, _s: &str, since: i64) -> AppResult<Changes> {
        let l = self.lib.lock();
        let changed = |id: &str| l.updated.get(id).copied().unwrap_or(0) >= since;
        Ok(Changes {
            artists: l.artists.iter().filter(|a| changed(&a.id)).cloned().collect(),
            albums: l.albums.iter().filter(|a| changed(&a.id)).cloned().collect(),
            tracks: l.tracks.iter().filter(|t| changed(&t.id)).cloned().collect(),
        })
    }

    async fn all_ids(&self, _s: &str, kind: ItemKind) -> AppResult<Vec<String>> {
        let l = self.lib.lock();
        Ok(match kind {
            ItemKind::Artist => l.artists.iter().map(|a| a.id.clone()).collect(),
            ItemKind::Album => l.albums.iter().map(|a| a.id.clone()).collect(),
            ItemKind::Track => l.tracks.iter().map(|t| t.id.clone()).collect(),
        })
    }

    async fn history(&self, _s: &str, start: u64, size: u64) -> AppResult<Vec<HistoryEntry>> {
        Ok(page(&self.lib.lock().history, start, size))
    }

    async fn playlists(&self) -> AppResult<Vec<Playlist>> {
        Ok(self.lib.lock().playlists.iter().map(|(p, _)| p.clone()).collect())
    }

    async fn playlist_items(&self, id: &str) -> AppResult<Vec<String>> {
        self.playlist_item_calls.lock().push(id.to_owned());
        let l = self.lib.lock();
        if l.broken_playlists.iter().any(|b| b == id) {
            return Err(AppError::Server("500".into()));
        }
        l.playlists
            .iter()
            .find(|(p, _)| p.id == id)
            .map(|(_, items)| items.clone())
            .ok_or_else(|| AppError::NotFound(id.into()))
    }

    async fn artist_similar(&self, artist_id: &str) -> AppResult<Vec<SimilarArtist>> {
        self.similar_calls.lock().push(artist_id.to_owned());
        if *self.fail_similar.lock() {
            return Err(AppError::Network("offline".into()));
        }
        Ok(self.lib.lock().similar.get(artist_id).cloned().unwrap_or_default())
    }
}

// ---------------------------------------------------------------- builders

pub fn section(id: &str, uuid: &str) -> MusicSection {
    MusicSection { id: id.into(), uuid: uuid.into(), title: "Music".into() }
}

pub fn artist(id: &str, title: &str) -> Artist {
    Artist {
        id: id.into(),
        title: title.into(),
        sort_title: title.into(),
        thumb: Some(format!("/library/metadata/{id}/thumb/1")),
        art: None,
        view_count: 0,
        summary: None,
    }
}

pub fn album(id: &str, title: &str, artist: &Artist, year: i32, format: Option<&str>, subs: &[&str]) -> Album {
    let subformats: Vec<String> = subs.iter().map(|s| s.to_lowercase()).collect();
    Album {
        id: id.into(),
        title: title.into(),
        sort_title: title.into(),
        artist_id: artist.id.clone(),
        artist_title: artist.title.clone(),
        year: Some(year),
        thumb: Some(format!("/library/metadata/{id}/thumb/1")),
        format: format.map(str::to_lowercase),
        section: release_section(format, &subformats),
        subformats,
        genres: vec!["Rock".into()],
        track_count: 0,
        added_at: 1_600_000_000 + year as i64,
        view_count: 0,
        last_viewed_at: None,
        tint: None,
    }
}

pub fn track(id: &str, title: &str, album: &Album, disc: u32, n: u32) -> Track {
    Track {
        id: id.into(),
        title: title.into(),
        album_id: album.id.clone(),
        album_title: album.title.clone(),
        artist_id: album.artist_id.clone(),
        artist_title: album.artist_title.clone(),
        track_artist: None,
        track_number: n,
        disc_number: disc,
        duration_ms: 200_000,
        thumb: album.thumb.clone(),
        part_key: format!("/library/parts/{id}/1/file.flac"),
        codec: Some("flac".into()),
        container: Some("flac".into()),
        bitrate: Some(900),
        bit_depth: None,
        sample_rate: None,
        user_rating: None,
        view_count: 0,
        last_viewed_at: None,
        rating_count: None,
    }
}

pub fn playlist(id: &str, title: &str, n: u32) -> Playlist {
    Playlist {
        id: id.into(),
        title: title.into(),
        smart: false,
        track_count: n,
        duration_ms: n as u64 * 200_000,
        thumb: None,
        summary: None,
        updated_at: None,
    }
}

/// A generated library: `n_artists` × `albums_per` × `tracks_per`.
pub fn generated(n_artists: usize, albums_per: usize, tracks_per: usize) -> FakeLibrary {
    let mut lib = FakeLibrary { sections: vec![section("1", "uuid-a")], ..Default::default() };
    let mut next = 1000u64;
    let mut id = || {
        next += 1;
        next.to_string()
    };
    for a in 0..n_artists {
        let ar = artist(&id(), &format!("Artist {a}"));
        for b in 0..albums_per {
            let mut al = album(&id(), &format!("Album {a}-{b}"), &ar, 1990 + b as i32, Some("album"), &[]);
            al.track_count = tracks_per as u32;
            for t in 0..tracks_per {
                lib.tracks.push(track(&id(), &format!("Song {a}-{b}-{t}"), &al, 1, t as u32 + 1));
            }
            lib.albums.push(al);
        }
        lib.artists.push(ar);
    }
    lib
}
