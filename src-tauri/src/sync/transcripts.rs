//! Transcriptions travel to the user's other devices (the `transcripts`
//! namespace), so a song is transcribed once. Each result keeps its model
//! name and is shown as transcribed elsewhere too, under the same label and
//! trust rules; "no words found" results travel as well.
//!
//! Keyed `<track id>|<model>`, so results from two models never overwrite
//! each other; each carries the file version (part key) it was made from.
//! A result for another version of the file is ignored, so a replaced file
//! is transcribed again. Which result a song shows: lyrics the user wrote or
//! edited always win; then the bigger model (Turbo the biggest); then the
//! one heard better (the voice alone over the whole mix); then the newest.
//! How it was heard travels too, so a computer that can hear the voice
//! alone redoes a song another device heard from the mix, and not one
//! already heard from the voice.
//!
//! Whether the user shared a song's lyrics to LRCLIB travels in
//! `lyricsShared`, so no device offers it again.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::store::{LYRICS_SHARED, SyncStore, TRANSCRIPTS};
use crate::db::DbHandle;
use crate::error::AppResult;
use crate::lyrics::transcribe::engine::TIMING_VERSION;
use crate::lyrics::transcribe::lines;
use crate::lyrics::transcribe::store as tstore;
use crate::lyrics::user;

/// The models, smallest first.
pub const MODELS: [&str; 3] = ["small", "medium", "turbo"];
/// Every model a transcription can come from.
pub const KNOWN_MODELS: [&str; 3] = MODELS;

/// A transcription as it travels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    /// Enhanced LRC; empty when no words were found.
    #[serde(default)]
    pub text: String,
    pub model: String,
    /// The file version it was made from (the part key).
    pub part: String,
    /// When it was made, unix seconds.
    #[serde(default)]
    pub at: i64,
    /// How its words were timed (engine::TIMING_VERSION); results from
    /// before the field are the first timing.
    #[serde(default = "first_timing")]
    pub timing: i64,
    /// How it was heard (lyrics::transcribe::store's HEARD_*); 0 from a
    /// build before the field, counted as the old way.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub heard: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

impl Transcript {
    /// How it was heard, a result from an older build as the old way.
    pub fn heard(&self) -> i64 {
        self.heard.max(tstore::HEARD_OLD)
    }
}

fn first_timing() -> i64 {
    1
}

/// A song whose lyrics the user shared to LRCLIB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shared {
    /// Unix seconds.
    pub at: i64,
}

/// Bigger models rank higher; unknown names lowest.
pub fn rank(model: &str) -> usize {
    MODELS.iter().position(|m| *m == model).map_or(0, |i| i + 1)
}

pub fn key(track_id: &str, model: &str) -> String {
    format!("{track_id}|{model}")
}

fn split_key(k: &str) -> Option<(&str, &str)> {
    k.rsplit_once('|')
}

/// The result a song shows among these: the bigger model, then the one
/// heard better, then the newest.
pub fn best<'a>(results: impl IntoIterator<Item = &'a Transcript>) -> Option<&'a Transcript> {
    results.into_iter().max_by(|a, b| (rank(&a.model), a.heard(), a.at).cmp(&(rank(&b.model), b.heard(), b.at)))
}

/// A result from another device Platter would show: a model it has, and
/// lyrics no longer than the editor takes.
pub fn usable(t: &Transcript) -> bool {
    KNOWN_MODELS.contains(&t.model.as_str()) && t.text.len() <= user::MAX_BYTES
}

/// After a result was saved here.
pub fn saved(store: &SyncStore, db: &DbHandle, track_id: &str, body: &str, model: &str, at: i64) -> AppResult<()> {
    let Some(part) = db.read(|c| tstore::part_key(c, track_id))? else { return Ok(()) };
    let heard = db.read(|c| tstore::heard(c, track_id))?.unwrap_or(tstore::HEARD_OLD);
    let t = Transcript { text: body.trim().to_owned(), model: model.to_owned(), part, at, timing: TIMING_VERSION, heard };
    store.put(&TRANSCRIPTS, &key(track_id, model), &t)
}

