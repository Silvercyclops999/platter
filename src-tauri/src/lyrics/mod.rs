//! Lyrics for the lyrics view and the full-window view (docs/IPC.md
//! `lyrics_get`).
//!
//! Candidates in order: the song's server (a sidecar .lrc or .txt file, or
//! the server's own lyrics), then lyrics the user wrote in the app, then
//! LRCLIB when its setting is on (the exact lookup, then every title search
//! hit), then a transcription made on this computer. The user's own lyrics
//! go first instead when they asked for that (for the song, or for every
//! song in Settings). See [`candidates`] for the rule that picks one, and
//! [`checker`] for the check against the recording. LRCLIB answers are
//! cached in SQLite (found: 30 days, none: 7 days); the server's aren't (it
//! keeps its own).
//!
//! Layout:
//! - [`lrclib`]     the LRCLIB requests and match rules (pure)
//! - [`cache`]      the `lyrics_cache` table
//! - [`user`]       the `user_lyrics` table (the lyrics editor's saves)
//! - [`live`]       whether a track is a live version (borrowed timing)
//! - [`candidates`] the candidates and which one shows (pure)
//! - [`check`]      comparing heard words with lyrics, offsets, drift (pure)
//! - [`checker`]    the check's worker and its tables
//! - [`choice`]     the user's pick and timing offset per song
//! - [`pronounce`] romanised lines under lyrics (local)
//! - [`publish`]   sharing to LRCLIB (challenge, proof of work, publish)
//! - [`share`]     when sharing a transcription is offered, and its commands
//! - [`translate`] translation through the service the user set
//! - [`commands`]   the Tauri commands and the Plex side
//!
//! Any source being down is silent: the view says there are no lyrics.

pub mod cache;
pub mod candidates;
pub mod check;
pub mod checker;
pub mod choice;
pub mod commands;
pub mod live;
pub mod lrclib;
pub mod pronounce;
pub mod publish;
pub mod share;
pub mod transcribe;
pub mod translate;
pub mod user;

#[cfg(test)]
mod share_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod trust_tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::RwLock;
use rusqlite::{Connection, OptionalExtension, params};

use crate::artistinfo::http::InfoHttp;
use crate::artistinfo::limit::Limiter;
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::{AppError, AppResult};
use crate::plex::details::{Lyrics, LyricsSource, parse_lyrics_text};
use crate::share::matching::exact_key;
use candidates::{Candidate, Decision, Note, Notice, Origin, SongFacts, Stored};
use check::Verdict;
use checker::{AudioCheck, CheckJob, JobCand};
use choice::Choice;
use lrclib::Query;

/// LRCLIB has no published limit; stay gentle.
pub const LRCLIB_GAP: Duration = Duration::from_millis(500);

pub const CREDIT: &str = "Lyrics from LRCLIB";
pub const CREDIT_MATCHED: &str = "Lyrics from LRCLIB, matched by title";

/// The server side: a track's lyrics from its server, if it has some.
#[async_trait]
pub trait ServerLyrics: Send + Sync {
    async fn lyrics(&self, track_id: &str) -> AppResult<Option<Lyrics>>;
}

/// The name the server side had when Plex was the only server.
pub use ServerLyrics as PlexLyrics;

