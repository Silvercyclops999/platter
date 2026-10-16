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

fn secret_account(id: &str) -> String {
    format!("bridge-link-{id}")
}

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
    #[serde(default)]
    pub guard: Guard,
    #[serde(default)]
    pub counter: u64,
}

#[derive(Clone)]
pub struct Links {
    db: DbHandle,
    keys: Arc<dyn TokenStore>,
}

impl Links {
    pub fn new(db: DbHandle, keys: Arc<dyn TokenStore>) -> Self {
        Self { db, keys }
    }

    pub fn all(&self) -> Vec<Paired> {
        self.db.read(|c| kv_get(c, LINKS_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save(&self, list: &[Paired]) -> AppResult<()> {
        let json = serde_json::to_string(list).map_err(|e| AppError::Internal(e.to_string()))?;
        self.db.write(|c| kv_set(c, LINKS_KEY, &json))
    }

    pub fn get(&self, id: &str) -> Option<Paired> {
        self.all().into_iter().find(|p| p.id == id)
    }

    /// The link with its secret, when both are there.
    pub fn link(&self, id: &str) -> Option<Link> {
        let p = self.get(id)?;
        let secret = self.keys.get(&secret_account(id)).ok().flatten().and_then(|t| seal::key32(t.expose()))?;
        Some(Link { id: p.id, secret, relay: p.relay, side: p.side, me: p.me })
    }

    pub fn links(&self) -> Vec<Link> {
        self.all().iter().filter_map(|p| self.link(&p.id)).collect()
    }

    pub fn add(&self, link: &Link, name: &str, version: &str, now_s: i64) -> AppResult<()> {
        self.keys.set(&secret_account(&link.id), &Token::new(seal::b64(&link.secret))?)?;
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
            guard: Guard::default(),
            counter: 0,
        });
        self.save(&list)
    }

    pub fn remove(&self, id: &str) -> AppResult<()> {
        let _ = self.keys.delete(&secret_account(id));
        let mut list = self.all();
        list.retain(|p| p.id != id);
        self.save(&list)?;
        self.db.write(|c| {
            c.execute("DELETE FROM kv WHERE key = ?1", [sent_key(id)])?;
            Ok(())
        })
    }

    /// Change one pairing's kept state.
    pub fn update(&self, id: &str, f: impl FnOnce(&mut Paired)) -> AppResult<()> {
        let mut list = self.all();
        if let Some(p) = list.iter_mut().find(|p| p.id == id) {
            f(p);
        }
        self.save(&list)
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

/// Which shared Lidarr entries the other end of a link already has: the
/// entry's time and device, by key. What's missing or different is sent.
pub struct Outbox<'a> {
    db: &'a DbHandle,
    id: &'a str,
}

/// Most entries in one message.
pub const BATCH: usize = 250;

type Sent = BTreeMap<String, (u64, String)>;

impl<'a> Outbox<'a> {
    pub fn new(db: &'a DbHandle, id: &'a str) -> Self {
        Self { db, id }
    }

    fn load(&self) -> Sent {
        self.db.read(|c| kv_get(c, &sent_key(self.id))).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save(&self, s: &Sent) -> AppResult<()> {
        let json = serde_json::to_string(s).map_err(|e| AppError::Internal(e.to_string()))?;
        self.db.write(|c| kv_set(c, &sent_key(self.id), &json))
    }

    /// Entries the other end doesn't have yet, in batches.
    pub fn pending(&self, store: &SyncStore) -> AppResult<Vec<EntryMap>> {
        let sent = self.load();
        let all = store.entries(&[LIDARR.name()])?;
        let mut out: Vec<EntryMap> = Vec::new();
        let mut cur = EntryMap::new();
        let mut n = 0;
        for (ns, entries) in all {
            for (k, e) in entries {
                if sent.get(&k).is_some_and(|(ts, dev)| *ts == e.ts && *dev == e.device) {
                    continue;
                }
                cur.entry(ns.clone()).or_default().insert(k, e);
                n += 1;
                if n == BATCH {
                    out.push(std::mem::take(&mut cur));
                    n = 0;
                }
            }
        }
        if n > 0 {
            out.push(cur);
        }
        Ok(out)
    }

    /// The other end has these now (sent, or received from it).
    pub fn mark(&self, m: &EntryMap) -> AppResult<()> {
        let mut sent = self.load();
        for e in m.values() {
            for (k, v) in e {
                sent.insert(k.clone(), (v.ts, v.device.clone()));
            }
        }
        self.save(&sent)
    }
}
