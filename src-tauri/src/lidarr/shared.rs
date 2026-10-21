//! Lidarr state every one of the user's devices shares, kept in the sync
//! store's `lidarr` namespace: per-artist modes and choices, the catch-up
//! cursor, claims on Lidarr events, the changes log, albums waiting in
//! staging and what better-copy hunting already tried.
//!
//! Claims keep two devices from acting on the same Lidarr event. The sync
//! file merges entry by entry with the later change winning, which would
//! let a late claim overwrite an early one, so each device writes its own
//! claim key and the earliest claim wins (then the smaller device id). A
//! device acts only once its claim has stood for a while ([`CLAIM_SETTLE_MS`])
//! and is still the winner, so a claim made on another device at the same
//! moment has time to arrive first.

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::sync::SyncStore;
use crate::sync::store::LIDARR;

/// How long a claim stands before its device acts on it.
pub const CLAIM_SETTLE_MS: u64 = 60_000;

/// One device's claim on an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub device: String,
    /// The device's name, for Settings.
    #[serde(default)]
    pub label: String,
    /// When it claimed, ms.
    pub at: u64,
}

/// Where a claim stands for this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimState {
    /// Ours and settled: act now.
    Ours,
    /// Ours but too new to act on yet.
    Settling,
    /// Another device has it.
    Theirs(Claim),
}

#[derive(Clone)]
pub struct Shared {
    store: SyncStore,
    label: Arc<dyn Fn() -> String + Send + Sync>,
}

impl Shared {
    /// `label` names this device in Settings ("which device last did the
    /// Lidarr work").
    pub fn new(store: SyncStore, label: Arc<dyn Fn() -> String + Send + Sync>) -> Self {
        Self { store, label }
    }

    pub fn device(&self) -> String {
        self.store.device().to_string()
    }

    pub fn label(&self) -> String {
        let l = (self.label)();
        if l.trim().is_empty() { "This computer".into() } else { l }
    }

    pub fn now(&self) -> u64 {
        self.store.now()
    }

    pub fn store(&self) -> &SyncStore {
        &self.store
    }

    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.store.get(&LIDARR, key).ok().flatten().and_then(|v| serde_json::from_value(v).ok())
    }

    pub fn put<T: Serialize>(&self, key: &str, value: &T) -> AppResult<()> {
        let v = serde_json::to_value(value).map_err(|e| AppError::Internal(e.to_string()))?;
        self.store.put(&LIDARR, key, &v)
    }

    pub fn remove(&self, key: &str) -> AppResult<()> {
        self.store.remove(&LIDARR, key).map(|_| ())
    }

    /// Every value whose key starts with `prefix`, with the prefix taken off.
    pub fn list<T: DeserializeOwned>(&self, prefix: &str) -> Vec<(String, T)> {
        self.store
            .list(&LIDARR)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| {
                let rest = k.strip_prefix(prefix)?.to_string();
                serde_json::from_value(v).ok().map(|t| (rest, t))
            })
            .collect()
    }

    /// Claim `event` (a key like `<server>:<history id>`) for this device,
    /// and say where the claims stand. Writing is idempotent: a device
    /// keeps its first claim time.
    pub fn claim(&self, event: &str) -> AppResult<ClaimState> {
        let me = self.device();
        let mine = format!("claim:{event}|{me}");
        let now = self.now();
        let at = match self.get::<Claim>(&mine) {
            Some(c) => c.at,
            None => {
                self.put(&mine, &Claim { device: me.clone(), label: self.label(), at: now })?;
                now
            }
        };
        Ok(self.claim_state(event, &me, at, now))
    }

    /// Who holds `event`, without claiming it.
    pub fn holder(&self, event: &str) -> Option<Claim> {
        self.claims(event).into_iter().min_by(|a, b| (a.at, &a.device).cmp(&(b.at, &b.device)))
    }

    fn claims(&self, event: &str) -> Vec<Claim> {
        self.list::<Claim>(&format!("claim:{event}|")).into_iter().map(|(_, c)| c).collect()
    }

    fn claim_state(&self, event: &str, me: &str, at: u64, now: u64) -> ClaimState {
        match self.holder(event) {
            Some(w) if w.device != me => ClaimState::Theirs(w),
            _ if now.saturating_sub(at) >= CLAIM_SETTLE_MS => ClaimState::Ours,
            _ => ClaimState::Settling,
        }
    }

    /// Forget claims older than `before` (ms).
    pub fn prune_claims(&self, before: u64) {
        for (k, c) in self.list::<Claim>("claim:") {
            if c.at < before {
                let _ = self.remove(&format!("claim:{k}"));
            }
        }
    }
}