/// After a transcription was forgotten here.
pub fn forgot(store: &SyncStore, track_id: &str) -> AppResult<()> {
    for m in MODELS {
        store.remove(&TRANSCRIPTS, &key(track_id, m))?;
    }
    Ok(())
}

/// The results for a song's current file.
fn for_track(store: &SyncStore, db: &DbHandle, track_id: &str) -> AppResult<Vec<Transcript>> {
    let Some(part) = db.read(|c| tstore::part_key(c, track_id))? else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for m in MODELS {
        if let Some(t) = store.get(&TRANSCRIPTS, &key(track_id, m))?
            && t.part == part
            && usable(&t)
        {
            out.push(t);
        }
    }
    Ok(out)
}

/// Before transcribing here: a result made elsewhere for this file, saved
/// here. Some(words found) when there was one.
pub fn take(store: &SyncStore, db: &DbHandle, track_id: &str) -> AppResult<Option<bool>> {
    let all = for_track(store, db, track_id)?;
    let Some(t) = best(&all) else { return Ok(None) };
    put_here(db, track_id, t)?;
    Ok(Some(!t.text.is_empty()))
}

/// Save a result in the local tables unless something there wins: the
/// user's own lyrics, or a result from a bigger model (or as big and heard
/// better, or as big, heard as well and newer), noting how it was heard.
/// True when it changed anything.
fn put_here(db: &DbHandle, track_id: &str, t: &Transcript) -> AppResult<bool> {
    // From another device (or a Bridge): only a model Platter has, lyrics
    // no longer than the editor takes, and never stamped in the future
    // (so it can't win every later comparison).
    if !usable(t) {
        tracing::warn!("a transcription from another device was left out (unknown model or too long)");
        return Ok(false);
    }
    let clamped;
    let t = if t.at > crate::db::queries::now_s() {
        clamped = Transcript { at: crate::db::queries::now_s(), ..t.clone() };
        &clamped
    } else {
        t
    };
    // Made with the first timing: moved into place before comparing, the
    // way this device moved its own.
    let moved;
    let t = if t.timing < TIMING_VERSION && !t.text.is_empty() {
        moved = Transcript { text: lines::retime_v1(&t.text).unwrap_or_else(|| t.text.clone()), timing: TIMING_VERSION, ..t.clone() };
        &moved
    } else {
        t
    };
    db.write(|c| {
        let cur = user::get(c, track_id)?;
        if cur.as_ref().is_some_and(|o| o.auto.is_none()) {
            return Ok(false);
        }
        let here = match &cur {
            Some(o) => Some((o.auto.clone().unwrap_or_default(), o.updated_at, o.text.trim() == t.text)),
            None => tstore::empty_model(c, track_id)?.map(|(m, at)| (m, at, t.text.is_empty())),
        };
        let heard_here = tstore::heard(c, track_id)?.unwrap_or(tstore::HEARD_OLD);
        if let Some((model, at, same)) = here {
            if model == t.model && same {
                // The same words: only how they were heard can be news.
                if t.heard() > heard_here {
                    tstore::set_heard(c, track_id, t.heard())?;
                }
                return Ok(false);
            }
            if (rank(&model), heard_here, at) > (rank(&t.model), t.heard(), t.at) {
                return Ok(false);
            }
        }
        if t.text.is_empty() && c.execute("DELETE FROM user_lyrics WHERE track_id = ?1 AND auto IS NOT NULL", [track_id])? > 0 {
            crate::lyricsearch::forget_or_log(c, track_id);
        }
        let saved = tstore::save_timed(c, track_id, &t.text, &t.model, t.at, t.timing)?;
        if saved {
            tstore::set_heard(c, track_id, t.heard())?;
        }
        Ok(saved)
    })
}

