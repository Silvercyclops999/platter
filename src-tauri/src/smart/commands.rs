//! Tauri commands for smart playlists and the AI provider, and the Plex
//! side of [`SmartRemote`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tauri::State;

use super::prompt::{AiProvider, DEFAULT_MODEL};
use super::rules::SmartRules;
use super::{Preview, SaveRequest, Saved, Smart, SmartPage, SmartRemote, SmartSummary};
use crate::db::DbHandle;
use crate::db::source::PlaylistEntry;
use crate::error::{AppError, AppResult};
use crate::model::{Playlist, Track};
use crate::plex::PlexState;

/// The connected server.
pub struct PlexSmart {
    pub plex: PlexState,
    pub db: DbHandle,
}

#[derive(Deserialize)]
struct Env<T> {
    #[serde(rename = "MediaContainer")]
    mc: T,
}

#[derive(Deserialize, Default)]
struct Dirs {
    #[serde(default, rename = "Directory")]
    directory: Vec<Dir>,
}

#[derive(Deserialize)]
struct Dir {
    #[serde(default, deserialize_with = "crate::plex::lenient::string")]
    key: String,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize, Default)]
struct Metas {
    #[serde(default, rename = "Metadata")]
    metadata: Vec<Meta>,
}

#[derive(Deserialize)]
struct Meta {
    #[serde(default)]
    content: Option<String>,
}

/// Plex's library type for a field's scope (`album.genre` -> 9).
fn type_of(field: &str) -> &'static str {
    match field.split('.').next() {
        Some("artist") => "8",
        Some("album") => "9",
        _ => "10",
    }
}

#[async_trait]
impl SmartRemote for PlexSmart {
    async fn online(&self) -> bool {
        self.plex.client().await.is_ok()
    }

    async fn target(&self) -> AppResult<(String, String)> {
        let machine = self.plex.machine_id().await.ok_or_else(|| AppError::Network("Your server is offline.".into()))?;
        let section = self
            .db
            .read_async(crate::db::store::sync_state)
            .await?
            .map(|s| s.section_id)
            .ok_or_else(|| AppError::Invalid("no music library yet".into()))?;
        Ok((machine, section))
    }

    async fn tags(&self, section_id: &str, plex_field: &str) -> AppResult<Vec<(String, String)>> {
        let tag = plex_field.rsplit('.').next().unwrap_or(plex_field);
        if !matches!(tag, "genre" | "style" | "mood") || !section_id.chars().all(|c| c.is_ascii_digit()) {
            return Err(AppError::Invalid(format!("not a tag field: {plex_field}")));
        }
        let path = format!("/library/sections/{section_id}/{tag}");
        let bytes = self.plex.client().await?.get_bytes(&path, &[("type", type_of(plex_field).to_string())]).await?;
        let env: Env<Dirs> = serde_json::from_slice(&bytes).map_err(|e| AppError::Decode(format!("{path}: {e}")))?;
        Ok(env.mc.directory.into_iter().map(|d| (d.key, d.title)).collect())
    }

    async fn create(&self, title: &str, uri: &str) -> AppResult<String> {
        Ok(self.plex.client().await?.create_smart_playlist(title, uri).await?.id)
    }

    async fn update(&self, id: &str, uri: &str) -> AppResult<()> {
        self.plex.client().await?.playlist_set_smart(id, uri).await
    }

    async fn rename(&self, id: &str, title: &str) -> AppResult<()> {
        self.plex.client().await?.playlist_edit(id, Some(title), None).await
    }

    async fn content(&self, id: &str) -> AppResult<String> {
        if !id.chars().all(|c| c.is_ascii_digit()) {
            return Err(AppError::Invalid("not a Plex playlist".into()));
        }
        let path = format!("/playlists/{id}");
        let bytes = self.plex.client().await?.get_bytes(&path, &[]).await?;
        let env: Env<Metas> = serde_json::from_slice(&bytes).map_err(|e| AppError::Decode(format!("{path}: {e}")))?;
        env.mc
            .metadata
            .into_iter()
            .next()
            .and_then(|m| m.content)
            .ok_or_else(|| AppError::Invalid("This isn't a smart playlist.".into()))
    }

    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)> {
        let c = self.plex.client().await?;
        let p = c.playlist(id).await?;
        let rows = c.playlist_rows(id).await?;
        Ok((p, rows.into_iter().map(|(track_id, item_id)| PlaylistEntry { track_id, item_id }).collect()))
    }
}

/// `smart_prompt`: the editor's rules, and the picked songs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResult {
    pub title: String,
    pub rules: SmartRules,
    pub picks: Vec<Track>,
}

/// `ai_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub provider: AiProvider,
    pub model: String,
    pub has_key: bool,
    /// A provider is chosen and its key saved: prompted playlists show.
    pub ready: bool,
}

pub fn ai_status_of(smart: &Smart) -> AiStatus {
    let s = crate::settings::load(&smart.inner.db);
    let has_key = smart.ai().has_key();
    let model = if s.ai_model.trim().is_empty() { DEFAULT_MODEL.to_string() } else { s.ai_model.clone() };
    AiStatus { provider: s.ai_provider, model, has_key, ready: s.ai_provider != AiProvider::None && has_key }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::Internal(e.to_string()))?
}

#[tauri::command]
pub async fn smart_preview(smart: State<'_, Smart>, rules: SmartRules, picks: Option<Vec<String>>) -> AppResult<Preview> {
    smart.preview(rules, picks.unwrap_or_default()).await
}

#[tauri::command]
pub async fn smart_save(smart: State<'_, Smart>, request: SaveRequest) -> AppResult<Saved> {
    smart.save(request).await
}

#[tauri::command]
pub async fn smart_list(smart: State<'_, Smart>) -> AppResult<Vec<SmartSummary>> {
    let s = smart.inner().clone();
    blocking(move || s.list()).await
}

#[tauri::command]
pub async fn smart_page(smart: State<'_, Smart>, id: String) -> AppResult<SmartPage> {
    smart.page(&id).await
}

#[tauri::command]
pub async fn smart_delete(smart: State<'_, Smart>, id: String) -> AppResult<()> {
    smart.delete(&id).await
}

/// A Plex smart playlist's rules, when they map to the editor.
#[tauri::command]
pub async fn smart_plex_rules(smart: State<'_, Smart>, id: String) -> AppResult<SmartRules> {
    smart.plex_rules(&id).await
}

#[tauri::command]
pub async fn smart_prompt(smart: State<'_, Smart>, prompt: String) -> AppResult<PromptResult> {
    let p = smart.prompt(&prompt).await?;
    let ids = p.picks.clone();
    let picks = smart.inner.db.read_async(move |c| crate::db::queries::tracks_by_ids(c, &ids)).await?;
    Ok(PromptResult { title: p.title, rules: p.rules, picks })
}

#[tauri::command]
pub async fn ai_status(smart: State<'_, Smart>) -> AppResult<AiStatus> {
    let s = smart.inner().clone();
    blocking(move || Ok(ai_status_of(&s))).await
}

/// Save (or remove, with null) the AI provider's key in the OS keychain.
#[tauri::command]
pub async fn ai_key_set(smart: State<'_, Smart>, key: Option<String>) -> AppResult<AiStatus> {
    let s = smart.inner().clone();
    blocking(move || {
        s.ai().set_key(key.as_deref())?;
        Ok(ai_status_of(&s))
    })
    .await
}
