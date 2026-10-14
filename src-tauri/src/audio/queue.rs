//! The play queue, pure logic.
//!
//! * A **context** (album, playlist, artist, tracks, search) supplies
//!   "Next up": the rest of the context after the current position.
//! * The **user queue** ("Play next" inserts at its front, "Add to queue"
//!   appends) always plays before the context continues.
//! * **Shuffle** reorders the context with artists spread apart and recently
//!   played tracks pushed back; the current track stays first. Turning it off
//!   restores context order from the current track.
//! * **Repeat** off / all / one. **Previous** steps back through history.
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
    /// `shuffle` overrides the shuffle mode when given. The user queue is
    /// kept. Returns the new current item.
    pub fn set_context(
        &mut self,
        info: ContextInfo,
        tracks: Vec<Track>,
        start: Option<usize>,
        shuffle: Option<bool>,
    ) -> Option<QueueItem> {
        if let Some(s) = shuffle {
            self.shuffle = s;
        }
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
        let start = match start {
            Some(s) if s < n => s,
            _ if self.shuffle => self.rng.0.random_range(0..n),
            _ => 0,
        };
        if self.shuffle {
            let others: Vec<usize> = (0..n).filter(|i| *i != start).collect();
            let mut order = vec![start];
            order.extend(self.spread(others));
            self.dedupe_adjacent(&mut order, 1);
            self.order = order;
            self.pos = 0;
        } else {
            self.order = (0..n).collect();
            self.pos = start as isize;
        }
        let item = self.items[start].clone();
        self.set_current(item.clone(), true);
        Some(item)
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
        let mut out: Vec<QueueItem> = self.user.iter().take(n).cloned().collect();
        let len = self.order.len();
        let mut p = self.pos + 1;
        let mut guard = 0;
        while out.len() < n && len > 0 && guard < len {
            if p as usize >= len {
                if self.repeat == Repeat::Off {
                    break;
                }
                p = 0;
            }
            out.push(self.items[self.order[p as usize]].clone());
            p += 1;
            guard += 1;
        }
        out
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
            match self.step(false) {
                Some(c) if c.item_id == item_id => return true,
                Some(_) => {}
                None => return false,
            }
        }
        false
    }

    /// User pressed next. `None` at the end of the queue (repeat off).
    pub fn next_manual(&mut self) -> Option<QueueItem> {
        self.step(true)
    }

    /// User pressed previous (and the position is ≤ 3 s; the controller
    /// handles the restart rule). Returns the new current, or `None` when
    /// there's no history.
    pub fn previous(&mut self) -> Option<QueueItem> {
        let h = self.history.pop_back()?;
        let old = self.current.take();
        let old_was_context = old.as_ref().is_some_and(|o| o.source == ItemSource::Context);
        if let Some(o) = old
            && o.source == ItemSource::User
        {
            self.user.push_front(o);
        }
        if let Some(k) = self
            .order
            .iter()
            .position(|&i| self.items[i].item_id == h.item_id)
        {
            self.pos = k as isize;
        } else if old_was_context {
            // h is a user/old-context entry; the old current plays after it.
            self.pos -= 1;
        }
        self.note_played(&h);
        self.current = Some(h.clone());
        Some(h)
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
    /// everything else queued; `End` appends.
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

    /// Clears the user queue only.
    pub fn clear_user(&mut self) {
        self.user.clear();
    }

    pub fn set_repeat(&mut self, r: Repeat) {
        self.repeat = r;
    }

    pub fn set_shuffle(&mut self, on: bool) {
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
            let spread = self.spread(others);
            let mut order: Vec<usize> = cur.into_iter().chain(spread).collect();
            self.dedupe_adjacent(&mut order, 1);
            self.order = order;
            self.pos = if cur.is_some() { 0 } else { -1 };
        } else {
            self.order = (0..n).filter(|i| live.contains(i)).collect();
            self.pos = match cur {
                Some(c) => self.order.iter().position(|i| *i == c).map_or(-1, |p| p as isize),
                None => -1,
            };
        }
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
        assert_eq!(q.next_manual().unwrap().track.id, "t1");
        q.set_repeat(Repeat::Off);
        assert!(q.next_manual().is_none());
        assert!(q.peek_auto().is_none());
    }

    #[test]
    fn previous_walks_back_through_history_and_then_forward_again() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(4), Some(0), Some(false));
        q.next_manual();
        q.next_manual();
        assert_eq!(cur(&q), "t2");
        assert_eq!(q.previous().unwrap().track.id, "t1");
        assert_eq!(q.previous().unwrap().track.id, "t0");
        assert!(q.previous().is_none(), "no more history");
        assert_eq!(q.next_manual().unwrap().track.id, "t1");
        assert_eq!(q.next_manual().unwrap().track.id, "t2");
    }

    #[test]
    fn previous_from_a_user_item_requeues_it() {
        let mut q = Queue::with_seed(1);
        q.set_context(ctx(), tracks(3), Some(0), Some(false));
        q.add(vec![track("u", "B", "/p")], AddAt::Next);
        assert_eq!(q.next_manual().unwrap().track.id, "u");
        assert_eq!(q.previous().unwrap().track.id, "t0");
        assert_eq!(ids(&q.view().user_queue), vec!["u"]);
        assert_eq!(q.next_manual().unwrap().track.id, "u");
        assert_eq!(q.next_manual().unwrap().track.id, "t1");
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
}
