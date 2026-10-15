//! Tauri commands for the sync file, pins, library folders and copying
//! keys (docs/IPC.md "Sync, folders and keys").

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

use super::detect::{self, CloudFolder};
use super::engine::{AccountTag, SyncEngine, SyncEvents, SyncFileStatus};
use super::folders::{self, FolderView, Folders};
use super::keycopy::{self, KeyEvents, KeyKind, KeyReceiver, KeyVault, ReceiveState, Secret, SendOutcome};
use super::pins::{self, Pin};
use super::store::SyncStore;
use crate::artistinfo::{ArtistInfoService, InfoSource};
use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;
use crate::remote::RemoteService;
use crate::settings::SettingsState;

pub const EV_STATUS: &str = "sync-file://status";
pub const EV_CHANGED: &str = "syncstore://changed";
pub const EV_KEYS: &str = "keys://receive";
pub const EV_SETTINGS: &str = crate::audio::commands::EV_SETTINGS;

/// Tell the window the settings changed (with all of them, like the output
/// device follower does).
async fn emit_settings(app: &AppHandle, settings: &SettingsState) {
    if let Ok(s) = settings.get().await {
        let _ = app.emit(EV_SETTINGS, &s);
    }
}

/// Managed state.
#[derive(Clone)]
pub struct SyncService {
    pub store: SyncStore,
    pub engine: SyncEngine,
    pub keys: KeyReceiver,
    pub vault: Arc<dyn KeyVault>,
}

/// The account tag, from the Listen on service (same hash other devices
/// announce with).
pub struct RemoteTag(pub RemoteService);

#[async_trait]
impl AccountTag for RemoteTag {
    async fn tag(&self) -> Option<String> {
        self.0.account_tag().await
    }
}

impl SyncEvents for AppHandle {
    fn status(&self, status: &SyncFileStatus) {
        let _ = self.emit(EV_STATUS, status);
    }
}

impl KeyEvents for AppHandle {
    fn receive_state(&self, s: &ReceiveState) {
        let _ = self.emit(EV_KEYS, s);
    }
}

/// Keys in the keychain, read directly and stored through the features that
/// use them (so their caches and checks follow).
pub struct AppVault {
    pub app: AppHandle,
    pub keychain: Arc<dyn TokenStore>,
}

fn info_source(k: KeyKind) -> Option<InfoSource> {
    match k {
        KeyKind::Listenbrainz => Some(InfoSource::Listenbrainz),
        KeyKind::Lastfm => Some(InfoSource::Lastfm),
        KeyKind::Fanart => Some(InfoSource::Fanart),
        KeyKind::Audiodb => Some(InfoSource::Audiodb),
        _ => None,
    }
}

impl AppVault {
    fn account(k: KeyKind) -> Option<&'static str> {
        match k {
            KeyKind::Imgur => Some(crate::platform::discord::art::IMGUR_ACCOUNT),
            KeyKind::Catbox => None,
            other => info_source(other).map(|s| s.key_account()),
        }
    }

    async fn catbox(&self) -> bool {
        match self.app.try_state::<SettingsState>() {
            Some(s) => s.get().await.map(|s| s.discord_upload_covers).unwrap_or(false),
            None => false,
        }
    }
}

#[async_trait]
impl KeyVault for AppVault {
    async fn available(&self) -> Vec<KeyKind> {
        let mut out = Vec::new();
        for k in KeyKind::ALL {
            if self.read(k).await.is_some() {
                out.push(k);
            }
        }
        out
    }

    async fn read(&self, kind: KeyKind) -> Option<Secret> {
        if kind == KeyKind::Catbox {
            return self.catbox().await.then(|| Secret::new("on"));
        }
        let account = Self::account(kind)?;
        let keychain = self.keychain.clone();
        tokio::task::spawn_blocking(move || keychain.get(account).ok().flatten())
            .await
            .ok()
            .flatten()
            .map(|t| Secret::new(t.expose()))
    }

    async fn store(&self, kind: KeyKind, value: Secret) -> AppResult<()> {
        match kind {
            KeyKind::Catbox => {
                let on = value.expose() == "on";
                let settings = self.app.state::<SettingsState>();
                settings
                    .modify_and_publish(move |s| {
                        let changed = s.discord_upload_covers != on;
                        s.discord_upload_covers = on;
                        Ok(changed)
                    })
                    .await?;
                emit_settings(&self.app, &settings).await;
                Ok(())
            }
            KeyKind::Imgur => {
                let resolver = self.app.state::<Arc<crate::platform::discord::art::ArtResolver>>();
                resolver.set_key(Some(value.expose().to_string())).await?;
                if let Some(d) = self.app.try_state::<crate::platform::discord::DiscordHandle>() {
                    d.key_changed();
                }
                Ok(())
            }
            other => {
                let source = info_source(other).ok_or_else(|| AppError::Invalid("unknown key".into()))?;
                let info = self.app.state::<ArtistInfoService>();
                info.set_key(source, Some(value.expose().to_string())).await?;
                Ok(())
            }
        }
    }
}

