//! Public cover art for Discord. Discord's servers fetch the image, so it
//! has to be a public HTTPS URL; Plex art needs the server and its token and
//! never leaves the computer as a URL.
//!
//! 1. Cover Art Archive, by the album's MusicBrainz release id from Plex
//!    (only the id is sent).
//! 2. For a released album without one, a MusicBrainz search by album
//!    artist and title; only a sure match is taken, and its release group's
//!    cover on Cover Art Archive is used.
//! 3. Otherwise the 512 px cover is uploaded once and its link kept: to
//!    Imgur with the user's own client id, else to catbox.moe when that's on.
//! 4. Otherwise nothing (the activity uses the app icon).
//!
//! Unreleased albums skip steps 1 and 2: their cover is only ever uploaded,
//! and only when unreleased covers may be shown.
//!
//! Answers are kept per album in the `discord_art` table. A cover uploaded
//! during a lookup is fetched once right away, so the host serves it by the
//! time Discord's image proxy asks.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::activity::image_allowed;
use crate::artistinfo::http::{HttpResponse, InfoHttp};
use crate::artistinfo::is_mbid;
use crate::artistinfo::limit::Limiter;
use crate::artistinfo::musicbrainz::MusicBrainz;
use crate::artistinfo::service::KeyState;
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;

/// Keychain account for the Imgur client id.
pub const IMGUR_ACCOUNT: &str = "imgur-client-id";
pub const IMGUR_UPLOAD: &str = "https://api.imgur.com/3/image";
/// A cheap authenticated call that checks a client id without uploading.
pub const IMGUR_CHECK: &str = "https://api.imgur.com/3/credits";
pub const CAA: &str = "https://coverartarchive.org/release";
pub const CAA_GROUP: &str = "https://coverartarchive.org/release-group";
/// catbox.moe's anonymous upload (no account, no key).
pub const CATBOX_UPLOAD: &str = "https://catbox.moe/user/api.php";
/// Cover size uploaded and asked of Cover Art Archive.
pub const COVER_PX: u32 = 512;

const DAY: i64 = 24 * 3600;
/// A Cover Art Archive cover is checked again after a month...
pub const CAA_TTL: i64 = 30 * DAY;
/// ...and an album it lacks after a week.
pub const CAA_MISS_TTL: i64 = 7 * DAY;

/// The album's MusicBrainz release id from Plex.
#[async_trait]
pub trait AlbumMbid: Send + Sync {
    async fn release_mbid(&self, album_id: &str) -> AppResult<Option<String>>;
}

/// A cover image's bytes and type (from the art cache).
#[async_trait]
pub trait CoverBytes: Send + Sync {
    async fn cover(&self, thumb: &str, px: u32) -> AppResult<(Vec<u8>, String)>;
}

