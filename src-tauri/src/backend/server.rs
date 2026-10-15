//! One music server as the app sees it: the narrow interfaces the rest of
//! the app already works against, in one place. Every server provides the
//! core ones; the rest are capabilities a server may not have.

use std::sync::Arc;

use async_trait::async_trait;

use super::{BackendKind, Caps};
use crate::audio::source::TrackResolver;
use crate::db::source::{ArtFetcher, LibrarySource, RemoteSearch};
use crate::downloads::DownloadSource;
use crate::likes::RatingRemote;
use crate::playlists::PlaylistRemote;
use crate::plex::types::ConnectionKind;
use crate::tags::scan::PartSource;
use crate::tracking::PlayReporter;

#[async_trait]
pub trait MusicServer: Send + Sync {
    fn kind(&self) -> BackendKind;

    /// What this server can do.
    fn caps(&self) -> Caps;

    /// The server's tag (`crate::sid`). For a Plex account, the connected
    /// server's. None while it isn't known.
    async fn tag(&self) -> Option<String>;

    /// The server's name as its owner set it.
    async fn name(&self) -> Option<String>;

    /// How the server is reached now; None when it can't be.
    async fn reach(&self) -> Option<ConnectionKind>;

    /// The server's own id, which share links and listen-along sessions
    /// hash (it never leaves this computer).
    async fn server_id(&self) -> Option<String>;

    /// The main library, for the sync engine.
    fn library(&self) -> Arc<dyn LibrarySource>;
    /// Server-side search while the first sync runs.
    fn search(&self) -> Arc<dyn RemoteSearch>;
    /// Resized artwork.
    fn art(&self) -> Arc<dyn ArtFetcher>;
    /// Streams for the player.
    fn streams(&self) -> Arc<dyn TrackResolver>;
    /// Files for downloads.
    fn downloads(&self) -> Arc<dyn DownloadSource>;
    /// File reads for the tag scan.
    fn parts(&self) -> Arc<dyn PartSource>;
    /// Like and unlike.
    fn likes(&self) -> Arc<dyn RatingRemote>;
    /// Playlist edits.
    fn playlists(&self) -> Arc<dyn PlaylistRemote>;
    /// Now playing and finished plays.
    fn plays(&self) -> Arc<dyn PlayReporter>;
    /// MusicBrainz ids the server matched.
    fn mbids(&self) -> Arc<dyn crate::artistinfo::service::MbidLookup>;
}
