//! Starting playback on a Cast device and following it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::conn::CastConn;
use super::proto::{NS_CONNECTION, NS_MEDIA, NS_PLEX, NS_RECEIVER, RECEIVER};
use super::{CastDevice, DEFAULT_APP, PLEX_APP};
use crate::audio::queue::Repeat;
use crate::error::{AppError, AppResult};
use crate::remote::companion::Timeline;
use crate::remote::lan::LanLink;
use crate::remote::types::{RemoteCommand, repeat_code};

/// What Plex's receiver needs to play a server play queue.
#[derive(Clone)]
pub struct PlexLoad {
    pub rating_key: String,
    pub offset_ms: u64,
    pub machine_id: String,
    pub protocol: String,
    pub address: String,
    pub port: u16,
    /// The most limited token Plex hands out for this (a transient one).
    pub token: String,
    pub play_queue_id: String,
}

/// One song for the default media receiver.
#[derive(Clone)]
pub struct DirectItem {
    pub rating_key: String,
    /// Stream URL. Carries a token: never logged.
    pub url: String,
    pub content_type: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub image: Option<String>,
    pub duration_ms: u64,
}

/// How to start playback on a Cast device.
#[derive(Clone, Default)]
pub struct CastPlan {
    /// Plex's receiver with a play queue (tried first when set).
    pub plex: Option<PlexLoad>,
    /// The queue as stream URLs for the default media receiver.
    pub items: Vec<DirectItem>,
    pub start: usize,
    pub offset_ms: u64,
    pub repeat: Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastApp {
    Plex,
    Default,
}

#[derive(Default)]
struct LinkState {
    media_session: Option<i64>,
    rating_key: Option<String>,
    duration_ms: u64,
}

/// Playback on a Cast device.
pub struct CastLink {
    conn: Arc<CastConn>,
    pub app: CastApp,
    session_id: String,
    transport: String,
    state: Mutex<LinkState>,
}

impl std::fmt::Debug for CastLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CastLink").field("app", &self.app).finish()
    }
}

fn refused(what: &str) -> AppError {
    AppError::Network(format!("cast device refused {what}"))
}

fn str_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The rating key in a media status's `media`, wherever the receiver put it.
fn rating_key_of(media: &Value) -> Option<String> {
    let cd = &media["customData"];
    for v in [&cd["ratingKey"], &cd["key"], &media["contentId"]] {
        if let Some(s) = str_of(v) {
            let k = s.split('?').next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
            if !k.is_empty() && k.chars().all(|c| c.is_ascii_digit()) {
                return Some(k);
            }
        }
    }
    None
}

fn repeat_mode(r: Repeat) -> &'static str {
    match r {
        Repeat::Off => "REPEAT_OFF",
        Repeat::One => "REPEAT_SINGLE",
        Repeat::All => "REPEAT_ALL",
    }
}

fn repeat_from_mode(m: &str) -> Repeat {
    match m {
        "REPEAT_SINGLE" => Repeat::One,
        "REPEAT_ALL" | "REPEAT_ALL_AND_SHUFFLE" => Repeat::All,
        _ => Repeat::Off,
    }
}

/// The app's session and transport in a receiver status, if it's running.
fn running(status: &Value, app: &str) -> Option<(String, String)> {
    let a = status["applications"].as_array()?.iter().find(|a| a["appId"].as_str() == Some(app))?;
    Some((a["sessionId"].as_str()?.to_string(), a["transportId"].as_str()?.to_string()))
}

async fn launch(conn: &CastConn, app: &str) -> AppResult<(String, String)> {
    let r = conn.request(RECEIVER, NS_RECEIVER, json!({"type": "LAUNCH", "appId": app})).await?;
    if r["type"] != "RECEIVER_STATUS" {
        return Err(refused("the app"));
    }
    if let Some(found) = running(&r["status"], app) {
        return Ok(found);
    }
    // Some devices answer before the app is up.
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let r = conn.request(RECEIVER, NS_RECEIVER, json!({"type": "GET_STATUS"})).await?;
        if let Some(found) = running(&r["status"], app) {
            return Ok(found);
        }
    }
    Err(refused("the app"))
}

