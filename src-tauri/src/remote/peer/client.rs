//! The sending side: signed requests to another Platter device, and the
//! link the Listen on service follows while playback is there.

use std::sync::Arc;
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
    /// It answered but refused the request; the text says why (see
    /// [`refusal`]).
    Refused(String),
    /// It answered with another error.
    Failed(String),
}

pub const NOT_SAME_ACCOUNT: &str = "It's signed in to a different Plex account or Home user";
pub const NEEDS_UPDATE: &str = "It needs the latest version of Platter";
pub const CANT_CHECK: &str = "It couldn't check this computer with plex.tv. Try again in a moment";
pub const SIGNED_OUT: &str = "It isn't signed in to Plex";
pub const NOT_ACCEPTED: &str = "It didn't accept the request";

/// "Its clock is 7 minutes behind this computer's".
pub fn clock_message(theirs_minus_ours_ms: i64) -> String {
    let mins = (theirs_minus_ours_ms.unsigned_abs() + 30_000) / 60_000;
    let way = if theirs_minus_ours_ms < 0 { "behind" } else { "ahead of" };
    let amount = match mins {
        0 | 1 => "about a minute".to_string(),
        m if m < 120 => format!("{m} minutes"),
        m if m < 48 * 60 => format!("{} hours", (m + 30) / 60),
        m => format!("{} days", (m + 12 * 60) / (24 * 60)),
    };
    format!("Its clock is {amount} {way} this computer's. Set both to the right time")
}

/// What a 401 answer means, from its body. `now_ms` is this computer's
/// clock when the answer came.
pub fn refusal(body: &str, now_ms: u64) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let checks_devices = v["auth"].as_u64().is_some_and(|a| a >= 2);
    match v["error"].as_str().unwrap_or_default() {
        "clock" => match v["now"].as_u64() {
            Some(theirs) => clock_message(theirs as i64 - now_ms as i64),
            None => "The two computers' clocks are too far apart. Set both to the right time".into(),
        },
        // Before devices checked senders against the account's device list,
        // they refused every other device this way.
        "notYourAccount" if !checks_devices => NEEDS_UPDATE.into(),
        "notYourAccount" => NOT_SAME_ACCOUNT.into(),
        "cantCheck" => CANT_CHECK.into(),
        "signedOut" => SIGNED_OUT.into(),
        _ => NOT_ACCEPTED.into(),
    }
}

/// The receiver's clock, when it refused a request for the time.
fn their_clock(body: &str) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    (v["error"] == "clock").then(|| v["now"].as_u64()).flatten()
}

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
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl PeerClient {
    pub fn new(http: reqwest::Client, own_id: String, key: PeerKey) -> Self {
        Self { http, own_id, key, clock: Arc::new(now_ms) }
    }

    /// Another clock than the system's (tests).
    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> u64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
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
        let url = base.join(path).map_err(|_| PeerError::Unreachable)?;
        // Signed with this computer's time; if the other one says its clock
        // is far off, once more with its time.
        let mut ts = (self.clock)();
        let mut corrected = false;
        let (status, text) = loop {
            let auth = sign(&self.key, method, path, &self.own_id, to, ts, &nonce(), &body);
            let req = if method == "POST" {
                self.http.post(url.clone()).header("content-type", "application/json").body(body.clone())
            } else {
                self.http.get(url.clone())
            };
            let sent = (self.clock)();
            let resp = req
                .header(AUTH_HEADER, auth)
                .timeout(timeout)
                .send()
                .await
                .map_err(|_| PeerError::Unreachable)?;
            let status = resp.status().as_u16();
            let text = resp.text().await.map_err(|_| PeerError::Unreachable)?;
            if status == 401
                && !corrected
                && let Some(theirs) = their_clock(&text)
            {
                tracing::info!(off_ms = theirs as i64 - sent as i64, "the other Platter's clock is off; signing with its time");
                ts = theirs.saturating_add((self.clock)().saturating_sub(sent) / 2);
                corrected = true;
                continue;
            }
            break (status, text);
        };
        if status == 401 {
            return Err(PeerError::Refused(refusal(&text, (self.clock)())));
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
        play_queue_version: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_say_what_went_wrong() {
        let now = 50_000_000;
        let body = |code: &str| format!(r#"{{"error":"{code}","message":"x","auth":2,"now":{now}}}"#);
        assert_eq!(refusal(&body("notYourAccount"), now), NOT_SAME_ACCOUNT);
        assert_eq!(refusal(&body("cantCheck"), now), CANT_CHECK);
        assert_eq!(refusal(&body("signedOut"), now), SIGNED_OUT);
        assert_eq!(refusal(&body("replayed"), now), NOT_ACCEPTED);
        assert_eq!(refusal("not json", now), NOT_ACCEPTED);
        // An older Platter refused every other device this way.
        let old = r#"{"error":"notYourAccount","message":"That device isn't signed in to the same Plex account."}"#;
        assert_eq!(refusal(old, now), NEEDS_UPDATE);
        let old_clock = r#"{"error":"clock","message":"x"}"#;
        assert!(refusal(old_clock, now).starts_with("The two computers' clocks are too far apart"));
    }

    #[test]
    fn clock_differences_read_naturally() {
        assert_eq!(clock_message(-7 * 60_000), "Its clock is 7 minutes behind this computer's. Set both to the right time");
        assert_eq!(clock_message(150_000), "Its clock is 3 minutes ahead of this computer's. Set both to the right time");
        assert!(clock_message(60_000).contains("about a minute ahead"));
        assert!(clock_message(-5 * 3_600_000).contains("5 hours behind"));
        assert!(clock_message(3 * 86_400_000).contains("3 days ahead"));
    }
}
