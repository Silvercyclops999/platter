//! Where each end keeps its pairings: the link's secret in the keychain (on
//! a Bridge, its keys file), everything else in the local database. Also
//! the per-link state that must outlive a restart: the replay guard, the
//! last relay message read, the frame counter, and which shared entries the
//! other end already has.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::link::{Counter, Guard, Link, Side};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;
use crate::social::listen::seal;
use crate::sync::store::{EntryMap, LIDARR};
use crate::sync::SyncStore;

const LINKS_KEY: &str = "bridge.links";
/// Links between the user's own devices, kept apart from Bridge links so
/// nothing meant for a Bridge (its settings, keys and token) can ever go to
/// a device link.
const DEVICE_LINKS_KEY: &str = "devices.links";

/// A pairing as kept (no secret).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paired {
    pub id: String,
    pub relay: String,
    pub side: Side,
    pub me: String,
    /// The other end's name.
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// Unix seconds.
    pub paired_at: i64,
    /// The last relay message read (ntfy's `since`).
    #[serde(default)]
    pub since: Option<String>,
    /// The same on each backup relay, by its address.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub since_on: BTreeMap<String, String>,
    #[serde(default)]
    pub guard: Guard,
    #[serde(default)]
    pub counter: u64,
    /// The other end's id inside frames (device links).
    #[serde(default)]
    pub peer: String,
    /// The other device's id (its Plex client id), from its hello.
    #[serde(default)]
    pub device: Option<String>,
    /// When the two were last in step, by this device's clock (ms).
    #[serde(default)]
    pub last_synced_at: Option<u64>,
    /// The namespaces both kept in step then.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_step: Vec<String>,
    /// Back in touch after a long time apart, and not fully in step yet:
    /// when the two were last in step before that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catching_up_since: Option<u64>,
    /// The other device said it answers Lidarr requests for this one (its
    /// last hello), for Settings' list of home sides.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub offers_lidarr: bool,
    /// Linked by Plex account (`super::account`), not with a code.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
}

#[derive(Clone)]
pub struct Links {
    db: DbHandle,
    keys: Arc<dyn TokenStore>,
    list_key: &'static str,
    secret_prefix: &'static str,
    /// One change to the kept list at a time.
    lock: Arc<parking_lot::Mutex<()>>,
}

impl Links {
    /// A Bridge's or a desktop's Bridge links.
    pub fn new(db: DbHandle, keys: Arc<dyn TokenStore>) -> Self {
        Self { db, keys, list_key: LINKS_KEY, secret_prefix: "bridge-link-", lock: Arc::default() }
    }

    /// Links to the user's other devices.
    pub fn devices(db: DbHandle, keys: Arc<dyn TokenStore>) -> Self {
        Self { db, keys, list_key: DEVICE_LINKS_KEY, secret_prefix: "device-link-", lock: Arc::default() }
    }

    fn secret_account(&self, id: &str) -> String {
        format!("{}{id}", self.secret_prefix)
    }

