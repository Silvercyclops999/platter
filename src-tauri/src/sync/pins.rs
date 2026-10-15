//! Library pins: up to [`MAX_PINS`] albums, artists or playlists kept at
//! the top of the library sidebar, newest first.

use serde::{Deserialize, Serialize};

use super::store::{Op, PINS, SyncStore};
use crate::error::{AppError, AppResult};

pub const MAX_PINS: usize = 12;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    /// album, artist or playlist.
    pub kind: String,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub thumb: Option<String>,
    /// When it was pinned (ms); orders the pins.
    #[serde(default)]
    pub pinned_at: u64,
}

fn key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

fn valid(p: &Pin) -> AppResult<()> {
    if !matches!(p.kind.as_str(), "album" | "artist" | "playlist") || p.id.is_empty() {
        return Err(AppError::Invalid("Only albums, artists and playlists can be pinned.".into()));
    }
    Ok(())
}

/// The pins, newest first.
pub fn list(store: &SyncStore) -> AppResult<Vec<Pin>> {
    let mut pins: Vec<Pin> = store.list(&PINS)?.into_iter().map(|(_, p)| p).collect();
    pins.sort_by(|a, b| b.pinned_at.cmp(&a.pinned_at).then_with(|| a.title.cmp(&b.title)));
    pins.truncate(MAX_PINS);
    Ok(pins)
}

/// Pin, or unpin when it's pinned. Pinning past the limit drops the oldest.
pub fn toggle(store: &SyncStore, mut pin: Pin) -> AppResult<Vec<Pin>> {
    valid(&pin)?;
    let k = key(&pin.kind, &pin.id);
    if store.get(&PINS, &k)?.is_some() {
        store.remove(&PINS, &k)?;
        return list(store);
    }
    pin.pinned_at = store.now();
    let mut ops = vec![Op::put(&PINS, &k, &pin)?];
    let mut all = list(store)?;
    all.insert(0, pin);
    for old in all.iter().skip(MAX_PINS) {
        ops.push(Op::remove(&PINS, key(&old.kind, &old.id)));
    }
    store.write_many(ops)?;
    list(store)
}

/// Take pins kept by an older build (in the window's storage), in their
/// order. Only once: when the store has never had a pin.
pub fn import(store: &SyncStore, pins: Vec<Pin>) -> AppResult<Vec<Pin>> {
    if store.ever_used(&PINS)? {
        return list(store);
    }
    let now = store.now();
    let mut ops = Vec::new();
    for (i, mut p) in pins.into_iter().take(MAX_PINS).enumerate() {
        if valid(&p).is_err() {
            continue;
        }
        p.pinned_at = now.saturating_sub(i as u64);
        ops.push(Op::put(&PINS, key(&p.kind, &p.id), &p)?);
    }
    store.write_many(ops)?;
    list(store)
}
