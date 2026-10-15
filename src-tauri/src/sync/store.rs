//! The typed store every feature with app-only state writes to.
//!
//! Each piece of state is an entry: a namespace ([`Ns`]), a key, and a JSON
//! value. Entries always live in the local database (`sync_entries`); when
//! the sync file is on, the [`engine`](super::engine) merges them with the
//! file. Every change records when it happened and on which device, and a
//! deletion stays behind as a tombstone, so two devices editing at the same
//! time merge entry by entry with the later change winning.

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Notify, broadcast};

use crate::db::DbHandle;
use crate::error::{AppError, AppResult};

/// One entry as stored and as written to the sync file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// None: deleted (a tombstone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// When it changed, ms since the epoch.
    pub ts: u64,
    /// The device that changed it.
    pub device: String,
}

impl Entry {
    /// Whether this entry wins over `other`: the later change, then the
    /// larger device id, then the larger value, so every device picks the
    /// same one.
    pub fn wins_over(&self, other: &Entry) -> bool {
        let v = |e: &Entry| e.value.as_ref().map(|v| v.to_string()).unwrap_or_default();
        (self.ts, &self.device, v(self)) > (other.ts, &other.device, v(other))
    }

    pub fn is_tombstone(&self) -> bool {
        self.value.is_none()
    }
}

/// Namespace → key → entry.
pub type EntryMap = BTreeMap<String, BTreeMap<String, Entry>>;

/// A namespace and the type its values have.
pub struct Ns<T> {
    name: &'static str,
    _t: PhantomData<fn() -> T>,
}

impl<T> Ns<T> {
    pub const fn new(name: &'static str) -> Self {
        Self { name, _t: PhantomData }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }
}

/// Settings marked Synced, one entry per setting (camelCase name → value).
pub const SETTINGS: Ns<Value> = Ns::new("settings");
/// Library pins, keyed `kind:id`.
pub const PINS: Ns<super::pins::Pin> = Ns::new("pins");
/// Library folders, keyed by folder id.
pub const FOLDERS: Ns<super::folders::Folder> = Ns::new("folders");
/// Which folder a playlist is in, keyed by playlist id.
pub const FOLDER_ITEMS: Ns<super::folders::Placement> = Ns::new("folderItems");
/// Followed artists, keyed by artist id.
pub const FOLLOWS: Ns<crate::social::follows::FollowedArtist> = Ns::new("follows");
/// Hidden and snoozed items, keyed `kind:id`.
pub const HIDDEN: Ns<Hidden> = Ns::new("hidden");
/// Places to resume long tracks, keyed by track id.
pub const BOOKMARKS: Ns<Bookmark> = Ns::new("bookmarks");
/// Lyrics the user wrote, keyed by track id.
pub const LYRICS: Ns<UserLyricsEntry> = Ns::new("lyrics");

/// Every namespace the store takes. Anything else is refused, so nothing
/// lands in the sync file by accident.
pub const NAMESPACES: [&str; 8] = [
    SETTINGS.name(),
    PINS.name(),
    FOLDERS.name(),
    FOLDER_ITEMS.name(),
    FOLLOWS.name(),
    HIDDEN.name(),
    BOOKMARKS.name(),
    LYRICS.name(),
];

/// A hidden item; with `until`, snoozed until then (ms).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hidden {
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub until: Option<u64>,
}

/// Where to resume a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub position_ms: u64,
    #[serde(default)]
    pub title: String,
}

/// Lyrics the user wrote for a track (LRC or plain lines).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserLyricsEntry {
    pub text: String,
    /// Shown ahead of Plex's lyrics for this song.
    #[serde(default)]
    pub prefer: bool,
}

/// What changed, for listeners.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changed {
    pub namespaces: Vec<String>,
    /// From the sync file (another device), not from this one.
    pub remote: bool,
}

/// The longest key and value the store takes.
pub const KEY_MAX: usize = 200;
pub const VALUE_MAX: usize = 80 * 1024;

