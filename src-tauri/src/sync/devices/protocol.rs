//! The sync protocol between two of the user's own devices. The same on
//! every path: a message in gives the messages to answer with, so the local
//! network (a request and its answer) and the relay (messages each way)
//! carry it alike.
//!
//! A round, started by either end:
//!
//! 1. A [`SyncHello`] with a digest per namespace this end keeps in step.
//! 2. The other end answers with its own hello (marked as the answer, so it
//!    isn't answered again) and, for each namespace whose digest differs,
//!    its key list (`SyncDiff`: keys, times and devices, no values).
//! 3. The first end works out what the other lacks or has older and sends
//!    those entries (`SyncEntries`), and asks for the rest (`SyncWant`).
//! 4. Both stores merge. Merging is idempotent, so a repeated or reordered
//!    message does nothing.
//!
//! Between rounds, local changes go out as they happen (`push`), sent only
//! when the [`Outbox`] says the other end doesn't have them yet. Catching
//! up never depends on those arriving: the next round compares state.
//!
//! Only devices of the same Plex account: a hello whose account tag isn't
//! this device's is refused and nothing is sent to that link, and nothing
//! but a hello is taken from a link until a hello has been checked.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;

use super::clock;
use crate::db::DbHandle;
use crate::homeside::link::Link;
use crate::homeside::links::{Links, Outbox, batches};
use crate::homeside::wire::{Msg, PROTO, SyncHello};
use crate::sync::SyncStore;
use crate::sync::engine::{AccountTag, Groups};
use crate::sync::store::{EntryMap, NAMESPACES, SETTINGS, diff_keys, looks_secret};

/// Most keys asked for in one message.
const WANT_CHUNK: usize = 2000;

/// What the protocol needs from the app.
#[derive(Clone)]
pub struct SyncDeps {
    pub store: SyncStore,
    pub db: DbHandle,
    /// Device links (`Links::devices`).
    pub links: Links,
    pub account: Arc<dyn AccountTag>,
    /// What this device keeps in step (Settings > Sync).
    pub groups: Arc<dyn Fn() -> Groups + Send + Sync>,
    /// This device's name as other devices show it.
    pub name: Arc<dyn Fn() -> String + Send + Sync>,
    pub platform: String,
    pub version: String,
}

/// Why a link is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Paused {
    /// The other device is signed in to another Plex account or Home user.
    OtherAccount,
    /// This device isn't signed in.
    SignedOut,
}

/// What this run has heard from the other end of a link.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerSeen {
    pub name: String,
    pub platform: String,
    /// The protocol it speaks.
    pub proto: u32,
    /// How far its clock is ahead of this one's (ms; negative: behind).
    pub clock_offset_ms: Option<i64>,
    pub paused: Option<Paused>,
    /// Its account was checked (a hello, or a signed request on the local
    /// network).
    pub verified: bool,
    /// Namespaces it has switched off.
    pub skip: Vec<String>,
    /// Entries from it held back for a clock far ahead.
    pub held: usize,
    /// Its last changes couldn't be saved here: not in step until they are.
    pub save_failed: bool,
    /// Which entries `held` counts.
    #[serde(skip)]
    pub held_keys: std::collections::BTreeSet<String>,
}

/// A settings copy asked for on a link.
#[derive(Debug, Clone, PartialEq)]
pub enum Copying {
    Waiting,
    Done,
    Failed(String),
}

pub struct SyncPeer {
    d: SyncDeps,
    seen: Mutex<HashMap<String, PeerSeen>>,
    /// Links whose synced settings are being copied here ("Copy settings
    /// from my computer"): their answer to the copy wins as new edits.
    adopting: Mutex<HashMap<String, Copying>>,
    /// Messages that came before the hello they follow, by link.
    waiting: Mutex<HashMap<String, Vec<Msg>>>,
}

/// Most messages kept per link while its hello hasn't come.
const WAITING_MAX: usize = 32;

