//! The play queue, pure logic.
//!
//! * A **context** (album, playlist, artist, tracks, search) supplies
//!   "Next up": the rest of the context after the current position.
//! * The **user queue** ("Play next" inserts at its front, "Add to queue"
//!   appends) always plays before the context continues.
//! * **Shuffle** reorders the context; the current track stays first. The
//!   Spread style keeps an artist's tracks apart and pushes recently played
//!   tracks back; Standard is a plain random order. Turning it off restores
//!   context order from the current track. Shuffle is remembered per context
//!   id; repeat is one global setting.
//! * **Repeat** off / all / one. Next, or an early Previous, drops repeat-one
//!   to repeat-all.
//! * **Previous** steps back through the context's play order (`order` /
//!   `pos`), never to a user-queue entry. `history` is only for display.
//! * **Resume points**: each context id remembers its last track and
//!   position, so playing it again without a chosen track picks up there.
//!
//! Every entry has a stable `itemId`, so the UI can move/remove/jump to a
//! specific entry even when the same track appears twice. The whole queue is
//! serializable for persistence.

use std::collections::{HashMap, HashSet, VecDeque};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};
use serde::{Deserialize, Serialize};

use super::source::ContextKind;
use crate::error::{AppError, AppResult};
use crate::model::Track;

const HISTORY_CAP: usize = 100;
const RECENT_CAP: usize = 50;
/// Contexts that keep a resume point / shuffle preference (least recently
/// used dropped first).
pub const CONTEXT_MEMORY_CAP: usize = 200;

/// How shuffle orders a context (the `shuffleStyle` setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShuffleStyle {
    /// Keeps an artist's tracks apart and pushes recent plays back.
    #[default]
    Spread,
    /// Plain random order (Fisher-Yates).
    Standard,
}

/// Where a context was left: an index into its tracks, checked against the
/// track id so a changed playlist doesn't resume on the wrong song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumePoint {
    pub index: usize,
    pub track_id: String,
    pub position_ms: u64,
    /// When this context was last listened to (Unix ms). Points saved
    /// before this was kept have none and count as expired.
    #[serde(default)]
    pub listened_at_ms: Option<i64>,
}

/// A resume point older than this starts its context over.
pub const RESUME_EXPIRY_MS: i64 = 60 * 60 * 1000;

/// Context kinds that keep a resume point: albums and playlists (Liked Songs
/// and mixes are playlists too). Artist play always starts at its first top
/// song; radio, search results and loose tracks start where asked.
pub fn resumable(kind: ContextKind) -> bool {
    matches!(kind, ContextKind::Album | ContextKind::Playlist | ContextKind::Liked | ContextKind::Mix)
}

fn system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemSource {
    User,
    Context,
    /// Similar music added after the context ran out (keep playing).
    Autoplay,
    /// A similar song smart shuffle put in among the context's songs.
    Smart,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub item_id: String,
    pub track: Track,
    pub source: ItemSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextInfo {
    pub kind: ContextKind,
    pub id: Option<String>,
    pub title: Option<String>,
}

/// `QueueView` in docs/IPC.md.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueView {
    pub current: Option<QueueItem>,
    pub user_queue: Vec<QueueItem>,
    pub next_up: Vec<QueueItem>,
    /// Most recent first.
    pub history: Vec<QueueItem>,
    pub context: Option<ContextInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AddAt {
    Next,
    End,
    /// At this position in the user queue (0 is first; past the end appends).
    Index(usize),
}

/// RNG that isn't persisted; reseeded from entropy after a restore.
struct QRng(StdRng);

impl Default for QRng {
    fn default() -> Self {
        QRng(StdRng::seed_from_u64(rand::rng().random::<u64>()))
    }
}

impl std::fmt::Debug for QRng {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QRng")
    }
}

impl Clone for QRng {
    /// A clone gets a fresh entropy seed (`StdRng` isn't `Clone`).
    fn clone(&self) -> Self {
        QRng::default()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    context: Option<ContextInfo>,
    /// Context entries in original (context) order.
    items: Vec<QueueItem>,
    /// Play order: indices into `items`.
    order: Vec<usize>,
    /// Index into `order` of the last context entry made current
    /// (-1 = none yet).
    pos: isize,
    user: VecDeque<QueueItem>,
    current: Option<QueueItem>,
    /// Oldest first.
    history: VecDeque<QueueItem>,
    shuffle: bool,
    repeat: Repeat,
    next_id: u64,
    /// Recently played track ids, newest last (shuffle avoids them).
    recent: VecDeque<String>,
    /// Resume point per context id, least recently used first.
    #[serde(default)]
    resume: VecDeque<(String, ResumePoint)>,
    /// Context ids with shuffle on, least recently used first.
    #[serde(default)]
    shuffled: VecDeque<String>,
    /// Context ids (playlists, Liked Songs) with smart shuffle on, least
    /// recently used first. Always a subset of `shuffled`.
    #[serde(default)]
    smart: VecDeque<String>,
    /// Comes from settings, so it isn't saved with the queue.
    #[serde(skip)]
    shuffle_style: ShuffleStyle,
    /// Settings turned resuming off: contexts start at track 1. Points are
    /// still recorded, so turning it back on picks up where things were.
    #[serde(skip)]
    ignore_resume: bool,
    /// Fixed clock for tests (Unix ms); None reads the system clock.
    #[serde(skip)]
    clock: Option<i64>,
    #[serde(skip)]
    rng: QRng,
}

impl Queue {
    pub fn new() -> Self {
        Self {
            pos: -1,
            ..Default::default()
        }
    }

