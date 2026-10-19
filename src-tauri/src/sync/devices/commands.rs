//! Tauri side of keeping the user's devices in sync (docs/IPC.md "Keep my
//! devices in sync"): setting it up when the app starts, the commands
//! Settings uses, and the events it listens to.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::watch;

use super::feeds::{FeedTiming, Feeds};
use super::lan::PeerLan;
use super::pair::{DevicePairing, PairDeps, PairState, Paired};
use super::protocol::{SyncDeps, SyncPeer};
use super::copy::{CopyProgress, CopyResult};
use super::run::{DeviceStatus, DeviceSync, Timing, Wake};
use crate::downloads::Downloads;
use crate::downloads::store::{ItemKey, ItemKind};
use crate::error::{AppError, AppResult};
use crate::homeside::links::Links;
use crate::homeside::peer::Wire;
use crate::remote::RemoteService;
use crate::remote::peer::pair::PairRoutes;
use crate::remote::peer::server::ExtraRoutes;
use crate::remote::peer::sync::SyncRoutes;
use crate::social::listen::relay::RelayFactory;
use crate::sync::SyncStore;
use crate::sync::download_intent::{self, Suggestion};
use crate::sync::engine::ConfigSwitch;
use crate::sync::store::{Quarantined, system_ms};

pub const EV_STATUS: &str = "device-sync://status";
pub const EV_PAIR: &str = "device-sync://pair";
/// A first copy moved on to its next group.
pub const EV_COPY: &str = "device-sync://copy";

/// Managed state.
#[derive(Clone)]
pub struct DeviceService {
    pub sync: Arc<DeviceSync>,
    pub pairing: Arc<DevicePairing>,
    pub feeds: Arc<Feeds>,
    pub store: SyncStore,
    pub on: Arc<AtomicBool>,
    /// Stops device sync when dropped with the app.
    _stop: Arc<watch::Sender<bool>>,
}

/// What Settings > Keep my devices in sync shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSyncOverview {
    pub on: bool,
    pub devices: Vec<DeviceStatus>,
    /// Changes held back from a device whose clock is far ahead.
    pub quarantined: usize,
    pub pairing: PairState,
}

/// What the app hands device sync when it starts.
pub struct Setup {
    pub app: AppHandle,
    pub store: SyncStore,
    pub db: crate::db::DbHandle,
    pub remote: RemoteService,
    pub groups: ConfigSwitch,
    pub on: Arc<AtomicBool>,
    /// Sync when I'm away from home (through the relay).
    pub away: Arc<AtomicBool>,
    /// Send big things only on Wi-Fi.
    pub wifi_only: Arc<AtomicBool>,
    pub relays: Arc<dyn RelayFactory>,
    /// The user's own relay (Settings > Listen along), or empty.
    pub relay: Arc<parking_lot::RwLock<String>>,
    pub downloads: Downloads,
    pub platform: String,
    pub version: String,
}

