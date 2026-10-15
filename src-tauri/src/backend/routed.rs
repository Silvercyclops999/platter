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
use crate::error::{AppError, AppResult};
use crate::likes::LikeRemote;
use crate::model::{Playlist, Track};
use crate::playlists::PlaylistRemote;
use crate::plex::types::ConnectionKind;
use crate::tags::scan::PartSource;
use crate::tracking::{PlayReport, PlayReporter};

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
impl LikeRemote for RoutedLikes {
    async fn online(&self) -> bool {
        match self.0.primary() {
            Ok(s) => s.likes().online().await,
            Err(_) => false,
        }
    }

    async fn set_liked(&self, track_id: &str, liked: bool) -> AppResult<()> {
        self.0.for_id(track_id)?.likes().set_liked(track_id, liked).await
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

    async fn timeline(&self, r: &PlayReport) -> AppResult<()> {
        self.0.for_id(&r.track_id)?.plays().timeline(r).await
    }

    async fn scrobble(&self, track_id: &str, at: Option<i64>) -> AppResult<()> {
        self.0.for_id(track_id)?.plays().scrobble(track_id, at).await
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

// ---------------------------------------------------------------- capabilities

const NO_SMART: &str = "This server can't keep smart playlists.";
const NO_ACTIVITY: &str = "This server doesn't share what others are playing.";
const NO_FILE_PATHS: &str =
    "Your server doesn't say where its files are, so Platter can't compare its folders with Lidarr's.";
const NO_EDIT: &str = "This server's details can't be edited from Platter.";
const NO_REMOTE: &str = "Playing on the server's other devices isn't available with this server.";

/// Lyrics from each track's server; none when it keeps no lyrics.
pub struct RoutedLyrics(pub Backends);

#[async_trait]
impl crate::lyrics::ServerLyrics for RoutedLyrics {
    async fn lyrics(&self, track_id: &str) -> AppResult<Option<crate::plex::details::Lyrics>> {
        match self.0.for_id(track_id)?.lyrics() {
            Some(l) => l.lyrics(track_id).await,
            None => Ok(None),
        }
    }
}

/// Smart playlists saved on the primary server.
pub struct RoutedSmart(pub Backends);

impl RoutedSmart {
    fn smart(&self) -> AppResult<std::sync::Arc<dyn crate::smart::SmartRemote>> {
        self.0.primary()?.smart().ok_or_else(|| AppError::Invalid(NO_SMART.into()))
    }
}

#[async_trait]
impl crate::smart::SmartRemote for RoutedSmart {
    async fn online(&self) -> bool {
        match self.smart() {
            Ok(s) => s.online().await,
            Err(_) => false,
        }
    }
    async fn target(&self) -> AppResult<(String, String)> {
        self.smart()?.target().await
    }
    async fn tags(&self, section_id: &str, plex_field: &str) -> AppResult<Vec<(String, String)>> {
        self.smart()?.tags(section_id, plex_field).await
    }
    async fn create(&self, title: &str, uri: &str) -> AppResult<String> {
        self.smart()?.create(title, uri).await
    }
    async fn update(&self, id: &str, uri: &str) -> AppResult<()> {
        self.smart()?.update(id, uri).await
    }
    async fn rename(&self, id: &str, title: &str) -> AppResult<()> {
        self.smart()?.rename(id, title).await
    }
    async fn content(&self, id: &str) -> AppResult<String> {
        self.smart()?.content(id).await
    }
    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)> {
        self.smart()?.fetch(id).await
    }
}

/// What others play on the primary server. Without the capability it reads
/// as not connected.
pub struct RoutedActivity(pub Backends);

impl RoutedActivity {
    fn source(&self) -> AppResult<std::sync::Arc<dyn crate::social::activity::ActivitySource>> {
        self.0.primary()?.activity().ok_or_else(|| AppError::Invalid(NO_ACTIVITY.into()))
    }
}

#[async_trait]
impl crate::social::activity::ActivitySource for RoutedActivity {
    async fn owned(&self) -> Option<bool> {
        self.source().ok()?.owned().await
    }
    async fn sessions(&self) -> AppResult<Vec<u8>> {
        self.source()?.sessions().await
    }
    async fn history(&self, size: u64) -> AppResult<Vec<u8>> {
        self.source()?.history(size).await
    }
    async fn accounts(&self) -> AppResult<Vec<u8>> {
        self.source()?.accounts().await
    }
}

/// Whether a track's file can be read, from its server (unknown when the
/// server can't say).
pub struct RoutedFiles(pub Backends);

#[async_trait]
impl crate::monitor::files::FileProbe for RoutedFiles {
    async fn readable(&self, track_id: &str) -> Option<bool> {
        self.0.for_id(track_id).ok()?.files()?.readable(track_id).await
    }
}

/// The primary server's folders and files, for the Lidarr check.
pub struct RoutedFilePaths(pub Backends);

#[async_trait]
impl crate::lidarr::PlexMusic for RoutedFilePaths {
    async fn music_files(&self) -> AppResult<crate::lidarr::check::PlexFiles> {
        let paths = self.0.primary()?.file_paths().ok_or_else(|| AppError::Invalid(NO_FILE_PATHS.into()))?;
        paths.music_files().await
    }
}

/// Editing on the item's server.
pub struct RoutedEdit(pub Backends);

#[async_trait]
impl crate::plexedit::EditServers for RoutedEdit {
    async fn client_for(&self, id: &str) -> AppResult<crate::plex::client::PlexClient> {
        let edit = self.0.for_id(id)?.edit().ok_or_else(|| AppError::Invalid(NO_EDIT.into()))?;
        edit.client_for(id).await
    }
    async fn owned(&self) -> (Vec<String>, bool) {
        match self.0.primary().ok().and_then(|s| s.edit()) {
            Some(e) => e.owned().await,
            None => (Vec::new(), false),
        }
    }
}

/// The primary server's other players (an item's own server for its play
/// queue).
pub struct RoutedRemote(pub Backends);

impl RoutedRemote {
    fn primary(&self) -> AppResult<std::sync::Arc<dyn crate::remote::RemoteEnv>> {
        self.0.primary()?.remote().ok_or_else(|| AppError::Invalid(NO_REMOTE.into()))
    }
}

#[async_trait]
impl crate::remote::RemoteEnv for RoutedRemote {
    async fn server(&self) -> AppResult<crate::remote::ServerCtx> {
        self.primary()?.server().await
    }
    async fn server_for(&self, id: &str) -> AppResult<crate::remote::ServerCtx> {
        let env = self.0.for_id(id)?.remote().ok_or_else(|| AppError::Invalid(NO_REMOTE.into()))?;
        env.server_for(id).await
    }
    async fn account_token(&self) -> Option<crate::plex::token::Token> {
        self.primary().ok()?.account_token().await
    }
    async fn account_id(&self) -> Option<String> {
        self.primary().ok()?.account_id().await
    }
}
