//! Glue between the Plex client and the db layer: the `LibrarySource` and
//! `ArtFetcher` the sync engine and artwork cache pull from.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::OptionalExtension as _;

use crate::db::artcache::sniff_content_type;
use crate::db::source::{
    ArtBytes, Changes, HistoryEntry, ItemKind, MusicSection, PlaylistEntry, RemoteHits, RemoteSearch,
    SimilarArtist,
};
use crate::audio::source::{ContextKind, MoreRequest, PlayContext, ResolvedContext, StreamSource, TrackLookup, TrackResolver};
use crate::audio::dsp::TrackAudio;
use crate::db::{ArtFetcher, DbHandle, LibrarySource, queries, trackaudio};
use crate::error::AppResult;
use crate::model::{Album, Artist, Playlist, Track};
use crate::plex::client::{MetaType, PlexHub, PlexMetadata};
use crate::plex::PlexState;
use crate::plex::types::ConnectionKind;

/// How long startup requests wait for the saved Plex session to restore.
const STARTUP_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

const PAGE: u64 = 500;
/// Items asked for in one `/library/metadata/{k1},{k2},…` call (live updates).
const LIVE_CHUNK: usize = 50;

fn meta_type(kind: ItemKind) -> MetaType {
    match kind {
        ItemKind::Artist => MetaType::Artist,
        ItemKind::Album => MetaType::Album,
        ItemKind::Track => MetaType::Track,
    }
}

/// Release-type, style and mood tags per album id. The album list endpoint
/// doesn't include `Format`/`Subformat`/`Style`/`Mood`, so they're collected
/// in bulk with one filtered list per tag value and merged into each album
/// page.
#[derive(Default, Clone)]
struct AlbumTags {
    format: HashMap<String, String>,
    subformats: HashMap<String, Vec<String>>,
    styles: HashMap<String, Vec<String>>,
    moods: HashMap<String, Vec<String>>,
}

/// Tag fields read per value, and how many filtered lists run at once.
const TAG_FIELDS: [&str; 4] = ["format", "subformat", "style", "mood"];
const TAG_LISTS_AT_ONCE: usize = 4;

/// `tracks` in the order of `ids` (a sorted or filtered page). Each id
/// takes the next unused copy, so a song listed twice plays twice; ids not
/// in `tracks` are left out.
fn in_order(tracks: Vec<crate::model::Track>, ids: &[String]) -> Vec<crate::model::Track> {
    let mut pool: HashMap<String, std::collections::VecDeque<crate::model::Track>> = HashMap::new();
    for t in tracks {
        pool.entry(t.id.clone()).or_default().push_back(t);
    }
    ids.iter().filter_map(|id| pool.get_mut(id).and_then(|q| q.pop_front())).collect()
}

fn merge_tags(into: &mut Vec<String>, more: Option<&Vec<String>>) {
    for t in more.into_iter().flatten() {
        if !into.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            into.push(t.clone());
        }
    }
}

impl AlbumTags {
    fn apply(&self, album: &mut Album) {
        if let Some(f) = self.format.get(&album.id) {
            album.format = Some(f.clone());
        }
        if let Some(s) = self.subformats.get(&album.id) {
            album.subformats = s.clone();
        }
        album.section = crate::db::release_section(album.format.as_deref(), &album.subformats);
        merge_tags(&mut album.styles, self.styles.get(&album.id));
        merge_tags(&mut album.moods, self.moods.get(&album.id));
    }

    fn add(&mut self, field: &str, tag: &str, album_id: String) {
        match field {
            "format" => {
                self.format.insert(album_id, tag.to_lowercase());
            }
            "subformat" => self.subformats.entry(album_id).or_default().push(tag.to_lowercase()),
            // Styles and moods keep Plex's own spelling (they're shown as mix names).
            "style" => self.styles.entry(album_id).or_default().push(tag.to_string()),
            _ => self.moods.entry(album_id).or_default().push(tag.to_string()),
        }
    }
}

pub struct PlexLibrary {
    plex: PlexState,
    tags: Mutex<HashMap<String, Arc<AlbumTags>>>,
    /// Another included library: its server's tag and its section. None
    /// reads the connected server (its main library).
    other: Option<(String, String)>,
}

impl PlexLibrary {
    pub fn new(plex: PlexState) -> Self {
        Self {
            plex,
            tags: Mutex::new(HashMap::new()),
            other: None,
        }
    }

    /// Reads one library (`section`) of the account's server with `tag`.
    pub fn other(plex: PlexState, tag: &str, section: &str) -> Self {
        Self {
            plex,
            tags: Mutex::new(HashMap::new()),
            other: Some((tag.to_string(), section.to_string())),
        }
    }

    async fn client(&self) -> AppResult<crate::plex::client::PlexClient> {
        match &self.other {
            Some((tag, _)) => self.plex.client_for_tag(tag).await,
            None => self.plex.client().await,
        }
    }

