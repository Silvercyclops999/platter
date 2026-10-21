//! Lyrics for every song, on the Bridge: songs with no lyrics anywhere are
//! transcribed at night on the server, and every device gets the result.
//!
//! It's the desktop's own transcriber ([`Transcriber`]) with the Bridge's
//! parts: songs are read from the music folder mounted at `/music` (never
//! written), "has lyrics already" asks Plex (and LRCLIB when the computer's
//! setting allows it), the night hours pause the queue, and results go into
//! the sync store's `transcripts` namespace, which the Bridge sends to each
//! paired device (`run::Bridge::flush_entries`). The speech model runs in
//! the helper process (`platter-bridge --speech-worker`) at the lowest
//! priority with the Light thread budget, waits while the server is busy,
//! and rests between songs, as on a computer.
//!
//! What to do: songs the Bridge has no lyrics for, minus the ones a
//! computer said it has (`Msg::LyricsHave`) and the ones marked here (they
//! have lyrics from Plex or LRCLIB, or couldn't be read lately), most
//! played first, a few at a time.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::wire::{LyricsLine, LyricsState, NightHours};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::AppResult;
use crate::lyrics::transcribe::audio::AudioError;
use crate::lyrics::transcribe::engine::Recognizer;
use crate::lyrics::transcribe::model::{ModelFetch, ModelKind, ModelStore};
use crate::lyrics::transcribe::{
    Conditions, Deps, FailureKind, Official, OtherLyrics, PauseReason, SongAudio, SongReader, TranscribeEvents, TranscribeStatus, Transcriber, gpu, pace,
    speed,
};
use crate::sync::SyncStore;

/// Marks: `<prefix><track>|<part>` = `other` (lyrics from Plex or LRCLIB)
/// or `retry:<unix s>` (couldn't be read or transcribed; again after then).
pub const SKIP_PREFIX: &str = "bridge.lyricsSkip.";
/// A computer's have list: `<prefix><link id>` = JSON array of track ids.
const HAVE_PREFIX: &str = "bridge.lyricsHave.";
const OPTS_KEY: &str = "bridge.lyricsOpts";
/// Songs put in the queue at a time.
pub const BATCH: usize = 6;
/// A song that couldn't be read or transcribed is tried again after this.
pub const RETRY_S: i64 = 7 * 86_400;
/// After the model download failed, wait this long before trying again.
pub const DOWNLOAD_RETRY: Duration = Duration::from_secs(3600);
/// Most ids kept from one computer's have list.
pub const HAVE_MAX: usize = 100_000;
/// How long the done and left counts are reused.
const COUNTS_FOR: Duration = Duration::from_secs(60);
/// The biggest song file the Bridge reads (it's held in memory to decode).
/// Bigger ones are skipped and marked like an unreadable file.
pub const MAX_SONG_BYTES: u64 = 300 * 1024 * 1024;
/// The model the Bridge uses: no graphics card in a container.
pub const MODEL: ModelKind = ModelKind::Small;

/// What the computers decided (Settings > Bridge), kept on the Bridge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Opts {
    pub on: bool,
    pub hours: NightHours,
    /// Seconds east of UTC on the computer.
    pub utc_offset_s: i64,
    /// Ask LRCLIB too before transcribing.
    pub lrclib: bool,
}

/// Unix seconds now.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The night hours as the transcriber's pause condition.
struct Night {
    opts: Arc<RwLock<Opts>>,
    clock: Clock,
}

#[async_trait]
impl Conditions for Night {
    async fn pause_reason(&self) -> Option<PauseReason> {
        let o = *self.opts.read();
        (!o.hours.contains((self.clock)(), o.utc_offset_s)).then_some(PauseReason::Hours)
    }
}

/// Where a song's file is on the mount: Plex's path for it moved under
/// MUSIC_PLEX_PATH to the mount, and still inside the mount once links are
/// followed. None for anything else.
pub fn resolve(plex_root: &str, mount: &Path, file: &str) -> Option<PathBuf> {
    let p = crate::musicshare::map_under(plex_root, &mount.to_string_lossy(), file)?;
    let real = p.canonicalize().ok()?;
    let root = mount.canonicalize().ok()?;
    (real.starts_with(&root) && real.is_file()).then_some(real)
}