/// A POST with a body, for the upload.
#[async_trait]
pub trait UploadHttp: Send + Sync {
    async fn post(&self, url: &str, headers: &[(&'static str, String)], content_type: &str, body: Vec<u8>) -> AppResult<HttpResponse>;
}

// ---------------------------------------------------------------- cache

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub url: Option<String>,
    pub source: String,
    pub expires_at: i64,
}

pub fn get(c: &Connection, album_id: &str) -> AppResult<Option<Row>> {
    Ok(c.query_row(
        "SELECT url, source, expires_at FROM discord_art WHERE album_id = ?1",
        params![album_id],
        |r| Ok(Row { url: r.get(0)?, source: r.get(1)?, expires_at: r.get(2)? }),
    )
    .optional()?)
}

pub fn put(c: &Connection, album_id: &str, url: Option<&str>, source: &str, now: i64, expires_at: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO discord_art (album_id, url, source, fetched_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (album_id) DO UPDATE SET url = excluded.url, source = excluded.source,
             fetched_at = excluded.fetched_at, expires_at = excluded.expires_at",
        params![album_id, url, source, now, expires_at],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- multipart

/// A multipart/form-data body with some text fields and one file, and its
/// content type.
pub fn multipart_form(fields: &[(&str, &str)], file_field: &str, bytes: &[u8], content_type: &str) -> (Vec<u8>, String) {
    let boundary = format!("platter-{}", uuid::Uuid::new_v4().simple());
    let ext = if content_type.contains("png") { "png" } else { "jpg" };
    let mut head = String::new();
    for (k, v) in fields {
        head.push_str(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"));
    }
    head.push_str(&format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"cover.{ext}\"\r\n\
         Content-Type: {content_type}\r\n\r\n"
    ));
    let mut body = Vec::with_capacity(bytes.len() + head.len() + 64);
    body.extend_from_slice(head.as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (body, format!("multipart/form-data; boundary={boundary}"))
}

/// Imgur's upload body (field `image`).
pub fn multipart(bytes: &[u8], content_type: &str) -> (Vec<u8>, String) {
    multipart_form(&[("type", "file")], "image", bytes, content_type)
}

/// Where a cover is uploaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Imgur,
    Catbox,
}

impl Host {
    fn source(self) -> &'static str {
        match self {
            Self::Imgur => "imgur",
            Self::Catbox => "catbox",
        }
    }
}

// ---------------------------------------------------------------- resolver

/// What to look for, for one album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtRequest {
    pub album_id: String,
    /// The Plex art path (only ever read through the art cache).
    pub thumb: Option<String>,
    /// The album artist and title, for the MusicBrainz search.
    pub artist: String,
    pub album: String,
    /// Unreleased albums are never looked up by name or id, only uploaded.
    pub unreleased: bool,
    pub use_archive: bool,
    /// Upload to Imgur (with a client id).
    pub upload: bool,
    /// Upload to catbox.moe (when Imgur isn't used).
    pub catbox: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtOutcome {
    Url(String),
    /// No public cover: the app icon.
    None,
    /// The upload was tried and failed (the app icon, and a note in Settings).
    UploadFailed(String),
}

pub struct ArtResolver {
    db: DbHandle,
    http: Arc<dyn InfoHttp>,
    upload: Arc<dyn UploadHttp>,
    mbids: Arc<dyn AlbumMbid>,
    covers: Arc<dyn CoverBytes>,
    keys: Arc<dyn TokenStore>,
    /// MusicBrainz pacing, shared with the artist pages.
    musicbrainz: Arc<Limiter>,
    /// The client id, read from the keychain once.
    key: Mutex<Option<Option<Token>>>,
    rejected: Mutex<bool>,
    /// Albums whose upload failed this session (not tried again until restart
    /// or a new key).
    failed: Mutex<HashSet<String>>,
}

impl ArtResolver {
    pub fn new(
        db: DbHandle,
        http: Arc<dyn InfoHttp>,
        upload: Arc<dyn UploadHttp>,
        mbids: Arc<dyn AlbumMbid>,
        covers: Arc<dyn CoverBytes>,
        keys: Arc<dyn TokenStore>,
    ) -> Self {
        Self {
            db,
            http,
            upload,
            mbids,
            covers,
            keys,
            musicbrainz: Arc::new(Limiter::per_second(1.0)),
            key: Mutex::new(None),
            rejected: Mutex::new(false),
            failed: Mutex::new(HashSet::new()),
        }
    }

    /// Use the app's MusicBrainz pacing (one request a second in all).
    pub fn with_musicbrainz_limiter(mut self, limiter: Arc<Limiter>) -> Self {
        self.musicbrainz = limiter;
        self
    }

    fn key(&self) -> Option<Token> {
        let mut k = self.key.lock();
        if k.is_none() {
            *k = Some(self.keys.get(IMGUR_ACCOUNT).unwrap_or_else(|e| {
                tracing::warn!("reading the Imgur client id: {e}");
                None
            }));
        }
        k.clone().flatten()
    }

    pub fn key_state(&self) -> KeyState {
        KeyState { saved: self.key().is_some(), rejected: *self.rejected.lock() }
    }

    /// Save (or with None, remove) the client id. A new id is checked with
    /// one request that uploads nothing.
    pub async fn set_key(&self, key: Option<String>) -> AppResult<KeyState> {
        let token = match key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => Some(Token::new(k)?),
            None => None,
        };
        match &token {
            Some(t) => self.keys.set(IMGUR_ACCOUNT, t)?,
            None => self.keys.delete(IMGUR_ACCOUNT)?,
        }
        *self.key.lock() = Some(token.clone());
        *self.rejected.lock() = false;
        self.failed.lock().clear();
        if let Some(t) = token {
            match self.http.get(IMGUR_CHECK, &[("Authorization", format!("Client-ID {}", t.expose()))]).await {
                Ok(r) if r.status == 401 || r.status == 403 => *self.rejected.lock() = true,
                Ok(_) => {}
                Err(e) => tracing::debug!("couldn't check the Imgur client id: {e}"),
            }
        }
        Ok(self.key_state())
    }

    /// Find a public cover for the album. Never panics, never blocks long:
    /// every failure is an outcome.
    pub async fn resolve(&self, req: &ArtRequest) -> ArtOutcome {
        let now = now_s();
        let id = req.album_id.clone();
        let cached = self.db.read_async(move |c| get(c, &id)).await.unwrap_or_else(|e| {
            tracing::debug!("discord art cache: {e}");
            None
        });
        let mut archive_missed = false;
        if let Some(row) = &cached
            && row.expires_at > now
        {
            match (&row.url, row.source.as_str()) {
                (Some(u), "imgur") if req.upload => return ArtOutcome::Url(u.clone()),
                (Some(u), "catbox") if req.catbox => return ArtOutcome::Url(u.clone()),
                (Some(u), "caa") if req.use_archive && !req.unreleased => return ArtOutcome::Url(u.clone()),
                (None, "caa") => archive_missed = true,
                _ => {}
            }
        }
        if req.use_archive && !req.unreleased && !archive_missed {
            match self.archive(req, now).await {
                Ok(Some(u)) => return ArtOutcome::Url(u),
                Ok(None) => {}
                Err(e) => tracing::debug!("cover art archive: {e}"),
            }
        }
        self.upload(req, now).await
    }

    /// A public cover from Cover Art Archive: by the album's release id, else
    /// by a sure MusicBrainz match on artist and title. The answer is cached
    /// either way; a step that failed (offline, Plex down, MusicBrainz busy)
    /// leaves a miss uncached so it's tried again next time.
    async fn archive(&self, req: &ArtRequest, now: i64) -> AppResult<Option<String>> {
        let mut complete = true;
        let mut url = None;
        match self.mbids.release_mbid(&req.album_id).await {
            Ok(Some(mbid)) if is_mbid(&mbid) => match self.front(&format!("{CAA}/{mbid}")).await {
                Ok(u) => url = u,
                Err(e) => {
                    tracing::debug!("cover art archive by release: {e}");
                    complete = false;
                }
            },
            Ok(_) => {}
            Err(e) => {
                tracing::debug!("album release id: {e}");
                complete = false;
            }
        }
        if url.is_none() {
            let mb = MusicBrainz { http: self.http.as_ref(), limiter: &self.musicbrainz };
            match mb.release_group_id(&req.artist, &req.album).await {
                Ok(Some(group)) => match self.front(&format!("{CAA_GROUP}/{group}")).await {
                    Ok(u) => url = u,
                    Err(e) => {
                        tracing::debug!("cover art archive by release group: {e}");
                        complete = false;
                    }
                },
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!("musicbrainz album search: {e:?}");
                    complete = false;
                }
            }
        }
        if url.is_some() || complete {
            let (id, u) = (req.album_id.clone(), url.clone());
            let ttl = if url.is_some() { CAA_TTL } else { CAA_MISS_TTL };
            self.db.write_async(move |c| put(c, &id, u.as_deref(), "caa", now, now + ttl)).await?;
        }
        Ok(url)
    }

    /// `<base>/front-500` when Cover Art Archive has a front cover at `base`
    /// (a release or a release group).
    async fn front(&self, base: &str) -> AppResult<Option<String>> {
        let r = self.http.get(base, &[]).await?;
        match r.status {
            200 => {
                let v: Value = r.json()?;
                let front = v["images"]
                    .as_array()
                    .is_some_and(|imgs| imgs.iter().any(|i| i["front"].as_bool() == Some(true)));
                Ok(front.then(|| format!("{base}/front-500")))
            }
            404 => Ok(None),
            s => Err(AppError::Server(format!("cover art archive answered {s}"))),
        }
    }

    /// Upload the cover: to Imgur when it's on with a working client id,
    /// else to catbox.moe when that's on.
    async fn upload(&self, req: &ArtRequest, now: i64) -> ArtOutcome {
        let Some(thumb) = req.thumb.as_deref() else { return ArtOutcome::None };
        let imgur_key = if req.upload { self.key() } else { None };
        let rejected = *self.rejected.lock();
        let (host, key) = match imgur_key {
            Some(k) if !rejected => (Host::Imgur, Some(k)),
            _ if req.catbox => (Host::Catbox, None),
            Some(_) => return ArtOutcome::UploadFailed("Imgur refused the client ID".into()),
            None => return ArtOutcome::None,
        };
        if self.failed.lock().contains(&req.album_id) {
            return ArtOutcome::UploadFailed("an earlier upload failed".into());
        }
        let sent = match &key {
            Some(k) => self.try_upload(k, thumb).await,
            None => self.try_catbox(thumb).await,
        };
        match sent {
            Ok(url) => {
                let (id, u) = (req.album_id.clone(), url.clone());
                if let Err(e) = self.db.write_async(move |c| put(c, &id, Some(&u), host.source(), now, i64::MAX)).await {
                    tracing::debug!("discord art cache: {e}");
                }
                self.warm(&url).await;
                ArtOutcome::Url(url)
            }
            Err(e) => {
                tracing::info!("cover upload failed: {e}");
                self.failed.lock().insert(req.album_id.clone());
                ArtOutcome::UploadFailed(e.to_string())
            }
        }
    }

    /// Fetch a fresh upload once before Discord sees it, so the host has the
    /// file ready when Discord's image proxy asks. Only the outcome is
    /// logged; a failure here changes nothing.
    async fn warm(&self, url: &str) {
        match self.http.get(url, &[]).await {
            Ok(r) if r.ok() => {}
            Ok(r) => tracing::debug!("the fresh cover answered {}", r.status),
            Err(e) => tracing::debug!("fetching the fresh cover: {e}"),
        }
    }

    async fn try_upload(&self, key: &Token, thumb: &str) -> AppResult<String> {
        let (bytes, ty) = self.covers.cover(thumb, COVER_PX).await?;
        let (body, ct) = multipart(&bytes, &ty);
        let r = self
            .upload
            .post(IMGUR_UPLOAD, &[("Authorization", format!("Client-ID {}", key.expose()))], &ct, body)
            .await?;
        if r.status == 401 || r.status == 403 {
            *self.rejected.lock() = true;
            return Err(AppError::Auth("Imgur refused the client ID".into()));
        }
        if !r.ok() {
            return Err(AppError::Server(format!("Imgur answered {}", r.status)));
        }
        let v: Value = r.json()?;
        let link = v["data"]["link"].as_str().unwrap_or_default().replace("http://", "https://");
        if !image_allowed(&link) {
            return Err(AppError::Decode("Imgur gave no usable link".into()));
        }
        Ok(link)
    }

    /// catbox.moe's anonymous upload. The answer is the file's link as text.
    async fn try_catbox(&self, thumb: &str) -> AppResult<String> {
        let (bytes, ty) = self.covers.cover(thumb, COVER_PX).await?;
        let (body, ct) = multipart_form(&[("reqtype", "fileupload")], "fileToUpload", &bytes, &ty);
        let r = self.upload.post(CATBOX_UPLOAD, &[], &ct, body).await?;
        if !r.ok() {
            return Err(AppError::Server(format!("catbox.moe answered {}", r.status)));
        }
        let link = String::from_utf8_lossy(&r.body).trim().replace("http://", "https://");
        if !link.starts_with("https://files.catbox.moe/") || !image_allowed(&link) {
            return Err(AppError::Decode("catbox.moe gave no usable link".into()));
        }
        Ok(link)
    }
}

/// The real POST client.
pub struct ReqwestUpload {
    client: reqwest::Client,
}

impl ReqwestUpload {
    pub fn new() -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(crate::artistinfo::http::user_agent())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl UploadHttp for ReqwestUpload {
    async fn post(&self, url: &str, headers: &[(&'static str, String)], content_type: &str, body: Vec<u8>) -> AppResult<HttpResponse> {
        let mut req = self.client.post(url).header(reqwest::header::CONTENT_TYPE, content_type).body(body);
        for (k, v) in headers {
            let mut hv = reqwest::header::HeaderValue::from_str(v)
                .map_err(|_| AppError::Invalid("header has invalid characters".into()))?;
            hv.set_sensitive(true);
            req = req.header(*k, hv);
        }
        let resp = req.send().await.map_err(|e| AppError::Network(e.without_url().to_string()))?;
        let status = resp.status().as_u16();
        let body = resp.bytes().await.map_err(|e| AppError::Network(e.without_url().to_string()))?;
        Ok(HttpResponse { status, body, ..Default::default() })
    }
}
