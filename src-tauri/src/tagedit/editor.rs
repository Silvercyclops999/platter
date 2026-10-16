//! The tag editor itself: opening an album's (or some songs') files,
//! saving changes, undoing them, and telling Plex and the tag cache.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde::{Deserialize, Serialize};

use super::Values;
use super::doc::{Ctx, Format};
use super::fields::{self, Field, Scheme};
use super::paths::Mapping;
use super::undo::{UndoEntry, UndoFile, UndoLog};
use super::write::{self, Edits, Hooks, Refusal, Refused};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};

/// What the editor follows from Settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// The music folder, once it's set and checked.
    pub mapping: Option<Mapping>,
    /// Ask Plex to rescan each saved folder.
    pub rescan: bool,
}

/// Plex, as far as the editor needs it.
#[async_trait]
pub trait PlexSide: Send + Sync {
    /// `GET /library/sections/{section}/refresh?path={folder}` on the
    /// server `track_id` is on.
    async fn refresh_folder(&self, track_id: &str, section: &str, folder: &str) -> AppResult<()>;
    /// The size Plex has for each track's file.
    async fn part_sizes(&self, track_ids: &[String]) -> AppResult<Vec<(String, Option<u64>)>>;
}

/// One track as the library cache has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRow {
    pub id: String,
    pub title: String,
    pub album_id: String,
    pub album_title: String,
    pub artist: String,
    pub track_artist: Option<String>,
    pub disc: i64,
    pub track: i64,
    pub duration_ms: i64,
    pub part_key: String,
    pub file: Option<String>,
    pub src: String,
    pub codec: Option<String>,
    pub bitrate: Option<i64>,
}

const ROW: &str = "id, title, album_id, album_title, artist_title, track_artist, disc_number, track_number, duration_ms,
                   part_key, file, src, codec, bitrate";

fn row(r: &rusqlite::Row) -> rusqlite::Result<TrackRow> {
    Ok(TrackRow {
        id: r.get(0)?,
        title: r.get(1)?,
        album_id: r.get(2)?,
        album_title: r.get(3)?,
        artist: r.get(4)?,
        track_artist: r.get(5)?,
        disc: r.get(6)?,
        track: r.get(7)?,
        duration_ms: r.get(8)?,
        part_key: r.get(9)?,
        file: r.get(10)?,
        src: r.get(11)?,
        codec: r.get(12)?,
        bitrate: r.get(13)?,
    })
}

