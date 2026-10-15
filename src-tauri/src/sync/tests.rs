//! Sync store, sync file, folders, pins and key copy. Every test works in
//! temp folders; nothing touches a real cloud folder or the keychain.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::json;
use tempfile::TempDir;

use super::engine::{self, Groups, PassError, SyncConfig, SyncEngine, SyncState, Timing};
use super::file;
use super::folders;
use super::pins::{self, Pin};
use super::store::{self, Entry, EntryMap, FOLLOWS, HIDDEN, Hidden, PINS, SETTINGS, SyncStore};
use crate::social::follows::FollowedArtist;
use crate::db::DbHandle;
use crate::settings::Settings;

const TAG: &str = "0123456789abcdef";

/// A device: its own database, store and clock.
struct Dev {
    store: SyncStore,
    clock: Arc<AtomicU64>,
    _dir: TempDir,
}

impl Dev {
    fn new(id: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("lib.db")).unwrap();
        let clock = Arc::new(AtomicU64::new(1_000_000));
        let c = clock.clone();
        let store = SyncStore::with_clock(db, id, Arc::new(move || c.load(Ordering::SeqCst)));
        Dev { store, clock, _dir: dir }
    }

    fn at(&self, ms: u64) {
        self.clock.store(ms, Ordering::SeqCst);
    }

    fn pass(&self, folder: &Path) -> Result<engine::PassOutcome, PassError> {
        engine::pass(&self.store, folder, TAG, Groups::default(), self.store.device(), self.store.now())
    }
}

fn pin(kind: &str, id: &str) -> Pin {
    Pin { kind: kind.into(), id: id.into(), title: format!("{kind} {id}"), subtitle: String::new(), thumb: None, pinned_at: 0 }
}

fn follow(name: &str) -> FollowedArtist {
    FollowedArtist { id: String::new(), title: name.into(), thumb: None, followed_at: 1 }
}

fn file_path(folder: &Path) -> std::path::PathBuf {
    file::dir(folder).join(file::file_name(TAG))
}

// ------------------------------------------------------------ store

#[test]
fn store_puts_gets_lists_and_removes() {
    let d = Dev::new("pc");
    d.store.put(&FOLLOWS, "a1", &follow("Ana")).unwrap();
    d.store.put(&FOLLOWS, "a2", &follow("Bo")).unwrap();
    assert_eq!(d.store.get(&FOLLOWS, "a1").unwrap().unwrap().title, "Ana");
    assert_eq!(d.store.list(&FOLLOWS).unwrap().len(), 2);
    assert!(d.store.remove(&FOLLOWS, "a1").unwrap());
    assert!(!d.store.remove(&FOLLOWS, "a1").unwrap(), "already gone");
    assert_eq!(d.store.get(&FOLLOWS, "a1").unwrap(), None);
    assert_eq!(d.store.deleted(&FOLLOWS).unwrap(), vec!["a1".to_string()]);
    assert!(d.store.has_entry(&FOLLOWS, "a1").unwrap(), "a tombstone stays");
    let e = d.store.entries(&["follows"]).unwrap();
    assert!(e["follows"]["a1"].is_tombstone());
}

#[test]
fn a_change_is_always_later_than_the_one_it_replaces() {
    let d = Dev::new("pc");
    d.at(5_000);
    d.store.put(&FOLLOWS, "a", &follow("one")).unwrap();
    // The clock jumps back (NTP, another time zone): the edit still wins.
    d.at(1_000);
    d.store.put(&FOLLOWS, "a", &follow("two")).unwrap();
    let e = &d.store.entries(&["follows"]).unwrap()["follows"]["a"];
    assert_eq!(e.ts, 5_001);
    assert_eq!(e.device, "pc");
}

#[test]
fn unknown_namespaces_and_secrets_are_refused() {
    let d = Dev::new("pc");
    let ns: store::Ns<serde_json::Value> = store::Ns::new("tokens");
    assert!(d.store.put(&ns, "k", &json!(1)).is_err());
    for bad in [json!({ "apiKey": "x" }), json!({ "token": "x" }), json!([{ "lastfm_password": 1 }]), json!({ "a": { "clientId": "x" } })] {
        assert!(d.store.put(&SETTINGS, "k", &bad).is_err(), "{bad}");
    }
    assert!(d.store.put(&SETTINGS, "k", &json!({ "volume": 1 })).is_ok());
    assert!(d.store.put(&SETTINGS, "", &json!(1)).is_err());
}

#[test]
fn merge_takes_later_entries_and_reports_them() {
    let d = Dev::new("pc");
    d.at(100);
    d.store.put(&FOLLOWS, "a", &follow("mine")).unwrap();
    let mut rx = d.store.subscribe();
    let mut other = EntryMap::new();
    let e = |v: Option<serde_json::Value>, ts, dev: &str| Entry { value: v, ts, device: dev.into() };
    other.entry("follows".into()).or_default().insert("a".into(), e(Some(json!({ "id": "a", "title": "older", "followedAt": 1 })), 50, "mac"));
    other.entry("follows".into()).or_default().insert("b".into(), e(Some(json!({ "id": "b", "title": "new", "followedAt": 1 })), 50, "mac"));
    let changed = d.store.merge(&other, &["follows"]).unwrap();
    assert_eq!(changed, vec!["follows".to_string()]);
    assert_eq!(d.store.get(&FOLLOWS, "a").unwrap().unwrap().title, "mine", "older loses");
    assert_eq!(d.store.get(&FOLLOWS, "b").unwrap().unwrap().title, "new");
    let c = rx.try_recv().unwrap();
    assert!(c.remote && c.namespaces == ["follows"]);
    // A later deletion wins over a value.
    let mut del = EntryMap::new();
    del.entry("follows".into()).or_default().insert("a".into(), e(None, 200, "mac"));
    d.store.merge(&del, &["follows"]).unwrap();
    assert_eq!(d.store.get(&FOLLOWS, "a").unwrap(), None);
    // Same time: the larger device id wins, on every device alike.
    let x = e(Some(json!(1)), 10, "a");
    let y = e(Some(json!(2)), 10, "b");
    assert!(y.wins_over(&x) && !x.wins_over(&y));
    // Namespaces outside the groups are left alone.
    let mut pins_only = EntryMap::new();
    pins_only.entry("pins".into()).or_default().insert("album:1".into(), e(Some(json!({ "kind": "album", "id": "1", "title": "t" })), 9, "mac"));
    assert!(d.store.merge(&pins_only, &["follows"]).unwrap().is_empty());
    assert!(d.store.list(&PINS).unwrap().is_empty());
}

