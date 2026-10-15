//! Settings marked Synced ([`crate::settings::SYNCED_SETTINGS`]) go through
//! the store, one entry per setting. Settings marked This device never do.
//!
//! - After every local save, [`push`] stores the synced values that differ.
//! - When the sync file brings settings, [`patch_from_store`] turns the ones
//!   that differ into a settings patch, applied like a change on the
//!   Settings page. The values then match the store, so nothing echoes back.

use serde_json::{Map, Value};

use super::store::{SETTINGS, SyncStore};
use crate::error::{AppError, AppResult};
use crate::settings::{SYNCED_SETTINGS, Settings, SettingsPatch};

fn as_map(s: &Settings) -> AppResult<Map<String, Value>> {
    match serde_json::to_value(s).map_err(|e| AppError::Internal(e.to_string()))? {
        Value::Object(m) => Ok(m),
        _ => Err(AppError::Internal("settings aren't an object".into())),
    }
}

/// Store synced settings that changed here.
pub fn push(store: &SyncStore, s: &Settings) -> AppResult<()> {
    let m = as_map(s)?;
    let mut ops = Vec::new();
    for name in SYNCED_SETTINGS {
        let Some(v) = m.get(*name) else { continue };
        if store.get(&SETTINGS, name)?.as_ref() != Some(v) {
            ops.push(super::store::Op::put(&SETTINGS, *name, v)?);
        }
    }
    store.write_many(ops)
}

/// First run with the store: settings changed from their defaults count as
/// choices made now; the rest are seeded as old, so another device's choice
/// wins over a default.
pub fn seed(store: &SyncStore, s: &Settings) -> AppResult<()> {
    let m = as_map(s)?;
    let d = as_map(&Settings::default())?;
    let now = store.now();
    for name in SYNCED_SETTINGS {
        let Some(v) = m.get(*name) else { continue };
        let ts = if d.get(*name) == Some(v) { 1 } else { now };
        store.seed(&SETTINGS, name, v, ts)?;
    }
    Ok(())
}

/// The synced settings the store has that differ from `s`, as a patch.
/// A value this build can't take (a newer build's option) is skipped.
pub fn patch_from_store(store: &SyncStore, s: &Settings) -> AppResult<Option<SettingsPatch>> {
    let m = as_map(s)?;
    let mut ok = Map::new();
    for name in SYNCED_SETTINGS {
        let Some(v) = store.get(&SETTINGS, name)? else { continue };
        if m.get(*name) == Some(&v) {
            continue;
        }
        let one = Value::Object(Map::from_iter([(name.to_string(), v.clone())]));
        if serde_json::from_value::<SettingsPatch>(one).is_ok() {
            ok.insert(name.to_string(), v);
        }
    }
    if ok.is_empty() {
        return Ok(None);
    }
    serde_json::from_value(Value::Object(ok)).map(Some).map_err(|e| AppError::Internal(e.to_string()))
}