pub fn album_rows(c: &Connection, album_id: &str) -> AppResult<Vec<TrackRow>> {
    let mut st = c.prepare_cached(&format!("SELECT {ROW} FROM tracks WHERE album_id = ?1 ORDER BY disc_number, track_number, title"))?;
    let rows = st.query_map([album_id], row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn track_rows(c: &Connection, ids: &[String]) -> AppResult<Vec<TrackRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let marks = vec!["?"; ids.len()].join(",");
    let mut st = c.prepare(&format!("SELECT {ROW} FROM tracks WHERE id IN ({marks}) ORDER BY album_id, disc_number, track_number"))?;
    let rows = st.query_map(params_from_iter(ids), row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// What the tag cache and Plex know of a file, for view-only details
/// (no music folder) and the tag check.
pub fn cached_values(c: &Connection, t: &TrackRow) -> AppResult<Values> {
    let mut v = Values::new();
    let mut put = |f: Field, s: Option<String>| {
        if let Some(s) = s.filter(|s| !s.trim().is_empty()) {
            v.insert(f, s);
        }
    };
    put(Field::Title, Some(t.title.clone()));
    put(Field::Artist, Some(t.track_artist.clone().unwrap_or_else(|| t.artist.clone())));
    put(Field::TrackNumber, (t.track > 0).then(|| t.track.to_string()));
    put(Field::DiscNumber, (t.disc > 0).then(|| t.disc.to_string()));
    type Cached = (Option<i64>, [Option<String>; 16], Option<i64>);
    let r: Option<Cached> = c
        .query_row(
            "SELECT explicit, album, album_artist, edition, grp, era, quality, version, tag_status, mb_status, mb_type,
                    custom_status, version_note, date, source, flags, isrc, year
             FROM file_tags WHERE part_key = ?1 AND status = 'ok'",
            [&t.part_key],
            |r| {
                let mut a: [Option<String>; 16] = Default::default();
                for (i, s) in a.iter_mut().enumerate() {
                    *s = r.get(i + 1)?;
                }
                Ok((r.get(0)?, a, r.get(17)?))
            },
        )
        .optional()?;
    if let Some((explicit, a, year)) = r {
        let [album, album_artist, edition, grp, era, quality, version, status, mb_status, mb_type, custom, note, date, source, flags, isrc] =
            a;
        put(Field::Explicit, explicit.map(|e| if e != 0 { "explicit".into() } else { "clean".into() }));
        put(Field::Album, album);
        put(Field::AlbumArtist, album_artist);
        put(Field::Edition, edition);
        put(Field::Group, grp);
        put(Field::Era, era);
        put(Field::Quality, quality);
        put(Field::ArchiveVersion, version);
        put(Field::Status, status);
        put(Field::PicardStatus, mb_status);
        put(Field::ReleaseType, mb_type);
        put(Field::CustomStatus, custom);
        put(Field::VersionNote, note);
        put(Field::ArchiveDate, date);
        put(Field::Source, source);
        put(Field::Flags, flags);
        put(Field::Isrc, isrc);
        put(Field::Year, year.map(|y| y.to_string()));
    } else {
        put(Field::Album, Some(t.album_title.clone()));
        put(Field::AlbumArtist, Some(t.artist.clone()));
    }
    Ok(v)
}

// ---------------------------------------------------------------- IPC shapes

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFile {
    pub track_id: String,
    pub album_id: String,
    pub server_path: Option<String>,
    pub local_path: Option<String>,
    pub file_name: String,
    pub format: Option<Format>,
    pub id3_version: Option<&'static str>,
    pub values: Values,
    /// Every tag name the file carries (for the tag check).
    pub keys: Vec<String>,
    /// Present when the file was read for editing.
    pub stamp: Option<String>,
    /// Why it couldn't be read.
    pub error: Option<String>,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorData {
    pub title: String,
    pub files: Vec<EditFile>,
    /// Where the release status is written for these files.
    pub scheme: Scheme,
    pub status_field: Field,
    pub status_choices: Vec<String>,
    pub custom_key: Option<String>,
    /// Save is offered.
    pub writable: bool,
    /// Why not, or what's partly wrong.
    pub reason: Option<String>,
    /// The music folder isn't set: Save becomes "Set up music folder".
    pub needs_music_folder: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFile {
    pub track_id: String,
    pub stamp: String,
    pub edits: Edits,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRequest {
    pub label: String,
    pub files: Vec<SaveFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileProblem {
    pub track_id: String,
    pub file_name: String,
    pub why: Refusal,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub saved: usize,
    pub unchanged: usize,
    pub problems: Vec<FileProblem>,
    /// "Saved 12 files", or why nothing was saved.
    pub message: String,
    /// Undo this with [`TagEditor::undo`].
    pub undo_id: Option<String>,
    /// Folders Plex was asked to rescan (server paths).
    pub rescanned: Vec<String>,
    pub rescan_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoInfo {
    pub id: String,
    pub at: i64,
    pub label: String,
    pub files: usize,
}

// ---------------------------------------------------------------- the editor

pub struct TagEditor {
    db: DbHandle,
    plex: Arc<dyn PlexSide>,
    hooks: Arc<dyn Hooks>,
    log: Arc<Mutex<UndoLog>>,
    options: RwLock<Options>,
}

fn file_name(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

/// The folder part of a server path.
pub fn server_folder(p: &str) -> String {
    match p.rfind(['/', '\\']) {
        Some(0) => p[..1].to_string(),
        Some(i) => p[..i].to_string(),
        None => String::new(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// "1 file" / "3 files".
pub fn files(n: usize) -> String {
    if n == 1 { "1 file".into() } else { format!("{n} files") }
}

pub const NEEDS_FOLDER: &str = "Set up the music folder in Settings, Library to edit tags.";

impl TagEditor {
    pub fn new(db: DbHandle, plex: Arc<dyn PlexSide>, hooks: Arc<dyn Hooks>, log_path: Option<std::path::PathBuf>) -> Self {
        Self { db, plex, hooks, log: Arc::new(Mutex::new(UndoLog::open(log_path))), options: RwLock::new(Options::default()) }
    }

    pub fn set_options(&self, o: Options) {
        *self.options.write() = o;
    }

    pub fn options(&self) -> Options {
        self.options.read().clone()
    }

    pub fn db(&self) -> &DbHandle {
        &self.db
    }

    pub fn plex(&self) -> &Arc<dyn PlexSide> {
        &self.plex
    }

    fn ctx(&self) -> Ctx {
        Ctx { custom_key: crate::tags::provenance::custom_key(), slash_numbers: false }
    }

    /// Opens files for the editor: an album's, or some songs'.
    pub fn open(&self, album_id: Option<&str>, track_ids: &[String]) -> AppResult<EditorData> {
        let rows = self.db.read(|c| match album_id {
            Some(a) => album_rows(c, a),
            None => track_rows(c, track_ids),
        })?;
        if rows.is_empty() {
            return Err(AppError::NotFound("Those songs aren't in the library any more.".into()));
        }
        self.open_rows(&rows)
    }

    pub fn open_rows(&self, rows: &[TrackRow]) -> AppResult<EditorData> {
        let opts = self.options();
        let ctx = self.ctx();
        let title = if rows.iter().all(|r| r.album_id == rows[0].album_id) {
            if rows.len() == 1 { rows[0].title.clone() } else { rows[0].album_title.clone() }
        } else {
            files(rows.len())
        };
        let mut out = Vec::new();
        for t in rows {
            let server = t.file.clone();
            let name = server.as_deref().map(file_name).unwrap_or_else(|| t.title.clone());
            let mut f = EditFile {
                track_id: t.id.clone(),
                album_id: t.album_id.clone(),
                server_path: server.clone(),
                local_path: None,
                file_name: name,
                format: None,
                id3_version: None,
                values: Values::new(),
                keys: Vec::new(),
                stamp: None,
                error: None,
                duration_ms: t.duration_ms,
            };
            let local = opts.mapping.as_ref().zip(server.as_deref()).and_then(|(m, s)| m.to_local(s));
            match (&opts.mapping, local) {
                (None, _) => f.values = self.db.read(|c| cached_values(c, t))?,
                (Some(_), None) => {
                    f.values = self.db.read(|c| cached_values(c, t))?;
                    f.error = Some(match &server {
                        Some(_) => "This file is outside the music folder.".into(),
                        None => "Plex hasn't said where this file is yet. Try again after the library syncs.".into(),
                    });
                }
                (Some(_), Some(local)) => match write::read_file(Path::new(&local), &ctx) {
                    Ok((doc, values, stamp)) => {
                        f.format = Some(doc.format);
                        f.id3_version = doc.id3_version();
                        f.keys = doc.keys();
                        f.values = values;
                        f.stamp = Some(stamp);
                        f.local_path = Some(local);
                    }
                    Err(r) => {
                        f.values = self.db.read(|c| cached_values(c, t))?;
                        f.error = Some(r.message);
                        f.local_path = Some(local);
                    }
                },
            }
            out.push(f);
        }
        let scheme = fields::detect_scheme(out.iter().map(|f| &f.values), ctx.custom_key.is_some());
        let readable = out.iter().filter(|f| f.stamp.is_some()).count();
        let needs = opts.mapping.is_none();
        let reason = if needs {
            Some(NEEDS_FOLDER.to_string())
        } else if readable == 0 {
            out.iter().find_map(|f| f.error.clone()).or(Some("These files couldn't be read.".into()))
        } else if readable < out.len() {
            Some(format!("{} couldn't be read and won't be changed.", files(out.len() - readable)))
        } else {
            None
        };
        Ok(EditorData {
            title,
            status_field: scheme.field(),
            status_choices: scheme.choices().iter().map(|s| s.to_string()).collect(),
            scheme,
            custom_key: ctx.custom_key,
            writable: readable > 0,
            reason,
            needs_music_folder: needs,
            files: out,
        })
    }

    /// Writes the changes. Every file is checked first; when any changed
    /// since the editor read it, or its folder is busy, nothing is written.
    /// Then Plex rescans each folder and the tag cache takes the new values.
    pub async fn save(&self, req: SaveRequest) -> AppResult<SaveResult> {
        let this = self.clone_parts();
        let (mut result, folders) = tokio::task::spawn_blocking(move || this.save_blocking(&req))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))??;
        self.rescan(&mut result, folders).await;
        Ok(result)
    }

    async fn rescan(&self, result: &mut SaveResult, folders: Folders) {
        if !self.options().rescan {
            return;
        }
        for ((src, folder), track) in folders {
            let section = self.db.read(|c| crate::db::store::sync_state_in(c, &src)).ok().flatten().map(|s| s.section_id);
            let Some(section) = section.filter(|s| !s.is_empty()) else { continue };
            match self.plex.refresh_folder(&track, &section, &folder).await {
                Ok(()) => result.rescanned.push(folder),
                Err(e) => {
                    tracing::warn!("folder rescan: {e}");
                    result.rescan_error =
                        Some("Plex couldn't be asked to rescan. It will pick the changes up on its next scan.".into());
                }
            }
        }
    }

    fn clone_parts(&self) -> Parts {
        Parts { db: self.db.clone(), hooks: self.hooks.clone(), options: self.options(), ctx: self.ctx(), log: self.log.clone() }
    }

    pub fn history(&self) -> Vec<UndoInfo> {
        self.log
            .lock()
            .entries
            .iter()
            .map(|e| UndoInfo { id: e.id.clone(), at: e.at, label: e.label.clone(), files: e.files.len() })
            .collect()
    }

    /// Puts back what an edit replaced, on the files still as it left them.
    pub async fn undo(&self, id: &str) -> AppResult<SaveResult> {
        let entry = self.log.lock().take(id).ok_or_else(|| AppError::NotFound("That edit can't be undone any more.".into()))?;
        let this = self.clone_parts();
        let (mut result, folders) = tokio::task::spawn_blocking(move || this.undo_blocking(entry))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))??;
        self.rescan(&mut result, folders).await;
        Ok(result)
    }
}

/// What the blocking half of a save needs.
struct Parts {
    db: DbHandle,
    hooks: Arc<dyn Hooks>,
    options: Options,
    ctx: Ctx,
    log: Arc<Mutex<UndoLog>>,
}

/// Folders written to: (library, server folder) and a track in it.
type Folders = BTreeMap<(String, String), String>;

impl Parts {
    fn save_blocking(&self, req: &SaveRequest) -> AppResult<(SaveResult, Folders)> {
        let Some(mapping) = self.options.mapping.clone() else {
            return Err(AppError::Invalid(NEEDS_FOLDER.into()));
        };
        // Every value checked before anything is written.
        let mut cleaned: Vec<(SaveFile, Edits)> = Vec::new();
        for f in &req.files {
            let mut e = Edits::new();
            for (field, v) in &f.edits {
                e.insert(*field, fields::clean_value(*field, v)?);
            }
            if !e.is_empty() {
                cleaned.push((f.clone(), e));
            }
        }
        if cleaned.is_empty() {
            return Ok((SaveResult { message: "Nothing to save.".into(), ..Default::default() }, Folders::new()));
        }
        let ids: Vec<String> = cleaned.iter().map(|(f, _)| f.track_id.clone()).collect();
        let rows: HashMap<String, TrackRow> = self.db.read(|c| track_rows(c, &ids))?.into_iter().map(|r| (r.id.clone(), r)).collect();

        // Check every file first.
        let mut problems = Vec::new();
        let mut plan: Vec<(TrackRow, String, String, Edits)> = Vec::new();
        let mut slash: (usize, usize) = (0, 0);
        let mut busy: HashMap<String, bool> = HashMap::new();
        for (f, e) in cleaned {
            let Some(t) = rows.get(&f.track_id).cloned() else {
                problems.push(FileProblem {
                    track_id: f.track_id.clone(),
                    file_name: f.track_id.clone(),
                    why: Refusal::Gone,
                    message: "This song isn't in the library any more.".into(),
                });
                continue;
            };
            let server = t.file.clone().unwrap_or_default();
            let name = file_name(&server);
            let Some(local) = mapping.to_local(&server) else {
                problems.push(FileProblem {
                    track_id: t.id.clone(),
                    file_name: name,
                    why: Refusal::Gone,
                    message: "This file is outside the music folder.".into(),
                });
                continue;
            };
            let refuse = |r: Refused| FileProblem { track_id: t.id.clone(), file_name: name.clone(), why: r.why, message: r.message };
            let dir = server_folder(&local);
            let is_busy = *busy
                .entry(dir.clone())
                .or_insert_with(|| write::folder_busy(Path::new(&dir), std::time::SystemTime::now(), &write::RECENT));
            if is_busy {
                problems.push(refuse(Refused { why: Refusal::Busy, message: write::BUSY_MESSAGE.into() }));
                continue;
            }
            match write::read_file(Path::new(&local), &self.ctx) {
                Ok((doc, _, stamp)) if stamp == f.stamp => {
                    match doc.slash_numbers() {
                        Some(true) => slash.0 += 1,
                        Some(false) => slash.1 += 1,
                        None => {}
                    }
                    plan.push((t.clone(), server, local, e));
                }
                Ok(_) => problems.push(refuse(Refused {
                    why: Refusal::Changed,
                    message: format!("{name} changed since the editor opened. Open it again to see the new tags."),
                })),
                Err(r) => problems.push(refuse(r)),
            }
        }
        if !problems.is_empty() {
            let first = problems[0].message.clone();
            let message = if problems.iter().all(|p| p.why == Refusal::Busy) {
                write::BUSY_MESSAGE.to_string()
            } else if problems.len() == 1 {
                format!("Nothing was saved. {first}")
            } else {
                format!("Nothing was saved: {} can't be written. {first}", files(problems.len()))
            };
            return Ok((SaveResult { problems, message, ..Default::default() }, Folders::new()));
        }

        // The album's way of keeping totals, for files that have none yet.
        let ctx = Ctx { slash_numbers: slash.0 > slash.1, ..self.ctx.clone() };
        let mut result = SaveResult::default();
        let mut undo = Vec::new();
        let mut folders = Folders::new();
        let mut cache: Vec<(String, Values)> = Vec::new();
        let stamps: HashMap<&str, &str> = req.files.iter().map(|f| (f.track_id.as_str(), f.stamp.as_str())).collect();
        for (t, server, local, e) in plan {
            let name = file_name(&server);
            match write::write_file(Path::new(&local), stamps[t.id.as_str()], &e, &ctx, self.hooks.as_ref()) {
                Ok(Some(w)) => {
                    result.saved += 1;
                    folders.insert((t.src.clone(), server_folder(&server)), t.id.clone());
                    cache.push((t.part_key.clone(), w.after.clone()));
                    undo.push(UndoFile { track_id: t.id.clone(), server_path: server, local_path: local, before: w.before, stamp: w.stamp });
                }
                Ok(None) => result.unchanged += 1,
                Err(r) => result.problems.push(FileProblem { track_id: t.id.clone(), file_name: name, why: r.why, message: r.message }),
            }
        }
        if let Err(e) = update_cache(&self.db, &cache) {
            tracing::warn!("tag cache after an edit: {e}");
        }
        if !undo.is_empty() {
            let id = uuid::Uuid::new_v4().to_string();
            self.log.lock().push(UndoEntry {
                id: id.clone(),
                at: now_ms(),
                label: if req.label.trim().is_empty() { "Edited tags".into() } else { req.label.trim().to_string() },
                custom_key: self.ctx.custom_key.clone(),
                files: undo,
            });
            result.undo_id = Some(id);
        }
        tracing::info!(saved = result.saved, failed = result.problems.len(), "tag edit: {}", req.label);
        result.message = match (result.saved, result.problems.len()) {
            (0, 0) => "Nothing changed.".into(),
            (n, 0) => format!("Saved {}", files(n)),
            (n, p) => format!("Saved {}. {} wasn't saved: {}", files(n), files(p), result.problems[0].message),
        };
        Ok((result, folders))
    }

    fn undo_blocking(&self, entry: UndoEntry) -> AppResult<(SaveResult, Folders)> {
        let ctx = Ctx { custom_key: entry.custom_key.clone(), slash_numbers: false };
        let mut result = SaveResult::default();
        let mut folders = Folders::new();
        let mut cache = Vec::new();
        let part_keys: HashMap<String, (String, String)> = self
            .db
            .read(|c| track_rows(c, &entry.files.iter().map(|f| f.track_id.clone()).collect::<Vec<_>>()))?
            .into_iter()
            .map(|r| (r.id, (r.part_key, r.src)))
            .collect();
        for f in &entry.files {
            // Where the file is now, if the music folder moved.
            let local = self.options.mapping.as_ref().and_then(|m| m.to_local(&f.server_path)).unwrap_or_else(|| f.local_path.clone());
            let edits: Edits = f.before.iter().map(|(k, v)| (*k, v.clone().unwrap_or_default())).collect();
            match write::write_file(Path::new(&local), &f.stamp, &edits, &ctx, self.hooks.as_ref()) {
                Ok(Some(w)) => {
                    result.saved += 1;
                    let (pk, src) = part_keys.get(&f.track_id).cloned().unwrap_or_default();
                    folders.insert((src, server_folder(&f.server_path)), f.track_id.clone());
                    if !pk.is_empty() {
                        cache.push((pk, w.after));
                    }
                }
                Ok(None) => result.unchanged += 1,
                Err(mut r) => {
                    if r.why == Refusal::Changed {
                        r.message = format!("{} changed after the edit, so it was kept as it is.", file_name(&local));
                    }
                    result.problems.push(FileProblem { track_id: f.track_id.clone(), file_name: file_name(&local), why: r.why, message: r.message });
                }
            }
        }
        if let Err(e) = update_cache(&self.db, &cache) {
            tracing::warn!("tag cache after an undo: {e}");
        }
        tracing::info!(restored = result.saved, kept = result.problems.len(), "tag edit undone: {}", entry.label);
        result.message = match (result.saved, result.problems.len()) {
            (n, 0) => format!("Undid the edit on {}", files(n)),
            (0, p) => format!("Nothing was undone: {} changed since.", files(p)),
            (n, p) => format!("Undid the edit on {}. {} changed since and kept as they are.", files(n), files(p)),
        };
        Ok((result, folders))
    }
}

/// The tag cache takes what was written, so badges and the archive follow
/// at once (Plex's rescan gives the file a new part key, read again later).
pub fn update_cache(db: &DbHandle, written: &[(String, Values)]) -> AppResult<()> {
    if written.is_empty() {
        return Ok(());
    }
    let custom = crate::tags::provenance::custom_key();
    db.write(|c| {
        crate::archive::in_tx(c, |c| {
            for (part_key, values) in written {
                let old = crate::tags::store::get(c, part_key)?;
                let mut t = crate::tags::FileTags::for_read();
                for (f, v) in values {
                    let key = match f {
                        Field::CustomStatus => custom.clone(),
                        _ => f.vorbis_key().map(str::to_string),
                    };
                    if let Some(k) = key {
                        t.set(&k, v);
                    }
                }
                t.recording_mbid = old.and_then(|(_, o)| o.recording_mbid);
                crate::tags::store::save(c, part_key, crate::tags::store::Status::Ok, &t, now_ms() / 1000)?;
            }
            crate::tags::store::refresh_albums(c)?;
            crate::identity::index::rebuild(c)?;
            crate::archive::refresh_all(c)
        })
    })
}

/// Values the files share, for the album fields: Some(value) when every
/// file has the same, None when they differ ("(several)").
pub fn shared(files: &[&Values], f: Field) -> Option<String> {
    let mut it = files.iter().map(|v| v.get(&f).cloned().unwrap_or_default());
    let first = it.next()?;
    it.all(|v| v == first).then_some(first)
}

/// Album-level edits for each file, leaving files that already hold the
/// value alone (so only real changes are written).
pub fn spread(files: &[EditFile], album: &BTreeMap<Field, String>) -> Vec<SaveFile> {
    files
        .iter()
        .filter_map(|f| {
            let stamp = f.stamp.clone()?;
            let edits: Edits = album
                .iter()
                .filter(|(k, v)| f.values.get(k).map(String::as_str).unwrap_or("") != v.as_str())
                .map(|(k, v)| (*k, v.clone()))
                .collect();
            (!edits.is_empty()).then(|| SaveFile { track_id: f.track_id.clone(), stamp, edits })
        })
        .collect()
}

// ---------------------------------------------------------------- the tag check

/// What the tag check looks at.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "id")]
pub enum CheckScope {
    Album(String),
    Artist(String),
    Library,
}

/// Findings marked "This is fine", kept in the library cache's key-value store.
const IGNORED_KEY: &str = "tagcheck.ignored";

pub fn ignored(c: &Connection) -> AppResult<std::collections::BTreeSet<String>> {
    Ok(crate::db::store::kv_get(c, IGNORED_KEY)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
}

pub fn set_ignored(c: &Connection, id: &str, fine: bool) -> AppResult<()> {
    let mut all = ignored(c)?;
    if fine {
        all.insert(id.to_string());
    } else {
        all.remove(id);
    }
    crate::db::store::kv_set(c, IGNORED_KEY, &serde_json::to_string(&all).unwrap_or_default())
}

fn library_context(c: &Connection) -> AppResult<super::check::Library> {
    let mut lib = super::check::Library::default();
    let mut st = c.prepare("SELECT ft.grp, t.album_id FROM tracks t JOIN file_tags ft ON ft.part_key = t.part_key WHERE ft.grp IS NOT NULL AND ft.grp != ''")?;
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (g, a) = r?;
        lib.groups.entry(g).or_default().insert(a);
    }
    let mut st = c.prepare("SELECT file, album_id FROM tracks WHERE file IS NOT NULL AND file != ''")?;
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (f, a) = r?;
        lib.folders.entry(server_folder(&f)).or_default().insert(a);
    }
    let mut st = c.prepare("SELECT id, title FROM albums")?;
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, t) = r?;
        lib.titles.insert(id, t);
    }
    Ok(lib)
}

impl TagEditor {
    /// The albums a scope covers.
    pub fn scope_albums(&self, scope: &CheckScope) -> AppResult<Vec<String>> {
        self.db.read(|c| {
            let (sql, arg) = match scope {
                CheckScope::Album(id) => return Ok(vec![id.clone()]),
                CheckScope::Artist(id) => ("SELECT id FROM albums WHERE artist_id = ?1 ORDER BY year, sort_title", Some(id.clone())),
                CheckScope::Library => ("SELECT id FROM albums ORDER BY artist_title COLLATE NOCASE, year, sort_title", None),
            };
            let mut st = c.prepare(sql)?;
            let ids = match arg {
                Some(a) => st.query_map([a], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?,
                None => st.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?,
            };
            Ok(ids)
        })
    }

    /// One album as the check sees it: from the files when the music folder
    /// is set, else from what Plex and the tag cache have.
    pub fn check_album_data(&self, album_id: &str) -> AppResult<super::check::CheckAlbum> {
        let rows = self.db.read(|c| album_rows(c, album_id))?;
        let title = rows.first().map(|r| r.album_title.clone()).unwrap_or_default();
        if rows.is_empty() {
            return Ok(super::check::CheckAlbum { album_id: album_id.to_string(), title, files: Vec::new() });
        }
        let data = self.open_rows(&rows)?;
        let files = rows
            .iter()
            .zip(data.files)
            .map(|(r, f)| super::check::CheckFile {
                track_id: r.id.clone(),
                file_name: f.file_name,
                folder: r.file.as_deref().map(server_folder).unwrap_or_default(),
                values: f.values,
                keys: f.keys,
                duration_ms: r.duration_ms,
                codec: r.codec.clone(),
                bitrate: r.bitrate,
                plex_disc: r.disc,
            })
            .collect();
        Ok(super::check::CheckAlbum { album_id: album_id.to_string(), title, files })
    }

    /// The tag check over a scope (on the caller's thread: files are read).
    pub fn check(&self, scope: &CheckScope) -> AppResult<super::check::Report> {
        let ids = self.scope_albums(scope)?;
        let mut albums = Vec::with_capacity(ids.len());
        for id in &ids {
            albums.push(self.check_album_data(id)?);
        }
        let opts = self.options();
        let (mut lib, ign) = self.db.read(|c| Ok((library_context(c)?, ignored(c)?)))?;
        // The files just read count for groups too.
        for a in &albums {
            for f in &a.files {
                if let Some(g) = f.values.get(&Field::Group) {
                    lib.groups.entry(g.clone()).or_default().insert(a.album_id.clone());
                }
            }
        }
        lib.custom_status = crate::tags::provenance::custom_key().is_some();
        lib.can_fix = opts.mapping.is_some();
        Ok(super::check::report(&albums, &lib, &ign, opts.mapping.is_some()))
    }
}