fn group_names(g: &Groups) -> Vec<String> {
    [
        ("settings", g.settings),
        ("library", g.library),
        ("follows", g.follows),
        ("lyrics", g.lyrics),
        ("diary", g.diary),
        ("looks", g.looks),
        ("downloads", g.downloads),
        ("plays", g.plays),
    ]
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(n, _)| n.to_string())
        .collect()
}

/// Leave out anything that looks like a secret (checked again on arrival).
fn clean(mut m: EntryMap) -> EntryMap {
    for entries in m.values_mut() {
        entries.retain(|_, e| !e.value.as_ref().is_some_and(looks_secret));
    }
    m.retain(|_, e| !e.is_empty());
    m
}

fn entries_msgs(m: EntryMap) -> Vec<Msg> {
    batches(clean(m)).into_iter().map(|entries| Msg::SyncEntries { entries }).collect()
}

impl SyncPeer {
    pub fn new(d: SyncDeps) -> Self {
        Self { d, seen: Mutex::default(), waiting: Mutex::default(), adopting: Mutex::default() }
    }

    pub fn deps(&self) -> &SyncDeps {
        &self.d
    }

    /// What this run has heard from a link.
    pub fn seen(&self, link_id: &str) -> Option<PeerSeen> {
        self.seen.lock().get(link_id).cloned()
    }

    fn set_seen(&self, link_id: &str, f: impl FnOnce(&mut PeerSeen)) {
        f(self.seen.lock().entry(link_id.to_string()).or_default());
    }

    /// Whether messages other than a hello are taken from this link.
    pub fn verified(&self, link_id: &str) -> bool {
        self.seen(link_id).is_some_and(|s| s.verified && s.paused.is_none())
    }

    /// The local network checked the sender's account (a signed request).
    pub fn verified_on_lan(&self, link_id: &str) {
        self.set_seen(link_id, |s| {
            if s.paused != Some(Paused::OtherAccount) {
                s.verified = true;
                s.paused = None;
            }
        });
    }

    /// The namespaces this device keeps in step: its groups', and any a
    /// newer build wrote that this one keeps and passes on.
    pub fn namespaces(&self) -> Vec<String> {
        let mut out: Vec<String> = (self.d.groups)().namespaces().into_iter().map(str::to_string).collect();
        out.extend(self.d.store.unknown_namespaces().unwrap_or_default());
        out
    }

    /// Namespaces this build knows but has switched off.
    fn skipped(&self) -> Vec<String> {
        let on = (self.d.groups)().namespaces();
        NAMESPACES.iter().filter(|n| !on.contains(n)).map(|n| n.to_string()).collect()
    }

    /// Whether entries in `ns` move on this link: this device keeps it (or
    /// doesn't know it, and keeps it for a newer build) and the other end
    /// hasn't switched it off.
    fn moves(&self, link_id: &str, ns: &str) -> bool {
        let here = !NAMESPACES.contains(&ns) || (self.d.groups)().namespaces().contains(&ns);
        here && !self.seen(link_id).is_some_and(|s| s.skip.iter().any(|x| x == ns))
    }

    /// This device's hello, or None when it isn't signed in (nothing is
    /// sent then).
    pub async fn hello(&self, answer: bool) -> Option<Msg> {
        let acct = self.d.account.tag().await?;
        if let Err(e) = self.d.store.adopt_learned() {
            tracing::debug!("device sync: {e}");
        }
        let ns = self.namespaces();
        let refs: Vec<&str> = ns.iter().map(String::as_str).collect();
        let digests = self.d.store.digest(&refs).ok()?;
        Some(Msg::SyncHello(Box::new(SyncHello {
            proto: PROTO,
            device: self.d.store.device().to_string(),
            name: (self.d.name)(),
            platform: self.d.platform.clone(),
            version: self.d.version.clone(),
            clock_ms: self.d.store.now(),
            groups: group_names(&(self.d.groups)()),
            acct,
            digests,
            skip: self.skipped(),
            answer,
        })))
    }