/// Bring the local tables in line with the store (after the sync file
/// brought changes). Returns the songs that changed and whether each now
/// has words.
pub fn apply(store: &SyncStore, db: &DbHandle) -> AppResult<Vec<(String, bool)>> {
    let mut by_track: BTreeMap<String, Vec<Transcript>> = BTreeMap::new();
    for (k, t) in store.list(&TRANSCRIPTS)? {
        if let Some((id, _)) = split_key(&k) {
            by_track.entry(id.to_owned()).or_default().push(t);
        }
    }
    let mut changed = Vec::new();
    for (id, results) in &by_track {
        if !user::valid_track_id(id) {
            continue;
        }
        let Some(part) = db.read(|c| tstore::part_key(c, id))? else { continue };
        let Some(t) = best(results.iter().filter(|t| t.part == part && usable(t))) else { continue };
        if put_here(db, id, t)? {
            changed.push((id.clone(), !t.text.is_empty()));
        }
    }
    // Forgotten on another device: forgotten here too (never the user's own).
    for k in store.deleted(&TRANSCRIPTS)? {
        let Some((id, _)) = split_key(&k) else { continue };
        if by_track.contains_key(id) || changed.iter().any(|(c, _)| c == id) {
            continue;
        }
        if db.write(|c| tstore::forget(c, id))? {
            changed.push((id.to_owned(), false));
        }
    }
    Ok(changed)
}

/// First run with transcription sync: results already here go in with the
/// time they were made (never over another device's).
pub fn seed(store: &SyncStore, db: &DbHandle) -> AppResult<()> {
    let ids = db.read(user::ids)?;
    for id in ids {
        let Some(own) = db.read(|c| user::get(c, &id))? else { continue };
        let Some(model) = own.auto else { continue };
        let Some(part) = db.read(|c| tstore::part_key(c, &id))? else { continue };
        let timing = db.read(|c| user::auto_timing(c, &id))?.unwrap_or(1);
        let heard = db.read(|c| tstore::heard(c, &id))?.unwrap_or(tstore::HEARD_OLD);
        let t = Transcript { text: own.text, model: model.clone(), part, at: own.updated_at, timing, heard };
        store.seed(&TRANSCRIPTS, &key(&id, &model), &t, (own.updated_at.max(1) as u64) * 1000)?;
    }
    for (id, model, at) in db.read(tstore::empties)? {
        let Some(part) = db.read(|c| tstore::part_key(c, &id))? else { continue };
        let heard = db.read(|c| tstore::heard(c, &id))?.unwrap_or(tstore::HEARD_OLD);
        let t = Transcript { text: String::new(), model: model.clone(), part, at, timing: TIMING_VERSION, heard };
        store.seed(&TRANSCRIPTS, &key(&id, &model), &t, (at.max(1) as u64) * 1000)?;
    }
    Ok(())
}

/// After the user shared a song's lyrics to LRCLIB.
pub fn shared(store: &SyncStore, track_id: &str, at: i64) -> AppResult<()> {
    store.put(&LYRICS_SHARED, track_id, &Shared { at })
}

/// Whether the user shared this song's lyrics (on any device).
pub fn was_shared(store: &SyncStore, track_id: &str) -> AppResult<bool> {
    Ok(store.get(&LYRICS_SHARED, track_id)?.is_some())
}

/// The transcriber's side: results go to and come from the store while
/// "Sync transcriptions to my devices" is on.
pub struct AppShared {
    pub store: SyncStore,
    pub db: DbHandle,
    pub on: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl AppShared {
    fn on(&self) -> bool {
        self.on.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl crate::lyrics::transcribe::SharedResults for AppShared {
    async fn take(&self, track_id: &str) -> Option<bool> {
        if !self.on() {
            return None;
        }
        let (db, id) = (self.db.clone(), track_id.to_owned());
        match self.store.run(move |s| take(s, &db, &id)).await {
            Ok(found) => found,
            Err(e) => {
                tracing::warn!("synced transcription not read: {e}");
                None
            }
        }
    }

    async fn saved(&self, track_id: &str, body: &str, model: &str) {
        if !self.on() {
            return;
        }
        let (db, id, body, model) = (self.db.clone(), track_id.to_owned(), body.to_owned(), model.to_owned());
        let at = crate::db::queries::now_s();
        if let Err(e) = self.store.run(move |s| saved(s, &db, &id, &body, &model, at)).await {
            tracing::warn!("transcription not added to the sync store: {e}");
        }
    }

    async fn forgot(&self, track_id: &str) {
        if !self.on() {
            return;
        }
        let id = track_id.to_owned();
        if let Err(e) = self.store.run(move |s| forgot(s, &id)).await {
            tracing::warn!("forgotten transcription not removed from the sync store: {e}");
        }
    }
}
