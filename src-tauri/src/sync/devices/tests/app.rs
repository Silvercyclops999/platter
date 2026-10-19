//! Device sync as the app runs it: the Keep my devices in sync switch,
//! status for Settings, Copy settings from my computer, and the feeds that
//! carry looks, downloads and imported plays.

use super::pairing::{ACCT_TOKEN, Host, host};
use super::*;
use crate::error::AppError;
use crate::remote::peer::client::{PeerClient, PeerError};
use crate::remote::peer::sync::LanBody;
use crate::sync::devices::feeds::{Applied, FeedTiming, Feeds};
use crate::sync::devices::lan::LanSend;
use crate::sync::devices::run::DeviceStatus;

/// Straight to the other device's control port (what mDNS would find).
struct Direct(PeerClient, url::Url);

#[async_trait]
impl LanSend for Direct {
    async fn post(&self, device: &str, path: &str, body: &LanBody) -> Result<LanBody, PeerError> {
        self.0.sync_exchange(&self.1, device, path, body, Duration::from_secs(5)).await
    }
}

/// Two hosts paired with the 6-digit code, on one network.
async fn paired() -> (Host, Host) {
    let pc = host(real_clock("pc", ACCT), ACCT_TOKEN, "http://relay").await;
    let phone = host(real_clock("phone", ACCT), ACCT_TOKEN, "http://relay").await;
    let code = pc.pairing.start().await.unwrap().code.unwrap();
    phone.pairing.join(&code, Some((phone.client.clone(), vec![pc.device.clone()]))).await.unwrap();
    pc.sync.set_lan(Arc::new(Direct(pc.client.clone(), phone.device.bases[0].clone())));
    phone.sync.set_lan(Arc::new(Direct(phone.client.clone(), pc.device.bases[0].clone())));
    (pc, phone)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn copy_settings_from_my_computer_wins_here_and_everywhere() {
    let (pc, phone) = paired().await;
    pc.dev.store.put(&SETTINGS, "accent", &json!("#c90")).unwrap();
    pc.dev.store.put(&HIDDEN, "track:1", &hidden("One")).unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    // Chosen on the phone later, so it would win a normal round.
    phone.dev.store.put(&SETTINGS, "accent", &json!("#09c")).unwrap();
    let mut changes = phone.dev.store.subscribe();
    let name = phone.sync.copy_settings(Duration::from_secs(5)).await.unwrap();
    assert_eq!(name, "Device pc");
    assert_eq!(phone.dev.store.get(&SETTINGS, "accent").unwrap(), Some(json!("#c90")), "the computer's setting");
    // It arrives as a change from another device, so Settings applies it.
    let c = changes.recv().await.unwrap();
    assert!(c.remote && c.namespaces.iter().any(|n| n == "settings"), "{c:?}");
    // And it stays that way on both after the next rounds.
    phone.sync.sync_now().await;
    pc.sync.sync_now().await;
    assert_eq!(pc.dev.store.get(&SETTINGS, "accent").unwrap(), Some(json!("#c90")));
    assert_eq!(phone.dev.store.get(&SETTINGS, "accent").unwrap(), Some(json!("#c90")));
    assert!(same(&pc.dev, &phone.dev));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn copy_settings_needs_a_paired_device_that_answers() {
    let lone = host(real_clock("lone", ACCT), ACCT_TOKEN, "http://relay").await;
    let e = lone.sync.copy_settings(Duration::from_millis(200)).await.unwrap_err();
    assert!(matches!(e, AppError::NotFound(_)), "{e}");
    assert!(e.to_string().contains("Pair this device"), "{e}");
    // Paired, but the computer is off: it says so, and nothing changes.
    let (pc, phone) = paired().await;
    phone.dev.store.put(&SETTINGS, "accent", &json!("#09c")).unwrap();
    pc.sync.set_switch(Arc::new(|| false));
    phone.sync.set_lan(Arc::new(Direct(phone.client.clone(), url::Url::parse("http://127.0.0.1:9").unwrap())));
    let e = phone.sync.copy_settings(Duration::from_millis(500)).await.unwrap_err();
    assert!(matches!(e, AppError::Network(_)), "{e}");
    assert!(e.to_string().contains("didn't answer"), "{e}");
    assert_eq!(phone.dev.store.get(&SETTINGS, "accent").unwrap(), Some(json!("#09c")));
    // Off here: refused before anything goes.
    phone.sync.set_switch(Arc::new(|| false));
    let e = phone.sync.copy_settings(Duration::from_millis(200)).await.unwrap_err();
    assert!(e.to_string().contains("Turn on"), "{e}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn copy_settings_says_so_at_once_when_there_are_none_or_they_dont_sync() {
    let (pc, phone) = paired().await;
    // The computer has no synced settings at all: an answer, not a wait.
    let started = std::time::Instant::now();
    let e = phone.sync.copy_settings(Duration::from_secs(20)).await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5), "answered, not timed out");
    assert!(e.to_string().contains("has no synced settings to copy"), "{e}");
    // Settings switched off here: refused before anything goes.
    phone.dev.groups.lock().settings = false;
    let e = phone.sync.copy_settings(Duration::from_secs(20)).await.unwrap_err();
    assert!(e.to_string().contains("Settings aren't kept in sync on this device"), "{e}");
    phone.dev.groups.lock().settings = true;
    // Switched off on the computer: it says so once it knows.
    pc.dev.groups.lock().settings = false;
    pc.dev.store.put(&SETTINGS, "accent", &json!("#c90")).unwrap();
    phone.sync.sync_now().await;
    let e = phone.sync.copy_settings(Duration::from_secs(20)).await.unwrap_err();
    assert!(e.to_string().contains("doesn't keep its settings in sync"), "{e}");
}

#[tokio::test]
async fn a_normal_push_with_settings_isnt_taken_as_the_copy() {
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, "http://relay");
    round(&a, &la, &b, &lb).await;
    let _want = a.peer.copy_settings(&la);
    // A change from b that happens to carry settings arrives first.
    b.store.put(&SETTINGS, "accent", &json!("#123")).unwrap();
    for m in b.peer.push(&lb).await {
        a.peer.handle(&la, m).await;
    }
    assert_eq!(a.peer.copying(&la.id), Some(crate::sync::devices::protocol::Copying::Waiting), "still waiting for the copy itself");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_keep_my_devices_in_sync_off_nothing_goes_or_comes() {
    let (pc, phone) = paired().await;
    let on = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let o = on.clone();
    pc.sync.set_switch(Arc::new(move || o.load(Ordering::SeqCst)));
    pc.dev.store.put(&HIDDEN, "track:pc", &hidden("from the computer")).unwrap();
    phone.dev.store.put(&HIDDEN, "track:phone", &hidden("from the phone")).unwrap();
    pc.sync.sync_now().await;
    pc.sync.push_all().await;
    assert!(!phone.dev.store.has_entry(&HIDDEN, "track:pc").unwrap(), "nothing sent while off");
    // The phone asks; the computer turns it away and takes nothing.
    phone.sync.sync_now().await;
    assert!(!pc.dev.store.has_entry(&HIDDEN, "track:phone").unwrap(), "nothing taken while off");
    assert_eq!(pc.sync.status()[0].words, "Keep my devices in sync is off");
    // On again: they catch up.
    on.store(true, Ordering::SeqCst);
    pc.sync.sync_now().await;
    assert!(same(&pc.dev, &phone.dev));
    assert_eq!(pc.sync.status()[0].words, "In sync");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settings_hears_when_a_devices_status_changes() {
    let (pc, phone) = paired().await;
    let heard: Arc<Mutex<Vec<Vec<DeviceStatus>>>> = Arc::default();
    let h = heard.clone();
    pc.sync.set_on_status(Arc::new(move |s: &[DeviceStatus]| h.lock().push(s.to_vec())));
    pc.sync.status_changed();
    pc.sync.status_changed();
    assert_eq!(heard.lock().len(), 1, "told once, not again for the same status");
    pc.dev.store.put(&HIDDEN, "track:1", &hidden("One")).unwrap();
    pc.sync.sync_now().await;
    pc.sync.status_changed();
    let last = heard.lock().last().cloned().unwrap();
    assert_eq!(last[0].words, "In sync");
    assert_eq!(last[0].name, "Device phone");
    assert!(last[0].last_synced_at.is_some());
    let _ = phone;
}

// ---------------------------------------------------------------- feeds

const LOOK: &str = r##"{"version":1,"name":"Dusk","base":"dark","tokens":{"color-accent":"#c08a3e"}}"##;

fn feeds(d: &Dev, themes: &std::path::Path, items: Vec<crate::downloads::store::ItemRow>, groups: Arc<Mutex<Groups>>) -> Arc<Feeds> {
    let name = format!("Device {}", d.id);
    Feeds::new(
        d.store.clone(),
        Some(themes.to_path_buf()),
        Arc::new(move || items.clone()),
        Arc::new(move || name.clone()),
        Arc::new(move || *groups.lock()),
        FeedTiming { every: Duration::from_millis(100), settle: Duration::from_millis(100) },
    )
}

fn album(id: &str, title: &str) -> crate::downloads::store::ItemRow {
    use crate::downloads::store::{ItemKey, ItemKind, ItemRow};
    ItemRow { key: ItemKey::new(ItemKind::Album, id), title: title.into(), added_at: 7 }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_feeds_carry_looks_downloads_and_plays_between_devices() {
    let (a, b) = (dev("pc"), dev("phone"));
    let (la, lb) = pair(&a, &b, "http://relay");
    let (ta, tb) = (a._dir.path().join("themes"), b._dir.path().join("themes"));
    crate::themes::import_text(&ta, LOOK).unwrap();
    let play = crate::db::listens::ImportedPlay {
        source: crate::db::listens::ImportSource::Lastfm,
        source_id: "lf1".into(),
        played_at: 1_700_000_000_000,
        ms_played: None,
        title: "Lullaby".into(),
        artist: None,
        album: None,
        rating_key: None,
    };
    a.db.write(|c| crate::db::listens::insert_imported(c, &[play], 1)).unwrap();
    let fa = feeds(&a, &ta, vec![album("a1", "Blue")], a.groups.clone());
    let fb = feeds(&b, &tb, Vec::new(), b.groups.clone());
    assert_eq!(fa.publish().unwrap(), 3, "a look, a download, a day of plays");
    assert_eq!(fa.publish().unwrap(), 0, "nothing new");
    let (stop, rx) = tokio::sync::watch::channel(false);
    let applied: Arc<Mutex<Vec<Applied>>> = Arc::default();
    let ap = applied.clone();
    fb.set_on_applied(Arc::new(move |x| ap.lock().push(x.clone())));
    tokio::spawn(fb.clone().run(rx));
    tokio::time::sleep(Duration::from_millis(50)).await;
    round(&a, &la, &b, &lb).await;
    until("the look in the phone's folder", || crate::themes::list_dir(&tb).themes.len() == 1).await;
    until("the play in the phone's history", || {
        b.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM imported_plays", [], |r| r.get::<_, i64>(0))?)).unwrap() == 1
    })
    .await;
    assert!(applied.lock().iter().any(|x| x.looks == 1));
    // The download is only a suggestion on the phone: nothing downloads.
    let s = crate::sync::download_intent::suggestions(&b.store, &[]).unwrap();
    assert_eq!((s.len(), s[0].title.as_str(), s[0].devices[0].as_str()), (1, "Blue", "Device pc"));
    let n: i64 = b.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM download_items", [], |r| r.get(0))?)).unwrap();
    assert_eq!(n, 0);
    let _ = stop.send(true);
}

#[tokio::test]
async fn a_feed_whose_group_is_off_neither_stores_nor_applies() {
    let a = dev("pc");
    let themes = a._dir.path().join("themes");
    crate::themes::import_text(&themes, LOOK).unwrap();
    *a.groups.lock() = Groups { looks: false, downloads: false, plays: false, ..Groups::default() };
    let f = feeds(&a, &themes, vec![album("a1", "Blue")], a.groups.clone());
    assert_eq!(f.publish().unwrap(), 0);
    assert!(a.store.list(&store::LOOKS).unwrap().is_empty());
    assert!(a.store.list(&store::DOWNLOAD_INTENT).unwrap().is_empty());
    // Entries from elsewhere wait in the store, untouched.
    let other = themes.join("..").join("other");
    let b = dev("mac");
    crate::themes::import_text(&other, LOOK).unwrap();
    crate::sync::looks::publish(&b.store, &other).unwrap();
    let e = b.store.entries(&[store::LOOKS.name()]).unwrap();
    a.store.merge(&e, &[store::LOOKS.name()]).unwrap();
    std::fs::remove_dir_all(&themes).unwrap();
    assert_eq!(f.apply(&[store::LOOKS.name().to_string()]).unwrap(), Applied::default());
    assert!(!themes.exists());
}
