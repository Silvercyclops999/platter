//! Glue between the Plex client and the db layer: the `LibrarySource` and
//! `ArtFetcher` the sync engine and artwork cache pull from.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::db::artcache::sniff_content_type;
use crate::db::source::{
    ArtBytes, Changes, HistoryEntry, ItemKind, MusicSection, PlaylistEntry, RemoteHits, RemoteSearch,
    SimilarArtist,
};
use crate::audio::source::{ContextKind, PlayContext, ResolvedContext, StreamSource, TrackLookup, TrackResolver};
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

fn meta_type(kind: ItemKind) -> MetaType {
    match kind {
        ItemKind::Artist => MetaType::Artist,
        ItemKind::Album => MetaType::Album,
        ItemKind::Track => MetaType::Track,
    }
}

/// Release-type tags per album id. The album list endpoint doesn't include
/// `Format`/`Subformat`, so they're collected in bulk with one filtered list
/// per tag value and merged into each album page.
#[derive(Default, Clone)]
struct AlbumTags {
    format: HashMap<String, String>,
    subformats: HashMap<String, Vec<String>>,
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
    }
}

pub struct PlexLibrary {
    plex: PlexState,
    tags: Mutex<HashMap<String, Arc<AlbumTags>>>,
}

impl PlexLibrary {
    pub fn new(plex: PlexState) -> Self {
        Self {
            plex,
            tags: Mutex::new(HashMap::new()),
        }
    }

