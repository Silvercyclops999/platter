//! Library changes as they happen, from the server's websocket (`/socket`,
//! signed in with the same header as every request). Changed items of the
//! main library are written into the cache item by item (`db::live`);
//! anything else, and a reconnect, runs the usual incremental sync after a
//! quiet moment. While the socket is down, a sync runs every 15 minutes.

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use super::server::Core;
use crate::db::DbHandle;
use crate::db::live::{Batch, ItemKindKey, LiveChange, LiveOutcome};
use crate::error::{AppError, AppResult};

const QUIET: Duration = Duration::from_secs(4);
const FALLBACK_EVERY: Duration = Duration::from_secs(15 * 60);

/// What one socket message means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEvent {
    /// Items added or changed (read again) and removed (raw ids).
    Changed { upsert: Vec<String>, removed: Vec<String> },
    /// The server wants a keep-alive this often.
    KeepAlive(Duration),
}

fn ids(v: &serde_json::Value, key: &str) -> Vec<String> {
    v[key].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Read a socket message. Others (scheduled tasks, sessions) are None.
pub fn event_of(text: &str) -> Option<LiveEvent> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let data = &v["Data"];
    match v["MessageType"].as_str()? {
        "LibraryChanged" => {
            let mut upsert = ids(data, "ItemsAdded");
            upsert.extend(ids(data, "ItemsUpdated"));
            let removed = ids(data, "ItemsRemoved");
            (!upsert.is_empty() || !removed.is_empty()).then_some(LiveEvent::Changed { upsert, removed })
        }
        "UserDataChanged" => {
            let upsert: Vec<String> = data["UserDataList"]
                .as_array()?
                .iter()
                .filter_map(|u| u["ItemId"].as_str().map(str::to_string))
                .collect();
            (!upsert.is_empty()).then_some(LiveEvent::Changed { upsert, removed: Vec::new() })
        }
        "ForceKeepAlive" => {
            let secs = data.as_u64().unwrap_or(60).clamp(10, 600);
            Some(LiveEvent::KeepAlive(Duration::from_secs(secs / 2)))
        }
        _ => None,
    }
}

/// The batch for the cache: every changed id read again, removed ids gone
/// whatever kind they were.
pub fn batch_of(tag: &str, upsert: &[String], removed: &[String]) -> Batch {
    let mut b = Batch::default();
    for id in upsert {
        b.add(LiveChange::Upsert(ItemKindKey::Track, crate::sid::qualify_any(tag, id)));
    }
    for id in removed {
        for k in [ItemKindKey::Track, ItemKindKey::Album, ItemKindKey::Artist] {
            b.add(LiveChange::Remove(k, crate::sid::qualify_any(tag, id)));
        }
    }
    b
}

fn socket_url(core: &Core) -> AppResult<url::Url> {
    let c = core.client()?;
    let mut u = c.url("/socket", &[("deviceId", c.identity().device_id.clone())])?;
    let scheme = if u.scheme() == "https" { "wss" } else { "ws" };
    u.set_scheme(scheme).map_err(|_| AppError::Internal("websocket scheme".into()))?;
    Ok(u)
}

/// Follow one server until `stop` turns true.
pub fn spawn(core: Arc<Core>, db: DbHandle, stop: watch::Receiver<bool>) {
    tauri::async_runtime::spawn(follow(core, db, stop));
}

/// How long connecting the live socket may take.
const SOCKET_CONNECT: Duration = Duration::from_secs(20);

async fn follow(core: Arc<Core>, db: DbHandle, mut stop: watch::Receiver<bool>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Option<LiveEvent>>();
    let tag = core.tag().to_string();
    // Writes changes after a quiet moment; None asks for a sync.
    let db2 = db.clone();
    let core2 = core.clone();
    tauri::async_runtime::spawn(async move {
        let mut upsert = Vec::new();
        let mut removed = Vec::new();
        let mut sync = false;
        let mut last_sync = tokio::time::Instant::now();
        loop {
            let first = tokio::time::timeout(FALLBACK_EVERY, rx.recv()).await;
            match first {
                Ok(None) => return,
                Ok(Some(ev)) => take(ev, &mut upsert, &mut removed, &mut sync),
                Err(_) => sync = true, // quiet for long: catch up
            }
            let until = tokio::time::Instant::now() + QUIET;
            while let Ok(Some(ev)) = tokio::time::timeout_at(until, rx.recv()).await {
                take(ev, &mut upsert, &mut removed, &mut sync);
            }
            while db2.sync_running() {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let main = match db2.source() {
                Some(s) => s.server_tag().await.as_deref() == Some(tag.as_str()),
                None => false,
            };
            if !sync && main && let Some(source) = db2.source() {
                let batch = batch_of(&tag, &upsert, &removed);
                match crate::db::live::apply(&db2, source.as_ref(), batch).await {
                    Ok(LiveOutcome::NeedsSync) | Err(_) => sync = true,
                    Ok(_) => {}
                }
            } else if !upsert.is_empty() || !removed.is_empty() {
                sync = true;
            }
            if sync && core2.signed_in() && last_sync.elapsed() >= QUIET {
                match db2.start_sync(false) {
                    Ok(_) => last_sync = tokio::time::Instant::now(),
                    Err(e) => tracing::debug!("jellyfin sync not started: {e}"),
                }
            }
            upsert.clear();
            removed.clear();
            sync = false;
        }
    });
    let mut attempt: u32 = 0;
    loop {
        if *stop.borrow() {
            return;
        }
        if !core.signed_in() {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(30)) => continue,
                _ = stop.changed() => continue,
            }
        }
        let opened = tokio::time::Instant::now();
        if let Ok(true) = run_once(&core, &tx, &mut stop).await {
            return;
        }
        if opened.elapsed() > Duration::from_secs(60) {
            attempt = 0;
        }
        let delay = Duration::from_secs((5u64 << attempt.min(4)).min(60));
        attempt = attempt.saturating_add(1);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = stop.changed() => {}
        }
    }
}

