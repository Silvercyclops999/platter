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
}

impl PlexServer {
    /// `prefs` are the streaming quality caps from Settings; `db` keeps
    /// loudness and measured silence.
    pub fn new(plex: PlexState, prefs: Arc<StreamPrefs>, db: Option<DbHandle>) -> Self {
        Self {
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
        plex_caps()
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
}
