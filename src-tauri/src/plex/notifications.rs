//! Server push events over `/:/websockets/notifications` (health
//! inputs). The token goes in the `X-Plex-Token` handshake header, never in
//! the URL. The connection is re-opened with exponential backoff
//! (1 s → 60 s cap, plus jitter).

use std::time::Duration;

use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use super::client::PlexClient;
use super::lenient;
use crate::error::{AppError, AppResult};

pub const BACKOFF_MIN: Duration = Duration::from_secs(1);
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Backoff before reconnect attempt `attempt` (0-based), without jitter:
/// 1 s, 2 s, 4 s … capped at 60 s.
pub fn backoff_base(attempt: u32) -> Duration {
    let secs = 1u64.checked_shl(attempt.min(16)).unwrap_or(u64::MAX);
    Duration::from_secs(secs).clamp(BACKOFF_MIN, BACKOFF_MAX)
}

/// Add up to +25% jitter. `r` is a uniform sample in `[0, 1)`; the result
/// never exceeds `BACKOFF_MAX` by more than the jitter of the cap.
pub fn backoff_with_jitter(attempt: u32, r: f64) -> Duration {
    let base = backoff_base(attempt);
    let r = r.clamp(0.0, 1.0);
    base + base.mul_f64(0.25 * r)
}

/// Events the app cares about.
#[derive(Debug, Clone, PartialEq)]
pub enum PlexEvent {
    /// Websocket open.
    Connected,
    /// Websocket closed or failed; reconnecting.
    Disconnected,
    /// Library item added/changed/removed (`timeline` entries).
    LibraryUpdate {
        section_id: Option<String>,
        item_id: Option<String>,
        /// Plex metadata type (8 artist, 9 album, 10 track).
        item_type: Option<i64>,
        /// Plex timeline state (0 created … 5 done, 9 deleted).
        state: Option<i64>,
    },
    /// Background activity progress, e.g. a library scan.
    Activity {
        /// "started" | "updated" | "ended"
        event: String,
        uuid: String,
        /// e.g. "library.update.section", "library.refresh.items"
        kind: String,
        title: Option<String>,
        subtitle: Option<String>,
        /// 0–100
        progress: Option<i64>,
        section_id: Option<String>,
    },
    /// A playback session on the server changed.
    Playing {
        session_key: Option<String>,
        rating_key: Option<String>,
        state: Option<String>,
        view_offset: Option<i64>,
    },
    /// Remote access reachability changed.
    Reachability { reachable: bool },
    /// Server update state (e.g. "installing").
    UpdateState { state: String },
    /// Server going down.
    Shutdown,
}

#[derive(Deserialize)]
struct Outer {
    #[serde(rename = "NotificationContainer")]
    container: NotificationContainer,
}

#[derive(Deserialize, Default)]
struct NotificationContainer {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default, rename = "TimelineEntry")]
    timeline: Vec<TimelineEntry>,
    #[serde(default, rename = "ActivityNotification")]
    activity: Vec<ActivityNotification>,
    #[serde(default, rename = "PlaySessionStateNotification")]
    playing: Vec<PlayingNotification>,
    #[serde(default, rename = "ReachabilityNotification")]
    reachability: Vec<ReachabilityNotification>,
    #[serde(default, rename = "StatusNotification")]
    status: Vec<StatusNotification>,
    #[serde(default, rename = "AutoUpdateNotification")]
    auto_update: Vec<AutoUpdateNotification>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimelineEntry {
    #[serde(
        default,
        rename = "sectionID",
        deserialize_with = "lenient::opt_string"
    )]
    section_id: Option<String>,
    #[serde(default, rename = "itemID", deserialize_with = "lenient::opt_string")]
    item_id: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "lenient::opt_i64")]
    kind: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    state: Option<i64>,
}

#[derive(Deserialize)]
struct ActivityNotification {
    #[serde(default)]
    event: String,
    #[serde(default)]
    uuid: String,
    #[serde(default, rename = "Activity")]
    activity: Option<Activity>,
}

#[derive(Deserialize)]
struct Activity {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    progress: Option<i64>,
    #[serde(default, rename = "Context")]
    context: Option<ActivityContext>,
}

#[derive(Deserialize)]
struct ActivityContext {
    #[serde(
        default,
        rename = "librarySectionID",
        deserialize_with = "lenient::opt_string"
    )]
    library_section_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayingNotification {
    #[serde(default, deserialize_with = "lenient::opt_string")]
    session_key: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    rating_key: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    view_offset: Option<i64>,
}