    /// Deterministic shuffles for tests.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            pos: -1,
            rng: QRng(StdRng::seed_from_u64(seed)),
            ..Default::default()
        }
    }

    pub fn reseed(&mut self, seed: u64) {
        self.rng = QRng(StdRng::seed_from_u64(seed));
    }

    fn new_id(&mut self) -> String {
        self.next_id += 1;
        format!("q{}", self.next_id)
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }
    pub fn repeat(&self) -> Repeat {
        self.repeat
    }
    pub fn current(&self) -> Option<&QueueItem> {
        self.current.as_ref()
    }
    pub fn context(&self) -> Option<&ContextInfo> {
        self.context.as_ref()
    }
    pub fn is_empty(&self) -> bool {
        self.current.is_none() && self.user.is_empty() && self.items.is_empty()
    }
    pub fn shuffle_style(&self) -> ShuffleStyle {
        self.shuffle_style
    }
    pub fn set_shuffle_style(&mut self, style: ShuffleStyle) {
        self.shuffle_style = style;
    }
    /// Whether contexts resume where they were left (the setting).
    pub fn resume_contexts(&self) -> bool {
        !self.ignore_resume
    }
    pub fn set_resume_contexts(&mut self, on: bool) {
        self.ignore_resume = !on;
    }

    /// Sets a fixed clock (Unix ms) for tests; None goes back to the system clock.
    pub fn set_clock(&mut self, now_ms: Option<i64>) {
        self.clock = now_ms;
    }

    fn now_ms(&self) -> i64 {
        self.clock.unwrap_or_else(system_now_ms)
    }

    fn context_id(&self) -> Option<&str> {
        self.context.as_ref()?.id.as_deref()
    }

    /// The playing context's id, if it keeps a resume point.
    fn resumable_id(&self) -> Option<&str> {
        let c = self.context.as_ref()?;
        if resumable(c.kind) { c.id.as_deref() } else { None }
    }

    fn order_at(&self, p: isize) -> Option<usize> {
        if p < 0 {
            return None;
        }
        self.order.get(p as usize).copied()
    }

    // ------------------------------------------------- per-context memory

    /// The stored shuffle preference for a context (off unless set).
    pub fn shuffle_pref(&self, id: &str) -> bool {
        self.shuffled.iter().any(|s| s == id)
    }

    /// Context ids with shuffle on, least recently used first.
    pub fn shuffled_contexts(&self) -> Vec<String> {
        self.shuffled.iter().cloned().collect()
    }

    /// Queues saved before shuffle was per context only had one flag. If a
    /// loaded queue is shuffled, remember that for its context so the page
    /// toggle and the player agree.
    pub fn adopt_legacy_shuffle(&mut self) {
        if !self.shuffle {
            return;
        }
        if let Some(id) = self.context.as_ref().and_then(|c| c.id.clone())
            && !self.shuffle_pref(&id)
        {
            self.store_shuffle_pref(&id, true);
        }
    }

    fn store_shuffle_pref(&mut self, id: &str, on: bool) {
        self.shuffled.retain(|s| s != id);
        if on {
            self.shuffled.push_back(id.to_owned());
            while self.shuffled.len() > CONTEXT_MEMORY_CAP {
                self.shuffled.pop_front();
            }
        } else {
            self.smart.retain(|s| s != id);
        }
    }

    /// The stored smart shuffle preference for a context (off unless set).
    pub fn smart_pref(&self, id: &str) -> bool {
        self.smart.iter().any(|s| s == id)
    }

    /// Context ids with smart shuffle on, least recently used first.
    pub fn smart_contexts(&self) -> Vec<String> {
        self.smart.iter().cloned().collect()
    }

    /// Remembers smart shuffle for a context. On also turns its shuffle
    /// preference on; off leaves shuffle as it is.
    pub fn store_smart_pref(&mut self, id: &str, on: bool) {
        self.smart.retain(|s| s != id);
        if on {
            if !self.shuffle_pref(id) {
                self.store_shuffle_pref(id, true);
            }
            self.smart.push_back(id.to_owned());
            while self.smart.len() > CONTEXT_MEMORY_CAP {
                self.smart.pop_front();
            }
        }
    }

    /// Shuffle off for the playing context without going back to context
    /// order: what's left plays in the order it was shuffled into.
    pub fn shuffle_off_keep_order(&mut self) {
        if let Some(id) = self.context_id().map(str::to_owned) {
            self.store_shuffle_pref(&id, false);
        }
        self.shuffle = false;
    }

    /// Sets shuffle for a context. For the playing context it applies now;
    /// for any other it's remembered for the next time it plays.
    pub fn set_context_shuffle(&mut self, id: &str, on: bool) {
        if self.context_id() == Some(id) {
            self.set_shuffle(on);
        } else {
            self.store_shuffle_pref(id, on);
        }
    }

    pub fn resume_point(&self, id: &str) -> Option<&ResumePoint> {
        self.resume.iter().find(|(k, _)| k == id).map(|(_, r)| r)
    }

    fn store_resume(&mut self, id: String, rp: ResumePoint) {
        self.resume.retain(|(k, _)| *k != id);
        self.resume.push_back((id, rp));
        while self.resume.len() > CONTEXT_MEMORY_CAP {
            self.resume.pop_front();
        }
    }

    fn clear_resume(&mut self, id: &str) {
        self.resume.retain(|(k, _)| k != id);
    }

    /// Records where the current context is (call on track change, pause
    /// and position saves). While a user-queue entry plays, the context
    /// would pick up at its next track. Only albums and playlists keep a
    /// point, and only while shuffle is off: shuffled listening leaves the
    /// unshuffled point as it was. Songs added after the context (autoplay)
    /// don't count as being in it.
    ///
    /// The point's time moves only when the point does, so a context left
    /// paused keeps the time it was paused at and still expires.
    pub fn note_position(&mut self, position_ms: u64) {
        if self.shuffle {
            return;
        }
        let Some(id) = self.resumable_id().map(str::to_owned) else {
            return;
        };
        let Some(cur) = self.current.as_ref() else {
            return;
        };
        let (index, position_ms) = if cur.source == ItemSource::Context {
            match self.order_at(self.pos) {
                Some(i) if self.items[i].item_id == cur.item_id => (i, position_ms),
                _ => return,
            }
        } else {
            let from = (self.pos + 1).max(0) as usize;
            let rest = self.order.get(from..).unwrap_or_default();
            match rest.iter().copied().find(|&i| self.items[i].source == ItemSource::Context) {
                Some(i) => (i, 0),
                None => return,
            }
        };
        let track_id = self.items[index].track.id.clone();
        let same = self.resume_point(&id).filter(|rp| {
            rp.index == index && rp.track_id == track_id && rp.position_ms == position_ms
        });
        let listened_at_ms = match same {
            Some(rp) => rp.listened_at_ms,
            None => Some(self.now_ms()),
        };
        self.store_resume(
            id,
            ResumePoint {
                index,
                track_id,
                position_ms,
                listened_at_ms,
            },
        );
    }

    /// The last song of the context finished on its own (repeat and
    /// shuffle off): the next play of it starts over.
    fn finish_if_last(&mut self) {
        if self.repeat != Repeat::Off || self.shuffle {
            return;
        }
        let Some(cur) = self.current.as_ref() else {
            return;
        };
        if cur.source != ItemSource::Context
            || self.order_at(self.pos).is_none_or(|i| self.items[i].item_id != cur.item_id)
        {
            return;
        }
        let from = (self.pos + 1) as usize;
        let more = self.order[from..].iter().any(|&i| self.items[i].source == ItemSource::Context);
        if !more && let Some(id) = self.context_id().map(str::to_owned) {
            self.clear_resume(&id);
        }
    }

    /// Index and position to resume `id` at, if its point hasn't expired
    /// and its stored track is still in `self.items` (same slot, or else
    /// the first slot with that track).
    fn resume_start(&self, id: &str) -> Option<(usize, u64)> {
        let rp = self.resume_point(id)?;
        let fresh = rp.listened_at_ms.is_some_and(|t| self.now_ms() - t <= RESUME_EXPIRY_MS);
        if !fresh {
            return None;
        }
        let at = |i: usize| self.items.get(i).is_some_and(|it| it.track.id == rp.track_id);
        if at(rp.index) {
            return Some((rp.index, rp.position_ms));
        }
        self.items
            .iter()
            .position(|it| it.track.id == rp.track_id)
            .map(|i| (i, rp.position_ms))
    }

    fn note_played(&mut self, item: &QueueItem) {
        self.recent.retain(|t| *t != item.track.id);
        self.recent.push_back(item.track.id.clone());
        while self.recent.len() > RECENT_CAP {
            self.recent.pop_front();
        }
    }

    fn set_current(&mut self, item: QueueItem, push_history: bool) {
        if let Some(old) = self.current.take()
            && push_history
        {
            self.history.push_back(old);
            while self.history.len() > HISTORY_CAP {
                self.history.pop_front();
            }
        }
        self.note_played(&item);
        self.current = Some(item);
    }

    /// Replaces the context and starts at `start` (index in `tracks`).
    /// See [`Queue::set_context_at`].
    pub fn set_context(
        &mut self,
        info: ContextInfo,
        tracks: Vec<Track>,
        start: Option<usize>,
        shuffle: Option<bool>,
    ) -> Option<QueueItem> {
        self.set_context_at(info, tracks, start, shuffle).map(|(i, _)| i)
    }

    /// Replaces the context. The user queue is kept.
    ///
    /// Shuffle: `shuffle` when given (and remembered for this context id),
    /// otherwise the context's stored preference (off by default).
    ///
    /// Start: `start` when given; with shuffle on, a random track; else the
    /// context's resume point (unless resuming is turned off); else track 1. Returns the new current item
    /// and the position to start it at.
    pub fn set_context_at(
        &mut self,
        info: ContextInfo,
        tracks: Vec<Track>,
        start: Option<usize>,
        shuffle: Option<bool>,
    ) -> Option<(QueueItem, u64)> {
        let id = info.id.clone();
        self.shuffle = match (shuffle, id.as_deref()) {
            (Some(s), Some(id)) => {
                self.store_shuffle_pref(id, s);
                s
            }
            (Some(s), None) => s,
            (None, Some(id)) => self.shuffle_pref(id),
            (None, None) => false,
        };
        self.context = Some(info);
        let mut items = Vec::with_capacity(tracks.len());
        for t in tracks {
            let id = self.new_id();
            items.push(QueueItem {
                item_id: id,
                track: t,
                source: ItemSource::Context,
            });
        }
        self.items = items;
        let n = self.items.len();
        if n == 0 {
            self.order.clear();
            self.pos = -1;
            return None;
        }
        let mut offset = 0;
        let chosen = matches!(start, Some(s) if s < n);
        let resume_id = id.as_deref().filter(|_| self.context.as_ref().is_some_and(|c| resumable(c.kind)));
        let start = match start {
            Some(s) if s < n => s,
            _ if self.shuffle => self.rng.0.random_range(0..n),
            _ => match resume_id.filter(|_| !self.ignore_resume).and_then(|id| self.resume_start(id)) {
                Some((i, ms)) => {
                    offset = ms;
                    i
                }
                None => 0,
            },
        };
        if self.shuffle {
            let others: Vec<usize> = (0..n).filter(|i| *i != start).collect();
            self.order = self.shuffled_order(Some(start), others);
            self.pos = 0;
        } else {
            self.order = (0..n).collect();
            self.pos = start as isize;
        }
        let item = self.items[start].clone();
        self.set_current(item.clone(), true);
        if chosen {
            // Starting a chosen song (a double-click) moves the point there.
            self.note_position(0);
        }
        Some((item, offset))
    }

    /// `first` (if any) followed by `others` in shuffled order, using the
    /// current shuffle style.
    fn shuffled_order(&mut self, first: Option<usize>, mut others: Vec<usize>) -> Vec<usize> {
        match self.shuffle_style {
            ShuffleStyle::Spread => {
                let spread = self.spread(others);
                let mut order: Vec<usize> = first.into_iter().chain(spread).collect();
                self.dedupe_adjacent(&mut order, 1);
                order
            }
            ShuffleStyle::Standard => {
                others.shuffle(&mut self.rng.0);
                first.into_iter().chain(others).collect()
            }
        }
    }

    fn artist_key(t: &Track) -> String {
        t.track_artist
            .as_deref()
            .unwrap_or(&t.artist_title)
            .to_lowercase()
    }

    /// Shuffle that spreads each artist's tracks evenly over the order
    /// (random offset + jitter per artist), with recently played tracks
    /// pushed to the end and no identical tracks back to back.
    fn spread(&mut self, idx: Vec<usize>) -> Vec<usize> {
        let recent: HashSet<&str> = self.recent.iter().map(String::as_str).collect();
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        let mut keys: Vec<String> = Vec::new();
        for i in idx {
            let k = Self::artist_key(&self.items[i].track);
            if !groups.contains_key(&k) {
                keys.push(k.clone());
            }
            groups.entry(k).or_default().push(i);
        }
        let rng = &mut self.rng.0;
        let mut placed: Vec<(f64, usize)> = Vec::new();
        for k in keys {
            let mut g = groups.remove(&k).unwrap_or_default();
            g.shuffle(rng);
            let n = g.len() as f64;
            let offset = rng.random::<f64>() / n;
            for (j, i) in g.into_iter().enumerate() {
                let jitter = (rng.random::<f64>() - 0.5) * 0.2 / n;
                let mut p = offset + j as f64 / n + jitter;
                if recent.contains(self.items[i].track.id.as_str()) {
                    p += 1.0;
                }
                placed.push((p, i));
            }
        }
        placed.sort_by(|a, b| a.0.total_cmp(&b.0));
        placed.into_iter().map(|p| p.1).collect()
    }

    /// No identical track twice in a row (playlists can repeat a track).
    /// Entries before `from` stay where they are.
    fn dedupe_adjacent(&self, order: &mut Vec<usize>, from: usize) {
        let same = |a: usize, b: usize| self.items[a].track.id == self.items[b].track.id;
        let mut i = from.max(1);
        let mut guard = 0;
        while i < order.len() && guard < order.len() * 4 {
            guard += 1;
            if !same(order[i], order[i - 1]) {
                i += 1;
                continue;
            }
            // Swap with a later, different entry...
            if let Some(j) = (i + 1..order.len()).find(|&j| !same(order[j], order[i])) {
                order.swap(i, j);
                continue;
            }
            // ...or move this one to an earlier slot where it fits.
            let it = order.remove(i);
            let slot = (from.max(1)..=order.len()).find(|&p| {
                !same(order[p - 1], it) && (p == order.len() || !same(order[p], it))
            });
            match slot {
                Some(p) => order.insert(p, it),
                None => {
                    order.insert(i, it);
                    i += 1;
                }
            }
        }
    }

    /// What plays after the current track ends on its own.
    pub fn peek_auto(&self) -> Option<QueueItem> {
        if self.repeat == Repeat::One {
            return self.current.clone();
        }
        self.peek_manual()
    }

    /// What "next" would play (ignores repeat-one).
    pub fn peek_manual(&self) -> Option<QueueItem> {
        if let Some(u) = self.user.front() {
            return Some(u.clone());
        }
        let next = self.pos + 1;
        if (next as usize) < self.order.len() {
            return Some(self.items[self.order[next as usize]].clone());
        }
        if self.repeat != Repeat::Off && !self.order.is_empty() {
            return Some(self.items[self.order[0]].clone());
        }
        None
    }

    /// Upcoming items in play order (user queue, then context, wrapping
    /// with repeat-all), at most `n`. Used for pre-caching.
    pub fn upcoming(&self, n: usize) -> Vec<QueueItem> {
        self.upcoming_iter().take(n).cloned().collect()
    }

    /// Upcoming items in play order, without copying any.
    pub fn upcoming_iter(&self) -> impl Iterator<Item = &QueueItem> + '_ {
        let len = self.order.len();
        let start = (self.pos + 1).max(0) as usize;
        let wrap = self.repeat != Repeat::Off;
        let ctx = (0..len)
            .map_while(move |k| {
                let p = start + k;
                if p < len {
                    Some(p)
                } else if wrap {
                    Some(p - len)
                } else {
                    None
                }
            })
            .filter_map(|p| self.order.get(p).and_then(|&i| self.items.get(i)));
        self.user.iter().chain(ctx)
    }

    /// How many entries are still to come (user queue and context).
    pub fn upcoming_len(&self) -> usize {
        self.user.len() + self.remaining()
    }

    /// Puts a queue read from disk right if it doesn't hold together (a
    /// damaged or hand-edited file): the play order is rebuilt and the
    /// position kept in range, so nothing indexes past the end. True if
    /// anything was fixed.
    pub fn repair(&mut self) -> bool {
        let len = self.items.len();
        let mut seen = vec![false; len];
        let order_ok = self.order.len() == len
            && self.order.iter().all(|&i| i < len && !std::mem::replace(&mut seen[i], true));
        let mut fixed = false;
        if !order_ok {
            self.order = (0..len).collect();
            // The current entry keeps its place if it's from the context.
            self.pos = self
                .current
                .as_ref()
                .and_then(|c| self.items.iter().position(|i| i.item_id == c.item_id))
                .map_or(-1, |p| p as isize);
            fixed = true;
        }
        let max = self.order.len() as isize - 1;
        if self.pos < -1 || self.pos > max {
            self.pos = self.pos.clamp(-1, max);
            fixed = true;
        }
        fixed
    }

    fn step(&mut self, push_history: bool) -> Option<QueueItem> {
        if let Some(u) = self.user.pop_front() {
            self.set_current(u.clone(), push_history);
            return Some(u);
        }
        let mut next = self.pos + 1;
        if next as usize >= self.order.len() {
            if self.repeat == Repeat::Off || self.order.is_empty() {
                return None;
            }
            next = 0;
        }
        self.pos = next;
        let item = self.items[self.order[next as usize]].clone();
        self.set_current(item.clone(), push_history);
        Some(item)
    }

    /// The current track ended on its own. Returns the new current, or
    /// `None` at the end of the queue (current is left as it was).
    pub fn advance_auto(&mut self) -> Option<QueueItem> {
        if self.repeat == Repeat::One {
            let cur = self.current.clone()?;
            self.history.push_back(cur.clone());
            while self.history.len() > HISTORY_CAP {
                self.history.pop_front();
            }
            self.note_played(&cur);
            return Some(cur);
        }
        self.finish_if_last();
        self.step(true)
    }

    /// Advances (auto rules first) until `item_id` is current; items passed
    /// on the way (e.g. ones that failed to load) are skipped without
    /// entering history. Returns false if it's not in the queue.
    pub fn advance_auto_to(&mut self, item_id: &str) -> bool {
        if self.advance_auto().is_some_and(|c| c.item_id == item_id) {
            return true;
        }
        let max = self.items.len() + self.user.len() + 1;
        for _ in 0..max {
            self.finish_if_last();
            match self.step(false) {
                Some(c) if c.item_id == item_id => return true,
                Some(_) => {}
                None => return false,
            }
        }
        false
    }

    /// User pressed next. `None` at the end of the queue (repeat off).
    /// Repeat-one drops to repeat-all.
    pub fn next_manual(&mut self) -> Option<QueueItem> {
        if self.repeat == Repeat::One {
            self.repeat = Repeat::All;
        }
        self.step(true)
    }

    /// Moves to the next entry without touching repeat (e.g. the first
    /// queued track when nothing was loaded).
    pub fn advance(&mut self) -> Option<QueueItem> {
        self.step(true)
    }

    /// Play-order index Previous would go to: the context entry before the
    /// current one, or for a playing user-queue entry, the context entry
    /// that played before it.
    fn previous_pos(&self) -> Option<isize> {
        let cur = self.current.as_ref()?;
        let p = if cur.source == ItemSource::User {
            self.pos
        } else {
            self.pos - 1
        };
        self.order_at(p).map(|_| p)
    }

    /// Whether an early Previous has somewhere to go (otherwise it restarts).
    pub fn has_previous(&self) -> bool {
        self.previous_pos().is_some()
    }

    /// User pressed previous within the first 3 s (the controller handles
    /// the restart rule). Goes to the previous context position; a playing
    /// user-queue entry is dropped. Repeat-one drops to repeat-all. Returns
    /// the new current, or `None` on the first position (restart instead).
    pub fn previous(&mut self) -> Option<QueueItem> {
        if self.repeat == Repeat::One {
            self.repeat = Repeat::All;
        }
        let p = self.previous_pos()?;
        self.pos = p;
        let item = self.items[self.order[p as usize]].clone();
        self.set_current(item.clone(), true);
        Some(item)
    }

    /// The context ran out (repeat off): go back to its first track in play
    /// order, with the rest listed as upcoming. `None` with no context.
    pub fn rewind(&mut self) -> Option<QueueItem> {
        let first = *self.order.first()?;
        // Played to the end: the next play starts over. Shuffled listening
        // leaves the point alone.
        if !self.shuffle
            && let Some(id) = self.context_id().map(str::to_owned)
        {
            self.clear_resume(&id);
        }
        self.pos = 0;
        let item = self.items[first].clone();
        if let Some(old) = self.current.replace(item.clone()) {
            self.history.push_back(old);
            while self.history.len() > HISTORY_CAP {
                self.history.pop_front();
            }
        }
        Some(item)
    }

    /// Plays the entry with `item_id` now. Earlier user-queue entries are
    /// dropped (as when jumping ahead in the queue).
    pub fn jump(&mut self, item_id: &str) -> AppResult<QueueItem> {
        if let Some(i) = self.user.iter().position(|u| u.item_id == item_id) {
            self.user.drain(..i);
            let item = self.user.pop_front().expect("found above");
            self.set_current(item.clone(), true);
            return Ok(item);
        }
        if let Some(k) = self
            .order
            .iter()
            .position(|&i| self.items[i].item_id == item_id)
        {
            self.pos = k as isize;
            let item = self.items[self.order[k]].clone();
            self.set_current(item.clone(), true);
            return Ok(item);
        }
        Err(AppError::NotFound(format!("queue item {item_id}")))
    }

    /// Adds tracks to the user queue. `Next` puts them (in order) before
    /// everything else queued; `End` appends; `Index` puts them (in order)
    /// at that spot.
    pub fn add(&mut self, tracks: Vec<Track>, at: AddAt) -> Vec<QueueItem> {
        let items: Vec<QueueItem> = tracks
            .into_iter()
            .map(|t| QueueItem {
                item_id: self.new_id(),
                track: t,
                source: ItemSource::User,
            })
            .collect();
        match at {
            AddAt::Next => {
                for it in items.iter().rev() {
                    self.user.push_front(it.clone());
                }
            }
            AddAt::End => self.user.extend(items.iter().cloned()),
            AddAt::Index(i) => {
                let at = i.min(self.user.len());
                for (k, it) in items.iter().enumerate() {
                    self.user.insert(at + k, it.clone());
                }
            }
        }
        items
    }

    pub fn remove(&mut self, item_id: &str) -> AppResult<()> {
        if self.current.as_ref().is_some_and(|c| c.item_id == item_id) {
            return Err(AppError::Invalid("can't remove the playing track".into()));
        }
        if let Some(i) = self.user.iter().position(|u| u.item_id == item_id) {
            self.user.remove(i);
            return Ok(());
        }
        if let Some(k) = self
            .order
            .iter()
            .position(|&i| self.items[i].item_id == item_id)
        {
            self.order.remove(k);
            if (k as isize) <= self.pos {
                self.pos -= 1;
            }
            return Ok(());
        }
        Err(AppError::NotFound(format!("queue item {item_id}")))
    }

    /// Moves an entry to `to_index` within the user queue. A "Next up"
    /// entry dragged there becomes a user-queue entry.
    pub fn move_item(&mut self, item_id: &str, to_index: usize) -> AppResult<()> {
        let item = if let Some(i) = self.user.iter().position(|u| u.item_id == item_id) {
            self.user.remove(i).expect("index valid")
        } else if let Some(k) = self
            .order
            .iter()
            .position(|&i| self.items[i].item_id == item_id)
            .filter(|k| (*k as isize) > self.pos)
        {
            let idx = self.order.remove(k);
            let mut it = self.items[idx].clone();
            it.source = ItemSource::User;
            it
        } else {
            return Err(AppError::NotFound(format!("queue item {item_id}")));
        };
        let at = to_index.min(self.user.len());
        self.user.insert(at, item);
        Ok(())
    }

    /// Several [`Self::move_item`]s, all or none: when one fails the
    /// queue is left as it was.
    pub fn move_items(&mut self, steps: &[(String, usize)]) -> AppResult<()> {
        let mut next = self.clone();
        for (id, to) in steps {
            next.move_item(id, *to)?;
        }
        *self = next;
        Ok(())
    }

    /// Clears the user queue only.
    pub fn clear_user(&mut self) {
        self.user.clear();
    }

    /// Takes over a queue from another device: `tracks` is the playing song
    /// then what comes next, in play order; `user` is the user queue. Shuffle
    /// and repeat are set as they were there, and the order is kept as given
    /// even with shuffle on (it was already shuffled there).
    pub fn adopt(
        &mut self,
        info: ContextInfo,
        tracks: Vec<Track>,
        user: Vec<Track>,
        shuffle: bool,
        repeat: Repeat,
    ) -> Option<QueueItem> {
        let plain = ContextInfo { id: None, ..info.clone() };
        let first = self.set_context_at(plain, tracks, Some(0), Some(false)).map(|(i, _)| i);
        self.context = Some(info);
        self.user.clear();
        for t in user {
            let item_id = self.new_id();
            self.user.push_back(QueueItem { item_id, track: t, source: ItemSource::User });
        }
        self.shuffle = shuffle;
        // The page's toggle reads the stored preference: keep them the same.
        if let Some(id) = self.context_id().map(str::to_owned) {
            self.store_shuffle_pref(&id, shuffle);
        }
        self.repeat = repeat;
        first
    }

    pub fn set_repeat(&mut self, r: Repeat) {
        self.repeat = r;
    }

    /// Shuffle for the playing context (also remembered for its id).
    pub fn set_shuffle(&mut self, on: bool) {
        if let Some(id) = self.context_id().map(str::to_owned) {
            self.store_shuffle_pref(&id, on);
        }
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        let cur = (self.pos >= 0)
            .then(|| self.order.get(self.pos as usize).copied())
            .flatten();
        let n = self.items.len();
        let live: HashSet<usize> = self.order.iter().copied().collect();
        if on {
            let others: Vec<usize> = (0..n)
                .filter(|i| Some(*i) != cur && live.contains(i))
                .collect();
            self.order = self.shuffled_order(cur, others);
            self.pos = if cur.is_some() { 0 } else { -1 };
        } else {
            // Smart shuffle suggestions only live in a shuffled order: out
            // of it they'd sit after the last playlist song and play again.
            let items = &self.items;
            self.order = (0..n)
                .filter(|i| live.contains(i) && (items[*i].source != ItemSource::Smart || Some(*i) == cur))
                .collect();
            self.pos = match cur {
                Some(c) => self.order.iter().position(|i| *i == c).map_or(-1, |p| p as isize),
                None => -1,
            };
        }
    }

    /// Shuffled, what's left of the context is put in an order whose
    /// neighbours fit (Automix on a mixed playlist): each next song is the
    /// best fit of the next `window` in the shuffled order, so the order
    /// stays random. Does nothing unshuffled.
    pub fn reorder_for_fit(&mut self, score: &dyn Fn(&Track, &Track) -> f32, window: usize) {
        if !self.shuffle {
            return;
        }
        let start = (self.pos + 1).max(0) as usize;
        if start + 1 >= self.order.len() {
            return;
        }
        let mut rest: Vec<usize> = self.order[start..].to_vec();
        let mut prev: Option<Track> = self.current.as_ref().map(|c| c.track.clone());
        let mut out = Vec::with_capacity(rest.len());
        while !rest.is_empty() {
            let pick = match &prev {
                Some(p) => (0..rest.len().min(window.max(1)))
                    .max_by(|&i, &j| {
                        score(p, &self.items[rest[i]].track)
                            .total_cmp(&score(p, &self.items[rest[j]].track))
                            .then(j.cmp(&i))
                    })
                    .unwrap_or(0),
                None => 0,
            };
            let idx = rest.remove(pick);
            prev = Some(self.items[idx].track.clone());
            out.push(idx);
        }
        self.order.truncate(start);
        self.order.extend(out);
        self.dedupe_adjacent_from(start);
    }

    fn dedupe_adjacent_from(&mut self, start: usize) {
        let mut order = std::mem::take(&mut self.order);
        self.dedupe_adjacent(&mut order, start);
        self.order = order;
    }

    /// The context's songs (for Automix's look at a whole playlist).
    pub fn context_tracks(&self, n: usize) -> Vec<Track> {
        self.items.iter().take(n).map(|i| i.track.clone()).collect()
    }

    /// Adds songs at the end of the context's play order (a radio refill,
    /// or similar music once the context runs out).
    pub fn extend_context(&mut self, tracks: Vec<Track>, source: ItemSource) {
        for t in tracks {
            let item_id = self.new_id();
            self.items.push(QueueItem { item_id, track: t, source });
            self.order.push(self.items.len() - 1);
        }
    }

    /// Where smart shuffle suggestions go in the upcoming play order: one
    /// after every `every` context songs, counting the playing song when it's
    /// a context song, and never after the last one (the list ends on a
    /// context song). Returns the play-order positions (into what's upcoming)
    /// a suggestion follows.
    fn smart_slots_at(&self, every: usize) -> Vec<usize> {
        let every = every.max(1);
        let from = ((self.pos + 1).max(0) as usize).min(self.order.len());
        let rest = &self.order[from..];
        let ctx: Vec<bool> = rest.iter().map(|&i| self.items[i].source == ItemSource::Context).collect();
        let mut left = ctx.iter().filter(|c| **c).count();
        let mut count = usize::from(self.current.as_ref().is_some_and(|c| c.source == ItemSource::Context));
        let mut out = Vec::new();
        for (k, is_ctx) in ctx.into_iter().enumerate() {
            if !is_ctx {
                continue;
            }
            count += 1;
            left -= 1;
            if count >= every && left > 0 {
                out.push(k);
                count = 0;
            }
        }
        out
    }

    /// How many suggestions the upcoming list has room for.
    pub fn smart_slots(&self, every: usize) -> usize {
        self.smart_slots_at(every).len()
    }

    /// Puts smart shuffle songs in among the upcoming context songs, one
    /// after every `every` (see [`Queue::smart_slots`]). With fewer songs
    /// than slots, the first slots get them.
    pub fn insert_smart(&mut self, tracks: Vec<Track>, every: usize) {
        let slots = self.smart_slots_at(every);
        let split = ((self.pos + 1).max(0) as usize).min(self.order.len());
        let rest: Vec<usize> = self.order.split_off(split);
        let mut added = Vec::with_capacity(tracks.len());
        for t in tracks.into_iter().take(slots.len()) {
            let item_id = self.new_id();
            self.items.push(QueueItem { item_id, track: t, source: ItemSource::Smart });
            added.push(self.items.len() - 1);
        }
        let mut added = added.into_iter();
        let mut slots = slots.into_iter().peekable();
        for (k, idx) in rest.into_iter().enumerate() {
            self.order.push(idx);
            if slots.peek() == Some(&k) {
                slots.next();
                if let Some(a) = added.next() {
                    self.order.push(a);
                }
            }
        }
    }

    /// Smart shuffle songs still to come.
    pub fn smart_upcoming(&self) -> usize {
        let from = ((self.pos + 1).max(0) as usize).min(self.order.len());
        self.order[from..].iter().filter(|&&i| self.items[i].source == ItemSource::Smart).count()
    }

    /// Takes the smart shuffle songs still to come out of the play order.
    pub fn drop_smart_upcoming(&mut self) {
        self.drop_upcoming(|i| i.source == ItemSource::Smart);
    }

    /// Takes the context entries still to come that `gone` picks out of
    /// the play order (the user queue is left alone).
    pub fn drop_upcoming(&mut self, gone: impl Fn(&QueueItem) -> bool) {
        let from = ((self.pos + 1).max(0) as usize).min(self.order.len());
        let items = &self.items;
        let mut keep: Vec<usize> = self.order[from..].iter().copied().filter(|&i| !gone(&items[i])).collect();
        self.order.truncate(from);
        self.order.append(&mut keep);
    }

    /// A smart shuffle song was added to the context for good: it becomes a
    /// context song (no badge). False when no such entry is in the queue.
    pub fn keep_smart(&mut self, item_id: &str) -> bool {
        let mut found = false;
        for it in self.items.iter_mut().chain(self.current.iter_mut()) {
            if it.item_id == item_id && it.source == ItemSource::Smart {
                it.source = ItemSource::Context;
                found = true;
            }
        }
        found
    }

    /// Up to `n` of the context's own songs (not added ones), spread evenly
    /// over the whole context.
    pub fn context_sample(&self, n: usize) -> Vec<String> {
        let own: Vec<&QueueItem> = self.items.iter().filter(|i| i.source == ItemSource::Context).collect();
        if own.len() <= n {
            return own.iter().map(|i| i.track.id.clone()).collect();
        }
        (0..n).map(|k| own[k * own.len() / n].track.id.clone()).collect()
    }

    /// Artist ids of the context's own songs (not added ones).
    pub fn context_artist_ids(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.items
            .iter()
            .filter(|i| i.source == ItemSource::Context)
            .map(|i| i.track.artist_id.clone())
            .filter(|a| seen.insert(a.clone()))
            .collect()
    }

    /// Context entries still to play after the current one.
    pub fn remaining(&self) -> usize {
        self.order.len().saturating_sub((self.pos + 1).max(0) as usize)
    }

    /// Number of context entries (changes when the context grows).
    pub fn context_len(&self) -> usize {
        self.items.len()
    }

    /// Track ids of the context up to the current position, newest last.
    pub fn played_context_ids(&self, n: usize) -> Vec<String> {
        let end = ((self.pos + 1).max(0) as usize).min(self.order.len());
        let start = end.saturating_sub(n);
        self.order[start..end].iter().map(|&i| self.items[i].track.id.clone()).collect()
    }

    /// Every track id the queue holds or has played lately.
    pub fn known_ids(&self) -> Vec<String> {
        self.items
            .iter()
            .chain(self.user.iter())
            .chain(self.history.iter())
            .chain(self.current.iter())
            .map(|i| i.track.id.clone())
            .collect()
    }

    pub fn view(&self) -> QueueView {
        let next_up = self
            .order
            .iter()
            .skip((self.pos + 1).max(0) as usize)
            .map(|&i| self.items[i].clone())
            .collect();
        QueueView {
            current: self.current.clone(),
            user_queue: self.user.iter().cloned().collect(),
            next_up,
            history: self.history.iter().rev().take(50).cloned().collect(),
            context: self.context.clone(),
        }
    }

    pub fn find(&self, item_id: &str) -> Option<&QueueItem> {
        self.current
            .iter()
            .chain(self.user.iter())
            .chain(self.items.iter())
            .find(|i| i.item_id == item_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::testutil::track;

    fn next_ids(q: &Queue) -> Vec<String> {
        q.view().next_up.iter().map(|i| i.track.id.clone()).collect()
    }

    fn suggestions(n: usize) -> Vec<Track> {
        (0..n).map(|i| track(&format!("s{i}"), "B", "/p")).collect()
    }

    #[test]
    fn smart_shuffle_puts_one_suggestion_after_every_two_playlist_songs() {
        let mut q = Queue::with_seed(1);
        let ts: Vec<Track> = (0..7).map(|i| track(&i.to_string(), "A", "/p")).collect();
        q.set_context(ctx(), ts, Some(0), Some(false));
        // The playing song counts: 0 1 s0 2 3 s1 4 5 s2 6 (ends on a playlist song).
        assert_eq!(q.smart_slots(2), 3);
        q.insert_smart(suggestions(5), 2);
        assert_eq!(next_ids(&q), vec!["1", "s0", "2", "3", "s1", "4", "5", "s2", "6"]);
        assert_eq!(q.smart_upcoming(), 3);
        q.drop_smart_upcoming();
        assert_eq!(q.smart_upcoming(), 0);
        assert_eq!(next_ids(&q), vec!["1", "2", "3", "4", "5", "6"]);
    }

    #[test]
    fn the_two_to_one_rhythm_holds_on_short_and_long_playlists() {
        for (n, want) in [(1, 0), (2, 0), (3, 1), (5, 2), (23, 11), (500, 249)] {
            let mut q = Queue::with_seed(3);
            let ts: Vec<Track> = (0..n).map(|i| track(&i.to_string(), "A", "/p")).collect();
            q.set_context(ctx(), ts, None, Some(true));
            assert_eq!(q.smart_slots(2), want, "{n} songs");
            q.insert_smart(suggestions(want + 5), 2);
            let up = q.view().next_up;
            assert_eq!(up.len(), n - 1 + want);
            // Two playlist songs (the playing one first), then one suggestion.
            let mut kinds: Vec<ItemSource> = vec![ItemSource::Context];
            kinds.extend(up.iter().map(|i| i.source));
            for (k, s) in kinds.iter().enumerate() {
                let expect = if k % 3 == 2 { ItemSource::Smart } else { ItemSource::Context };
                assert_eq!(*s, expect, "{n} songs, entry {k}");
            }
            assert_ne!(up.last().map(|i| i.source), Some(ItemSource::Smart), "ends on a playlist song");
        }
    }

    #[test]
    fn fewer_suggestions_than_slots_fill_the_first_ones() {
        let mut q = Queue::with_seed(1);
        let ts: Vec<Track> = (0..7).map(|i| track(&i.to_string(), "A", "/p")).collect();
        q.set_context(ctx(), ts, Some(0), Some(false));
        q.insert_smart(suggestions(1), 2);
        assert_eq!(next_ids(&q), vec!["1", "s0", "2", "3", "4", "5", "6"]);
    }

    #[test]
    fn a_suggestion_playing_starts_the_count_afresh() {
        let mut q = Queue::with_seed(1);
        let ts: Vec<Track> = (0..5).map(|i| track(&i.to_string(), "A", "/p")).collect();
        q.set_context(ctx(), ts, Some(0), Some(false));
        q.insert_smart(suggestions(2), 2);
        q.advance_auto();
        q.advance_auto();
        assert_eq!(q.current().unwrap().source, ItemSource::Smart);
        q.drop_smart_upcoming();
        q.insert_smart(suggestions(3), 2);
        assert_eq!(next_ids(&q), vec!["2", "3", "s0", "4"]);
    }

    #[test]
    fn a_handed_in_queue_keeps_its_shuffle_preference() {
        let mut q = Queue::with_seed(1);
        let ts: Vec<Track> = (0..3).map(|i| track(&i.to_string(), "A", "/p")).collect();
        let info = ContextInfo { kind: ContextKind::Playlist, id: Some("pl".into()), title: None };
        q.adopt(info.clone(), ts.clone(), Vec::new(), true, Repeat::Off);
        assert!(q.shuffle_pref("pl"));
        q.adopt(info, ts, Vec::new(), false, Repeat::Off);
        assert!(!q.shuffle_pref("pl"));
    }

    #[test]
    fn shuffle_off_drops_suggestions_that_already_played() {
        let mut q = Queue::with_seed(1);
        let ts: Vec<Track> = (0..5).map(|i| track(&i.to_string(), "A", "/p")).collect();
        q.set_context(ctx(), ts, Some(0), Some(true));
        q.insert_smart(suggestions(2), 2);
        q.advance_auto();
        assert_eq!(q.advance_auto().unwrap().source, ItemSource::Smart);
        q.advance_auto();
        q.set_shuffle(false);
        assert!(next_ids(&q).iter().all(|id| !id.starts_with('s')), "{:?}", next_ids(&q));
    }

    #[test]
    fn a_kept_suggestion_loses_its_badge() {
        let mut q = Queue::with_seed(1);
        let ts: Vec<Track> = (0..4).map(|i| track(&i.to_string(), "A", "/p")).collect();
        q.set_context(ctx(), ts, Some(0), Some(false));
        q.insert_smart(suggestions(1), 2);
        q.advance_auto();
        let cur = q.advance_auto().unwrap();
        assert_eq!(cur.source, ItemSource::Smart);
        assert!(q.keep_smart(&cur.item_id));
        assert_eq!(q.current().unwrap().source, ItemSource::Context);
        assert!(!q.keep_smart(&cur.item_id), "only suggestions");
        assert!(q.context_sample(10).contains(&"s0".to_string()));
    }

    #[test]
    fn smart_preference_is_kept_per_context_inside_shuffle() {
        let mut q = Queue::with_seed(1);
        q.store_smart_pref("pl", true);
        assert!(q.smart_pref("pl") && q.shuffle_pref("pl"));
        assert_eq!(q.smart_contexts(), vec!["pl".to_string()]);
        // Shuffle off for it ends smart shuffle too.
        q.set_context_shuffle("pl", false);
        assert!(!q.smart_pref("pl") && !q.shuffle_pref("pl"));
        q.store_smart_pref("pl", true);
        q.store_smart_pref("pl", false);
        assert!(!q.smart_pref("pl") && q.shuffle_pref("pl"), "smart off leaves shuffle");
    }

    #[test]
    fn shuffle_off_can_keep_the_shuffled_order() {
        let mut q = Queue::with_seed(9);
        let ts: Vec<Track> = (0..8).map(|i| track(&i.to_string(), "A", "/p")).collect();
        q.set_context(ctx(), ts, None, Some(true));
        let before = next_ids(&q);
        q.shuffle_off_keep_order();
        assert!(!q.shuffle() && !q.shuffle_pref("al"));
        assert_eq!(next_ids(&q), before);
    }

    #[test]
    fn the_context_sample_spreads_over_the_whole_context() {
        let mut q = Queue::with_seed(1);
        let ts: Vec<Track> = (0..10).map(|i| track(&i.to_string(), &format!("A{}", i % 3), "/p")).collect();
        q.set_context(ctx(), ts, Some(0), Some(false));
        assert_eq!(q.context_sample(2), vec!["0".to_string(), "5".to_string()]);
        assert_eq!(q.context_artist_ids().len(), 3);
    }

    #[test]
    fn extending_the_context_adds_to_the_end_of_the_play_order() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), vec![track("1", "A", "/p"), track("2", "A", "/p")], Some(1), Some(false));
        assert_eq!(q.remaining(), 0);
        q.extend_context(vec![track("3", "B", "/p"), track("4", "B", "/p")], ItemSource::Autoplay);
        assert_eq!(q.remaining(), 2);
        assert_eq!(q.context_len(), 4);
        let next = q.advance_auto().unwrap();
        assert_eq!((next.track.id.as_str(), next.source), ("3", ItemSource::Autoplay));
        assert_eq!(q.played_context_ids(2), vec!["2".to_string(), "3".to_string()]);
        assert!(q.known_ids().contains(&"4".to_string()));
    }

    fn ctx() -> ContextInfo {
        ContextInfo {
            kind: ContextKind::Album,
            id: Some("al".into()),
            title: None,
        }
    }

    fn tracks(n: usize) -> Vec<Track> {
        (0..n).map(|i| track(&format!("t{i}"), "A", "/p")).collect()
    }

    fn ids(items: &[QueueItem]) -> Vec<String> {
        items.iter().map(|i| i.track.id.clone()).collect()
    }

    fn cur(q: &Queue) -> String {
        q.current().unwrap().track.id.clone()
    }

    #[test]
    fn context_plays_in_order_and_next_up_is_the_rest() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(4), Some(1), Some(false));
        assert_eq!(cur(&q), "t1");
        assert_eq!(ids(&q.view().next_up), vec!["t2", "t3"]);
        assert_eq!(q.advance_auto().unwrap().track.id, "t2");
        assert_eq!(q.advance_auto().unwrap().track.id, "t3");
        assert!(q.advance_auto().is_none(), "end of context, repeat off");
        assert_eq!(cur(&q), "t3");
    }

    #[test]
    fn add_at_index_inserts_in_order_and_clamps_past_the_end() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(3), Some(0), Some(false));
        q.add(vec![track("a", "B", "/p"), track("b", "B", "/p")], AddAt::End);
        q.add(vec![track("x", "B", "/p"), track("y", "B", "/p")], AddAt::Index(1));
        assert_eq!(ids(&q.view().user_queue), vec!["a", "x", "y", "b"]);
        q.add(vec![track("z", "B", "/p")], AddAt::Index(99));
        assert_eq!(ids(&q.view().user_queue), vec!["a", "x", "y", "b", "z"]);
        q.add(vec![track("f", "B", "/p")], AddAt::Index(0));
        assert_eq!(ids(&q.view().user_queue)[0], "f");
    }

    #[test]
    fn adopting_a_queue_keeps_its_order_user_queue_shuffle_and_repeat() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(3), Some(0), Some(false));
        q.add(vec![track("old", "B", "/p")], AddAt::End);
        let order = vec![track("t3", "A", "/p"), track("t1", "A", "/p"), track("t2", "A", "/p")];
        q.adopt(ctx(), order, vec![track("u", "B", "/p")], true, Repeat::All);
        assert_eq!(cur(&q), "t3");
        assert_eq!(ids(&q.view().user_queue), vec!["u"], "the old user queue is replaced");
        assert_eq!(ids(&q.view().next_up), vec!["t1", "t2"], "shuffled order kept as sent");
        assert!(q.shuffle());
        assert_eq!(q.repeat(), Repeat::All);
        assert_eq!(q.context().and_then(|c| c.id.as_deref()), Some("al"));
        assert!(q.shuffle_pref("al"), "the page toggle shows the shuffle the queue came with");
    }

    #[test]
    fn add_at_deserializes_from_the_ui_forms() {
        assert_eq!(serde_json::from_str::<AddAt>(r#""next""#).unwrap(), AddAt::Next);
        assert_eq!(serde_json::from_str::<AddAt>(r#""end""#).unwrap(), AddAt::End);
        assert_eq!(serde_json::from_str::<AddAt>(r#"{"index":2}"#).unwrap(), AddAt::Index(2));
    }

    #[test]
    fn play_next_goes_before_add_to_queue_and_both_before_context() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(3), Some(0), Some(false));
        q.add(vec![track("e1", "B", "/p")], AddAt::End);
        q.add(vec![track("e2", "B", "/p")], AddAt::End);
        q.add(vec![track("n1", "B", "/p"), track("n2", "B", "/p")], AddAt::Next);
        let v = q.view();
        assert_eq!(ids(&v.user_queue), vec!["n1", "n2", "e1", "e2"]);
        assert!(v.user_queue.iter().all(|i| i.source == ItemSource::User));
        let mut played = vec![];
        while let Some(i) = q.advance_auto() {
            played.push(i.track.id);
        }
        assert_eq!(played, vec!["n1", "n2", "e1", "e2", "t1", "t2"]);
    }

    #[test]
    fn new_context_keeps_user_queue_and_clear_only_clears_user_queue() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(3), None, Some(false));
        q.add(vec![track("u", "B", "/p")], AddAt::End);
        q.set_context(ctx(), tracks(2), Some(0), None);
        assert_eq!(ids(&q.view().user_queue), vec!["u"]);
        q.clear_user();
        let v = q.view();
        assert!(v.user_queue.is_empty());
        assert_eq!(ids(&v.next_up), vec!["t1"]);
        assert_eq!(cur(&q), "t0");
    }

    #[test]
    fn item_ids_are_unique_and_stable() {
        let mut q = Queue::with_seed(1);
        let mut t = tracks(2);
        t.push(t[0].clone()); // same track twice
        q.set_context(ctx(), t, Some(0), Some(false));
        q.add(vec![track("t0", "A", "/p")], AddAt::End);
        let v = q.view();
        let mut all: Vec<String> = v.next_up.iter().map(|i| i.item_id.clone()).collect();
        all.extend(v.user_queue.iter().map(|i| i.item_id.clone()));
        all.push(v.current.unwrap().item_id);
        let set: HashSet<_> = all.iter().collect();
        assert_eq!(set.len(), all.len());
    }

    #[test]
    fn repeat_all_wraps_and_repeat_one_repeats_until_manual_next() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(2), Some(0), Some(false));
        q.set_repeat(Repeat::All);
        assert_eq!(q.advance_auto().unwrap().track.id, "t1");
        assert_eq!(q.peek_auto().unwrap().track.id, "t0");
        assert_eq!(q.advance_auto().unwrap().track.id, "t0");
        q.set_repeat(Repeat::One);
        let id = q.current().unwrap().item_id.clone();
        assert_eq!(q.peek_auto().unwrap().item_id, id);
        assert_eq!(q.advance_auto().unwrap().item_id, id);
        assert_eq!(q.advance_auto().unwrap().item_id, id);
        // Next skips on and drops repeat-one to repeat-all.
        assert_eq!(q.next_manual().unwrap().track.id, "t1");
        assert_eq!(q.repeat(), Repeat::All);
        q.set_repeat(Repeat::Off);
        assert!(q.next_manual().is_none());
        assert!(q.peek_auto().is_none());
    }

    #[test]
    fn early_previous_in_repeat_one_drops_to_repeat_all() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(3), Some(1), Some(false));
        q.set_repeat(Repeat::One);
        assert_eq!(q.previous().unwrap().track.id, "t0");
        assert_eq!(q.repeat(), Repeat::All);
        // On the first track there is nowhere to go, but it still drops.
        q.set_repeat(Repeat::One);
        assert!(q.previous().is_none());
        assert_eq!(q.repeat(), Repeat::All);
    }

    #[test]
    fn previous_walks_back_through_context_positions_then_forward_again() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(4), Some(0), Some(false));
        q.next_manual();
        q.next_manual();
        assert_eq!(cur(&q), "t2");
        assert_eq!(q.previous().unwrap().track.id, "t1");
        assert_eq!(q.previous().unwrap().track.id, "t0");
        assert!(!q.has_previous());
        assert!(q.previous().is_none(), "first track: caller restarts");
        assert_eq!(cur(&q), "t0");
        assert_eq!(q.next_manual().unwrap().track.id, "t1");
        assert_eq!(q.next_manual().unwrap().track.id, "t2");
    }

    #[test]
    fn previous_after_a_jump_ahead_goes_to_the_skipped_neighbour() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(6), Some(0), Some(false));
        let t4 = q.view().next_up[3].item_id.clone();
        q.jump(&t4).unwrap();
        assert_eq!(cur(&q), "t4");
        // t3 never played, but it's the previous context position.
        assert_eq!(q.previous().unwrap().track.id, "t3");
        assert_eq!(ids(&q.view().next_up), vec!["t4", "t5"]);
    }

    #[test]
    fn previous_skips_a_queued_song_that_played_between_context_tracks() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(4), Some(1), Some(false));
        q.add(vec![track("u", "B", "/p")], AddAt::End);
        assert_eq!(q.advance_auto().unwrap().track.id, "u");
        assert_eq!(q.advance_auto().unwrap().track.id, "t2");
        assert_eq!(q.previous().unwrap().track.id, "t1");
        assert!(q.view().user_queue.is_empty());
    }

    #[test]
    fn previous_on_a_queued_song_drops_it_and_returns_to_the_context_track() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(3), Some(0), Some(false));
        q.add(vec![track("u", "B", "/p")], AddAt::Next);
        assert_eq!(q.next_manual().unwrap().track.id, "u");
        assert_eq!(q.previous().unwrap().track.id, "t0");
        assert!(q.view().user_queue.is_empty(), "the queued song is dropped");
        assert_eq!(q.next_manual().unwrap().track.id, "t1");
        assert_eq!(q.next_manual().unwrap().track.id, "t2");
    }

    #[test]
    fn previous_on_a_queued_song_with_no_context_track_before_it_restarts() {
        let mut q = Queue::with_seed(1);
        q.add(vec![track("u", "B", "/p")], AddAt::End);
        q.advance();
        assert_eq!(cur(&q), "u");
        assert!(!q.has_previous());
        assert!(q.previous().is_none());
        assert_eq!(cur(&q), "u");
    }

    #[test]
    fn shuffled_previous_walks_the_shuffled_order() {
        let mut q = Queue::with_seed(4);
        q.set_context(ctx(), tracks(8), None, Some(true));
        let mut played = vec![cur(&q)];
        for _ in 0..3 {
            played.push(q.next_manual().unwrap().track.id);
        }
        for want in played.iter().rev().skip(1) {
            assert_eq!(&q.previous().unwrap().track.id, want);
        }
        assert!(q.previous().is_none());
    }

    #[test]
    fn rewind_goes_back_to_the_first_track_with_the_rest_upcoming() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(3), Some(0), Some(false));
        q.advance_auto();
        q.advance_auto();
        assert!(q.advance_auto().is_none());
        assert_eq!(q.rewind().unwrap().track.id, "t0");
        assert_eq!(cur(&q), "t0");
        assert_eq!(ids(&q.view().next_up), vec!["t1", "t2"]);
        assert_eq!(ids(&q.view().history), vec!["t2", "t1", "t0"]);
        assert!(Queue::new().rewind().is_none());
    }

    #[test]
    fn history_is_most_recent_first() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(3), Some(0), Some(false));
        q.advance_auto();
        q.advance_auto();
        assert_eq!(ids(&q.view().history), vec!["t1", "t0"]);
    }

    #[test]
    fn jump_into_user_queue_drops_earlier_entries_and_into_next_up_keeps_queue() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(5), Some(0), Some(false));
        let added = q.add(
            vec![track("a", "B", "/p"), track("b", "B", "/p"), track("c", "B", "/p")],
            AddAt::End,
        );
        q.jump(&added[1].item_id).unwrap();
        assert_eq!(cur(&q), "b");
        assert_eq!(ids(&q.view().user_queue), vec!["c"]);
        let t3 = q.view().next_up[2].item_id.clone();
        q.jump(&t3).unwrap();
        assert_eq!(cur(&q), "t3");
        assert_eq!(ids(&q.view().user_queue), vec!["c"]);
        assert_eq!(ids(&q.view().next_up), vec!["t4"]);
        assert!(q.jump("nope").is_err());
    }

    #[test]
    fn remove_and_move() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(4), Some(0), Some(false));
        let a = q.add(
            vec![track("a", "B", "/p"), track("b", "B", "/p"), track("c", "B", "/p")],
            AddAt::End,
        );
        q.move_item(&a[2].item_id, 0).unwrap();
        assert_eq!(ids(&q.view().user_queue), vec!["c", "a", "b"]);
        q.remove(&a[0].item_id).unwrap();
        assert_eq!(ids(&q.view().user_queue), vec!["c", "b"]);
        // Drag a Next-up entry into the user queue.
        let t2 = q.view().next_up[1].item_id.clone();
        q.move_item(&t2, 1).unwrap();
        let v = q.view();
        assert_eq!(ids(&v.user_queue), vec!["c", "t2", "b"]);
        assert_eq!(v.user_queue[1].source, ItemSource::User);
        assert_eq!(ids(&v.next_up), vec!["t1", "t3"]);
        // Remove from next up.
        let t3 = v.next_up[1].item_id.clone();
        q.remove(&t3).unwrap();
        assert_eq!(ids(&q.view().next_up), vec!["t1"]);
        let current = q.current().unwrap().item_id.clone();
        assert!(q.remove(&current).is_err());
        assert!(q.move_item(&current, 0).is_err());
    }

    #[test]
    fn several_moves_are_all_or_none() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(4), Some(0), Some(false));
        let v = q.view();
        let (t1, t2) = (v.next_up[0].item_id.clone(), v.next_up[1].item_id.clone());
        // The second step can't be made: the first is undone.
        assert!(q.move_items(&[(t1.clone(), usize::MAX), ("gone".into(), 0)]).is_err());
        let v = q.view();
        assert!(v.user_queue.is_empty());
        assert_eq!(ids(&v.next_up), vec!["t1", "t2", "t3"]);
        q.move_items(&[(t1, usize::MAX), (t2, 0)]).unwrap();
        assert_eq!(ids(&q.view().user_queue), vec!["t2", "t1"]);
    }

    #[test]
    fn shuffle_keeps_current_first_and_unshuffle_restores_order_from_current() {
        let mut q = Queue::with_seed(7);
        q.set_context(ctx(), tracks(10), Some(3), Some(false));
        q.set_shuffle(true);
        assert_eq!(cur(&q), "t3");
        let up = ids(&q.view().next_up);
        assert_eq!(up.len(), 9);
        assert!(!up.contains(&"t3".to_string()));
        assert_ne!(up, vec!["t0", "t1", "t2", "t4", "t5", "t6", "t7", "t8", "t9"]);
        q.next_manual();
        let now = cur(&q);
        q.set_shuffle(false);
        let n: usize = now[1..].parse().unwrap();
        let expect: Vec<String> = (n + 1..10).map(|i| format!("t{i}")).collect();
        assert_eq!(ids(&q.view().next_up), expect);
    }

    #[test]
    fn shuffle_play_is_deterministic_with_a_seed() {
        let run = |seed| {
            let mut q = Queue::with_seed(seed);
            q.set_context(ctx(), tracks(20), None, Some(true));
            let mut v = vec![cur(&q)];
            v.extend(ids(&q.view().next_up));
            v
        };
        assert_eq!(run(42), run(42));
        assert_ne!(run(42), run(43));
        let mut all = run(42);
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 20, "a permutation");
    }

    fn adjacent_same_artist(order: &[QueueItem]) -> usize {
        order
            .windows(2)
            .filter(|w| w[0].track.artist_title == w[1].track.artist_title)
            .count()
    }

    #[test]
    fn shuffle_spreads_artists_apart() {
        // 3 artists x 10 tracks. A plain shuffle puts ~9 same-artist pairs
        // side by side on average; the spread shuffle should do far better.
        let mut t = Vec::new();
        for a in ["A", "B", "C"] {
            for i in 0..10 {
                t.push(track(&format!("{a}{i}"), a, "/p"));
            }
        }
        let mut spread_total = 0;
        let mut naive_total = 0;
        let runs = 200;
        for seed in 0..runs {
            let mut q = Queue::with_seed(seed);
            q.set_context(ctx(), t.clone(), Some(0), Some(true));
            let mut order = vec![q.current().unwrap().clone()];
            order.extend(q.view().next_up);
            spread_total += adjacent_same_artist(&order);

            let mut naive = order.clone();
            let mut rng = StdRng::seed_from_u64(seed + 10_000);
            naive.shuffle(&mut rng);
            naive_total += adjacent_same_artist(&naive);
        }
        let spread_avg = spread_total as f64 / runs as f64;
        let naive_avg = naive_total as f64 / runs as f64;
        assert!(naive_avg > 7.0, "naive {naive_avg}");
        assert!(spread_avg < 2.5, "spread {spread_avg} vs naive {naive_avg}");
    }

    #[test]
    fn shuffle_never_repeats_a_track_back_to_back() {
        let mut t = tracks(6);
        // Playlist with duplicates of the same track.
        t.push(t[1].clone());
        t.push(t[1].clone());
        t.push(t[2].clone());
        for seed in 0..100 {
            let mut q = Queue::with_seed(seed);
            q.set_context(ctx(), t.clone(), Some(0), Some(true));
            let mut order = vec![cur(&q)];
            order.extend(ids(&q.view().next_up));
            for w in order.windows(2) {
                assert_ne!(w[0], w[1], "seed {seed}: {order:?}");
            }
        }
    }

    #[test]
    fn shuffle_pushes_recently_played_to_the_back() {
        let mut q = Queue::with_seed(3);
        q.set_context(ctx(), tracks(10), Some(0), Some(false));
        for _ in 0..4 {
            q.advance_auto();
        }
        // t0..t4 were just played; reshuffle the same tracks.
        q.set_context(ctx(), tracks(10), Some(9), Some(true));
        let up = ids(&q.view().next_up);
        let tail: HashSet<&str> = up[up.len() - 5..].iter().map(String::as_str).collect();
        for t in ["t0", "t1", "t2", "t3", "t4"] {
            assert!(tail.contains(t), "{t} should be near the end: {up:?}");
        }
    }

    #[test]
    fn upcoming_lists_user_then_context_and_wraps_with_repeat_all() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(3), Some(1), Some(false));
        q.add(vec![track("u", "B", "/p")], AddAt::End);
        assert_eq!(ids(&q.upcoming(5)), vec!["u", "t2"]);
        q.set_repeat(Repeat::All);
        assert_eq!(ids(&q.upcoming(5)), vec!["u", "t2", "t0", "t1"]);
    }

    #[test]
    fn advance_auto_to_skips_failed_items_without_history() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(4), Some(0), Some(false));
        let t2 = q.view().next_up[1].item_id.clone();
        assert!(q.advance_auto_to(&t2));
        assert_eq!(cur(&q), "t2");
        // t0 in history; t1 (skipped) not.
        assert_eq!(ids(&q.view().history), vec!["t0"]);
    }

    #[test]
    fn serde_round_trip_preserves_everything_observable() {
        let mut q = Queue::with_seed(5);
        q.set_context(ctx(), tracks(6), Some(2), Some(true));
        q.add(vec![track("u", "B", "/p")], AddAt::Next);
        q.set_repeat(Repeat::All);
        q.advance_auto();
        let json = serde_json::to_string(&q).unwrap();
        let mut r: Queue = serde_json::from_str(&json).unwrap();
        assert_eq!(r.view(), q.view());
        assert_eq!(r.repeat(), Repeat::All);
        assert!(r.shuffle());
        // Continues identically.
        assert_eq!(r.advance_auto(), q.advance_auto());
        // New ids don't collide after restore.
        let a = r.add(vec![track("x", "B", "/p")], AddAt::End);
        assert!(q.find(&a[0].item_id).is_none());
    }

    fn ctx_id(id: &str) -> ContextInfo {
        ContextInfo {
            kind: ContextKind::Album,
            id: Some(id.into()),
            title: None,
        }
    }

    #[test]
    fn fresh_context_starts_at_track_one() {
        let mut q = Queue::with_seed(1);
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(5), None, None).unwrap();
        assert_eq!(item.track.id, "t0");
        assert_eq!(ms, 0);
        assert!(!q.shuffle());
    }

    #[test]
    fn resume_off_starts_at_track_one_but_keeps_the_point() {
        let mut q = Queue::with_seed(1);
        assert!(q.resume_contexts(), "on by default");
        q.set_context(ctx_id("x"), tracks(5), Some(2), None);
        q.note_position(42_000);
        q.set_context(ctx_id("y"), tracks(3), None, None);

        q.set_resume_contexts(false);
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(5), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t0", 0));
        assert!(q.resume_point("x").is_some(), "points are still stored");

        // Back on: the latest point for x is used again.
        q.note_position(5_000);
        q.set_context(ctx_id("y"), tracks(3), None, None);
        q.set_resume_contexts(true);
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(5), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t0", 5_000));
    }

    #[test]
    fn a_context_played_before_resumes_at_its_track_and_position() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx_id("x"), tracks(5), Some(2), None);
        q.note_position(42_000);
        // Something else plays in between.
        q.set_context(ctx_id("y"), tracks(3), None, None);
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(5), None, None).unwrap();
        assert_eq!(item.track.id, "t2");
        assert_eq!(ms, 42_000);
        assert_eq!(ids(&q.view().next_up), vec!["t3", "t4"]);
        // An explicit start track wins over the resume point.
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(5), Some(4), None).unwrap();
        assert_eq!(item.track.id, "t4");
        assert_eq!(ms, 0);
    }

    #[test]
    fn resume_while_a_queued_song_plays_points_at_the_next_context_track() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx_id("x"), tracks(5), Some(1), None);
        q.add(vec![track("u", "B", "/p")], AddAt::End);
        q.advance_auto();
        q.note_position(9_000);
        let rp = q.resume_point("x").unwrap();
        assert_eq!((rp.index, rp.track_id.as_str(), rp.position_ms), (2, "t2", 0));
    }

    #[test]
    fn resume_point_for_a_missing_track_starts_at_track_one() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx_id("x"), tracks(5), Some(3), None);
        q.note_position(5_000);
        // The playlist changed: t3 is gone.
        let changed: Vec<Track> = tracks(5).into_iter().filter(|t| t.id != "t3").collect();
        let (item, ms) = q.set_context_at(ctx_id("x"), changed, None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t0", 0));
        // A moved track is still found.
        q.set_context(ctx_id("z"), tracks(5), Some(4), None);
        q.note_position(1_000);
        let mut moved = tracks(5);
        moved.rotate_right(1); // t4 is now first
        let (item, ms) = q.set_context_at(ctx_id("z"), moved, None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t4", 1_000));
    }

    #[test]
    fn shuffle_on_starts_on_a_random_track_not_the_resume_point() {
        let mut starts = HashSet::new();
        for seed in 0..30 {
            let mut q = Queue::with_seed(seed);
            q.set_context(ctx_id("x"), tracks(10), Some(5), Some(false));
            q.note_position(30_000);
            let (item, ms) = q.set_context_at(ctx_id("x"), tracks(10), None, Some(true)).unwrap();
            assert_eq!(ms, 0);
            starts.insert(item.track.id);
        }
        assert!(starts.len() > 3, "{starts:?}");
    }

    const MIN: i64 = 60_000;
    const T0: i64 = 1_800_000_000_000;

    fn kctx(kind: ContextKind, id: &str) -> ContextInfo {
        ContextInfo {
            kind,
            id: Some(id.into()),
            title: None,
        }
    }

    /// Leaves album x on t2 at 42 s at time T0, then plays y.
    fn left_at_t2() -> Queue {
        let mut q = Queue::with_seed(1);
        q.set_clock(Some(T0));
        q.set_context(ctx_id("x"), tracks(5), Some(2), None);
        q.note_position(42_000);
        q.set_context(ctx_id("y"), tracks(3), None, None);
        q
    }

    #[test]
    fn a_point_resumes_within_the_hour_and_expires_after_it() {
        let mut q = left_at_t2();
        q.set_clock(Some(T0 + 59 * MIN));
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(5), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t2", 42_000));

        let mut q = left_at_t2();
        q.set_clock(Some(T0 + 61 * MIN));
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(5), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t0", 0));
    }

    #[test]
    fn the_hour_counts_from_the_last_listen_not_the_first() {
        let mut q = Queue::with_seed(1);
        q.set_clock(Some(T0));
        q.set_context(ctx_id("x"), tracks(5), Some(2), None);
        q.note_position(1_000);
        q.set_clock(Some(T0 + 50 * MIN));
        q.note_position(2_000);
        // Paused there: saving the same point again doesn't refresh it.
        q.set_clock(Some(T0 + 55 * MIN));
        q.note_position(2_000);
        q.set_context(ctx_id("y"), tracks(3), None, None);
        q.set_clock(Some(T0 + 50 * MIN + 59 * MIN));
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(5), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t2", 2_000));
        assert_eq!(q.resume_point("x").unwrap().listened_at_ms, Some(T0 + 50 * MIN));
    }

    #[test]
    fn points_saved_without_a_time_count_as_expired() {
        let q = left_at_t2();
        let mut v = serde_json::to_value(&q).unwrap();
        for entry in v["resume"].as_array_mut().unwrap() {
            entry[1].as_object_mut().unwrap().remove("listenedAtMs");
        }
        let mut old: Queue = serde_json::from_value(v).unwrap();
        let rp = old.resume_point("x").unwrap();
        assert_eq!((rp.track_id.as_str(), rp.position_ms, rp.listened_at_ms), ("t2", 42_000, None));
        let (item, ms) = old.set_context_at(ctx_id("x"), tracks(5), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t0", 0));
    }

    #[test]
    fn pausing_near_the_end_still_resumes() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx_id("x"), tracks(3), Some(2), None);
        // 20 s before the end of the last song.
        q.note_position(160_000);
        q.set_context(ctx_id("y"), tracks(3), None, None);
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(3), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t2", 160_000));
    }

    #[test]
    fn playing_to_the_end_starts_over_next_time() {
        // Repeat off, nothing after: the queue rewinds.
        let mut q = Queue::with_seed(1);
        q.set_context(ctx_id("x"), tracks(3), Some(2), None);
        q.note_position(160_000);
        assert!(q.advance_auto().is_none());
        assert!(q.resume_point("x").is_none(), "cleared when the last song finished");
        q.rewind();
        q.note_position(0);
        q.set_context(ctx_id("y"), tracks(3), None, None);
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(3), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t0", 0));

        // Autoplay carries on with other songs: those don't become the point.
        let mut q = Queue::with_seed(1);
        q.set_context(ctx_id("x"), tracks(3), Some(2), None);
        q.note_position(160_000);
        q.extend_context(vec![track("other", "B", "/p")], ItemSource::Autoplay);
        let next = q.advance_auto().unwrap();
        assert_eq!(next.source, ItemSource::Autoplay);
        q.note_position(30_000);
        assert!(q.resume_point("x").is_none());
        q.set_context(ctx_id("y"), tracks(3), None, None);
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(3), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t0", 0));
    }

    #[test]
    fn repeat_all_keeps_the_point_going_round() {
        let mut q = Queue::with_seed(1);
        q.set_repeat(Repeat::All);
        q.set_context(ctx_id("x"), tracks(3), Some(2), None);
        q.note_position(1_000);
        let next = q.advance_auto().unwrap();
        assert_eq!(next.track.id, "t0");
        q.note_position(5_000);
        let rp = q.resume_point("x").unwrap();
        assert_eq!((rp.track_id.as_str(), rp.position_ms), ("t0", 5_000));
    }

    #[test]
    fn shuffle_on_leaves_the_unshuffled_point_alone() {
        let mut q = Queue::with_seed(3);
        q.set_context(ctx_id("x"), tracks(10), Some(5), Some(false));
        q.note_position(135_000);
        // Two shuffled plays, listened to and skipped through.
        for _ in 0..2 {
            q.set_context_at(ctx_id("x"), tracks(10), None, Some(true)).unwrap();
            q.note_position(20_000);
            q.advance_auto();
            q.note_position(4_000);
            q.set_context(ctx_id("y"), tracks(3), None, None);
        }
        // Nor does a shuffled play that runs to its end.
        q.set_context_at(ctx_id("x"), tracks(10), None, Some(true)).unwrap();
        while q.advance_auto().is_some() {}
        q.rewind();
        q.set_context(ctx_id("y"), tracks(3), None, None);
        // A double-click with shuffle on doesn't move it either.
        q.set_context_at(ctx_id("x"), tracks(10), Some(8), Some(true)).unwrap();
        q.note_position(1_000);
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(10), None, Some(false)).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t5", 135_000));
    }

    #[test]
    fn a_double_clicked_song_becomes_the_point() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx_id("x"), tracks(10), Some(2), None);
        q.note_position(85_000);
        q.set_context(ctx_id("y"), tracks(3), None, None);
        // Double-click song 7 on x's page; something else plays before it's heard.
        q.set_context(ctx_id("x"), tracks(10), Some(6), None);
        q.set_context(ctx_id("y"), tracks(3), None, None);
        let (item, ms) = q.set_context_at(ctx_id("x"), tracks(10), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t6", 0));
    }

    #[test]
    fn artist_play_never_resumes() {
        let mut q = Queue::with_seed(1);
        q.set_context(kctx(ContextKind::Artist, "a"), tracks(5), Some(2), None);
        q.note_position(60_000);
        assert!(q.resume_point("a").is_none(), "artists keep no point");
        q.set_context(ctx_id("y"), tracks(3), None, None);
        let (item, ms) = q.set_context_at(kctx(ContextKind::Artist, "a"), tracks(5), None, None).unwrap();
        assert_eq!((item.track.id.as_str(), ms), ("t0", 0));
    }

    #[test]
    fn playlists_liked_songs_and_mixes_resume_like_albums() {
        for kind in [ContextKind::Playlist, ContextKind::Liked, ContextKind::Mix] {
            let mut q = Queue::with_seed(1);
            q.set_context(kctx(kind, "p"), tracks(5), Some(3), None);
            q.note_position(7_000);
            q.set_context(ctx_id("y"), tracks(3), None, None);
            let (item, ms) = q.set_context_at(kctx(kind, "p"), tracks(5), None, None).unwrap();
            assert_eq!((item.track.id.as_str(), ms), ("t3", 7_000), "{kind:?}");
        }
        for kind in [ContextKind::Radio, ContextKind::Search, ContextKind::Tracks] {
            assert!(!resumable(kind), "{kind:?}");
        }
    }

    #[test]
    fn context_memory_is_capped_dropping_the_least_recent() {
        let mut q = Queue::with_seed(1);
        for i in 0..CONTEXT_MEMORY_CAP + 5 {
            let id = format!("c{i}");
            q.set_context(ctx_id(&id), tracks(2), Some(1), Some(false));
            q.note_position(1);
            q.set_context_shuffle(&id, true);
        }
        assert!(q.resume_point("c0").is_none());
        assert!(q.resume_point("c4").is_none());
        assert!(q.resume_point("c5").is_some());
        assert!(!q.shuffle_pref("c0"));
        assert!(q.shuffle_pref("c5"));
        assert_eq!(q.shuffled_contexts().len(), CONTEXT_MEMORY_CAP);
    }

    #[test]
    fn shuffle_is_remembered_per_context() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx_id("x"), tracks(5), None, None);
        q.set_shuffle(true);
        assert!(q.shuffle_pref("x"));
        // Another album plays unshuffled.
        q.set_context(ctx_id("y"), tracks(5), None, None);
        assert!(!q.shuffle());
        // Back to x: still shuffled.
        q.set_context(ctx_id("x"), tracks(5), None, None);
        assert!(q.shuffle());
        // A preference set for a context that isn't playing.
        q.set_context_shuffle("y", true);
        assert!(q.shuffle(), "x is still playing, unchanged");
        q.set_context(ctx_id("y"), tracks(5), None, None);
        assert!(q.shuffle());
        // For the playing context it applies now.
        q.set_context_shuffle("y", false);
        assert!(!q.shuffle());
        assert!(!q.shuffle_pref("y"));
        assert_eq!(q.shuffled_contexts(), vec!["x".to_string()]);
    }

    #[test]
    fn a_shuffled_queue_from_before_per_context_shuffle_keeps_it() {
        let mut q = Queue::with_seed(3);
        q.set_context(ctx(), tracks(3), Some(0), Some(true));
        let id = ctx().id.unwrap();
        // An old queue file: shuffled, but no per-context record.
        q.shuffled.clear();
        q.adopt_legacy_shuffle();
        assert_eq!(q.shuffled_contexts(), vec![id]);
    }

    #[test]
    fn shuffle_prefs_and_resume_points_survive_serde() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx_id("x"), tracks(5), Some(2), Some(true));
        q.note_position(7_000);
        q.set_context(ctx_id("y"), tracks(5), Some(3), Some(false));
        q.note_position(8_000);
        let json = serde_json::to_string(&q).unwrap();
        let r: Queue = serde_json::from_str(&json).unwrap();
        assert!(r.shuffle_pref("x"));
        assert!(!r.shuffle_pref("y"));
        assert_eq!(r.resume_point("y").unwrap().position_ms, 8_000);
        assert_eq!(r.resume_point("y").unwrap().track_id, "t3");
        // Files written before these fields existed still load.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("resume");
        v.as_object_mut().unwrap().remove("shuffled");
        let old: Queue = serde_json::from_value(v).unwrap();
        assert!(old.resume_point("y").is_none());
    }

    /// Several artists, uneven counts, like a real playlist.
    fn mixed_tracks() -> Vec<Track> {
        let mut t = Vec::new();
        for (a, n) in [("A", 12), ("B", 8), ("C", 6), ("D", 4)] {
            for i in 0..n {
                t.push(track(&format!("{a}{i}"), a, "/p"));
            }
        }
        t
    }

    fn full_order(q: &Queue) -> Vec<QueueItem> {
        let mut o = vec![q.current().unwrap().clone()];
        o.extend(q.view().next_up);
        o
    }

    #[test]
    fn spread_is_the_default_style_and_standard_is_plain_random() {
        assert_eq!(Queue::new().shuffle_style(), ShuffleStyle::Spread);
        let t = mixed_tracks();
        let runs = 200;
        let (mut spread_adj, mut std_adj) = (0, 0);
        let mut first_counts: HashMap<String, usize> = HashMap::new();
        for seed in 0..runs {
            let mut q = Queue::with_seed(seed);
            q.set_context(ctx_id("x"), t.clone(), Some(0), Some(true));
            spread_adj += adjacent_same_artist(&full_order(&q));

            let mut q = Queue::with_seed(seed);
            q.set_shuffle_style(ShuffleStyle::Standard);
            q.set_context(ctx_id("x"), t.clone(), Some(0), Some(true));
            let o = full_order(&q);
            // A permutation with the start first.
            assert_eq!(o.len(), t.len());
            assert_eq!(o[0].track.id, "A0");
            let set: HashSet<_> = o.iter().map(|i| i.track.id.clone()).collect();
            assert_eq!(set.len(), t.len());
            std_adj += adjacent_same_artist(&o);
            *first_counts.entry(o[1].track.id.clone()).or_default() += 1;
        }
        let spread_avg = spread_adj as f64 / runs as f64;
        let std_avg = std_adj as f64 / runs as f64;
        // Plain random: about sum(n*(n-1))/N = (132+56+30+12)/30 = 7.7 pairs.
        assert!((6.0..9.5).contains(&std_avg), "standard {std_avg}");
        assert!(spread_avg < std_avg / 3.0, "spread {spread_avg} vs standard {std_avg}");
        // Standard isn't biased: many different tracks come up second.
        assert!(first_counts.len() > 20, "{}", first_counts.len());
    }

    #[test]
    fn standard_style_also_applies_when_toggling_shuffle_on() {
        let mut q = Queue::with_seed(2);
        q.set_shuffle_style(ShuffleStyle::Standard);
        q.set_context(ctx_id("x"), tracks(10), Some(3), Some(false));
        q.set_shuffle(true);
        let o = full_order(&q);
        assert_eq!(o[0].track.id, "t3");
        assert_eq!(o.len(), 10);
    }

    #[test]
    fn a_mixed_shuffle_puts_fitting_songs_next_to_each_other() {
        // Songs t0..t9 "fit" when their numbers are one apart.
        let num = |t: &Track| t.id[1..].parse::<i32>().unwrap();
        let score = |a: &Track, b: &Track| if (num(a) - num(b)).abs() == 1 { 1.0 } else { 0.0 };
        let mut q = Queue::with_seed(7);
        q.set_context(ctx(), tracks(10), None, Some(true));
        let before: Vec<String> = ids(&q.view().next_up);
        let fits = |q: &Queue| {
            let mut all = vec![cur(q)];
            all.extend(ids(&q.view().next_up));
            all.windows(2).filter(|w| (w[0][1..].parse::<i32>().unwrap() - w[1][1..].parse::<i32>().unwrap()).abs() == 1).count()
        };
        let was = fits(&q);
        q.reorder_for_fit(&score, 10);
        let after: Vec<String> = ids(&q.view().next_up);
        let mut a = before.clone();
        let mut b = after.clone();
        a.sort();
        b.sort();
        assert_eq!(a, b, "same songs");
        assert!(fits(&q) > was, "{before:?} -> {after:?}");
        // Unshuffled, the order is the playlist's.
        let mut q = Queue::with_seed(7);
        q.set_context(ctx(), tracks(5), Some(0), Some(false));
        q.reorder_for_fit(&|_, _| 0.0, 10);
        assert_eq!(ids(&q.view().next_up), vec!["t1", "t2", "t3", "t4"]);
    }
}
