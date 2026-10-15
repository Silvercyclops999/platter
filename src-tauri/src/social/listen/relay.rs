//! The relay every listen along message goes through, so participants only
//! ever connect to it and never to each other.
//!
//! [`Relay`] is the seam: [`Ntfy`] talks to an ntfy server (ntfy.sh by
//! default, or the user's own), and another relay can stand in later.
//!
//! ntfy, as used here:
//! - publish: `POST <base>/<topic>` with the sealed text as the body, and
//!   `X-Firebase: no` so ntfy.sh doesn't pass it on to Firebase.
//! - subscribe: `GET <base>/<t1>,<t2>/json[?since=<id>]`, one JSON object a
//!   line: `{"event":"open"|"keepalive"|"message", "id", "time", "topic",
//!   "message"}`. Messages carry no publisher identity or address.
//!
//! Errors never include the URL (it has topic names in it).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde::Serialize;
use tokio::sync::mpsc;

/// What a subscription delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayEvent {
    /// Connected and listening.
    Open,
    Message { id: String, topic: String, body: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayError {
    Unreachable,
    /// The relay is turning requests away for a while (ntfy answers 429).
    Limited,
    /// Any other refusal (status code).
    Refused(u16),
}

/// How the relay connection is doing, for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RelayHealth {
    #[default]
    Connecting,
    Ok,
    Unreachable,
    Limited,
}

impl From<RelayError> for RelayHealth {
    fn from(e: RelayError) -> Self {
        match e {
            RelayError::Limited => RelayHealth::Limited,
            _ => RelayHealth::Unreachable,
        }
    }
}

#[async_trait]
pub trait Relay: Send + Sync {
    async fn publish(&self, topic: &str, body: &str) -> Result<(), RelayError>;
    /// Listen on these topics. The channel closes when the connection ends.
    async fn subscribe(&self, topics: &[String], since: Option<&str>) -> Result<mpsc::Receiver<RelayEvent>, RelayError>;
}

/// Makes a relay for a base URL.
pub trait RelayFactory: Send + Sync {
    fn relay(&self, base: &str) -> Arc<dyn Relay>;
}

/// ntfy servers.
pub struct Ntfy {
    http: reqwest::Client,
    /// No bytes (not even a keepalive) for this long: the stream is dead.
    idle: Duration,
}

impl Ntfy {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "relay http client".to_string())?;
        // ntfy sends a keepalive every 45 s by default.
        Ok(Self { http, idle: Duration::from_secs(120) })
    }
}

impl RelayFactory for Ntfy {
    fn relay(&self, base: &str) -> Arc<dyn Relay> {
        Arc::new(NtfyRelay { http: self.http.clone(), base: base.trim_end_matches('/').to_string(), idle: self.idle })
    }
}

pub struct NtfyRelay {
    http: reqwest::Client,
    base: String,
    idle: Duration,
}

fn status_err(code: u16) -> RelayError {
    if code == 429 { RelayError::Limited } else { RelayError::Refused(code) }
}

fn ok_topic(t: &str) -> bool {
    !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// One line of the JSON stream.
pub fn parse_line(line: &str) -> Option<RelayEvent> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    match v.get("event")?.as_str()? {
        "open" => Some(RelayEvent::Open),
        "message" => Some(RelayEvent::Message {
            id: v.get("id")?.as_str()?.to_string(),
            topic: v.get("topic")?.as_str()?.to_string(),
            body: v.get("message")?.as_str()?.to_string(),
        }),
        _ => None,
    }
}

#[async_trait]
impl Relay for NtfyRelay {
    async fn publish(&self, topic: &str, body: &str) -> Result<(), RelayError> {
        if !ok_topic(topic) {
            return Err(RelayError::Refused(400));
        }
        let resp = self
            .http
            .post(format!("{}/{topic}", self.base))
            .header("X-Firebase", "no")
            .header("content-type", "text/plain")
            .body(body.to_string())
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|_| RelayError::Unreachable)?;
        let code = resp.status().as_u16();
        if resp.status().is_success() { Ok(()) } else { Err(status_err(code)) }
    }

    async fn subscribe(&self, topics: &[String], since: Option<&str>) -> Result<mpsc::Receiver<RelayEvent>, RelayError> {
        if topics.is_empty() || !topics.iter().all(|t| ok_topic(t)) {
            return Err(RelayError::Refused(400));
        }
        let mut req = self.http.get(format!("{}/{}/json", self.base, topics.join(",")));
        if let Some(id) = since.filter(|s| ok_topic(s)) {
            req = req.query(&[("since", id)]);
        }
        let resp = req.send().await.map_err(|_| RelayError::Unreachable)?;
        if !resp.status().is_success() {
            return Err(status_err(resp.status().as_u16()));
        }
        let (tx, rx) = mpsc::channel(64);
        let idle = self.idle;
        tokio::spawn(async move {
            let mut stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            loop {
                let chunk = match tokio::time::timeout(idle, stream.next()).await {
                    Ok(Some(Ok(c))) => c,
                    _ => return,
                };
                buf.extend_from_slice(&chunk);
                while let Some(i) = buf.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=i).collect();
                    if let Some(ev) = std::str::from_utf8(&line).ok().and_then(parse_line)
                        && tx.send(ev).await.is_err()
                    {
                        return;
                    }
                }
                if buf.len() > 256 * 1024 {
                    return;
                }
            }
        });
        Ok(rx)
    }
}
