//! Source images go through the same disk cache as Plex art. Their art path
//! is `/ext/<https url>`; [`RoutedArt`] sends those to the image hosts of the
//! info sources (and nowhere else) and everything else to Plex.

use std::sync::Arc;

use async_trait::async_trait;

use super::http::InfoHttp;
use crate::db::ArtFetcher;
use crate::db::artcache::sniff_content_type;
use crate::db::source::ArtBytes;
use crate::error::{AppError, AppResult};

pub const EXT_PREFIX: &str = "/ext/";

/// Image hosts the info sources serve from, plus plex.tv for the account's
/// profile picture (it redirects to assets.plex.tv and needs no token).
const ALLOWED_HOSTS: &[&str] = &[
    "assets.fanart.tv",
    "r2.theaudiodb.com",
    "www.theaudiodb.com",
    "theaudiodb.com",
    "plex.tv",
    "assets.plex.tv",
    // Previews of the artwork a server offers for an item.
    "images.plex.tv",
];

/// Largest image accepted from a source.
const MAX_BYTES: usize = 12 * 1024 * 1024;

/// The https URL inside an `/ext/` art path, when it points at an allowed host.
pub fn ext_url(path: &str) -> Option<url::Url> {
    let raw = path.strip_prefix(EXT_PREFIX)?;
    let u = url::Url::parse(raw).ok()?;
    (u.scheme() == "https" && u.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h))).then_some(u)
}

pub struct RoutedArt {
    pub plex: Arc<dyn ArtFetcher>,
    pub http: Arc<dyn InfoHttp>,
}

#[async_trait]
impl ArtFetcher for RoutedArt {
    async fn fetch(&self, path: &str, width: u32, height: u32) -> AppResult<ArtBytes> {
        if !path.starts_with(EXT_PREFIX) {
            return self.plex.fetch(path, width, height).await;
        }
        let url = ext_url(path).ok_or_else(|| AppError::Invalid("image host not allowed".into()))?;
        let resp = self.http.get(url.as_str(), &[]).await?;
        match resp.status {
            404 => return Err(AppError::NotFound("image".into())),
            s if !(200..300).contains(&s) => return Err(AppError::Server(format!("image host answered {s}"))),
            _ => {}
        }
        if resp.body.len() > MAX_BYTES {
            return Err(AppError::Invalid("image too large".into()));
        }
        let content_type = match resp.content_type.as_deref() {
            Some(ct) if ct.starts_with("image/") => ct.to_owned(),
            _ => sniff_content_type(&resp.body).to_owned(),
        };
        if !content_type.starts_with("image/") {
            return Err(AppError::Decode("not an image".into()));
        }
        Ok(ArtBytes { bytes: resp.body, content_type })
    }
}

#[cfg(test)]
mod tests {
    use super::super::http::fake::FakeHttp;
    use super::super::http::HttpResponse;
    use super::*;

    struct PlexOnly;
    #[async_trait]
    impl ArtFetcher for PlexOnly {
        async fn fetch(&self, path: &str, _: u32, _: u32) -> AppResult<ArtBytes> {
            Ok(ArtBytes { bytes: path.as_bytes().to_vec().into(), content_type: "image/jpeg".into() })
        }
    }

    #[tokio::test]
    async fn routes_ext_paths_to_allowed_hosts_only() {
        let http = Arc::new(FakeHttp::new());
        http.route_resp(
            "assets.fanart.tv",
            HttpResponse { status: 200, body: vec![0xFF, 0xD8, 0xFF, 0xE0].into(), ..Default::default() },
        );
        let art = RoutedArt { plex: Arc::new(PlexOnly), http: http.clone() };
        let plex = art.fetch("/library/metadata/1/thumb/2", 10, 10).await.unwrap();
        assert_eq!(&plex.bytes[..], b"/library/metadata/1/thumb/2");
        let img = art.fetch("/ext/https://assets.fanart.tv/fanart/music/a.jpg", 10, 10).await.unwrap();
        assert_eq!(img.content_type, "image/jpeg");
        assert!(art.fetch("/ext/https://evil.example/a.jpg", 10, 10).await.is_err());
        assert!(art.fetch("/ext/http://assets.fanart.tv/a.jpg", 10, 10).await.is_err());
        assert_eq!(http.urls(), ["https://assets.fanart.tv/fanart/music/a.jpg"]);
    }

    #[tokio::test]
    async fn plex_avatar_is_fetched_from_plex_tv() {
        let http = Arc::new(FakeHttp::new());
        http.route_resp(
            "plex.tv",
            HttpResponse { status: 200, body: vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A].into(), ..Default::default() },
        );
        let art = RoutedArt { plex: Arc::new(PlexOnly), http: http.clone() };
        let img = art.fetch("/ext/https://plex.tv/users/abc/avatar?c=1", 64, 64).await.unwrap();
        assert_eq!(img.content_type, "image/png");
        assert_eq!(http.urls(), ["https://plex.tv/users/abc/avatar?c=1"]);
        assert!(ext_url("/ext/https://assets.plex.tv/avatars/a.png").is_some());
        assert!(ext_url("/ext/https://other.plex.tv/a.png").is_none());
    }
}