// ------------------------------------------------------------ the file

#[test]
fn two_devices_meet_through_the_file() {
    let cloud = tempfile::tempdir().unwrap();
    let (pc, mac) = (Dev::new("pc"), Dev::new("mac"));
    pins::toggle(&pc.store, pin("album", "1")).unwrap();
    let o = pc.pass(cloud.path()).unwrap();
    assert!(o.wrote);
    assert!(file_path(cloud.path()).exists());
    let o = mac.pass(cloud.path()).unwrap();
    assert_eq!(o.changed, vec!["pins".to_string()]);
    assert_eq!(pins::list(&mac.store).unwrap()[0].id, "1");
    // Both devices are listed, this one first.
    assert_eq!(o.devices.len(), 2);
    assert!(o.devices[0].this_device);
    // Nothing new: nothing written.
    assert!(!pc.pass(cloud.path()).unwrap().changed.contains(&"pins".to_string()));
}

#[test]
fn concurrent_edits_merge_per_entry_and_the_later_one_wins() {
    let cloud = tempfile::tempdir().unwrap();
    let (pc, mac) = (Dev::new("pc"), Dev::new("mac"));
    pc.at(10_000);
    mac.at(10_000);
    pc.store.put(&FOLLOWS, "x", &follow("pc's")).unwrap();
    pc.store.put(&FOLLOWS, "only-pc", &follow("p")).unwrap();
    mac.at(20_000);
    mac.store.put(&FOLLOWS, "x", &follow("mac's")).unwrap();
    mac.store.put(&FOLLOWS, "only-mac", &follow("m")).unwrap();
    // Both write before seeing each other.
    pc.pass(cloud.path()).unwrap();
    mac.pass(cloud.path()).unwrap();
    pc.pass(cloud.path()).unwrap();
    for d in [&pc, &mac] {
        let names: Vec<String> = d.store.list(&FOLLOWS).unwrap().into_iter().map(|(k, f)| format!("{k}={}", f.title)).collect();
        assert_eq!(names, ["only-mac=m", "only-pc=p", "x=mac's"]);
    }
}

#[test]
fn deletions_travel_and_old_tombstones_go() {
    let cloud = tempfile::tempdir().unwrap();
    let (pc, mac) = (Dev::new("pc"), Dev::new("mac"));
    pc.store.put(&HIDDEN, "album:9", &Hidden { kind: "album".into(), title: "t".into(), ..Default::default() }).unwrap();
    pc.pass(cloud.path()).unwrap();
    mac.pass(cloud.path()).unwrap();
    assert!(mac.store.get(&HIDDEN, "album:9").unwrap().is_some());
    pc.at(2_000_000);
    pc.store.remove(&HIDDEN, "album:9").unwrap();
    pc.pass(cloud.path()).unwrap();
    mac.at(2_000_000);
    mac.pass(cloud.path()).unwrap();
    assert!(mac.store.get(&HIDDEN, "album:9").unwrap().is_none());
    let text = std::fs::read_to_string(file_path(cloud.path())).unwrap();
    assert!(text.contains("album:9"), "the tombstone is kept for now");
    // 90 days on, the tombstone is gone from the file and the stores.
    let later = 2_000_000 + file::TOMBSTONE_MS + 1;
    pc.at(later);
    pc.store.put(&FOLLOWS, "touch", &follow("t")).unwrap();
    pc.pass(cloud.path()).unwrap();
    let text = std::fs::read_to_string(file_path(cloud.path())).unwrap();
    assert!(!text.contains("album:9"));
    assert!(!pc.store.has_entry(&HIDDEN, "album:9").unwrap());
}

#[test]
fn conflicted_copies_are_merged_and_removed() {
    let cloud = tempfile::tempdir().unwrap();
    let (pc, mac) = (Dev::new("pc"), Dev::new("mac"));
    pc.store.put(&FOLLOWS, "a", &follow("from the file")).unwrap();
    pc.pass(cloud.path()).unwrap();
    // The cloud app kept the mac's version beside it.
    mac.store.put(&FOLLOWS, "b", &follow("from the copy")).unwrap();
    let other = tempfile::tempdir().unwrap();
    mac.pass(other.path()).unwrap();
    let dir = file::dir(cloud.path());
    let names = [format!("sync-{TAG} (conflicted copy).json"), format!("sync-{TAG}-MAC.json")];
    std::fs::copy(file_path(other.path()), dir.join(&names[0])).unwrap();
    std::fs::copy(file_path(other.path()), dir.join(&names[1])).unwrap();
    // Not a copy: another account's file, and a broken file set aside.
    std::fs::write(dir.join("sync-0123456789abcdef0.json"), "{}").unwrap();
    assert_eq!(file::conflicted_copies(&dir, TAG).len(), 2);
    let o = pc.pass(cloud.path()).unwrap();
    assert_eq!(o.merged_copies, 2);
    assert_eq!(pc.store.get(&FOLLOWS, "b").unwrap().unwrap().title, "from the copy");
    assert!(!dir.join(&names[0]).exists() && !dir.join(&names[1]).exists());
    let data = file::read(&file_path(cloud.path())).unwrap().unwrap();
    assert!(data.entries["follows"].contains_key("a") && data.entries["follows"].contains_key("b"));
    assert!(data.devices.contains_key("mac"));
}