/// A Value that's an object, for callers that build loose entries.
pub fn object(v: Value) -> Value {
    if v.is_object() { v } else { Value::Object(Default::default()) }
}

#[cfg(test)]
pub mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::db::DbHandle;

    /// A device with its own database and a clock the test moves.
    pub fn device(name: &str, clock: Arc<AtomicU64>) -> Shared {
        // Kept for the test run (each device has its own database).
        let dir: &'static tempfile::TempDir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let db = DbHandle::open(dir.path().join(format!("{name}.db"))).unwrap();
        let c = clock.clone();
        let store = SyncStore::with_clock(db, name, Arc::new(move || c.load(Ordering::SeqCst)));
        let label = name.to_string();
        Shared::new(store, Arc::new(move || label.clone()))
    }

    /// The shared store on a test's own database, with a clock (ms) the test moves.
    pub fn on_with_clock(db: &DbHandle, clock: Arc<AtomicU64>) -> Shared {
        let store = SyncStore::with_clock(db.clone(), "test-device", Arc::new(move || clock.load(Ordering::SeqCst)));
        Shared::new(store, Arc::new(|| "Test computer".to_string()))
    }

    /// The shared store on a test's own database.
    pub fn on(db: &DbHandle) -> Shared {
        Shared::new(SyncStore::new(db.clone(), "test-device"), Arc::new(|| "Test computer".to_string()))
    }

    /// Send what `from` has to `to`, like the sync file would.
    pub fn sync(from: &Shared, to: &Shared) {
        let ns = [LIDARR.name()];
        let e = from.store.entries(&ns).unwrap();
        to.store.merge(&e, &ns).unwrap();
    }

    #[test]
    fn values_round_trip_by_prefix() {
        let clock = Arc::new(AtomicU64::new(1_000));
        let s = device("pc", clock);
        s.put("artist:a", &serde_json::json!({"mode": "protected"})).unwrap();
        s.put("artist:b", &serde_json::json!({"mode": "managed"})).unwrap();
        s.put("cursor:x", &serde_json::json!({"lastId": 1})).unwrap();
        let l: Vec<(String, Value)> = s.list("artist:");
        assert_eq!(l.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        s.remove("artist:a").unwrap();
        assert!(s.get::<Value>("artist:a").is_none());
    }

    #[test]
    fn the_earliest_claim_wins_across_devices_and_waits_to_settle() {
        let clock = Arc::new(AtomicU64::new(10_000));
        let (a, b) = (device("aaa", clock.clone()), device("bbb", clock.clone()));
        assert_eq!(a.claim("srv:7").unwrap(), ClaimState::Settling);
        clock.fetch_add(5, Ordering::SeqCst);
        // B claims before A's claim reached it: both think they might win.
        assert_eq!(b.claim("srv:7").unwrap(), ClaimState::Settling);
        sync(&a, &b);
        sync(&b, &a);
        clock.fetch_add(CLAIM_SETTLE_MS, Ordering::SeqCst);
        assert_eq!(a.claim("srv:7").unwrap(), ClaimState::Ours);
        match b.claim("srv:7").unwrap() {
            ClaimState::Theirs(c) => assert_eq!(c.device, "aaa"),
            other => panic!("{other:?}"),
        }
        // A later look keeps the first claim time.
        assert_eq!(a.claim("srv:7").unwrap(), ClaimState::Ours);
        assert_eq!(a.holder("srv:7").unwrap().device, "aaa");
    }

    #[test]
    fn a_claim_already_seen_is_never_taken() {
        let clock = Arc::new(AtomicU64::new(10_000));
        let (a, b) = (device("bbb", clock.clone()), device("aaa", clock.clone()));
        a.claim("srv:9").unwrap();
        sync(&a, &b);
        clock.fetch_add(1, Ordering::SeqCst);
        // B's id sorts first, but A's claim is older.
        assert!(matches!(b.claim("srv:9").unwrap(), ClaimState::Theirs(_)));
    }

    #[test]
    fn old_claims_are_pruned() {
        let clock = Arc::new(AtomicU64::new(10_000));
        let a = device("aaa", clock.clone());
        a.claim("srv:1").unwrap();
        clock.store(50_000, Ordering::SeqCst);
        a.claim("srv:2").unwrap();
        a.prune_claims(20_000);
        assert!(a.holder("srv:1").is_none());
        assert!(a.holder("srv:2").is_some());
    }
}
