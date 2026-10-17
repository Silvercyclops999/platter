//! Public cover art for Discord. Discord's servers fetch the image, so it
//! has to be a public HTTPS URL; Plex art needs the server and its token and
//! never leaves the computer as a URL.
//!
//! 1. Cover Art Archive, by the album's MusicBrainz release id from Plex
//!    (only the id is sent).
//! 2. For a released album without one, the ids the release check found
//!    for an album Plex couldn't match (`artistinfo::releasecheck`), else a
//!    MusicBrainz search by album artist and title; only a sure match is
//!    taken, and its release group's cover on Cover Art Archive is used.
//! 3. Otherwise the 512 px cover is uploaded and its link kept: to Imgur
//!    with the user's own client id, else (or when Imgur fails) to the
//!    keyless hosts in [`super::hosts`] in order, when cover uploads are on.
//!    A link from a host that deletes files is used until about 12 hours
//!    before it goes, then the cover is uploaded again.
//! 4. Otherwise nothing (the activity uses the app icon).
//!
//! Unreleased albums skip steps 1 and 2: their cover is only ever uploaded,
//! and only when unreleased covers may be shown.
//!
//! Answers are kept per album in the `discord_art` table. A cover uploaded
//! during a lookup is fetched once right away, so the host serves it by the
//! time Discord's image proxy asks; a host that answers that fetch with
//! something other than an image is skipped. When Discord's proxy can't
//! show an uploaded cover, [`ArtResolver::replace`] uploads it to the next
//! host.

use std::collections::HashSet;
use std::io::Cursor;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::activity::image_allowed;
pub use super::hosts::{CATBOX_UPLOAD, CoverHost, Host};
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
/// Cover size uploaded and asked of Cover Art Archive.
pub const COVER_PX: u32 = 512;
/// A JPEG cover up to this size and [`COVER_PX`] is uploaded as it is;
/// anything else is made a [`COVER_PX`] JPEG first.
pub const SMALL_COVER_BYTES: usize = 200 * 1024;
pub const JPEG_QUALITY: u8 = 85;
/// An uploaded link is replaced when less than this is left before its host
/// deletes it (or half its life, for a host that keeps files only hours).
pub const REUPLOAD_MARGIN: i64 = 12 * 3600;

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
    /// "caa", or the upload host's name.
    pub source: String,
    pub fetched_at: i64,
    pub expires_at: i64,
    /// The upload host's name (None for Cover Art Archive).
    pub host: Option<String>,
    /// When the upload host deletes the file (None: kept, or not an upload).
    pub link_expires_at: Option<i64>,
}

impl Row {
    /// The upload host of this row's link, if it's an upload.
    pub fn upload_host(&self) -> Option<Host> {
        self.url
            .as_deref()
            .and_then(Host::from_url)
            .or_else(|| self.host.as_deref().and_then(Host::from_name))
            .or_else(|| Host::from_name(&self.source))
    }

    /// An uploaded link with enough time left before its host deletes it.
    pub fn link_fresh(&self, now: i64) -> bool {
        match self.link_expires_at {
            None => true,
            Some(end) => {
                let life = (end - self.fetched_at).max(0);
                end - now > REUPLOAD_MARGIN.min(life / 2)
            }
        }
    }
}

pub fn get(c: &Connection, album_id: &str) -> AppResult<Option<Row>> {
    Ok(c.query_row(
        "SELECT url, source, fetched_at, expires_at, host, link_expires_at FROM discord_art WHERE album_id = ?1",
        params![album_id],
        |r| {
            Ok(Row {
                url: r.get(0)?,
                source: r.get(1)?,
                fetched_at: r.get(2)?,
                expires_at: r.get(3)?,
                host: r.get(4)?,
                link_expires_at: r.get(5)?,
            })
        },
    )
    .optional()?)
}

/// A Cover Art Archive answer (a link, or a miss).
pub fn put(c: &Connection, album_id: &str, url: Option<&str>, source: &str, now: i64, expires_at: i64) -> AppResult<()> {
    put_row(c, album_id, url, source, now, expires_at, None, None)
}

/// An uploaded link, with its host and when the host deletes it.
pub fn put_upload(c: &Connection, album_id: &str, url: &str, host: Host, now: i64, link_expires_at: Option<i64>) -> AppResult<()> {
    let expires_at = link_expires_at.unwrap_or(i64::MAX);
    put_row(c, album_id, Some(url), host.name(), now, expires_at, Some(host.name()), link_expires_at)
}

