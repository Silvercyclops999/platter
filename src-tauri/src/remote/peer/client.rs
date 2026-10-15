//! The sending side: signed requests to another Platter device, and the
//! link the Listen on service follows while playback is there.

use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use super::auth::{AUTH_HEADER, PeerKey, nonce, now_ms, sign};
use super::keys::{KeysDeliver, KeysHello, KeysHelloReply, KeysWaiting, PATH_KEYS_DELIVER, PATH_KEYS_HELLO, PATH_KEYS_WAITING};
use super::{Hello, PATH_COMMAND, PATH_HELLO, PATH_PLAY, PATH_STATE, PATH_STOP, PeerTimeline, PlayRequest};
use crate::error::{AppError, AppResult};
use crate::remote::companion::Timeline;
use crate::remote::lan::LanLink;
use crate::remote::types::{RemoteCommand, repeat_code};

/// Why a device didn't take a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerError {
    /// No answer (off, another network, or a firewall).
    Unreachable,
    /// It answered but refused the signature (not the same account, or the
    /// clocks are far apart).
    Refused(String),
    /// It answered with another error.
    Failed(String),
}

pub const NOT_SAME_ACCOUNT: &str = "Couldn't confirm it's signed in to your Plex account";

impl From<PeerError> for AppError {
    fn from(e: PeerError) -> Self {
        match e {
            PeerError::Unreachable => AppError::Network("unreachable".into()),
            PeerError::Refused(m) => AppError::Auth(m),
            PeerError::Failed(m) => AppError::Invalid(m),
        }
    }
}

/// Signed requests from this computer.
#[derive(Clone)]
pub struct PeerClient {
    http: reqwest::Client,
    own_id: String,
    key: PeerKey,
}

impl PeerClient {
    pub fn new(http: reqwest::Client, own_id: String, key: PeerKey) -> Self {
        Self { http, own_id, key }
    }

    /// This computer's device id.
    pub fn own_id(&self) -> &str {
        &self.own_id
    }

    async fn call<T: DeserializeOwned>(
        &self,
        base: &Url,
        to: &str,
        path: &str,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<T, PeerError> {
        let method = if body.is_some() { "POST" } else { "GET" };
        let body = body.unwrap_or_default();
        let auth = sign(&self.key, method, path, &self.own_id, to, now_ms(), &nonce(), &body);
        let url = base.join(path).map_err(|_| PeerError::Unreachable)?;
        let req = if method == "POST" {
            self.http.post(url).header("content-type", "application/json").body(body)
        } else {
            self.http.get(url)
        };
        let resp = req
            .header(AUTH_HEADER, auth)
            .timeout(timeout)
            .send()
            .await
            .map_err(|_| PeerError::Unreachable)?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|_| PeerError::Unreachable)?;
        if status == 401 {
            return Err(PeerError::Refused(NOT_SAME_ACCOUNT.into()));
        }
        if !(200..300).contains(&status) {
            let msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_string))
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(PeerError::Failed(msg));
        }
        serde_json::from_str(&text).map_err(|_| PeerError::Failed("unreadable answer".into()))
    }

    fn json<T: Serialize>(v: &T) -> Vec<u8> {
        serde_json::to_vec(v).unwrap_or_default()
    }

    pub async fn hello(&self, base: &Url, to: &str, timeout: Duration) -> Result<Hello, PeerError> {
        self.call(base, to, PATH_HELLO, None, timeout).await
    }

    /// The first base that answers hello, with its answer.
    pub async fn first_answer(&self, bases: &[Url], to: &str, timeout: Duration) -> Result<(Url, Hello), PeerError> {
        let mut last = PeerError::Unreachable;
        for b in bases {
            match self.hello(b, to, timeout).await {
                Ok(h) if h.id == to => return Ok((b.clone(), h)),
                Ok(_) => last = PeerError::Failed("another device answered".into()),
                Err(e @ PeerError::Refused(_)) => return Err(e),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    pub async fn state(&self, base: &Url, to: &str, timeout: Duration) -> Result<PeerTimeline, PeerError> {
        self.call(base, to, PATH_STATE, None, timeout).await
    }

    pub async fn play(&self, base: &Url, to: &str, req: &PlayRequest, timeout: Duration) -> Result<(), PeerError> {
        self.call::<serde_json::Value>(base, to, PATH_PLAY, Some(Self::json(req)), timeout).await.map(|_| ())
    }

    pub async fn command(&self, base: &Url, to: &str, cmd: &RemoteCommand, timeout: Duration) -> Result<(), PeerError> {
        self.call::<serde_json::Value>(base, to, PATH_COMMAND, Some(Self::json(cmd)), timeout).await.map(|_| ())
    }

    pub async fn stop(&self, base: &Url, to: &str, timeout: Duration) -> Result<(), PeerError> {
        self.call::<serde_json::Value>(base, to, PATH_STOP, Some(b"{}".to_vec()), timeout).await.map(|_| ())
    }

    /// Whether the device is showing a code to copy keys to it.
    pub async fn keys_waiting(&self, base: &Url, to: &str, timeout: Duration) -> Result<bool, PeerError> {
        self.call::<KeysWaiting>(base, to, PATH_KEYS_WAITING, None, timeout).await.map(|w| w.waiting)
    }

    pub async fn keys_hello(&self, base: &Url, to: &str, req: &KeysHello, timeout: Duration) -> Result<KeysHelloReply, PeerError> {
        self.call(base, to, PATH_KEYS_HELLO, Some(Self::json(req)), timeout).await
    }

    pub async fn keys_deliver(&self, base: &Url, to: &str, req: &KeysDeliver, timeout: Duration) -> Result<(), PeerError> {
        self.call::<serde_json::Value>(base, to, PATH_KEYS_DELIVER, Some(Self::json(req)), timeout).await.map(|_| ())
    }
}

/// The companion timeline the service already follows.
pub fn as_timeline(t: &PeerTimeline) -> Timeline {
    Timeline {
        state: t.state.clone(),
        time_ms: t.time_ms,
        duration_ms: t.duration_ms,
        rating_key: t.track_id.clone(),
        play_queue_id: None,
        play_queue_item_id: None,
        volume: Some((t.volume.clamp(0.0, 1.0) * 100.0).round() as i64),
        shuffle: t.shuffle,
        repeat: i64::from(repeat_code(t.repeat)),
    }
}

/// Playback on another Platter device.
pub struct PeerLink {
    pub client: PeerClient,
    pub base: Url,
    pub id: String,
    pub timeout: Duration,
}

#[async_trait]
impl LanLink for PeerLink {
    async fn poll(&self) -> AppResult<Option<Timeline>> {
        let t = self.client.state(&self.base, &self.id, self.timeout).await?;
        Ok((t.state != "stopped").then(|| as_timeline(&t)))
    }
    async fn command(&self, cmd: &RemoteCommand) -> AppResult<()> {
        Ok(self.client.command(&self.base, &self.id, cmd, self.timeout).await?)
    }
    async fn leave(&self, stop: bool) {
        if stop && let Err(e) = self.client.stop(&self.base, &self.id, self.timeout).await {
            tracing::info!(?e, "the other Platter didn't take stop");
        }
    }
}
