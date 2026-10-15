//! Tauri command for the now-playing feed (docs/IPC.md "Now-playing feed").
//! It's turned on and off, and its port chosen, with `settings_set`.

use tauri::State;

use super::FeedStatus;
use super::server::FeedServer;
use crate::error::AppResult;

/// Whether the feed is serving, and where; or why it couldn't start.
/// Changes arrive as `feed://status`.
#[tauri::command]
pub async fn feed_status(feed: State<'_, FeedServer>) -> AppResult<FeedStatus> {
    Ok(feed.status())
}