#[allow(clippy::too_many_arguments)]
fn put_row(
    c: &Connection,
    album_id: &str,
    url: Option<&str>,
    source: &str,
    now: i64,
    expires_at: i64,
    host: Option<&str>,
    link_expires_at: Option<i64>,
) -> AppResult<()> {
    c.execute(
        "INSERT INTO discord_art (album_id, url, source, fetched_at, expires_at, host, link_expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (album_id) DO UPDATE SET url = excluded.url, source = excluded.source,
             fetched_at = excluded.fetched_at, expires_at = excluded.expires_at,
             host = excluded.host, link_expires_at = excluded.link_expires_at",
        params![album_id, url, source, now, expires_at, host, link_expires_at],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- image

/// The cover as uploaded: a small JPEG is sent as it is, anything else is
/// scaled to fit [`COVER_PX`] and made a JPEG. If it can't be read it goes
/// as it came.
pub fn prepare_cover(bytes: Vec<u8>, content_type: String) -> (Vec<u8>, String) {
    let jpeg = content_type.contains("jpeg") || content_type.contains("jpg");
    let dims = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok());
    if jpeg && bytes.len() <= SMALL_COVER_BYTES && dims.is_some_and(|(w, h)| w.max(h) <= COVER_PX) {
        return (bytes, content_type);
    }
    match reencode(&bytes) {
        Ok(out) => (out, "image/jpeg".into()),
        Err(e) => {
            tracing::debug!("couldn't make the cover a JPEG, sending it as it is: {e}");
            (bytes, content_type)
        }
    }
}

fn reencode(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let img = crate::platform::art::decode_limited(bytes)?;
    let img = if img.width().max(img.height()) > COVER_PX {
        img.resize(COVER_PX, COVER_PX, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let rgb = image::DynamicImage::ImageRgb8(img.to_rgb8());
    let mut out = Vec::new();
    rgb.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)).map_err(|e| e.to_string())?;
    Ok(out)
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
    /// Upload to the keyless hosts (after Imgur, when it's used).
    pub anon: bool,
    /// Which keyless hosts.
    pub hosts: CoverHost,
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

    /// The key state if the key was read already, without reading it.
    pub fn known_key_state(&self) -> Option<KeyState> {
        let saved = self.key.lock().as_ref()?.is_some();
        Some(KeyState { saved, rejected: *self.rejected.lock() })
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
        if let Some(row) = &cached {
            match (&row.url, row.source.as_str()) {
                (Some(u), "caa") if row.expires_at > now && req.use_archive && !req.unreleased => {
                    return ArtOutcome::Url(u.clone());
                }
                (None, "caa") if row.expires_at > now => archive_missed = true,
                (Some(u), _) if row.link_fresh(now) && row.upload_host().is_some_and(|h| allowed(req, h)) => {
                    return ArtOutcome::Url(u.clone());
                }
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
        self.upload(req, now, &[]).await
    }

    /// Discord's proxy couldn't show `failed` (the covers tried so far this
    /// play): upload the cover to the next host that isn't one of theirs.
    /// None when none is left or the covers weren't uploads.
    pub async fn replace(&self, req: &ArtRequest, failed: &[String]) -> ArtOutcome {
        let exclude: Vec<Host> = failed.iter().filter_map(|u| Host::from_url(u)).collect();
        if exclude.is_empty() {
            return ArtOutcome::None;
        }
        self.upload(req, now_s(), &exclude).await
    }

    /// A public cover from Cover Art Archive: by the album's release id, else
    /// by a sure MusicBrainz match on artist and title. The answer is cached
    /// either way; a step that failed (offline, Plex down, MusicBrainz busy)
    /// leaves a miss uncached so it's tried again next time.
    async fn archive(&self, req: &ArtRequest, now: i64) -> AppResult<Option<String>> {
        let mut complete = true;
        let mut url = None;
        // Ids the release check found for an album Plex couldn't match.
        let id = req.album_id.clone();
        let (stored_group, stored_release) =
            match self.db.read_async(move |c| crate::artistinfo::releasecheck::found_ids(c, &id)).await {
                Ok(Some((g, r))) => (Some(g), r),
                _ => (None, None),
            };
        let plex_release = match stored_release {
            Some(r) => Ok(Some(r)),
            None => self.mbids.release_mbid(&req.album_id).await,
        };
        match plex_release {
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
            let group = match stored_group {
                Some(g) => Ok(Some(g)),
                None => mb.release_group_id(&req.artist, &req.album).await,
            };
            match group {
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
    /// then to the keyless hosts in order when they're on, leaving out
    /// `exclude`. The first host that gives a direct image link wins.
    async fn upload(&self, req: &ArtRequest, now: i64, exclude: &[Host]) -> ArtOutcome {
        let Some(thumb) = req.thumb.as_deref() else { return ArtOutcome::None };
        let imgur_key = if req.upload { self.key() } else { None };
        let rejected = *self.rejected.lock();
        let mut hosts = Vec::new();
        if imgur_key.is_some() && !rejected {
            hosts.push(Host::Imgur);
        }
        if req.anon {
            hosts.extend_from_slice(req.hosts.chain());
        }
        hosts.retain(|h| !exclude.contains(h));
        if hosts.is_empty() {
            if imgur_key.is_some() && exclude.is_empty() {
                return ArtOutcome::UploadFailed("Imgur refused the client ID".into());
            }
            return ArtOutcome::None;
        }
        if exclude.is_empty() && self.failed.lock().contains(&req.album_id) {
            return ArtOutcome::UploadFailed("an earlier upload failed".into());
        }
        let cover = match self.covers.cover(thumb, COVER_PX).await {
            Ok((bytes, ty)) => tokio::task::spawn_blocking(move || prepare_cover(bytes, ty))
                .await
                .map_err(|e| AppError::Internal(e.to_string())),
            Err(e) => Err(e),
        };
        let (bytes, ty) = match cover {
            Ok(c) => c,
            Err(e) => {
                tracing::info!("cover upload failed: {e}");
                self.failed.lock().insert(req.album_id.clone());
                return ArtOutcome::UploadFailed(e.to_string());
            }
        };
        let mut last = String::new();
        for host in hosts {
            let sent = match (host, &imgur_key) {
                (Host::Imgur, Some(k)) => self.try_upload(k, &bytes, &ty).await,
                _ => self.try_keyless(host, &bytes, &ty).await,
            };
            let url = match sent {
                Ok(u) => u,
                Err(e) => {
                    tracing::info!("cover upload to {} failed: {e}", host.label());
                    last = e.to_string();
                    continue;
                }
            };
            if !self.warm(&url).await {
                tracing::info!("{} doesn't serve the cover as an image; trying the next host", host.label());
                last = format!("{} gave no direct image link", host.label());
                continue;
            }
            let link_expires_at = host.adapter().and_then(|a| a.keeps(bytes.len())).map(|t| now + t);
            let (id, u) = (req.album_id.clone(), url.clone());
            if let Err(e) = self.db.write_async(move |c| put_upload(c, &id, &u, host, now, link_expires_at)).await {
                tracing::debug!("discord art cache: {e}");
            }
            return ArtOutcome::Url(url);
        }
        if exclude.is_empty() {
            self.failed.lock().insert(req.album_id.clone());
        }
        ArtOutcome::UploadFailed(last)
    }

    /// Fetch a fresh upload once before Discord sees it, so the host has the
    /// file ready when Discord's image proxy asks. False only when the host
    /// answers with something other than an image (a page, not the file); a
    /// failed fetch changes nothing.
    async fn warm(&self, url: &str) -> bool {
        match self.http.get(url, &[]).await {
            Ok(r) if r.ok() => r
                .content_type
                .as_deref()
                .is_none_or(|t| t.trim().to_ascii_lowercase().starts_with("image/")),
            Ok(r) => {
                tracing::debug!("the fresh cover answered {}", r.status);
                true
            }
            Err(e) => {
                tracing::debug!("fetching the fresh cover: {e}");
                true
            }
        }
    }

    async fn try_upload(&self, key: &Token, bytes: &[u8], ty: &str) -> AppResult<String> {
        let (body, ct) = multipart(bytes, ty);
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

    /// An upload to a keyless host, through its adapter.
    async fn try_keyless(&self, host: Host, bytes: &[u8], ty: &str) -> AppResult<String> {
        let Some(adapter) = host.adapter() else {
            return Err(AppError::Internal(format!("{} needs a key", host.label())));
        };
        let form = adapter.form(bytes, ty);
        let r = self.upload.post(form.url, &[], &form.content_type, form.body).await?;
        adapter.link(&r)
    }
}

/// Whether the request lets a link on `host` be used.
fn allowed(req: &ArtRequest, host: Host) -> bool {
    match host {
        Host::Imgur => req.upload,
        h => req.anon && req.hosts.chain().contains(&h),
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
