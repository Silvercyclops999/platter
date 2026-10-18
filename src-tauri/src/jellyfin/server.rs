//! A Jellyfin server as one of the app's music servers: the library for
//! sync, streams, downloads, file reads for the tag scan, favourites as
//! likes, playlists, play reports, lyrics, artwork, Instant Mix, artist
//! details and what others are playing.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use reqwest::Method;
use serde::Serialize;

use super::client::{JellyfinClient, NOT_ALLOWED, OFFLINE, SIGN_IN_AGAIN};
use super::likes::LikeDates;
use super::map;
use super::types::{Item, LyricDto, QueryResult, SessionInfo, User, iso, ms_ticks};
use crate::audio::dsp::TrackAudio;
use crate::audio::source::{StreamSource, TrackResolver};
use crate::backend::server::{MusicServer, ServerRadio};
use crate::backend::{BackendKind, Caps};
use crate::bridge::StreamPrefs;
use crate::db::DbHandle;
use crate::db::likes::{LIKED_RATING, Rated};
use crate::db::source::{
    ArtBytes, ArtFetcher, Changes, HistoryEntry, ItemKind, LibrarySource, MusicSection, PlaylistEntry, RemoteHits,
    RemoteSearch, SimilarArtist,
};
use crate::downloads::{DownloadSource, Fetch};
use crate::error::{AppError, AppResult};
use crate::likes::LikeRemote;
use crate::model::{Playlist, Track};
use crate::playlists::PlaylistRemote;
use crate::plex::types::ConnectionKind;
use crate::sid::{key, qualify_any};
use crate::tags::scan::PartSource;
use crate::tracking::{PlayReport, PlayReporter, PlaybackState};

const PAGE: u64 = 500;
const ID_PAGE: u64 = 5000;
/// Items asked for by id in one request.
const ID_CHUNK: usize = 50;
pub const TRACK_FIELDS: &str = "MediaSources,ProviderIds,Genres,DateCreated,Path,SortName";
pub const ALBUM_FIELDS: &str = "ProviderIds,Genres,DateCreated,SortName,ChildCount,Studios,Overview";
pub const ARTIST_FIELDS: &str = "Overview,SortName,DateCreated,ProviderIds";
pub const PLAYLIST_FIELDS: &str = "ChildCount,CumulativeRunTimeTicks,Overview,DateCreated,DateLastSaved";

pub const NO_TRANSCODE: &str = "Your Jellyfin account isn't allowed to convert songs, so they play as they are.";
pub const NO_DOWNLOAD: &str = "Your Jellyfin account isn't allowed to download songs. The server's owner can allow it.";
const NO_POSTER: &str = "Jellyfin playlist covers can't be changed from Platter.";
const NO_SUMMARY: &str = "Jellyfin playlists don't have descriptions.";

/// What the server can do for this account.
pub fn jellyfin_caps(admin: bool, transcode: bool) -> Caps {
    let mut c = Caps::LIVE_UPDATES
        | Caps::INCREMENTAL_SYNC
        | Caps::PLAYLIST_MOVE
        | Caps::LYRICS
        | Caps::SYNCED_LYRICS
        | Caps::SERVER_RADIO
        | Caps::LOUDNESS
        | Caps::BACKDATED_SCROBBLE;
    if transcode {
        c |= Caps::TRANSCODE;
    }
    if admin {
        // Only an administrator sees other people's sessions and the files'
        // real paths.
        c |= Caps::FRIEND_ACTIVITY | Caps::REAL_FILE_PATHS;
    }
    c
}

/// Where a server is, by its address: on this network or out on the internet.
pub fn connection_for(base: &str) -> ConnectionKind {
    let host = url::Url::parse(base).ok().and_then(|u| u.host_str().map(|h| h.trim_matches(['[', ']']).to_ascii_lowercase()));
    let Some(host) = host else { return ConnectionKind::Remote };
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".lan") || host.ends_with(".home.arpa") || !host.contains('.') && !host.contains(':') {
        return ConnectionKind::Local;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) if ip.is_private() || ip.is_loopback() || ip.is_link_local() => ConnectionKind::Local,
        Ok(std::net::IpAddr::V6(ip)) if ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80 => {
            ConnectionKind::Local
        }
        _ => ConnectionKind::Remote,
    }
}

/// A music library on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: String,
    pub title: String,
}

/// How the server is doing, for Settings and the capability list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerState {
    /// Not asked yet.
    Unknown,
    Online,
    Offline,
    /// The token was revoked or ran out: sign in again.
    SignInAgain,
}

impl ServerState {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Online,
            2 => Self::Offline,
            3 => Self::SignInAgain,
            _ => Self::Unknown,
        }
    }
    fn as_u8(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Online => 1,
            Self::Offline => 2,
            Self::SignInAgain => 3,
        }
    }
}

struct Session {
    client: JellyfinClient,
    user: User,
}

/// Everything about one server; each of the app's interfaces is
/// implemented on it.
pub struct Core {
    tag: String,
    name: RwLock<String>,
    unsigned: JellyfinClient,
    session: RwLock<Option<Session>>,
    views: RwLock<Vec<View>>,
    /// Included libraries; None: every music library.
    include: RwLock<Option<Vec<String>>>,
    state: AtomicU8,
    likes: LikeDates,
    prefs: Arc<StreamPrefs>,
    db: Option<DbHandle>,
    /// The song now reported as playing, and its play session.
    playing: Mutex<Option<(String, String)>>,
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Go offline: no requests at all.
    offline: RwLock<Arc<std::sync::atomic::AtomicBool>>,
}

fn now() -> i64 {
    crate::db::queries::now_s()
}

fn q<'a>(pairs: &[(&'a str, &str)]) -> Vec<(&'a str, String)> {
    pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
}

