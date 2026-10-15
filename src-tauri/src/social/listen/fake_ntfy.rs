//! A small in-process ntfy server for tests (on this computer, never the
//! real ntfy.sh): `POST /<topic>`, and `GET /<t1>,<t2>/json[?since=<id>]`
//! as a chunked JSON stream with an `open` event first. It records every
//! request and everything published, can go down (dropping every
//! connection), and can answer 429 like a rate-limited ntfy.sh.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, watch};

#[derive(Debug, Clone)]
pub struct Stored {
    pub id: String,
    pub topic: String,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

struct St {
    msgs: Mutex<Vec<Stored>>,
    requests: Mutex<Vec<String>>,
    tx: broadcast::Sender<Stored>,
    down: AtomicBool,
    limited: AtomicBool,
    next: AtomicU64,
    kill: watch::Sender<u64>,
}

#[derive(Clone)]
pub struct FakeNtfy {
    pub url: String,
    st: Arc<St>,
}

impl FakeNtfy {
    pub async fn start() -> Self {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let st = Arc::new(St {
            msgs: Mutex::default(),
            requests: Mutex::default(),
            tx: broadcast::channel(1024).0,
            down: AtomicBool::new(false),
            limited: AtomicBool::new(false),
            next: AtomicU64::new(0),
            kill: watch::channel(0).0,
        });
        let s = st.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = l.accept().await {
                tokio::spawn(conn(s.clone(), sock));
            }
        });
        Self { url, st }
    }

    /// Everything published, in order.
    pub fn published(&self) -> Vec<Stored> {
        self.st.msgs.lock().clone()
    }

    /// "METHOD /path?query" of every request.
    pub fn requests(&self) -> Vec<String> {
        self.st.requests.lock().clone()
    }

    /// Down: every open stream is cut and new connections are dropped.
    pub fn set_down(&self, down: bool) {
        self.st.down.store(down, Ordering::SeqCst);
        if down {
            self.st.kill.send_modify(|k| *k += 1);
        }
    }

    pub fn set_limited(&self, on: bool) {
        self.st.limited.store(on, Ordering::SeqCst);
    }

    /// Publish as anyone could (an outsider, or a replayed copy).
    pub fn inject(&self, topic: &str, body: &str) {
        store(&self.st, topic, body, Vec::new());
    }
}

fn store(st: &St, topic: &str, body: &str, headers: Vec<(String, String)>) -> Stored {
    let id = format!("m{:011}", st.next.fetch_add(1, Ordering::SeqCst));
    let m = Stored { id, topic: topic.into(), body: body.into(), headers };
    st.msgs.lock().push(m.clone());
    let _ = st.tx.send(m.clone());
    m
}

fn line(m: &Stored) -> String {
    let v = serde_json::json!({"id": m.id, "time": 1_800_000_000u64, "event": "message", "topic": m.topic, "message": m.body});
    format!("{v}\n")
}

fn chunk(s: &str) -> Vec<u8> {
    format!("{:x}\r\n{s}\r\n", s.len()).into_bytes()
}

async fn conn(st: Arc<St>, mut sock: TcpStream) {
    if st.down.load(Ordering::SeqCst) {
        return;
    }
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
        if buf.len() > 64 * 1024 {
            return;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or_default().to_string();
    let mut parts = first.split(' ');
    let (method, target) = (parts.next().unwrap_or_default().to_string(), parts.next().unwrap_or_default().to_string());
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => return,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
        }
    }
    st.requests.lock().push(format!("{method} {target}"));
    if st.limited.load(Ordering::SeqCst) {
        let _ = sock.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    }
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    if method == "POST" {
        let topic = path.trim_start_matches('/');
        let m = store(&st, topic, &String::from_utf8_lossy(&body), headers);
        let reply = serde_json::json!({"id": m.id, "event": "message", "topic": m.topic}).to_string();
        let _ = sock
            .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes())
            .await;
        return;
    }
    let Some(list) = path.strip_prefix('/').and_then(|p| p.strip_suffix("/json")) else {
        let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    };
    let topics: Vec<String> = list.split(',').map(str::to_string).collect();
    let since = query.split('&').find_map(|kv| kv.strip_prefix("since=")).map(str::to_string);
    let mut rx = st.tx.subscribe();
    let mut kill = st.kill.subscribe();
    kill.borrow_and_update();
    if sock
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
        .await
        .is_err()
    {
        return;
    }
    let open = format!("{}\n", serde_json::json!({"id": "open0000000", "time": 1_800_000_000u64, "event": "open", "topic": list}));
    if sock.write_all(&chunk(&open)).await.is_err() {
        return;
    }
    if let Some(since) = since {
        let old: Vec<Stored> = {
            let msgs = st.msgs.lock();
            match msgs.iter().position(|m| m.id == since) {
                Some(i) => msgs[i + 1..].iter().filter(|m| topics.contains(&m.topic)).cloned().collect(),
                None => Vec::new(),
            }
        };
        for m in old {
            if sock.write_all(&chunk(&line(&m))).await.is_err() {
                return;
            }
        }
    }
    loop {
        tokio::select! {
            m = rx.recv() => match m {
                Ok(m) if topics.contains(&m.topic) => {
                    if sock.write_all(&chunk(&line(&m))).await.is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            },
            _ = kill.changed() => return,
        }
    }
}
