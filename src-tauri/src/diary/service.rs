//! The diary service and its commands.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use super::detect::{self, Finished};
use super::export::{self, ExportFormat};
use super::{DiaryEdit, DiaryEntry, ListenedOn, check_rating, clean_note, listened_on};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::sync::SyncStore;
use crate::sync::store::DIARY;

/// The diary's settings, as the service follows them. Cheap to clone.
#[derive(Clone)]
pub struct DiarySwitches {
    /// Keep a listening diary.
    pub on: Arc<AtomicBool>,
    /// Add albums played start to finish automatically.
    pub auto: Arc<AtomicBool>,
}

impl Default for DiarySwitches {
    fn default() -> Self {
        Self { on: Arc::new(AtomicBool::new(true)), auto: Arc::new(AtomicBool::new(true)) }
    }
}

/// An entry as the UI gets it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryItem {
    /// The entry's key.
    pub id: String,
    #[serde(flatten)]
    pub entry: DiaryEntry,
    /// The album's cover now, when it's still in the library.
    pub thumb: Option<String>,
    pub in_library: bool,
    pub listened_on: Option<ListenedOn>,
}

/// Which entries to list (all of them by default).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiaryQuery {
    /// Entries at or after this time (unix ms).
    pub from: Option<i64>,
    /// Entries before this time.
    pub until: Option<i64>,
    pub rated: bool,
    pub with_notes: bool,
    /// Words to find in notes, titles and artists.
    pub text: Option<String>,
    pub album_id: Option<String>,
}

impl DiaryQuery {
    fn matches(&self, e: &DiaryEntry) -> bool {
        if self.from.is_some_and(|f| e.at < f) || self.until.is_some_and(|u| e.at >= u) {
            return false;
        }
        if (self.rated && e.rating.is_none()) || (self.with_notes && e.note.trim().is_empty()) {
            return false;
        }
        if self.album_id.as_ref().is_some_and(|a| *a != e.album_id) {
            return false;
        }
        match self.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            None => true,
            Some(t) => {
                let hay = format!("{}\n{}\n{}", e.note, e.title, e.artist).to_lowercase();
                t.to_lowercase().split_whitespace().all(|w| hay.contains(w))
            }
        }
    }
}

/// `diary_entries` result.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryList {
    /// Newest first.
    pub entries: Vec<DiaryItem>,
    /// Entries in the whole diary, whatever the query.
    pub total: usize,
    /// The oldest and newest entry in the whole diary (unix ms).
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
}

pub trait DiaryEvents: Send + Sync {
    fn changed(&self);
}

impl<R: tauri::Runtime> DiaryEvents for tauri::AppHandle<R> {
    fn changed(&self) {
        use tauri::Emitter;
        if let Err(e) = self.emit("diary://changed", ()) {
            tracing::warn!("diary event: {e}");
        }
    }
}

/// Where music is playing: the output device's name.
#[async_trait]
pub trait ListenPlace: Send + Sync {
    async fn output_name(&self) -> Option<String>;
}

#[async_trait]
impl ListenPlace for crate::audio::player::PlayerHandle {
    async fn output_name(&self) -> Option<String> {
        self.output_state().await.ok()?.current.map(|d| d.name)
    }
}

/// The listening diary. Cheap to clone.
#[derive(Clone)]
pub struct Diary {
    db: DbHandle,
    store: SyncStore,
    switches: DiarySwitches,
    /// This computer's name, for where an entry was heard.
    device: String,
    events: Arc<RwLock<Option<Arc<dyn DiaryEvents>>>>,
    place: Arc<RwLock<Option<Arc<dyn ListenPlace>>>>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// Makes manual keys unique within one millisecond.
    seq: Arc<AtomicU64>,
}

fn system_ms() -> i64 {
    crate::sync::store::system_ms() as i64
}

impl Diary {
    pub fn new(db: DbHandle, store: SyncStore, switches: DiarySwitches, device: impl Into<String>) -> Self {
        Self {
            db,
            store,
            switches,
            device: device.into(),
            events: Arc::default(),
            place: Arc::default(),
            clock: Arc::new(system_ms),
            seq: Arc::default(),
        }
    }

    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    pub fn set_events(&self, e: Arc<dyn DiaryEvents>) {
        *self.events.write() = Some(e);
    }

    pub fn set_place(&self, p: Arc<dyn ListenPlace>) {
        *self.place.write() = Some(p);
    }

    pub fn switches(&self) -> &DiarySwitches {
        &self.switches
    }

    fn on(&self) -> bool {
        self.switches.on.load(Ordering::Relaxed)
    }

    fn changed(&self) {
        if let Some(e) = self.events.read().clone() {
            e.changed();
        }
    }

    async fn output_name(&self) -> Option<String> {
        let place = self.place.read().clone();
        match place {
            Some(p) => p.output_name().await.filter(|n| !n.trim().is_empty()),
            None => None,
        }
    }