#[test]
fn writes_are_whole_files_with_nothing_left_behind() {
    let cloud = tempfile::tempdir().unwrap();
    let pc = Dev::new("pc");
    for i in 0..5 {
        pc.store.put(&FOLLOWS, &format!("a{i}"), &follow("x")).unwrap();
        pc.pass(cloud.path()).unwrap();
        let data = file::read(&file_path(cloud.path())).unwrap().unwrap();
        assert_eq!(data.entries["follows"].len(), i + 1);
    }
    let files: Vec<String> = std::fs::read_dir(file::dir(cloud.path()))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(files, [file::file_name(TAG)], "no temp files left");
    let data = file::read(&file_path(cloud.path())).unwrap().unwrap();
    assert_eq!((data.format, data.app.as_str()), (file::FORMAT, "Platter"));
}

#[test]
fn an_unreadable_file_is_left_alone_until_replaced() {
    let cloud = tempfile::tempdir().unwrap();
    let pc = Dev::new("pc");
    std::fs::create_dir_all(file::dir(cloud.path())).unwrap();
    std::fs::write(file_path(cloud.path()), "not json {").unwrap();
    pc.store.put(&FOLLOWS, "a", &follow("kept")).unwrap();
    assert_eq!(pc.pass(cloud.path()).unwrap_err(), PassError::Unreadable);
    assert_eq!(std::fs::read_to_string(file_path(cloud.path())).unwrap(), "not json {");
    assert!(pc.store.get(&FOLLOWS, "a").unwrap().is_some(), "local changes stay");
    let aside = file::set_aside(&file_path(cloud.path()), 42).unwrap();
    assert!(aside.exists());
    assert!(pc.pass(cloud.path()).unwrap().wrote);
    // The set-aside file isn't taken for a conflicted copy.
    assert!(file::conflicted_copies(&file::dir(cloud.path()), TAG).is_empty());
}

#[test]
fn a_newer_format_is_not_overwritten() {
    let cloud = tempfile::tempdir().unwrap();
    let pc = Dev::new("pc");
    std::fs::create_dir_all(file::dir(cloud.path())).unwrap();
    let body = json!({ "format": 99, "app": "Platter", "entries": {} }).to_string();
    std::fs::write(file_path(cloud.path()), &body).unwrap();
    assert_eq!(pc.pass(cloud.path()).unwrap_err(), PassError::Newer);
    assert_eq!(std::fs::read_to_string(file_path(cloud.path())).unwrap(), body);
}

#[test]
fn changes_queue_while_the_folder_is_missing() {
    let cloud = tempfile::tempdir().unwrap();
    let gone = cloud.path().join("unplugged");
    let pc = Dev::new("pc");
    pins::toggle(&pc.store, pin("artist", "5")).unwrap();
    assert_eq!(pc.pass(&gone).unwrap_err(), PassError::FolderMissing);
    assert!(!gone.exists(), "a missing folder is never created");
    pins::toggle(&pc.store, pin("album", "6")).unwrap();
    std::fs::create_dir(&gone).unwrap();
    pc.pass(&gone).unwrap();
    let data = file::read(&file_path(&gone)).unwrap().unwrap();
    assert_eq!(data.entries["pins"].len(), 2, "both queued changes went out");
}

#[test]
fn groups_turned_off_stay_out_but_other_devices_entries_are_kept() {
    let cloud = tempfile::tempdir().unwrap();
    let (pc, mac) = (Dev::new("pc"), Dev::new("mac"));
    mac.store.put(&FOLLOWS, "m", &follow("mac")).unwrap();
    mac.pass(cloud.path()).unwrap();
    pins::toggle(&pc.store, pin("album", "1")).unwrap();
    pc.store.put(&FOLLOWS, "p", &follow("pc")).unwrap();
    let only_library = Groups { settings: true, library: true, follows: false, lyrics: true };
    engine::pass(&pc.store, cloud.path(), TAG, only_library, "PC", pc.store.now()).unwrap();
    let data = file::read(&file_path(cloud.path())).unwrap().unwrap();
    assert!(data.entries["pins"].contains_key("album:1"));
    assert!(data.entries["follows"].contains_key("m"), "the mac's follows stay");
    assert!(!data.entries["follows"].contains_key("p"), "this device's follows don't go");
    assert!(pc.store.get(&FOLLOWS, "m").unwrap().is_none(), "nor come in");
}

#[test]
fn the_sync_file_never_holds_secrets() {
    let cloud = tempfile::tempdir().unwrap();
    let pc = Dev::new("pc");
    // Keys in this device's keychain (a fake one).
    let secrets = ["lastfm-0d6c1f2a9b", "listenbrainz-77aa31", "fanart-5e5e5e", "imgur-client-1234", "plex-token-abcdef"];
    let keychain = crate::plex::keychain::MemoryStore::new();
    use crate::plex::keychain::TokenStore;
    for (i, s) in secrets.iter().enumerate() {
        keychain.set(&format!("k{i}"), &crate::plex::Token::new(*s).unwrap()).unwrap();
    }
    // Every synced setting, changed from its default, plus everything else.
    let mut s = Settings { info_lastfm: true, info_fanart: true, discord_upload_art: true, computer_name: "Den".into(), ..Default::default() };
    s.mix_exclude_artists = vec!["9".into()];
    super::settings::seed(&pc.store, &s).unwrap();
    super::settings::push(&pc.store, &s).unwrap();
    pins::toggle(&pc.store, pin("playlist", "3")).unwrap();
    let f = folders::create(&pc.store, Some("Road trips"), None).unwrap();
    folders::place(&pc.store, "3", Some(&f.id)).unwrap();
    pc.store.put(&FOLLOWS, "a", &follow("Ana")).unwrap();
    super::lyrics::saved(&pc.store, "77", "[00:01.00]la la", true).unwrap();
    // Trying to slip a key in is refused.
    assert!(pc.store.put(&SETTINGS, "infoLastfm", &json!({ "apiKey": secrets[0] })).is_err());
    pc.pass(cloud.path()).unwrap();

    let text = std::fs::read_to_string(file_path(cloud.path())).unwrap();
    for s in secrets {
        assert!(!text.contains(s), "a secret is in the sync file");
    }
    let lower = text.to_ascii_lowercase();
    for word in ["token", "password", "apikey", "api_key", "secret", "clientid"] {
        assert!(!lower.contains(word), "{word} in the sync file");
    }
    // This-device settings never go in.
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let synced: Vec<&String> = v["entries"]["settings"].as_object().unwrap().keys().collect();
    for n in ["computerName", "volume", "downloadFolder", "syncFolder", "discordUploadArt"] {
        assert!(!synced.iter().any(|k| *k == n), "{n} is this device only");
    }
    assert!(synced.iter().any(|k| *k == "infoLastfm"));
}