/// Plex's path for the mounted folder: MUSIC_PLEX_PATH, or the Bridge's own
/// match of the folder against Plex's paths when that isn't set. Empty
/// until one is known.
pub type MusicRoot = Arc<RwLock<String>>;

/// Songs from the music folder mounted on the Bridge, read only.
pub struct MountSongs {
    pub db: DbHandle,
    pub plex_root: MusicRoot,
    /// Where it's mounted (/music).
    pub mount: PathBuf,
}

#[async_trait]
impl SongReader for MountSongs {
    async fn read(&self, track_id: &str, cancel: &AtomicBool) -> Result<SongAudio, AudioError> {
        self.read_up_to(track_id, cancel, MAX_SONG_BYTES).await?.ok_or_else(|| AudioError::Unreadable("file too big".into()))
    }

    async fn read_up_to(&self, track_id: &str, cancel: &AtomicBool, max_bytes: u64) -> Result<Option<SongAudio>, AudioError> {
        let id = track_id.to_owned();
        let row = self
            .db
            .read_async(move |c| {
                Ok(c.query_row("SELECT file, codec, container FROM tracks WHERE id = ?1", params![id], |r| {
                    Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?))
                })
                .optional()?)
            })
            .await
            .map_err(|e| AudioError::Unreachable(e.to_string()))?;
        let Some((Some(file), codec, container)) = row else { return Err(AudioError::Unreadable("not in the library".into())) };
        if !self.mount.is_dir() {
            // The mount went away: the song is fine, try later.
            return Err(AudioError::Unreachable("the music folder isn't mounted".into()));
        }
        let (root, mount) = (self.plex_root.read().clone(), self.mount.clone());
        if root.is_empty() {
            return Err(AudioError::Unreachable("Plex's path for the music folder isn't known yet".into()));
        }
        let path = tokio::task::spawn_blocking(move || resolve(&root, &mount, &file))
            .await
            .ok()
            .flatten()
            .ok_or_else(|| AudioError::Unreadable("not under the music folder".into()))?;
        if cancel.load(Ordering::Relaxed) {
            return Err(AudioError::Cancelled);
        }
        let len = tokio::fs::metadata(&path).await.map_err(|e| AudioError::Unreadable(e.to_string()))?.len();
        if len > max_bytes.min(MAX_SONG_BYTES) {
            return Ok(None);
        }
        // tokio::fs::read opens the file read-only.
        let bytes = tokio::fs::read(&path).await.map_err(|e| AudioError::Unreadable(e.to_string()))?;
        let hint = crate::audio::decode::extension_hint(codec.as_deref(), container.as_deref())
            .or_else(|| path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()));
        Ok(Some(SongAudio { bytes, hint }))
    }
}

/// "Has lyrics already", remembering a yes so the song isn't asked about
/// every night.
struct MarkOther {
    inner: Arc<dyn OtherLyrics>,
    db: DbHandle,
}

#[async_trait]
impl OtherLyrics for MarkOther {
    async fn has_lyrics(&self, track_id: &str) -> bool {
        let yes = self.inner.has_lyrics(track_id).await;
        if yes {
            let id = track_id.to_owned();
            if let Err(e) = self.db.write_async(move |c| mark(c, &id, "other")).await {
                tracing::warn!("Lyrics: couldn't note a song that has lyrics already: {e}");
            }
        }
        yes
    }
    async fn official_synced(&self, _track_id: &str) -> Official {
        Official::Unknown
    }
}

fn skip_key(c: &Connection, track_id: &str) -> AppResult<Option<String>> {
    let part = crate::lyrics::transcribe::store::part_key(c, track_id)?;
    Ok(part.map(|p| format!("{SKIP_PREFIX}{track_id}|{p}")))
}

