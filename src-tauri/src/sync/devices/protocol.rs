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
use crate::sync::store::{EntryMap, NAMESPACES, diff_keys, looks_secret};

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
}

pub struct SyncPeer {
    d: SyncDeps,
    seen: Mutex<HashMap<String, PeerSeen>>,
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
        Self { d, seen: Mutex::default(), waiting: Mutex::default() }
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
    pub async fn handle(&self, link: &Link, msg: Msg) -> Vec<Msg> {
        tracing::debug!(kind = msg.kind(), "device sync: in");
        match msg {
            Msg::SyncHello(h) => self.on_hello(link, *h).await,
            Msg::SyncBye => {
                tracing::info!("A device unpaired itself.");
                let _ = self.d.links.remove(&link.id);
                self.seen.lock().remove(&link.id);
                Vec::new()
            }
            m @ (Msg::SyncDiff { .. } | Msg::SyncWant { .. } | Msg::SyncEntries { .. }) if !self.verified(&link.id) => {
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
            m => self.on_checked(link, m),
        }
    }

    /// A message from a link whose account was checked.
    fn on_checked(&self, link: &Link, msg: Msg) -> Vec<Msg> {
        match msg {
            Msg::SyncDiff { ns, keys } => self.on_diff(link, &ns, &keys),
            Msg::SyncWant { ns, keys } => self.on_want(link, &ns, &keys),
            Msg::SyncEntries { entries } => self.on_entries(link, &entries),
            // Bridge messages and kinds from a newer build.
            _ => Vec::new(),
        }
    }

    async fn on_hello(&self, link: &Link, h: SyncHello) -> Vec<Msg> {
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
        let _ = self.d.links.update(&link.id, |p| {
            p.name = name;
            p.version = h.version.chars().take(20).collect();
            p.device = Some(device);
        });
        // This end's namespaces, and ones the other keeps that this build
        // doesn't know (kept and passed on).
        let mut ns = self.namespaces();
        for k in h.digests.keys() {
            if !NAMESPACES.contains(&k.as_str()) && !ns.contains(k) {
                ns.push(k.clone());
            }
        }
        let mut diffs = Vec::new();
        for n in ns {
            if h.skip.contains(&n) {
                continue;
            }
            let Ok(stamps) = self.d.store.stamps(&n) else { continue };
            let ours = crate::sync::store::digest_of(&stamps);
            match h.digests.get(&n) {
                Some(theirs) if *theirs == ours => {}
                None if ours.n == 0 => {}
                _ => diffs.push(Msg::SyncDiff { ns: n, keys: stamps }),
            }
        }
        if diffs.is_empty() {
            self.mark_synced(&link.id);
        }
        let mut out: Vec<Msg> = Vec::new();
        if !h.answer {
            out.extend(self.hello(true).await);
            out.append(&mut diffs);
        }
        // Anything that came ahead of this hello.
        let early = self.waiting.lock().remove(&link.id).unwrap_or_default();
        for m in early {
            out.extend(self.on_checked(link, m));
        }
        out
    }

    fn on_diff(&self, link: &Link, ns: &str, theirs: &[crate::sync::store::Stamp]) -> Vec<Msg> {
        if !self.moves(&link.id, ns) {
            return Vec::new();
        }
        let Ok(mine) = self.d.store.stamps(ns) else { return Vec::new() };
        // What it has, so it isn't sent again.
        let _ = Outbox::new(&self.d.db, &link.id).mark_stamps(ns, theirs);
        let d = diff_keys(&mine, theirs);
        let mut out = match self.d.store.entries_for(ns, &d.send) {
            Ok(m) => entries_msgs(m),
            Err(_) => Vec::new(),
        };
        for chunk in d.want.chunks(WANT_CHUNK) {
            out.push(Msg::SyncWant { ns: ns.to_string(), keys: chunk.to_vec() });
        }
        if out.is_empty() {
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
            Err(_) => Vec::new(),
        }
    }

    fn on_entries(&self, link: &Link, entries: &EntryMap) -> Vec<Msg> {
        let ns: Vec<String> = entries.keys().filter(|n| self.moves(&link.id, n)).cloned().collect();
        let refs: Vec<&str> = ns.iter().map(String::as_str).collect();
        let from = self.seen(&link.id).map(|s| s.name).unwrap_or_default();
        match self.d.store.merge_from_device(entries, &refs, &from) {
            Ok(out) => {
                if !out.held.is_empty() {
                    tracing::info!(held = out.held.len(), "device sync: held entries from a clock far ahead");
                    self.set_seen(&link.id, |s| s.held += out.held.len());
                }
            }
            Err(e) => tracing::warn!("device sync: couldn't take entries: {e}"),
        }
        // It has these now, whether or not they won here.
        let _ = Outbox::new(&self.d.db, &link.id).mark(entries);
        self.mark_synced(&link.id);
        Vec::new()
    }

    fn mark_synced(&self, link_id: &str) {
        let now = self.d.store.now();
        let _ = self.d.links.update(link_id, |p| p.last_synced_at = Some(now));
    }

    /// Local changes the other end doesn't have yet. Nothing until its
    /// account is checked: then a hello goes instead.
    pub async fn push(&self, link: &Link) -> Vec<Msg> {
        if !self.verified(&link.id) {
            return self.hello(false).await.into_iter().collect();
        }
        let ns: Vec<String> = self.namespaces().into_iter().filter(|n| self.moves(&link.id, n)).collect();
        let refs: Vec<&str> = ns.iter().map(String::as_str).collect();
        match Outbox::new(&self.d.db, &link.id).pending(&self.d.store, &refs) {
            Ok(b) => b.into_iter().map(|entries| Msg::SyncEntries { entries }).collect(),
            Err(e) => {
                tracing::debug!("device sync: {e}");
                Vec::new()
            }
        }
    }

    /// A message reached the other end.
    pub fn sent(&self, link: &Link, msg: &Msg) {
        tracing::debug!(kind = msg.kind(), "device sync: out");
        if let Msg::SyncEntries { entries } = msg {
            let _ = Outbox::new(&self.d.db, &link.id).mark(entries);
        }
    }

    /// Unpair: the bye to send first, then forget the link here.
    pub fn forget(&self, link_id: &str) -> crate::error::AppResult<()> {
        self.seen.lock().remove(link_id);
        self.d.links.remove(link_id)
    }
}