/// Words that mark a secret. A value holding an object field with one of
/// these in its name is refused: tokens and keys live in the keychain only.
const SECRET_WORDS: [&str; 6] = ["token", "password", "secret", "apikey", "api_key", "clientid"];

/// Whether a value carries a field that looks like a secret.
pub fn looks_secret(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, v)| {
            let k = k.to_ascii_lowercase().replace('-', "");
            SECRET_WORDS.iter().any(|w| k.contains(w)) || looks_secret(v)
        }),
        Value::Array(a) => a.iter().any(looks_secret),
        _ => false,
    }
}

pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub fn system_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct Inner {
    db: DbHandle,
    device: String,
    clock: Clock,
    changed: broadcast::Sender<Changed>,
    edited: Notify,
}

/// The store. Cheap to clone.
#[derive(Clone)]
pub struct SyncStore {
    inner: Arc<Inner>,
}

/// One write in a [`SyncStore::write_many`].
pub enum Op {
    Put { ns: &'static str, key: String, value: Value },
    Remove { ns: &'static str, key: String },
}

impl Op {
    pub fn put<T: Serialize>(ns: &Ns<T>, key: impl Into<String>, value: &T) -> AppResult<Op> {
        let value = serde_json::to_value(value).map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(Op::Put { ns: ns.name(), key: key.into(), value })
    }

    pub fn remove<T>(ns: &Ns<T>, key: impl Into<String>) -> Op {
        Op::Remove { ns: ns.name(), key: key.into() }
    }
}

fn row_entry(value: Option<String>, ts: i64, device: String) -> Entry {
    Entry { value: value.and_then(|v| serde_json::from_str(&v).ok()), ts: ts.max(0) as u64, device }
}

fn get_entry(c: &Connection, ns: &str, key: &str) -> AppResult<Option<Entry>> {
    Ok(c
        .query_row("SELECT value, ts, device FROM sync_entries WHERE ns = ?1 AND key = ?2", params![ns, key], |r| {
            Ok(row_entry(r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?)
}

fn set_entry(c: &Connection, ns: &str, key: &str, e: &Entry) -> AppResult<()> {
    let value = e.value.as_ref().map(|v| v.to_string());
    c.execute(
        "INSERT INTO sync_entries (ns, key, value, ts, device) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (ns, key) DO UPDATE SET value = excluded.value, ts = excluded.ts, device = excluded.device",
        params![ns, key, value, e.ts as i64, e.device],
    )?;
    Ok(())
}

fn check(ns: &str, key: &str, value: Option<&Value>) -> AppResult<()> {
    if !NAMESPACES.contains(&ns) {
        return Err(AppError::Invalid(format!("unknown sync namespace {ns}")));
    }
    if key.is_empty() || key.len() > KEY_MAX {
        return Err(AppError::Invalid("bad sync key".into()));
    }
    if let Some(v) = value {
        if looks_secret(v) {
            return Err(AppError::Invalid("secrets never go in the sync store".into()));
        }
        if v.to_string().len() > VALUE_MAX {
            return Err(AppError::Invalid("that's too big to sync".into()));
        }
    }
    Ok(())
}

impl SyncStore {
    pub fn new(db: DbHandle, device: impl Into<String>) -> Self {
        Self::with_clock(db, device, Arc::new(system_ms))
    }

    pub fn with_clock(db: DbHandle, device: impl Into<String>, clock: Clock) -> Self {
        let (changed, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(Inner { db, device: device.into(), clock, changed, edited: Notify::new() }),
        }
    }

    /// This device's id (the Plex client id).
    pub fn device(&self) -> &str {
        &self.inner.device
    }

    pub fn now(&self) -> u64 {
        (self.inner.clock)()
    }

    /// Hear about changes, local and from the sync file.
    pub fn subscribe(&self) -> broadcast::Receiver<Changed> {
        self.inner.changed.subscribe()
    }

    /// Wakes the sync engine after a local change.
    pub(crate) fn edited(&self) -> &Notify {
        &self.inner.edited
    }

    fn emit(&self, namespaces: Vec<String>, remote: bool) {
        if !namespaces.is_empty() {
            let _ = self.inner.changed.send(Changed { namespaces, remote });
        }
    }

    /// One value, or None when missing or deleted.
    pub fn get<T: DeserializeOwned>(&self, ns: &Ns<T>, key: &str) -> AppResult<Option<T>> {
        let e = self.inner.db.read(|c| get_entry(c, ns.name(), key))?;
        Ok(e.and_then(|e| e.value).and_then(|v| serde_json::from_value(v).ok()))
    }

    /// Every live value in a namespace, by key. Values this build can't read
    /// (written by a newer one) are left out but kept.
    pub fn list<T: DeserializeOwned>(&self, ns: &Ns<T>) -> AppResult<Vec<(String, T)>> {
        let name = ns.name();
        let rows: Vec<(String, String)> = self.inner.db.read(|c| {
            let mut st = c.prepare_cached("SELECT key, value FROM sync_entries WHERE ns = ?1 AND value IS NOT NULL ORDER BY key")?;
            let rows = st.query_map([name], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        Ok(rows
            .into_iter()
            .filter_map(|(k, v)| serde_json::from_str::<T>(&v).ok().map(|t| (k, t)))
            .collect())
    }

    /// Whether the namespace has ever had an entry (deleted ones count).
    pub fn ever_used<T>(&self, ns: &Ns<T>) -> AppResult<bool> {
        let name = ns.name();
        self.inner.db.read(|c| {
            Ok(c.query_row("SELECT 1 FROM sync_entries WHERE ns = ?1 LIMIT 1", [name], |_| Ok(()))
                .optional()?
                .is_some())
        })
    }

    /// Keys deleted (tombstones) in a namespace.
    pub fn deleted<T>(&self, ns: &Ns<T>) -> AppResult<Vec<String>> {
        let name = ns.name();
        self.inner.db.read(|c| {
            let mut st = c.prepare_cached("SELECT key FROM sync_entries WHERE ns = ?1 AND value IS NULL ORDER BY key")?;
            let rows = st.query_map([name], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok(rows)
        })
    }

    /// Whether a key has an entry (deleted counts).
    pub fn has_entry<T>(&self, ns: &Ns<T>, key: &str) -> AppResult<bool> {
        let name = ns.name();
        Ok(self.inner.db.read(|c| get_entry(c, name, key))?.is_some())
    }

    pub fn put<T: Serialize>(&self, ns: &Ns<T>, key: &str, value: &T) -> AppResult<()> {
        self.write_many(vec![Op::put(ns, key, value)?])
    }

    /// Delete (leaves a tombstone). Returns whether there was a value.
    pub fn remove<T>(&self, ns: &Ns<T>, key: &str) -> AppResult<bool> {
        let had = self.inner.db.read(|c| get_entry(c, ns.name(), key))?.is_some_and(|e| !e.is_tombstone());
        if had {
            self.write_many(vec![Op::remove(ns, key)])?;
        }
        Ok(had)
    }

    /// Several writes in one transaction (one change event).
    pub fn write_many(&self, ops: Vec<Op>) -> AppResult<()> {
        for op in &ops {
            match op {
                Op::Put { ns, key, value } => check(ns, key, Some(value))?,
                Op::Remove { ns, key } => check(ns, key, None)?,
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        let now = self.now();
        let device = self.inner.device.clone();
        let mut touched: Vec<String> = Vec::new();
        self.inner.db.write(|c| {
            let tx = c.transaction()?;
            for op in &ops {
                let (ns, key, value) = match op {
                    Op::Put { ns, key, value } => (*ns, key, Some(value.clone())),
                    Op::Remove { ns, key } => (*ns, key, None),
                };
                let prev = get_entry(&tx, ns, key)?;
                if prev.as_ref().is_some_and(|p| p.value == value) {
                    continue;
                }
                // Always later than what it replaces, even with a clock behind.
                let ts = prev.map_or(now, |p| now.max(p.ts + 1));
                set_entry(&tx, ns, key, &Entry { value, ts, device: device.clone() })?;
                if !touched.iter().any(|t| t == ns) {
                    touched.push(ns.to_string());
                }
            }
            tx.commit()?;
            Ok(())
        })?;
        if !touched.is_empty() {
            self.inner.edited.notify_one();
            self.emit(touched, false);
        }
        Ok(())
    }

    /// Store a value with a given time, unless the key already has an entry
    /// (used to seed existing state without beating other devices' changes).
    pub fn seed<T: Serialize>(&self, ns: &Ns<T>, key: &str, value: &T, ts: u64) -> AppResult<bool> {
        let value = serde_json::to_value(value).map_err(|e| AppError::Internal(e.to_string()))?;
        check(ns.name(), key, Some(&value))?;
        let device = self.inner.device.clone();
        let name = ns.name();
        let done = self.inner.db.write(|c| {
            if get_entry(c, name, key)?.is_some() {
                return Ok(false);
            }
            set_entry(c, name, key, &Entry { value: Some(value), ts, device })?;
            Ok(true)
        })?;
        if done {
            self.inner.edited.notify_one();
        }
        Ok(done)
    }

    /// Every entry (tombstones too) in these namespaces.
    pub fn entries(&self, namespaces: &[&str]) -> AppResult<EntryMap> {
        let wanted: Vec<String> = namespaces.iter().map(|s| s.to_string()).collect();
        self.inner.db.read(|c| {
            let mut st = c.prepare_cached("SELECT ns, key, value, ts, device FROM sync_entries")?;
            let mut out = EntryMap::new();
            let rows = st.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, row_entry(r.get(2)?, r.get(3)?, r.get(4)?)))
            })?;
            for row in rows {
                let (ns, key, e) = row?;
                if wanted.contains(&ns) {
                    out.entry(ns).or_default().insert(key, e);
                }
            }
            Ok(out)
        })
    }

    /// Take entries from elsewhere (the sync file) where they win. Only
    /// these namespaces; values that look secret are dropped. Returns the
    /// namespaces that changed and tells listeners.
    pub fn merge(&self, other: &EntryMap, namespaces: &[&str]) -> AppResult<Vec<String>> {
        let changed = self.inner.db.write(|c| {
            let tx = c.transaction()?;
            let mut changed: Vec<String> = Vec::new();
            for (ns, entries) in other {
                if !namespaces.contains(&ns.as_str()) || !NAMESPACES.contains(&ns.as_str()) {
                    continue;
                }
                for (key, e) in entries {
                    if check(ns, key, e.value.as_ref()).is_err() {
                        continue;
                    }
                    let mine = get_entry(&tx, ns, key)?;
                    if mine.as_ref().is_none_or(|m| e.wins_over(m)) {
                        let differs = mine.as_ref().is_none_or(|m| m.value != e.value);
                        set_entry(&tx, ns, key, e)?;
                        if differs && !changed.contains(ns) {
                            changed.push(ns.clone());
                        }
                    }
                }
            }
            tx.commit()?;
            Ok(changed)
        })?;
        self.emit(changed.clone(), true);
        Ok(changed)
    }

    /// Forget tombstones older than `before` (ms).
    pub fn prune(&self, before: u64) -> AppResult<usize> {
        self.inner.db.write(|c| {
            Ok(c.execute("DELETE FROM sync_entries WHERE value IS NULL AND ts < ?1", [before as i64])?)
        })
    }

    /// Run blocking store work from async code.
    pub async fn run<R, F>(&self, f: F) -> AppResult<R>
    where
        R: Send + 'static,
        F: FnOnce(&SyncStore) -> AppResult<R> + Send + 'static,
    {
        let s = self.clone();
        tokio::task::spawn_blocking(move || f(&s))
            .await
            .map_err(|e| AppError::Internal(format!("sync task: {e}")))?
    }
}