/// What LRCLIB is asked about a library track: the track artist (or the
/// album artist), title, album and length.
fn track_query(c: &Connection, id: &str) -> AppResult<Option<Query>> {
    Ok(c.query_row(
        "SELECT title, album_title, artist_title, track_artist, duration_ms FROM tracks WHERE id = ?1",
        params![id],
        |r| {
            let album_artist: String = r.get(2)?;
            let track_artist: Option<String> = r.get(3)?;
            Ok(Query {
                title: r.get(0)?,
                album: r.get(1)?,
                artist: track_artist.filter(|a| !a.trim().is_empty()).unwrap_or(album_artist),
                duration_ms: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        },
    )
    .optional()?)
}

fn get_key(q: &Query) -> String {
    format!("lrclib2|get|{}|{}|{}|{}", exact_key(&q.artist), exact_key(&q.title), exact_key(&q.album), q.seconds())
}

fn search_key(q: &Query) -> String {
    format!("lrclib2|search|{}|{}", exact_key(&q.artist), exact_key(&q.title))
}

/// Cached or fetched LRCLIB text to lyrics.
fn to_lyrics(body: &str, matched: bool) -> Option<Lyrics> {
    let mut l = parse_lyrics_text(body);
    if l.lines.is_empty() {
        return None;
    }
    l.source = LyricsSource::Lrclib;
    l.credit = Some(if matched { CREDIT_MATCHED } else { CREDIT }.to_owned());
    Some(l)
}

/// Sources found by artist and title rather than read from this file: their
/// timing belongs to whichever version they were written against.
pub fn borrows_timing(source: LyricsSource) -> bool {
    matches!(source, LyricsSource::Plex | LyricsSource::Lrclib)
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

/// `lyrics_get`'s answer: the lyrics, whether their timing comes from
/// another version of the song (a live take shown with studio lyrics), and
/// what the check against the recording said. With a `notice` the lyrics
/// are empty: the view says why instead.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    #[serde(flatten)]
    pub lyrics: Lyrics,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub borrowed_timing: bool,
    /// Checked, not checked, or being checked against the recording.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<Note>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<Notice>,
    /// The candidate shown (or held back, with `notice`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pick: Option<String>,
    /// The user picked these for the song.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub chosen: bool,
    /// The user's timing nudge, already applied.
    #[serde(skip_serializing_if = "is_zero")]
    pub offset_ms: i32,
    /// The measured shift where the lyrics start, already applied.
    #[serde(skip_serializing_if = "is_zero")]
    pub auto_offset_ms: i32,
    /// The timing was stretched to follow drift.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub retimed: bool,
    /// Other lyrics found for the song say something quite different.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disagree: bool,
}

impl Shown {
    fn notice(n: Notice, pick: Option<String>) -> Self {
        Self { notice: Some(n), pick, ..Default::default() }
    }

    fn has_words(&self) -> bool {
        self.notice.is_none()
    }
}

/// One candidate as the "Wrong lyrics?" picker lists it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateInfo {
    pub key: String,
    pub origin: Origin,
    pub source: LyricsSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    pub synced: bool,
    /// What the check against the recording said, if it ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<Verdict>,
    /// Found for the released version while this song is another version.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub other_version: bool,
    /// The first few lines.
    pub preview: Vec<String>,
    /// Shown now.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shown: bool,
}

/// `lyrics_candidates`: everything found for a song, and the user's choice.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateList {
    pub items: Vec<CandidateInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pick: Option<String>,
    pub offset_ms: i32,
    /// The check against the recording can run.
    pub can_check: bool,
}

/// Lyrics from the song's server, the user, LRCLIB and transcription,
/// checked against the recording when it can be. Cheap to clone.
#[derive(Clone)]
pub struct LyricsService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DbHandle,
    http: Arc<dyn InfoHttp>,
    plex: Arc<dyn PlexLyrics>,
    limiter: Limiter,
    sw: Switches,
    /// The check against the recording, once it exists.
    checker: RwLock<Option<Arc<dyn AudioCheck>>>,
}

/// The switches lyrics read from Settings.
#[derive(Clone)]
pub struct Switches {
    /// "Lyrics from LRCLIB when Plex has none".
    pub lrclib: Arc<AtomicBool>,
    /// "Prefer my own lyrics over Plex's".
    pub prefer_own: Arc<AtomicBool>,
    /// "Show auto-transcribed lyrics".
    pub show_auto: Arc<AtomicBool>,
    /// "Show borrowed timing on live versions".
    pub live_timing: Arc<AtomicBool>,
    /// "Fix lyrics timing automatically".
    pub fix_timing: Arc<AtomicBool>,
    /// "Use released lyrics for other versions".
    pub released_ok: Arc<AtomicBool>,
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            lrclib: Default::default(),
            prefer_own: Default::default(),
            show_auto: Arc::new(AtomicBool::new(true)),
            live_timing: Default::default(),
            fix_timing: Arc::new(AtomicBool::new(true)),
            released_ok: Default::default(),
        }
    }
}

