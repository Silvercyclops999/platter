//! Glue between the Plex client and the db layer: the `LibrarySource` and
//! `ArtFetcher` the sync engine and artwork cache pull from.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::db::artcache::sniff_content_type;
use crate::db::source::{ArtBytes, Changes, HistoryEntry, ItemKind, MusicSection, RemoteHits, RemoteSearch};
use crate::audio::source::{ContextKind, PlayContext, ResolvedContext, StreamSource, TrackLookup, TrackResolver};
use crate::db::{ArtFetcher, DbHandle, LibrarySource, queries};
use crate::error::AppResult;
use crate::model::{Album, Artist, Playlist, Track};
use crate::plex::client::{MetaType, PlexHub, PlexMetadata};
use crate::plex::PlexState;
use crate::plex::types::ConnectionKind;

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
        let bytes = self.0.client().await?.photo(path, width, height).await?;
        let content_type = sniff_content_type(&bytes).to_string();
        Ok(ArtBytes { bytes, content_type })
    }
}

/// Bitrate cap for a connection, or None to play the original file. Away from
/// home the default is 320 kbps; the relay is bandwidth-capped by Plex, so it
/// drops to 128. Files already at or under the cap play untouched.
pub fn stream_cap_kbps(conn: Option<ConnectionKind>, file_kbps: Option<u32>) -> Option<u32> {
    let cap = match conn {
        Some(ConnectionKind::Remote) => 320,
        Some(ConnectionKind::Relay) => 128,
        _ => return None,
    };
    match file_kbps {
        Some(b) if b <= cap => None,
        _ => Some(cap),
    }
}

/// Turns a track into an HTTP source on the connected server. The token
/// travels only in headers.
pub struct PlexStreams(pub PlexState);

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
        let client = self.0.client().await?;
        if let Some(kbps) = stream_cap_kbps(client.connection(), track.bitrate) {
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
}

/// Resolves play contexts from the local cache.
pub struct CacheLookup(pub DbHandle);

#[async_trait]
impl TrackLookup for CacheLookup {
    async fn tracks(&self, ids: &[String]) -> AppResult<Vec<Track>> {
        let ids = ids.to_vec();
        self.0.read_async(move |c| queries::tracks_by_ids(c, &ids)).await
    }

    async fn context(&self, ctx: &PlayContext) -> AppResult<ResolvedContext> {
        let ctx = ctx.clone();
        self.0
            .read_async(move |c| {
                let id = ctx.id.clone().unwrap_or_default();
                Ok(match ctx.kind {
                    ContextKind::Album => {
                        let page = queries::album_page(c, &id)?;
                        ResolvedContext { tracks: page.tracks, title: Some(page.album.title) }
                    }
                    ContextKind::Playlist => {
                        let page = queries::playlist_page(c, &id)?;
                        ResolvedContext { tracks: page.tracks, title: Some(page.playlist.title) }
                    }
                    ContextKind::Artist => {
                        let page = queries::artist_page(c, &id)?;
                        let mut tracks = page.popular;
                        let mut seen: std::collections::HashSet<String> =
                            tracks.iter().map(|t| t.id.clone()).collect();
                        for section in &page.sections {
                            for album in &section.albums {
                                for t in queries::album_tracks(c, &album.id)? {
                                    if seen.insert(t.id.clone()) {
                                        tracks.push(t);
                                    }
                                }
                            }
                        }
                        ResolvedContext { tracks, title: Some(page.artist.title) }
                    }
                    ContextKind::Tracks | ContextKind::Search => ResolvedContext {
                        tracks: queries::tracks_by_ids(c, &ctx.track_ids.clone().unwrap_or_default())?,
                        title: None,
                    },
                })
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
        }
    }

    #[test]
    fn stream_quality_follows_the_connection() {
        use ConnectionKind::*;
        assert_eq!(stream_cap_kbps(Some(Local), Some(1411)), None);
        assert_eq!(stream_cap_kbps(None, Some(1411)), None);
        assert_eq!(stream_cap_kbps(Some(Remote), Some(1411)), Some(320));
        assert_eq!(stream_cap_kbps(Some(Remote), Some(320)), None);
        assert_eq!(stream_cap_kbps(Some(Remote), Some(256)), None);
        assert_eq!(stream_cap_kbps(Some(Remote), None), Some(320));
        assert_eq!(stream_cap_kbps(Some(Relay), Some(320)), Some(128));
        assert_eq!(stream_cap_kbps(Some(Relay), Some(96)), None);
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