fn plex_load(p: &PlexLoad, session: &str) -> Value {
    let secs = p.offset_ms as f64 / 1000.0;
    json!({
        "type": "LOAD",
        "sessionId": session,
        "media": {
            "contentId": format!("/library/metadata/{}", crate::sid::key(&p.rating_key)),
            "streamType": "BUFFERED",
            "contentType": "audio",
            "customData": {
                "offset": secs,
                "directPlay": true,
                "directStream": true,
                "subtitleSize": 100,
                "audioBoost": 100,
                "server": {
                    "machineIdentifier": p.machine_id,
                    "transcoderVideo": true,
                    "transcoderVideoRemuxOnly": false,
                    "transcoderAudio": true,
                    "version": "1.10.1.4602",
                    "myPlexSubscription": false,
                    "isVerifiedHostname": p.address.ends_with(".plex.direct"),
                    "protocol": p.protocol,
                    "address": p.address,
                    "port": p.port,
                    "accessToken": p.token,
                    "user": {"username": ""},
                },
                "containerKey": format!("/playQueues/{}?own=1&window=200", p.play_queue_id),
            },
        },
        "autoplay": true,
        "currentTime": secs,
    })
}

fn queue_load(plan: &CastPlan, session: &str) -> Value {
    let items: Vec<Value> = plan
        .items
        .iter()
        .map(|i| {
            let mut metadata = json!({
                "metadataType": 3,
                "title": i.title,
                "artist": i.artist,
                "albumName": i.album,
            });
            if let Some(img) = &i.image {
                metadata["images"] = json!([{"url": img}]);
            }
            json!({
                "media": {
                    "contentId": i.url,
                    "streamType": "BUFFERED",
                    "contentType": i.content_type,
                    "duration": i.duration_ms as f64 / 1000.0,
                    "metadata": metadata,
                    "customData": {"ratingKey": i.rating_key},
                },
                "autoplay": true,
                "preloadTime": 10,
            })
        })
        .collect();
    json!({
        "type": "QUEUE_LOAD",
        "sessionId": session,
        "items": items,
        "startIndex": plan.start.min(plan.items.len().saturating_sub(1)),
        "currentTime": plan.offset_ms as f64 / 1000.0,
        "repeatMode": repeat_mode(plan.repeat),
    })
}

fn media_session_of(reply: &Value) -> AppResult<i64> {
    if reply["type"] != "MEDIA_STATUS" {
        return Err(refused("the music"));
    }
    reply["status"][0]["mediaSessionId"]
        .as_i64()
        .ok_or_else(|| refused("the music"))
}

impl CastLink {
    /// Connects and starts playback: Plex's receiver first, the default
    /// receiver if that fails.
    pub async fn start(
        dev: &CastDevice,
        plan: &CastPlan,
        tls: Arc<rustls::ClientConfig>,
        timeout: Duration,
    ) -> AppResult<Self> {
        let conn = Arc::new(CastConn::connect(dev.addr, tls, timeout).await?);
        if let Some(p) = &plan.plex {
            match Self::start_app(&conn, CastApp::Plex, plex_load(p, "")).await {
                Ok(link) => {
                    link.state.lock().rating_key = Some(p.rating_key.clone());
                    return Ok(link);
                }
                Err(e) => tracing::info!(kind = e.kind(), "Plex's cast receiver didn't start; trying the default one"),
            }
        }
        if plan.items.is_empty() {
            return Err(refused("the music"));
        }
        let link = Self::start_app(&conn, CastApp::Default, queue_load(plan, "")).await?;
        link.state.lock().rating_key = plan.items.get(plan.start).map(|i| i.rating_key.clone());
        Ok(link)
    }