    /// A play from `album_id` ended: write an entry for each sitting that
    /// heard it through and has none yet (a deleted one counts). Returns how
    /// many were written.
    pub async fn album_played(&self, album_id: &str) -> AppResult<usize> {
        if !self.on() || !self.switches.auto.load(Ordering::Relaxed) {
            return Ok(0);
        }
        let since = (self.clock)() - detect::LOOKBACK_MS;
        let id = album_id.to_owned();
        let (done, basics) = self
            .db
            .read_async(move |c| Ok((detect::finished_sessions(c, &id, since)?, detect::album_basics(c, &id)?)))
            .await?;
        let Some((title, artist, year, _)) = basics else { return Ok(0) };
        let mut fresh: Vec<Finished> = Vec::new();
        for f in done {
            let key = f.key();
            if !self.store.run(move |s| s.has_entry(&DIARY, &key)).await? {
                fresh.push(f);
            }
        }
        if fresh.is_empty() {
            return Ok(0);
        }
        let output = self.output_name().await;
        let n = fresh.len();
        for f in fresh {
            let entry = DiaryEntry {
                album_id: f.album_id.clone(),
                title: title.clone(),
                artist: artist.clone(),
                year,
                at: f.ended_at,
                started_at: Some(f.started_at),
                songs_played: f.songs_played,
                songs_total: f.songs_total,
                minutes: ((f.ms_listened + 30_000) / 60_000).max(0) as u32,
                device: Some(self.device.clone()).filter(|d| !d.is_empty()),
                output: output.clone(),
                rating: None,
                note: String::new(),
                auto: true,
            };
            let key = f.key();
            self.store.run(move |s| s.put(&DIARY, &key, &entry)).await?;
        }
        self.changed();
        Ok(n)
    }

    async fn item(&self, id: String, entry: DiaryEntry) -> AppResult<DiaryItem> {
        let album = entry.album_id.clone();
        let basics = self.db.read_async(move |c| detect::album_basics(c, &album)).await?;
        Ok(to_item(id, entry, basics.map(|b| b.3)))
    }

    /// Entries matching `q`, newest first.
    pub async fn list(&self, q: &DiaryQuery) -> AppResult<DiaryList> {
        let all = self.store.run(|s| s.list(&DIARY)).await?;
        let total = all.len();
        let first_at = all.iter().map(|(_, e)| e.at).min();
        let last_at = all.iter().map(|(_, e)| e.at).max();
        let mut picked: Vec<(String, DiaryEntry)> = all.into_iter().filter(|(_, e)| q.matches(e)).collect();
        picked.sort_by(|a, b| b.1.at.cmp(&a.1.at).then_with(|| b.0.cmp(&a.0)));
        let entries = self
            .db
            .read_async(move |c| {
                let mut out = Vec::with_capacity(picked.len());
                for (id, e) in picked {
                    let thumb = detect::album_basics(c, &e.album_id)?.map(|b| b.3);
                    out.push(to_item(id, e, thumb));
                }
                Ok(out)
            })
            .await?;
        Ok(DiaryList { entries, total, first_at, last_at })
    }

    /// Rate, note or move an entry.
    pub async fn edit(&self, id: &str, edit: DiaryEdit) -> AppResult<DiaryItem> {
        let rating = check_rating(edit.rating)?;
        let note = clean_note(&edit.note)?;
        if let Some(at) = edit.at
            && (at <= 0 || at > (self.clock)() + 86_400_000)
        {
            return Err(AppError::Invalid("That date is in the future.".into()));
        }
        let key = id.to_owned();
        let Some(mut e) = self.store.run(move |s| s.get(&DIARY, &key)).await? else {
            return Err(AppError::NotFound("That diary entry is gone.".into()));
        };
        e.rating = rating;
        e.note = note;
        if let Some(at) = edit.at {
            e.at = at;
        }
        let (key, value) = (id.to_owned(), e.clone());
        self.store.run(move |s| s.put(&DIARY, &key, &value)).await?;
        self.changed();
        self.item(id.to_owned(), e).await
    }

    /// Delete an entry. Returns whether there was one.
    pub async fn delete(&self, id: &str) -> AppResult<bool> {
        let key = id.to_owned();
        let gone = self.store.run(move |s| s.remove(&DIARY, &key)).await?;
        if gone {
            self.changed();
        }
        Ok(gone)
    }