impl Core {
    pub fn new(tag: &str, name: &str, unsigned: JellyfinClient, prefs: Arc<StreamPrefs>, db: Option<DbHandle>, likes: LikeDates) -> Self {
        Self {
            tag: tag.into(),
            name: RwLock::new(name.into()),
            unsigned,
            session: RwLock::new(None),
            views: RwLock::new(Vec::new()),
            include: RwLock::new(None),
            state: AtomicU8::new(0),
            likes,
            prefs,
            db,
            playing: Mutex::new(None),
            on_change: Mutex::new(None),
            offline: RwLock::new(Arc::default()),
        }
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn base(&self) -> &str {
        self.unsigned.base()
    }

    pub fn name(&self) -> String {
        self.name.read().clone()
    }

    pub fn set_name(&self, n: &str) {
        *self.name.write() = n.to_string();
    }

    /// Signed in as `user` with `client`.
    pub fn sign_in(&self, client: JellyfinClient, user: User) {
        *self.session.write() = Some(Session { client, user });
        self.set_state(ServerState::Unknown);
        self.changed();
    }

    pub fn signed_in(&self) -> bool {
        self.session.read().is_some()
    }

    pub fn user(&self) -> Option<User> {
        self.session.read().as_ref().map(|s| s.user.clone())
    }

    pub fn state(&self) -> ServerState {
        if !self.signed_in() {
            return ServerState::SignInAgain;
        }
        ServerState::from_u8(self.state.load(Ordering::Acquire))
    }

    fn set_state(&self, s: ServerState) {
        let before = self.state.swap(s.as_u8(), Ordering::AcqRel);
        if before != s.as_u8() && (s == ServerState::SignInAgain || before == ServerState::SignInAgain.as_u8()) {
            self.changed();
        }
    }

    pub fn on_change(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *self.on_change.lock() = Some(f);
    }

    fn changed(&self) {
        let f = self.on_change.lock().clone();
        if let Some(f) = f {
            f();
        }
    }

    pub fn caps(&self) -> Caps {
        let policy = self.user().and_then(|u| u.policy).unwrap_or_default();
        jellyfin_caps(policy.is_administrator, policy.enable_audio_playback_transcoding)
    }

    fn admin(&self) -> bool {
        self.user().and_then(|u| u.policy).is_some_and(|p| p.is_administrator)
    }

    /// Follow the app's Go offline switch.
    pub fn set_offline_switch(&self, off: Arc<std::sync::atomic::AtomicBool>) {
        *self.offline.write() = off;
    }

    pub fn client(&self) -> AppResult<JellyfinClient> {
        if self.offline.read().load(Ordering::Relaxed) {
            return Err(AppError::Network(OFFLINE.into()));
        }
        self.session.read().as_ref().map(|s| s.client.clone()).ok_or_else(|| AppError::Auth(SIGN_IN_AGAIN.into()))
    }

    fn uid(&self) -> AppResult<String> {
        self.session.read().as_ref().map(|s| s.user.id.clone()).ok_or_else(|| AppError::Auth(SIGN_IN_AGAIN.into()))
    }

    fn note<T>(&self, r: &AppResult<T>) {
        match r {
            Ok(_) => self.set_state(ServerState::Online),
            Err(AppError::Network(_)) => self.set_state(ServerState::Offline),
            Err(AppError::Auth(m)) if m == SIGN_IN_AGAIN => self.set_state(ServerState::SignInAgain),
            Err(_) => {}
        }
    }

    pub async fn get<T: serde::de::DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> AppResult<T> {
        let c = self.client()?;
        let r = c.get_json(path, &q(query)).await;
        self.note(&r);
        r
    }

    async fn call(&self, method: Method, path: &str, query: &[(&str, &str)], body: Option<&serde_json::Value>) -> AppResult<()> {
        let c = self.client()?;
        let r = c.call(method, path, &q(query), body).await;
        self.note(&r);
        r
    }

    /// Whether the server answers now, checked with `/Users/Me` (which also
    /// refreshes the account's rights).
    pub async fn check(&self) -> ServerState {
        let Ok(c) = self.client() else { return ServerState::SignInAgain };
        let r = super::auth::me(&c).await;
        self.note(&r);
        if let Ok(user) = r
            && let Some(s) = self.session.write().as_mut()
        {
            s.user = user;
        }
        self.state()
    }

    // ---- libraries --------------------------------------------------------

    /// The server's music libraries, read again.
    pub async fn load_views(&self) -> AppResult<Vec<View>> {
        let uid = self.uid()?;
        let r: QueryResult = self.get("/UserViews", &[("userId", &uid)]).await?;
        let views: Vec<View> = r
            .items
            .into_iter()
            .filter(|i| i.collection_type.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("music")))
            .map(|i| View { id: i.id, title: i.name.unwrap_or_else(|| "Music".into()) })
            .collect();
        *self.views.write() = views.clone();
        Ok(views)
    }

    pub fn views(&self) -> Vec<View> {
        self.views.read().clone()
    }

    pub fn set_include(&self, ids: Option<Vec<String>>) {
        *self.include.write() = ids;
    }

    pub fn include(&self) -> Option<Vec<String>> {
        self.include.read().clone()
    }

    /// The included libraries among those known.
    pub fn included_views(&self) -> Vec<View> {
        let inc = self.include.read().clone();
        self.views
            .read()
            .iter()
            .filter(|v| inc.as_ref().is_none_or(|i| i.contains(&v.id)))
            .cloned()
            .collect()
    }

    fn sections(&self, only: Option<&str>) -> Vec<MusicSection> {
        let views = match only {
            Some(id) => self.views().into_iter().filter(|v| v.id == id).collect(),
            None => self.included_views(),
        };
        views.into_iter().map(|v| MusicSection { uuid: v.id.clone(), id: v.id, title: v.title }).collect()
    }

    async fn page(&self, section: &str, types: &str, fields: &str, start: u64, size: u64, extra: &[(&str, &str)]) -> AppResult<QueryResult> {
        let uid = self.uid()?;
        let (start, size) = (start.to_string(), size.to_string());
        let mut query: Vec<(&str, &str)> = vec![
            ("userId", &uid),
            ("ParentId", section),
            ("Recursive", "true"),
            ("IncludeItemTypes", types),
            ("StartIndex", &start),
            ("Limit", &size),
            ("Fields", fields),
            ("SortBy", "SortName"),
            ("EnableUserData", "true"),
        ];
        query.extend_from_slice(extra);
        self.get("/Items", &query).await
    }

    async fn artist_page(&self, section: &str, start: u64, size: u64, fields: &str) -> AppResult<QueryResult> {
        let uid = self.uid()?;
        let (start, size) = (start.to_string(), size.to_string());
        self.get(
            "/Artists/AlbumArtists",
            &[
                ("userId", &uid),
                ("ParentId", section),
                ("StartIndex", &start),
                ("Limit", &size),
                ("Fields", fields),
                ("SortBy", "SortName"),
                ("EnableUserData", "true"),
            ],
        )
        .await
    }

    /// Every page of a list, until a short one.
    async fn all_pages(&self, section: &str, types: &str, fields: &str, extra: &[(&str, &str)]) -> AppResult<Vec<Item>> {
        let mut out = Vec::new();
        let mut start = 0;
        loop {
            let r = self.page(section, types, fields, start, PAGE, extra).await?;
            let n = r.items.len() as u64;
            out.extend(r.items);
            if n < PAGE {
                return Ok(out);
            }
            start += n;
        }
    }

    fn track_of(&self, it: &Item) -> Track {
        map::track(&self.tag, it, |id| Some(self.likes.first_seen(id, now())))
    }