/// Build device sync, put its routes on the control port next to key
/// copy, and start it. The sync file's engine keeps its own wake-ups.
pub fn start(s: Setup) -> DeviceService {
    let keychain: Arc<dyn crate::plex::keychain::TokenStore> = Arc::new(crate::plex::keychain::KeyringStore::new());
    let links = Links::devices(s.db.clone(), keychain);
    let account: Arc<dyn crate::sync::engine::AccountTag> = Arc::new(crate::sync::commands::RemoteTag(s.remote.clone()));
    let name: Arc<dyn Fn() -> String + Send + Sync> = {
        let sw = s.remote.switches().clone();
        Arc::new(move || sw.computer_name())
    };
    let groups = {
        let g = s.groups.clone();
        Arc::new(move || g.get().groups)
    };
    let peer = Arc::new(SyncPeer::new(SyncDeps {
        store: s.store.clone(),
        db: s.db.clone(),
        links: links.clone(),
        account: account.clone(),
        groups: groups.clone(),
        name: name.clone(),
        platform: s.platform.clone(),
        version: s.version.clone(),
    }));
    let wire = Arc::new(Wire::new(links.clone(), s.relays.clone(), Arc::new(system_ms)));
    let sync = Arc::new(DeviceSync::new(peer, wire, Timing::default()));
    {
        let on = s.on.clone();
        sync.set_switch(Arc::new(move || on.load(Ordering::Relaxed)));
        let (away, wifi) = (s.away.clone(), s.wifi_only.clone());
        sync.set_rules(Arc::new(move || away.load(Ordering::Relaxed)), Arc::new(move || wifi.load(Ordering::Relaxed)));
    }
    {
        let app = s.app.clone();
        sync.set_on_copy(Arc::new(move |p: &CopyProgress| {
            let _ = app.emit(EV_COPY, p);
        }));
    }
    {
        let app = s.app.clone();
        sync.set_on_status(Arc::new(move |st: &[DeviceStatus]| {
            let _ = app.emit(EV_STATUS, st);
        }));
    }
    let own_id = s.remote.own_id().to_string();
    // Over the local network first: this account's devices found over mDNS.
    if let Some(dir) = s.remote.peers() {
        sync.set_lan(Arc::new(PeerLan::new(Arc::new(s.remote.clone()), dir, account.clone(), own_id.clone())));
    }
    let pairing = DevicePairing::new(PairDeps {
        own_id: own_id.clone(),
        links,
        account,
        name: name.clone(),
        version: s.version.clone(),
        relay: {
            let r = s.relay.clone();
            Arc::new(move || r.read().clone())
        },
        relays: s.relays.clone(),
        clock: Arc::new(system_ms),
    });
    {
        let ds = sync.clone();
        let app = s.app.clone();
        pairing.set_on_paired(Arc::new(move |p: &Paired| {
            ds.kick();
            let _ = app.emit(EV_STATUS, ds.status());
            tracing::info!("device sync: paired with {}", p.name);
        }));
        let app = s.app.clone();
        pairing.set_on_state(Arc::new(move |st: &PairState| {
            let _ = app.emit(EV_PAIR, st);
        }));
    }
    // Pairing and sync answer on the control port beside key copy (same
    // account only, every request signed).
    {
        let keys = s.remote.peer_keys();
        let pair: Arc<dyn ExtraRoutes> = Arc::new(PairRoutes::new(own_id.clone(), keys.clone(), pairing.clone()));
        let routes: Arc<dyn ExtraRoutes> = Arc::new(SyncRoutes::new(own_id, keys, sync.clone()));
        let slot = s.remote.extra_routes();
        let mut list = vec![pair, routes];
        list.extend(slot.read().clone());
        *slot.write() = Some(Arc::new(crate::remote::peer::keys::RouteList(list)));
    }
    let feeds = {
        let d = s.downloads.clone();
        Feeds::new(
            s.store.clone(),
            crate::themes::themes_dir(&s.app).ok(),
            Arc::new(move || d.items()),
            name,
            groups,
            FeedTiming::default(),
        )
    };
    {
        let app = s.app.clone();
        feeds.set_on_applied(Arc::new(move |a| {
            if a.looks > 0 {
                let _ = app.emit(crate::sync::commands::EV_CHANGED, serde_json::json!({ "namespaces": ["looks"], "remote": true }));
            }
        }));
    }
    let (keep, stop) = watch::channel(false);
    // Both started again if they stop on a fault; the devices say so meanwhile.
    {
        let (ds, s1) = (sync.clone(), stop.clone());
        let shown = sync.clone();
        tauri::async_runtime::spawn(super::run::supervise("Device sync", move |down| shown.set_stopped(down), move || ds.clone().run(s1.clone())));
        let (f, s2) = (feeds.clone(), stop);
        let shown = sync.clone();
        tauri::async_runtime::spawn(super::run::supervise("Device sync feeds", move |down| shown.set_stopped(down), move || f.clone().run(s2.clone())));
    }
    DeviceService { sync, pairing, feeds, store: s.store, on: s.on, _stop: Arc::new(keep) }
}

impl DeviceService {
    fn overview(&self) -> DeviceSyncOverview {
        DeviceSyncOverview {
            on: self.on.load(Ordering::Relaxed),
            devices: self.sync.status(),
            quarantined: self.store.quarantined().map(|q| q.len()).unwrap_or(0),
            pairing: self.pairing.state(),
        }
    }
}

#[tauri::command]
pub async fn device_sync_status(d: State<'_, DeviceService>) -> AppResult<DeviceSyncOverview> {
    Ok(d.overview())
}