#[derive(Deserialize)]
struct ReachabilityNotification {
    #[serde(default, deserialize_with = "lenient::bool")]
    reachability: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusNotification {
    #[serde(default)]
    notification_name: Option<String>,
}

#[derive(Deserialize)]
struct AutoUpdateNotification {
    #[serde(default)]
    state: Option<String>,
}

/// Parse one websocket text frame. Unknown types yield no events.
pub fn parse(text: &str) -> Vec<PlexEvent> {
    let Ok(outer) = serde_json::from_str::<Outer>(text) else {
        return Vec::new();
    };
    let c = outer.container;
    let mut out = Vec::new();
    match c.kind.as_str() {
        "timeline" => {
            for t in c.timeline {
                out.push(PlexEvent::LibraryUpdate {
                    section_id: t.section_id,
                    item_id: t.item_id,
                    item_type: t.kind,
                    state: t.state,
                });
            }
        }
        "activity" => {
            for a in c.activity {
                let act = a.activity;
                out.push(PlexEvent::Activity {
                    event: a.event,
                    uuid: a.uuid,
                    kind: act.as_ref().map(|x| x.kind.clone()).unwrap_or_default(),
                    title: act.as_ref().and_then(|x| x.title.clone()),
                    subtitle: act.as_ref().and_then(|x| x.subtitle.clone()),
                    progress: act.as_ref().and_then(|x| x.progress),
                    section_id: act
                        .as_ref()
                        .and_then(|x| x.context.as_ref())
                        .and_then(|c| c.library_section_id.clone()),
                });
            }
        }
        "playing" => {
            for p in c.playing {
                out.push(PlexEvent::Playing {
                    session_key: p.session_key,
                    rating_key: p.rating_key,
                    state: p.state,
                    view_offset: p.view_offset,
                });
            }
        }
        "reachability" => {
            for r in c.reachability {
                out.push(PlexEvent::Reachability {
                    reachable: r.reachability,
                });
            }
        }
        "update.statechange" => {
            for u in c.auto_update {
                if let Some(state) = u.state {
                    out.push(PlexEvent::UpdateState { state });
                }
            }
        }
        "status" => {
            for s in c.status {
                if s.notification_name.as_deref() == Some("SERVER_SHUTDOWN") {
                    out.push(PlexEvent::Shutdown);
                }
            }
        }
        _ => {}
    }
    out
}

/// `ws(s)://…/:/websockets/notifications` for a client's base URL.
pub fn websocket_url(client: &PlexClient) -> AppResult<url::Url> {
    let mut u = client.url("/:/websockets/notifications")?;
    let scheme = if u.scheme() == "https" { "wss" } else { "ws" };
    u.set_scheme(scheme)
        .map_err(|_| AppError::Internal("websocket scheme".into()))?;
    Ok(u)
}

/// Open the websocket once and forward events until it closes or `stop`
/// flips to true. Returns `Ok(true)` if stopped, `Ok(false)` on close.
async fn run_once(
    client: &PlexClient,
    tx: &mpsc::Sender<PlexEvent>,
    stop: &mut watch::Receiver<bool>,
) -> AppResult<bool> {
    let url = websocket_url(client)?;
    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|e| AppError::Internal(format!("websocket request: {e}")))?;
    for (k, v) in client.auth_headers()?.iter() {
        req.headers_mut().insert(k.clone(), v.clone());
    }
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|_| AppError::Network("notification socket failed to connect".into()))?;
    let _ = tx.send(PlexEvent::Connected).await;
    loop {
        tokio::select! {
            msg = ws.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    for ev in parse(t.as_str()) {
                        if tx.send(ev).await.is_err() {
                            return Ok(true);
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => return Ok(false),
                Some(Ok(_)) => {}
                Some(Err(_)) => return Ok(false),
            },
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    let _ = ws.close(None).await;
                    return Ok(true);
                }
            }
        }
    }
}