    pub fn all(&self) -> Vec<Paired> {
        self.db.read(|c| kv_get(c, self.list_key)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save(&self, list: &[Paired]) -> AppResult<()> {
        let json = serde_json::to_string(list).map_err(|e| AppError::Internal(e.to_string()))?;
        self.db.write(|c| kv_set(c, self.list_key, &json))
    }

    pub fn get(&self, id: &str) -> Option<Paired> {
        self.all().into_iter().find(|p| p.id == id)
    }

    /// The link with its secret, when both are there.
    pub fn link(&self, id: &str) -> Option<Link> {
        let p = self.get(id)?;
        let secret = self.keys.get(&self.secret_account(id)).ok().flatten().and_then(|t| seal::key32(t.expose()))?;
        Some(Link { id: p.id, secret, relay: p.relay, side: p.side, me: p.me, peer: p.peer })
    }

    pub fn links(&self) -> Vec<Link> {
        self.all().iter().filter_map(|p| self.link(&p.id)).collect()
    }

    pub fn add(&self, link: &Link, name: &str, version: &str, now_s: i64) -> AppResult<()> {
        self.keys.set(&self.secret_account(&link.id), &Token::new(seal::b64(&link.secret))?)?;
        let _one = self.lock.lock();
        let mut list = self.all();
        list.retain(|p| p.id != link.id);
        list.push(Paired {
            id: link.id.clone(),
            relay: link.relay.clone(),
            side: link.side,
            me: link.me.clone(),
            name: name.to_string(),
            version: version.to_string(),
            paired_at: now_s,
            since: None,
            since_on: BTreeMap::new(),
            guard: Guard::default(),
            counter: 0,
            peer: link.peer.clone(),
            device: None,
            last_synced_at: None,
            in_step: Vec::new(),
            catching_up_since: None,
            offers_lidarr: false,
            auto: false,
        });
        self.save(&list)
    }

    /// A link by Plex account. One that was here before (its device left
    /// the list for a while) carries on with its replay guard, so nothing
    /// it already took is taken again. It reads its inbox from a little
    /// before now, for what the other end sent before this one linked.
    pub fn add_auto(&self, w: &super::account::Wanted, now_s: i64) -> AppResult<()> {
        let kept: Option<Guard> = self.db.read(|c| kv_get(c, &self.kept_key(&w.link.id))).ok().flatten().and_then(|s| serde_json::from_str(&s).ok());
        self.add(&w.link, &w.name, "", now_s)?;
        let since = (now_s - super::account::BACKLOG_S).max(0).to_string();
        self.update(&w.link.id, |p| {
            p.auto = true;
            p.device = Some(w.device.clone());
            p.since = Some(since);
            if let Some(g) = kept {
                p.guard = g;
            }
        })
    }

    fn kept_key(&self, id: &str) -> String {
        format!("{}.kept.{id}", self.list_key)
    }

    fn apart_key(&self) -> String {
        format!("{}.apart", self.list_key)
    }

    /// Account links the user unpaired: not made again. A link's id comes
    /// from both ends' sign-ins, so signing in again on either links anew.
    pub fn apart(&self) -> Vec<String> {
        self.db.read(|c| kv_get(c, &self.apart_key())).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Keep this account link from coming back (it was unpaired).
    pub fn keep_apart(&self, id: &str) -> AppResult<()> {
        let _one = self.lock.lock();
        let mut l = self.apart();
        if !l.iter().any(|d| d == id) {
            l.push(id.to_string());
            l.truncate(500);
            let json = serde_json::to_string(&l).map_err(|e| AppError::Internal(e.to_string()))?;
            self.db.write(|c| kv_set(c, &self.apart_key(), &json))?;
        }
        Ok(())
    }

    /// The user unpaired this link: an account link stays apart.
    pub fn unpaired(&self, id: &str) {
        if self.get(id).is_some_and(|p| p.auto)
            && let Err(e) = self.keep_apart(id)
        {
            tracing::warn!("couldn't keep an unpaired device apart: {e}");
        }
    }

    pub fn remove(&self, id: &str) -> AppResult<()> {
        let _ = self.keys.delete(&self.secret_account(id));
        let _one = self.lock.lock();
        let mut list = self.all();
        if let Some(p) = list.iter().find(|p| p.id == id && p.auto)
            && let Ok(json) = serde_json::to_string(&p.guard)
        {
            // An account link can come back with the same id: its guard stays.
            self.db.write(|c| kv_set(c, &self.kept_key(id), &json))?;
        }
        list.retain(|p| p.id != id);
        self.save(&list)?;
        self.db.write(|c| {
            c.execute("DELETE FROM kv WHERE key = ?1", [sent_key(id)])?;
            Ok(())
        })
    }

    /// Change one pairing's kept state.
    pub fn update(&self, id: &str, f: impl FnOnce(&mut Paired)) -> AppResult<()> {
        let _one = self.lock.lock();
        let mut list = self.all();
        if let Some(p) = list.iter_mut().find(|p| p.id == id) {
            f(p);
        }
        self.save(&list)
    }

    /// Check frames against the link's replay guard and record them, all
    /// in one step under the list's lock (two at once can't both pass the
    /// same counter). All or nothing: false when any is a replay or the
    /// link is gone.
    pub fn accept(&self, id: &str, frames: &[&super::link::Frame]) -> bool {
        let _one = self.lock.lock();
        let mut list = self.all();
        let Some(p) = list.iter_mut().find(|p| p.id == id) else { return false };
        let mut guard = p.guard.clone();
        if frames.iter().any(|f| guard.accept(f).is_err()) {
            return false;
        }
        p.guard = guard;
        match self.save(&list) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("link replay guard not saved: {e}");
                false
            }
        }
    }

    /// The counter for frames sent on this link, carried on from the last run.
    pub fn counter(&self, id: &str) -> Counter {
        Counter::starting_at(self.get(id).map_or(0, |p| p.counter))
    }
}

// ---------------------------------------------------------------- shared entries

fn sent_key(id: &str) -> String {
    format!("bridge.sent.{id}")
}

/// Which shared entries the other end of a link already has: the entry's
/// time and device, by namespace and key. What's missing or different is
/// sent. Only a shortcut: a lost outbox means some entries go again, and
/// the digest round between devices repairs anything it got wrong.
pub struct Outbox<'a> {
    db: &'a DbHandle,
    id: &'a str,
}

