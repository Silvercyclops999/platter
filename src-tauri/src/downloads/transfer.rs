//! One file fetch through the player's transfer gate at background
//! priority. It waits while playback's own streams are still downloading,
//! yields the moment anything else wants the gate, and picks up again by
//! Range (or from the start, for a transcode).

use std::path::Path;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::RANGE;
use tokio::io::AsyncWriteExt;

use super::Fetch;
use crate::audio::cache::PrecacheHost;
use crate::audio::gate::{Priority, TransferGate};

/// Network failures in a row before the song counts as failed.
const TRIES: u32 = 3;
const IDLE_POLL: Duration = Duration::from_millis(250);

pub(super) struct Ctx<'a> {
    pub client: &'a reqwest::Client,
    pub gate: &'a std::sync::Arc<TransferGate>,
    pub host: Option<&'a dyn PrecacheHost>,
    /// Bytes left under the storage limit.
    pub room: u64,
    /// False once the song is no longer wanted or the queue must pause.
    pub keep_going: &'a (dyn Fn() -> bool + Sync),
    pub progress: &'a (dyn Fn(u64, Option<u64>) + Sync),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// The whole file is in the partial file; its size.
    Done(u64),
    Failed(String),
    /// It would go past the storage limit.
    Full,
    Stopped,
}

enum Attempt {
    Done(u64),
    Yield,
    Retry(String),
    Fatal(String),
    Full,
    Stopped,
}

fn idle(ctx: &Ctx<'_>) -> bool {
    ctx.host.is_none_or(|h| h.precache_allowed())
}

pub(super) async fn fetch_file(ctx: &Ctx<'_>, fetch: &Fetch, part: &Path) -> Outcome {
    let prio = Priority::Background.cell();
    let mut failures = 0u32;
    loop {
        if !(ctx.keep_going)() {
            return Outcome::Stopped;
        }
        // Playback's streams first.
        if !idle(ctx) {
            tokio::time::sleep(IDLE_POLL).await;
            continue;
        }
        let permit = ctx.gate.acquire(&prio).await;
        if !idle(ctx) {
            drop(permit);
            continue;
        }
        let a = attempt(ctx, fetch, part, &permit).await;
        drop(permit);
        match a {
            Attempt::Done(n) => return Outcome::Done(n),
            Attempt::Full => return Outcome::Full,
            Attempt::Stopped => return Outcome::Stopped,
            Attempt::Fatal(e) => return Outcome::Failed(e),
            Attempt::Yield => {
                // Something better wanted the gate; give it a moment.
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Attempt::Retry(e) => {
                failures += 1;
                if failures >= TRIES {
                    return Outcome::Failed(e);
                }
                tokio::time::sleep(Duration::from_millis(500 * failures as u64)).await;
            }
        }
    }
}

fn status_error(code: StatusCode) -> Attempt {
    let msg = match code.as_u16() {
        404 | 410 => "The file isn't on the server any more.".to_string(),
        401 | 403 => "The server refused the download.".to_string(),
        c => format!("The server answered {c}."),
    };
    if code.is_client_error() { Attempt::Fatal(msg) } else { Attempt::Retry(msg) }
}

async fn attempt(ctx: &Ctx<'_>, fetch: &Fetch, part: &Path, permit: &crate::audio::gate::Permit) -> Attempt {
    let have = if fetch.resumable {
        tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    let mut req = ctx.client.get(&fetch.src.url);
    for (k, v) in &fetch.src.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if fetch.resumable && have > 0 {
        req = req.header(RANGE, format!("bytes={have}-"));
    }
    let sent = tokio::select! {
        r = req.send() => r,
        _ = permit.preempted() => return Attempt::Yield,
    };
    let mut resp = match sent {
        Ok(r) => r,
        Err(e) => return Attempt::Retry(format!("Network trouble: {}", e.without_url())),
    };
    let status = resp.status();
    if status == StatusCode::RANGE_NOT_SATISFIABLE && have > 0 {
        return Attempt::Done(have);
    }
    if !status.is_success() {
        return status_error(status);
    }
    let append = status == StatusCode::PARTIAL_CONTENT;
    let total = crate::audio::http::parse_total(resp.headers())
        .or_else(|| resp.content_length().map(|n| if append { n + have } else { n }));
    let file = if append {
        tokio::fs::OpenOptions::new().append(true).create(true).open(part).await
    } else {
        tokio::fs::File::create(part).await
    };
    let mut f = match file {
        Ok(f) => f,
        Err(e) => return Attempt::Fatal(format!("Couldn't write the file: {e}")),
    };
    let mut written = if append { have } else { 0 };
    if total.is_some_and(|t| t > ctx.room) {
        return Attempt::Full;
    }
    (ctx.progress)(written, total);
    loop {
        let next = tokio::select! {
            c = resp.chunk() => c,
            _ = permit.preempted() => {
                let _ = f.flush().await;
                // A transcode starts over next time (`have` is 0 for it).
                return Attempt::Yield;
            }
        };
        match next {
            Ok(Some(chunk)) => {
                if let Err(e) = f.write_all(&chunk).await {
                    let _ = f.flush().await;
                    return Attempt::Fatal(format!("Couldn't write the file: {e}"));
                }
                written += chunk.len() as u64;
                if written > ctx.room {
                    let _ = f.flush().await;
                    return Attempt::Full;
                }
                (ctx.progress)(written, total);
                if !(ctx.keep_going)() {
                    let _ = f.flush().await;
                    return Attempt::Stopped;
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = f.flush().await;
                return Attempt::Retry(format!("Network trouble: {}", e.without_url()));
            }
        }
    }
    if let Err(e) = f.flush().await {
        return Attempt::Fatal(format!("Couldn't write the file: {e}"));
    }
    match total {
        Some(t) if written < t => Attempt::Retry("The download stopped early.".into()),
        _ => Attempt::Done(written),
    }
}