// ------------------------------------------------------------ settings

#[test]
fn synced_settings_reach_the_other_device_as_a_patch() {
    let cloud = tempfile::tempdir().unwrap();
    let (pc, mac) = (Dev::new("pc"), Dev::new("mac"));
    let defaults = Settings::default();
    super::settings::seed(&pc.store, &defaults).unwrap();
    super::settings::seed(&mac.store, &defaults).unwrap();
    pc.pass(cloud.path()).unwrap();
    mac.pass(cloud.path()).unwrap();
    assert_eq!(super::settings::patch_from_store(&mac.store, &defaults).unwrap(), None, "defaults meet defaults");
    // The PC changes crossfade (synced) and volume (this device).
    pc.at(5_000_000);
    let changed = Settings { crossfade_seconds: 9, volume: 0.1, ..Default::default() };
    super::settings::push(&pc.store, &changed).unwrap();
    pc.pass(cloud.path()).unwrap();
    mac.pass(cloud.path()).unwrap();
    let p = super::settings::patch_from_store(&mac.store, &defaults).unwrap().unwrap();
    assert_eq!(p.crossfade_seconds, Some(9));
    assert_eq!(p.volume, None, "volume stays on its device");
    // Applied, nothing differs, so nothing echoes back.
    let mut applied = defaults.clone();
    applied.apply(&p).unwrap();
    super::settings::push(&mac.store, &applied).unwrap();
    assert_eq!(super::settings::patch_from_store(&mac.store, &applied).unwrap(), None);
    let e = &mac.store.entries(&["settings"]).unwrap()["settings"]["crossfadeSeconds"];
    assert_eq!(e.device, "pc", "the mac didn't write it again");
}

#[test]
fn a_new_device_doesnt_override_choices_with_its_defaults() {
    let cloud = tempfile::tempdir().unwrap();
    let (pc, mac) = (Dev::new("pc"), Dev::new("mac"));
    pc.at(3_000_000);
    super::settings::seed(&pc.store, &Settings { show_fan_edits: true, ..Default::default() }).unwrap();
    pc.pass(cloud.path()).unwrap();
    // The mac joins later with every setting at its default.
    mac.at(9_000_000);
    super::settings::seed(&mac.store, &Settings::default()).unwrap();
    mac.pass(cloud.path()).unwrap();
    let p = super::settings::patch_from_store(&mac.store, &Settings::default()).unwrap().unwrap();
    assert_eq!(p.show_fan_edits, Some(true));
}

// ------------------------------------------------------------ follows

#[test]
fn follows_move_from_the_old_list_once_and_then_live_in_the_store() {
    use crate::db::store::{kv_get, kv_set};
    use crate::social::follows::{FollowStore, set_follow};
    use super::follows::{LEGACY_KEY, SyncFollows};
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("lib.db")).unwrap();
    let store = SyncStore::new(db.clone(), "pc");
    let old = vec![
        FollowedArtist { id: "1".into(), title: "Ana".into(), thumb: None, followed_at: 100 },
        FollowedArtist { id: "credit:bo".into(), title: "Bo".into(), thumb: None, followed_at: 50 },
    ];
    db.write(|c| kv_set(c, LEGACY_KEY, &serde_json::to_string(&old).unwrap())).unwrap();
    assert_eq!(SyncFollows::migrate(&store, &db).unwrap(), 2);
    assert_eq!(db.read(|c| kv_get(c, LEGACY_KEY)).unwrap().as_deref(), Some("[]"));
    let f = SyncFollows { store: store.clone() };
    let ids: Vec<String> = f.list().unwrap().into_iter().map(|a| a.id).collect();
    assert_eq!(ids, ["credit:bo", "1"], "in the order they were followed");
    // The first follow date travels with it.
    let e = &store.entries(&["follows"]).unwrap()["follows"]["1"];
    assert_eq!(e.ts, 100_000);
    // Follow and unfollow through the social module's own rules.
    let new = FollowedArtist { id: "9".into(), title: "Cy".into(), thumb: None, followed_at: 200 };
    assert_eq!(set_follow(&f, new, true).unwrap().len(), 3);
    let gone = FollowedArtist { id: "1".into(), title: "Ana".into(), thumb: None, followed_at: 0 };
    assert_eq!(set_follow(&f, gone, false).unwrap().len(), 2);
    assert_eq!(store.deleted(&FOLLOWS).unwrap(), ["1"]);
    // Never again, even with an old list back in place.
    db.write(|c| kv_set(c, LEGACY_KEY, &serde_json::to_string(&old).unwrap())).unwrap();
    assert_eq!(SyncFollows::migrate(&store, &db).unwrap(), 0);
    assert!(f.list().unwrap().iter().all(|a| a.id != "1"));
}

// ------------------------------------------------------------ pins

