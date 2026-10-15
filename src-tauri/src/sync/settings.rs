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

/// Settings renamed since the sync file began keep their first name in the
/// store, so devices on older builds still share them.
const STORE_NAMES: &[(&str, &str)] = &[("reportPlays", "reportToPlex")];

/// The store key for a setting.
fn store_key(name: &str) -> &str {
    STORE_NAMES.iter().find(|(n, _)| *n == name).map_or(name, |(_, k)| k)
}

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
        let key = store_key(name);
        if store.get(&SETTINGS, key)?.as_ref() != Some(v) {
            ops.push(super::store::Op::put(&SETTINGS, key, v)?);
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
        store.seed(&SETTINGS, store_key(name), v, ts)?;
    }
    Ok(())
}

/// The synced settings the store has that differ from `s`, as a patch.
/// A value this build can't take (a newer build's option) is skipped.
pub fn patch_from_store(store: &SyncStore, s: &Settings) -> AppResult<Option<SettingsPatch>> {
    let m = as_map(s)?;
    let mut ok = Map::new();
    for name in SYNCED_SETTINGS {
        let Some(v) = store.get(&SETTINGS, store_key(name))? else { continue };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbHandle;

    #[test]
    fn a_renamed_setting_keeps_its_store_name_for_older_builds() {
        let dir = tempfile::tempdir().unwrap();
        let store = SyncStore::new(DbHandle::open(dir.path().join("lib.db")).unwrap(), "pc");
        let off = Settings { report_plays: false, ..Settings::default() };
        push(&store, &off).unwrap();
        // Stored under the name older builds read and write.
        assert_eq!(store.get(&SETTINGS, "reportToPlex").unwrap(), Some(Value::Bool(false)));
        assert_eq!(store.get(&SETTINGS, "reportPlays").unwrap(), None);
        // A change from an older build arrives as the new setting.
        store.put(&SETTINGS, "reportToPlex", &Value::Bool(true)).unwrap();
        let p = patch_from_store(&store, &off).unwrap().unwrap();
        assert_eq!(p.report_plays, Some(true));
        assert_eq!(store_key("volumePercent"), "volumePercent");
    }
}