/// Mark a song for its current file: `other` or `retry:<until>`.
pub fn mark(c: &Connection, track_id: &str, what: &str) -> AppResult<()> {
    if let Some(k) = skip_key(c, track_id)? {
        kv_set(c, &k, what)?;
    }
    Ok(())
}

/// The next songs to look at, most played first: no lyrics here, not marked
/// (or a retry that's due), not on a computer's have list.
pub fn next_songs(c: &Connection, have: &HashSet<String>, now: i64, n: usize) -> AppResult<Vec<String>> {
    let mut st = c.prepare(
        "SELECT t.id FROM tracks t
         WHERE t.file IS NOT NULL AND t.file != '' AND t.part_key != ''
           AND NOT EXISTS (SELECT 1 FROM user_lyrics u WHERE u.track_id = t.id)
           AND NOT EXISTS (SELECT 1 FROM lyrics_auto_empty e WHERE e.track_id = t.id)
           AND NOT EXISTS (SELECT 1 FROM kv k WHERE k.key = ?1 || t.id || '|' || t.part_key
                AND (k.value = 'other' OR CAST(substr(k.value, 7) AS INTEGER) > ?2))
         ORDER BY t.view_count DESC, t.last_viewed_at DESC, t.pk",
    )?;
    let mut rows = st.query(params![SKIP_PREFIX, now])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        if have.contains(&id) {
            continue;
        }
        out.push(id);
        if n > 0 && out.len() >= n {
            break;
        }
    }
    Ok(out)
}

/// (songs done here, songs left about).
pub fn counts(c: &Connection, have: &HashSet<String>, now: i64) -> AppResult<(usize, usize)> {
    let done: i64 = c.query_row(
        "SELECT (SELECT count(*) FROM user_lyrics WHERE auto IS NOT NULL) + (SELECT count(*) FROM lyrics_auto_empty)",
        [],
        |r| r.get(0),
    )?;
    Ok((done.max(0) as usize, next_songs(c, have, now, 0)?.len()))
}

/// The transcriber's news, kept for the status line; songs that failed are
/// marked to try again later, so a bad file doesn't come back every night.
struct Recorder {
    db: DbHandle,
    last: Mutex<Option<TranscribeStatus>>,
    clock: Clock,
    marked: Mutex<Option<String>>,
}

impl TranscribeEvents for Recorder {
    fn status(&self, s: &TranscribeStatus) {
        if let Some(f) = s.error.as_ref().filter(|f| f.kind == FailureKind::Transcribe)
            && let Some(id) = &f.track_id
        {
            let model_here = s.models.iter().any(|m| m.kind == s.model && m.installed);
            let mut m = self.marked.lock();
            // The same failure is said in every status until the next song
            // goes in; each new failure (the same song again included) is marked.
            if model_here && m.as_deref() != Some(id.as_str()) {
                *m = Some(id.clone());
                let until = format!("retry:{}", (self.clock)() + RETRY_S);
                let (db, id, message) = (self.db.clone(), id.clone(), f.message.clone());
                let write = move || match db.write(|c| mark(c, &id, &until)) {
                    Err(e) => tracing::warn!("Lyrics: couldn't note a song to try again later: {e}"),
                    Ok(()) => tracing::info!("Lyrics: a song couldn't be transcribed ({message}). It's tried again in a week."),
                };
                match tokio::runtime::Handle::try_current() {
                    Ok(rt) => drop(rt.spawn_blocking(write)),
                    Err(_) => write(),
                }
            }
        } else {
            // No failure said now (the queue moved on): the next one is new.
            *self.marked.lock() = None;
        }
        *self.last.lock() = Some(s.clone());
    }
    fn done(&self, _track_id: &str, _found: bool) {}
}

