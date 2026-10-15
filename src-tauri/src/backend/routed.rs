//! The app's narrow interfaces, each sending a call to the server the item
//! belongs to ([`Backends::for_id`]). Calls about no item in particular
//! (is a server connected, search, the download connection) go to the
//! primary server. Every method of each interface is passed through, so a
//! server's own behaviour (defaults included) is what the app gets.

use async_trait::async_trait;

use super::Backends;
use crate::audio::dsp::TrackAudio;
use crate::audio::source::{StreamSource, TrackResolver};
use crate::db::source::{ArtBytes, ArtFetcher, PlaylistEntry, RemoteHits, RemoteSearch};
use crate::downloads::{DownloadSource, Fetch};
use crate::error::AppResult;
use crate::likes::RatingRemote;
use crate::model::{Playlist, Track};
use crate::playlists::PlaylistRemote;
use crate::plex::client::TimelineReport;
use crate::plex::types::ConnectionKind;
use crate::tags::scan::PartSource;
use crate::tracking::PlayReporter;

/// Server-side search on the primary server.
pub struct RoutedSearch(pub Backends);

#[async_trait]
impl RemoteSearch for RoutedSearch {
    async fn search(&self, query: &str, section_id: Option<&str>, limit: u32) -> AppResult<RemoteHits> {
        self.0.primary()?.search().search(query, section_id, limit).await
    }
}

/// Artwork from the server the image path names.
pub struct RoutedArt(pub Backends);

#[async_trait]
impl ArtFetcher for RoutedArt {
    async fn fetch(&self, path: &str, width: u32, height: u32) -> AppResult<ArtBytes> {
        self.0.for_id(path)?.art().fetch(path, width, height).await
    }
}

/// Streams from each track's server.
pub struct RoutedStreams(pub Backends);

#[async_trait]
impl TrackResolver for RoutedStreams {
    async fn stream(&self, track: &Track) -> AppResult<StreamSource> {
        self.0.for_id(&track.id)?.streams().stream(track).await
    }

    async fn transcode(&self, track: &Track) -> AppResult<StreamSource> {
        self.0.for_id(&track.id)?.streams().transcode(track).await
    }

    async fn audio_info(&self, track: &Track) -> Option<TrackAudio> {
        self.0.for_id(&track.id).ok()?.streams().audio_info(track).await
    }

    fn save_silence(&self, track_id: &str, head_ms: u64, tail_ms: u64) {
        if let Ok(s) = self.0.for_id(track_id) {
            s.streams().save_silence(track_id, head_ms, tail_ms);
        }
    }
}

/// Downloads from each track's server; the connection is the primary's.
pub struct RoutedDownloads(pub Backends);

#[async_trait]
impl DownloadSource for RoutedDownloads {
    async fn connection(&self) -> Option<ConnectionKind> {
        self.0.primary().ok()?.downloads().connection().await
    }

    async fn fetch(&self, track: &Track, kbps: Option<u32>) -> AppResult<Fetch> {
        self.0.for_id(&track.id)?.downloads().fetch(track, kbps).await
    }

    async fn finished(&self, fetch: &Fetch) {
        if let Ok(s) = self.0.primary() {
            s.downloads().finished(fetch).await;
        }
    }
}

/// File reads for the tag scan from each track's server.
pub struct RoutedParts(pub Backends);

#[async_trait]
impl PartSource for RoutedParts {
    async fn part(&self, track_id: &str, part_key: &str) -> AppResult<StreamSource> {
        self.0.for_id(track_id)?.parts().part(track_id, part_key).await
    }
}

/// Likes on each track's server.
pub struct RoutedLikes(pub Backends);

#[async_trait]
impl RatingRemote for RoutedLikes {
    async fn online(&self) -> bool {
        match self.0.primary() {
            Ok(s) => s.likes().online().await,
            Err(_) => false,
        }
    }

    async fn rate(&self, track_id: &str, rating: Option<f64>) -> AppResult<()> {
        self.0.for_id(track_id)?.likes().rate(track_id, rating).await
    }
}

/// Playlist edits on the playlist's server (a new one: its songs' server).
pub struct RoutedPlaylists(pub Backends);

#[async_trait]
impl PlaylistRemote for RoutedPlaylists {
    async fn online(&self) -> bool {
        match self.0.primary() {
            Ok(s) => s.playlists().online().await,
            Err(_) => false,
        }
    }

    async fn create(&self, title: &str, track_ids: &[String]) -> AppResult<String> {
        let server = match track_ids.first() {
            Some(first) => self.0.for_id(first)?,
            None => self.0.primary()?,
        };
        server.playlists().create(title, track_ids).await
    }

    async fn add(&self, id: &str, track_ids: &[String]) -> AppResult<()> {
        self.0.for_id(id)?.playlists().add(id, track_ids).await
    }

    async fn remove(&self, id: &str, item_id: &str) -> AppResult<()> {
        self.0.for_id(id)?.playlists().remove(id, item_id).await
    }

    async fn move_item(&self, id: &str, item_id: &str, after: Option<&str>) -> AppResult<()> {
        self.0.for_id(id)?.playlists().move_item(id, item_id, after).await
    }

    async fn update(&self, id: &str, title: Option<&str>, summary: Option<&str>) -> AppResult<()> {
        self.0.for_id(id)?.playlists().update(id, title, summary).await
    }

    async fn poster(&self, id: &str, bytes: Vec<u8>, content_type: &str) -> AppResult<()> {
        self.0.for_id(id)?.playlists().poster(id, bytes, content_type).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.0.for_id(id)?.playlists().delete(id).await
    }

    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)> {
        self.0.for_id(id)?.playlists().fetch(id).await
    }
}

/// Play reports to each track's server.
pub struct RoutedPlays(pub Backends);

#[async_trait]
impl PlayReporter for RoutedPlays {
    async fn online(&self) -> bool {
        match self.0.primary() {
            Ok(s) => s.plays().online().await,
            Err(_) => false,
        }
    }

    async fn timeline(&self, r: &TimelineReport) -> AppResult<()> {
        self.0.for_id(&r.rating_key)?.plays().timeline(r).await
    }

    async fn scrobble(&self, rating_key: &str) -> AppResult<()> {
        self.0.for_id(rating_key)?.plays().scrobble(rating_key).await
    }
}

/// MusicBrainz ids from each item's server.
pub struct RoutedMbids(pub Backends);

#[async_trait]
impl crate::artistinfo::service::MbidLookup for RoutedMbids {
    async fn artist_mbid(&self, artist_id: &str) -> AppResult<Option<String>> {
        self.0.for_id(artist_id)?.mbids().artist_mbid(artist_id).await
    }

    async fn album_mbid(&self, album_id: &str) -> AppResult<Option<String>> {
        self.0.for_id(album_id)?.mbids().album_mbid(album_id).await
    }
}

/// The primary server's own id (share links, listen-along sessions).
pub struct RoutedServerId(pub Backends);

#[async_trait]
impl crate::share::ServerId for RoutedServerId {
    async fn machine_id(&self) -> Option<String> {
        self.0.primary().ok()?.server_id().await
    }
}

#[async_trait]
impl crate::social::listen::ServerId for RoutedServerId {
    async fn machine_id(&self) -> Option<String> {
        self.0.primary().ok()?.server_id().await
    }
}