/// What the app needs to apply changes another device made.
pub struct Appliers {
    pub app: AppHandle,
    pub settings: SettingsState,
    pub player: crate::audio::player::PlayerHandle,
    pub downloads: crate::downloads::Downloads,
    pub mixes: crate::mixes::MixService,
    pub db: crate::db::DbHandle,
}

/// Tell the window about store changes, and apply what the sync file
/// brought: synced settings like a change on the Settings page, lyrics into
/// the lyrics table.
pub fn spawn_listener(store: SyncStore, a: Appliers) {
    let mut rx = store.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let c = match rx.recv().await {
                Ok(c) => c,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => super::store::Changed {
                    namespaces: super::store::NAMESPACES.iter().map(|s| s.to_string()).collect(),
                    remote: true,
                },
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let _ = a.app.emit(EV_CHANGED, &c);
            if !c.remote {
                continue;
            }
            let has = |ns: &str| c.namespaces.iter().any(|n| n == ns);
            if has(super::store::SETTINGS.name())
                && let Ok(cur) = a.settings.get().await
            {
                match store.run(move |st| super::settings::patch_from_store(st, &cur)).await {
                    Ok(Some(patch)) => {
                        if let Err(e) = crate::settings::apply_patch(&a.settings, &a.player, &a.downloads, &a.mixes, patch).await {
                            tracing::warn!("synced settings not applied: {e}");
                        }
                        emit_settings(&a.app, &a.settings).await;
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("synced settings not read: {e}"),
                }
            }
            if has(super::store::FOLLOWS.name()) {
                use crate::social::follows::FollowStore;
                let f = super::follows::SyncFollows { store: store.clone() };
                if let Ok(Ok(all)) = tokio::task::spawn_blocking(move || f.list()).await {
                    let _ = a.app.emit(crate::social::EVENT_FOLLOWS, &all);
                }
            }
            if has(super::store::LYRICS.name()) {
                let db = a.db.clone();
                if let Err(e) = store.run(move |st| super::lyrics::apply(st, &db)).await {
                    tracing::warn!("synced lyrics not applied: {e}");
                }
            }
            if has(super::store::LYRICS_CHOICES.name()) {
                let db = a.db.clone();
                match store.run(move |st| super::lyrics::apply_choices(st, &db)).await {
                    Ok(n) if n > 0 => {
                        let _ = a.app.emit(crate::lyrics::commands::EV_CHOICES, ());
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("synced lyrics choices not applied: {e}"),
                }
            }
        }
    });
}

// ------------------------------------------------------------ sync file

#[tauri::command]
pub async fn sync_file_status(sync: State<'_, SyncService>) -> AppResult<SyncFileStatus> {
    Ok(sync.engine.status())
}

/// Run a pass now.
#[tauri::command]
pub async fn sync_file_now(sync: State<'_, SyncService>) -> AppResult<SyncFileStatus> {
    sync.engine.sync_now().await;
    Ok(sync.engine.status())
}

/// Cloud folders found on this computer, to suggest.
#[tauri::command]
pub async fn sync_file_folders() -> AppResult<Vec<CloudFolder>> {
    tokio::task::spawn_blocking(detect::detect_here)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
}

async fn use_folder(sync: &SyncService, settings: &SettingsState, app: &AppHandle, folder: PathBuf) -> AppResult<SyncFileStatus> {
    if !folder.is_dir() {
        return Err(AppError::NotFound("That folder isn't there.".into()));
    }
    let text = folder.to_string_lossy().to_string();
    settings
        .modify_and_publish(move |s| {
            s.sync_folder = text;
            s.sync_enabled = true;
            Ok(true)
        })
        .await?;
    emit_settings(app, settings).await;
    sync.engine.sync_now().await;
    Ok(sync.engine.status())
}

/// Sync with this folder (a suggested one) and turn sync on.
#[tauri::command]
pub async fn sync_file_use_folder(
    app: AppHandle,
    sync: State<'_, SyncService>,
    settings: State<'_, SettingsState>,
    path: String,
) -> AppResult<SyncFileStatus> {
    use_folder(&sync, &settings, &app, PathBuf::from(path)).await
}

