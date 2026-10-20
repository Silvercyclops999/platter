//! `/health` for Docker's healthcheck, and a tiny status page, on 127.0.0.1
//! inside the container only (never the LAN). `/health` answers 200 while
//! the main loop keeps coming round, else 503. `/` is the status page:
//! what the Bridge is doing, its pair code while it isn't paired, the
//! plex.tv/link code while it waits to be let into Plex, and what
//! MUSIC_PLEX_PATH should be. `platter-bridge health` and `platter-bridge
//! status` ask a running one (the image has no curl).

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::run::Bridge;

/// The loop is stuck when it hasn't come round in this long.
const STUCK_S: i64 = 10 * 60;

/// The status as plain lines.
pub fn status_text(bridge: &Bridge) -> String {
    let st = bridge.status();
    let mut out = vec![format!("{} {}", st.name, st.version), format!("Now: {}", st.doing)];
    if let Some(code) = bridge.shown_code() {
        out.push(format!("Pair code (works once, for an hour): {code}"));
        out.push("In Platter on your computer or phone: Settings > Bridge > Add the Bridge.".into());
    }
    if let Some(code) = &st.plex_link {
        out.push(format!("Plex: open https://plex.tv/link and enter {code}"));
    }
    if let Some(p) = &st.music_plex_path {
        out.push(format!("Music folder: set MUSIC_PLEX_PATH={p}"));
    }
    if let Some(l) = &st.lyrics {
        out.push(super::lyrics::words(l));
    }
    if let Some(p) = &st.problem {
        out.push(format!("Problem: {p}"));
    }
    out.join("\n") + "\n"
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// What one request gets: (status line, content type, body).
pub fn answer(bridge: &Bridge, request: &str, now_s: i64) -> (&'static str, &'static str, String) {
    let ok = now_s - bridge.alive.load(std::sync::atomic::Ordering::SeqCst) < STUCK_S;
    let path = request.strip_prefix("GET ").and_then(|r| r.split_whitespace().next()).unwrap_or("");
    match path {
        "/health" => {
            let st = bridge.status();
            let body = serde_json::json!({ "ok": ok, "doing": st.doing, "version": st.version }).to_string();
            (if ok { "200 OK" } else { "503 Service Unavailable" }, "application/json", body)
        }
        "/status" => ("200 OK", "text/plain; charset=utf-8", status_text(bridge)),
        "/" => {
            let page = format!(
                "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Platter Bridge</title><pre style=\"white-space:pre-wrap;font:15px/1.5 ui-monospace,monospace;margin:16px\">{}</pre>",
                escape(&status_text(bridge))
            );
            ("200 OK", "text/html; charset=utf-8", page)
        }
        _ => ("404 Not Found", "application/json", "{}".to_string()),
    }
}

pub async fn serve(bridge: Arc<Bridge>, port: u16) -> std::io::Result<()> {
    let l = TcpListener::bind(("127.0.0.1", port)).await?;
    loop {
        let (mut sock, _) = l.accept().await?;
        let b = bridge.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf)).await.ok().and_then(Result::ok).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let (code, kind, text) = answer(&b, &req, crate::db::queries::now_s());
            let resp = format!(
                "HTTP/1.1 {code}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        });
    }
}

async fn ask(port: u16, path: &str) -> Option<Vec<u8>> {
    let ask = async {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes()).await.ok()?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.ok()?;
        Some(out)
    };
    tokio::time::timeout(Duration::from_secs(5), ask).await.ok().flatten()
}

/// Ask the running Bridge whether it's well.
pub async fn check(port: u16) -> bool {
    ask(port, "/health").await.is_some_and(|out| out.starts_with(b"HTTP/1.1 200"))
}

/// The running Bridge's status, as plain lines (`platter-bridge status`).
pub async fn status(port: u16) -> Option<String> {
    let out = ask(port, "/status").await?;
    let text = String::from_utf8_lossy(&out);
    text.split_once("\r\n\r\n").map(|(_, body)| body.to_string())
}