/// Everything the Bridge's lyrics work runs on (real ones from [`real`],
/// fakes in tests).
pub struct LyricsDeps {
    pub db: DbHandle,
    pub store: SyncStore,
    pub models: ModelStore,
    pub fetch: Arc<dyn ModelFetch>,
    pub recognizer: Arc<dyn Recognizer>,
    pub songs: Arc<dyn SongReader>,
    pub other: Arc<dyn OtherLyrics>,
    pub pacer: Arc<pace::Pacer>,
    /// The music folder is mounted.
    pub music: bool,
    /// Plex's path for it (shared with `songs`).
    pub root: MusicRoot,
    pub clock: Clock,
    /// LRCLIB on or off for `other` (the real one reads it).
    pub lrclib: Arc<AtomicBool>,
}

pub struct BridgeLyrics {
    t: Transcriber,
    db: DbHandle,
    opts: Arc<RwLock<Opts>>,
    have: RwLock<HashMap<String, HashSet<String>>>,
    /// Every computer's list together (rebuilt when one changes).
    have_all: RwLock<Arc<HashSet<String>>>,
    /// The counts need reading again.
    counts_stale: AtomicBool,
    music: bool,
    root: MusicRoot,
    rec: Arc<Recorder>,
    clock: Clock,
    lrclib: Arc<AtomicBool>,
    counts: Mutex<Option<(Instant, usize, usize)>>,
    /// When the model download last failed.
    download_failed: Mutex<Option<Instant>>,
    retry_after: Duration,
}

impl BridgeLyrics {
    pub fn new(d: LyricsDeps) -> Arc<Self> {
        Self::with_retry(d, DOWNLOAD_RETRY)
    }