/// Pick a folder in a dialog and sync with it. None when cancelled.
#[tauri::command]
pub async fn sync_file_choose_folder(
    app: AppHandle,
    sync: State<'_, SyncService>,
    settings: State<'_, SettingsState>,
) -> AppResult<Option<SyncFileStatus>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose a folder to sync with")
        .pick_folder(move |p| {
            let _ = tx.send(p);
        });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let dir = picked.into_path().map_err(|e| AppError::Io(format!("folder: {e}")))?;
    use_folder(&sync, &settings, &app, dir).await.map(Some)
}

/// Start a new sync file when the old one can't be read (it's kept aside).
#[tauri::command]
pub async fn sync_file_replace(sync: State<'_, SyncService>) -> AppResult<SyncFileStatus> {
    sync.engine.replace_file().await?;
    Ok(sync.engine.status())
}

// ------------------------------------------------------------ pins

#[tauri::command]
pub async fn pins_get(sync: State<'_, SyncService>) -> AppResult<Vec<Pin>> {
    sync.store.run(pins::list).await
}

#[tauri::command]
pub async fn pins_toggle(sync: State<'_, SyncService>, pin: Pin) -> AppResult<Vec<Pin>> {
    sync.store.run(move |s| pins::toggle(s, pin)).await
}

/// Pins an older build kept in the window's storage (taken once).
#[tauri::command]
pub async fn pins_import(sync: State<'_, SyncService>, pins: Vec<Pin>) -> AppResult<Vec<Pin>> {
    sync.store.run(move |s| pins::import(s, pins)).await
}

// ------------------------------------------------------------ folders

#[tauri::command]
pub async fn folders_get(sync: State<'_, SyncService>) -> AppResult<Folders> {
    sync.store.run(folders::tree).await
}

#[tauri::command]
pub async fn folder_create(sync: State<'_, SyncService>, name: Option<String>, parent: Option<String>) -> AppResult<FolderView> {
    sync.store.run(move |s| folders::create(s, name.as_deref(), parent.as_deref())).await
}

#[tauri::command]
pub async fn folder_rename(sync: State<'_, SyncService>, id: String, name: String) -> AppResult<()> {
    sync.store.run(move |s| folders::rename(s, &id, &name)).await
}

#[tauri::command]
pub async fn folder_delete(sync: State<'_, SyncService>, id: String) -> AppResult<()> {
    sync.store.run(move |s| folders::delete(s, &id)).await
}

#[tauri::command]
pub async fn folder_move(sync: State<'_, SyncService>, id: String, parent: Option<String>) -> AppResult<()> {
    sync.store.run(move |s| folders::move_folder(s, &id, parent.as_deref())).await
}

#[tauri::command]
pub async fn playlist_folder_set(sync: State<'_, SyncService>, playlist_id: String, folder_id: Option<String>) -> AppResult<()> {
    sync.store.run(move |s| folders::place(s, &playlist_id, folder_id.as_deref())).await
}

// ------------------------------------------------------------ keys

/// The keys this device has (what "Send keys to a device" offers).
#[tauri::command]
pub async fn keys_available(sync: State<'_, SyncService>) -> AppResult<Vec<KeyKind>> {
    Ok(sync.vault.available().await)
}

/// Show a code to copy keys to this device.
#[tauri::command]
pub async fn keys_receive_start(sync: State<'_, SyncService>, remote: State<'_, RemoteService>) -> AppResult<ReceiveState> {
    if remote.account_tag().await.is_none() {
        return Err(AppError::Auth("Sign in to Plex first.".into()));
    }
    if !*remote.switches().announce.borrow() {
        return Err(AppError::Invalid(
            "Turn on Show this computer to my other devices first, so the other computer can find this one.".into(),
        ));
    }
    Ok(sync.keys.start())
}

#[tauri::command]
pub async fn keys_receive_state(sync: State<'_, SyncService>) -> AppResult<ReceiveState> {
    Ok(sync.keys.state())
}

#[tauri::command]
pub async fn keys_receive_cancel(sync: State<'_, SyncService>) -> AppResult<()> {
    sync.keys.cancel();
    Ok(())
}

/// Send the ticked keys to the device showing `code`.
#[tauri::command]
pub async fn keys_send(
    sync: State<'_, SyncService>,
    remote: State<'_, RemoteService>,
    code: String,
    kinds: Vec<KeyKind>,
) -> AppResult<SendOutcome> {
    let client = remote.peer_client().await.ok_or_else(|| AppError::Auth("Sign in to Plex first.".into()))?;
    let devices = remote.platter_devices().await;
    let name = remote.switches().computer_name();
    keycopy::send(&client, &name, devices, &code, &kinds, sync.vault.as_ref()).await
}
