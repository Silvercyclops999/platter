//! `backend_caps` (docs/IPC.md "Servers"): what the servers in use can do,
//! so the UI shows only what works. `backends://changed` says to ask again.

use serde::Serialize;
use tauri::State;

use super::{BackendKind, Backends, Caps};
use crate::error::AppResult;

/// One server in use.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCaps {
    /// Its tag (the prefix its ids carry); None while not connected.
    pub tag: Option<String>,
    pub kind: BackendKind,
    /// The name its owner gave it.
    pub name: Option<String>,
    /// The server with the main library.
    pub primary: bool,
    pub caps: Caps,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendCaps {
    /// What any server in use can do.
    pub caps: Caps,
    pub servers: Vec<ServerCaps>,
}

impl BackendCaps {
    pub async fn of(b: &Backends) -> Self {
        let primary = b.primary().ok();
        let mut servers = Vec::new();
        for s in b.all() {
            let is_primary = primary.as_ref().is_some_and(|p| std::ptr::addr_eq(std::sync::Arc::as_ptr(p), std::sync::Arc::as_ptr(&s)));
            servers.push(ServerCaps {
                tag: s.tag().await,
                kind: s.kind(),
                name: s.name().await,
                primary: is_primary,
                caps: s.caps(),
            });
        }
        Self { caps: b.caps_all(), servers }
    }
}

#[tauri::command]
pub async fn backend_caps(backends: State<'_, Backends>) -> AppResult<BackendCaps> {
    Ok(BackendCaps::of(&backends).await)
}