    /// With another wait after a failed download (tests).
    pub fn with_retry(d: LyricsDeps, retry_after: Duration) -> Arc<Self> {
        let kept: Opts = d.db.read(|c| kv_get(c, OPTS_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let opts = Arc::new(RwLock::new(kept));
        d.lrclib.store(kept.lrclib, Ordering::Relaxed);
        let rec = Arc::new(Recorder { db: d.db.clone(), last: Mutex::default(), clock: d.clock.clone(), marked: Mutex::default() });
        // The Light budget at the lowest priority, the busy pause on, never "faster while away".
        d.pacer.set_options(pace::ProcessorUse::Light, true, false);
        let t = Transcriber::new(Deps {
            db: d.db.clone(),
            models: d.models,
            fetch: d.fetch,
            recognizer: d.recognizer,
            songs: d.songs,
            other: Arc::new(MarkOther { inner: d.other, db: d.db.clone() }),
            conditions: Arc::new(Night { opts: opts.clone(), clock: d.clock.clone() }),
            events: rec.clone(),
            pacer: d.pacer,
            gpu: gpu::Gpu::none(),
            speeds: Arc::new(speed::Speeds::load(None)),
            voice: None,
        });
        t.set_shared(Arc::new(crate::sync::transcripts::AppShared { store: d.store, db: d.db.clone(), on: Arc::new(AtomicBool::new(true)) }));
        t.set_options(kept.on, Some(MODEL));
        let mut have: HashMap<String, HashSet<String>> = HashMap::new();
        let rows: Vec<(String, String)> = d
            .db
            .read(|c| {
                let mut st = c.prepare("SELECT key, value FROM kv WHERE key >= ?1 AND key < ?2")?;
                let rows = st.query_map(params![HAVE_PREFIX, format!("{HAVE_PREFIX}\u{10FFFF}")], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.filter_map(Result::ok).collect())
            })
            .unwrap_or_default();
        for (k, v) in rows {
            let ids: Vec<String> = serde_json::from_str(&v).unwrap_or_default();
            have.insert(k[HAVE_PREFIX.len()..].to_string(), ids.into_iter().collect());
        }
        let all: HashSet<String> = have.values().flat_map(|s| s.iter().cloned()).collect();
        Arc::new(Self {
            t,
            db: d.db,
            opts,
            have: RwLock::new(have),
            have_all: RwLock::new(Arc::new(all)),
            counts_stale: AtomicBool::new(true),
            music: d.music,
            root: d.root,
            rec,
            clock: d.clock,
            lrclib: d.lrclib,
            counts: Mutex::default(),
            download_failed: Mutex::default(),
            retry_after,
        })
    }

    /// Start the queue's worker (on the current runtime).
    pub fn spawn(&self) {
        self.t.spawn();
    }

    pub fn transcriber(&self) -> &Transcriber {
        &self.t
    }

    pub fn opts(&self) -> Opts {
        *self.opts.read()
    }

    /// What a computer sent. Kept, so a restart before the next one keeps it.
    pub async fn set_opts(&self, o: Opts) {
        let before = std::mem::replace(&mut *self.opts.write(), o);
        self.lrclib.store(o.lrclib, Ordering::Relaxed);
        if before == o {
            return;
        }
        if let Ok(json) = serde_json::to_string(&o)
            && let Err(e) = self.db.write_async(move |c| kv_set(c, OPTS_KEY, &json)).await
        {
            tracing::warn!("Lyrics: couldn't keep the settings from the computer: {e}");
        }
        if before.on != o.on {
            tracing::info!("Lyrics for every song: {}.", if o.on { "on" } else { "off" });
        }
        self.t.set_options(o.on, Some(MODEL));
        self.t.poke();
    }

    /// A computer's list of songs it has lyrics for.
    pub async fn set_have(&self, link: &str, ids: Vec<String>) {
        let set: HashSet<String> = ids.into_iter().filter(|i| crate::lyrics::user::valid_track_id(i)).take(HAVE_MAX).collect();
        if let Ok(json) = serde_json::to_string(&set.iter().collect::<Vec<_>>()) {
            let key = format!("{HAVE_PREFIX}{link}");
            if let Err(e) = self.db.write_async(move |c| kv_set(c, &key, &json)).await {
                tracing::warn!("Lyrics: couldn't keep a computer's list of songs with lyrics: {e}");
            }
        }
        self.have.write().insert(link.to_string(), set);
        self.rebuild_have();
    }

    /// An unpaired computer's list goes.
    pub async fn forget_link(&self, link: &str) {
        self.have.write().remove(link);
        self.rebuild_have();
        let key = format!("{HAVE_PREFIX}{link}");
        let gone = self
            .db
            .write_async(move |c| {
                c.execute("DELETE FROM kv WHERE key = ?1", [key])?;
                Ok(())
            })
            .await;
        if let Err(e) = gone {
            tracing::warn!("Lyrics: couldn't forget an unpaired computer's list: {e}");
        }
    }

    fn rebuild_have(&self) {
        let all: HashSet<String> = self.have.read().values().flat_map(|s| s.iter().cloned()).collect();
        *self.have_all.write() = Arc::new(all);
        self.counts_stale.store(true, Ordering::SeqCst);
    }

    fn have_all(&self) -> Arc<HashSet<String>> {
        self.have_all.read().clone()
    }

    /// The folder is mounted and Plex's path for it is known.
    fn music(&self) -> bool {
        self.music && !self.root.read().is_empty()
    }

    /// The Bridge matched the mounted folder with Plex's paths: used for
    /// reading songs while MUSIC_PLEX_PATH isn't set (reading only; adding
    /// albums still waits for it).
    pub fn suggest_root(&self, path: &str) {
        let mut r = self.root.write();
        if r.is_empty() && !path.trim().is_empty() {
            tracing::info!("Lyrics for every song reads songs from the music folder as Plex's {path}.");
            *r = path.trim().to_string();
            drop(r);
            *self.counts.lock() = None;
        }
    }

    fn in_hours(&self) -> bool {
        let o = self.opts();
        o.hours.contains((self.clock)(), o.utc_offset_s)
    }

    fn status(&self) -> TranscribeStatus {
        self.t.status()
    }

    /// The download failed (or the file didn't match) and its wait isn't over.
    fn download_waiting(&self, s: &TranscribeStatus) -> bool {
        let failed = s.error.as_ref().is_some_and(|e| matches!(e.kind, FailureKind::Download | FailureKind::Checksum | FailureKind::Disk));
        let mut at = self.download_failed.lock();
        if !failed {
            *at = None;
            return false;
        }
        let since = *at.get_or_insert_with(Instant::now);
        since.elapsed() < self.retry_after
    }

    /// Every little while: inside the night hours, with the queue empty, the
    /// next few songs go in. Returns how many.
    pub async fn tick(&self) -> usize {
        let o = self.opts();
        if o.on {
            self.refresh_counts().await;
        }
        if !o.on || !self.music() || !self.in_hours() {
            return 0;
        }
        let s = self.status();
        if !s.available || s.current.is_some() || s.queued > 0 || s.models.iter().any(|m| m.downloading) || self.download_waiting(&s) {
            return 0;
        }
        let have = self.have_all();
        let now = (self.clock)();
        let ids = match self.db.read_async(move |c| next_songs(c, &have, now, BATCH)).await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!("Lyrics: couldn't read the library: {e}");
                return 0;
            }
        };
        if ids.is_empty() {
            return 0;
        }
        self.counts_stale.store(true, Ordering::SeqCst);
        match self.t.transcribe_background(ids) {
            Ok(n) => n,
            Err(e) => {
                tracing::info!("Lyrics: {e}");
                0
            }
        }
    }