    /// One message from the other end of `link`: the messages to answer
    /// with.
    pub async fn handle(self: &Arc<Self>, link: &Link, msg: Msg) -> Vec<Msg> {
        tracing::debug!(kind = msg.kind(), "device sync: in");
        match msg {
            Msg::SyncHello(h) => self.on_hello(link, *h).await,
            Msg::SyncBye => {
                tracing::info!("A device unpaired itself.");
                if let Err(e) = self.d.links.remove(&link.id) {
                    tracing::warn!("device sync: couldn't forget the device: {e}");
                }
                self.forget_seen(&link.id);
                Vec::new()
            }
            m @ (Msg::SyncDiff { .. } | Msg::SyncWant { .. } | Msg::SyncEntries { .. } | Msg::SyncCopyWant { .. } | Msg::SyncCopy { .. })
                if !self.verified(&link.id) =>
            {
                // Arrived ahead of the hello it follows: kept (a few) until
                // the hello is checked, never used before.
                if self.seen(&link.id).is_some_and(|s| s.paused == Some(Paused::OtherAccount)) {
                    return Vec::new();
                }
                let mut w = self.waiting.lock();
                let q = w.entry(link.id.clone()).or_default();
                if q.len() < WAITING_MAX {
                    q.push(m);
                } else {
                    tracing::debug!("device sync: dropped a message from a link not checked yet");
                }
                Vec::new()
            }
            m => {
                let link = link.clone();
                self.blocking(move |p| p.on_checked(&link, m)).await
            }
        }
    }