    /// Items by id (raw ids), any kind.
    async fn by_ids(&self, ids: &[String], fields: &str) -> AppResult<Vec<Item>> {
        let uid = self.uid()?;
        let mut out = Vec::new();
        for chunk in ids.chunks(ID_CHUNK) {
            let list = chunk.join(",");
            let r: QueryResult = self.get("/Items", &[("userId", &uid), ("Ids", &list), ("Fields", fields), ("EnableUserData", "true")]).await?;
            out.extend(r.items);
        }
        Ok(out)
    }

    fn split(&self, items: Vec<Item>) -> Changes {
        let mut c = Changes::default();
        for it in items {
            match it.kind.as_deref() {
                Some("MusicArtist") => c.artists.push(map::artist(&self.tag, &it)),
                Some("MusicAlbum") => c.albums.push(map::album(&self.tag, &it)),
                Some("Audio") => c.tracks.push(self.track_of(&it)),
                _ => {}
            }
        }
        c
    }

    // ---- streams ------------------------------------------------------------

    fn may_transcode(&self) -> bool {
        self.user().and_then(|u| u.policy).unwrap_or_default().enable_audio_playback_transcoding
    }

    fn may_download(&self) -> bool {
        self.user().and_then(|u| u.policy).unwrap_or_default().enable_content_downloading
    }

    fn source(&self, c: &JellyfinClient, url: url::Url, codec: Option<String>) -> AppResult<StreamSource> {
        Ok(StreamSource { url: url.to_string(), headers: c.stream_headers()?, size: None, codec_hint: codec })
    }

    /// The original file, as it is (Range works).
    fn static_stream(&self, track: &Track) -> AppResult<StreamSource> {
        let c = self.client()?;
        let path = if crate::sid::tag_of(&track.part_key) == Some(self.tag.as_str()) {
            key(&track.part_key).to_string()
        } else {
            format!("/Audio/{}/stream", key(&track.id))
        };
        let url = c.url(&path, &[("static", "true".into())])?;
        self.source(&c, url, track.container.clone().or_else(|| track.codec.clone()))
    }

    /// The song converted to MP3 at `kbps` by the server, and its play
    /// session (to stop it after).
    fn universal(&self, track: &Track, kbps: u32) -> AppResult<(StreamSource, String)> {
        if !self.may_transcode() {
            return Err(AppError::Auth(NO_TRANSCODE.into()));
        }
        let c = self.client()?;
        let session = uuid::Uuid::new_v4().simple().to_string();
        let url = c.url(
            &format!("/Audio/{}/universal", key(&track.id)),
            &[
                ("UserId", self.uid()?),
                ("DeviceId", c.identity().device_id.clone()),
                ("PlaySessionId", session.clone()),
                ("MaxStreamingBitrate", (kbps.max(32) * 1000).to_string()),
                ("Container", "mp3".into()),
                ("TranscodingContainer", "mp3".into()),
                ("TranscodingProtocol", "http".into()),
                ("AudioCodec", "mp3".into()),
                ("EnableRedirection", "false".into()),
            ],
        )?;
        Ok((self.source(&c, url, Some("mp3".into()))?, session))
    }

    async fn stop_encoding(&self, session: &str) {
        let Ok(c) = self.client() else { return };
        let device = c.identity().device_id.clone();
        if let Err(e) = self.call(Method::DELETE, "/Videos/ActiveEncodings", &[("deviceId", &device), ("playSessionId", session)], None).await {
            tracing::debug!("jellyfin conversion not stopped: {}", e.kind());
        }
    }

    // ---- radio and lookups ----------------------------------------------------

    async fn item(&self, id: &str) -> AppResult<Item> {
        let uid = self.uid()?;
        self.get(&format!("/Items/{}", key(id)), &[("userId", &uid)]).await
    }

    async fn provider_id(&self, id: &str, which: &str) -> AppResult<Option<String>> {
        let it = self.item(id).await?;
        Ok(it.provider_ids.iter().find(|(k, _)| k.eq_ignore_ascii_case(which)).map(|(_, v)| v.clone()).filter(|v| !v.is_empty()))
    }

    // ---- friend activity, as the shape the panel reads ------------------------

    /// Account ids for the panel: the signed-in user is the owner (1), others
    /// a stable number from their id.
    fn account_number(&self, user_id: &str) -> i64 {
        if self.uid().is_ok_and(|u| u.eq_ignore_ascii_case(user_id)) {
            return crate::social::activity::OWNER_ACCOUNT;
        }
        let h = crate::sid::tag(&user_id.to_ascii_lowercase());
        i64::from_str_radix(&h, 16).unwrap_or(2).max(2)
    }

    /// `/Sessions` as the panel's session list.
    pub fn sessions_json(&self, sessions: &[SessionInfo]) -> serde_json::Value {
        let me = self.client().map(|c| c.identity().device_id.clone()).unwrap_or_default();
        let rows: Vec<serde_json::Value> = sessions
            .iter()
            .filter(|s| s.device_id.as_deref() != Some(me.as_str()))
            .filter_map(|s| {
                let it = s.now_playing_item.as_ref().filter(|i| i.kind.as_deref() == Some("Audio"))?;
                let user = s.user_id.as_deref()?;
                let t = map::track(&self.tag, it, |_| None);
                Some(serde_json::json!({
                    "type": "track",
                    "ratingKey": t.id,
                    "title": t.title,
                    "originalTitle": t.track_artist,
                    "grandparentTitle": t.artist_title,
                    "parentTitle": t.album_title,
                    "parentRatingKey": (!t.album_id.is_empty()).then_some(t.album_id),
                    "parentThumb": t.thumb,
                    "User": { "id": self.account_number(user), "title": s.user_name },
                    "Player": { "state": if s.play_state.as_ref().is_some_and(|p| p.is_paused) { "paused" } else { "playing" } },
                }))
            })
            .collect();
        serde_json::json!({ "MediaContainer": { "Metadata": rows } })
    }
}

