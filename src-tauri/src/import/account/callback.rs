//! The sign-in answer: a one-off listener on 127.0.0.1 that the browser is
//! sent back to. It's open only while a sign-in waits, takes the first
//! answer to `/callback`, and closes.
//!
//! The answer must carry the `state` this sign-in made; anything else is
//! refused. Other paths (a browser asking for an icon) get a 404 and the
//! wait goes on.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::Fail;

/// The path the developer app's redirect URI names.
pub const PATH: &str = "/callback";

/// How long a sign-in waits for the browser.
pub const WAIT: Duration = Duration::from_secs(300);

/// Most of a request the listener reads.
const REQUEST_MAX: usize = 16 * 1024;

const DONE_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Platter</title></head>\
<body style=\"font-family:system-ui,sans-serif;padding:48px;background:#141414;color:#eee\">\
<h1 style=\"font-size:20px\">You're signed in</h1><p>You can close this tab and go back to Platter.</p></body></html>";

const REFUSED_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Platter</title></head>\
<body style=\"font-family:system-ui,sans-serif;padding:48px;background:#141414;color:#eee\">\
<h1 style=\"font-size:20px\">Not signed in</h1><p>Go back to Platter to see why, or to try again.</p></body></html>";

/// A listener on a free port of 127.0.0.1.
pub async fn bind() -> Result<TcpListener, Fail> {
    TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| Fail::Other(format!("Couldn't open the sign-in answer port: {e}")))
}

/// The redirect URI for a listener's port.
pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{PATH}")
}

/// What one request to the listener was.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// Not the callback path.
    Other,
    Code(String),
    /// The sign-in page said no (`access_denied` and the like).
    Error(String),
    /// The state didn't match this sign-in.
    WrongState,
}

/// Reads a request target such as `/callback?code=..&state=..`.
pub fn parse_target(target: &str, expected_state: &str) -> Answer {
    let Ok(u) = url::Url::parse(&format!("http://127.0.0.1{target}")) else { return Answer::Other };
    if u.path() != PATH {
        return Answer::Other;
    }
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (k, v) in u.query_pairs() {
        match &*k {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            _ => {}
        }
    }
    // A mismatched state is refused before anything else is believed.
    if state.as_deref() != Some(expected_state) {
        return Answer::WrongState;
    }
    if let Some(e) = error {
        return Answer::Error(e);
    }
    match code.filter(|c| !c.is_empty()) {
        Some(c) => Answer::Code(c),
        None => Answer::Error("no code".into()),
    }
}

async fn read_target(s: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let read = async {
        loop {
            let n = s.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() >= REQUEST_MAX {
                break;
            }
        }
        Some(())
    };
    tokio::time::timeout(Duration::from_secs(5), read).await.ok()??;
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().next()?;
    let mut parts = line.split(' ');
    let method = parts.next()?;
    let target = parts.next()?;
    (method == "GET").then(|| target.to_string())
}

async fn reply(s: &mut TcpStream, status: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes()).await;
    let _ = s.write_all(body.as_bytes()).await;
    let _ = s.shutdown().await;
}

/// Waits for the browser's answer and returns the code. Stops on `cancel`,
/// after `wait`, or on a refused or wrong answer.
pub async fn wait(listener: TcpListener, expected_state: &str, wait: Duration, cancel: Arc<AtomicBool>) -> Result<String, Fail> {
    let deadline = Instant::now() + wait;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Fail::Stopped);
        }
        if Instant::now() >= deadline {
            return Err(Fail::Refused("The sign-in page wasn't finished in time. Try again.".into()));
        }
        let Ok(accepted) = tokio::time::timeout(Duration::from_millis(250), listener.accept()).await else { continue };
        let Ok((mut s, _)) = accepted else { continue };
        let Some(target) = read_target(&mut s).await else {
            reply(&mut s, "400 Bad Request", REFUSED_PAGE).await;
            continue;
        };
        match parse_target(&target, expected_state) {
            Answer::Other => reply(&mut s, "404 Not Found", "").await,
            Answer::Code(c) => {
                reply(&mut s, "200 OK", DONE_PAGE).await;
                return Ok(c);
            }
            Answer::Error(e) => {
                reply(&mut s, "200 OK", REFUSED_PAGE).await;
                return Err(Fail::Refused(if e == "access_denied" {
                    "You didn't allow Platter to read your playlists.".into()
                } else {
                    "The sign-in page didn't sign you in. Try again.".into()
                }));
            }
            Answer::WrongState => {
                reply(&mut s, "400 Bad Request", REFUSED_PAGE).await;
                return Err(Fail::Refused("That sign-in answer wasn't for this sign-in, so it was refused. Try again.".into()));
            }
        }
    }
}
