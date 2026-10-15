//! The audio check's worker: one song at a time, newest first, with the
//! background thread budget, waiting while the computer is busy. It reads the song the way transcription does (the
//! pre-cache first, else the server behind playback's transfer, giving way
//! to it), decodes it, transcribes a 30 s window with the speech model on
//! this computer, and judges the candidates in order until one matches.
//! A synced match is checked again near the end for drift. Verdicts and the
//! words heard are kept in SQLite. Nothing leaves the computer.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::Notify;

use super::check::{self, HeardWord, Judged, Point, Verdict};
use super::candidates::Stored;
use super::transcribe::audio::{self, AudioError};
use super::transcribe::engine::{self, Recognizer};
use super::transcribe::model::{ModelKind, ModelStore};
use super::transcribe::pace::{PaceGuard, Pacer};
use super::transcribe::{SongAudio, SongReader};
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::AppResult;
use crate::plex::details::Lyrics;

/// Songs waiting at most (older requests are dropped: the user moved on).
const QUEUE_MAX: usize = 3;
/// A check that couldn't read the song is retried after this long.
const RETRY_S: i64 = 3600;

/// One candidate to judge.
#[derive(Debug, Clone, PartialEq)]
pub struct JobCand {
    pub key: String,
    pub hash: String,
    pub lyrics: Lyrics,
}

/// A song to check: its candidates in order.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckJob {
    pub track_id: String,
    pub duration_ms: u32,
    pub cands: Vec<JobCand>,
    /// "Fix lyrics timing automatically": measure drift too.
    pub fix_timing: bool,
}

/// What the lyrics service asks of the check.
pub trait AudioCheck: Send + Sync {
    /// The check can run: its setting is on, a speech model is installed
    /// and this build and computer can transcribe.
    fn available(&self) -> bool;
    /// Check a song (a repeat request for a waiting song is merged).
    fn request(&self, job: CheckJob);
}

/// A song's audio as 16 kHz mono samples.
#[async_trait::async_trait]
pub trait SongPcm: Send + Sync {
    async fn pcm(&self, track_id: &str) -> Result<Vec<f32>, AudioError>;
}

/// Songs read like transcription reads them, decoded with the player's decoder.
pub struct DecodedSongs(pub Arc<dyn SongReader>);

#[async_trait::async_trait]
impl SongPcm for DecodedSongs {
    async fn pcm(&self, track_id: &str) -> Result<Vec<f32>, AudioError> {
        let cancel = Arc::new(AtomicBool::new(false));
        let SongAudio { bytes, hint } = self.0.read(track_id, &cancel).await?;
        match tokio::task::spawn_blocking(move || audio::to_16k_mono(bytes, hint.as_deref(), &cancel)).await {
            Ok(r) => r,
            Err(e) => Err(AudioError::Unreachable(format!("decode task: {e}"))),
        }
    }
}

pub trait CheckEvents: Send + Sync {
    /// A song's candidates were judged.
    fn checked(&self, track_id: &str);
}

/// Called when every candidate disagreed with the recording (the app
/// queues a full transcription when transcription is on).
pub type NoneMatched = Arc<dyn Fn(&str) + Send + Sync>;

pub struct CheckerDeps {
    pub db: DbHandle,
    pub models: ModelStore,
    pub recognizer: Arc<dyn Recognizer>,
    pub audio: Arc<dyn SongPcm>,
    pub events: Arc<dyn CheckEvents>,
    /// "Check lyrics against the song".
    pub enabled: Arc<AtomicBool>,
    pub none_matched: Option<NoneMatched>,
    /// Threads and the busy check (shared with transcription).
    pub pacer: Arc<Pacer>,
}

struct Inner {
    deps: CheckerDeps,
    queue: Mutex<VecDeque<CheckJob>>,
    wake: Notify,
}

/// Cheap to clone.
#[derive(Clone)]
pub struct Checker {
    inner: Arc<Inner>,
}

// ---------------------------------------------------------------- storage