fn json_bytes(v: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

// ---- the library ----------------------------------------------------------------

/// One server's library for the sync engine: every included music library
/// (the main sync picks one), or one library on its own.
pub struct JellyfinLibrary {
    core: Arc<Core>,
    only: Option<String>,
}

impl JellyfinLibrary {
    pub fn all(core: Arc<Core>) -> Self {
        Self { core, only: None }
    }
    pub fn one(core: Arc<Core>, view: &str) -> Self {
        Self { core, only: Some(view.to_string()) }
    }
}

#[async_trait]
impl LibrarySource for JellyfinLibrary {
    async fn server_tag(&self) -> Option<String> {
        Some(self.core.tag.clone())
    }

    async fn music_sections(&self) -> AppResult<Vec<MusicSection>> {
        if self.core.views().is_empty() {
            self.core.load_views().await?;
        }
        Ok(self.core.sections(self.only.as_deref()))
    }

    async fn count(&self, section_id: &str, kind: ItemKind) -> AppResult<u64> {
        let r = match kind {
            ItemKind::Artist => self.core.artist_page(section_id, 0, 0, "").await?,
            ItemKind::Album => self.core.page(section_id, "MusicAlbum", "", 0, 0, &[]).await?,
            ItemKind::Track => self.core.page(section_id, "Audio", "", 0, 0, &[]).await?,
        };
        Ok(r.total_record_count.unwrap_or(r.items.len() as u64))
    }

    async fn artists(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<crate::model::Artist>> {
        let r = self.core.artist_page(section_id, start, size, ARTIST_FIELDS).await?;
        Ok(r.items.iter().map(|i| map::artist(&self.core.tag, i)).collect())
    }

    async fn albums(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<crate::model::Album>> {
        let r = self.core.page(section_id, "MusicAlbum", ALBUM_FIELDS, start, size, &[]).await?;
        Ok(r.items.iter().map(|i| map::album(&self.core.tag, i)).collect())
    }

    async fn tracks(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Track>> {
        let r = self.core.page(section_id, "Audio", TRACK_FIELDS, start, size, &[]).await?;
        Ok(r.items.iter().map(|i| self.core.track_of(i)).collect())
    }

    /// Items saved since `since`, songs whose plays or favourite changed,
    /// and the album artists of anything new.
    async fn changed_since(&self, section_id: &str, since: i64) -> AppResult<Changes> {
        let when = iso(since);
        let saved = [("MinDateLastSaved", when.as_str())];
        let user = [("MinDateLastSavedForUser", when.as_str())];
        let mut albums = self.core.all_pages(section_id, "MusicAlbum", ALBUM_FIELDS, &saved).await?;
        let mut tracks = self.core.all_pages(section_id, "Audio", TRACK_FIELDS, &saved).await?;
        let mut seen: HashSet<String> = tracks.iter().map(|t| t.id.clone()).collect();
        for t in self.core.all_pages(section_id, "Audio", TRACK_FIELDS, &user).await? {
            if seen.insert(t.id.clone()) {
                tracks.push(t);
            }
        }
        let mut album_seen: HashSet<String> = albums.iter().map(|a| a.id.clone()).collect();
        for a in self.core.all_pages(section_id, "MusicAlbum", ALBUM_FIELDS, &user).await? {
            if album_seen.insert(a.id.clone()) {
                albums.push(a);
            }
        }
        let artist_ids: Vec<String> = albums
            .iter()
            .chain(tracks.iter())
            .flat_map(|i| i.album_artists.iter().filter_map(|a| a.id.clone()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let artists = if artist_ids.is_empty() { Vec::new() } else { self.core.by_ids(&artist_ids, ARTIST_FIELDS).await? };
        Ok(Changes {
            artists: artists.iter().map(|i| map::artist(&self.core.tag, i)).collect(),
            albums: albums.iter().map(|i| map::album(&self.core.tag, i)).collect(),
            tracks: tracks.iter().map(|i| self.core.track_of(i)).collect(),
        })
    }

    async fn items(&self, _section_id: &str, ids: &[String]) -> AppResult<Option<Changes>> {
        let raw: Vec<String> = ids.iter().map(|i| key(i).to_string()).collect();
        let fields = format!("{TRACK_FIELDS},ChildCount,Studios,Overview");
        let items = self.core.by_ids(&raw, &fields).await?;
        Ok(Some(self.core.split(items)))
    }

    async fn all_ids(&self, section_id: &str, kind: ItemKind) -> AppResult<Vec<String>> {
        let mut out = Vec::new();
        let mut start = 0;
        loop {
            let r = match kind {
                ItemKind::Artist => self.core.artist_page(section_id, start, ID_PAGE, "").await?,
                ItemKind::Album => self.core.page(section_id, "MusicAlbum", "", start, ID_PAGE, &[("EnableImages", "false")]).await?,
                ItemKind::Track => self.core.page(section_id, "Audio", "", start, ID_PAGE, &[("EnableImages", "false")]).await?,
            };
            let n = r.items.len() as u64;
            out.extend(r.items.into_iter().map(|i| qualify_any(&self.core.tag, &i.id)));
            if n < ID_PAGE {
                return Ok(out);
            }
            start += n;
        }
    }

    /// Jellyfin keeps play counts and the last play, not a history.
    async fn history(&self, _section_id: &str, _start: u64, _size: u64) -> AppResult<Vec<HistoryEntry>> {
        Ok(Vec::new())
    }

    async fn playlists(&self) -> AppResult<Vec<Playlist>> {
        let uid = self.core.uid()?;
        let r: QueryResult = self
            .core
            .get(
                "/Items",
                &[("userId", &uid), ("IncludeItemTypes", "Playlist"), ("Recursive", "true"), ("Fields", PLAYLIST_FIELDS), ("SortBy", "SortName")],
            )
            .await?;
        Ok(r.items.iter().filter(|i| map::is_audio_playlist(i)).map(|i| map::playlist(&self.core.tag, i)).collect())
    }

    async fn playlist_items(&self, playlist_id: &str) -> AppResult<Vec<String>> {
        Ok(self.playlist_entries(playlist_id).await?.into_iter().map(|e| e.track_id).collect())
    }

    async fn playlist_entries(&self, playlist_id: &str) -> AppResult<Vec<PlaylistEntry>> {
        entries(&self.core, playlist_id).await
    }

    /// Favourites, with the date each was first seen here.
    async fn liked_tracks(&self, section_id: &str) -> AppResult<Option<Vec<Rated>>> {
        let items = self.core.all_pages(section_id, "Audio", "", &[("Filters", "IsFavorite"), ("EnableImages", "false")]).await?;
        let t = now();
        let rated: Vec<Rated> = items
            .iter()
            .map(|i| {
                let id = qualify_any(&self.core.tag, &i.id);
                Rated { rated_at: Some(self.core.likes.first_seen(&id, t)), track_id: id, rating: LIKED_RATING }
            })
            .collect();
        // With one library, every song of the server is in this list's
        // reach: dates of songs no longer liked can go.
        if self.core.included_views().len() <= 1 {
            let liked: HashSet<String> = rated.iter().map(|r| r.track_id.clone()).collect();
            let tag = self.core.tag.clone();
            self.core.likes.keep_only(&liked, |id| crate::sid::tag_of(id) == Some(tag.as_str()));
        } else {
            self.core.likes.save();
        }
        Ok(Some(rated))
    }

    async fn artist_similar(&self, artist_id: &str) -> AppResult<Vec<SimilarArtist>> {
        let uid = self.core.uid()?;
        let r: QueryResult = self.core.get(&format!("/Artists/{}/Similar", key(artist_id)), &[("userId", &uid), ("Limit", "20")]).await?;
        Ok(r.items
            .into_iter()
            .filter_map(|i| Some(SimilarArtist { id: Some(qualify_any(&self.core.tag, &i.id)), name: i.name.filter(|n| !n.trim().is_empty())? }))
            .collect())
    }
}

async fn entries(core: &Core, playlist_id: &str) -> AppResult<Vec<PlaylistEntry>> {
    let uid = core.uid()?;
    let r: QueryResult = core.get(&format!("/Playlists/{}/Items", key(playlist_id)), &[("userId", &uid), ("EnableImages", "false")]).await?;
    Ok(r.items
        .into_iter()
        .map(|i| PlaylistEntry { track_id: qualify_any(&core.tag, &i.id), item_id: i.playlist_item_id.filter(|e| !e.is_empty()) })
        .collect())
}

// ---- search and art ---------------------------------------------------------

#[async_trait]
impl RemoteSearch for Core {
    /// Each kind is asked on its own, so many matching songs don't crowd
    /// out the artists and albums, and only in the included libraries
    /// (`section_id`, or every included one). Playlists live outside the
    /// libraries.
    async fn search(&self, query: &str, section_id: Option<&str>, limit: u32) -> AppResult<RemoteHits> {
        let uid = self.uid()?;
        let n = limit.max(1) as usize;
        let limit = n.to_string();
        let fields = format!("{TRACK_FIELDS},ChildCount");
        let parents: Vec<Option<String>> = match (section_id, self.include()) {
            (Some(s), _) => vec![Some(s.to_string())],
            (None, None) => vec![None],
            (None, Some(_)) => {
                let views = self.included_views();
                if views.is_empty() { vec![None] } else { views.into_iter().map(|v| Some(v.id)).collect() }
            }
        };
        let mut hits = RemoteHits::default();
        for kind in ["MusicArtist", "MusicAlbum", "Audio", "Playlist"] {
            let under: &[Option<String>] = if kind == "Playlist" { &[None] } else { &parents };
            for parent in under {
                let mut q: Vec<(&str, &str)> = vec![
                    ("userId", &uid),
                    ("searchTerm", query),
                    ("IncludeItemTypes", kind),
                    ("Recursive", "true"),
                    ("Limit", &limit),
                    ("Fields", &fields),
                    ("EnableUserData", "true"),
                ];
                if let Some(p) = parent {
                    q.push(("ParentId", p));
                }
                let r: QueryResult = self.get("/Items", &q).await?;
                for it in r.items {
                    match it.kind.as_deref() {
                        Some("MusicArtist") => hits.artists.push(map::artist(&self.tag, &it)),
                        Some("MusicAlbum") => hits.albums.push(map::album(&self.tag, &it)),
                        Some("Audio") => hits.tracks.push(self.track_of(&it)),
                        Some("Playlist") if map::is_audio_playlist(&it) => hits.playlists.push(map::playlist(&self.tag, &it)),
                        _ => {}
                    }
                }
            }
        }
        hits.artists.truncate(n);
        hits.albums.truncate(n);
        hits.tracks.truncate(n);
        hits.playlists.truncate(n);
        Ok(hits)
    }
}

#[async_trait]
impl ArtFetcher for Core {
    /// The server resizes the picture.
    async fn fetch(&self, path: &str, width: u32, height: u32) -> AppResult<ArtBytes> {
        let c = self.client()?;
        let mut url = url::Url::parse(&format!("{}{}", c.base(), key(path))).map_err(|e| AppError::Invalid(format!("art path: {e}")))?;
        url.query_pairs_mut()
            .append_pair("fillWidth", &width.to_string())
            .append_pair("fillHeight", &height.to_string())
            .append_pair("quality", "90");
        let r = reqwest::Client::get(&jf_http(), url).header("Authorization", c.header()?).send().await;
        let resp = match r {
            Ok(r) => r,
            Err(_) => {
                self.set_state(ServerState::Offline);
                return Err(AppError::Network(OFFLINE.into()));
            }
        };
        super::client::check(resp.status())?;
        let declared = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_string);
        let bytes = crate::web::body_capped(resp, crate::web::SERVER_MAX, |_| AppError::Network(OFFLINE.into())).await?;
        let content_type = declared
            .filter(|t| t.starts_with("image/"))
            .unwrap_or_else(|| crate::db::artcache::sniff_content_type(&bytes).to_string());
        Ok(ArtBytes { bytes, content_type })
    }
}

fn jf_http() -> reqwest::Client {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP.get_or_init(super::client::http).clone()
}

// ---- streams, downloads, file reads ---------------------------------------------

#[async_trait]
impl TrackResolver for Core {
    async fn stream(&self, track: &Track) -> AppResult<StreamSource> {
        let conn = Some(connection_for(self.base()));
        let cap = crate::bridge::stream_cap_kbps(conn, track.bitrate, self.prefs.home(), self.prefs.away());
        match cap {
            Some(kbps) if self.may_transcode() => Ok(self.universal(track, kbps)?.0),
            _ => self.static_stream(track),
        }
    }

    async fn transcode(&self, track: &Track) -> AppResult<StreamSource> {
        Ok(self.universal(track, 320)?.0)
    }

    /// Jellyfin's loudness value (from its audio normalization task), kept
    /// in the cache after the first ask.
    async fn audio_info(&self, track: &Track) -> Option<TrackAudio> {
        let db = self.db.clone();
        let id = track.id.clone();
        let cached = match &db {
            Some(db) => {
                let id = id.clone();
                db.read_async(move |c| crate::db::trackaudio::get(c, &id)).await.ok().flatten()
            }
            None => None,
        };
        if let Some((a, true)) = cached {
            return Some(a);
        }
        let mut a = cached.map(|c| c.0).unwrap_or_default();
        match self.item(&track.id).await {
            Ok(it) => {
                a.gain_db = it.normalization_gain.filter(|g| g.is_finite()).map(|g| g as f32);
                if let Some(db) = db {
                    let stored = a;
                    if let Err(e) = db.write_async(move |c| crate::db::trackaudio::set_loudness(c, &id, &stored, now())).await {
                        tracing::debug!("could not store loudness: {e}");
                    }
                }
            }
            Err(e) => tracing::debug!("no loudness from jellyfin: {}", e.kind()),
        }
        Some(a)
    }

    fn save_silence(&self, track_id: &str, head_ms: u64, tail_ms: u64) {
        if let Some(db) = &self.db
            && let Err(e) = db.write(|c| crate::db::trackaudio::set_silence(c, track_id, head_ms, tail_ms))
        {
            tracing::debug!("could not store silence: {e}");
        }
    }
}

#[async_trait]
impl DownloadSource for Core {
    async fn connection(&self) -> Option<ConnectionKind> {
        (self.signed_in() && self.state() != ServerState::Offline).then(|| connection_for(self.base()))
    }

    async fn fetch(&self, track: &Track, kbps: Option<u32>) -> AppResult<Fetch> {
        if !self.may_download() {
            return Err(AppError::Auth(NO_DOWNLOAD.into()));
        }
        match kbps {
            None => {
                let c = self.client()?;
                let url = c.url(&format!("/Items/{}/Download", key(&track.id)), &[])?;
                Ok(Fetch { src: self.source(&c, url, track.container.clone())?, resumable: true, session: None })
            }
            Some(k) => {
                let (src, session) = self.universal(track, k)?;
                Ok(Fetch { src, resumable: false, session: Some(format!("{}:{session}", self.tag)) })
            }
        }
    }

    async fn finished(&self, fetch: &Fetch) {
        if let Some(s) = &fetch.session
            && crate::sid::tag_of(s) == Some(self.tag.as_str())
        {
            self.stop_encoding(key(s)).await;
        }
    }
}

#[async_trait]
impl PartSource for Core {
    async fn part(&self, track_id: &str, part_key: &str) -> AppResult<StreamSource> {
        let c = self.client()?;
        let path = if crate::sid::tag_of(part_key) == Some(self.tag.as_str()) {
            key(part_key).to_string()
        } else {
            format!("/Audio/{}/stream", key(track_id))
        };
        let url = c.url(&path, &[("static", "true".into())])?;
        self.source(&c, url, None)
    }
}

// ---- likes, playlists, plays ------------------------------------------------------

#[async_trait]
impl LikeRemote for Core {
    async fn online(&self) -> bool {
        self.signed_in() && !matches!(self.state(), ServerState::Offline | ServerState::SignInAgain)
    }

    async fn set_liked(&self, track_id: &str, liked: bool) -> AppResult<()> {
        let uid = self.uid()?;
        let method = if liked { Method::POST } else { Method::DELETE };
        self.call(method, &format!("/UserFavoriteItems/{}", key(track_id)), &[("userId", &uid)], None).await?;
        if liked {
            self.likes.liked(track_id, now());
        } else {
            self.likes.unliked(track_id);
        }
        Ok(())
    }
}

/// The index a row lands on when moved after `after` (None: to the top), in
/// the list with the row taken out, which is how Jellyfin counts.
pub fn move_index(entries: &[PlaylistEntry], item_id: &str, after: Option<&str>) -> AppResult<usize> {
    let rest: Vec<&PlaylistEntry> = entries.iter().filter(|e| e.item_id.as_deref() != Some(item_id)).collect();
    if rest.len() == entries.len() {
        return Err(AppError::NotFound("That song isn't in the playlist any more.".into()));
    }
    match after {
        None => Ok(0),
        Some(a) => rest
            .iter()
            .position(|e| e.item_id.as_deref() == Some(a))
            .map(|i| i + 1)
            .ok_or_else(|| AppError::NotFound("That song isn't in the playlist any more.".into())),
    }
}

#[async_trait]
impl PlaylistRemote for Core {
    async fn online(&self) -> bool {
        LikeRemote::online(self).await
    }

    async fn create(&self, title: &str, track_ids: &[String]) -> AppResult<String> {
        let uid = self.uid()?;
        let ids: Vec<&str> = track_ids.iter().map(|i| key(i)).collect();
        let body = serde_json::json!({ "Name": title, "Ids": ids, "UserId": uid, "MediaType": "Audio", "IsPublic": false });
        let c = self.client()?;
        let r = c.post_json::<super::types::PlaylistCreated>("/Playlists", &[], &body).await;
        self.note(&r);
        Ok(qualify_any(&self.tag, &r?.id))
    }

    async fn add(&self, id: &str, track_ids: &[String]) -> AppResult<()> {
        let uid = self.uid()?;
        let ids = track_ids.iter().map(|i| key(i)).collect::<Vec<_>>().join(",");
        self.call(Method::POST, &format!("/Playlists/{}/Items", key(id)), &[("ids", &ids), ("userId", &uid)], None).await
    }

    async fn remove(&self, id: &str, item_id: &str) -> AppResult<()> {
        self.call(Method::DELETE, &format!("/Playlists/{}/Items", key(id)), &[("entryIds", item_id)], None).await
    }

    async fn move_item(&self, id: &str, item_id: &str, after: Option<&str>) -> AppResult<()> {
        let rows = entries(self, id).await?;
        let to = move_index(&rows, item_id, after)?;
        self.call(Method::POST, &format!("/Playlists/{}/Items/{item_id}/Move/{to}", key(id)), &[], None).await
    }

    async fn update(&self, id: &str, title: Option<&str>, summary: Option<&str>) -> AppResult<()> {
        if summary.is_some() && title.is_none() {
            return Err(AppError::Invalid(NO_SUMMARY.into()));
        }
        let Some(title) = title else { return Ok(()) };
        self.call(Method::POST, &format!("/Playlists/{}", key(id)), &[], Some(&serde_json::json!({ "Name": title }))).await
    }

    async fn poster(&self, _id: &str, _bytes: Vec<u8>, _content_type: &str) -> AppResult<()> {
        Err(AppError::Invalid(NO_POSTER.into()))
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.call(Method::DELETE, &format!("/Items/{}", key(id)), &[], None).await
    }

    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)> {
        let uid = self.uid()?;
        let it: Item = self.get(&format!("/Items/{}", key(id)), &[("userId", &uid), ("Fields", PLAYLIST_FIELDS)]).await?;
        Ok((map::playlist(&self.tag, &it), entries(self, id).await?))
    }
}

/// Play reports. Sessions (Playing, Progress, Stopped) only show what's
/// playing: the Stopped report says the play didn't count, so the server
/// doesn't count it on its own. Each finished play is counted once, by
/// marking the song played with the time it happened.
#[async_trait]
impl PlayReporter for Core {
    async fn online(&self) -> bool {
        LikeRemote::online(self).await
    }

    async fn timeline(&self, r: &PlayReport) -> AppResult<()> {
        let id = key(&r.track_id).to_string();
        let ticks = ms_ticks(r.time_ms);
        let (session, started) = {
            let mut p = self.playing.lock();
            match p.as_ref() {
                Some((t, s)) if *t == r.track_id => (s.clone(), false),
                _ => {
                    let s = uuid::Uuid::new_v4().simple().to_string();
                    *p = Some((r.track_id.clone(), s.clone()));
                    (s, true)
                }
            }
        };
        let paused = r.state == PlaybackState::Paused;
        let body = |extra: serde_json::Value| {
            let mut b = serde_json::json!({
                "ItemId": id, "PositionTicks": ticks, "PlaySessionId": session, "IsPaused": paused,
                "CanSeek": true, "PlayMethod": "DirectPlay",
            });
            if let (Some(b), Some(e)) = (b.as_object_mut(), extra.as_object()) {
                b.extend(e.clone());
            }
            b
        };
        match r.state {
            PlaybackState::Stopped => {
                *self.playing.lock() = None;
                if started {
                    // Never reported as playing: nothing to stop.
                    return Ok(());
                }
                self.call(Method::POST, "/Sessions/Playing/Stopped", &[], Some(&body(serde_json::json!({ "Failed": true })))).await
            }
            _ if started => self.call(Method::POST, "/Sessions/Playing", &[], Some(&body(serde_json::json!({})))).await,
            _ => {
                let event = match r.state {
                    PlaybackState::Paused => "Pause",
                    _ => "TimeUpdate",
                };
                self.call(Method::POST, "/Sessions/Playing/Progress", &[], Some(&body(serde_json::json!({ "EventName": event })))).await
            }
        }
    }

    async fn scrobble(&self, track_id: &str, at: Option<i64>) -> AppResult<()> {
        let uid = self.uid()?;
        let when = iso(at.unwrap_or_else(now));
        self.call(Method::POST, &format!("/UserPlayedItems/{}", key(track_id)), &[("userId", &uid), ("datePlayed", &when)], None).await
    }
}

#[async_trait]
impl crate::artistinfo::service::MbidLookup for Core {
    async fn artist_mbid(&self, artist_id: &str) -> AppResult<Option<String>> {
        self.provider_id(artist_id, "MusicBrainzArtist").await
    }
    async fn album_mbid(&self, album_id: &str) -> AppResult<Option<String>> {
        self.provider_id(album_id, "MusicBrainzAlbum").await
    }
}

/// Jellyfin lyrics as the app's: timed when the lines have start times.
pub fn lyrics_of(dto: LyricDto) -> Option<crate::plex::details::Lyrics> {
    use crate::plex::details::{LyricLine, Lyrics, LyricsSource};
    if dto.lyrics.iter().all(|l| l.text.trim().is_empty()) {
        return None;
    }
    let synced = dto.lyrics.iter().any(|l| l.start.is_some());
    let timed = if synced {
        dto.lyrics
            .iter()
            .filter_map(|l| Some(LyricLine { start_ms: (l.start? / 10_000).min(u32::MAX as u64) as u32, text: l.text.clone(), words: Vec::new() }))
            .collect()
    } else {
        Vec::new()
    };
    Some(Lyrics {
        synced,
        lines: dto.lyrics.iter().map(|l| l.text.clone()).collect(),
        timed,
        credit: dto.metadata.author.filter(|a| !a.trim().is_empty()).map(|a| format!("Written by {a}")),
        source: LyricsSource::Plex,
    })
}

#[async_trait]
impl crate::lyrics::ServerLyrics for Core {
    async fn lyrics(&self, track_id: &str) -> AppResult<Option<crate::plex::details::Lyrics>> {
        match self.get::<LyricDto>(&format!("/Audio/{}/Lyrics", key(track_id)), &[]).await {
            Ok(dto) => Ok(lyrics_of(dto)),
            Err(AppError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl ServerRadio for Core {
    /// Instant Mix from a song, album or artist.
    async fn mix(&self, seed_id: &str, n: usize) -> AppResult<Vec<String>> {
        let uid = self.uid()?;
        let limit = n.clamp(1, 200).to_string();
        let r: QueryResult = self.get(&format!("/Items/{}/InstantMix", key(seed_id)), &[("userId", &uid), ("Limit", &limit), ("EnableImages", "false")]).await?;
        Ok(r.items
            .into_iter()
            .filter(|i| i.kind.as_deref().is_none_or(|k| k == "Audio"))
            .map(|i| qualify_any(&self.tag, &i.id))
            .collect())
    }
}

#[async_trait]
impl crate::social::activity::ActivitySource for Core {
    async fn owned(&self) -> Option<bool> {
        if !self.signed_in() || self.state() == ServerState::Offline {
            return None;
        }
        Some(self.admin())
    }

    async fn sessions(&self) -> AppResult<Vec<u8>> {
        if !self.admin() {
            return Err(AppError::Auth(NOT_ALLOWED.into()));
        }
        let list: Vec<SessionInfo> = self.get("/Sessions", &[("ActiveWithinSeconds", "960")]).await?;
        Ok(json_bytes(&self.sessions_json(&list)))
    }

    /// Jellyfin keeps no play history of other people.
    async fn history(&self, _size: u64) -> AppResult<Vec<u8>> {
        Ok(json_bytes(&serde_json::json!({ "MediaContainer": {} })))
    }

    async fn accounts(&self) -> AppResult<Vec<u8>> {
        let users: Vec<User> = self.get("/Users", &[]).await?;
        let rows: Vec<serde_json::Value> = users
            .iter()
            .filter_map(|u| Some(serde_json::json!({ "id": self.account_number(&u.id), "name": u.name.clone()? })))
            .collect();
        Ok(json_bytes(&serde_json::json!({ "MediaContainer": { "Account": rows } })))
    }
}

#[async_trait]
impl crate::lidarr::PlexMusic for Core {
    /// The included libraries' folders and every song's file, for the
    /// Lidarr check (administrators only).
    async fn music_files(&self) -> AppResult<crate::lidarr::check::PlexFiles> {
        #[derive(serde::Deserialize, Default)]
        #[serde(rename_all = "PascalCase", default)]
        struct Folder {
            item_id: Option<String>,
            locations: Vec<String>,
        }
        if !self.admin() {
            return Err(AppError::Auth(NOT_ALLOWED.into()));
        }
        if self.views().is_empty() {
            self.load_views().await?;
        }
        let views = self.included_views();
        let folders: Vec<Folder> = self.get("/Library/VirtualFolders", &[]).await?;
        let mut out = crate::lidarr::check::PlexFiles::default();
        for f in folders {
            if f.item_id.as_ref().is_some_and(|id| views.iter().any(|v| &v.id == id)) {
                out.locations.extend(f.locations);
            }
        }
        for v in &views {
            for it in self.all_pages(&v.id, "Audio", "Path,MediaSources", &[("EnableImages", "false")]).await? {
                let src = it.media_sources.first();
                if let Some(p) = src.and_then(|s| s.path.clone()).or(it.path).filter(|p| !p.trim().is_empty()) {
                    out.files.push(crate::lidarr::check::PlexFile { path: p, size: src.and_then(|s| s.size).map(|s| s as i64) });
                }
            }
        }
        Ok(out)
    }
}

// ---- the music server ----------------------------------------------------------------

pub struct JellyfinServer {
    core: Arc<Core>,
}

impl JellyfinServer {
    pub fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }

    /// One library of this server, for syncing it besides the main one.
    pub fn view_library(&self, view: &str) -> Arc<dyn LibrarySource> {
        Arc::new(JellyfinLibrary::one(self.core.clone(), view))
    }
}

#[async_trait]
impl MusicServer for JellyfinServer {
    fn kind(&self) -> BackendKind {
        BackendKind::Jellyfin
    }

    fn caps(&self) -> Caps {
        self.core.caps()
    }

    async fn tag(&self) -> Option<String> {
        Some(self.core.tag.clone())
    }

    async fn name(&self) -> Option<String> {
        Some(self.core.name()).filter(|n| !n.trim().is_empty())
    }

    async fn reach(&self) -> Option<ConnectionKind> {
        (self.core.signed_in() && self.core.state() != ServerState::Offline).then(|| connection_for(self.core.base()))
    }

    /// The id share links and listen-along hash: the server's tag input.
    async fn server_id(&self) -> Option<String> {
        Some(format!("jellyfin:{}", self.core.tag))
    }

    fn library(&self) -> Arc<dyn LibrarySource> {
        Arc::new(JellyfinLibrary::all(self.core.clone()))
    }
    fn libraries(&self) -> Vec<(String, Arc<dyn LibrarySource>)> {
        self.core.included_views().into_iter().map(|v| (v.id.clone(), self.view_library(&v.id))).collect()
    }
    fn search(&self) -> Arc<dyn RemoteSearch> {
        self.core.clone()
    }
    fn art(&self) -> Arc<dyn ArtFetcher> {
        self.core.clone()
    }
    fn streams(&self) -> Arc<dyn TrackResolver> {
        self.core.clone()
    }
    fn downloads(&self) -> Arc<dyn DownloadSource> {
        self.core.clone()
    }
    fn parts(&self) -> Arc<dyn PartSource> {
        self.core.clone()
    }
    fn likes(&self) -> Arc<dyn LikeRemote> {
        self.core.clone()
    }
    fn playlists(&self) -> Arc<dyn PlaylistRemote> {
        self.core.clone()
    }
    fn plays(&self) -> Arc<dyn PlayReporter> {
        self.core.clone()
    }
    fn mbids(&self) -> Arc<dyn crate::artistinfo::service::MbidLookup> {
        self.core.clone()
    }
    fn lyrics(&self) -> Option<Arc<dyn crate::lyrics::ServerLyrics>> {
        Some(self.core.clone())
    }
    fn radio(&self) -> Option<Arc<dyn ServerRadio>> {
        Some(self.core.clone())
    }
    fn activity(&self) -> Option<Arc<dyn crate::social::activity::ActivitySource>> {
        self.core.caps().has(Caps::FRIEND_ACTIVITY).then(|| self.core.clone() as Arc<dyn crate::social::activity::ActivitySource>)
    }
    fn file_paths(&self) -> Option<Arc<dyn crate::lidarr::PlexMusic>> {
        self.core.caps().has(Caps::REAL_FILE_PATHS).then(|| self.core.clone() as Arc<dyn crate::lidarr::PlexMusic>)
    }
}

/// Track ids by server tag (for asking each server about its own songs).
pub fn by_tag(ids: &[String]) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for id in ids {
        if let Some(t) = crate::sid::tag_of(id) {
            m.entry(t.to_string()).or_default().push(id.clone());
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_are_local() {
        for local in ["http://localhost:8096", "http://10.0.0.5:8096", "http://172.20.1.1", "http://den.local:8096", "http://nas:8096", "http://[::1]:8096", "http://[fd00::5]"] {
            assert_eq!(connection_for(local), ConnectionKind::Local, "{local}");
        }
        for remote in ["https://music.example.com", "http://8.8.8.8", "https://[2001:db8::1]"] {
            assert_eq!(connection_for(remote), ConnectionKind::Remote, "{remote}");
        }
    }

    #[test]
    fn moves_count_without_the_moved_row() {
        let e = |i: &str| PlaylistEntry { track_id: format!("t{i}"), item_id: Some(i.into()) };
        let rows = vec![e("a"), e("b"), e("c"), e("d")];
        assert_eq!(move_index(&rows, "a", Some("c")).unwrap(), 2, "a after c: b c a d");
        assert_eq!(move_index(&rows, "d", None).unwrap(), 0);
        assert_eq!(move_index(&rows, "b", Some("d")).unwrap(), 3);
        assert!(move_index(&rows, "x", None).is_err());
        assert!(move_index(&rows, "a", Some("x")).is_err());
    }

    #[test]
    fn caps_follow_the_account() {
        let user = jellyfin_caps(false, true);
        assert!(user.has(Caps::TRANSCODE | Caps::LYRICS | Caps::BACKDATED_SCROBBLE | Caps::PLAYLIST_MOVE));
        assert!(!user.has(Caps::FRIEND_ACTIVITY) && !user.has(Caps::REAL_FILE_PATHS));
        for never in [Caps::LIKE_DATES, Caps::SERVER_SMART_PLAYLISTS, Caps::REMOTE_PLAYERS, Caps::METADATA_EDIT, Caps::HOME_USERS, Caps::PLAYLIST_POSTER, Caps::SERVER_HISTORY, Caps::RELEASE_TYPES] {
            assert!(!jellyfin_caps(true, true).has(never));
        }
        assert!(!jellyfin_caps(false, false).has(Caps::TRANSCODE));
        assert!(jellyfin_caps(true, false).has(Caps::FRIEND_ACTIVITY | Caps::REAL_FILE_PATHS));
    }

    #[test]
    fn lyrics_are_timed_when_lines_have_starts() {
        let dto: LyricDto = serde_json::from_value(serde_json::json!({
            "Metadata": {"Author": "Someone"},
            "Lyrics": [{"Text": "One", "Start": 12_340_000u64}, {"Text": "Two", "Start": 50_000_000u64}]
        }))
        .unwrap();
        let l = lyrics_of(dto).unwrap();
        assert!(l.synced);
        assert_eq!(l.timed.iter().map(|t| t.start_ms).collect::<Vec<_>>(), [1234, 5000]);
        assert_eq!(l.credit.as_deref(), Some("Written by Someone"));
        let plain: LyricDto = serde_json::from_value(serde_json::json!({"Lyrics": [{"Text": "Just words"}]})).unwrap();
        let l = lyrics_of(plain).unwrap();
        assert!(!l.synced && l.timed.is_empty());
        assert!(lyrics_of(LyricDto::default()).is_none());
    }
}