#[test]
fn pins_toggle_keep_twelve_and_import_once() {
    let d = Dev::new("pc");
    for i in 0..14 {
        d.at(1_000 + i);
        pins::toggle(&d.store, pin("album", &i.to_string())).unwrap();
    }
    let list = pins::list(&d.store).unwrap();
    assert_eq!(list.len(), 12);
    assert_eq!(list[0].id, "13", "newest first");
    assert!(!list.iter().any(|p| p.id == "0" || p.id == "1"), "the oldest went");
    let list = pins::toggle(&d.store, pin("album", "13")).unwrap();
    assert_eq!(list[0].id, "12");
    assert!(pins::toggle(&d.store, pin("track", "1")).is_err());
    // Import only happens into a store that never had pins.
    let fresh = Dev::new("mac");
    let got = pins::import(&fresh.store, vec![pin("artist", "a"), pin("album", "b")]).unwrap();
    assert_eq!(got.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["a", "b"], "order kept");
    pins::toggle(&fresh.store, pin("artist", "a")).unwrap();
    pins::toggle(&fresh.store, pin("album", "b")).unwrap();
    assert!(pins::import(&fresh.store, vec![pin("album", "c")]).unwrap().is_empty());
}

// ------------------------------------------------------------ folders

#[test]
fn folders_create_nest_rename_and_hold_playlists() {
    let d = Dev::new("pc");
    let a = folders::create(&d.store, None, None).unwrap();
    assert_eq!(a.name, "New Folder");
    d.at(2_000_000);
    let b = folders::create(&d.store, Some("Inside"), Some(&a.id)).unwrap();
    folders::place(&d.store, "p1", Some(&b.id)).unwrap();
    folders::place(&d.store, "p2", Some(&a.id)).unwrap();
    folders::rename(&d.store, &a.id, "  Road trips ").unwrap();
    let t = folders::tree(&d.store).unwrap();
    assert_eq!(t.folders[0].id, b.id, "newest first");
    assert_eq!(t.folders.iter().find(|f| f.id == a.id).unwrap().name, "Road trips");
    assert_eq!(t.folders.iter().find(|f| f.id == b.id).unwrap().parent.as_deref(), Some(a.id.as_str()));
    assert_eq!(t.items["p1"], b.id);
    assert!(folders::rename(&d.store, &a.id, "  ").is_err());
    assert!(folders::rename(&d.store, &a.id, &"x".repeat(101)).is_err());
    assert!(folders::place(&d.store, "liked", Some(&a.id)).is_err());
    assert!(folders::place(&d.store, "p3", Some("nope")).is_err());
    // Back out to the top.
    folders::place(&d.store, "p2", None).unwrap();
    assert!(!folders::tree(&d.store).unwrap().items.contains_key("p2"));
}

#[test]
fn deleting_a_folder_moves_what_was_in_it_up() {
    let d = Dev::new("pc");
    let top = folders::create(&d.store, Some("Top"), None).unwrap();
    let mid = folders::create(&d.store, Some("Mid"), Some(&top.id)).unwrap();
    let low = folders::create(&d.store, Some("Low"), Some(&mid.id)).unwrap();
    folders::place(&d.store, "in-mid", Some(&mid.id)).unwrap();
    folders::place(&d.store, "in-top", Some(&top.id)).unwrap();
    folders::delete(&d.store, &mid.id).unwrap();
    let t = folders::tree(&d.store).unwrap();
    assert!(!t.folders.iter().any(|f| f.id == mid.id));
    assert_eq!(t.folders.iter().find(|f| f.id == low.id).unwrap().parent.as_deref(), Some(top.id.as_str()));
    assert_eq!(t.items["in-mid"], top.id, "the playlist moved up, not deleted");
    folders::delete(&d.store, &top.id).unwrap();
    let t = folders::tree(&d.store).unwrap();
    assert_eq!(t.folders.iter().find(|f| f.id == low.id).unwrap().parent, None);
    assert!(t.items.is_empty(), "playlists back at the top of the library");
    assert!(folders::delete(&d.store, &top.id).is_err());
}

#[test]
fn folders_cant_go_inside_themselves_and_merges_are_tidied() {
    let d = Dev::new("pc");
    let a = folders::create(&d.store, Some("A"), None).unwrap();
    let b = folders::create(&d.store, Some("B"), Some(&a.id)).unwrap();
    assert!(folders::move_folder(&d.store, &a.id, Some(&b.id)).is_err());
    assert!(folders::move_folder(&d.store, &a.id, Some(&a.id)).is_err());
    folders::move_folder(&d.store, &b.id, None).unwrap();
    // Two devices each moved one folder into the other: a loop after merging.
    let mut m = EntryMap::new();
    let put = |m: &mut EntryMap, id: &str, parent: &str| {
        m.entry("folders".into()).or_default().insert(
            id.into(),
            Entry { value: Some(json!({ "name": id, "parent": parent, "createdAt": 1 })), ts: 9_000_000, device: "mac".into() },
        );
    };
    put(&mut m, &a.id, &b.id);
    put(&mut m, &b.id, &a.id);
    // And a playlist placed in a folder the merge doesn't have.
    m.entry("folderItems".into())
        .or_default()
        .insert("p".into(), Entry { value: Some(json!({ "folder": "gone" })), ts: 9_000_000, device: "mac".into() });
    d.store.merge(&m, &["folders", "folderItems"]).unwrap();
    let t = folders::tree(&d.store).unwrap();
    let top: Vec<&str> = t.folders.iter().filter(|f| f.parent.is_none()).map(|f| f.id.as_str()).collect();
    assert_eq!(top.len(), 1, "the loop is broken at one folder");
    assert!(!t.items.contains_key("p"), "shows at the top");
    // Depth is capped.
    let mut parent = folders::create(&d.store, Some("0"), None).unwrap().id;
    for i in 1..folders::DEPTH_MAX {
        parent = folders::create(&d.store, Some(&i.to_string()), Some(&parent)).unwrap().id;
    }
    assert!(folders::create(&d.store, None, Some(&parent)).is_err());
}