    async fn start_app(conn: &Arc<CastConn>, app: CastApp, mut load: Value) -> AppResult<Self> {
        let id = match app {
            CastApp::Plex => PLEX_APP,
            CastApp::Default => DEFAULT_APP,
        };
        let (session_id, transport) = launch(conn, id).await?;
        conn.open_channel(&transport)?;
        conn.watch_transport(&transport);
        load["sessionId"] = json!(session_id);
        let reply = conn.request(&transport, NS_MEDIA, load).await?;
        let media_session = media_session_of(&reply)?;
        Ok(Self {
            conn: conn.clone(),
            app,
            session_id,
            transport,
            state: Mutex::new(LinkState {
                media_session: Some(media_session),
                ..Default::default()
            }),
        })
    }

    fn media(&self, mut payload: Value) -> AppResult<()> {
        if let Some(id) = self.state.lock().media_session {
            payload["mediaSessionId"] = json!(id);
        }
        self.conn.send(&self.transport, NS_MEDIA, payload)
    }

    fn timeline_of(&self, status: &Value) -> Option<Timeline> {
        let state = match status["playerState"].as_str().unwrap_or("") {
            "PLAYING" => "playing",
            "PAUSED" => "paused",
            "BUFFERING" | "LOADING" => "buffering",
            "IDLE" if status["idleReason"].is_null() => "buffering",
            _ => return None,
        };
        let mut st = self.state.lock();
        if let Some(id) = status["mediaSessionId"].as_i64() {
            st.media_session = Some(id);
        }
        let media = &status["media"];
        if let Some(k) = rating_key_of(media) {
            st.rating_key = Some(k);
        } else if let (Some(cur), Some(items)) = (status["currentItemId"].as_i64(), status["items"].as_array()) {
            let k = items
                .iter()
                .find(|i| i["itemId"].as_i64() == Some(cur))
                .and_then(|i| rating_key_of(&i["media"]));
            if k.is_some() {
                st.rating_key = k;
            }
        }
        if let Some(d) = media["duration"].as_f64().filter(|d| d.is_finite() && *d > 0.0) {
            st.duration_ms = (d * 1000.0) as u64;
        }
        let volume = self.conn.receiver_status().and_then(|r| {
            let v = &r["volume"];
            let level = v["level"].as_f64()?;
            Some(if v["muted"].as_bool() == Some(true) { 0 } else { (level * 100.0).round() as i64 })
        });
        let repeat = repeat_from_mode(status["repeatMode"].as_str().unwrap_or(""));
        Some(Timeline {
            state: state.to_string(),
            time_ms: (status["currentTime"].as_f64().unwrap_or(0.0).max(0.0) * 1000.0) as u64,
            duration_ms: st.duration_ms,
            rating_key: st.rating_key.clone(),
            play_queue_id: None,
            play_queue_item_id: None,
            play_queue_version: None,
            volume,
            shuffle: false,
            repeat: repeat_code(repeat) as i64,
        })
    }

    fn app_gone(&self) -> bool {
        if self.conn.app_closed() {
            return true;
        }
        let id = match self.app {
            CastApp::Plex => PLEX_APP,
            CastApp::Default => DEFAULT_APP,
        };
        // Another app took the device over, or it went back to idle.
        self.conn
            .receiver_status()
            .is_some_and(|r| running(&r, id).is_none_or(|(s, _)| s != self.session_id))
    }
}

#[async_trait]
impl LanLink for CastLink {
    async fn poll(&self) -> AppResult<Option<Timeline>> {
        if self.conn.is_closed() {
            return Err(AppError::Network("cast device closed the connection".into()));
        }
        if self.app_gone() {
            return Ok(None);
        }
        let reply = self.conn.request(&self.transport, NS_MEDIA, json!({"type": "GET_STATUS"})).await?;
        if self.app_gone() {
            return Ok(None);
        }
        Ok(reply["status"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|s| self.timeline_of(s)))
    }

