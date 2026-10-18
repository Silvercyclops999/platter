//! An OpenSubsonic server (Navidrome first) as one of the app's music
//! servers: the library for sync, streams, downloads, file reads for the tag
//! scan, stars as likes, playlists, play reports, lyrics, cover art,
//! similar songs, what others are playing and the saved queue. What it can
//! do comes from its extension list, never from its name.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use serde::de::DeserializeOwned;

use super::client::{OFFLINE, SIGN_IN_AGAIN, SubsonicClient, is_sign_in_again};
use super::map;
use super::types::{
    Album as SAlbum, AlbumList, Artist as SArtist, ArtistInfo, Artists, Found, Indexes, LyricsList, MusicFolders,
    NowPlayingList, PlayQueue, Playlist as SPlaylist, Playlists, ScanStatus, Song, SongList, SonicMatch, User,
};
use crate::audio::dsp::TrackAudio;
use crate::audio::source::{StreamSource, TrackResolver};
use crate::backend::server::{MusicServer, ResumeQueue, SavedQueue, ServerRadio};
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
use crate::jellyfin::server::{ServerState, View, connection_for};
use crate::jellyfin::types::unix_s;
use crate::likes::LikeRemote;
use crate::model::{Playlist, Track};
use crate::playlists::PlaylistRemote;
use crate::plex::types::ConnectionKind;
use crate::sid::{key, qualify_any};
use crate::tags::scan::PartSource;
use crate::tracking::{PlayReport, PlayReporter, PlaybackState};

/// Most items a list call returns.
pub const PAGE: u64 = 500;
/// How long the artist list is reused while a sync pages through it.
const ARTISTS_FRESH: Duration = Duration::from_secs(120);

pub const NO_DOWNLOAD: &str = "Your account on this server isn't allowed to download songs. The server's owner can allow it.";
pub const NO_STREAM: &str = "Your account on this server isn't allowed to play songs.";
const NO_POSTER: &str = "Playlist covers on this server can't be changed from Platter.";
const ROW_GONE: &str = "That song isn't in the playlist any more.";

/// Extension names Platter uses.
pub mod ext {
    pub const API_KEY: &str = "apiKeyAuthentication";
    pub const FORM_POST: &str = "formPost";
    pub const LYRICS: &str = "songLyrics";
    pub const SONIC: &str = "sonicSimilarity";
    pub const PLAYBACK_REPORT: &str = "playbackReport";
    pub const INDEX_QUEUE: &str = "indexBasedQueue";
    pub const TRANSCODE_OFFSET: &str = "transcodeOffset";
}

/// What the server says about itself: whether it's OpenSubsonic and its
/// extensions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Features {
    pub open: bool,
    pub names: HashSet<String>,
}