/// Most entries in one message.
pub const BATCH: usize = 250;
/// Most JSON in one message's entries, so a batch stays well inside one
/// relay message and one request on the local network.
pub const BATCH_BYTES: usize = 160 * 1024;

/// Namespace, then key, to the stamp the other end has.
type Sent = BTreeMap<String, BTreeMap<String, (u64, String)>>;

/// Entries cut into batches of at most [`BATCH`] entries and about
/// [`BATCH_BYTES`] of JSON.
pub fn batches(all: EntryMap) -> Vec<EntryMap> {
    let mut out: Vec<EntryMap> = Vec::new();
    let mut cur = EntryMap::new();
    let (mut n, mut bytes) = (0, 0);
    for (ns, entries) in all {
        for (k, e) in entries {
            let size = k.len() + e.device.len() + e.value.as_ref().map_or(0, |v| v.to_string().len()) + 32;
            if n > 0 && (n == BATCH || bytes + size > BATCH_BYTES) {
                out.push(std::mem::take(&mut cur));
                (n, bytes) = (0, 0);
            }
            cur.entry(ns.clone()).or_default().insert(k, e);
            n += 1;
            bytes += size;
        }
    }
    if n > 0 {
        out.push(cur);
    }
    out
}

impl<'a> Outbox<'a> {
    pub fn new(db: &'a DbHandle, id: &'a str) -> Self {
        Self { db, id }
    }

    fn load(&self) -> Sent {
        let Some(s) = self.db.read(|c| kv_get(c, &sent_key(self.id))).ok().flatten() else { return Sent::new() };
        if let Ok(sent) = serde_json::from_str::<Sent>(&s) {
            return sent;
        }
        // Before outboxes knew namespaces they held only Lidarr's keys.
        serde_json::from_str::<BTreeMap<String, (u64, String)>>(&s)
            .map(|old| Sent::from([(LIDARR.name().to_string(), old)]))
            .unwrap_or_default()
    }

    fn save(&self, s: &Sent) -> AppResult<()> {
        let json = serde_json::to_string(s).map_err(|e| AppError::Internal(e.to_string()))?;
        self.db.write(|c| kv_set(c, &sent_key(self.id), &json))
    }

    /// Entries in these namespaces the other end doesn't have yet, in
    /// batches. Anything that looks like a secret is never sent.
    pub fn pending(&self, store: &SyncStore, namespaces: &[&str]) -> AppResult<Vec<EntryMap>> {
        Ok(batches(self.unsent(store.entries(namespaces)?)))
    }

    /// The entries of `all` the other end doesn't have yet.
    pub fn unsent(&self, all: EntryMap) -> EntryMap {
        let sent = self.load();
        let mut out = EntryMap::new();
        for (ns, entries) in all {
            let had = sent.get(&ns);
            for (k, e) in entries {
                if had.and_then(|h| h.get(&k)).is_some_and(|(ts, dev)| *ts == e.ts && *dev == e.device) {
                    continue;
                }
                if e.value.as_ref().is_some_and(crate::sync::store::looks_secret) {
                    continue;
                }
                out.entry(ns.clone()).or_default().insert(k, e);
            }
        }
        out
    }

    /// The other end has these now (sent, or received from it).
    pub fn mark(&self, m: &EntryMap) -> AppResult<()> {
        let mut sent = self.load();
        for (ns, e) in m {
            let s = sent.entry(ns.clone()).or_default();
            for (k, v) in e {
                s.insert(k.clone(), (v.ts, v.device.clone()));
            }
        }
        self.save(&sent)
    }

    /// The other end has these stamps (from its key list).
    pub fn mark_stamps(&self, ns: &str, stamps: &[crate::sync::store::Stamp]) -> AppResult<()> {
        let mut sent = self.load();
        let s = sent.entry(ns.to_string()).or_default();
        for (k, ts, dev) in stamps {
            s.insert(k.clone(), (*ts, dev.clone()));
        }
        self.save(&sent)
    }

    /// Forget what the other end has (everything goes again).
    pub fn clear(&self) -> AppResult<()> {
        self.db.write(|c| {
            c.execute("DELETE FROM kv WHERE key = ?1", [sent_key(self.id)])?;
            Ok(())
        })
    }
}
