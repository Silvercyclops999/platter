//! Tauri side of lyrics (docs/IPC.md `lyrics_get`, `lyrics_own_*`,
//! `lyrics_export`) and the Plex source.

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::user::{self, OwnLyrics};
use super::{CandidateList, LyricsService, ServerLyrics, Shown};
use crate::error::{AppError, AppResult};
use crate::plex::PlexState;
use crate::plex::details::{Lyrics, fetch_lyrics};

/// Lyrics streams from the signed-in Plex server.
pub struct PlexSource(pub PlexState);

#[async_trait::async_trait]
impl ServerLyrics for PlexSource {
    async fn lyrics(&self, track_id: &str) -> AppResult<Option<Lyrics>> {
        let client = self.0.client_for(track_id).await?;
        fetch_lyrics(&client, track_id).await
    }
}

#[tauri::command]
pub async fn lyrics_get(lyrics: State<'_, LyricsService>, track_id: String) -> AppResult<Option<Shown>> {
    lyrics.shown(&track_id).await
}

/// A song's candidates were judged against the recording: `{ trackId }`.
pub const EV_CHECKED: &str = "lyrics://checked";
/// Lyrics choices changed (from the sync file).
pub const EV_CHOICES: &str = "lyrics://choices";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Checked<'a> {
    track_id: &'a str,
}

impl<R: tauri::Runtime> super::checker::CheckEvents for AppHandle<R> {
    fn checked(&self, track_id: &str) {
        use tauri::Emitter;
        let _ = self.emit(EV_CHECKED, Checked { track_id });
    }
}

/// Every set of lyrics found for the song, for "Wrong lyrics?".
#[tauri::command]
pub async fn lyrics_candidates(lyrics: State<'_, LyricsService>, track_id: String) -> AppResult<CandidateList> {
    lyrics.candidates(&track_id).await
}

/// Pick a song's lyrics: a candidate key, "none", or null for automatic.
#[tauri::command]
pub async fn lyrics_choose(
    lyrics: State<'_, LyricsService>,
    sync: State<'_, crate::sync::commands::SyncService>,
    track_id: String,
    pick: Option<String>,
) -> AppResult<()> {
    let c = lyrics.choose(&track_id, pick).await?;
    sync.store.run(move |s| crate::sync::lyrics::chose(s, &track_id, &c)).await
}

/// Set a song's timing offset in ms (positive: the lyrics come later).
#[tauri::command]
pub async fn lyrics_offset(
    lyrics: State<'_, LyricsService>,
    sync: State<'_, crate::sync::commands::SyncService>,
    track_id: String,
    offset_ms: i32,
) -> AppResult<()> {
    let c = lyrics.set_offset(&track_id, offset_ms).await?;
    sync.store.run(move |s| crate::sync::lyrics::chose(s, &track_id, &c)).await
}

/// How many songs have a lyrics choice (Settings).
#[tauri::command]
pub async fn lyrics_choices_count(lyrics: State<'_, LyricsService>) -> AppResult<usize> {
    lyrics.choice_count().await
}

/// Forget every lyrics choice (Settings). Returns how many there were.
#[tauri::command]
pub async fn lyrics_choices_clear(
    lyrics: State<'_, LyricsService>,
    sync: State<'_, crate::sync::commands::SyncService>,
) -> AppResult<usize> {
    let ids = lyrics.clear_choices().await?;
    let n = ids.len();
    sync.store.run(move |s| crate::sync::lyrics::cleared(s, &ids)).await?;
    Ok(n)
}

#[tauri::command]
pub async fn lyrics_own_get(lyrics: State<'_, LyricsService>, track_id: String) -> AppResult<Option<OwnLyrics>> {
    lyrics.own(&track_id).await
}

#[tauri::command]
pub async fn lyrics_own_save(
    lyrics: State<'_, LyricsService>,
    sync: State<'_, crate::sync::commands::SyncService>,
    track_id: String,
    text: String,
    prefer: bool,
) -> AppResult<()> {
    lyrics.save_own(&track_id, &text, prefer).await?;
    // What was stored (cleaned), so other devices get the same text.
    if let Some(own) = lyrics.own(&track_id).await? {
        sync.store
            .run(move |s| crate::sync::lyrics::saved(s, &own.track_id, &own.text, own.prefer))
            .await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn lyrics_own_delete(
    lyrics: State<'_, LyricsService>,
    sync: State<'_, crate::sync::commands::SyncService>,
    track_id: String,
) -> AppResult<bool> {
    let gone = lyrics.delete_own(&track_id).await?;
    sync.store.run(move |s| crate::sync::lyrics::deleted(s, &track_id)).await?;
    Ok(gone)
}

#[tauri::command]
pub async fn lyrics_own_ids(lyrics: State<'_, LyricsService>) -> AppResult<Vec<String>> {
    lyrics.own_ids().await
}

/// Save lyrics text as an .lrc file wherever the user picks in a save
/// dialog. Returns the saved file's name, or None when they cancelled.
#[tauri::command]
pub async fn lyrics_export(app: AppHandle, text: String, file_name: String) -> AppResult<Option<String>> {
    if text.trim().is_empty() {
        return Err(AppError::Invalid("There are no lyrics to export.".into()));
    }
    if text.len() > user::MAX_BYTES {
        return Err(AppError::Invalid("Those lyrics are too long to export.".into()));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Export lyrics")
        .set_file_name(user::export_file_name(&file_name))
        .add_filter("LRC lyrics", &["lrc"])
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| AppError::Io(format!("export path: {e}")))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let body = if text.ends_with('\n') { text } else { format!("{text}\n") };
    tokio::task::spawn_blocking(move || std::fs::write(&path, body))
        .await
        .map_err(|e| AppError::Internal(format!("export: {e}")))??;
    Ok(Some(name))
}
