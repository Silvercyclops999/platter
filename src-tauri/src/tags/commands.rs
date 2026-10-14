//! Tauri side of the tag scan (docs/IPC.md "File tags"): the commands, the
//! events, reading parts through the Plex client, and the "skipped" toast
//! when a play context leaves songs out.

use std::time::Duration;

use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Runtime, State};

use crate::audio::StreamSource;
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::plex::PlexState;
use crate::plex::types::ConnectionKind;

use super::scan::{PartSource, TagEvents, TagScanStatus, TagScanner};
use super::store::{self, Skipped, TagIndex};

pub const EV_PROGRESS: &str = "tags://progress";
pub const EV_CHANGED: &str = "tags://changed";

const CLIENT_WAIT: Duration = Duration::from_secs(15);

/// Parts through the connected server. Not over the relay: it's slow and
/// capped, so tags wait for a direct connection.
pub struct PlexParts(pub PlexState);

#[async_trait]
impl PartSource for PlexParts {
    async fn part(&self, part_key: &str) -> AppResult<StreamSource> {
        let client = self.0.client_when_ready(CLIENT_WAIT).await?;
        if client.connection() == Some(ConnectionKind::Relay) {
            return Err(AppError::Network("relay only: tags wait for a direct connection".into()));
        }
        let req = client.stream_request(part_key)?;
        Ok(StreamSource {
            url: req.url.to_string(),
            headers: req
                .headers
                .iter()
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
                .collect(),
            size: None,
            codec_hint: None,
        })
    }
}

impl<R: Runtime> TagEvents for AppHandle<R> {
    fn progress(&self, status: &TagScanStatus) {
        let _ = self.emit(EV_PROGRESS, status);
    }
    fn changed(&self) {
        let _ = self.emit(EV_CHANGED, ());
    }
}

/// Toast text for songs a play context left out, if any.
pub fn skipped_message(s: Skipped) -> Option<String> {
    let explicit = |n: usize| if n == 1 { "1 explicit song".to_string() } else { format!("{n} explicit songs") };
    let edits = |n: usize| if n == 1 { "1 fan or AI edit".to_string() } else { format!("{n} fan or AI edits") };
    match (s.explicit, s.edits) {
        (0, 0) => None,
        (e, 0) => Some(format!("Skipped {}. Explicit content is filtered in Settings.", explicit(e))),
        (0, f) => Some(format!("Skipped {}.", edits(f))),
        (e, f) => Some(format!("Skipped {} and {}.", explicit(e), edits(f))),
    }
}

/// Badges, album editions and groups for the UI.
#[tauri::command]
pub async fn tags_index(db: State<'_, DbHandle>) -> AppResult<TagIndex> {
    db.read_async(store::index).await
}

/// Progress of the tag scan (Settings).
#[tauri::command]
pub async fn tag_scan_status(scanner: State<'_, TagScanner>) -> AppResult<TagScanStatus> {
    Ok(scanner.status())
}