impl Features {
    pub fn new(open: bool, names: impl IntoIterator<Item = String>) -> Self {
        Self { open, names: names.into_iter().collect() }
    }
    pub fn has(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

/// What the server can do, from its extension list.
pub fn subsonic_caps(f: &Features) -> Caps {
    // Likes are stars; the server's own 1 to 5 ratings aren't read or
    // set, so star ratings aren't claimed.
    let mut c = Caps::LIKE_DATES
        | Caps::PLAYLIST_MOVE
        | Caps::PLAYLIST_SUMMARY
        | Caps::TRANSCODE
        | Caps::SERVER_RADIO
        | Caps::FRIEND_ACTIVITY
        | Caps::RESUME_QUEUE
        | Caps::BACKDATED_SCROBBLE;
    if f.open {
        // OpenSubsonic answers carry ReplayGain and release types.
        c |= Caps::LOUDNESS | Caps::RELEASE_TYPES;
    }
    if f.has(ext::LYRICS) {
        c |= Caps::LYRICS | Caps::SYNCED_LYRICS;
    }
    if f.has(ext::SONIC) {
        c |= Caps::SONIC;
    }
    c
}

struct Session {
    client: SubsonicClient,
    username: String,
    user: User,
}

/// Everything about one server; each of the app's interfaces is
/// implemented on it.
pub struct Core {
    tag: String,
    name: RwLock<String>,
    unsigned: SubsonicClient,
    session: RwLock<Option<Session>>,
    features: RwLock<Features>,
    views: RwLock<Vec<View>>,
    include: RwLock<Option<Vec<String>>>,
    state: Mutex<ServerState>,
    prefs: Arc<StreamPrefs>,
    db: Option<DbHandle>,
    /// The song now reported as playing.
    playing: Mutex<Option<String>>,
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    offline: RwLock<Arc<std::sync::atomic::AtomicBool>>,
    /// The artist list while a sync pages through it.
    artists: Mutex<Option<(String, Instant, Vec<SArtist>)>>,
    /// Whether `search3` with an empty query lists every song (None: not
    /// asked yet). Servers where it doesn't get their songs album by album.
    search_lists_all: Mutex<Option<bool>>,
    /// Songs gathered album by album, while a sync pages through them.
    album_walk: Mutex<Option<(String, Vec<Track>)>>,
    /// Where the next page of songs starts on the server: (section, songs
    /// handed out so far, server offset). Entries that aren't songs are
    /// skipped, so the two offsets drift apart.
    song_cursor: Mutex<Option<(String, u64, u64)>>,
    /// Change polling: what the server said last time.
    last_modified: Mutex<Option<u64>>,
    last_scan: Mutex<Option<String>>,
    /// The extension list was read from the server in this run (not only
    /// remembered from the last one).
    features_read: std::sync::atomic::AtomicBool,
    /// Called when the server answers again after being out of reach while
    /// its extensions haven't been read yet.
    on_back: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

fn now() -> i64 {
    crate::db::queries::now_s()
}

fn p(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
}

impl Core {
    pub fn new(tag: &str, name: &str, unsigned: SubsonicClient, prefs: Arc<StreamPrefs>, db: Option<DbHandle>) -> Self {
        Self {
            tag: tag.into(),
            name: RwLock::new(name.into()),
            unsigned,
            session: RwLock::new(None),
            features: RwLock::new(Features::default()),
            views: RwLock::new(Vec::new()),
            include: RwLock::new(None),
            state: Mutex::new(ServerState::Unknown),
            prefs,
            db,
            playing: Mutex::new(None),
            on_change: Mutex::new(None),
            offline: RwLock::new(Arc::default()),
            artists: Mutex::new(None),
            search_lists_all: Mutex::new(None),
            album_walk: Mutex::new(None),
            song_cursor: Mutex::new(None),
            last_modified: Mutex::new(None),
            last_scan: Mutex::new(None),
            features_read: std::sync::atomic::AtomicBool::new(false),
            on_back: Mutex::new(None),
        }
    }

    /// The extension list came from the server in this run.
    pub fn features_read(&self) -> bool {
        self.features_read.load(Ordering::Relaxed)
    }

    pub fn mark_features_read(&self) {
        self.features_read.store(true, Ordering::Relaxed);
    }

    /// What to do when the server answers again with its extensions unread.
    pub fn on_back(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *self.on_back.lock() = Some(f);
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

    pub fn set_features(&self, f: Features) {
        let changed = *self.features.read() != f;
        *self.features.write() = f;
        if changed {
            self.changed();
        }
    }

    pub fn features(&self) -> Features {
        self.features.read().clone()
    }

    fn has(&self, ext: &str) -> bool {
        self.features.read().has(ext)
    }

    /// Signed in as `username` with `client`.
    pub fn sign_in(&self, client: SubsonicClient, username: &str, user: User) {
        let client = client.with_form_post(self.has(ext::FORM_POST));
        *self.session.write() = Some(Session { client, username: username.to_string(), user });
        *self.state.lock() = ServerState::Unknown;
        self.changed();
    }

    pub fn signed_in(&self) -> bool {
        self.session.read().is_some()
    }

    pub fn user(&self) -> Option<User> {
        self.session.read().as_ref().map(|s| s.user.clone())
    }

    pub fn username(&self) -> Option<String> {
        self.session.read().as_ref().map(|s| s.username.clone())
    }

    pub fn state(&self) -> ServerState {
        if !self.signed_in() {
            return ServerState::SignInAgain;
        }
        *self.state.lock()
    }

    fn set_state(&self, s: ServerState) {
        let before = std::mem::replace(&mut *self.state.lock(), s);
        if before != s && (s == ServerState::SignInAgain || before == ServerState::SignInAgain) {
            self.changed();
        }
        // Out of reach at start: what it can do is asked once it answers.
        if s == ServerState::Online && before != ServerState::Online && !self.features_read() {
            let f = self.on_back.lock().clone();
            if let Some(f) = f {
                f();
            }
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
        subsonic_caps(&self.features.read())
    }

    pub fn may_download(&self) -> bool {
        self.user().and_then(|u| u.download_role).unwrap_or(true)
    }

    fn may_stream(&self) -> bool {
        self.user().and_then(|u| u.stream_role).unwrap_or(true)
    }

    /// Follow the app's Go offline switch.
    pub fn set_offline_switch(&self, off: Arc<std::sync::atomic::AtomicBool>) {
        *self.offline.write() = off;
    }

    pub fn client(&self) -> AppResult<SubsonicClient> {
        if self.offline.read().load(Ordering::Relaxed) {
            return Err(AppError::Network(OFFLINE.into()));
        }
        self.session.read().as_ref().map(|s| s.client.clone()).ok_or_else(|| AppError::Auth(SIGN_IN_AGAIN.into()))
    }

    fn note<T>(&self, r: &AppResult<T>) {
        match r {
            Ok(_) => self.set_state(ServerState::Online),
            Err(AppError::Network(_)) => self.set_state(ServerState::Offline),
            Err(e) if is_sign_in_again(e) => self.set_state(ServerState::SignInAgain),
            Err(_) => {}
        }
    }

    pub async fn get<T: DeserializeOwned + Default>(&self, method: &str, params: &[(&'static str, String)], key: &str) -> AppResult<T> {
        let c = self.client()?;
        let r = c.get(method, params, Some(key)).await;
        self.note(&r);
        r
    }

    async fn call(&self, method: &str, params: &[(&'static str, String)]) -> AppResult<()> {
        let c = self.client()?;
        let r = c.call(method, params).await;
        self.note(&r);
        r
    }

    /// Whether the server answers now (`ping`), and the account's rights again.
    pub async fn check(&self) -> ServerState {
        let Ok(c) = self.client() else { return ServerState::SignInAgain };
        let r = c.ping().await;
        self.note(&r);
        if r.is_ok()
            && let Some(name) = self.username()
            && let Ok(user) = c.get::<User>("getUser", &[("username", name)], Some("user")).await
            && let Some(s) = self.session.write().as_mut()
        {
            s.user = user;
        }
        self.state()
    }

    // ---- libraries --------------------------------------------------------

    /// The server's music folders, read again. A server that lists none has
    /// one library: everything.
    pub async fn load_views(&self) -> AppResult<Vec<View>> {
        let r: MusicFolders = self.get("getMusicFolders", &[], "musicFolders").await?;
        let mut views: Vec<View> = r
            .music_folder
            .into_iter()
            .filter(|f| !f.id.is_empty())
            .map(|f| View { title: f.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Music".into()), id: f.id })
            .collect();
        if views.is_empty() {
            views.push(View { id: String::new(), title: "Music".into() });
        }
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

    pub fn included_views(&self) -> Vec<View> {
        let inc = self.include.read().clone();
        self.views.read().iter().filter(|v| inc.as_ref().is_none_or(|i| i.contains(&v.id))).cloned().collect()
    }

    fn sections(&self, only: Option<&str>) -> Vec<MusicSection> {
        let views = match only {
            Some(id) => self.views().into_iter().filter(|v| v.id == id).collect(),
            None => self.included_views(),
        };
        views
            .into_iter()
            .map(|v| MusicSection { uuid: format!("{}/{}", self.tag, v.id), id: v.id, title: v.title })
            .collect()
    }

    /// The `musicFolderId` for a library ("": every folder).
    fn folder(section: &str) -> Vec<(&'static str, String)> {
        if section.is_empty() { Vec::new() } else { vec![("musicFolderId", section.to_string())] }
    }

    async fn all_artists(&self, section: &str) -> AppResult<Vec<SArtist>> {
        if let Some((s, at, list)) = self.artists.lock().as_ref()
            && s == section
            && at.elapsed() < ARTISTS_FRESH
        {
            return Ok(list.clone());
        }
        let r: Artists = self.get("getArtists", &Self::folder(section), "artists").await?;
        let list: Vec<SArtist> = r.index.into_iter().flat_map(|i| i.artist).filter(|a| !a.id.is_empty()).collect();
        *self.artists.lock() = Some((section.to_string(), Instant::now(), list.clone()));
        Ok(list)
    }

    async fn album_page(&self, section: &str, kind: &str, offset: u64, size: u64) -> AppResult<Vec<SAlbum>> {
        let mut q = p(&[("type", kind)]);
        q.push(("size", size.min(PAGE).to_string()));
        q.push(("offset", offset.to_string()));
        q.extend(Self::folder(section));
        Ok(self.get::<AlbumList>("getAlbumList2", &q, "albumList2").await?.album)
    }

    async fn all_albums(&self, section: &str) -> AppResult<Vec<SAlbum>> {
        let mut out = Vec::new();
        loop {
            let page = self.album_page(section, "alphabeticalByName", out.len() as u64, PAGE).await?;
            let n = page.len() as u64;
            out.extend(page);
            if n < PAGE {
                return Ok(out);
            }
        }
    }

    pub async fn album(&self, id: &str) -> AppResult<SAlbum> {
        self.get("getAlbum", &[("id", key(id).to_string())], "album").await
    }

    /// One page of `search3` songs at a server offset: the songs, and how
    /// many entries the server sent (videos and folders included), which is
    /// what tells the end of the list.
    async fn song_page(&self, section: &str, offset: u64, size: u64) -> AppResult<(Vec<Song>, u64)> {
        let mut q = p(&[("query", ""), ("artistCount", "0"), ("albumCount", "0")]);
        q.push(("songCount", size.min(PAGE).to_string()));
        q.push(("songOffset", offset.to_string()));
        q.extend(Self::folder(section));
        let raw = self.get::<Found>("search3", &q, "searchResult3").await?.song;
        let n = raw.len() as u64;
        Ok((raw.into_iter().filter(map::is_song).collect(), n))
    }

    /// Whether `search3` lists songs for an empty query: asked once, by
    /// comparing with the album list.
    async fn songs_by_search(&self, section: &str, first_raw: u64) -> AppResult<bool> {
        if let Some(v) = *self.search_lists_all.lock() {
            return Ok(v);
        }
        let ok = first_raw > 0 || self.album_page(section, "alphabeticalByName", 0, 1).await?.is_empty();
        *self.search_lists_all.lock() = Some(ok);
        Ok(ok)
    }

    /// `size` songs from the `start`th, by search. Entries that aren't songs
    /// don't count, so a page is only short at the real end of the list.
    async fn search_songs(&self, section: &str, start: u64, size: u64) -> AppResult<(Vec<Song>, u64)> {
        let mut raw = match self.song_cursor.lock().as_ref() {
            _ if start == 0 => 0,
            Some((s, handed, raw)) if s == section && *handed == start => *raw,
            _ => start,
        };
        let mut out = Vec::new();
        let mut first_raw = None;
        while (out.len() as u64) < size {
            let want = (size - out.len() as u64).min(PAGE);
            let (page, n) = self.song_page(section, raw, want).await?;
            first_raw.get_or_insert(n);
            raw += n;
            out.extend(page);
            if n < want {
                break;
            }
        }
        *self.song_cursor.lock() = Some((section.to_string(), start + out.len() as u64, raw));
        Ok((out, first_raw.unwrap_or(0)))
    }

    /// Every song, album by album (for servers whose search can't list them).
    async fn walk_albums(&self, section: &str) -> AppResult<Vec<Track>> {
        let mut out = Vec::new();
        for a in self.all_albums(section).await? {
            let full = self.album(&a.id).await?;
            out.extend(full.song.iter().filter(|s| map::is_song(s)).map(|s| map::track(&self.tag, s)));
        }
        Ok(out)
    }

    /// One page of songs, by search or album by album.
    async fn tracks_page(&self, section: &str, start: u64, size: u64) -> AppResult<Vec<Track>> {
        let by_search = *self.search_lists_all.lock();
        if by_search != Some(false) {
            let (page, first_raw) = self.search_songs(section, start, size).await?;
            if start > 0 || self.songs_by_search(section, first_raw).await? {
                return Ok(page.iter().map(|s| map::track(&self.tag, s)).collect());
            }
        }
        if start == 0 || self.album_walk.lock().as_ref().is_none_or(|(s, _)| s != section) {
            let all = self.walk_albums(section).await?;
            *self.album_walk.lock() = Some((section.to_string(), all));
        }
        let mut g = self.album_walk.lock();
        let list = g.as_ref().map(|(_, l)| l.as_slice()).unwrap_or_default();
        let page: Vec<Track> = list.iter().skip(start as usize).take(size as usize).cloned().collect();
        if (page.len() as u64) < size {
            *g = None;
        }
        Ok(page)
    }

    async fn all_track_ids(&self, section: &str) -> AppResult<Vec<String>> {
        let mut out = Vec::new();
        loop {
            let page = self.tracks_page(section, out.len() as u64, PAGE).await?;
            let n = page.len() as u64;
            out.extend(page.into_iter().map(|t| t.id));
            if n < PAGE {
                return Ok(out);
            }
        }
    }

    /// Whether the library changed since the last ask (the scan finished
    /// again, or the index changed). The first ask only remembers. A scan
    /// still running waits for the next ask.
    pub async fn poll_changes(&self) -> AppResult<bool> {
        let scan: ScanStatus = self.get("getScanStatus", &[], "scanStatus").await?;
        if scan.scanning == Some(true) {
            return Ok(false);
        }
        let since = self.last_modified.lock().unwrap_or(0);
        let idx: Indexes = self.get("getIndexes", &[("ifModifiedSince", since.to_string())], "indexes").await?;
        let mut changed = false;
        if let Some(m) = idx.last_modified {
            let mut g = self.last_modified.lock();
            changed |= g.is_some_and(|before| m > before);
            *g = Some(g.map_or(m, |b| b.max(m)));
        }
        if let Some(s) = scan.last_scan {
            let mut g = self.last_scan.lock();
            changed |= g.as_ref().is_some_and(|before| *before != s);
            *g = Some(s);
        }
        Ok(changed)
    }

    // ---- streams ------------------------------------------------------------

    fn source(&self, url: url::Url, codec: Option<String>, size: Option<u64>) -> StreamSource {
        StreamSource { url: url.to_string(), headers: Vec::new(), size, codec_hint: codec }
    }

    /// The original file, as it is (Range works).
    fn raw_stream(&self, track: &Track) -> AppResult<StreamSource> {
        if !self.may_stream() {
            return Err(AppError::Auth(NO_STREAM.into()));
        }
        let c = self.client()?;
        let url = c.url("stream", &p(&[("id", key(&track.id)), ("format", "raw")]))?;
        Ok(self.source(url, track.container.clone().or_else(|| track.codec.clone()), None))
    }

    /// The song converted to MP3 at `kbps` by the server.
    fn mp3(&self, track: &Track, kbps: u32) -> AppResult<StreamSource> {
        if !self.may_stream() {
            return Err(AppError::Auth(NO_STREAM.into()));
        }
        let c = self.client()?;
        let url = c.url("stream", &p(&[("id", key(&track.id)), ("format", "mp3"), ("maxBitRate", &kbps.max(32).to_string())]))?;
        Ok(self.source(url, Some("mp3".into()), None))
    }

    async fn song(&self, id: &str) -> AppResult<Song> {
        self.get("getSong", &[("id", key(id).to_string())], "song").await
    }

    // ---- friend activity, as the shape the panel reads ------------------------

    fn account_number(&self, user: &str) -> i64 {
        if self.username().is_some_and(|u| u.eq_ignore_ascii_case(user)) {
            return crate::social::activity::OWNER_ACCOUNT;
        }
        let h = crate::sid::tag(&format!("subsonic-user:{}", user.to_ascii_lowercase()));
        i64::from_str_radix(&h, 16).unwrap_or(2).max(2)
    }

    /// `getNowPlaying` as the panel's session list (without this app).
    pub fn sessions_json(&self, list: &NowPlayingList) -> serde_json::Value {
        let me = self.username().unwrap_or_default();
        let rows: Vec<serde_json::Value> = list
            .entry
            .iter()
            .filter(|e| !(e.username.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(&me)) && e.player_name.as_deref() == Some(super::client::CLIENT_NAME)))
            .filter(|e| map::is_song(&e.song))
            .filter_map(|e| {
                let user = e.username.as_deref().filter(|u| !u.is_empty())?;
                let t = map::track(&self.tag, &e.song);
                Some(serde_json::json!({
                    "type": "track",
                    "ratingKey": t.id,
                    "title": t.title,
                    "originalTitle": t.track_artist,
                    "grandparentTitle": t.artist_title,
                    "parentTitle": t.album_title,
                    "parentRatingKey": (!t.album_id.is_empty()).then_some(t.album_id),
                    "parentThumb": t.thumb,
                    "User": { "id": self.account_number(user), "title": user },
                    "Player": { "state": "playing", "title": e.player_name },
                }))
            })
            .collect();
        serde_json::json!({ "MediaContainer": { "Metadata": rows } })
    }

    /// Set a star rating (Platter's 0 to 10 scale; None clears it).
    pub async fn rate(&self, track_id: &str, rating: Option<f64>) -> AppResult<()> {
        let stars = match rating {
            None => 0,
            Some(r) if r.is_finite() => (r / 2.0).round().clamp(0.0, 5.0) as u32,
            Some(_) => return Err(AppError::Invalid("not a rating".into())),
        };
        self.call("setRating", &p(&[("id", key(track_id)), ("rating", &stars.to_string())])).await
    }
}

fn json_bytes(v: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

// ---- the library ----------------------------------------------------------------

/// One server's library for the sync engine: every included folder (the
/// main sync picks one), or one folder on its own. Subsonic can't list what
/// changed since a time; see [`LibrarySource::changed_since`] here.
pub struct SubsonicLibrary {
    core: Arc<Core>,
    only: Option<String>,
}

impl SubsonicLibrary {
    pub fn all(core: Arc<Core>) -> Self {
        Self { core, only: None }
    }
    pub fn one(core: Arc<Core>, view: &str) -> Self {
        Self { core, only: Some(view.to_string()) }
    }
}

#[async_trait]
impl LibrarySource for SubsonicLibrary {
    async fn server_tag(&self) -> Option<String> {
        Some(self.core.tag.clone())
    }

    async fn music_sections(&self) -> AppResult<Vec<MusicSection>> {
        if self.core.views().is_empty() {
            self.core.load_views().await?;
        }
        Ok(self.core.sections(self.only.as_deref()))
    }

    /// What the server says it has, so a list that ends early (a capped
    /// page, a scan in progress) isn't taken for the whole library: the
    /// sync then leaves deletions for another time. Albums are the sum of
    /// each artist's album count; songs the scan's count, which covers
    /// every folder, so only for a server with one. While the server scans,
    /// one more than it has so far: the list isn't complete yet. 0: not
    /// known (counted as they arrive).
    async fn count(&self, section_id: &str, kind: ItemKind) -> AppResult<u64> {
        let scanning = || async {
            let scan: ScanStatus = self.core.get("getScanStatus", &[], "scanStatus").await.unwrap_or_default();
            scan
        };
        match kind {
            ItemKind::Artist => Ok(self.core.all_artists(section_id).await?.len() as u64),
            ItemKind::Album => {
                let artists = self.core.all_artists(section_id).await?;
                if artists.iter().any(|a| a.album_count.is_none()) {
                    return Ok(0);
                }
                let n: u64 = artists.iter().filter_map(|a| a.album_count).sum();
                Ok(if scanning().await.scanning == Some(true) { n + 1 } else { n })
            }
            ItemKind::Track => {
                if self.core.views().len() > 1 {
                    return Ok(0);
                }
                let scan = scanning().await;
                let n = scan.count.unwrap_or(0);
                Ok(if scan.scanning == Some(true) { n + 1 } else { n })
            }
        }
    }

    async fn artists(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<crate::model::Artist>> {
        let all = self.core.all_artists(section_id).await?;
        Ok(all.iter().skip(start as usize).take(size as usize).map(|a| map::artist(&self.core.tag, a)).collect())
    }

    async fn albums(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<crate::model::Album>> {
        let page = self.core.album_page(section_id, "alphabeticalByName", start, size).await?;
        Ok(page.iter().map(|a| map::album(&self.core.tag, a)).collect())
    }

    async fn tracks(&self, section_id: &str, start: u64, size: u64) -> AppResult<Vec<Track>> {
        self.core.tracks_page(section_id, start, size).await
    }

    /// Albums added since `since` (the newest first, until an older one),
    /// albums played since then (for play counts), each with its songs, and
    /// their artists. Edits to existing albums arrive with the weekly full
    /// pass; likes with every sync.
    async fn changed_since(&self, section_id: &str, since: i64) -> AppResult<Changes> {
        let mut picked: Vec<SAlbum> = Vec::new();
        let mut seen = HashSet::new();
        let mut offset = 0;
        'newest: loop {
            let page = self.core.album_page(section_id, "newest", offset, PAGE).await?;
            let n = page.len() as u64;
            for a in page {
                if a.created.as_deref().and_then(unix_s).is_some_and(|c| c < since) {
                    break 'newest;
                }
                if seen.insert(a.id.clone()) {
                    picked.push(a);
                }
            }
            if n < PAGE {
                break;
            }
            offset += n;
        }
        for a in self.core.album_page(section_id, "recent", 0, 50).await? {
            if a.played.as_deref().and_then(unix_s).is_some_and(|t| t >= since) && seen.insert(a.id.clone()) {
                picked.push(a);
            }
        }
        let mut out = Changes::default();
        let mut artist_ids = HashSet::new();
        for a in &picked {
            let full = self.core.album(&a.id).await?;
            if let Some(id) = full.artist_id.clone().or_else(|| full.artists.first().map(|x| x.id.clone())) {
                artist_ids.insert(id);
            }
            out.tracks.extend(full.song.iter().filter(|s| map::is_song(s)).map(|s| map::track(&self.core.tag, s)));
            out.albums.push(map::album(&self.core.tag, &full));
        }
        if !artist_ids.is_empty() {
            let all = self.core.all_artists(section_id).await?;
            out.artists = all.iter().filter(|a| artist_ids.contains(&a.id)).map(|a| map::artist(&self.core.tag, a)).collect();
        }
        Ok(out)
    }

    async fn all_ids(&self, section_id: &str, kind: ItemKind) -> AppResult<Vec<String>> {
        let tag = &self.core.tag;
        match kind {
            ItemKind::Artist => Ok(self.core.all_artists(section_id).await?.iter().map(|a| qualify_any(tag, &a.id)).collect()),
            ItemKind::Album => Ok(self.core.all_albums(section_id).await?.iter().map(|a| qualify_any(tag, &a.id)).collect()),
            ItemKind::Track => self.core.all_track_ids(section_id).await,
        }
    }

    /// Subsonic keeps play counts and the last play, not a history.
    async fn history(&self, _section_id: &str, _start: u64, _size: u64) -> AppResult<Vec<HistoryEntry>> {
        Ok(Vec::new())
    }

    async fn playlists(&self) -> AppResult<Vec<Playlist>> {
        let r: Playlists = self.core.get("getPlaylists", &[], "playlists").await?;
        Ok(r.playlist.iter().map(|x| map::playlist(&self.core.tag, x)).collect())
    }

    async fn playlist_items(&self, playlist_id: &str) -> AppResult<Vec<String>> {
        Ok(self.playlist_entries(playlist_id).await?.into_iter().map(|e| e.track_id).collect())
    }

    async fn playlist_entries(&self, playlist_id: &str) -> AppResult<Vec<PlaylistEntry>> {
        Ok(entries(&self.core.tag, &read_playlist(&self.core, playlist_id).await?))
    }

    /// Starred songs, with the date each was starred.
    async fn liked_tracks(&self, section_id: &str) -> AppResult<Option<Vec<Rated>>> {
        let r: Found = self.core.get("getStarred2", &Core::folder(section_id), "starred2").await?;
        let t = now();
        Ok(Some(
            r.song
                .iter()
                .filter(|s| map::is_song(s))
                .map(|s| Rated {
                    track_id: qualify_any(&self.core.tag, &s.id),
                    rating: LIKED_RATING,
                    rated_at: Some(s.starred.as_deref().and_then(unix_s).unwrap_or(t)),
                })
                .collect(),
        ))
    }

    async fn artist_similar(&self, artist_id: &str) -> AppResult<Vec<SimilarArtist>> {
        let r: ArtistInfo = self.core.get("getArtistInfo2", &p(&[("id", key(artist_id)), ("count", "20")]), "artistInfo2").await?;
        Ok(r.similar_artist
            .into_iter()
            .filter_map(|a| {
                let name = a.name.filter(|n| !n.trim().is_empty())?;
                Some(SimilarArtist { id: (!a.id.is_empty()).then(|| qualify_any(&self.core.tag, &a.id)), name })
            })
            .collect())
    }
}

async fn read_playlist(core: &Core, id: &str) -> AppResult<SPlaylist> {
    core.get("getPlaylist", &[("id", key(id).to_string())], "playlist").await
}

/// A playlist's rows; each row's id is its index and its song.
fn entries(tag: &str, pl: &SPlaylist) -> Vec<PlaylistEntry> {
    pl.entry
        .iter()
        .enumerate()
        .map(|(i, s)| PlaylistEntry { track_id: qualify_any(tag, &s.id), item_id: Some(map::row_id(i, &s.id)) })
        .collect()
}

/// Where the row named `item_id` is in the playlist as it is now: at its
/// old index while that still holds its song, else the nearest row with it.
pub fn find_row(songs: &[String], item_id: &str) -> AppResult<usize> {
    let (index, song) = map::parse_row(item_id).ok_or_else(|| AppError::NotFound(ROW_GONE.into()))?;
    if songs.get(index).is_some_and(|s| s == song) {
        return Ok(index);
    }
    songs
        .iter()
        .enumerate()
        .filter(|(_, s)| *s == song)
        .min_by_key(|(i, _)| i.abs_diff(index))
        .map(|(i, _)| i)
        .ok_or_else(|| AppError::NotFound(ROW_GONE.into()))
}

/// The song order after moving the row at `from` to just after the row at
/// `after` (None: to the top).
pub fn reorder(songs: &[String], from: usize, after: Option<usize>) -> Vec<String> {
    let mut out: Vec<String> = songs.to_vec();
    let moved = out.remove(from);
    let to = match after {
        None => 0,
        Some(a) if a < from => a + 1,
        Some(a) => a,
    };
    out.insert(to.min(out.len()), moved);
    out
}

// ---- search and art ---------------------------------------------------------

#[async_trait]
impl RemoteSearch for Core {
    async fn search(&self, query: &str, section_id: Option<&str>, limit: u32) -> AppResult<RemoteHits> {
        let n = limit.max(1).to_string();
        let mut q = p(&[("query", query), ("artistCount", &n), ("albumCount", &n), ("songCount", &n)]);
        q.extend(Core::folder(section_id.unwrap_or_default()));
        let r: Found = self.get("search3", &q, "searchResult3").await?;
        Ok(RemoteHits {
            artists: r.artist.iter().map(|a| map::artist(&self.tag, a)).collect(),
            albums: r.album.iter().map(|a| map::album(&self.tag, a)).collect(),
            tracks: r.song.iter().filter(|s| map::is_song(s)).map(|s| map::track(&self.tag, s)).collect(),
            playlists: Vec::new(),
        })
    }
}

#[async_trait]
impl ArtFetcher for Core {
    /// The server resizes the picture (`getCoverArt`).
    async fn fetch(&self, path: &str, width: u32, height: u32) -> AppResult<ArtBytes> {
        let id = key(path).strip_prefix("/cover/").ok_or_else(|| AppError::Invalid("not a cover path".into()))?;
        let c = self.client()?;
        let url = c.url("getCoverArt", &p(&[("id", id), ("size", &width.max(height).max(1).to_string())]))?;
        let resp = match ss_http().get(url).send().await {
            Ok(r) => r,
            Err(_) => {
                self.set_state(ServerState::Offline);
                return Err(AppError::Network(OFFLINE.into()));
            }
        };
        super::client::check_http(resp.status())?;
        let declared = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_string);
        let bytes = crate::web::body_capped(resp, crate::web::SERVER_MAX, |_| AppError::Network(OFFLINE.into())).await?;
        if declared.as_deref().is_some_and(|t| t.contains("json") || t.contains("xml")) {
            // An error answer instead of a picture.
            let (status, _) = super::client::envelope(&bytes)?;
            super::client::check_status(&status, c.secret())?;
            return Err(AppError::NotFound("no cover".into()));
        }
        let content_type = declared
            .filter(|t| t.starts_with("image/"))
            .unwrap_or_else(|| crate::db::artcache::sniff_content_type(&bytes).to_string());
        Ok(ArtBytes { bytes, content_type })
    }
}

fn ss_http() -> reqwest::Client {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP.get_or_init(super::client::http).clone()
}

// ---- streams, downloads, file reads ---------------------------------------------

#[async_trait]
impl TrackResolver for Core {
    async fn stream(&self, track: &Track) -> AppResult<StreamSource> {
        let conn = Some(connection_for(self.base()));
        match crate::bridge::stream_cap_kbps(conn, track.bitrate, self.prefs.home(), self.prefs.away()) {
            Some(kbps) => self.mp3(track, kbps),
            None => self.raw_stream(track),
        }
    }

    async fn transcode(&self, track: &Track) -> AppResult<StreamSource> {
        self.mp3(track, 320)
    }

    /// ReplayGain from the song's details, kept in the cache after the
    /// first ask.
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
        match self.song(&track.id).await {
            Ok(s) => {
                let rg = s.replay_gain.unwrap_or_default();
                let f = |v: Option<f64>| v.filter(|g| g.is_finite()).map(|g| g as f32);
                a.gain_db = f(rg.track_gain);
                a.album_gain_db = f(rg.album_gain);
                a.peak = f(rg.track_peak).filter(|p| *p > 0.0);
                a.album_peak = f(rg.album_peak).filter(|p| *p > 0.0);
                if let Some(db) = db {
                    let stored = a;
                    if let Err(e) = db.write_async(move |c| crate::db::trackaudio::set_loudness(c, &id, &stored, now())).await {
                        tracing::debug!("could not store loudness: {e}");
                    }
                }
            }
            Err(e) => tracing::debug!("no replaygain from the server: {}", e.kind()),
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
                let url = c.url("download", &p(&[("id", key(&track.id))]))?;
                Ok(Fetch { src: self.source(url, track.container.clone(), None), resumable: true, session: None })
            }
            Some(k) => Ok(Fetch { src: self.mp3(track, k)?, resumable: false, session: None }),
        }
    }

    async fn finished(&self, _fetch: &Fetch) {}
}

#[async_trait]
impl PartSource for Core {
    async fn part(&self, track_id: &str, _part_key: &str) -> AppResult<StreamSource> {
        let c = self.client()?;
        let url = c.url("stream", &p(&[("id", key(track_id)), ("format", "raw")]))?;
        Ok(self.source(url, None, None))
    }
}

// ---- likes, playlists, plays ------------------------------------------------------

#[async_trait]
impl LikeRemote for Core {
    async fn online(&self) -> bool {
        self.signed_in() && !matches!(self.state(), ServerState::Offline | ServerState::SignInAgain)
    }

    /// Star or unstar (the server keeps the date).
    async fn set_liked(&self, track_id: &str, liked: bool) -> AppResult<()> {
        self.call(if liked { "star" } else { "unstar" }, &p(&[("id", key(track_id))])).await
    }
}

fn ids_param(name: &'static str, ids: &[String]) -> Vec<(&'static str, String)> {
    ids.iter().map(|i| (name, key(i).to_string())).collect()
}

#[async_trait]
impl PlaylistRemote for Core {
    async fn online(&self) -> bool {
        LikeRemote::online(self).await
    }

    async fn create(&self, title: &str, track_ids: &[String]) -> AppResult<String> {
        let mut q = p(&[("name", title)]);
        q.extend(ids_param("songId", track_ids));
        let made: SPlaylist = self.get("createPlaylist", &q, "playlist").await?;
        if !made.id.is_empty() {
            return Ok(qualify_any(&self.tag, &made.id));
        }
        // Servers before 1.14 don't answer with the playlist: the newest
        // with that name is it.
        let all: Playlists = self.get("getPlaylists", &[], "playlists").await?;
        all.playlist
            .iter()
            .filter(|x| x.name.as_deref() == Some(title))
            .max_by_key(|x| x.created.as_deref().and_then(unix_s).unwrap_or(0))
            .map(|x| qualify_any(&self.tag, &x.id))
            .ok_or_else(|| AppError::Server("the new playlist didn't show up".into()))
    }

    async fn add(&self, id: &str, track_ids: &[String]) -> AppResult<()> {
        let mut q = p(&[("playlistId", key(id))]);
        q.extend(ids_param("songIdToAdd", track_ids));
        self.call("updatePlaylist", &q).await
    }

    /// Rows are indexes: the playlist is read again first, so the row
    /// removed is the one that holds the song now.
    async fn remove(&self, id: &str, item_id: &str) -> AppResult<()> {
        let pl = read_playlist(self, id).await?;
        let songs: Vec<String> = pl.entry.iter().map(|s| s.id.clone()).collect();
        let at = find_row(&songs, item_id)?;
        self.call("updatePlaylist", &p(&[("playlistId", key(id)), ("songIndexToRemove", &at.to_string())])).await
    }

    /// Subsonic can't move a row: the playlist is written again in the
    /// new order (read again first).
    async fn move_item(&self, id: &str, item_id: &str, after: Option<&str>) -> AppResult<()> {
        let pl = read_playlist(self, id).await?;
        let songs: Vec<String> = pl.entry.iter().map(|s| s.id.clone()).collect();
        let from = find_row(&songs, item_id)?;
        let after = after.map(|a| find_row(&songs, a)).transpose()?;
        let order = reorder(&songs, from, after);
        let mut q = p(&[("playlistId", key(id))]);
        q.extend(order.iter().map(|s| ("songId", s.clone())));
        self.call("createPlaylist", &q).await
    }

    async fn update(&self, id: &str, title: Option<&str>, summary: Option<&str>) -> AppResult<()> {
        let mut q = p(&[("playlistId", key(id))]);
        if let Some(t) = title {
            q.push(("name", t.to_string()));
        }
        if let Some(s) = summary {
            q.push(("comment", s.to_string()));
        }
        if q.len() == 1 {
            return Ok(());
        }
        self.call("updatePlaylist", &q).await
    }

    async fn poster(&self, _id: &str, _bytes: Vec<u8>, _content_type: &str) -> AppResult<()> {
        Err(AppError::Invalid(NO_POSTER.into()))
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.call("deletePlaylist", &p(&[("id", key(id))])).await
    }

    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)> {
        let pl = read_playlist(self, id).await?;
        Ok((map::playlist(&self.tag, &pl), entries(&self.tag, &pl)))
    }
}

/// Play reports. With `playbackReport` the server shows what's playing and
/// where, told not to count the play; without it a "now playing" scrobble
/// does that. Each finished play is counted once, by a scrobble with the
/// time it happened.
#[async_trait]
impl PlayReporter for Core {
    async fn online(&self) -> bool {
        LikeRemote::online(self).await
    }

    async fn timeline(&self, r: &PlayReport) -> AppResult<()> {
        let id = key(&r.track_id).to_string();
        let started = {
            let mut g = self.playing.lock();
            if g.as_deref() == Some(r.track_id.as_str()) {
                false
            } else {
                *g = Some(r.track_id.clone());
                true
            }
        };
        if r.state == PlaybackState::Stopped {
            *self.playing.lock() = None;
            if started {
                return Ok(());
            }
        }
        if !self.has(ext::PLAYBACK_REPORT) {
            if started && r.state != PlaybackState::Stopped {
                return self.call("scrobble", &p(&[("id", &id), ("submission", "false")])).await;
            }
            return Ok(());
        }
        let report = |state: &str| {
            p(&[("mediaId", &id), ("mediaType", "song"), ("positionMs", &r.time_ms.to_string()), ("state", state), ("ignoreScrobble", "true")])
        };
        let state = match r.state {
            PlaybackState::Stopped => "stopped",
            PlaybackState::Paused => "paused",
            PlaybackState::Playing | PlaybackState::Buffering => "playing",
        };
        if started {
            self.call("reportPlayback", &report("starting")).await?;
        }
        self.call("reportPlayback", &report(state)).await
    }

    async fn scrobble(&self, track_id: &str, at: Option<i64>) -> AppResult<()> {
        let ms = at.unwrap_or_else(now).saturating_mul(1000);
        self.call("scrobble", &p(&[("id", key(track_id)), ("time", &ms.to_string()), ("submission", "true")])).await
    }
}

#[async_trait]
impl crate::artistinfo::service::MbidLookup for Core {
    async fn artist_mbid(&self, artist_id: &str) -> AppResult<Option<String>> {
        let a: SArtist = self.get("getArtist", &[("id", key(artist_id).to_string())], "artist").await?;
        Ok(a.music_brainz_id.filter(|m| !m.trim().is_empty()))
    }
    async fn album_mbid(&self, album_id: &str) -> AppResult<Option<String>> {
        Ok(self.album(album_id).await?.music_brainz_id.filter(|m| !m.trim().is_empty()))
    }
}

/// The server's lyrics as the app's: the main (or first) set, timed when
/// the server says they're synced.
pub fn lyrics_of(list: LyricsList) -> Option<crate::plex::details::Lyrics> {
    use crate::plex::details::{LyricLine, Lyrics, LyricsSource};
    let pick = list
        .structured_lyrics
        .iter()
        .filter(|l| l.kind.as_deref().is_none_or(|k| k.eq_ignore_ascii_case("main")))
        .max_by_key(|l| l.synced == Some(true))
        .or_else(|| list.structured_lyrics.first())?;
    if pick.line.iter().all(|l| l.value.trim().is_empty()) {
        return None;
    }
    let synced = pick.synced == Some(true) && pick.line.iter().any(|l| l.start.is_some());
    let offset = pick.offset.unwrap_or(0);
    let timed = if synced {
        pick.line
            .iter()
            .filter_map(|l| {
                // A positive offset shows the lines sooner (OpenSubsonic).
                let at = (l.start? - offset).clamp(0, u32::MAX as i64) as u32;
                Some(LyricLine { start_ms: at, text: l.value.clone(), words: Vec::new() })
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(Lyrics { synced, lines: pick.line.iter().map(|l| l.value.clone()).collect(), timed, credit: None, source: LyricsSource::Plex })
}

#[async_trait]
impl crate::lyrics::ServerLyrics for Core {
    async fn lyrics(&self, track_id: &str) -> AppResult<Option<crate::plex::details::Lyrics>> {
        if !self.has(ext::LYRICS) {
            return Ok(None);
        }
        match self.get::<LyricsList>("getLyricsBySongId", &[("id", key(track_id).to_string())], "lyricsList").await {
            Ok(l) => Ok(lyrics_of(l)),
            Err(AppError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl ServerRadio for Core {
    /// Songs like the seed: sonically similar ones when the server analyses
    /// sound, else its similar songs, else the artist's top songs.
    async fn mix(&self, seed_id: &str, n: usize) -> AppResult<Vec<String>> {
        let count = n.clamp(1, 200).to_string();
        let raw = key(seed_id).to_string();
        let ids = |songs: Vec<Song>| -> Vec<String> {
            songs.iter().filter(|s| map::is_song(s) && s.id != raw).map(|s| qualify_any(&self.tag, &s.id)).collect()
        };
        if self.has(ext::SONIC)
            && let Ok(m) = self.get::<Vec<SonicMatch>>("getSonicSimilarTracks", &p(&[("id", &raw), ("count", &count)]), "sonicMatch").await
        {
            let out = ids(m.into_iter().map(|x| x.entry).collect());
            if !out.is_empty() {
                return Ok(out);
            }
        }
        let similar = self.get::<SongList>("getSimilarSongs", &p(&[("id", &raw), ("count", &count)]), "similarSongs").await;
        match similar {
            Ok(l) if !l.song.is_empty() => return Ok(ids(l.song)),
            Ok(_) | Err(AppError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        // An artist seed: its top songs.
        let artist = match self.get::<SArtist>("getArtist", &[("id", raw.clone())], "artist").await {
            Ok(a) => a,
            Err(AppError::NotFound(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let Some(name) = artist.name.filter(|n| !n.trim().is_empty()) else { return Ok(Vec::new()) };
        let top: SongList = self.get("getTopSongs", &p(&[("artist", &name), ("count", &count)]), "topSongs").await?;
        Ok(ids(top.song))
    }
}

#[async_trait]
impl crate::social::activity::ActivitySource for Core {
    /// Everyone on a Subsonic server sees what the others are playing.
    async fn owned(&self) -> Option<bool> {
        (self.signed_in() && self.state() != ServerState::Offline).then_some(true)
    }

    async fn sessions(&self) -> AppResult<Vec<u8>> {
        let list: NowPlayingList = self.get("getNowPlaying", &[], "nowPlaying").await?;
        Ok(json_bytes(&self.sessions_json(&list)))
    }

    /// Subsonic keeps no play history of other people.
    async fn history(&self, _size: u64) -> AppResult<Vec<u8>> {
        Ok(json_bytes(&serde_json::json!({ "MediaContainer": {} })))
    }

    /// The people playing now (listing every account needs an
    /// administrator).
    async fn accounts(&self) -> AppResult<Vec<u8>> {
        let list: NowPlayingList = self.get("getNowPlaying", &[], "nowPlaying").await?;
        let mut names: Vec<String> = list.entry.iter().filter_map(|e| e.username.clone()).filter(|u| !u.is_empty()).collect();
        names.extend(self.username());
        names.sort();
        names.dedup();
        let rows: Vec<serde_json::Value> = names.iter().map(|n| serde_json::json!({ "id": self.account_number(n), "name": n })).collect();
        Ok(json_bytes(&serde_json::json!({ "MediaContainer": { "Account": rows } })))
    }
}

#[async_trait]
impl ResumeQueue for Core {
    async fn save(&self, track_ids: &[String], current: usize, position_ms: u64) -> AppResult<()> {
        let ids: Vec<&str> = track_ids.iter().filter(|i| crate::sid::tag_of(i) == Some(self.tag.as_str())).map(|i| key(i)).collect();
        if ids.is_empty() {
            return Err(AppError::Invalid("None of these songs are on this server.".into()));
        }
        let current = current.min(ids.len() - 1);
        let mut q: Vec<(&'static str, String)> = ids.iter().map(|i| ("id", i.to_string())).collect();
        q.push(("position", position_ms.to_string()));
        if self.has(ext::INDEX_QUEUE) {
            q.push(("currentIndex", current.to_string()));
            self.call("savePlayQueueByIndex", &q).await
        } else {
            q.push(("current", ids[current].to_string()));
            self.call("savePlayQueue", &q).await
        }
    }

    async fn load(&self) -> AppResult<Option<SavedQueue>> {
        let r = if self.has(ext::INDEX_QUEUE) {
            self.get::<PlayQueue>("getPlayQueueByIndex", &[], "playQueueByIndex").await
        } else {
            self.get::<PlayQueue>("getPlayQueue", &[], "playQueue").await
        };
        let q = match r {
            Ok(q) => q,
            Err(AppError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let songs: Vec<&Song> = q.entry.iter().filter(|s| map::is_song(s)).collect();
        if songs.is_empty() {
            return Ok(None);
        }
        let current = match (q.current_index, q.current.as_deref()) {
            (Some(i), _) => (i as usize).min(songs.len() - 1),
            (None, Some(c)) => songs.iter().position(|s| s.id == c).unwrap_or(0),
            _ => 0,
        };
        Ok(Some(SavedQueue {
            track_ids: songs.iter().map(|s| qualify_any(&self.tag, &s.id)).collect(),
            current,
            position_ms: q.position.unwrap_or(0),
            changed_by: q.changed_by.filter(|c| !c.trim().is_empty()),
            changed_at: q.changed.as_deref().and_then(unix_s),
        }))
    }
}

// ---- the music server ----------------------------------------------------------------

pub struct SubsonicServer {
    core: Arc<Core>,
}

impl SubsonicServer {
    pub fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }
}

#[async_trait]
impl MusicServer for SubsonicServer {
    fn kind(&self) -> BackendKind {
        BackendKind::Subsonic
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
        Some(format!("subsonic:{}", self.core.tag))
    }

    fn library(&self) -> Arc<dyn LibrarySource> {
        Arc::new(SubsonicLibrary::all(self.core.clone()))
    }
    fn libraries(&self) -> Vec<(String, Arc<dyn LibrarySource>)> {
        self.core
            .included_views()
            .into_iter()
            .map(|v| (v.id.clone(), Arc::new(SubsonicLibrary::one(self.core.clone(), &v.id)) as Arc<dyn LibrarySource>))
            .collect()
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
        self.core.caps().has(Caps::LYRICS).then(|| self.core.clone() as Arc<dyn crate::lyrics::ServerLyrics>)
    }
    fn radio(&self) -> Option<Arc<dyn ServerRadio>> {
        Some(self.core.clone())
    }
    fn activity(&self) -> Option<Arc<dyn crate::social::activity::ActivitySource>> {
        Some(self.core.clone())
    }
    fn resume(&self) -> Option<Arc<dyn ResumeQueue>> {
        Some(self.core.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_follow_the_extension_list() {
        let plain = subsonic_caps(&Features::default());
        assert!(plain.has(Caps::LIKE_DATES | Caps::TRANSCODE | Caps::SERVER_RADIO | Caps::RESUME_QUEUE | Caps::BACKDATED_SCROBBLE));
        for never in [Caps::LIVE_UPDATES, Caps::INCREMENTAL_SYNC, Caps::SERVER_SMART_PLAYLISTS, Caps::REMOTE_PLAYERS, Caps::METADATA_EDIT, Caps::HOME_USERS, Caps::PLAYLIST_POSTER, Caps::SERVER_HISTORY, Caps::REAL_FILE_PATHS, Caps::PLAY_QUEUE_HANDOFF] {
            assert!(!subsonic_caps(&Features::new(true, [ext::LYRICS.to_string(), ext::SONIC.to_string()])).has(never));
        }
        assert!(!plain.has(Caps::LYRICS) && !plain.has(Caps::SONIC) && !plain.has(Caps::LOUDNESS));
        let open = subsonic_caps(&Features::new(true, [ext::LYRICS.to_string(), ext::SONIC.to_string()]));
        assert!(open.has(Caps::LYRICS | Caps::SYNCED_LYRICS | Caps::SONIC | Caps::LOUDNESS | Caps::RELEASE_TYPES));
    }

    #[test]
    fn rows_are_found_again_after_the_playlist_changed() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let songs = s(&["a", "b", "a", "c"]);
        assert_eq!(find_row(&songs, "2@a").unwrap(), 2, "still there");
        assert_eq!(find_row(&songs, "1@a").unwrap(), 0, "moved: the nearest copy");
        assert_eq!(find_row(&songs, "3@a").unwrap(), 2);
        assert!(find_row(&songs, "0@z").is_err(), "gone");
        assert!(find_row(&songs, "junk").is_err());
    }

    #[test]
    fn reorders_like_a_move() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let songs = s(&["a", "b", "c", "d"]);
        assert_eq!(reorder(&songs, 0, Some(2)), s(&["b", "c", "a", "d"]), "a after c");
        assert_eq!(reorder(&songs, 3, None), s(&["d", "a", "b", "c"]));
        assert_eq!(reorder(&songs, 3, Some(0)), s(&["a", "d", "b", "c"]));
        assert_eq!(reorder(&songs, 1, Some(3)), s(&["a", "c", "d", "b"]));
    }

    #[test]
    fn lyrics_are_timed_with_the_offset() {
        let l: LyricsList = serde_json::from_value(serde_json::json!({"structuredLyrics": [
            {"kind": "translation", "synced": true, "line": [{"start": 1, "value": "x"}]},
            {"lang": "en", "synced": true, "offset": -100, "line": [{"start": 50, "value": "One"}, {"start": 2000, "value": "Two"}]}
        ]}))
        .unwrap();
        let out = lyrics_of(l).unwrap();
        assert!(out.synced);
        assert_eq!(out.timed.iter().map(|t| t.start_ms).collect::<Vec<_>>(), [150, 2100], "negative: later");
        assert_eq!(out.lines, ["One", "Two"]);
        let sooner: LyricsList = serde_json::from_value(serde_json::json!({"structuredLyrics": [
            {"synced": true, "offset": 500, "line": [{"start": 300, "value": "One"}, {"start": 2000, "value": "Two"}]}
        ]}))
        .unwrap();
        assert_eq!(lyrics_of(sooner).unwrap().timed.iter().map(|t| t.start_ms).collect::<Vec<_>>(), [0, 1500], "positive: sooner");
        let plain: LyricsList = serde_json::from_value(serde_json::json!({"structuredLyrics": [{"synced": false, "line": [{"value": "Words"}]}]})).unwrap();
        let out = lyrics_of(plain).unwrap();
        assert!(!out.synced && out.timed.is_empty());
        assert!(lyrics_of(LyricsList::default()).is_none());
    }
}