    async fn command(&self, cmd: &RemoteCommand) -> AppResult<()> {
        let plex = self.app == CastApp::Plex;
        let simple = |ty: &str| -> AppResult<()> {
            if plex {
                self.conn.send(&self.transport, NS_PLEX, json!({"type": ty}))
            } else {
                self.media(json!({"type": ty}))
            }
        };
        match cmd {
            RemoteCommand::Play => simple("PLAY"),
            RemoteCommand::Pause => simple("PAUSE"),
            RemoteCommand::Next if plex => simple("NEXT"),
            RemoteCommand::Previous if plex => simple("PREVIOUS"),
            RemoteCommand::Next => self.media(json!({"type": "QUEUE_UPDATE", "jump": 1})),
            RemoteCommand::Previous => self.media(json!({"type": "QUEUE_UPDATE", "jump": -1})),
            RemoteCommand::Seek { position_ms } => {
                let secs = *position_ms as f64 / 1000.0;
                if plex {
                    self.conn.send(
                        &self.transport,
                        NS_PLEX,
                        json!({"type": "SEEK", "currentTime": secs, "resumeState": "PLAYBACK_START"}),
                    )
                } else {
                    self.media(json!({"type": "SEEK", "currentTime": secs}))
                }
            }
            RemoteCommand::Volume { volume } => {
                let v = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 0.0 };
                self.conn
                    .send(RECEIVER, NS_RECEIVER, json!({"type": "SET_VOLUME", "volume": {"level": v}}))
            }
            RemoteCommand::Shuffle { on } => self.media(json!({"type": "QUEUE_UPDATE", "shuffle": on})),
            RemoteCommand::Repeat { mode } => {
                self.media(json!({"type": "QUEUE_UPDATE", "repeatMode": repeat_mode(*mode)}))
            }
        }
    }

    async fn leave(&self, stop: bool) {
        if stop {
            let _ = self
                .conn
                .request(RECEIVER, NS_RECEIVER, json!({"type": "STOP", "sessionId": self.session_id}))
                .await;
        } else {
            let _ = self.conn.send(&self.transport, NS_CONNECTION, json!({"type": "CLOSE"}));
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn rating_keys_come_from_custom_data_or_the_content_id() {
        assert_eq!(rating_key_of(&json!({"customData": {"ratingKey": "12"}})).as_deref(), Some("12"));
        assert_eq!(rating_key_of(&json!({"contentId": "/library/metadata/34"})).as_deref(), Some("34"));
        assert_eq!(rating_key_of(&json!({"customData": {"key": "/library/metadata/56?x=1"}})).as_deref(), Some("56"));
        assert_eq!(rating_key_of(&json!({"contentId": "http://x/file.flac"})), None);
    }

    #[test]
    fn repeat_modes_round_trip() {
        for r in [Repeat::Off, Repeat::One, Repeat::All] {
            assert_eq!(repeat_from_mode(repeat_mode(r)), r);
        }
    }

    #[test]
    fn plex_load_carries_the_queue_and_offset() {
        let p = PlexLoad {
            rating_key: "7".into(),
            offset_ms: 61_500,
            machine_id: "srv".into(),
            protocol: "https".into(),
            address: "a.b.plex.direct".into(),
            port: 32400,
            token: "transient".into(),
            play_queue_id: "99".into(),
        };
        let v = plex_load(&p, "s1");
        assert_eq!(v["media"]["contentId"], "/library/metadata/7");
        assert_eq!(v["media"]["customData"]["containerKey"], "/playQueues/99?own=1&window=200");
        assert_eq!(v["media"]["customData"]["server"]["isVerifiedHostname"], true);
        assert_eq!(v["currentTime"], 61.5);
        assert_eq!(v["media"]["contentType"], "audio");
    }

    #[test]
    fn plex_load_names_the_song_as_its_server_knows_it() {
        let p = PlexLoad {
            rating_key: format!("{}:7", crate::sid::tag("srv")),
            offset_ms: 0,
            machine_id: "srv".into(),
            protocol: "https".into(),
            address: "a.b.plex.direct".into(),
            port: 32400,
            token: "transient".into(),
            play_queue_id: "99".into(),
        };
        assert_eq!(plex_load(&p, "s1")["media"]["contentId"], "/library/metadata/7");
    }
}
