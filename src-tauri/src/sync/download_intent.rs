//! What each of the user's devices has downloaded, so the others can offer
//! the same with one tap. Downloads stay per device: an entry is only an
//! intent ("the computer has this album downloaded"), and nothing here
//! ever starts a download. Each device writes only its own entries
//! ([`DOWNLOAD_INTENT`], keyed `<device>|<kind>:<id>`).
//!
//! - [`publish`] makes this device's entries match its downloads.
//! - [`suggestions`] lists what other devices downloaded that this one
//!   hasn't, leaving out what was dismissed here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::store::{DOWNLOAD_INTENT, Op, SyncStore};
use crate::db::store::{kv_get, kv_set};
use crate::downloads::store::{ItemKey, ItemKind, ItemRow};
use crate::error::{AppError, AppResult};

/// Suggestions dismissed on this device (`<kind>:<id>`).
const DISMISSED_KEY: &str = "sync.downloads.dismissed";

/// One download on one device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadIntent {
    pub kind: ItemKind,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// The device's name, as its owner sees it.
    #[serde(default)]
    pub device_name: String,
    /// When it was downloaded there (unix s).
    #[serde(default)]
    pub added_at: i64,
}

/// Something another device has downloaded and this one hasn't.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub kind: ItemKind,
    pub id: String,
    pub title: String,
    /// The devices that have it.
    pub devices: Vec<String>,
    pub added_at: i64,
}

fn item(kind: ItemKind, id: &str) -> String {
    format!("{}:{id}", kind.as_str())
}

fn key(device: &str, k: &ItemKey) -> String {
    format!("{device}|{}", item(k.kind, &k.id))
}

/// Make this device's entries match its downloads: new ones stored, ones
/// turned off removed. Other devices' entries aren't touched.
pub fn publish(store: &SyncStore, device_name: &str, items: &[ItemRow]) -> AppResult<usize> {
    let me = store.device().to_string();
    let prefix = format!("{me}|");
    let mut ops = Vec::new();
    let mut want = BTreeMap::new();
    for r in items {
        let v = DownloadIntent {
            kind: r.key.kind,
            id: r.key.id.clone(),
            title: r.title.chars().take(200).collect(),
            device_name: device_name.chars().take(60).collect(),
            added_at: r.added_at,
        };
        want.insert(key(&me, &r.key), v);
    }
    for (k, v) in &want {
        if store.get(&DOWNLOAD_INTENT, k)?.as_ref() != Some(v) {
            ops.push(Op::put(&DOWNLOAD_INTENT, k.clone(), v)?);
        }
    }
    for (k, _) in store.list(&DOWNLOAD_INTENT)? {
        if k.starts_with(&prefix) && !want.contains_key(&k) {
            ops.push(Op::remove(&DOWNLOAD_INTENT, k));
        }
    }
    let n = ops.len();
    store.write_many(ops)?;
    Ok(n)
}