    /// Store work off the async threads.
    pub(crate) async fn blocking<T: Default + Send + 'static>(self: &Arc<Self>, f: impl FnOnce(&SyncPeer) -> T + Send + 'static) -> T {
        let me = self.clone();
        match tokio::task::spawn_blocking(move || f(&me)).await {
            Ok(v) => v,
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(e) => {
                tracing::warn!("device sync: store work stopped: {e}");
                T::default()
            }
        }
    }

    /// A message from a link whose account was checked.
    fn on_checked(&self, link: &Link, msg: Msg) -> Vec<Msg> {
        match msg {
            Msg::SyncDiff { ns, keys } => self.on_diff(link, &ns, &keys),
            Msg::SyncWant { ns, keys } => self.on_want(link, &ns, &keys),
            Msg::SyncEntries { entries } => self.on_entries(link, &entries),
            Msg::SyncCopyWant { ns, keys } => self.on_copy_want(link, &ns, &keys),
            Msg::SyncCopy { entries } => self.on_copy(link, entries),
            // Bridge messages and kinds from a newer build.
            _ => Vec::new(),
        }
    }

    async fn on_hello(self: &Arc<Self>, link: &Link, h: SyncHello) -> Vec<Msg> {
        let Some(mine) = self.d.account.tag().await else {
            self.set_seen(&link.id, |s| {
                s.paused = Some(Paused::SignedOut);
                s.verified = false;
            });
            return Vec::new();
        };
        if h.acct != mine {
            tracing::info!("device sync: a device of another account was refused");
            self.set_seen(&link.id, |s| {
                s.paused = Some(Paused::OtherAccount);
                s.verified = false;
            });
            self.waiting.lock().remove(&link.id);
            return Vec::new();
        }
        let offset = clock::offset(h.clock_ms, self.d.store.now());
        let name: String = h.name.chars().take(60).collect();
        self.set_seen(&link.id, |s| {
            s.name = name.clone();
            s.platform = h.platform.chars().take(30).collect();
            s.proto = h.proto;
            s.clock_offset_ms = Some(offset);
            s.paused = None;
            s.verified = true;
            s.skip = h.skip.clone();
        });
        let device: String = h.device.chars().take(100).collect();
        if let Err(e) = self.d.links.update(&link.id, |p| {
            p.name = name;
            p.version = h.version.chars().take(20).collect();
            p.device = Some(device);
        }) {
            tracing::warn!("device sync: couldn't save what a device said about itself: {e}");
        }
        // This end's namespaces, and ones the other keeps that this build
        // doesn't know (kept and passed on).
        let mut ns = self.namespaces();
        for k in h.digests.keys().take(NAMESPACES.len() + crate::sync::store::UNKNOWN_NS_MAX) {
            if !NAMESPACES.contains(&k.as_str()) && !ns.contains(k) && crate::sync::store::plausible_ns(k) {
                ns.push(k.clone());
            }
        }
        let (skip, digests, id) = (h.skip, h.digests, link.id.clone());
        let mut diffs = self
            .blocking(move |p| {
                let mut diffs = Vec::new();
                let mut ok = true;
                for n in ns {
                    if skip.contains(&n) {
                        continue;
                    }
                    let stamps = match p.d.store.stamps(&n) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(ns = n, "device sync: couldn't read this device's entries: {e}");
                            ok = false;
                            continue;
                        }
                    };
                    let ours = crate::sync::store::digest_of(&stamps);
                    match digests.get(&n) {
                        Some(theirs) if *theirs == ours => {}
                        None if ours.n == 0 => {}
                        _ => diffs.push(Msg::SyncDiff { ns: n, keys: stamps }),
                    }
                }
                if diffs.is_empty() && ok {
                    p.mark_synced(&id);
                }
                diffs
            })
            .await;
        let mut out: Vec<Msg> = Vec::new();
        if !h.answer {
            out.extend(self.hello(true).await);
            out.append(&mut diffs);
        }
        // Anything that came ahead of this hello.
        let early = self.waiting.lock().remove(&link.id).unwrap_or_default();
        if !early.is_empty() {
            let link = link.clone();
            out.extend(self.blocking(move |p| early.into_iter().flat_map(|m| p.on_checked(&link, m)).collect::<Vec<_>>()).await);
        }
        out
    }

    fn on_diff(&self, link: &Link, ns: &str, theirs: &[crate::sync::store::Stamp]) -> Vec<Msg> {
        if !self.moves(&link.id, ns) {
            return Vec::new();
        }
        let mine = match self.d.store.stamps(ns) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(ns, "device sync: couldn't read this device's entries: {e}");
                return Vec::new();
            }
        };
        // What it has, so it isn't sent again.
        if let Err(e) = Outbox::new(&self.d.db, &link.id).mark_stamps(ns, theirs) {
            tracing::warn!("device sync: couldn't note what the other device has: {e}");
        }
        let d = diff_keys(&mine, theirs);
        let mut ok = true;
        let mut out = match self.d.store.entries_for(ns, &d.send) {
            Ok(m) => entries_msgs(m),
            Err(e) => {
                // Not in step: the next round tries again.
                tracing::warn!(ns, "device sync: couldn't read entries to send: {e}");
                ok = false;
                Vec::new()
            }
        };
        for chunk in d.want.chunks(WANT_CHUNK) {
            out.push(Msg::SyncWant { ns: ns.to_string(), keys: chunk.to_vec() });
        }
        if out.is_empty() && ok {
            self.mark_synced(&link.id);
        }
        out
    }

    fn on_want(&self, link: &Link, ns: &str, keys: &[String]) -> Vec<Msg> {
        if !self.moves(&link.id, ns) {
            return Vec::new();
        }
        match self.d.store.entries_for(ns, keys) {
            Ok(m) => entries_msgs(m),
            Err(e) => {
                tracing::warn!(ns, "device sync: couldn't read entries it asked for: {e}");
                Vec::new()
            }
        }
    }

    /// Ask the other end of `link` for its synced settings, to take them
    /// as they are there.
    pub fn copy_settings(&self, link: &Link) -> Msg {
        self.adopting.lock().insert(link.id.clone(), Copying::Waiting);
        Msg::SyncCopyWant { ns: SETTINGS.name().to_string(), keys: crate::sync::settings::store_keys() }
    }

    /// Where a settings copy from `link` stands (None: none asked for).
    pub fn copying(&self, link_id: &str) -> Option<Copying> {
        self.adopting.lock().get(link_id).cloned()
    }

    /// Done waiting for a settings copy (a late answer is dropped).
    pub fn stop_copying(&self, link_id: &str) {
        self.adopting.lock().remove(link_id);
    }

    /// The other device asked for this one's settings as they are. Always
    /// answered, empty when there's nothing to give, so it isn't left
    /// waiting.
    fn on_copy_want(&self, link: &Link, ns: &str, keys: &[String]) -> Vec<Msg> {
        if ns != SETTINGS.name() || !self.moves(&link.id, ns) {
            return vec![Msg::SyncCopy { entries: EntryMap::new() }];
        }
        let keys: Vec<String> = keys.iter().take(WANT_CHUNK).cloned().collect();
        match self.d.store.entries_for(ns, &keys) {
            Ok(m) => vec![Msg::SyncCopy { entries: clean(m) }],
            Err(e) => {
                tracing::warn!("device sync: couldn't read settings to copy: {e}");
                vec![Msg::SyncCopy { entries: EntryMap::new() }]
            }
        }
    }

    /// The answer to "Copy settings from my computer": what the other
    /// device has wins here, and from here everywhere, as if just chosen.
    fn on_copy(&self, link: &Link, mut entries: EntryMap) -> Vec<Msg> {
        if self.copying(&link.id) != Some(Copying::Waiting) {
            // Not asked for (or given up on): never taken as a copy.
            return Vec::new();
        }
        let name = self.seen(&link.id).map(|s| s.name).filter(|n| !n.is_empty()).unwrap_or_else(|| "The other device".into());
        let done = |r: Copying| {
            self.adopting.lock().insert(link.id.clone(), r);
        };
        entries.retain(|n, _| n == SETTINGS.name());
        let Some(s) = entries.get_mut(SETTINGS.name()).filter(|s| s.values().any(|e| e.value.is_some())) else {
            done(Copying::Failed(format!("{name} has no synced settings to copy.")));
            return Vec::new();
        };
        // Later than anything here, even an edit made this millisecond.
        let now = self.d.store.now();
        let here: HashMap<String, u64> = match self.d.store.stamps(SETTINGS.name()) {
            Ok(st) => st.into_iter().map(|(k, ts, _)| (k, ts)).collect(),
            Err(e) => {
                tracing::warn!("device sync: couldn't read settings here: {e}");
                done(Copying::Failed("Couldn't read the settings on this device.".into()));
                return Vec::new();
            }
        };
        for (k, e) in s.iter_mut().filter(|(_, e)| e.value.is_some()) {
            e.ts = e.ts.max(now).max(here.get(k).map_or(0, |t| t + 1));
        }
        match self.d.store.merge_from_device(&entries, &[SETTINGS.name()], &name) {
            Ok(_) => done(Copying::Done),
            Err(e) => {
                tracing::warn!("device sync: couldn't save copied settings: {e}");
                done(Copying::Failed(format!("Couldn't save the settings from {name}.")));
            }
        }
        Vec::new()
    }

    fn on_entries(&self, link: &Link, entries: &EntryMap) -> Vec<Msg> {
        let ns: Vec<String> = entries.keys().filter(|n| self.moves(&link.id, n)).cloned().collect();
        let refs: Vec<&str> = ns.iter().map(String::as_str).collect();
        let from = self.seen(&link.id).map(|s| s.name).unwrap_or_default();
        match self.d.store.merge_from_device(entries, &refs, &from) {
            Ok(out) => {
                if !out.held.is_empty() {
                    tracing::info!(held = out.held.len(), "device sync: held entries from a clock far ahead");
                }
                self.set_seen(&link.id, |s| {
                    // Counted once per entry, however often it comes again.
                    s.held_keys.extend(out.held.iter().map(|(ns, key, _)| format!("{ns}/{key}")));
                    s.held = s.held_keys.len();
                    s.save_failed = false;
                });
            }
            Err(e) => {
                // Not saved: not marked as had, and not in step, so the
                // next round sends them again.
                tracing::warn!("device sync: couldn't take entries: {e}");
                self.set_seen(&link.id, |s| s.save_failed = true);
                return Vec::new();
            }
        }
        // It has these now, whether or not they won here.
        if let Err(e) = Outbox::new(&self.d.db, &link.id).mark(entries) {
            tracing::warn!("device sync: couldn't note what the other device has: {e}");
        }
        self.mark_synced(&link.id);
        Vec::new()
    }

    /// The two are in step now (not while changes from it couldn't be saved).
    pub(crate) fn mark_synced(&self, link_id: &str) {
        if self.seen(link_id).is_some_and(|s| s.save_failed) {
            return;
        }
        let now = self.d.store.now();
        if let Err(e) = self.d.links.update(link_id, |p| p.last_synced_at = Some(now)) {
            tracing::warn!("device sync: couldn't note the time in step: {e}");
        }
    }

    /// Local changes the other end doesn't have yet. Nothing until its
    /// account is checked: then a hello goes instead.
    pub async fn push(self: &Arc<Self>, link: &Link) -> Vec<Msg> {
        if !self.verified(&link.id) {
            return self.hello(false).await.into_iter().collect();
        }
        let id = link.id.clone();
        self.blocking(move |p| {
            let ns: Vec<String> = p.namespaces().into_iter().filter(|n| p.moves(&id, n)).collect();
            let refs: Vec<&str> = ns.iter().map(String::as_str).collect();
            match Outbox::new(&p.d.db, &id).pending(&p.d.store, &refs) {
                Ok(b) => b.into_iter().map(|entries| Msg::SyncEntries { entries }).collect(),
                Err(e) => {
                    tracing::warn!("device sync: couldn't read changes to send: {e}");
                    Vec::new()
                }
            }
        })
        .await
    }

    /// This device isn't signed in: nothing goes to the link until it is.
    pub fn signed_out(&self, link_id: &str) {
        self.set_seen(link_id, |s| {
            if s.paused != Some(Paused::OtherAccount) {
                s.paused = Some(Paused::SignedOut);
                s.verified = false;
            }
        });
    }

    /// Several messages reached the other end: noted in one write.
    pub fn sent_all(&self, link: &Link, msgs: &[Msg]) {
        let mut all = EntryMap::new();
        for m in msgs {
            tracing::debug!(kind = m.kind(), "device sync: out");
            if let Msg::SyncEntries { entries } = m {
                for (ns, e) in entries {
                    all.entry(ns.clone()).or_default().extend(e.iter().map(|(k, v)| (k.clone(), v.clone())));
                }
            }
        }
        if !all.is_empty()
            && let Err(e) = Outbox::new(&self.d.db, &link.id).mark(&all)
        {
            tracing::warn!("device sync: couldn't note what was sent: {e}");
        }
    }

    /// A message reached the other end.
    pub fn sent(&self, link: &Link, msg: &Msg) {
        tracing::debug!(kind = msg.kind(), "device sync: out");
        if let Msg::SyncEntries { entries } = msg
            && let Err(e) = Outbox::new(&self.d.db, &link.id).mark(entries)
        {
            tracing::warn!("device sync: couldn't note what was sent: {e}");
        }
    }

    /// Unpair: the bye to send first, then forget the link here.
    pub fn forget(&self, link_id: &str) -> crate::error::AppResult<()> {
        self.forget_seen(link_id);
        self.d.links.remove(link_id)
    }

    /// Everything this run kept about a link.
    fn forget_seen(&self, link_id: &str) {
        self.seen.lock().remove(link_id);
        self.waiting.lock().remove(link_id);
        self.adopting.lock().remove(link_id);
    }

    /// Whether the other end keeps this namespace in step (from its hello).
    pub fn peer_skips(&self, link_id: &str, ns: &str) -> bool {
        self.seen(link_id).is_some_and(|s| s.skip.iter().any(|x| x == ns))
    }

    /// Whether this device keeps this group in step.
    pub fn keeps_settings(&self) -> bool {
        (self.d.groups)().settings
    }
}
