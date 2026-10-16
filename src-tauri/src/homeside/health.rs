//! `/health` for Docker's healthcheck, on 127.0.0.1 inside the container
//! only. It answers 200 while the main loop keeps coming round, else 503.
//! `platter-bridge health` asks it and exits 0 or 1 (the image has no curl).

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::run::Bridge;

/// The loop is stuck when it hasn't come round in this long.
const STUCK_S: i64 = 10 * 60;

pub async fn serve(bridge: Arc<Bridge>, port: u16) -> std::io::Result<()> {
    let l = TcpListener::bind(("127.0.0.1", port)).await?;
    loop {
        let (mut sock, _) = l.accept().await?;
        let b = bridge.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf)).await.ok().and_then(Result::ok).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let now = crate::db::queries::now_s();
            let ok = now - b.alive.load(std::sync::atomic::Ordering::SeqCst) < STUCK_S;
            let (code, text) = if !req.starts_with("GET /health") {
                ("404 Not Found", "{}".to_string())
            } else {
                let st = b.status();
                let body = serde_json::json!({ "ok": ok, "doing": st.doing, "version": st.version }).to_string();
                (if ok { "200 OK" } else { "503 Service Unavailable" }, body)
            };
            let resp = format!("HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
            let _ = sock.write_all(resp.as_bytes()).await;
        });
    }
}

/// Ask the running Bridge whether it's well.
pub async fn check(port: u16) -> bool {
    let ask = async {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        s.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.ok()?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.ok()?;
        Some(out.starts_with(b"HTTP/1.1 200"))
    };
    tokio::time::timeout(Duration::from_secs(5), ask).await.ok().flatten().unwrap_or(false)
}
