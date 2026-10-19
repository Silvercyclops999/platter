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
pub(super) async fn paired() -> (Host, Host) {
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

// ---------------------------------------------------------------- copy from my computer

use crate::sync::devices::copy::CopyProgress;
use crate::sync::devices::run::Wake;

fn pin(id: &str) -> crate::sync::pins::Pin {
    serde_json::from_value(json!({"kind": "album", "id": id, "title": "A"})).unwrap()
}

/// The computer as it is before the phone is set up.
fn computer_has_things(pc: &Dev) {
    pc.store.put(&SETTINGS, "accent", &json!("#c90")).unwrap();
    pc.store.put(&PINS, "album:a1", &pin("a1")).unwrap();
    pc.store.put(&PINS, "album:a2", &pin("a2")).unwrap();
    pc.store.put(&FOLLOWS, "ar1", &serde_json::from_value(json!({"id": "ar1", "title": "B", "followedAt": 1})).unwrap()).unwrap();
    pc.store.put(&HIDDEN, "track:1", &hidden("One")).unwrap();
    pc.store.put(&store::LYRICS, "song:1", &store::UserLyricsEntry { text: "words".into(), prefer: false }).unwrap();
}

fn heard(sync: &DeviceSync) -> Arc<Mutex<Vec<CopyProgress>>> {
    let all: Arc<Mutex<Vec<CopyProgress>>> = Arc::default();
    let a = all.clone();
    sync.set_on_copy(Arc::new(move |p: &CopyProgress| a.lock().push(p.clone())));
    all
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn copy_from_my_computer_goes_group_by_group_with_counts() {
    let (pc, phone) = paired().await;
    computer_has_things(&pc.dev);
    let steps = heard(&phone.sync);
    let r = phone.sync.copy_from_computer(Duration::from_secs(5)).await.unwrap();
    assert_eq!(r.name, "Device pc");
    assert!(!r.cancelled && r.later.is_none(), "{r:?}");
    let got: Vec<(&str, usize)> = r.groups.iter().map(|g| (g.label.as_str(), g.count)).collect();
    assert_eq!(
        got,
        [("Settings", 1), ("Pins and folders", 2), ("Follows and hidden items", 2), ("Lyrics", 1), ("Looks", 0), ("Download choices", 0), ("Imported plays", 0)]
    );
    let steps = steps.lock().clone();
    assert_eq!(steps.len(), 7);
    assert_eq!(steps[1], CopyProgress { name: "Device pc".into(), step: 2, of: 7, label: "Copying your pins and folders".into() });
    assert!(steps.iter().enumerate().all(|(i, s)| s.step == i + 1 && s.of == 7));
    assert!(same(&pc.dev, &phone.dev));
    // The computer sees the phone's groups as they are again.
    let link = pc.dev.links.all()[0].id.clone();
    assert!(pc.dev.peer.seen(&link).unwrap().skip.iter().all(|n| n != "pins"), "pins aren't left switched off");
    assert_eq!(phone.sync.status()[0].words, "In sync");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stopped_copy_keeps_what_came_and_the_next_round_brings_the_rest() {
    let (pc, phone) = paired().await;
    computer_has_things(&pc.dev);
    let steps = heard(&phone.sync);
    {
        // Stop it as it moves to the second group.
        let s = phone.sync.clone();
        let st = steps.clone();
        phone.sync.set_on_copy(Arc::new(move |p: &CopyProgress| {
            st.lock().push(p.clone());
            if p.step == 2 {
                s.cancel_copy();
            }
        }));
    }
    let r = phone.sync.copy_from_computer(Duration::from_secs(5)).await.unwrap();
    assert!(r.cancelled);
    assert_eq!(r.groups.iter().map(|g| (g.label.as_str(), g.count)).collect::<Vec<_>>(), [("Settings", 1), ("Pins and folders", 0)]);
    assert_eq!(phone.dev.store.get(&SETTINGS, "accent").unwrap(), Some(json!("#c90")), "what came stays");
    assert!(phone.dev.store.list(&PINS).unwrap().is_empty(), "nothing after the stop");
    let link = pc.dev.links.all()[0].id.clone();
    assert!(pc.dev.peer.seen(&link).unwrap().skip.iter().all(|n| n != "pins"));
    // A stop with no copy running does nothing.
    phone.sync.cancel_copy();
    phone.sync.sync_now().await;
    assert!(same(&pc.dev, &phone.dev), "the next round finishes it");
    assert_eq!(phone.dev.store.list(&PINS).unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_copy_waits_for_wifi_and_needs_a_paired_computer() {
    let lone = host(real_clock("lone", ACCT), ACCT_TOKEN, "http://relay").await;
    let e = lone.sync.copy_from_computer(Duration::from_millis(200)).await.unwrap_err();
    assert!(e.to_string().contains("Pair this device"), "{e}");
    let (pc, phone) = paired().await;
    computer_has_things(&pc.dev);
    phone.sync.set_metered(true);
    let e = phone.sync.copy_from_computer(Duration::from_secs(5)).await.unwrap_err();
    assert!(e.to_string().contains("Copying waits for Wi-Fi"), "{e}");
    assert!(phone.dev.store.list(&PINS).unwrap().is_empty());
    // Send big things only on Wi-Fi off: it goes on mobile data too.
    phone.sync.set_rules(Arc::new(|| true), Arc::new(|| false));
    let r = phone.sync.copy_from_computer(Duration::from_secs(5)).await.unwrap();
    assert_eq!(r.groups[1].count, 2);
    // Off here: refused.
    phone.sync.set_switch(Arc::new(|| false));
    let e = phone.sync.copy_from_computer(Duration::from_secs(5)).await.unwrap_err();
    assert!(e.to_string().contains("Turn on Keep my devices in sync"), "{e}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_copy_from_a_computer_with_its_settings_off_brings_the_rest() {
    let (pc, phone) = paired().await;
    computer_has_things(&pc.dev);
    pc.dev.groups.lock().settings = false;
    phone.sync.sync_now().await;
    let r = phone.sync.copy_from_computer(Duration::from_secs(5)).await.unwrap();
    assert_eq!(r.groups[0].count, 0, "no settings");
    assert_eq!(phone.dev.store.list(&PINS).unwrap().len(), 2, "but the pins came");
}

// ---------------------------------------------------------------- away from home and mobile data

fn relay_sync(d: &Dev) -> DeviceSync {
    let wire = Arc::new(Wire::new(d.links.clone(), Arc::new(Ntfy::new().unwrap()), Arc::new(crate::sync::store::system_ms)));
    DeviceSync::new(d.peer.clone(), wire, Timing::default())
}

#[tokio::test]
async fn with_sync_away_from_home_off_nothing_goes_through_the_relay() {
    let ntfy = FakeNtfy::start().await;
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, &ntfy.url);
    round(&a, &la, &b, &lb).await;
    let sync = relay_sync(&a);
    sync.set_rules(Arc::new(|| false), Arc::new(|| true));
    tick(&a, 5);
    a.store.put(&HIDDEN, "track:2", &hidden("Two")).unwrap();
    let msgs = a.peer.push(&la).await;
    assert!(!msgs.is_empty());
    sync.deliver(&la, msgs).await;
    assert!(ntfy.published().is_empty(), "nothing on the relay");
    assert_eq!(sync.status()[0].words, "Waiting for Device b on your home network");
    assert!(!a.peer.push(&la).await.is_empty(), "kept for later");
    // On again: it goes.
    sync.set_rules(Arc::new(|| true), Arc::new(|| true));
    sync.deliver(&la, a.peer.push(&la).await).await;
    assert!(!ntfy.published().is_empty());
}

#[tokio::test]
async fn on_mobile_data_imported_plays_wait_for_wifi_and_the_rest_goes() {
    let ntfy = FakeNtfy::start().await;
    let a = dev("a");
    let b = dev("b");
    let (la, lb) = pair(&a, &b, &ntfy.url);
    round(&a, &la, &b, &lb).await;
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
    tick(&a, 5);
    crate::sync::imported_plays::publish(&a.store).unwrap();
    a.store.put(&HIDDEN, "track:2", &hidden("Two")).unwrap();
    let sync = relay_sync(&a);
    sync.set_metered(true);
    sync.deliver(&la, a.peer.push(&la).await).await;
    assert!(!ntfy.published().is_empty(), "the small change went");
    assert_eq!(sync.status()[0].words, "Imported plays wait for Wi-Fi");
    let left = a.peer.push(&la).await;
    let ns: Vec<String> = left.iter().flat_map(|m| if let Msg::SyncEntries { entries } = m { entries.keys().cloned().collect() } else { Vec::new() }).collect();
    assert_eq!(ns, [store::IMPORTED_PLAYS.name()], "only the plays wait");
    // On Wi-Fi again they go.
    sync.set_metered(false);
    sync.deliver(&la, left).await;
    assert!(a.peer.push(&la).await.is_empty());
    assert_eq!(sync.status()[0].words, "In sync");
}

// ---------------------------------------------------------------- waking up

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coming_back_to_the_foreground_starts_a_round_at_once() {
    let ntfy = FakeNtfy::start().await;
    let a = real_clock("a", ACCT);
    let b = real_clock("b", ACCT);
    pair(&a, &b, &ntfy.url);
    let slow = Timing { debounce: Duration::from_millis(200), every: Duration::from_secs(3600), ..Timing::default() };
    let start = |d: Dev, fg: Option<tokio::sync::watch::Receiver<bool>>| {
        let wire = Arc::new(Wire::new(d.links.clone(), Arc::new(Ntfy::new().unwrap()) as Arc<dyn RelayFactory>, Arc::new(crate::sync::store::system_ms)));
        let sync = Arc::new(DeviceSync::new(d.peer.clone(), wire, slow));
        if let Some(fg) = fg {
            sync.set_foreground(fg);
        }
        let (stop, rx) = tokio::sync::watch::channel(false);
        tokio::spawn(sync.clone().run(rx));
        (d, sync, stop)
    };
    let (fg_tx, fg_rx) = tokio::sync::watch::channel(true);
    let (a, a_sync, a_stop) = start(a, Some(fg_rx));
    let (b, b_sync, b_stop) = start(b, None);
    until("the first round", || a_sync.status().first().is_some_and(|s| s.words == "In sync")).await;
    // Something new on b that nothing announces (as if it came in while a slept).
    b.db
        .write(|c| {
            c.execute("INSERT INTO sync_entries (ns, key, value, ts, device) VALUES ('hidden', 'track:quiet', '{\"kind\":\"track\",\"title\":\"Q\"}', ?1, 'b')", [crate::sync::store::system_ms() as i64])?;
            Ok(())
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(!a.store.has_entry(&HIDDEN, "track:quiet").unwrap(), "no round by itself for an hour");
    // The phone goes to the background and comes back.
    fg_tx.send(false).unwrap();
    fg_tx.send(true).unwrap();
    until("a round on coming back", || a.store.has_entry(&HIDDEN, "track:quiet").unwrap()).await;
    // Not again straight away for the same reason; a network change still counts.
    assert!(!a_sync.wake(Wake::Foreground), "at most once every few seconds");
    assert!(a_sync.wake(Wake::Network));
    assert!(!a_sync.wake(Wake::Network));
    let _ = (a_stop.send(true), b_stop.send(true));
    let _ = b_sync;
}

/// Counts how often the network path is told the network changed.
#[derive(Default)]
struct CountsChanges(std::sync::atomic::AtomicUsize);

#[async_trait]
impl LanSend for CountsChanges {
    async fn post(&self, _: &str, _: &str, _: &LanBody) -> Result<LanBody, PeerError> {
        Err(PeerError::Unreachable)
    }
    fn network_changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn a_network_change_right_after_coming_back_still_tries_the_local_network_again() {
    let a = dev("a");
    let sync = relay_sync(&a);
    let lan = Arc::new(CountsChanges::default());
    sync.set_lan(lan.clone());
    assert!(sync.wake(Wake::Foreground));
    assert!(sync.wake(Wake::Network), "its own limit");
    assert_eq!(lan.0.load(Ordering::SeqCst), 1);
    // Even inside the limit, a new network is worth trying at once.
    assert!(!sync.wake(Wake::Network));
    assert_eq!(lan.0.load(Ordering::SeqCst), 2);
    assert!(!sync.wake(Wake::Foreground));
    assert_eq!(lan.0.load(Ordering::SeqCst), 2, "coming back alone doesn't");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stop_that_comes_after_a_copy_ended_leaves_nothing_stopped() {
    let (pc, phone) = paired().await;
    computer_has_things(&pc.dev);
    let r = phone.sync.copy_from_computer(Duration::from_secs(5)).await.unwrap();
    assert!(!r.cancelled);
    // The stop arrives late (the copy had just ended): nothing is stopped by it.
    phone.sync.cancel_copy();
    tick(&pc.dev, 5);
    pc.dev.store.put(&HIDDEN, "track:late", &hidden("after")).unwrap();
    phone.sync.sync_now().await;
    assert!(phone.dev.store.has_entry(&HIDDEN, "track:late").unwrap(), "rounds still run on the local network");
    let r = phone.sync.copy_from_computer(Duration::from_secs(5)).await.unwrap();
    assert!(!r.cancelled && r.groups.len() == 7, "the next copy runs to its end: {r:?}");
    // A stop while one copy runs touches only that copy: rounds after it go on.
    let s = phone.sync.clone();
    phone.sync.set_on_copy(Arc::new(move |p: &CopyProgress| {
        if p.step == 1 {
            s.cancel_copy();
        }
    }));
    let r = phone.sync.copy_from_computer(Duration::from_secs(5)).await.unwrap();
    assert!(r.cancelled);
    tick(&pc.dev, 5);
    pc.dev.store.put(&HIDDEN, "track:after-stop", &hidden("after")).unwrap();
    phone.sync.sync_now().await;
    assert!(phone.dev.store.has_entry(&HIDDEN, "track:after-stop").unwrap());
}

#[tokio::test]
async fn waking_does_nothing_while_keep_my_devices_in_sync_is_off() {
    let a = dev("a");
    let sync = relay_sync(&a);
    sync.set_switch(Arc::new(|| false));
    assert!(!sync.wake(Wake::Foreground));
    sync.set_switch(Arc::new(|| true));
    assert!(sync.wake(Wake::Network));
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
