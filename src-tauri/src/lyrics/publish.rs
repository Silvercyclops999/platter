//! Sharing a transcription to LRCLIB (docs/IPC.md "Sharing lyrics"). Only
//! ever one song, after the user saw exactly what goes and said yes.
//!
//! LRCLIB's publish API: `POST /api/request-challenge` answers a `prefix`
//! and a `target`; the client finds a nonce whose SHA-256 of
//! `prefix + nonce` is at or below the target (read as a big-endian
//! number), then `POST /api/publish` with `X-Publish-Token: prefix:nonce`
//! and the song's title, artist, album, length and lyrics. 201 means
//! published. The search runs on its own thread at low priority and stops
//! when cancelled.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What is sent to LRCLIB, as the share dialog shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    pub track_name: String,
    pub artist_name: String,
    pub album_name: String,
    /// Seconds.
    pub duration: f64,
    pub plain_lyrics: String,
    pub synced_lyrics: String,
}

/// Why sharing didn't happen, in words for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    Cancelled,
    /// LRCLIB couldn't be reached.
    Offline,
    /// LRCLIB said no (with its reason, when it gave one).
    Rejected(String),
    /// LRCLIB is busy or failed on its side.
    Busy,
}

impl PublishError {
    pub fn message(&self) -> String {
        match self {
            Self::Cancelled => "Sharing was cancelled. Nothing was sent.".into(),
            Self::Offline => "Couldn't reach LRCLIB. Check your connection and try again. Nothing was shared.".into(),
            Self::Rejected(why) if !why.is_empty() => format!("LRCLIB didn't take these lyrics: {why}"),
            Self::Rejected(_) => "LRCLIB didn't take these lyrics. Nothing was shared.".into(),
            Self::Busy => "LRCLIB is busy right now. Try again in a while. Nothing was shared.".into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Challenge {
    prefix: String,
    target: String,
}

#[derive(Debug, Default, Deserialize)]
struct ApiError {
    #[serde(default)]
    message: String,
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Whether `nonce` answers the challenge.
pub fn answers(prefix: &str, target: &[u8], nonce: &str) -> bool {
    let mut h = Sha256::new();
    h.update(prefix.as_bytes());
    h.update(nonce.as_bytes());
    h.finalize().as_slice() <= target
}

/// Run the search on this thread, giving way to everything else. None when
/// cancelled or the target can't be read.
pub fn solve(prefix: &str, target_hex: &str, cancel: &AtomicBool) -> Option<String> {
    let target = unhex(target_hex).filter(|t| t.len() == 32)?;
    lower_priority();
    let mut n: u64 = 0;
    loop {
        if n.is_multiple_of(4096) {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            std::thread::yield_now();
        }
        let nonce = n.to_string();
        if answers(prefix, &target, &nonce) {
            return Some(nonce);
        }
        n += 1;
    }
}

/// This thread runs behind everything else on the computer.
fn lower_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain syscalls on the calling thread.
    unsafe {
        let tid = libc::gettid();
        libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 15);
    }
    #[cfg(target_os = "windows")]
    // SAFETY: the pseudo handle for the calling thread.
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST};
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
    }
}

/// The LRCLIB publish client.
#[derive(Clone)]
pub struct Publisher {
    client: reqwest::Client,
    base: String,
}

pub const BASE: &str = super::lrclib::BASE;

impl Publisher {
    pub fn new() -> Self {
        Self::with_base(BASE)
    }

    /// Against another server (tests).
    pub fn with_base(base: &str) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(crate::artistinfo::http::user_agent())
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_default();
        Self { client, base: base.trim_end_matches('/').to_owned() }
    }

    /// Ask for a challenge, solve it, publish. Nothing is published unless
    /// every step works.
    pub async fn publish(&self, p: &Payload, cancel: Arc<AtomicBool>) -> Result<(), PublishError> {
        let resp = self
            .client
            .post(format!("{}/request-challenge", self.base))
            .send()
            .await
            .map_err(|_| PublishError::Offline)?;
        let status = resp.status().as_u16();
        if status >= 500 || status == 429 {
            return Err(PublishError::Busy);
        }
        let body = resp.bytes().await.map_err(|_| PublishError::Offline)?;
        let ch: Challenge = serde_json::from_slice(&body).map_err(|_| PublishError::Busy)?;
        let nonce = {
            let cancel = cancel.clone();
            let (prefix, target) = (ch.prefix.clone(), ch.target.clone());
            std::thread::Builder::new()
                .name("lrclib-challenge".into())
                .spawn(move || solve(&prefix, &target, &cancel))
                .map(|h| tokio::task::spawn_blocking(move || h.join().ok().flatten()))
                .map_err(|_| PublishError::Busy)?
                .await
                .ok()
                .flatten()
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(PublishError::Cancelled);
        }
        let Some(nonce) = nonce else {
            return Err(PublishError::Busy);
        };
        let resp = self
            .client
            .post(format!("{}/publish", self.base))
            .header("X-Publish-Token", format!("{}:{nonce}", ch.prefix))
            .json(p)
            .send()
            .await
            .map_err(|_| PublishError::Offline)?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(());
        }
        if status >= 500 || status == 429 {
            return Err(PublishError::Busy);
        }
        let body = resp.bytes().await.unwrap_or_default();
        let e: ApiError = serde_json::from_slice(&body).unwrap_or_default();
        Err(PublishError::Rejected(e.message.trim().to_owned()))
    }
}

impl Default for Publisher {
    fn default() -> Self {
        Self::new()
    }
}

/// Transcribed (enhanced) LRC as LRCLIB takes it: synced lines without word
/// times, and the plain words.
pub fn lrc_for_lrclib(text: &str) -> (String, String) {
    let mut synced = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // The line's own time tags stay; word tags go.
        let mut stamp = String::new();
        let mut rest = line;
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            stamp.push_str(&rest[..=end]);
            rest = &rest[end + 1..];
        }
        let mut words = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '<' {
                for c in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                }
                continue;
            }
            words.push(c);
        }
        let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
        if stamp.is_empty() {
            if !words.is_empty() {
                plain.push(words);
            }
            continue;
        }
        synced.push(format!("{stamp}{words}"));
        if words.is_empty() {
            if plain.last().is_some_and(|l| !l.is_empty()) {
                plain.push(String::new());
            }
        } else {
            plain.push(words);
        }
    }
    while plain.last().is_some_and(|l| l.is_empty()) {
        plain.pop();
    }
    (synced.join("\n"), plain.join("\n"))
}
