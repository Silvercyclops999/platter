//! What Platter Bridge needs from the Lidarr work (`crate::homeside`).
//!
//! - **Bridge mode** (on the Bridge only): Lidarr's settings, profiles,
//!   roots and exclusions are never changed. A write of that kind is not
//!   sent; it waits in the shared store (`ask:<id>`) for a person to look
//!   at it on a desktop ([`Lidarr::queued_writes`]).
//! - **Modes from a desktop**: which artists are protected comes from the
//!   library's tags, which only a desktop reads. Desktops publish the modes
//!   they work out (`mode:<server>:<key>`, Lidarr artists only), and the
//!   Bridge works from those; with none yet it doesn't catch up.
//! - **The home Platter route**: a desktop away from home can reach Lidarr
//!   through its paired Bridge, after the home and away addresses.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use super::Lidarr;
use super::api::{LidarrRoute, Route};
use super::modes::{ArtistMode, Mode, ModeSetting};
use super::shared::Shared;
use super::write::{LidarrWrite, LidarrWriter, RelayedWrite, WriteCall};
use crate::artistinfo::http::HttpResponse;
use crate::error::{AppError, AppResult};

/// Gives the Bridge route when a Bridge is paired and answering.
pub type RouteFn = Arc<dyn Fn() -> Option<Arc<dyn LidarrRoute>> + Send + Sync>;

#[derive(Default)]
pub struct Hooks {
    bridge: AtomicBool,
    route: RwLock<Option<RouteFn>>,
}

/// A Lidarr settings change the Bridge didn't make, waiting for a person
/// (shared, `ask:<id>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedWrite {
    pub id: String,
    /// What it would change, in words.
    pub what: String,
    pub method: String,
    pub path: String,
    pub body: String,
    /// Unix seconds.
    pub at: i64,
    /// Which device queued it.
    pub device: String,
}

/// What a change to this path is, in words.
pub fn describe(path: &str) -> String {
    let p = path.trim_start_matches('/');
    if p.starts_with("config/naming") {
        "Change Lidarr's file naming settings".into()
    } else if p.starts_with("config/metadataprovider") {
        "Change Lidarr's metadata settings".into()
    } else if p.starts_with("config/mediamanagement") {
        "Change Lidarr's media management settings".into()
    } else if p.starts_with("importlistexclusion") {
        "Change Lidarr's import list exclusions".into()
    } else if p.starts_with("rootfolder") {
        "Change Lidarr's root folders".into()
    } else if p.starts_with("qualityprofile") {
        "Change Lidarr's quality profiles".into()
    } else {
        format!("Change Lidarr ({path})")
    }
}

/// The Bridge's writer: settings writes are queued, never sent.
struct Guarded {
    inner: Arc<dyn LidarrRoute>,
    shared: Shared,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

#[async_trait]
impl LidarrWrite for Guarded {
    async fn write(&self, call: &WriteCall) -> AppResult<HttpResponse> {
        if call.is_settings_write() {
            let id = crate::remote::peer::auth::hex(&crate::social::listen::seal::random16()[..8]);
            let q = QueuedWrite {
                id: id.clone(),
                what: describe(&call.path()),
                method: call.method().as_str().to_string(),
                path: call.path(),
                body: call.body(),
                at: (self.clock)(),
                device: self.shared.label(),
            };
            self.shared.put(&format!("ask:{id}"), &q)?;
            tracing::info!(what = q.what, "Bridge: a Lidarr settings change is waiting for you in Platter");
            return Err(AppError::Invalid("The Bridge never changes Lidarr's settings on its own. It's waiting for you in Platter (Settings > Bridge).".into()));
        }
        self.inner.write(call).await
    }

    fn route(&self) -> Route {
        LidarrWrite::route(self.inner.as_ref())
    }
}

/// One artist's mode as desktops share it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedMode {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub artist_id: Option<String>,
    #[serde(default)]
    pub mbid: Option<String>,
    pub lidarr_id: i64,
    #[serde(default)]
    pub lidarr_path: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    pub mode: Mode,
    pub setting: ModeSetting,
    pub auto: Mode,
    #[serde(default)]
    pub release_groups: Vec<String>,
}

impl SharedMode {
    fn of(m: &ArtistMode) -> Option<Self> {
        Some(Self {
            key: m.key.clone(),
            name: m.name.clone(),
            artist_id: m.artist_id.clone(),
            mbid: m.mbid.clone(),
            lidarr_id: m.lidarr_id?,
            lidarr_path: m.lidarr_path.clone(),
            folder: m.folder.clone(),
            mode: m.mode,
            setting: m.setting,
            auto: m.auto,
            release_groups: m.release_groups.clone(),
        })
    }