fn on(b: &AtomicBool) -> bool {
    b.load(Ordering::Relaxed)
}

/// Everything found for a song and what to show.
struct Resolved {
    track_id: String,
    cands: Vec<Candidate>,
    verdicts: Vec<(String, String, Stored)>,
    facts: SongFacts,
    choice: Choice,
    shown: Option<Shown>,
}

fn verdict_in<'a>(v: &'a [(String, String, Stored)], c: &Candidate) -> Option<&'a Stored> {
    v.iter().find(|(k, h, _)| *k == c.key && *h == c.hash).map(|(_, _, s)| s)
}

impl LyricsService {
    pub fn new(db: DbHandle, http: Arc<dyn InfoHttp>, plex: Arc<dyn PlexLyrics>, switches: Switches) -> Self {
        Self::with_gap(db, http, plex, switches, LRCLIB_GAP)
    }

    pub fn with_gap(db: DbHandle, http: Arc<dyn InfoHttp>, plex: Arc<dyn PlexLyrics>, switches: Switches, gap: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                http,
                plex,
                limiter: Limiter::with_gap(gap),
                sw: switches,
                checker: RwLock::new(None),
            }),
        }
    }

    /// Connect the check against the recording.
    pub fn set_checker(&self, c: Arc<dyn AudioCheck>) {
        *self.inner.checker.write() = Some(c);
    }

    /// The check, when it can run now.
    fn checker(&self) -> Option<Arc<dyn AudioCheck>> {
        self.inner.checker.read().clone().filter(|c| c.available())
    }

    /// The track's lyrics, or None when there are none to show (or a source
    /// is down: that's silent).
    pub async fn get(&self, track_id: &str) -> AppResult<Option<Lyrics>> {
        Ok(self.shown(track_id).await?.filter(Shown::has_words).map(|s| s.lyrics))
    }

    /// The lyrics as the lyrics view shows them (docs/IPC.md `lyrics_get`).
    ///
    /// The user's pick for the song wins. Otherwise [`candidates::decide`]
    /// picks; a candidate the check hasn't judged yet is sent to it. Shown
    /// lyrics the check matched are moved by the measured offset (and
    /// stretched for drift) when "Fix lyrics timing automatically" is on.
    /// Unchecked LRCLIB timing is kept only within the stricter length
    /// rule. On a live version, unchecked lyrics found by artist and title
    /// carry another version's timing: they come back unsynced and marked
    /// `borrowed_timing`, unless "Show borrowed timing on live versions" is
    /// on (then they keep their times, still marked). The user's timing
    /// nudge applies last.
    pub async fn shown(&self, track_id: &str) -> AppResult<Option<Shown>> {
        Ok(self.resolve(track_id, false).await?.shown)
    }

    /// Everything found for the song ("Wrong lyrics?"): every candidate,
    /// its check result, and the user's choice.
    pub async fn candidates(&self, track_id: &str) -> AppResult<CandidateList> {
        let r = self.resolve(track_id, true).await?;
        let shown = r.shown.as_ref().and_then(|s| s.pick.clone().filter(|_| s.has_words()));
        let items = r
            .cands
            .iter()
            .map(|c| {
                let lines: Vec<String> = if c.lyrics.synced && !c.lyrics.timed.is_empty() {
                    c.lyrics.timed.iter().map(|t| t.text.clone()).collect()
                } else {
                    c.lyrics.lines.clone()
                };
                CandidateInfo {
                    key: c.key.clone(),
                    origin: c.origin,
                    source: c.lyrics.source,
                    album: c.album.clone(),
                    duration_ms: c.duration_ms,
                    synced: c.lyrics.synced,
                    check: verdict_in(&r.verdicts, c).map(|v| v.verdict),
                    other_version: c.other_version,
                    preview: lines.into_iter().filter(|l| !l.trim().is_empty()).take(3).collect(),
                    shown: shown.as_deref() == Some(c.key.as_str()),
                }
            })
            .collect();
        Ok(CandidateList { items, pick: r.choice.pick, offset_ms: r.choice.offset_ms, can_check: self.checker().is_some() })
    }

    async fn resolve(&self, track_id: &str, full: bool) -> AppResult<Resolved> {
        let sw = &self.inner.sw;
        let id = track_id.to_owned();
        let (facts, choice, verdicts) = self
            .inner
            .db
            .read_async(move |c| {
                let facts = candidates::song_facts(c, &id)?;
                let choice = if user::valid_track_id(&id) { choice::get(c, &id)? } else { None };
                Ok((facts, choice.unwrap_or_default(), checker::stored_all(c, &id, now_s())?))
            })
            .await?;
        let facts = facts.unwrap_or_default();
        let mut out = Resolved { track_id: track_id.to_owned(), cands: Vec::new(), verdicts, facts, choice, shown: None };
        if out.choice.pick.as_deref() == Some(choice::NONE) && !full {
            out.shown = Some(Shown::notice(Notice::ChosenNone, None));
            return Ok(out);
        }
        let checker = self.checker();
        let picked = out.choice.pick.is_some();
        // Without the check (and without a pick), stop at the first usable
        // stage: LRCLIB isn't asked when the server has lyrics.
        let lazy = !full && !picked && checker.is_none();
        let released_ok = on(&sw.released_ok);

        // The user's own, the server's, and a transcription (kept for last).
        let own = self.own(track_id).await?;
        let auto = own
            .as_ref()
            .filter(|o| o.auto.is_some() && on(&sw.show_auto))
            .and_then(|o| o.lyrics())
            .map(|l| Candidate::new("auto", Origin::Auto, l));
        let mine = own.as_ref().filter(|o| o.auto.is_none());
        let prefer = mine.is_some_and(|o| o.prefer || on(&sw.prefer_own));
        let mine = mine.and_then(|o| o.lyrics()).map(|l| Candidate::new("user", Origin::User, l));
        if prefer && let Some(m) = &mine {
            out.cands.push(m.clone());
        }
        // The user's own lyrics first: the server isn't asked.
        let skip_server = lazy && prefer && mine.is_some();
        if !skip_server {
            match self.inner.plex.lyrics(track_id).await {
                Ok(Some(l)) => {
                    let mut c = Candidate::new("server", Origin::Server, l);
                    c.other_version = out.facts.other_version && c.lyrics.source == LyricsSource::Plex;
                    out.cands.push(c);
                }
                Ok(None) => {}
                Err(e) if e.kind() == "invalid" => return Err(e),
                Err(e) => tracing::debug!(kind = e.kind(), "server lyrics unavailable"),
            }
        }
        if !prefer && let Some(m) = mine {
            out.cands.push(m);
        }
        let settled = |cands: &[Candidate], verdicts: &[(String, String, Stored)]| {
            matches!(candidates::decide(cands, |c| verdict_in(verdicts, c), false, released_ok), Decision::Show { .. })
        };
        if !(lazy && settled(&out.cands, &out.verdicts)) && on(&sw.lrclib) {
            self.add_lrclib(track_id, &mut out, lazy, settled).await?;
        }
        if let Some(a) = auto {
            out.cands.push(a);
        }
        out.shown = self.pick(&out, checker.as_deref());
        Ok(out)
    }

    /// LRCLIB's candidates: the exact lookup, then (unless that settled it)
    /// every search hit not already found.
    async fn add_lrclib(
        &self,
        track_id: &str,
        out: &mut Resolved,
        lazy: bool,
        settled: impl Fn(&[Candidate], &[(String, String, Stored)]) -> bool,
    ) -> AppResult<()> {
        let id = track_id.to_owned();
        let Some(q) = self.inner.db.read_async(move |c| track_query(c, &id)).await? else {
            return Ok(());
        };
        if q.title.trim().is_empty() || q.artist.trim().is_empty() {
            return Ok(());
        }
        let Some(exact) = self.lrclib_exact(&q).await? else {
            // LRCLIB couldn't be asked: silent.
            return Ok(());
        };
        if let Some(r) = exact {
            self.push_record(out, &q, r, Origin::LrclibExact);
        }
        if lazy && settled(&out.cands, &out.verdicts) {
            return Ok(());
        }
        for r in self.lrclib_search(&q).await?.unwrap_or_default() {
            self.push_record(out, &q, r, Origin::LrclibSearch);
        }
        Ok(())
    }

    fn push_record(&self, out: &mut Resolved, q: &Query, r: lrclib::Record, origin: Origin) {
        let Some(text) = r.text(true) else { return };
        let Some(l) = to_lyrics(&text, origin == Origin::LrclibSearch) else { return };
        let hash = check::lyrics_hash(&l);
        if out.cands.iter().any(|c| c.hash == hash) {
            return;
        }
        let mut c = Candidate::new(candidates::lrclib_key(&hash), origin, l);
        let rec_album = r.album_name.clone().unwrap_or_default();
        c.sync_ok = candidates::lrclib_sync_ok(q.duration_ms, &q.album, r.duration, Some(&rec_album));
        c.other_version = out.facts.other_version && !candidates::album_matches(&q.album, &rec_album);
        c.album = r.album_name;
        c.duration_ms = r.duration.map(|d| (d * 1000.0).round() as i64);
        out.cands.push(c);
    }

    /// What to show from what was found (and ask the check about the rest).
    fn pick(&self, r: &Resolved, checker: Option<&dyn AudioCheck>) -> Option<Shown> {
        let sw = &self.inner.sw;
        let verdict = |c: &Candidate| verdict_in(&r.verdicts, c);
        // The user's pick, when it's still there.
        if let Some(p) = r.choice.pick.as_deref()
            && let Some(idx) = r.cands.iter().position(|c| c.key == p)
        {
            let note = verdict(&r.cands[idx]).filter(|v| v.verdict.matched()).map(|_| Note::Checked);
            let mut s = self.build(r, idx, note, true);
            s.chosen = true;
            return Some(s);
        }
        let released_ok = on(&sw.released_ok);
        let decision = candidates::decide(&r.cands, verdict, checker.is_some(), released_ok);
        if let Some(ch) = checker {
            self.ask_check(r, ch);
        }
        match decision {
            Decision::Show { idx, note } => {
                let mut s = self.build(r, idx, note, false);
                if note == Some(Note::NotChecked) {
                    s.disagree = candidates::disagrees(&r.cands, idx, verdict);
                }
                Some(s)
            }
            Decision::Held { idx } => Some(Shown::notice(Notice::VersionHeld, Some(r.cands[idx].key.clone()))),
            Decision::Notice(n) => Some(Shown::notice(n, None)),
            Decision::Nothing => None,
        }
    }

    /// Send the candidates the check hasn't judged (before the one that
    /// already won) to the check.
    fn ask_check(&self, r: &Resolved, ch: &dyn AudioCheck) {
        let verdict = |c: &Candidate| verdict_in(&r.verdicts, c);
        let winner = r.cands.iter().position(|c| c.trusted() || verdict(c).is_some_and(|v| v.verdict.matched()));
        let todo: Vec<JobCand> = r.cands[..winner.unwrap_or(r.cands.len())]
            .iter()
            .filter(|c| !c.trusted() && verdict(c).is_none())
            .map(|c| JobCand { key: c.key.clone(), hash: c.hash.clone(), lyrics: c.lyrics.clone() })
            .collect();
        if todo.is_empty() || r.facts.duration_ms <= 0 {
            return;
        }
        ch.request(CheckJob {
            track_id: r.track_id.clone(),
            duration_ms: r.facts.duration_ms.clamp(0, u32::MAX as i64) as u32,
            cands: todo,
            fix_timing: on(&self.inner.sw.fix_timing),
        });
    }

    /// The shown lyrics for a candidate: timing fixed, unsynced or marked
    /// borrowed as the rules say, then the user's nudge.
    fn build(&self, r: &Resolved, idx: usize, note: Option<Note>, chosen: bool) -> Shown {
        let sw = &self.inner.sw;
        let mut c = &r.cands[idx];
        let mut note = note;
        let v = verdict_in(&r.verdicts, c).filter(|v| v.verdict.matched());
        let mut s = Shown { check: note, pick: Some(c.key.clone()), ..Default::default() };
        let mut l = c.lyrics.clone();
        if let Some(v) = v {
            // Matched against this recording: its timing is this recording's.
            if l.synced && on(&sw.fix_timing) && !v.points.is_empty() {
                check::retime(&mut l, &v.points, 0);
                s.auto_offset_ms = v.points[0].offset_ms;
                s.retimed = v.points.len() > 1;
            }
        } else {
            if !chosen && matches!(c.origin, Origin::LrclibExact | Origin::LrclibSearch) && !c.sync_ok {
                candidates::unsync(&mut l);
            }
            if l.synced && c.borrows_timing() && r.facts.live {
                if !on(&sw.live_timing) {
                    // A transcription of this very recording beats words
                    // timed to another version.
                    if !chosen
                        && let Some(a) = r.cands.iter().find(|a| a.origin == Origin::Auto && a.lyrics.synced)
                    {
                        c = a;
                        note = None;
                        l = a.lyrics.clone();
                        s = Shown { check: note, pick: Some(c.key.clone()), ..Default::default() };
                    } else {
                        candidates::unsync(&mut l);
                        s.borrowed_timing = true;
                    }
                } else {
                    s.borrowed_timing = true;
                }
            }
        }
        if l.synced && r.choice.offset_ms != 0 {
            check::retime(&mut l, &[], r.choice.offset_ms);
            s.offset_ms = r.choice.offset_ms;
        }
        s.lyrics = l;
        s
    }

    /// Whether a source other than transcription has lyrics for the track
    /// (asked before transcribing an album or the background batch).
    pub async fn has_other(&self, track_id: &str) -> bool {
        if let Ok(Some(o)) = self.own(track_id).await {
            return o.auto.is_none();
        }
        // No row of its own, so whatever get finds is the server's or LRCLIB's.
        matches!(self.get(track_id).await, Ok(Some(_)))
    }

    /// The track's own lyrics, as saved by the editor.
    pub async fn own(&self, track_id: &str) -> AppResult<Option<user::OwnLyrics>> {
        if !user::valid_track_id(track_id) {
            return Ok(None);
        }
        let id = track_id.to_owned();
        self.inner.db.read_async(move |c| user::get(c, &id)).await
    }

    /// Save the track's own lyrics (LRC or plain lines).
    pub async fn save_own(&self, track_id: &str, text: &str, prefer: bool) -> AppResult<()> {
        if !user::valid_track_id(track_id) {
            return Err(AppError::Invalid("track id".into()));
        }
        let text = user::clean(text)?;
        let id = track_id.to_owned();
        let now = now_s();
        self.inner.db.write_async(move |c| user::put(c, &id, &text, prefer, now)).await
    }

    /// Remove the track's own lyrics. True when there were some.
    pub async fn delete_own(&self, track_id: &str) -> AppResult<bool> {
        if !user::valid_track_id(track_id) {
            return Ok(false);
        }
        let id = track_id.to_owned();
        self.inner.db.write_async(move |c| user::delete(c, &id)).await
    }

    /// Every track with its own lyrics.
    pub async fn own_ids(&self) -> AppResult<Vec<String>> {
        self.inner.db.read_async(user::ids).await
    }

    // ------------------------------------------------------------ choices

    /// The user's choice for a song.
    pub async fn choice(&self, track_id: &str) -> AppResult<Choice> {
        if !user::valid_track_id(track_id) {
            return Ok(Choice::default());
        }
        let id = track_id.to_owned();
        Ok(self.inner.db.read_async(move |c| choice::get(c, &id)).await?.unwrap_or_default())
    }

    /// Set a song's choice. Returns it as stored (for the sync file).
    pub async fn set_choice(&self, track_id: &str, next: Choice) -> AppResult<Choice> {
        if !user::valid_track_id(track_id) {
            return Err(AppError::Invalid("track id".into()));
        }
        choice::check(&next)?;
        let (id, now, c2) = (track_id.to_owned(), now_s(), next.clone());
        self.inner.db.write_async(move |c| choice::put(c, &id, &c2, now)).await?;
        Ok(next)
    }

    /// Pick lyrics for a song: a candidate key, "none", or None for
    /// automatic. A new pick starts with no timing offset.
    pub async fn choose(&self, track_id: &str, pick: Option<String>) -> AppResult<Choice> {
        let cur = self.choice(track_id).await?;
        let offset_ms = if cur.pick == pick { cur.offset_ms } else { 0 };
        self.set_choice(track_id, Choice { pick, offset_ms }).await
    }

    /// The song's timing offset (the nudge buttons, "Tap to sync").
    pub async fn set_offset(&self, track_id: &str, offset_ms: i32) -> AppResult<Choice> {
        let cur = self.choice(track_id).await?;
        self.set_choice(track_id, Choice { pick: cur.pick, offset_ms: check::round_100(offset_ms as i64) }).await
    }

    /// How many songs have a choice.
    pub async fn choice_count(&self) -> AppResult<usize> {
        Ok(self.inner.db.read_async(choice::all).await?.len())
    }

    /// Forget every choice; the songs that had one.
    pub async fn clear_choices(&self) -> AppResult<Vec<String>> {
        self.inner.db.write_async(|c| choice::clear(c)).await
    }

    // ------------------------------------------------------------ LRCLIB

    /// A cached LRCLIB reply, or None when there's none fresh.
    async fn cached(&self, key: &str) -> AppResult<Option<Option<String>>> {
        let (k, now) = (key.to_owned(), now_s());
        Ok(self
            .inner
            .db
            .read_async(move |c| cache::get(c, &k))
            .await?
            .filter(|row| row.expires_at > now)
            .map(|row| row.body))
    }

    async fn remember(&self, key: &str, body: Option<String>) -> AppResult<()> {
        let (key, now) = (key.to_owned(), now_s());
        self.inner
            .db
            .write_async(move |c| {
                cache::prune(c, now)?;
                cache::put(c, &key, body.as_deref(), false, now)
            })
            .await
    }

    /// The exact lookup: Some(record or none found), or None when LRCLIB
    /// couldn't be asked.
    async fn lrclib_exact(&self, q: &Query) -> AppResult<Option<Option<lrclib::Record>>> {
        let key = get_key(q);
        if let Some(body) = self.cached(&key).await? {
            return Ok(Some(body.and_then(|b| lrclib::parse_get(b.as_bytes()).ok().flatten())));
        }
        let resp = match self.inner.limiter.get(self.inner.http.as_ref(), &lrclib::get_url(q), &[]).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(kind = e.kind(), "lyrics lookup failed");
                return Ok(None);
            }
        };
        let rec = match resp.status {
            200 => match lrclib::parse_get(&resp.body) {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!(kind = e.kind(), "lyrics reply unreadable");
                    return Ok(None);
                }
            },
            404 => None,
            s => {
                tracing::debug!(status = s, "lyrics lookup answered with an error");
                return Ok(None);
            }
        };
        let body = rec.as_ref().and_then(|r| serde_json::to_string(r).ok());
        self.remember(&key, body).await?;
        Ok(Some(rec))
    }

    /// The title search: Some(hits), or None when LRCLIB couldn't be asked.
    async fn lrclib_search(&self, q: &Query) -> AppResult<Option<Vec<lrclib::Record>>> {
        let key = search_key(q);
        if let Some(body) = self.cached(&key).await? {
            return Ok(Some(body.and_then(|b| lrclib::search_hits(q, b.as_bytes()).ok()).unwrap_or_default()));
        }
        let resp = match self.inner.limiter.get(self.inner.http.as_ref(), &lrclib::search_url(q), &[]).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(kind = e.kind(), "lyrics search failed");
                return Ok(None);
            }
        };
        if !resp.ok() {
            tracing::debug!(status = resp.status, "lyrics search answered with an error");
            return Ok(None);
        }
        let hits = match lrclib::search_hits(q, &resp.body) {
            Ok(h) => h,
            Err(e) => {
                tracing::debug!(kind = e.kind(), "lyrics search reply unreadable");
                return Ok(None);
            }
        };
        let body = (!hits.is_empty()).then(|| serde_json::to_string(&hits).ok()).flatten();
        self.remember(&key, body).await?;
        Ok(Some(hits))
    }
}