    /// The done and left counts, read off the async threads when they're
    /// old or something changed (each tick).
    async fn refresh_counts(&self) {
        let fresh = self.counts.lock().is_some_and(|(at, _, _)| at.elapsed() < COUNTS_FOR);
        if fresh && !self.counts_stale.load(Ordering::SeqCst) {
            return;
        }
        self.counts_stale.store(false, Ordering::SeqCst);
        let have = self.have_all();
        let now = (self.clock)();
        match self.db.read_async(move |c| counts(c, &have, now)).await {
            Ok((d, l)) => *self.counts.lock() = Some((Instant::now(), d, l)),
            Err(e) => tracing::warn!("Lyrics: couldn't count the songs: {e}"),
        }
    }

    /// The counts as last read (read once here when there are none yet).
    fn counted(&self) -> (usize, usize) {
        if let Some((_, d, l)) = *self.counts.lock() {
            return (d, l);
        }
        let have = self.have_all();
        let now = (self.clock)();
        let (d, l) = self.db.read(|c| counts(c, &have, now)).unwrap_or((0, 0));
        *self.counts.lock() = Some((Instant::now(), d, l));
        (d, l)
    }


    /// The line for Settings > Bridge on the computers.
    pub fn line(&self) -> LyricsLine {
        let o = self.opts();
        let s = self.rec.last.lock().clone().unwrap_or_else(|| self.status());
        let mut line = LyricsLine { hours: o.hours, ..Default::default() };
        if !o.on {
            line.state = LyricsState::Off;
            return line;
        }
        if !s.available {
            line.state = LyricsState::Unavailable;
            line.problem = Some("This Bridge was built without transcription.".into());
            return line;
        }
        if !self.music {
            line.state = LyricsState::NoMusic;
            line.problem = Some("Mount your music folder at /music (read-only is fine), so the Bridge can hear your songs.".into());
            return line;
        }
        if !self.music() {
            line.state = LyricsState::NoMusic;
            line.problem = Some("Waiting to read your Plex library, to match the music folder with it.".into());
            return line;
        }
        let (done, left) = self.counted();
        line.done = done;
        line.left = left;
        line.song = s.current.as_ref().and_then(|c| c.title.clone());
        if let Some(e) = s.error.as_ref().filter(|e| e.track_id.is_none()) {
            line.state = LyricsState::Problem;
            line.problem = Some(match e.kind {
                FailureKind::Download => "The speech model didn't download. The Bridge tries again in an hour.".into(),
                FailureKind::Checksum => "The speech model didn't match its checksum, so it was deleted. The Bridge tries again in an hour.".into(),
                FailureKind::Disk => format!("{} Free some space in the Bridge's data folder.", e.message),
                FailureKind::Transcribe => e.message.clone(),
            });
            return line;
        }
        line.state = if s.models.iter().any(|m| m.downloading) {
            LyricsState::Downloading
        } else if s.paused == Some(PauseReason::Busy) {
            LyricsState::Busy
        } else if s.current.is_some() && s.paused.is_none() {
            LyricsState::Working
        } else if left == 0 && s.queued == 0 && s.current.is_none() {
            LyricsState::Done
        } else if self.in_hours() {
            LyricsState::Working
        } else {
            LyricsState::Waiting
        };
        line
    }
}