    async fn load_tags(&self, section: &str) -> AppResult<Arc<AlbumTags>> {
        let client = self.client().await?;
        let mut lists = Vec::new();
        for field in TAG_FIELDS {
            for value in client.tag_values(section, field, MetaType::Album).await? {
                lists.push((field, value));
            }
        }
        // A few filtered lists at a time (one per tag value).
        let mut found: Vec<(&'static str, String, Vec<PlexMetadata>)> = Vec::with_capacity(lists.len());
        for chunk in lists.chunks(TAG_LISTS_AT_ONCE) {
            let mut calls = Vec::with_capacity(chunk.len());
            for (field, value) in chunk {
                let filter = [(*field, value.key.clone())];
                let client = client.clone();
                calls.push(Box::pin(async move { client.list_all(section, MetaType::Album, &filter).await }));
            }
            for ((field, value), albums) in chunk.iter().zip(futures::future::try_join_all(calls).await?) {
                found.push((field, value.title.clone(), albums));
            }
        }
        let mut tags = AlbumTags::default();
        for (field, tag, albums) in found {
            for a in albums {
                tags.add(field, &tag, a.rating_key);
            }
        }
        let tags = Arc::new(tags);
        self.tags.lock().insert(section.to_string(), tags.clone());
        Ok(tags)
    }

    /// Tags for a section, refreshed when `refresh` (start of a sync pass).
    async fn tags(&self, section: &str, refresh: bool) -> AppResult<Arc<AlbumTags>> {
        if !refresh
            && let Some(t) = self.tags.lock().get(section)
        {
            return Ok(t.clone());
        }
        self.load_tags(section).await
    }

    async fn changed(&self, section: &str, ty: MetaType, since: i64) -> AppResult<Vec<PlexMetadata>> {
        let client = self.client().await?;
        let mut out = Vec::new();
        let mut start = 0;
        loop {
            let page = client.updated_since(section, ty, since, start, PAGE).await?;
            let n = page.items.len() as u64;
            out.extend(page.items);
            start += n;
            if n == 0 || start >= page.total {
                break;
            }
        }
        // New items get updatedAt too, so this also covers additions.
        Ok(out)
    }
}

#[async_trait]
impl LibrarySource for PlexLibrary {
    async fn server_tag(&self) -> Option<String> {
        if let Some((tag, _)) = &self.other {
            return Some(tag.clone());
        }
        self.client().await.ok().and_then(|c| c.tag().map(str::to_string))
    }

    async fn music_sections(&self) -> AppResult<Vec<MusicSection>> {
        let client = self.client().await?;
        Ok(client
            .music_sections()
            .await?
            .into_iter()
            // Another library lists only itself.
            .filter(|s| self.other.as_ref().is_none_or(|(_, section)| *section == s.id))
            .map(|s| MusicSection {
                uuid: s.uuid.clone().unwrap_or_else(|| s.id.clone()),
                id: s.id,
                title: s.title,
            })
            .collect())
    }

    async fn count(&self, section_id: &str, kind: ItemKind) -> AppResult<u64> {
        self.client().await?.count(section_id, meta_type(kind)).await
    }

