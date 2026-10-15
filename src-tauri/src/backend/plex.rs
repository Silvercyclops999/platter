//! The Plex account as a music server. It wraps the Plex client code that
//! was already behind the app's narrow interfaces; every request it sends is
//! the one those sent (see the request recording tests). One `PlexServer`
//! covers every Plex server on the account: `PlexState` picks the server
//! for each item from its tag.

use std::sync::Arc;

use async_trait::async_trait;

use super::server::MusicServer;
use super::{BackendKind, Caps};
use crate::audio::source::TrackResolver;
use crate::bridge::{PlexArt, PlexLibrary, PlexSearch, PlexStreams, StreamPrefs};
use crate::db::DbHandle;
use crate::db::source::{ArtFetcher, LibrarySource, RemoteSearch};
use crate::downloads::DownloadSource;
use crate::likes::LikeRemote;
use crate::playlists::PlaylistRemote;
use crate::plex::PlexState;
use crate::plex::types::ConnectionKind;
use crate::tags::scan::PartSource;
use crate::tracking::PlayReporter;

/// Everything Platter uses from Plex. Plex has no way to save a queue for
/// later on the server, to date a play, or to make radio of its own that
/// Platter uses.
pub fn plex_caps() -> Caps {
    Caps::all() - Caps::RESUME_QUEUE - Caps::BACKDATED_SCROBBLE - Caps::SERVER_RADIO
}

pub struct PlexServer {
    plex: PlexState,
    library: Arc<PlexLibrary>,
    search: Arc<PlexSearch>,
    art: Arc<PlexArt>,
    streams: Arc<PlexStreams>,
    downloads: Arc<crate::downloads::commands::PlexSource>,
    parts: Arc<crate::tags::commands::PlexParts>,
    likes: Arc<crate::likes::PlexRatings>,
    playlists: Arc<crate::playlists::PlexPlaylists>,
    mbids: Arc<crate::artistinfo::service::PlexMbid>,
    lyrics: Arc<crate::lyrics::commands::PlexSource>,
    files: Arc<crate::monitor::files::PlexProbe>,
    file_paths: Arc<crate::lidarr::PlexMusicFiles>,
    edit: Arc<crate::plexedit::PlexEditServers>,
    remote: Arc<crate::remote::commands::PlexRemoteEnv>,
    /// Need the library cache (its music section).
    smart: Option<Arc<crate::smart::commands::PlexSmart>>,
    activity: Option<Arc<crate::social::commands::PlexActivity>>,
}

impl PlexServer {
    /// `prefs` are the streaming quality caps from Settings; `db` keeps
    /// loudness and measured silence, and names the music section smart
    /// playlists and friend activity use (without it those are off).
    pub fn new(plex: PlexState, prefs: Arc<StreamPrefs>, db: Option<DbHandle>) -> Self {
        Self {
            lyrics: Arc::new(crate::lyrics::commands::PlexSource(plex.clone())),
            files: Arc::new(crate::monitor::files::PlexProbe(plex.clone())),
            file_paths: Arc::new(crate::lidarr::PlexMusicFiles(plex.clone())),
            edit: Arc::new(crate::plexedit::PlexEditServers(plex.clone())),
            remote: Arc::new(crate::remote::commands::PlexRemoteEnv::new(plex.clone())),
            smart: db
                .clone()
                .map(|db| Arc::new(crate::smart::commands::PlexSmart { plex: plex.clone(), db })),
            activity: db
                .clone()
                .map(|db| Arc::new(crate::social::commands::PlexActivity { plex: plex.clone(), db })),
            library: Arc::new(PlexLibrary::new(plex.clone())),
            search: Arc::new(PlexSearch(plex.clone())),
            art: Arc::new(PlexArt(plex.clone())),
            streams: Arc::new(PlexStreams(plex.clone(), prefs, db)),
            downloads: Arc::new(crate::downloads::commands::PlexSource(plex.clone())),
            parts: Arc::new(crate::tags::commands::PlexParts(plex.clone())),
            likes: Arc::new(crate::likes::PlexRatings(plex.clone())),
            playlists: Arc::new(crate::playlists::PlexPlaylists(plex.clone())),
            mbids: Arc::new(crate::artistinfo::service::PlexMbid(plex.clone())),
            plex,
        }
    }

    pub fn state(&self) -> &PlexState {
        &self.plex
    }
}

#[async_trait]
impl MusicServer for PlexServer {
    fn kind(&self) -> BackendKind {
        BackendKind::Plex
    }

    fn caps(&self) -> Caps {
        let mut caps = plex_caps();
        if self.smart.is_none() {
            caps = caps - Caps::SERVER_SMART_PLAYLISTS;
        }
        if self.activity.is_none() {
            caps = caps - Caps::FRIEND_ACTIVITY - Caps::FRIEND_HISTORY;
        }
        caps
    }

    async fn tag(&self) -> Option<String> {
        self.plex.tag().await
    }

    async fn name(&self) -> Option<String> {
        self.plex.auth_status().await.server.map(|s| s.name).filter(|n| !n.trim().is_empty())
    }

    async fn reach(&self) -> Option<ConnectionKind> {
        self.plex.client().await.ok()?.connection()
    }

    async fn server_id(&self) -> Option<String> {
        self.plex.machine_id().await
    }

    fn library(&self) -> Arc<dyn LibrarySource> {
        self.library.clone()
    }

    fn search(&self) -> Arc<dyn RemoteSearch> {
        self.search.clone()
    }

    fn art(&self) -> Arc<dyn ArtFetcher> {
        self.art.clone()
    }

    fn streams(&self) -> Arc<dyn TrackResolver> {
        self.streams.clone()
    }

    fn downloads(&self) -> Arc<dyn DownloadSource> {
        self.downloads.clone()
    }

    fn parts(&self) -> Arc<dyn PartSource> {
        self.parts.clone()
    }

    fn likes(&self) -> Arc<dyn LikeRemote> {
        self.likes.clone()
    }

    fn playlists(&self) -> Arc<dyn PlaylistRemote> {
        self.playlists.clone()
    }

    fn plays(&self) -> Arc<dyn PlayReporter> {
        Arc::new(self.plex.clone())
    }

    fn mbids(&self) -> Arc<dyn crate::artistinfo::service::MbidLookup> {
        self.mbids.clone()
    }

    fn lyrics(&self) -> Option<Arc<dyn crate::lyrics::ServerLyrics>> {
        Some(self.lyrics.clone())
    }

    fn smart(&self) -> Option<Arc<dyn crate::smart::SmartRemote>> {
        self.smart.clone().map(|s| s as Arc<dyn crate::smart::SmartRemote>)
    }

    fn activity(&self) -> Option<Arc<dyn crate::social::activity::ActivitySource>> {
        self.activity.clone().map(|a| a as Arc<dyn crate::social::activity::ActivitySource>)
    }

    fn files(&self) -> Option<Arc<dyn crate::monitor::files::FileProbe>> {
        Some(self.files.clone())
    }

    fn file_paths(&self) -> Option<Arc<dyn crate::lidarr::PlexMusic>> {
        Some(self.file_paths.clone())
    }

    fn edit(&self) -> Option<Arc<dyn crate::plexedit::EditServers>> {
        Some(self.edit.clone())
    }

    fn remote(&self) -> Option<Arc<dyn crate::remote::RemoteEnv>> {
        Some(self.remote.clone())
    }
}
