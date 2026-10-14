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

/// Sites a credit or source link may open.
const LINK_HOSTS: &[&str] = &[
    "wikipedia.org",
    "www.wikidata.org",
    "www.last.fm",
    "last.fm",
    "fanart.tv",
    "www.theaudiodb.com",
    "theaudiodb.com",
    "creativecommons.org",
];

pub fn link_allowed(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    u.scheme() == "https"
        && u.host_str().is_some_and(|h| LINK_HOSTS.iter().any(|ok| h == *ok || h.ends_with(&format!(".{ok}"))))
}

/// Open a credit link (source page or licence) in the default browser.
#[tauri::command]
pub async fn open_info_link(app: AppHandle, url: String) -> AppResult<()> {
    if !link_allowed(&url) {
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
    }
}