#[test]
fn folders_travel_between_devices() {
    let cloud = tempfile::tempdir().unwrap();
    let (pc, mac) = (Dev::new("pc"), Dev::new("mac"));
    let f = folders::create(&pc.store, Some("Gym"), None).unwrap();
    folders::place(&pc.store, "p1", Some(&f.id)).unwrap();
    pc.pass(cloud.path()).unwrap();
    mac.pass(cloud.path()).unwrap();
    let t = folders::tree(&mac.store).unwrap();
    assert_eq!(t.folders[0].name, "Gym");
    assert_eq!(t.items["p1"], f.id);
}

// ------------------------------------------------------------ the engine

struct Tag(Option<&'static str>);
#[async_trait::async_trait]
impl engine::AccountTag for Tag {
    async fn tag(&self) -> Option<String> {
        self.0.map(str::to_string)
    }
}

fn fast() -> Timing {
    Timing { debounce: Duration::from_millis(50), poll: Duration::from_millis(30), reread: Duration::from_secs(60) }
}

async fn eventually(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn the_engine_writes_after_a_quiet_moment_and_rereads_when_the_file_changes() {
    let cloud = tempfile::tempdir().unwrap();
    let (pc, mac) = (Dev::new("pc"), Dev::new("mac"));
    let (tx, rx) = tokio::sync::watch::channel(SyncConfig { enabled: true, folder: Some(cloud.path().to_path_buf()), groups: Groups::default() });
    let engine = SyncEngine::new(pc.store.clone(), Arc::new(Tag(Some(TAG))), rx, Arc::new(|| "PC".into()), fast());
    let task = tokio::spawn(engine.clone().run());
    assert!(eventually(|| engine.status().state == SyncState::Ok).await);
    // A local change goes out after the debounce.
    pins::toggle(&pc.store, pin("album", "1")).unwrap();
    let path = file_path(cloud.path());
    assert!(
        eventually(|| file::read(&path).ok().flatten().is_some_and(|d| d.entries.get("pins").is_some_and(|p| p.contains_key("album:1"))))
            .await
    );
    // Another device writes the file: the engine notices and takes it in.
    mac.store.put(&FOLLOWS, "z", &follow("from the mac")).unwrap();
    mac.pass(cloud.path()).unwrap();
    assert!(eventually(|| pc.store.get(&FOLLOWS, "z").unwrap().is_some()).await, "watcher reload");
    // Turning sync off shows Off.
    tx.send(SyncConfig::default()).unwrap();
    assert!(eventually(|| engine.status().state == SyncState::Off).await);
    task.abort();
}

#[tokio::test]
async fn engine_states_for_missing_folder_unreadable_file_and_signed_out() {
    let cloud = tempfile::tempdir().unwrap();
    let pc = Dev::new("pc");
    let missing = cloud.path().join("gone");
    let cfg = |f: &Path| SyncConfig { enabled: true, folder: Some(f.to_path_buf()), groups: Groups::default() };
    let (_tx, rx) = tokio::sync::watch::channel(cfg(&missing));
    let e = SyncEngine::new(pc.store.clone(), Arc::new(Tag(Some(TAG))), rx, Arc::new(|| "PC".into()), fast());
    e.sync_now().await;
    let s = e.status();
    assert_eq!(s.state, SyncState::FolderMissing);
    assert!(s.message.unwrap().contains("kept on this computer"));

    std::fs::create_dir_all(file::dir(cloud.path())).unwrap();
    std::fs::write(file_path(cloud.path()), "garbage").unwrap();
    let (_tx, rx) = tokio::sync::watch::channel(cfg(cloud.path()));
    let e = SyncEngine::new(pc.store.clone(), Arc::new(Tag(Some(TAG))), rx, Arc::new(|| "PC".into()), fast());
    e.sync_now().await;
    assert_eq!(e.status().state, SyncState::Unreadable);
    e.replace_file().await.unwrap();
    assert_eq!(e.status().state, SyncState::Ok);
    let aside = std::fs::read_dir(file::dir(cloud.path())).unwrap().filter(|x| {
        x.as_ref().unwrap().file_name().to_string_lossy().contains(".broken-")
    });
    assert_eq!(aside.count(), 1, "the old file is kept");

    let (_tx, rx) = tokio::sync::watch::channel(cfg(cloud.path()));
    let e = SyncEngine::new(pc.store.clone(), Arc::new(Tag(None)), rx, Arc::new(|| "PC".into()), fast());
    e.sync_now().await;
    assert_eq!(e.status().state, SyncState::SignedOut);
}

// ------------------------------------------------------------ detection

#[test]
fn cloud_folders_are_found_on_windows_and_macos() {
    use super::detect::{Probe, detect};
    use std::path::PathBuf;
    let dirs = [
        r"C:\Users\a\OneDrive",
        r"C:\Users\a\iCloudDrive",
        r"G:\My Drive",
        r"D:\Dropbox",
        "/Users/a/Library/Mobile Documents/com~apple~CloudDocs",
        "/Users/a/Library/CloudStorage/OneDrive-Personal",
        "/Users/a/Library/CloudStorage/GoogleDrive-a@example.com/My Drive",
        "/Users/a/Dropbox",
    ];
    let is_dir = |p: &Path| dirs.iter().any(|d| Path::new(d) == p);
    let env = |k: &str| match k {
        "OneDrive" => Some(r"C:\Users\a\OneDrive".to_string()),
        "APPDATA" => Some(r"C:\Users\a\AppData\Roaming".to_string()),
        _ => None,
    };
    let read = |p: &Path| p.ends_with("info.json").then(|| r#"{"personal":{"path":"D:\\Dropbox"}}"#.to_string());
    let list = |_: &Path| {
        vec![
            PathBuf::from("/Users/a/Library/CloudStorage/OneDrive-Personal"),
            PathBuf::from("/Users/a/Library/CloudStorage/GoogleDrive-a@example.com"),
        ]
    };
    let win = detect(&Probe { home: Some(PathBuf::from(r"C:\Users\a")), env: &env, is_dir: &is_dir, read: &read, list: &list, windows: true });
    let names: Vec<&str> = win.iter().map(|f| f.provider.as_str()).collect();
    assert_eq!(names, ["OneDrive", "iCloud Drive", "Google Drive", "Dropbox"]);
    let mac = detect(&Probe { home: Some(PathBuf::from("/Users/a")), env: &|_| None, is_dir: &is_dir, read: &|_| None, list: &list, windows: false });
    let names: Vec<&str> = mac.iter().map(|f| f.provider.as_str()).collect();
    assert_eq!(names, ["iCloud Drive", "Google Drive", "OneDrive", "Dropbox"]);
    let none = detect(&Probe { home: None, env: &|_| None, is_dir: &|_| false, read: &|_| None, list: &|_| Vec::new(), windows: true });
    assert!(none.is_empty());
}

// ------------------------------------------------------------ key copy

mod keys {
    use std::collections::HashMap;
    use std::net::Ipv4Addr;

    use async_trait::async_trait;
    use parking_lot::Mutex;
    use url::Url;

    use super::*;
    use crate::error::AppResult;
    use crate::remote::peer::auth::PeerKey;
    use crate::remote::peer::client::PeerClient;
    use crate::remote::peer::server::{self, KeySource, PeerHandler, PeerServer};
    use crate::remote::peer::{Hello, PeerDevice, PeerTimeline, PlayRequest};
    use crate::remote::types::RemoteCommand;
    use crate::sync::keycopy::{self, KeyKind, KeyReceiver, KeyVault, ReceivePhase, Secret, crypto};

    #[derive(Default)]
    struct Vault(Mutex<HashMap<KeyKind, String>>);

    #[async_trait]
    impl KeyVault for Vault {
        async fn available(&self) -> Vec<KeyKind> {
            KeyKind::ALL.into_iter().filter(|k| self.0.lock().contains_key(k)).collect()
        }
        async fn read(&self, kind: KeyKind) -> Option<Secret> {
            self.0.lock().get(&kind).map(|v| Secret::new(v.clone()))
        }
        async fn store(&self, kind: KeyKind, value: Secret) -> AppResult<()> {
            self.0.lock().insert(kind, value.expose().to_string());
            Ok(())
        }
    }

    struct NoPlayer;
    #[async_trait]
    impl PeerHandler for NoPlayer {
        async fn hello(&self) -> Hello {
            Hello { id: "den".into(), name: "Den".into(), platform: "Windows".into(), version: "0".into(), playing: None }
        }
        async fn timeline(&self) -> PeerTimeline {
            PeerTimeline { state: "stopped".into(), time_ms: 0, duration_ms: 0, track_id: None, volume: 0.0, shuffle: false, repeat: Default::default() }
        }
        async fn play(&self, _: PlayRequest) -> AppResult<()> {
            Ok(())
        }
        async fn command(&self, _: RemoteCommand) -> AppResult<()> {
            Ok(())
        }
        async fn stop(&self) -> AppResult<()> {
            Ok(())
        }
    }

    struct Fixed(PeerKey);
    #[async_trait]
    impl KeySource for Fixed {
        async fn key(&self) -> Option<PeerKey> {
            Some(self.0.clone())
        }
    }

    fn key(t: &str) -> PeerKey {
        PeerKey::from_token(&crate::plex::Token::new(t).unwrap())
    }

    struct Rig {
        receiver: KeyReceiver,
        got: Arc<Vault>,
        clock: Arc<AtomicU64>,
        device: PeerDevice,
        stop: tokio::sync::watch::Sender<bool>,
    }

    /// The device that needs keys ("den"), answering on loopback.
    async fn receiver(token: &str) -> Rig {
        let got = Arc::new(Vault::default());
        let clock = Arc::new(AtomicU64::new(10_000));
        let c = clock.clone();
        let receiver = KeyReceiver::new("den".into(), got.clone(), Arc::new(move || c.load(Ordering::SeqCst)));
        // Key copy answers through the control port's extra routes.
        let slot = server::ExtraSlot::default();
        let routes = crate::remote::peer::keys::KeyCopyRoutes::new("den".into(), Arc::new(Fixed(key(token))), Arc::new(receiver.clone()));
        *slot.write() = Some(Arc::new(crate::remote::peer::keys::RouteList(vec![Arc::new(routes)])));
        let srv = Arc::new(PeerServer::new("den".into(), Arc::new(NoPlayer), Arc::new(Fixed(key(token)))).with_extra(slot));
        let (listeners, port) = server::bind(&[Ipv4Addr::LOCALHOST], 0, 1).await.unwrap();
        let (stop, rx) = tokio::sync::watch::channel(false);
        tokio::spawn(server::serve(listeners, srv, rx));
        let device = PeerDevice {
            id: "den".into(),
            name: "Den".into(),
            platform: "Windows".into(),
            bases: vec![Url::parse(&format!("http://127.0.0.1:{port}")).unwrap()],
        };
        Rig { receiver, got, clock, device, stop }
    }

    /// The device that has keys ("studio").
    fn sender(token: &str) -> (PeerClient, Vault) {
        let v = Vault::default();
        v.0.lock().insert(KeyKind::Lastfm, "lastfm-secret-1".into());
        v.0.lock().insert(KeyKind::Listenbrainz, "lb-secret-2".into());
        v.0.lock().insert(KeyKind::Imgur, "imgur-secret-3".into());
        (PeerClient::new(reqwest::Client::new(), "studio".into(), key(token)), v)
    }

    #[tokio::test]
    async fn the_right_code_copies_the_ticked_keys() {
        let rig = receiver("acct").await;
        let (client, vault) = sender("acct");
        let st = rig.receiver.start();
        assert_eq!(st.phase, ReceivePhase::Waiting);
        let code = st.code.clone().unwrap();
        assert_eq!(code.len(), 6);
        assert!(st.qr.as_ref().unwrap().size >= 21);
        let out = keycopy::send(&client, "Studio", vec![rig.device.clone()], &code, &[KeyKind::Lastfm, KeyKind::Imgur, KeyKind::Fanart], &vault)
            .await
            .unwrap();
        assert_eq!(out.device, "Den");
        assert_eq!(out.sent, [KeyKind::Lastfm, KeyKind::Imgur], "Fanart isn't here; ListenBrainz wasn't ticked");
        let got = rig.got.0.lock().clone();
        assert_eq!(got.get(&KeyKind::Lastfm).map(String::as_str), Some("lastfm-secret-1"));
        assert_eq!(got.get(&KeyKind::Imgur).map(String::as_str), Some("imgur-secret-3"));
        assert!(!got.contains_key(&KeyKind::Listenbrainz));
        let st = rig.receiver.state();
        assert_eq!(st.phase, ReceivePhase::Received);
        assert_eq!(st.from.as_deref(), Some("Studio"));
        assert!(st.code.is_none(), "the code is used up");
        // Sending again finds nobody waiting.
        assert!(keycopy::send(&client, "Studio", vec![rig.device.clone()], &code, &[KeyKind::Lastfm], &vault).await.is_err());
        let _ = rig.stop.send(true);
    }

    #[tokio::test]
    async fn a_wrong_code_sends_nothing_and_too_many_lock_the_code() {
        let rig = receiver("acct").await;
        let (client, vault) = sender("acct");
        let code = rig.receiver.start().code.unwrap();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        let e = keycopy::send(&client, "Studio", vec![rig.device.clone()], wrong, &[KeyKind::Lastfm], &vault).await.unwrap_err();
        assert!(e.to_string().contains("doesn't match"), "{e}");
        assert!(rig.got.0.lock().is_empty());
        // The right code still works afterwards.
        keycopy::send(&client, "Studio", vec![rig.device.clone()], &code, &[KeyKind::Lastfm], &vault).await.unwrap();
        assert_eq!(rig.got.0.lock().len(), 1);

        let code = rig.receiver.start().code.unwrap();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        for _ in 0..keycopy::TRIES {
            assert!(keycopy::send(&client, "S", vec![rig.device.clone()], wrong, &[KeyKind::Lastfm], &vault).await.is_err());
        }
        assert!(keycopy::send(&client, "S", vec![rig.device.clone()], &code, &[KeyKind::Lastfm], &vault).await.is_err());
        assert_eq!(rig.receiver.state().phase, ReceivePhase::Locked);
        let _ = rig.stop.send(true);
    }

    #[tokio::test]
    async fn codes_time_out_after_five_minutes() {
        let rig = receiver("acct").await;
        let (client, vault) = sender("acct");
        let code = rig.receiver.start().code.unwrap();
        rig.clock.fetch_add(keycopy::CODE_TTL_MS, Ordering::SeqCst);
        assert_eq!(rig.receiver.state().phase, ReceivePhase::Expired);
        let e = keycopy::send(&client, "Studio", vec![rig.device.clone()], &code, &[KeyKind::Lastfm], &vault).await.unwrap_err();
        assert!(e.to_string().contains("No device is showing a code"), "{e}");
        assert!(rig.got.0.lock().is_empty());
        let _ = rig.stop.send(true);
    }

    #[tokio::test]
    async fn another_account_cant_even_ask() {
        let rig = receiver("acct").await;
        let (client, vault) = sender("someone-else");
        let code = rig.receiver.start().code.unwrap();
        assert!(keycopy::send(&client, "Studio", vec![rig.device.clone()], &code, &[KeyKind::Lastfm], &vault).await.is_err());
        assert!(rig.got.0.lock().is_empty());
        assert_eq!(rig.receiver.state().phase, ReceivePhase::Waiting, "no try used up");
        let _ = rig.stop.send(true);
    }

    #[tokio::test]
    async fn sending_checks_the_code_and_the_choice_first() {
        let (client, vault) = sender("acct");
        let none: Vec<PeerDevice> = Vec::new();
        assert!(keycopy::send(&client, "S", none.clone(), "12345", &[KeyKind::Lastfm], &vault).await.is_err());
        assert!(keycopy::send(&client, "S", none.clone(), "123 456", &[KeyKind::Fanart], &vault).await.is_err(), "nothing to send");
        let e = keycopy::send(&client, "S", none, "123-456", &[KeyKind::Lastfm], &vault).await.unwrap_err();
        assert!(e.to_string().contains("No device"));
        assert_eq!(keycopy::valid_code(" 123 456 "), Some("123456".into()));
    }

    #[test]
    fn sealed_keys_open_only_with_the_same_code_and_devices() {
        let (s, msg_a) = crypto::sender_start("123456", "studio", "den");
        let (msg_b, rk) = crypto::receiver_answer("123456", "studio", "den", &msg_a).unwrap();
        let sk = crypto::sender_finish(s, &msg_b).unwrap();
        assert_eq!(sk.confirmation(), rk.confirmation());
        let (n, ct) = crypto::seal(&sk, "studio", "den", b"keys").unwrap();
        assert_eq!(crypto::open(&rk, "studio", "den", &n, &ct).unwrap(), b"keys");
        assert!(crypto::open(&rk, "studio", "other", &n, &ct).is_err(), "bound to the devices");
        let mut bad = ct.clone();
        bad[0] ^= 1;
        assert!(crypto::open(&rk, "studio", "den", &n, &bad).is_err());
        // A wrong code reaches another key.
        let (s, msg_a) = crypto::sender_start("123456", "studio", "den");
        let (msg_b, rk) = crypto::receiver_answer("654321", "studio", "den", &msg_a).unwrap();
        let sk = crypto::sender_finish(s, &msg_b).unwrap();
        assert_ne!(sk.confirmation(), rk.confirmation());
        assert_eq!(format!("{sk:?}"), "SharedKey(..)");
        assert_eq!(format!("{:?}", Secret::new("x")), "Secret(..)");
        assert!(crypto::unhex("zz").is_err() && crypto::unhex("é1").is_err());
    }
}