    fn into_mode(self) -> ArtistMode {
        let label = match self.mode {
            Mode::Managed => "Managed",
            Mode::Protected => "Protected",
        };
        ArtistMode {
            key: self.key,
            name: self.name,
            artist_id: self.artist_id,
            mbid: self.mbid,
            lidarr_id: Some(self.lidarr_id),
            lidarr_path: self.lidarr_path,
            folder: self.folder,
            mode: self.mode,
            setting: self.setting,
            auto: self.auto,
            line: format!("{label}: as a desktop worked it out"),
            reasons: Vec::new(),
            files: 0,
            curated_files: 0,
            excluded: false,
            release_groups: self.release_groups,
        }
    }
}

fn mode_prefix(server: &str) -> String {
    format!("mode:{server}:")
}

impl Lidarr {
    /// On the Bridge: settings writes are queued, modes come from desktops.
    pub fn set_bridge_mode(&self, on: bool) {
        self.home.bridge.store(on, Ordering::SeqCst);
    }

    pub fn bridge_mode(&self) -> bool {
        self.home.bridge.load(Ordering::SeqCst)
    }

    /// On a desktop: how to reach Lidarr through a paired Bridge.
    pub fn set_bridge_route(&self, f: Option<RouteFn>) {
        *self.home.route.write() = f;
    }

    pub(super) fn bridge_route(&self) -> Option<Arc<dyn LidarrRoute>> {
        if self.bridge_mode() {
            return None;
        }
        self.home.route.read().as_ref().and_then(|f| f())
    }

    /// The writer for a connection (guarded on the Bridge).
    pub(super) fn writer_for(&self, route: Arc<dyn LidarrRoute>) -> LidarrWriter {
        if self.bridge_mode() {
            LidarrWriter::new(Arc::new(Guarded { inner: route, shared: self.inner.shared.clone(), clock: self.inner.clock.clone() }))
        } else {
            LidarrWriter::new(route as Arc<dyn LidarrWrite>)
        }
    }

    /// Settings changes a Bridge queued, oldest first.
    pub fn queued_writes(&self) -> Vec<QueuedWrite> {
        let mut l: Vec<QueuedWrite> = self.inner.shared.list::<QueuedWrite>("ask:").into_iter().map(|(_, q)| q).collect();
        l.sort_by_key(|q| q.at);
        l
    }

    pub fn dismiss_queued(&self, id: &str) -> AppResult<()> {
        self.inner.shared.remove(&format!("ask:{id}"))
    }

    /// The user said yes to a queued change: checked again and made from
    /// this device, logged as asked.
    pub async fn apply_queued(&self, id: &str) -> AppResult<()> {
        if self.bridge_mode() {
            return Err(AppError::Invalid("Only a person on a desktop can say yes to this.".into()));
        }
        let q: QueuedWrite =
            self.inner.shared.get(&format!("ask:{id}")).ok_or_else(|| AppError::NotFound("That change isn't waiting any more.".into()))?;
        let call = WriteCall::Relayed(RelayedWrite::check(&q.method, &q.path, &q.body)?);
        let (_, writer) = self.connection().await?;
        writer.send_call(call).await?;
        self.log_change(&q.what, "", "Done, after you said yes to what the Bridge queued", true, None)?;
        self.dismiss_queued(id)
    }

    /// A desktop shares the modes it worked out (only what changed).
    pub(super) fn publish_modes(&self, modes: &[ArtistMode]) {
        if self.bridge_mode() {
            return;
        }
        let server = self.server_key();
        let prefix = mode_prefix(&server);
        let shared = &self.inner.shared;
        let had: std::collections::HashMap<String, SharedMode> = shared.list::<SharedMode>(&prefix).into_iter().collect();
        let mut now: std::collections::HashSet<String> = std::collections::HashSet::new();
        for m in modes.iter().filter_map(SharedMode::of) {
            now.insert(m.key.clone());
            if had.get(&m.key) != Some(&m) {
                let _ = shared.put(&format!("{prefix}{}", m.key), &m);
            }
        }
        for k in had.keys().filter(|k| !now.contains(*k)) {
            let _ = shared.remove(&format!("{prefix}{k}"));
        }
        if shared.get::<i64>(&format!("modesAt:{server}")).is_none() {
            let _ = shared.put(&format!("modesAt:{server}"), &(self.inner.clock)());
        }
    }

    /// The modes desktops shared, when any have.
    pub(super) fn shared_modes(&self) -> Option<Vec<ArtistMode>> {
        let server = self.server_key();
        self.inner.shared.get::<i64>(&format!("modesAt:{server}"))?;
        Some(self.inner.shared.list::<SharedMode>(&mode_prefix(&server)).into_iter().map(|(_, m)| m.into_mode()).collect())
    }

    /// Whether a desktop has shared which artists are protected yet.
    pub fn has_shared_modes(&self) -> bool {
        self.inner.shared.get::<i64>(&format!("modesAt:{}", self.server_key())).is_some()
    }
}
