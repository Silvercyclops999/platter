//! Tauri commands for artist info (docs/IPC.md "Artist info").

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::service::{KeyState, KeysStatus};
use super::{ArtistInfo, ArtistInfoService, InfoSource};
use crate::error::{AppError, AppResult};

/// Outside info for an artist page. Answers from the cache at once; missing
/// or stale sources refresh in the background and `artistinfo://changed`
/// fires when something new arrived.
#[tauri::command]
pub async fn artist_info(info: State<'_, ArtistInfoService>, id: String) -> AppResult<ArtistInfo> {
    info.info(&id).await
}

/// An album's original release date, label and credits from MusicBrainz.
/// Answers from the cache at once (`state: "pending"` while a lookup runs);
/// `albumcredits://changed` fires when the lookup is done.
#[tauri::command]
pub async fn album_credits(info: State<'_, ArtistInfoService>, id: String) -> AppResult<super::mbdetails::AlbumCredits> {
    info.album_credits(&id).await
}

/// Whether each keyed source has a key saved on this device, and whether the
/// service refused it this session.
#[tauri::command]
pub async fn artist_info_keys(info: State<'_, ArtistInfoService>) -> AppResult<KeysStatus> {
    let info = info.inner().clone();
    // The first read of each key touches the keychain.
    tokio::task::spawn_blocking(move || info.keys_status())
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// Save a source's API key in the OS keychain (null removes it). The key is
/// checked with one lookup; the answer says whether it was refused.
#[tauri::command]
pub async fn artist_info_key_set(
    info: State<'_, ArtistInfoService>,
    source: InfoSource,
    key: Option<String>,
) -> AppResult<KeyState> {
    info.set_key(source, key).await
}

/// Forget every cached artist info answer. Resolves to how many artists had some.
#[tauri::command]
pub async fn artist_info_cache_clear(info: State<'_, ArtistInfoService>) -> AppResult<u64> {
    info.clear_cache().await
}

/// Sites a credit, source or profile link may open. An artist's official
/// website opens too, but only when it's the one their info lists.
const LINK_HOSTS: &[&str] = &[
    "instagram.com",
    "x.com",
    "facebook.com",
    "youtube.com",
    "bandcamp.com",
    "soundcloud.com",
    "wikipedia.org",
    "www.wikidata.org",
    "www.last.fm",
    "last.fm",
    "fanart.tv",
    "www.theaudiodb.com",
    "theaudiodb.com",
    "musicbrainz.org",
    "listenbrainz.org",
    "creativecommons.org",
];

pub fn link_allowed(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    u.scheme() == "https"
        && u.host_str().is_some_and(|h| LINK_HOSTS.iter().any(|ok| h == *ok || h.ends_with(&format!(".{ok}"))))
}

/// Whether `url` is one of the links shown for the artist (their official website).
pub fn listed_for(info: &ArtistInfo, url: &str) -> bool {
    url.starts_with("https://") && info.links.iter().any(|l| l.url == url)
}

/// Open a credit, source or profile link in the default browser. With
/// `artist_id`, the artist's own listed links (their website) open too.
#[tauri::command]
pub async fn open_info_link(
    app: AppHandle,
    info: State<'_, ArtistInfoService>,
    url: String,
    artist_id: Option<String>,
) -> AppResult<()> {
    let allowed = link_allowed(&url)
        || match artist_id {
            Some(id) => info.info(&id).await.is_ok_and(|i| listed_for(&i, &url)),
            None => false,
        };
    if !allowed {
        return Err(AppError::Invalid("link not allowed".into()));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::Internal(format!("open browser: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_source_and_licence_links_open() {
        assert!(link_allowed("https://en.wikipedia.org/wiki/Band"));
        assert!(link_allowed("https://www.last.fm/music/Band"));
        assert!(link_allowed("https://creativecommons.org/licenses/by-sa/4.0/"));
        assert!(!link_allowed("http://en.wikipedia.org/wiki/Band"));
        assert!(!link_allowed("https://wikipedia.org.evil.example/"));
        assert!(!link_allowed("file:///etc/passwd"));
        assert!(link_allowed("https://www.instagram.com/band/"));
        assert!(link_allowed("https://band.bandcamp.com/"));
        assert!(!link_allowed("https://band.example/"));
    }

    #[test]
    fn an_artists_own_website_opens_only_when_listed() {
        let info = ArtistInfo {
            links: vec![super::super::InfoLink { kind: "website".into(), label: "Official website".into(), url: "https://band.example/".into() }],
            ..Default::default()
        };
        assert!(listed_for(&info, "https://band.example/"));
        assert!(!listed_for(&info, "https://other.example/"));
    }
}