/// A round with every paired device now.
#[tauri::command]
pub async fn device_sync_now(d: State<'_, DeviceService>) -> AppResult<DeviceSyncOverview> {
    if !d.on.load(Ordering::Relaxed) {
        return Err(AppError::Invalid("Turn on Keep my devices in sync first.".into()));
    }
    d.sync.sync_now().await;
    d.sync.status_changed();
    Ok(d.overview())
}

/// Unpair a device (it's told, and forgets this one too).
#[tauri::command]
pub async fn device_sync_forget(d: State<'_, DeviceService>, id: String) -> AppResult<DeviceSyncOverview> {
    d.sync.forget(&id).await?;
    d.sync.status_changed();
    Ok(d.overview())
}

/// Changes held back from a device whose clock is far ahead.
#[tauri::command]
pub async fn device_sync_quarantined(d: State<'_, DeviceService>) -> AppResult<Vec<Quarantined>> {
    d.store.run(|s| s.quarantined()).await
}

/// "Take them anyway".
#[tauri::command]
pub async fn device_sync_take_quarantined(d: State<'_, DeviceService>) -> AppResult<DeviceSyncOverview> {
    let ns: Vec<String> = d.sync.peer.namespaces();
    d.store.run(move |s| s.take_quarantined(&ns.iter().map(String::as_str).collect::<Vec<_>>())).await?;
    Ok(d.overview())
}

/// Show codes for another device to pair with this one.
#[tauri::command]
pub async fn device_pair_start(d: State<'_, DeviceService>) -> AppResult<PairState> {
    d.pairing.start().await
}

#[tauri::command]
pub async fn device_pair_cancel(d: State<'_, DeviceService>) -> AppResult<()> {
    d.pairing.cancel();
    Ok(())
}

/// The answer to "Pair with <name>?" for a device that joined with the
/// long code.
#[tauri::command]
pub async fn device_pair_confirm(d: State<'_, DeviceService>, yes: bool) -> AppResult<PairState> {
    d.pairing.confirm(yes)
}

#[tauri::command]
pub async fn device_pair_state(d: State<'_, DeviceService>) -> AppResult<PairState> {
    Ok(d.pairing.state())
}

/// Pair with the device showing this code: the 6 digits on the same
/// network, or the long code (or its QR code) anywhere.
#[tauri::command]
pub async fn device_pair_join(d: State<'_, DeviceService>, remote: State<'_, RemoteService>, code: String) -> AppResult<Paired> {
    let lan = match remote.peer_client().await {
        Some(client) => Some((client, remote.platter_devices().await)),
        None => None,
    };
    d.pairing.join(&code, lan).await
}

/// What other devices downloaded that this one hasn't: one tap downloads
/// it here too.
#[tauri::command]
pub async fn device_download_suggestions(d: State<'_, DeviceService>, downloads: State<'_, Downloads>) -> AppResult<Vec<Suggestion>> {
    let have: Vec<ItemKey> = downloads.items().into_iter().map(|r| r.key).collect();
    d.store.run(move |s| download_intent::suggestions(s, &have)).await
}

#[tauri::command]
pub async fn device_download_dismiss(d: State<'_, DeviceService>, kind: String, id: Option<String>) -> AppResult<()> {
    let kind = ItemKind::parse(&kind)?;
    let id = id.unwrap_or_default();
    d.store.run(move |s| download_intent::dismiss(s, kind, &id)).await
}

/// Copy from my computer: everything this device keeps in step, from the
/// paired computer, one group at a time (progress on `device-sync://copy`).
/// Its settings win here and everywhere; the rest merges as usual.
#[tauri::command]
pub async fn pair_copy_settings(d: State<'_, DeviceService>) -> AppResult<CopyResult> {
    d.sync.copy_from_computer(Duration::from_secs(20)).await
}

/// Stop a copy from the computer (what came stays).
#[tauri::command]
pub async fn pair_copy_cancel(d: State<'_, DeviceService>) -> AppResult<()> {
    d.sync.cancel_copy();
    Ok(())
}

/// The window saw the network change (`metered`: on mobile data, when it
/// can tell): a round with every device now, and big things wait for
/// Wi-Fi while on mobile data.
#[tauri::command]
pub async fn device_sync_network(d: State<'_, DeviceService>, metered: Option<bool>) -> AppResult<()> {
    if let Some(m) = metered {
        d.sync.set_metered(m);
    }
    d.sync.wake(Wake::Network);
    Ok(())
}