/// A song's stored verdict for a candidate (expired ones are none).
pub fn stored(c: &Connection, track_id: &str, key: &str, hash: &str, now: i64) -> AppResult<Option<Stored>> {
    let row = c
        .query_row(
            "SELECT verdict, score, points, expires_at FROM lyrics_check WHERE track_id = ?1 AND cand_key = ?2 AND text_hash = ?3",
            params![track_id, key, hash],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<i64>>(3)?)),
        )
        .optional()?;
    let Some((v, score, points, expires)) = row else { return Ok(None) };
    if expires.is_some_and(|e| e <= now) {
        return Ok(None);
    }
    let Some(verdict) = Verdict::parse(&v) else { return Ok(None) };
    Ok(Some(Stored { verdict, score: score as f32, points: serde_json::from_str(&points).unwrap_or_default() }))
}

/// Every stored verdict for a song, by (key, hash).
pub fn stored_all(c: &Connection, track_id: &str, now: i64) -> AppResult<Vec<(String, String, Stored)>> {
    let mut st = c.prepare("SELECT cand_key, text_hash FROM lyrics_check WHERE track_id = ?1")?;
    let keys: Vec<(String, String)> = st.query_map(params![track_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (k, h) in keys {
        if let Some(s) = stored(c, track_id, &k, &h, now)? {
            out.push((k, h, s));
        }
    }
    Ok(out)
}

pub fn save(c: &Connection, track_id: &str, key: &str, hash: &str, s: &Stored, now: i64, expires: Option<i64>) -> AppResult<()> {
    c.execute(
        "INSERT INTO lyrics_check (track_id, cand_key, text_hash, verdict, score, points, checked_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (track_id, cand_key, text_hash) DO UPDATE SET verdict = excluded.verdict, score = excluded.score,
           points = excluded.points, checked_at = excluded.checked_at, expires_at = excluded.expires_at",
        params![
            track_id,
            key,
            hash,
            s.verdict.as_str(),
            s.score as f64,
            serde_json::to_string(&s.points).unwrap_or_else(|_| "[]".into()),
            now,
            expires
        ],
    )?;
    Ok(())
}

fn heard_get(c: &Connection, track_id: &str, start_ms: u32) -> AppResult<Option<Vec<HeardWord>>> {
    let words: Option<String> = c
        .query_row("SELECT words FROM lyrics_heard WHERE track_id = ?1 AND start_ms = ?2", params![track_id, start_ms], |r| r.get(0))
        .optional()?;
    Ok(words.and_then(|w| serde_json::from_str(&w).ok()))
}

fn heard_put(c: &Connection, track_id: &str, w: (u32, u32), words: &[HeardWord], now: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO lyrics_heard (track_id, start_ms, end_ms, words, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (track_id, start_ms) DO UPDATE SET end_ms = excluded.end_ms, words = excluded.words,
           created_at = excluded.created_at",
        params![track_id, w.0, w.1, serde_json::to_string(words).unwrap_or_else(|_| "[]".into()), now],
    )?;
    Ok(())
}

/// A judged candidate as stored: the offset as a point (or two for drift).
pub fn to_stored(j: &Judged, drift: Option<Point>) -> Stored {
    let mut points = Vec::new();
    if j.verdict.matched()
        && let Some(at) = j.at_ms
    {
        points.push(Point { at_ms: at, offset_ms: j.offset_ms });
        if let Some(d) = drift {
            points.push(d);
        }
    }
    let verdict = if j.verdict == Verdict::Match && points.iter().any(|p| p.offset_ms.abs() >= check::OFFSET_MIN_MS) {
        Verdict::MatchOffset
    } else {
        j.verdict
    };
    Stored { verdict, score: j.score, points }
}

// ---------------------------------------------------------------- worker

/// Why a window couldn't be heard.
enum Fail {
    /// Try again later (the server can't be reached).
    Later,
    /// This file can't be read or decoded.
    Never,
}

impl Checker {
    pub fn new(deps: CheckerDeps) -> Self {
        Self { inner: Arc::new(Inner { deps, queue: Mutex::new(VecDeque::new()), wake: Notify::new() }) }
    }

    pub fn spawn(&self) {
        let me = self.clone();
        tokio::spawn(async move { me.run().await });
    }

    /// The smallest installed model (the check needs little).
    fn model(&self) -> Option<(PathBuf, ModelKind)> {
        ModelKind::ALL.iter().find_map(|k| self.inner.deps.models.installed(*k).map(|p| (p, *k)))
    }

    /// Songs waiting.
    pub fn queued(&self) -> Vec<String> {
        self.inner.queue.lock().iter().map(|j| j.track_id.clone()).collect()
    }

    /// Check every waiting song now (what the worker does, one by one).
    pub async fn drain(&self) -> AppResult<()> {
        loop {
            let job = self.inner.queue.lock().pop_front();
            let Some(job) = job else { return Ok(()) };
            self.check(&job).await?;
        }
    }

    async fn run(self) {
        // Held while songs wait (the busy check reads the processor only then).
        let mut pace: Option<PaceGuard> = None;
        loop {
            let notified = self.inner.wake.notified();
            if !self.inner.queue.lock().is_empty() {
                if pace.is_none() {
                    pace = Some(self.inner.deps.pacer.want());
                }
                // Wait while other programs keep the processor busy.
                self.inner.deps.pacer.wait_calm().await;
            }
            if self.inner.deps.recognizer.crashed_too_often() {
                // Off for the session after repeated crashes.
                self.inner.queue.lock().clear();
            }
            let job = self.inner.queue.lock().pop_front();
            match job {
                Some(job) => {
                    if let Err(e) = self.check(&job).await {
                        tracing::warn!(kind = e.kind(), "lyrics check failed");
                    }
                    if self.inner.queue.lock().is_empty() {
                        self.inner.deps.recognizer.unload();
                    }
                }
                None => {
                    drop(pace.take());
                    notified.await
                }
            }
        }
    }

    /// Judge a song's candidates (the worker's step; tests call it directly).
    pub async fn check(&self, job: &CheckJob) -> AppResult<()> {
        let d = &self.inner.deps;
        let Some((model, kind)) = self.model() else { return Ok(()) };
        let first = job.cands.iter().find_map(|c| check::sung_span(&c.lyrics).map(|s| s.0));
        let w1 = check::first_window(first, job.duration_ms);
        let mut pcm: Option<Vec<f32>> = None;
        let heard1 = match self.heard(job, w1, &mut pcm, &model, kind).await {
            Ok(h) => h,
            Err(f) => {
                // Not checked: shown as such, asked again later (or never, for a bad file).
                let now = now_s();
                let expires = match f {
                    Fail::Later => now + RETRY_S,
                    Fail::Never => now + 30 * 24 * 3600,
                };
                let cands = job.cands.clone();
                let id = job.track_id.clone();
                d.db
                    .write_async(move |c| {
                        let s = Stored { verdict: Verdict::CantTell, score: 0.0, points: Vec::new() };
                        for cand in &cands {
                            save(c, &id, &cand.key, &cand.hash, &s, now, Some(expires))?;
                        }
                        Ok(())
                    })
                    .await?;
                d.events.checked(&job.track_id);
                return Ok(());
            }
        };
        let mut all_no = !job.cands.is_empty();
        for cand in &job.cands {
            let j = check::judge(&heard1, &cand.lyrics, w1);
            let mut drift = None;
            if j.verdict.matched() && job.fix_timing && cand.lyrics.synced {
                drift = self.drift(job, cand, &j, w1, &mut pcm, &model, kind).await;
            }
            let s = to_stored(&j, drift);
            all_no &= s.verdict == Verdict::NoMatch;
            let matched = s.verdict.matched();
            let (id, key, hash, now) = (job.track_id.clone(), cand.key.clone(), cand.hash.clone(), now_s());
            d.db.write_async(move |c| save(c, &id, &key, &hash, &s, now, None)).await?;
            if matched {
                // The first that matches wins; the rest needn't be judged.
                break;
            }
        }
        if all_no && let Some(f) = &d.none_matched {
            f(&job.track_id);
        }
        d.events.checked(&job.track_id);
        Ok(())
    }

    /// The second window: the offset there, when it differs enough to be drift.
    #[allow(clippy::too_many_arguments)]
    async fn drift(
        &self,
        job: &CheckJob,
        cand: &JobCand,
        j: &Judged,
        w1: (u32, u32),
        pcm: &mut Option<Vec<f32>>,
        model: &std::path::Path,
        kind: ModelKind,
    ) -> Option<Point> {
        let (_, last) = check::sung_span(&cand.lyrics)?;
        let last_heard = (last as i64 + j.offset_ms as i64).max(0) as u32;
        let w2 = check::second_window(w1, Some(last_heard), job.duration_ms)?;
        let heard2 = self.heard(job, w2, pcm, model, kind).await.ok()?;
        // Judged against the lyrics as written: the window is where they'd be heard.
        let back = (w2.0 as i64 - j.offset_ms as i64).max(0) as u32;
        let j2 = check::judge(&heard2, &cand.lyrics, (back, back + check::WINDOW_MS));
        if !j2.verdict.matched() || (j2.offset_ms - j.offset_ms).abs() < check::DRIFT_MIN_MS {
            return None;
        }
        Some(Point { at_ms: j2.at_ms?, offset_ms: j2.offset_ms })
    }

    /// The words heard in a window: from the cache, else transcribed now.
    async fn heard(
        &self,
        job: &CheckJob,
        w: (u32, u32),
        pcm: &mut Option<Vec<f32>>,
        model: &std::path::Path,
        kind: ModelKind,
    ) -> Result<Vec<HeardWord>, Fail> {
        let d = &self.inner.deps;
        let id = job.track_id.clone();
        if let Ok(Some(h)) = d.db.read_async(move |c| heard_get(c, &id, w.0)).await {
            return Ok(h);
        }
        if pcm.is_none() {
            *pcm = Some(self.decode(&job.track_id).await?);
        }
        let all = pcm.as_ref().expect("just decoded");
        let rate = audio::RATE as usize;
        let (a, b) = ((w.0 as usize * rate / 1000).min(all.len()), (w.1 as usize * rate / 1000).min(all.len()));
        let slice = all[a..b].to_vec();
        let words = if slice.len() < rate * 3 || check::rms(&slice) < check::QUIET_RMS {
            Vec::new()
        } else {
            let rec = d.recognizer.clone();
            let model = model.to_path_buf();
            let threads = d.pacer.threads(false);
            let res = tokio::task::spawn_blocking(move || {
                rec.transcribe(&model, kind, &slice, threads, engine::LANGUAGE, Arc::new(AtomicBool::new(false)), Arc::new(|_| {}))
            })
            .await;
            match res {
                Ok(Ok(segs)) => check::heard_words(&segs, w.0),
                Ok(Err(e)) => {
                    tracing::info!("lyrics check couldn't transcribe: {e:?}");
                    return Err(Fail::Later);
                }
                Err(e) => {
                    tracing::warn!("lyrics check task failed: {e}");
                    return Err(Fail::Later);
                }
            }
        };
        let (id, now) = (job.track_id.clone(), now_s());
        let stored = words.clone();
        let _ = d.db.write_async(move |c| heard_put(c, &id, w, &stored, now)).await;
        Ok(words)
    }

    async fn decode(&self, track_id: &str) -> Result<Vec<f32>, Fail> {
        match self.inner.deps.audio.pcm(track_id).await {
            Ok(p) => Ok(p),
            Err(AudioError::Unreadable(m)) => {
                tracing::info!("lyrics check can't use this file: {m}");
                Err(Fail::Never)
            }
            Err(e) => {
                tracing::info!("lyrics check can't read the song now: {e:?}");
                Err(Fail::Later)
            }
        }
    }
}

impl AudioCheck for Checker {
    fn available(&self) -> bool {
        self.inner.deps.enabled.load(Ordering::Relaxed)
            && self.inner.deps.recognizer.unavailable().is_none()
            && !self.inner.deps.recognizer.crashed_too_often()
            && self.model().is_some()
    }

    fn request(&self, job: CheckJob) {
        {
            let mut q = self.inner.queue.lock();
            q.retain(|j| j.track_id != job.track_id);
            q.push_front(job);
            q.truncate(QUEUE_MAX);
        }
        self.inner.wake.notify_one();
    }
}