    /// Log an album by hand, now.
    pub async fn log(&self, album_id: &str) -> AppResult<DiaryItem> {
        if !self.on() {
            return Err(AppError::Invalid("Turn on Keep a listening diary to log albums.".into()));
        }
        let id = album_id.to_owned();
        let (basics, songs) = self
            .db
            .read_async(move |c| Ok((detect::album_basics(c, &id)?, detect::album_songs(c, &id)?.len())))
            .await?;
        let Some((title, artist, year, thumb)) = basics else {
            return Err(AppError::NotFound(format!("album {album_id}")));
        };
        let at = (self.clock)();
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let key = format!("log:{album_id}:{at}:{n}");
        let entry = DiaryEntry {
            album_id: album_id.to_owned(),
            title,
            artist,
            year,
            at,
            songs_total: songs as u32,
            auto: false,
            ..Default::default()
        };
        let (k, v) = (key.clone(), entry.clone());
        self.store.run(move |s| s.put(&DIARY, &k, &v)).await?;
        self.changed();
        Ok(to_item(key, entry, Some(thumb)))
    }

    /// The first time any of the album's songs was played (imported history
    /// included), or the earliest diary entry for it when that's earlier.
    pub async fn first_heard(&self, album_id: &str) -> AppResult<Option<i64>> {
        let id = album_id.to_owned();
        let played = self.db.read_async(move |c| detect::first_play(c, &id)).await?;
        let id = album_id.to_owned();
        let logged = self
            .store
            .run(move |s| s.list(&DIARY))
            .await?
            .into_iter()
            .filter(|(_, e)| e.album_id == id && e.at > 0)
            .map(|(_, e)| e.at)
            .min();
        Ok(match (played, logged) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        })
    }

    /// The listed entries as text or CSV, with this computer's local dates.
    pub async fn export_body(&self, q: &DiaryQuery, format: ExportFormat) -> AppResult<String> {
        let list = self.list(q).await?;
        let mut at: Vec<i64> = list.entries.iter().map(|e| e.entry.at).collect();
        at.push((self.clock)());
        let mut times = self.db.read_async(move |c| export::local_times(c, &at)).await?;
        let today = times.pop().unwrap_or(export::LocalTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0 });
        let items: Vec<_> = list.entries.into_iter().zip(times).collect();
        Ok(match format {
            ExportFormat::Text => export::to_text(&items, today),
            ExportFormat::Csv => export::to_csv(&items),
        })
    }
}

fn to_item(id: String, entry: DiaryEntry, thumb: Option<Option<String>>) -> DiaryItem {
    let listened_on = entry.output.as_deref().and_then(listened_on);
    DiaryItem { id, entry, in_library: thumb.is_some(), thumb: thumb.flatten(), listened_on }
}

/// The tracker tells the diary when a play from an album ends.
impl crate::tracking::PlayEnded for Diary {
    fn album_play_ended(&self, album_id: &str) {
        let me = self.clone();
        let id = album_id.to_owned();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = me.album_played(&id).await {
                tracing::warn!("diary: {e}");
            }
        });
    }
}

// ------------------------------------------------------------------ commands

/// Diary entries, newest first (`diary_entries`).
#[tauri::command]
pub async fn diary_entries(diary: tauri::State<'_, Diary>, query: Option<DiaryQuery>) -> AppResult<DiaryList> {
    diary.list(&query.unwrap_or_default()).await
}

/// Rate, note or move an entry (`diary_entry_edit`).
#[tauri::command]
pub async fn diary_entry_edit(diary: tauri::State<'_, Diary>, id: String, edit: DiaryEdit) -> AppResult<DiaryItem> {
    diary.edit(&id, edit).await
}

/// Delete an entry (`diary_entry_delete`).
#[tauri::command]
pub async fn diary_entry_delete(diary: tauri::State<'_, Diary>, id: String) -> AppResult<bool> {
    diary.delete(&id).await
}

/// Log an album by hand (`diary_log_album`).
#[tauri::command]
pub async fn diary_log_album(diary: tauri::State<'_, Diary>, id: String) -> AppResult<DiaryItem> {
    diary.log(&id).await
}

/// When an album was first heard (`album_first_heard`).
#[tauri::command]
pub async fn album_first_heard(diary: tauri::State<'_, Diary>, id: String) -> AppResult<Option<i64>> {
    diary.first_heard(&id).await
}

/// Save the diary as text or CSV wherever the user picks
/// (`diary_export`). Returns the file's name, or None when cancelled.
#[tauri::command]
pub async fn diary_export(
    app: tauri::AppHandle,
    diary: tauri::State<'_, Diary>,
    format: ExportFormat,
    query: Option<DiaryQuery>,
) -> AppResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let body = diary.export_body(&query.unwrap_or_default(), format).await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (label, ext) = match format {
        ExportFormat::Text => ("Text", format.extension()),
        ExportFormat::Csv => ("CSV", format.extension()),
    };
    app.dialog()
        .file()
        .set_title("Export listening diary")
        .set_file_name(format!("Listening diary.{ext}"))
        .add_filter(label, &[ext])
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| AppError::Io(format!("export path: {e}")))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    tokio::task::spawn_blocking(move || std::fs::write(&path, body))
        .await
        .map_err(|e| AppError::Internal(format!("export: {e}")))??;
    Ok(Some(name))
}