    async fn artists(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Artist>> {
        Ok(self.client().await?.artists(section_id, start, size).await?.items)
    }

    async fn albums(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Album>> {
        let tags = self.tags(section_id, start == 0).await?;
        let mut items = self.client().await?.albums(section_id, start, size).await?.items;
        for a in &mut items {
            tags.apply(a);
        }
        Ok(items)
    }

    async fn tracks(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Track>> {
        Ok(self.client().await?.tracks(section_id, start, size).await?.items)
    }

    async fn changed_since(&self, section_id: &str, since: i64) -> AppResult<Changes> {
        let artists = self.changed(section_id, MetaType::Artist, since).await?;
        let albums = self.changed(section_id, MetaType::Album, since).await?;
        let tracks = self.changed(section_id, MetaType::Track, since).await?;
        let tags = if albums.is_empty() {
            self.tags(section_id, false).await?
        } else {
            self.tags(section_id, true).await?
        };
        Ok(Changes {
            artists: artists.iter().map(PlexMetadata::to_artist).collect(),
            albums: albums
                .iter()
                .map(|m| {
                    let mut a = m.to_album();
                    tags.apply(&mut a);
                    a
                })
                .collect(),
            tracks: tracks.iter().map(PlexMetadata::to_track).collect(),
        })
    }

    async fn items(&self, section_id: &str, ids: &[String]) -> AppResult<Option<Changes>> {
        let client = self.client().await?;
        let mut found = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(LIVE_CHUNK) {
            found.extend(client.metadata_many(chunk).await?);
        }
        // Album detail has its own Format and Style tags; the bulk lists
        // (when loaded) add what detail leaves out.
        let tags = self.tags.lock().get(section_id).cloned();
        let mut out = Changes::default();
        for m in found {
            match m.kind.as_deref() {
                Some("artist") => out.artists.push(m.to_artist()),
                Some("album") => {
                    let mut a = m.to_album();
                    match &tags {
                        Some(t) => t.apply(&mut a),
                        None => a.section = crate::db::release_section(a.format.as_deref(), &a.subformats),
                    }
                    out.albums.push(a);
                }
                Some("track") => out.tracks.push(m.to_track()),
                _ => {}
            }
        }
        Ok(Some(out))
    }

    async fn all_ids(&self, section_id: &str, kind: ItemKind) -> AppResult<Vec<String>> {
        let items = self
            .plex
            .client()
            .await?
            .list_all(section_id, meta_type(kind), &[])
            .await?;
        Ok(items.into_iter().map(|m| m.rating_key).collect())
    }

    async fn history(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<HistoryEntry>> {
        let page = self.client().await?.history(section_id, start, size).await?;
        Ok(page
            .items
            .into_iter()
            .map(|h| HistoryEntry {
                track_id: h.rating_key,
                viewed_at: h.viewed_at,
            })
            .collect())
    }

    async fn playlists(&self) -> AppResult<Vec<Playlist>> {
        self.client().await?.playlists().await
    }

    async fn rated_tracks(&self, section_id: &str) -> AppResult<Option<Vec<crate::db::likes::Rated>>> {
        let rated = self.client().await?.rated_tracks(section_id).await?;
        Ok(Some(
            rated
                .into_iter()
                .map(|(track_id, rating, rated_at)| crate::db::likes::Rated { track_id, rating, rated_at })
                .collect(),
        ))
    }

    async fn artist_similar(&self, artist_id: &str) -> AppResult<Vec<SimilarArtist>> {
        let m = self.plex.client_for(artist_id).await?.metadata(artist_id).await?;
        Ok(m.similar_artists().into_iter().map(|(id, name)| SimilarArtist { id, name }).collect())
    }

    async fn playlist_entries(&self, playlist_id: &str) -> AppResult<Vec<PlaylistEntry>> {
        let rows = self.client().await?.playlist_rows(playlist_id).await?;
        Ok(rows.into_iter().map(|(track_id, item_id)| PlaylistEntry { track_id, item_id }).collect())
    }

    async fn playlist_items(&self, playlist_id: &str) -> AppResult<Vec<String>> {
        let client = self.client().await?;
        let mut out = Vec::new();
        let mut start = 0;
        loop {
            let page = client.playlist_items(playlist_id, start, PAGE).await?;
            let n = page.items.len() as u64;
            out.extend(page.items.into_iter().map(|t| t.id));
            start += n;
            if n == 0 || start >= page.total {
                break;
            }
        }
        Ok(out)
    }
}

/// Server-side search (`/hubs/search`), used until the first sync finishes.
pub struct PlexSearch(pub PlexState);

/// Sort Plex search hubs into kinds. Each item's own `type` wins; the hub's
/// type (or identifier) is the fallback.
pub(crate) fn hits_from_hubs(hubs: Vec<PlexHub>) -> RemoteHits {
    let mut out = RemoteHits::default();
    for hub in hubs {
        let hub_kind = hub.kind.clone().unwrap_or_else(|| hub.hub_identifier.clone());
        for m in hub.metadata {
            let kind = m.kind.clone().unwrap_or_else(|| hub_kind.clone());
            match kind.as_str() {
                "artist" => out.artists.push(m.to_artist()),
                "album" => out.albums.push(m.to_album()),
                "track" => out.tracks.push(m.to_track()),
                "playlist" if m.playlist_type.as_deref().is_none_or(|t| t == "audio") => {
                    out.playlists.push(m.to_playlist())
                }
                _ => {}
            }
        }
    }
    out
}

#[async_trait]
impl RemoteSearch for PlexSearch {
    async fn search(&self, query: &str, section_id: Option<&str>, limit: u32) -> AppResult<RemoteHits> {
        let hubs = self.0.client().await?.search_music(query, section_id, limit).await?;
        Ok(hits_from_hubs(hubs))
    }
}

/// Fetches resized artwork through the connected Plex server.
pub struct PlexArt(pub PlexState);

#[async_trait]
impl ArtFetcher for PlexArt {
    async fn fetch(&self, path: &str, width: u32, height: u32) -> AppResult<ArtBytes> {
        let bytes = self.0.client_for_when_ready(path, STARTUP_WAIT).await?.photo(path, width, height).await?;
        let content_type = sniff_content_type(&bytes).to_string();
        Ok(ArtBytes { bytes, content_type })
    }
}

/// The Plex relay is bandwidth-limited by the server; it never streams more.
pub const RELAY_MAX_KBPS: u32 = 128;

/// Streaming quality caps from Settings, in kbps (0 = original file).
/// Read on every stream request, so a change applies from the next track.
#[derive(Debug, Default)]
pub struct StreamPrefs {
    home_kbps: AtomicU32,
    away_kbps: AtomicU32,
}

impl StreamPrefs {
    pub fn new(home: Option<u32>, away: Option<u32>) -> Self {
        let p = Self::default();
        p.set(home, away);
        p
    }

    pub fn set(&self, home: Option<u32>, away: Option<u32>) {
        self.home_kbps.store(home.unwrap_or(0), Ordering::Relaxed);
        self.away_kbps.store(away.unwrap_or(0), Ordering::Relaxed);
    }

    pub fn home(&self) -> Option<u32> {
        Some(self.home_kbps.load(Ordering::Relaxed)).filter(|k| *k > 0)
    }

    pub fn away(&self) -> Option<u32> {
        Some(self.away_kbps.load(Ordering::Relaxed)).filter(|k| *k > 0)
    }
}

/// Bitrate cap for a connection, or None to play the original file. On the
/// local connection the home setting applies, on remote access the away
/// setting; the relay uses the away setting but never more than
/// [`RELAY_MAX_KBPS`]. Files already at or under the cap play untouched.
pub fn stream_cap_kbps(
    conn: Option<ConnectionKind>,
    file_kbps: Option<u32>,
    home: Option<u32>,
    away: Option<u32>,
) -> Option<u32> {
    let cap = match conn {
        Some(ConnectionKind::Remote) => away?,
        Some(ConnectionKind::Relay) => away.map_or(RELAY_MAX_KBPS, |a| a.min(RELAY_MAX_KBPS)),
        _ => home?,
    };
    match file_kbps {
        Some(b) if b <= cap => None,
        _ => Some(cap),
    }
}

/// Transcodes the server is running for us, oldest first. Plex keeps each
/// one going until it times out and refuses new ones once enough are open,
/// so skipping through songs on a remote connection ended in "Bad Request"
/// for every song after a few.
static OPEN_TRANSCODES: Mutex<std::collections::VecDeque<(crate::plex::client::PlexClient, String)>> =
    Mutex::new(std::collections::VecDeque::new());

/// How many may stay open: the song playing and the one loaded after it.
const TRANSCODES_KEPT: usize = 2;

/// Notes a new transcode and stops the oldest beyond [`TRANSCODES_KEPT`].
fn keep_transcode(client: crate::plex::client::PlexClient, session: String) {
    let stale: Vec<_> = {
        let mut open = OPEN_TRANSCODES.lock();
        open.push_back((client, session));
        let extra = open.len().saturating_sub(TRANSCODES_KEPT);
        open.drain(..extra).collect()
    };
    for (client, session) in stale {
        tokio::spawn(async move {
            if let Err(e) = client.transcode_stop(&session).await {
                tracing::debug!("transcode not stopped: {e}");
            }
        });
    }
}

/// Turns a track into an HTTP source on the connected server. The token
/// travels only in headers. With the db it also serves loudness data and
/// remembers measured silence.
pub struct PlexStreams(pub PlexState, pub Arc<StreamPrefs>, pub Option<DbHandle>);

fn source_of(req: crate::plex::client::StreamRequest, codec: Option<String>) -> StreamSource {
    StreamSource {
        url: req.url.to_string(),
        headers: req
            .headers
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect(),
        size: None,
        codec_hint: codec,
    }
}

#[async_trait]
impl TrackResolver for PlexStreams {
    async fn stream(&self, track: &Track) -> AppResult<StreamSource> {
        let client = self.0.client_for_when_ready(&track.id, STARTUP_WAIT).await?;
        let prefs = &self.1;
        if let Some(kbps) = stream_cap_kbps(client.connection(), track.bitrate, prefs.home(), prefs.away()) {
            let session = uuid::Uuid::new_v4().to_string();
            let req = client.transcode_request(&track.id, kbps, &session)?;
            keep_transcode(client.clone(), session);
            return Ok(source_of(req, Some("mp3".into())));
        }
        let req = client.stream_request(&track.part_key)?;
        Ok(source_of(req, track.codec.clone()))
    }

    async fn transcode(&self, track: &Track) -> AppResult<StreamSource> {
        let session = uuid::Uuid::new_v4().to_string();
        let client = self.0.client_for(&track.id).await?;
        let req = client.transcode_request(&track.id, 320, &session)?;
        keep_transcode(client, session);
        Ok(source_of(req, Some("mp3".into())))
    }

    /// Loudness from the cache, or from Plex track detail on first play
    /// (then cached); measured silence from the cache.
    async fn audio_info(&self, track: &Track) -> Option<TrackAudio> {
        let db = self.2.clone()?;
        let id = track.id.clone();
        let cached = db.read_async(move |c| trackaudio::get(c, &id)).await.ok().flatten();
        if let Some((a, true)) = cached {
            return Some(a);
        }
        let mut a = cached.map(|c| c.0).unwrap_or_default();
        let Ok(client) = self.0.client_for(&track.id).await else { return Some(a) };
        match client.track(&track.id).await {
            Ok((_, stream)) => {
                let s = stream.unwrap_or_default();
                let f = |v: Option<f64>| v.filter(|v| v.is_finite()).map(|v| v as f32);
                a.gain_db = f(s.gain);
                a.album_gain_db = f(s.album_gain);
                a.peak = f(s.peak);
                a.album_peak = f(s.album_peak);
                let (id, stored) = (track.id.clone(), a);
                let now = unix_now();
                if let Err(e) = db.write_async(move |c| trackaudio::set_loudness(c, &id, &stored, now)).await {
                    tracing::debug!("could not store loudness: {e}");
                }
            }
            Err(e) => tracing::debug!("no loudness for {}: {e}", track.id),
        }
        Some(a)
    }

    fn save_silence(&self, track_id: &str, head_ms: u64, tail_ms: u64) {
        if let Some(db) = &self.2
            && let Err(e) = db.write(|c| trackaudio::set_silence(c, track_id, head_ms, tail_ms))
        {
            tracing::debug!("could not store silence: {e}");
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Resolves play contexts from the local cache.
/// Contexts and tracks from the cache. With the servers, a toast says when
/// the explicit filter or hidden fan edits left songs out of a context.
/// The third part says which songs can play while the server is out of reach.
/// The fourth makes radio and similar music (none: those find nothing).
pub struct CacheLookup(
    pub DbHandle,
    pub Option<crate::backend::Backends>,
    pub OfflinePlay,
    pub Option<crate::mixes::MixService>,
);

/// While only music on this computer can play, plays keep to the songs in
/// the audio cache, and the first play that leaves songs out says so once.
#[derive(Debug, Default)]
pub struct OfflinePlay {
    audio_dir: Option<std::path::PathBuf>,
    /// Downloaded songs play too.
    downloads: Option<crate::audio::source::LocalFiles>,
    told: std::sync::atomic::AtomicBool,
}

impl OfflinePlay {
    pub fn new(audio_dir: std::path::PathBuf) -> Self {
        OfflinePlay {
            audio_dir: Some(audio_dir),
            downloads: None,
            told: Default::default(),
        }
    }

    /// Downloaded songs count as on this computer.
    pub fn with_downloads(mut self, d: crate::audio::source::LocalFiles) -> Self {
        self.downloads = Some(d);
        self
    }

    /// Ids that can play now, or `None` when the server can be reached.
    pub async fn playable(&self, cached_only: bool) -> Option<std::collections::HashSet<String>> {
        if !cached_only {
            self.told.store(false, Ordering::Relaxed);
            return None;
        }
        let dir = self.audio_dir.clone()?;
        let ids = tokio::task::spawn_blocking(move || crate::monitor::cached::cached_ids(&dir))
            .await
            .unwrap_or_default();
        let mut ok: std::collections::HashSet<String> = ids.into_iter().collect();
        if let Some(d) = &self.downloads {
            ok.extend(d.0.ids());
        }
        Some(ok)
    }

    /// The notice for `n` songs left out, the first time only.
    pub fn notice(&self, n: usize) -> Option<String> {
        if n == 0 || self.told.swap(true, Ordering::Relaxed) {
            return None;
        }
        Some(offline_left_out_message(n))
    }
}

pub fn offline_left_out_message(n: usize) -> String {
    let songs = if n == 1 { "1 song that isn't".to_string() } else { format!("{n} songs that aren't") };
    format!("Skipped {songs} on this computer. They play again when Platter can reach your server.")
}

#[async_trait]
impl TrackLookup for CacheLookup {
    fn hidden(&self) -> Option<Arc<crate::hidden::HiddenSet>> {
        self.3.as_ref().map(|m| m.hidden())
    }

    async fn tracks(&self, ids: &[String]) -> AppResult<Vec<Track>> {
        // Ids saved before server tags (links, other players) still match.
        let ids = crate::sid::resolve_all(ids);
        self.0.read_async(move |c| queries::tracks_by_ids(c, &ids)).await
    }

    async fn context(&self, ctx: &PlayContext) -> AppResult<ResolvedContext> {
        let view = self.0.tag_view();
        let (resolved, skipped) = self.context_with(ctx, view).await?;
        let message = crate::tags::commands::skipped_message(skipped);
        if resolved.tracks.is_empty() && skipped != crate::tags::store::Skipped::default() {
            return Err(crate::error::AppError::Invalid(
                "Nothing here can play: every song is left out by your explicit content or fan edit settings.".into(),
            ));
        }
        if let (Some(m), Some(servers)) = (message, &self.1) {
            servers.toast("info", m);
        }
        Ok(resolved)
    }

    async fn playable_now(&self) -> Option<std::collections::HashSet<String>> {
        let servers = self.1.as_ref()?;
        self.2.playable(servers.cached_only()).await
    }

    fn left_out_offline(&self, n: usize) {
        if let (Some(m), Some(servers)) = (self.2.notice(n), &self.1) {
            servers.toast("info", m);
        }
    }

    /// The next radio batch, similar music after a context, or smart
    /// shuffle suggestions.
    async fn more(&self, req: &MoreRequest) -> AppResult<Vec<Track>> {
        use crate::mixes::radio::Seed;
        let Some(mixes) = self.3.clone() else { return Ok(Vec::new()) };
        let seed = match req.kind {
            ContextKind::Radio => match req.id.as_deref().and_then(Seed::parse) {
                Some(s) => s,
                None => return Ok(Vec::new()),
            },
            _ => Seed::Tracks(req.seed_ids.clone()),
        };
        let avoid: std::collections::HashSet<String> = req.avoid.iter().cloned().collect();
        let n = req.n;
        let smart = req.smart.then(|| (req.seed_ids.clone(), req.avoid_artists.iter().cloned().collect::<std::collections::HashSet<_>>()));
        // A radio on a server that makes radio of its own mixes in the
        // server's picks.
        let server_seed = match (&seed, req.kind, req.smart) {
            (Seed::Track(id) | Seed::Artist(id), ContextKind::Radio, false) => Some(id.clone()),
            _ => None,
        };
        let avoid_server = avoid.clone();
        let ids = tokio::task::spawn_blocking(move || match smart {
            Some((seeds, artists)) => mixes.suggestion_ids(&seeds, &avoid, &artists, n),
            None => mixes.radio_ids(&seed, &avoid, n, false),
        })
            .await
            .map_err(|e| crate::error::AppError::Internal(format!("radio task: {e}")))??;
        let ids = match (server_seed, &self.1) {
            (Some(seed), Some(servers)) => {
                let from_server = server_radio(servers, &seed, n * 2).await;
                blend_radio(&from_server, ids, &avoid_server, n)
            }
            _ => ids,
        };
        let view = self.0.tag_view();
        let mut tracks = self
            .0
            .read_async(move |c| {
                let mut t = queries::tracks_by_ids(c, &ids)?;
                crate::tags::store::drop_hidden(c, &mut t, view)?;
                Ok(t)
            })
            .await?;
        if let Some(ok) = self.playable_now().await {
            tracks.retain(|t| ok.contains(&t.id));
        }
        Ok(tracks)
    }
}

/// The seed's server's own radio picks (empty when it makes none or can't
/// be reached).
async fn server_radio(servers: &crate::backend::Backends, seed: &str, n: usize) -> Vec<String> {
    let Ok(server) = servers.for_id(seed) else { return Vec::new() };
    if !server.caps().has(crate::backend::Caps::SERVER_RADIO) {
        return Vec::new();
    }
    let Some(radio) = server.radio() else { return Vec::new() };
    match radio.mix(seed, n).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::debug!("server radio unavailable: {}", e.kind());
            Vec::new()
        }
    }
}

/// The server's picks and Platter's own, taken in turn, without repeats or
/// songs to avoid, up to `n`.
pub fn blend_radio(server: &[String], local: Vec<String>, avoid: &std::collections::HashSet<String>, n: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(n);
    let mut seen = std::collections::HashSet::new();
    let mut from_server = server.iter().filter(|i| !avoid.contains(*i));
    let mut own = local.iter();
    while out.len() < n {
        let (x, y) = (own.next(), from_server.next());
        if x.is_none() && y.is_none() {
            break;
        }
        for id in [x, y].into_iter().flatten() {
            if out.len() < n && seen.insert(id.clone()) {
                out.push(id.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod blend_tests {
    #[test]
    fn server_picks_and_own_take_turns() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let avoid = ["x".to_string()].into_iter().collect();
        assert_eq!(super::blend_radio(&s(&["x", "a", "b"]), s(&["1", "a", "2"]), &avoid, 4), s(&["1", "a", "b", "2"]));
        assert_eq!(super::blend_radio(&[], s(&["1", "2"]), &avoid, 5), s(&["1", "2"]), "no server picks: as before");
        assert_eq!(super::blend_radio(&s(&["a"]), Vec::new(), &avoid, 5), s(&["a"]));
    }
}

impl CacheLookup {
    /// The context's songs with the tag view applied, and what it left out.
    pub async fn context_with(
        &self,
        ctx: &PlayContext,
        view: crate::tags::TagView,
    ) -> AppResult<(ResolvedContext, crate::tags::store::Skipped)> {
        let mut ctx = ctx.clone();
        ctx.id = ctx.id.as_deref().map(crate::sid::resolve);
        ctx.track_ids = ctx.track_ids.as_deref().map(crate::sid::resolve_all);
        let popular = self.0.popular_source();
        let liked_min = crate::settings::load(&self.0).liked_min_rating();
        // Popular leaves hidden songs and artists out.
        let hidden_ids = self.3.as_ref().map(|m| m.hidden());
        // A radio's first songs come from the mix engine.
        let radio_ids: Option<Vec<String>> = match (ctx.kind, &self.3, ctx.id.as_deref()) {
            // Played from its page: the page's songs.
            (ContextKind::Radio, _, _) if ctx.track_ids.is_some() => ctx.track_ids.clone(),
            (ContextKind::Radio, Some(mixes), Some(id)) => {
                let seed = crate::mixes::radio::Seed::parse(id)
                    .ok_or_else(|| crate::error::AppError::Invalid(format!("not a radio: {id}")))?;
                let mixes = mixes.clone();
                Some(
                    tokio::task::spawn_blocking(move || {
                        mixes.radio_ids(&seed, &Default::default(), crate::mixes::radio::RADIO_BATCH, true)
                    })
                    .await
                    .map_err(|e| crate::error::AppError::Internal(format!("radio task: {e}")))??,
                )
            }
            _ => None,
        };
        // Songs on more than one server play from the best copy (db::merge).
        let ranks = matches!(self.0.server_view(), crate::db::merge::ServerView::All).then(|| self.0.source_ranks());
        self.0
            .read_async(move |c| {
                let hidden = crate::tags::store::hidden_albums(c, view)?;
                let id = ctx.id.clone().unwrap_or_default();
                let mut resolved = match ctx.kind {
                    ContextKind::Album => {
                        let page = queries::album_page(c, &id)?;
                        let mut tracks = page.tracks;
                        // One side of the record: those songs, in album order.
                        if let Some(ids) = &ctx.track_ids {
                            tracks.retain(|t| ids.contains(&t.id));
                        }
                        ResolvedContext { tracks, title: Some(page.album.title) }
                    }
                    ContextKind::Playlist => {
                        let page = queries::playlist_page(c, &id)?;
                        // The page's own order when it sends one (a sort or
                        // filter); a song in it twice keeps both copies.
                        let tracks = match &ctx.track_ids {
                            Some(ids) => in_order(page.tracks, ids),
                            None => page.tracks,
                        };
                        ResolvedContext { tracks, title: Some(page.playlist.title) }
                    }
                    ContextKind::Artist => {
                        let page = queries::artist_page_with(c, &id, popular, hidden_ids.as_deref())?;
                        let mut tracks = page.popular;
                        let mut seen: std::collections::HashSet<String> =
                            tracks.iter().map(|t| t.id.clone()).collect();
                        for section in &page.sections {
                            // Editions behind their group's face play once, as the face.
                            for album in section.albums.iter().filter(|a| !hidden.contains(&a.id)) {
                                for t in queries::album_tracks(c, &album.id)? {
                                    if seen.insert(t.id.clone()) {
                                        tracks.push(t);
                                    }
                                }
                            }
                        }
                        // A credited artist has no albums: the rest of the
                        // songs that credit them follow Popular.
                        for t in queries::credited_context_tracks(c, &id)? {
                            if seen.insert(t.id.clone()) {
                                tracks.push(t);
                            }
                        }
                        ResolvedContext { tracks, title: Some(page.artist.title) }
                    }
                    ContextKind::Liked => ResolvedContext {
                        // The page's own order when it sends one (a sort or filter).
                        tracks: match &ctx.track_ids {
                            Some(ids) => queries::tracks_by_ids(c, ids)?,
                            None => crate::db::likes::liked_tracks(c, liked_min)?,
                        },
                        title: Some("Liked Songs".into()),
                    },
                    ContextKind::Mix => {
                        let title = c
                            .query_row("SELECT title FROM mixes WHERE id = ?1", [&id], |r| r.get::<_, String>(0))
                            .optional()?;
                        let mut tracks = match &ctx.track_ids {
                            Some(ids) => queries::tracks_by_ids(c, ids)?,
                            None => crate::mixes::store::tracks(c, &id)?.0,
                        };
                        // Songs hidden since the mix was made stay out, as
                        // they do on its page.
                        if let Some(h) = &hidden_ids {
                            tracks.retain(|t| !h.hides(&t.id, &t.artist_id));
                        }
                        if title.is_none() && ctx.track_ids.is_none() {
                            return Err(crate::error::AppError::NotFound(format!("mix {id}")));
                        }
                        ResolvedContext { tracks, title }
                    }
                    ContextKind::Radio => {
                        let title = match id.split_once(':') {
                            Some(("track", t)) => c
                                .query_row("SELECT title FROM tracks WHERE id = ?1", [t], |r| r.get::<_, String>(0))
                                .optional()?,
                            Some(("artist", a)) => c
                                .query_row("SELECT title FROM artists WHERE id = ?1", [a], |r| r.get::<_, String>(0))
                                .optional()?,
                            _ => None,
                        };
                        ResolvedContext {
                            tracks: queries::tracks_by_ids(c, radio_ids.as_deref().unwrap_or_default())?,
                            title: title.map(|t| format!("{t} Radio")),
                        }
                    }
                    ContextKind::Tracks | ContextKind::Search => ResolvedContext {
                        tracks: queries::tracks_by_ids(c, &ctx.track_ids.clone().unwrap_or_default())?,
                        title: None,
                    },
                };
                let skipped = crate::tags::store::drop_hidden(c, &mut resolved.tracks, view)?;
                if let Some(ranks) = &ranks {
                    crate::db::merge::best_copies(c, &mut resolved.tracks, ranks)?;
                }
                Ok((resolved, skipped))
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ReleaseSection;

    #[tokio::test]
    async fn offline_plays_keep_to_cached_songs_and_say_so_once() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("101-0123456789abcdef.audio"), b"x").unwrap();
        let off = OfflinePlay::new(d.path().to_path_buf());
        // Connected: everything plays, nothing to filter.
        assert_eq!(off.playable(false).await, None);
        let ids = off.playable(true).await.unwrap();
        assert_eq!(ids.into_iter().collect::<Vec<_>>(), vec!["101".to_string()]);
        assert_eq!(
            off.notice(3).as_deref(),
            Some("Skipped 3 songs that aren't on this computer. They play again when Platter can reach your server.")
        );
        assert_eq!(off.notice(2), None, "once while offline");
        // Back online, then offline again: says it again.
        off.playable(false).await;
        assert_eq!(
            off.notice(1).as_deref(),
            Some("Skipped 1 song that isn't on this computer. They play again when Platter can reach your server.")
        );
        assert_eq!(OfflinePlay::default().notice(0), None);
    }

    struct Downloaded;
    impl crate::audio::source::LocalAudio for Downloaded {
        fn path(&self, _: &Track) -> Option<std::path::PathBuf> {
            None
        }
        fn ids(&self) -> Vec<String> {
            vec!["202".into()]
        }
    }

    #[tokio::test]
    async fn offline_plays_include_downloaded_songs() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("101-0123456789abcdef.audio"), b"x").unwrap();
        let off = OfflinePlay::new(d.path().to_path_buf())
            .with_downloads(crate::audio::source::LocalFiles(Arc::new(Downloaded)));
        let mut ids: Vec<String> = off.playable(true).await.unwrap().into_iter().collect();
        ids.sort();
        assert_eq!(ids, vec!["101".to_string(), "202".to_string()]);
    }

    fn album(id: &str) -> Album {
        Album { mbid: None,
            id: id.into(),
            title: "t".into(),
            sort_title: "t".into(),
            artist_id: "a".into(),
            artist_title: "A".into(),
            year: None,
            thumb: None,
            format: None,
            subformats: vec![],
            section: ReleaseSection::Albums,
            genres: vec![],
            track_count: 1,
            added_at: 0,
            view_count: 0,
            last_viewed_at: None,
            tint: None,
            released: None,
            studio: None,
            matched: None,
            styles: vec![],
            moods: vec![],
        }
    }

    #[test]
    fn stream_quality_follows_the_connection() {
        use ConnectionKind::*;
        // Defaults: original at home, 320 away.
        let cap = |c, f| stream_cap_kbps(c, f, None, Some(320));
        assert_eq!(cap(Some(Local), Some(1411)), None);
        assert_eq!(cap(None, Some(1411)), None);
        assert_eq!(cap(Some(Remote), Some(1411)), Some(320));
        assert_eq!(cap(Some(Remote), Some(320)), None);
        assert_eq!(cap(Some(Remote), Some(256)), None);
        assert_eq!(cap(Some(Remote), None), Some(320));
        assert_eq!(cap(Some(Relay), Some(320)), Some(128));
        assert_eq!(cap(Some(Relay), Some(96)), None);
    }

    #[test]
    fn quality_settings_cap_the_stream_request() {
        use ConnectionKind::*;
        // A home cap applies only on the local connection.
        assert_eq!(stream_cap_kbps(Some(Local), Some(1411), Some(192), Some(320)), Some(192));
        assert_eq!(stream_cap_kbps(Some(Local), Some(128), Some(192), Some(320)), None);
        assert_eq!(stream_cap_kbps(Some(Remote), Some(1411), Some(192), Some(320)), Some(320));
        // Original away from home plays the file as is over remote access...
        assert_eq!(stream_cap_kbps(Some(Remote), Some(1411), None, None), None);
        // ...but the relay keeps the server's limit as the most it streams.
        assert_eq!(stream_cap_kbps(Some(Relay), Some(1411), None, None), Some(128));
        assert_eq!(stream_cap_kbps(Some(Relay), Some(1411), None, Some(256)), Some(128));
        assert_eq!(stream_cap_kbps(Some(Relay), Some(1411), None, Some(96)), Some(96));
        assert_eq!(stream_cap_kbps(Some(Remote), Some(1411), None, Some(128)), Some(128));
    }

    #[test]
    fn stream_prefs_store_original_as_no_cap() {
        let p = StreamPrefs::new(None, Some(320));
        assert_eq!((p.home(), p.away()), (None, Some(320)));
        p.set(Some(256), None);
        assert_eq!((p.home(), p.away()), (Some(256), None));
    }

    #[test]
    fn tags_map_into_sections() {
        let mut t = AlbumTags::default();
        t.format.insert("1".into(), "ep".into());
        t.format.insert("2".into(), "album".into());
        t.subformats.insert("2".into(), vec!["soundtrack".into()]);
        let (mut a, mut b, mut c) = (album("1"), album("2"), album("3"));
        t.apply(&mut a);
        t.apply(&mut b);
        t.apply(&mut c);
        assert_eq!(a.section, ReleaseSection::SinglesEps);
        assert_eq!(b.section, ReleaseSection::Soundtracks);
        assert_eq!(c.section, ReleaseSection::Albums);
        assert_eq!(c.format, None);
    }

    #[test]
    fn styles_and_moods_merge_into_albums() {
        let mut t = AlbumTags::default();
        t.add("style", "Trap", "1".into());
        t.add("mood", "Brooding", "1".into());
        t.add("mood", "Night", "1".into());
        t.add("format", "EP", "1".into());
        let mut a = album("1");
        a.moods = vec!["night".into()];
        t.apply(&mut a);
        assert_eq!(a.styles, vec!["Trap"]);
        assert_eq!(a.moods, vec!["night", "Brooding"], "no repeats, any case");
        assert_eq!(a.format.as_deref(), Some("ep"));
    }
    #[test]
    fn search_hubs_sort_into_kinds() {
        let item = |kind: &str, key: &str| PlexMetadata {
            rating_key: key.into(),
            title: format!("{kind} {key}"),
            kind: Some(kind.into()),
            ..Default::default()
        };
        let hubs = vec![
            PlexHub {
                hub_identifier: "artist".into(),
                kind: Some("artist".into()),
                metadata: vec![item("artist", "1")],
                ..Default::default()
            },
            PlexHub {
                hub_identifier: "track".into(),
                kind: Some("track".into()),
                metadata: vec![item("track", "2"), item("track", "3")],
                ..Default::default()
            },
            PlexHub {
                hub_identifier: "playlist".into(),
                kind: Some("playlist".into()),
                metadata: vec![
                    item("playlist", "4"),
                    PlexMetadata { playlist_type: Some("video".into()), ..item("playlist", "5") },
                ],
                ..Default::default()
            },
            PlexHub {
                hub_identifier: "album".into(),
                kind: None,
                metadata: vec![PlexMetadata { kind: None, ..item("album", "6") }],
                ..Default::default()
            },
        ];
        let h = hits_from_hubs(hubs);
        assert_eq!(h.artists.len(), 1);
        assert_eq!(h.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["2", "3"]);
        assert_eq!(h.playlists.len(), 1);
        assert_eq!(h.albums[0].id, "6");
    }
}

#[cfg(test)]
mod in_order_tests {
    use super::in_order;

    fn t(id: &str, n: u32) -> crate::model::Track {
        crate::audio::testutil::track(id, "Artist", &format!("/p/{n}"))
    }

    #[test]
    fn a_sorted_page_plays_in_its_order_with_both_copies_of_a_repeat() {
        let tracks = vec![t("a", 1), t("b", 2), t("a", 3), t("c", 4)];
        let ids: Vec<String> = ["c", "a", "b", "a", "gone"].iter().map(|s| s.to_string()).collect();
        let out = in_order(tracks, &ids);
        let got: Vec<(&str, &str)> = out.iter().map(|t| (t.id.as_str(), t.part_key.as_str())).collect();
        assert_eq!(got, vec![("c", "/p/4"), ("a", "/p/1"), ("b", "/p/2"), ("a", "/p/3")]);
    }
}

#[cfg(test)]
mod transcode_tests {
    use std::sync::Arc;

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{OPEN_TRANSCODES, TRANSCODES_KEPT, keep_transcode};
    use crate::plex::client::PlexClient;
    use crate::plex::identity::ClientIdentity;
    use crate::plex::token::Token;
    use crate::plex::types::ConnectionKind;

    #[tokio::test]
    async fn only_the_playing_and_next_transcodes_stay_open() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/music/:/transcode/universal/stop"))
            .and(query_param("session", "s1"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let c = PlexClient::new(
            &server.uri(),
            Token::new("tok-123456789").unwrap(),
            Arc::new(ClientIdentity::with_id("cid")),
            Some(ConnectionKind::Remote),
        )
        .unwrap();
        OPEN_TRANSCODES.lock().clear();
        for s in ["s1", "s2", "s3"] {
            keep_transcode(c.clone(), s.into());
        }
        let open: Vec<String> = OPEN_TRANSCODES.lock().iter().map(|(_, s)| s.clone()).collect();
        assert_eq!(open, ["s2", "s3"]);
        assert_eq!(open.len(), TRANSCODES_KEPT);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // `expect(1)` is checked when the server drops: s1 was stopped once.
    }
}