fn dismissed(store: &SyncStore) -> Vec<String> {
    store.db().read(|c| kv_get(c, DISMISSED_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Stop suggesting this one here (this device only).
pub fn dismiss(store: &SyncStore, kind: ItemKind, id: &str) -> AppResult<()> {
    let mut d = dismissed(store);
    let k = item(kind, id);
    if !d.contains(&k) {
        d.push(k);
    }
    // Only the latest few hundred matter.
    if d.len() > 500 {
        d.drain(..d.len() - 500);
    }
    let json = serde_json::to_string(&d).map_err(|e| AppError::Internal(e.to_string()))?;
    store.db().write(|c| kv_set(c, DISMISSED_KEY, &json))
}

/// What other devices downloaded that this one hasn't (`have`: this
/// device's downloads), newest first, one per item with every device
/// that has it.
pub fn suggestions(store: &SyncStore, have: &[ItemKey]) -> AppResult<Vec<Suggestion>> {
    let me = format!("{}|", store.device());
    let dismissed = dismissed(store);
    let mut by_item: BTreeMap<String, Suggestion> = BTreeMap::new();
    for (k, v) in store.list(&DOWNLOAD_INTENT)? {
        if k.starts_with(&me) || (v.kind != ItemKind::Liked && v.id.is_empty()) {
            continue;
        }
        let ik = ItemKey::new(v.kind, &v.id);
        let name = item(ik.kind, &ik.id);
        if have.contains(&ik) || dismissed.contains(&name) {
            continue;
        }
        let device = if v.device_name.trim().is_empty() { "Your other device".to_string() } else { v.device_name.clone() };
        let s = by_item.entry(name).or_insert_with(|| Suggestion {
            kind: ik.kind,
            id: ik.id.clone(),
            title: v.title.clone(),
            devices: Vec::new(),
            added_at: v.added_at,
        });
        if !s.devices.contains(&device) {
            s.devices.push(device);
        }
        s.added_at = s.added_at.max(v.added_at);
    }
    let mut out: Vec<Suggestion> = by_item.into_values().collect();
    out.sort_by(|a, b| b.added_at.cmp(&a.added_at).then_with(|| a.title.cmp(&b.title)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbHandle;

    fn row(kind: ItemKind, id: &str, title: &str, at: i64) -> ItemRow {
        ItemRow { key: ItemKey::new(kind, id), title: title.into(), added_at: at }
    }

    fn carry(from: &SyncStore, to: &SyncStore) {
        let e = from.entries(&[DOWNLOAD_INTENT.name()]).unwrap();
        to.merge(&e, &[DOWNLOAD_INTENT.name()]).unwrap();
    }

    #[test]
    fn a_download_on_one_device_is_a_suggestion_on_the_other_and_nothing_more() {
        let tmp = tempfile::tempdir().unwrap();
        let pc_db = DbHandle::open(tmp.path().join("pc.db")).unwrap();
        let phone_db = DbHandle::open(tmp.path().join("phone.db")).unwrap();
        let pc = SyncStore::new(pc_db, "pc");
        let phone = SyncStore::new(phone_db.clone(), "phone");
        let items = [row(ItemKind::Album, "a1", "Blue", 20), row(ItemKind::Playlist, "p1", "Run", 10), row(ItemKind::Liked, "", "Liked Songs", 5)];
        assert_eq!(publish(&pc, "Studio PC", &items).unwrap(), 3);
        assert_eq!(publish(&pc, "Studio PC", &items).unwrap(), 0, "unchanged");
        carry(&pc, &phone);
        // The phone already has the playlist.
        let s = suggestions(&phone, &[ItemKey::new(ItemKind::Playlist, "p1")]).unwrap();
        assert_eq!(s.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Blue", "Liked Songs"], "newest first");
        assert_eq!(s[0].devices, ["Studio PC"]);
        // Never downloaded by itself: the phone's own downloads are untouched.
        let n: i64 = phone_db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM download_items", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 0);
        // The phone's own (empty) publish leaves the computer's entries alone.
        publish(&phone, "Phone", &[]).unwrap();
        assert_eq!(phone.list(&DOWNLOAD_INTENT).unwrap().len(), 3);
        // Dismissed here: gone from the list, on this device only.
        dismiss(&phone, ItemKind::Album, "a1").unwrap();
        assert_eq!(suggestions(&phone, &[]).unwrap().len(), 2);
        // Turned off on the computer: the suggestion goes.
        publish(&pc, "Studio PC", &items[1..]).unwrap();
        carry(&pc, &phone);
        let s = suggestions(&phone, &[]).unwrap();
        assert!(s.iter().all(|s| s.id != "a1"));
        assert!(suggestions(&pc, &[]).unwrap().is_empty(), "a device isn't offered its own downloads");
    }

    #[test]
    fn two_devices_with_the_same_album_make_one_suggestion() {
        let tmp = tempfile::tempdir().unwrap();
        let s = |id: &str| SyncStore::new(DbHandle::open(tmp.path().join(format!("{id}.db"))).unwrap(), id);
        let (pc, mac, phone) = (s("pc"), s("mac"), s("phone"));
        publish(&pc, "PC", &[row(ItemKind::Album, "a1", "Blue", 20)]).unwrap();
        publish(&mac, "Mac", &[row(ItemKind::Album, "a1", "Blue", 30)]).unwrap();
        carry(&pc, &phone);
        carry(&mac, &phone);
        let out = suggestions(&phone, &[]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].devices.len(), 2);
        assert_eq!(out[0].added_at, 30);
    }
}