    async fn load_tags(&self, section: &str) -> AppResult<Arc<AlbumTags>> {
        let client = self.plex.client().await?;
        let mut tags = AlbumTags::default();
        for field in ["format", "subformat"] {
            for value in client.tag_values(section, field, MetaType::Album).await? {
                let albums = client
                    .list_all(section, MetaType::Album, &[(field, value.key.clone())])
                    .await?;
                let tag = value.title.to_lowercase();
                for a in albums {
                    if field == "format" {
                        tags.format.insert(a.rating_key, tag.clone());
                    } else {
                        tags.subformats.entry(a.rating_key).or_default().push(tag.clone());
                    }
                }
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
        let client = self.plex.client().await?;
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
    async fn music_sections(&self) -> AppResult<Vec<MusicSection>> {
        let client = self.plex.client().await?;
        Ok(client
            .music_sections()
            .await?
            .into_iter()
            .map(|s| MusicSection {
                uuid: s.uuid.clone().unwrap_or_else(|| s.id.clone()),
                id: s.id,
                title: s.title,
            })
            .collect())
    }

    async fn count(&self, section_id: &str, kind: ItemKind) -> AppResult<u64> {
        self.plex.client().await?.count(section_id, meta_type(kind)).await
    }

    async fn artists(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Artist>> {
        Ok(self.plex.client().await?.artists(section_id, start, size).await?.items)
    }

    async fn albums(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Album>> {
        let tags = self.tags(section_id, start == 0).await?;
        let mut items = self.plex.client().await?.albums(section_id, start, size).await?.items;
        for a in &mut items {
            tags.apply(a);
        }
        Ok(items)
    }

    async fn tracks(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Track>> {
        Ok(self.plex.client().await?.tracks(section_id, start, size).await?.items)
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
        let page = self.plex.client().await?.history(section_id, start, size).await?;
        Ok(page
            .items
            .into_iter()
            .map(|h| HistoryEntry {
                rating_key: h.rating_key,
                viewed_at: h.viewed_at,
            })
            .collect())
    }

    async fn playlists(&self) -> AppResult<Vec<Playlist>> {
        self.plex.client().await?.playlists().await
    }

    async fn rated_tracks(&self, section_id: &str) -> AppResult<Option<Vec<crate::db::likes::Rated>>> {
        let rated = self.plex.client().await?.rated_tracks(section_id).await?;
        Ok(Some(
            rated
                .into_iter()
                .map(|(track_id, rating, rated_at)| crate::db::likes::Rated { track_id, rating, rated_at })
                .collect(),
        ))
    }

    async fn artist_similar(&self, artist_id: &str) -> AppResult<Vec<SimilarArtist>> {
        let m = self.plex.client().await?.metadata(artist_id).await?;
        Ok(m.similar_artists().into_iter().map(|(id, name)| SimilarArtist { id, name }).collect())
    }

    async fn playlist_entries(&self, playlist_id: &str) -> AppResult<Vec<PlaylistEntry>> {
        let rows = self.plex.client().await?.playlist_rows(playlist_id).await?;
        Ok(rows.into_iter().map(|(track_id, item_id)| PlaylistEntry { track_id, item_id }).collect())
    }

    async fn playlist_items(&self, playlist_id: &str) -> AppResult<Vec<String>> {
        let client = self.plex.client().await?;
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
        let bytes = self.0.client_when_ready(STARTUP_WAIT).await?.photo(path, width, height).await?;
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
        let client = self.0.client_when_ready(STARTUP_WAIT).await?;
        let prefs = &self.1;
        if let Some(kbps) = stream_cap_kbps(client.connection(), track.bitrate, prefs.home(), prefs.away()) {
            let session = uuid::Uuid::new_v4().to_string();
            let req = client.transcode_request(&track.id, kbps, &session)?;
            return Ok(source_of(req, Some("mp3".into())));
        }
        let req = client.stream_request(&track.part_key)?;
        Ok(source_of(req, track.codec.clone()))
    }

    async fn transcode(&self, track: &Track) -> AppResult<StreamSource> {
        let session = uuid::Uuid::new_v4().to_string();
        let req = self.0.client().await?.transcode_request(&track.id, 320, &session)?;
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
        let Ok(client) = self.0.client().await else { return Some(a) };
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
/// Contexts and tracks from the cache. With a `PlexState`, a toast says when
/// the explicit filter or hidden fan edits left songs out of a context.
pub struct CacheLookup(pub DbHandle, pub Option<PlexState>);

#[async_trait]
impl TrackLookup for CacheLookup {
    async fn tracks(&self, ids: &[String]) -> AppResult<Vec<Track>> {
        let ids = ids.to_vec();
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
        if let (Some(m), Some(plex)) = (message, &self.1) {
            plex.toast("info", m);
        }
        Ok(resolved)
    }
}

impl CacheLookup {
    /// The context's songs with the tag view applied, and what it left out.
    pub async fn context_with(
        &self,
        ctx: &PlayContext,
        view: crate::tags::TagView,
    ) -> AppResult<(ResolvedContext, crate::tags::store::Skipped)> {
        let ctx = ctx.clone();
        let popular = self.0.popular_source();
        let liked_min = crate::settings::load(&self.0).liked_min_rating();
        self.0
            .read_async(move |c| {
                let hidden = crate::tags::store::hidden_albums(c, view)?;
                let id = ctx.id.clone().unwrap_or_default();
                let mut resolved = match ctx.kind {
                    ContextKind::Album => {
                        let page = queries::album_page(c, &id)?;
                        ResolvedContext { tracks: page.tracks, title: Some(page.album.title) }
                    }
                    ContextKind::Playlist => {
                        let page = queries::playlist_page(c, &id)?;
                        ResolvedContext { tracks: page.tracks, title: Some(page.playlist.title) }
                    }
                    ContextKind::Artist => {
                        let page = queries::artist_page_by(c, &id, popular)?;
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
                    ContextKind::Tracks | ContextKind::Search => ResolvedContext {
                        tracks: queries::tracks_by_ids(c, &ctx.track_ids.clone().unwrap_or_default())?,
                        title: None,
                    },
                };
                let skipped = crate::tags::store::drop_hidden(c, &mut resolved.tracks, view)?;
                Ok((resolved, skipped))
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ReleaseSection;

    fn album(id: &str) -> Album {
        Album {
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