fn take(ev: Option<LiveEvent>, upsert: &mut Vec<String>, removed: &mut Vec<String>, sync: &mut bool) {
    match ev {
        Some(LiveEvent::Changed { upsert: u, removed: r }) => {
            upsert.extend(u);
            removed.extend(r);
        }
        Some(LiveEvent::KeepAlive(_)) => {}
        None => *sync = true,
    }
}

async fn run_once(
    core: &Core,
    tx: &tokio::sync::mpsc::UnboundedSender<Option<LiveEvent>>,
    stop: &mut watch::Receiver<bool>,
) -> AppResult<bool> {
    let url = socket_url(core)?;
    let mut req = url.as_str().into_client_request().map_err(|e| AppError::Internal(format!("websocket request: {e}")))?;
    req.headers_mut().insert("Authorization", core.client()?.header()?);
    // A port that never answers mustn't hold up the fallback sync.
    let (mut ws, _) = tokio::time::timeout(SOCKET_CONNECT, tokio_tungstenite::connect_async(req))
        .await
        .map_err(|_| AppError::Network("jellyfin socket took too long to connect".into()))?
        .map_err(|_| AppError::Network("jellyfin socket failed to connect".into()))?;
    // Connected (again): catch up on anything missed.
    let _ = tx.send(None);
    let mut keep_alive = tokio::time::interval(Duration::from_secs(30));
    keep_alive.tick().await;
    loop {
        tokio::select! {
            msg = ws.next() => match msg {
                Some(Ok(Message::Text(t))) => match event_of(t.as_str()) {
                    Some(LiveEvent::KeepAlive(every)) => {
                        keep_alive = tokio::time::interval(every);
                        keep_alive.tick().await;
                    }
                    Some(ev) => {
                        if tx.send(Some(ev)).is_err() {
                            return Ok(true);
                        }
                    }
                    None => {}
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Ok(false),
                Some(Ok(_)) => {}
            },
            _ = keep_alive.tick() => {
                if ws.send(Message::Text(r#"{"MessageType":"KeepAlive"}"#.into())).await.is_err() {
                    return Ok(false);
                }
            }
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    let _ = ws.close(None).await;
                    return Ok(true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_messages() {
        let lib = r#"{"MessageType":"LibraryChanged","Data":{"ItemsAdded":["a"],"ItemsUpdated":["b"],"ItemsRemoved":["c"],"FoldersAddedTo":[]}}"#;
        assert_eq!(
            event_of(lib),
            Some(LiveEvent::Changed { upsert: vec!["a".into(), "b".into()], removed: vec!["c".into()] })
        );
        let fav = r#"{"MessageType":"UserDataChanged","Data":{"UserId":"u","UserDataList":[{"ItemId":"t1","IsFavorite":true}]}}"#;
        assert_eq!(event_of(fav), Some(LiveEvent::Changed { upsert: vec!["t1".into()], removed: vec![] }));
        assert_eq!(event_of(r#"{"MessageType":"ForceKeepAlive","Data":60}"#), Some(LiveEvent::KeepAlive(Duration::from_secs(30))));
        assert_eq!(event_of(r#"{"MessageType":"LibraryChanged","Data":{}}"#), None);
        assert_eq!(event_of(r#"{"MessageType":"ScheduledTasksInfo","Data":[]}"#), None);
        assert_eq!(event_of("not json"), None);
    }

    #[test]
    fn batches_carry_the_tag() {
        let b = batch_of("0badf00d", &["a".into()], &["c".into()]);
        assert_eq!(b.upsert_ids(), vec!["0badf00d:a".to_string()]);
        assert_eq!(b.len(), 4, "one read again, one gone under every kind");
    }
}