/// The real parts: the speech helper is this program started again with
/// the worker flag, the model goes in `<data>/models`, songs come from the
/// mount, "has lyrics" asks Plex (and LRCLIB when allowed).
/// `music_dir`: where the music folder is mounted; `music_root`: MUSIC_PLEX_PATH.
pub fn real(
    data: &Path,
    db: DbHandle,
    store: SyncStore,
    music_dir: PathBuf,
    music_root: Option<String>,
    plex: Option<crate::plex::PlexState>,
) -> AppResult<Arc<BridgeLyrics>> {
    use crate::lyrics::transcribe::{helper, model};
    let models = ModelStore::new(data.join("models"), model::catalogue(), model::os_free_space());
    let pacer = pace::Pacer::new(Arc::new(pace::SystemSignals::default()), pace::Timing::default());
    let lowest = {
        let p = pacer.clone();
        Arc::new(move || p.light())
    };
    let recognizer: Arc<dyn Recognizer> = Arc::new(helper::Helper::new(Arc::new(helper::ProcessLaunch::current()), lowest, helper::IDLE_EXIT));
    let mounted = music_dir.is_dir();
    let root: MusicRoot = Arc::new(RwLock::new(music_root.unwrap_or_default()));
    let songs: Arc<dyn SongReader> = Arc::new(MountSongs { db: db.clone(), plex_root: root.clone(), mount: music_dir });
    let lrclib = Arc::new(AtomicBool::new(false));
    let other: Arc<dyn OtherLyrics> = match plex {
        Some(p) => Arc::new(crate::lyrics::transcribe::commands::AppOther(crate::lyrics::LyricsService::new(
            db.clone(),
            Arc::new(crate::artistinfo::http::ReqwestHttp::new()?),
            Arc::new(crate::lyrics::commands::PlexSource(p)),
            crate::lyrics::Switches { lrclib: lrclib.clone(), ..Default::default() },
        ))),
        None => Arc::new(NoOther),
    };
    Ok(BridgeLyrics::new(LyricsDeps {
        db,
        store,
        models,
        fetch: Arc::new(model::HttpFetch::new()),
        recognizer,
        songs,
        other,
        pacer,
        music: mounted,
        root,

        clock: Arc::new(crate::db::queries::now_s),
        lrclib,
    }))
}

/// No Plex to ask (tests, or no server yet): nothing has lyrics elsewhere.
pub struct NoOther;

#[async_trait]
impl OtherLyrics for NoOther {
    async fn has_lyrics(&self, _track_id: &str) -> bool {
        false
    }
}

/// The lyrics line in plain words, for the status page.
pub fn words(l: &LyricsLine) -> String {
    let tally = format!("{} done, about {} to go", l.done, l.left);
    match l.state {
        LyricsState::Off => "Lyrics for every song: off.".into(),
        LyricsState::Unavailable | LyricsState::NoMusic | LyricsState::Problem => format!("Lyrics: {}", l.problem.clone().unwrap_or_default()),
        LyricsState::Downloading => "Lyrics: downloading the speech model.".into(),
        LyricsState::Waiting => format!("Lyrics: waiting for the night hours. {tally}."),
        LyricsState::Busy => format!("Lyrics: waiting while the server is busy. {tally}."),
        LyricsState::Working => match &l.song {
            Some(s) => format!("Lyrics: transcribing {s}. {tally}."),
            None => format!("Lyrics: working. {tally}."),
        },
        LyricsState::Done => format!("Lyrics: every song has lyrics ({} done).", l.done),
    }
}