/// Keep the notification socket open, reconnecting with backoff. Ends when
/// `stop` becomes true (or its sender drops) or the receiver is dropped.
pub async fn run(client: PlexClient, tx: mpsc::Sender<PlexEvent>, mut stop: watch::Receiver<bool>) {
    let mut attempt: u32 = 0;
    loop {
        if *stop.borrow() {
            return;
        }
        let opened_at = tokio::time::Instant::now();
        match run_once(&client, &tx, &mut stop).await {
            Ok(true) => return,
            Ok(false) | Err(_) => {
                if tx.send(PlexEvent::Disconnected).await.is_err() {
                    return;
                }
            }
        }
        // A connection that stayed up a while resets the backoff.
        if opened_at.elapsed() > Duration::from_secs(60) {
            attempt = 0;
        }
        let delay = backoff_with_jitter(attempt, rand::random::<f64>());
        attempt = attempt.saturating_add(1);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let secs: Vec<u64> = (0..9).map(|a| backoff_base(a).as_secs()).collect();
        assert_eq!(secs, vec![1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(backoff_base(1000), BACKOFF_MAX);
    }

    #[test]
    fn jitter_stays_in_range() {
        for a in 0..10 {
            let base = backoff_base(a);
            assert_eq!(backoff_with_jitter(a, 0.0), base);
            let hi = backoff_with_jitter(a, 0.999);
            assert!(hi >= base && hi <= base + base / 4, "{hi:?}");
        }
        assert!(backoff_with_jitter(100, 1.0) <= Duration::from_secs(75));
    }

    #[test]
    fn parses_timeline() {
        let ev = parse(
            r#"{"NotificationContainer":{"type":"timeline","size":1,"TimelineEntry":[
            {"identifier":"com.plexapp.plugins.library","sectionID":"3","itemID":123,"type":10,"title":"x","state":5,"updatedAt":1}]}}"#,
        );
        assert_eq!(
            ev,
            vec![PlexEvent::LibraryUpdate {
                section_id: Some("3".into()),
                item_id: Some("123".into()),
                item_type: Some(10),
                state: Some(5),
            }]
        );
    }

    #[test]
    fn parses_scan_activity() {
        let ev = parse(
            r#"{"NotificationContainer":{"type":"activity","size":1,"ActivityNotification":[
            {"event":"updated","uuid":"u1","Activity":{"uuid":"u1","type":"library.update.section","cancellable":false,
             "title":"Scanning Music","subtitle":"Album X","progress":45,"Context":{"librarySectionID":"3"}}}]}}"#,
        );
        match &ev[0] {
            PlexEvent::Activity {
                event,
                kind,
                progress,
                section_id,
                ..
            } => {
                assert_eq!(event, "updated");
                assert_eq!(kind, "library.update.section");
                assert_eq!(*progress, Some(45));
                assert_eq!(section_id.as_deref(), Some("3"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parses_playing_reachability_update_shutdown() {
        let p = parse(
            r#"{"NotificationContainer":{"type":"playing","PlaySessionStateNotification":[
            {"sessionKey":"7","ratingKey":"500","state":"paused","viewOffset":1234}]}}"#,
        );
        assert!(
            matches!(&p[0], PlexEvent::Playing { state: Some(s), view_offset: Some(1234), .. } if s == "paused")
        );
        let r = parse(
            r#"{"NotificationContainer":{"type":"reachability","ReachabilityNotification":[{"reachability":false}]}}"#,
        );
        assert_eq!(r, vec![PlexEvent::Reachability { reachable: false }]);
        let u = parse(
            r#"{"NotificationContainer":{"type":"update.statechange","AutoUpdateNotification":[{"state":"installing"}]}}"#,
        );
        assert_eq!(
            u,
            vec![PlexEvent::UpdateState {
                state: "installing".into()
            }]
        );
        let s = parse(
            r#"{"NotificationContainer":{"type":"status","StatusNotification":[{"notificationName":"SERVER_SHUTDOWN"}]}}"#,
        );
        assert_eq!(s, vec![PlexEvent::Shutdown]);
    }

    #[test]
    fn ignores_garbage_and_unknown() {
        assert!(parse("not json").is_empty());
        assert!(parse(r#"{"NotificationContainer":{"type":"preference"}}"#).is_empty());
    }

    #[test]
    fn websocket_url_switches_scheme_and_has_no_token() {
        use crate::plex::identity::ClientIdentity;
        use crate::plex::token::Token;
        use std::sync::Arc;
        let c = PlexClient::new(
            "https://a.plex.direct:32400",
            Token::new("ws-secret").unwrap(),
            Arc::new(ClientIdentity::with_id("c")),
            None,
        )
        .unwrap();
        let u = websocket_url(&c).unwrap();
        assert_eq!(
            u.as_str(),
            "wss://a.plex.direct:32400/:/websockets/notifications"
        );
        assert!(!u.as_str().contains("ws-secret"));
    }

    #[tokio::test]
    async fn run_reports_disconnect_and_stops() {
        use crate::plex::identity::ClientIdentity;
        use crate::plex::token::Token;
        use std::sync::Arc;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        let c = PlexClient::new(
            &format!("http://{addr}"),
            Token::new("t").unwrap(),
            Arc::new(ClientIdentity::with_id("c")),
            None,
        )
        .unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let (stop_tx, stop_rx) = watch::channel(false);
        let task = tokio::spawn(run(c, tx, stop_rx));
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ev, PlexEvent::Disconnected);
        stop_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("run should stop")
            .unwrap();
    }
}
