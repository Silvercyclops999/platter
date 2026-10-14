//! The local Discord client's IPC socket: a named pipe on Windows
//! (`\\.\pipe\discord-ipc-N`) and a unix socket on macOS (`discord-ipc-N` in
//! the temp folder), N from 0 to 9.
//!
//! Each frame is an opcode and a length (both u32, little-endian) followed
//! by that many bytes of JSON. The client opens with a handshake
//! (`{"v":1,"client_id":…}`), Discord answers with a READY event, and then
//! each command is a frame with a nonce that Discord echoes in its reply.
//! The transport sits behind [`Connector`] so tests use an in-memory pipe.

use std::fmt;
use std::io;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_PING: u32 = 3;
pub const OP_PONG: u32 = 4;

/// Discord's replies are small; anything bigger is a broken stream.
const MAX_FRAME: u32 = 1 << 20;

/// A connected socket.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Opens a socket to the Discord client.
#[async_trait]
pub trait Connector: Send + Sync {
    async fn connect(&self) -> io::Result<Box<dyn Stream>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcError {
    /// No Discord client to talk to, or the socket broke.
    Io(String),
    /// Discord closed the connection (with its reason).
    Closed(String),
    /// Discord sent something that isn't a frame.
    Protocol(String),
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "discord socket: {e}"),
            Self::Closed(e) => write!(f, "discord closed the connection: {e}"),
            Self::Protocol(e) => write!(f, "discord protocol: {e}"),
        }
    }
}

impl From<io::Error> for IpcError {
    fn from(e: io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin + ?Sized>(w: &mut W, op: u32, body: &Value) -> Result<(), IpcError> {
    let bytes = serde_json::to_vec(body).map_err(|e| IpcError::Protocol(e.to_string()))?;
    let mut buf = Vec::with_capacity(8 + bytes.len());
    buf.extend_from_slice(&op.to_le_bytes());
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(&bytes);
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin + ?Sized>(r: &mut R) -> Result<(u32, Value), IpcError> {
    let mut head = [0u8; 8];
    r.read_exact(&mut head).await?;
    let op = u32::from_le_bytes(head[..4].try_into().unwrap_or_default());
    let len = u32::from_le_bytes(head[4..].try_into().unwrap_or_default());
    if len > MAX_FRAME {
        return Err(IpcError::Protocol(format!("frame of {len} bytes")));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    let v = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).map_err(|e| IpcError::Protocol(e.to_string()))?
    };
    Ok((op, v))
}

/// How Discord answered a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Ok,
    /// An ERROR event: the code and message Discord gave.
    Refused { code: i64, message: String },
}

fn close_reason(v: &Value) -> String {
    let code = v.get("code").and_then(Value::as_i64).unwrap_or_default();
    let msg = v.get("message").and_then(Value::as_str).unwrap_or("no reason");
    format!("{code} {msg}")
}

/// One open connection after the handshake.
pub struct IpcClient {
    stream: Box<dyn Stream>,
    nonce: u64,
}

impl IpcClient {
    /// Connect and handshake. Ends with Discord's READY.
    pub async fn open(connector: &dyn Connector, client_id: &str) -> Result<Self, IpcError> {
        let stream = connector.connect().await?;
        let mut c = Self { stream, nonce: 0 };
        write_frame(&mut c.stream, OP_HANDSHAKE, &json!({ "v": 1, "client_id": client_id })).await?;
        loop {
            match c.next().await? {
                (OP_FRAME, v) if v.get("evt").and_then(Value::as_str) == Some("READY") => return Ok(c),
                (OP_FRAME, v) if v.get("evt").and_then(Value::as_str) == Some("ERROR") => {
                    return Err(IpcError::Closed(close_reason(v.get("data").unwrap_or(&Value::Null))));
                }
                _ => {}
            }
        }
    }

    /// The next frame that isn't a ping (pings are answered here).
    async fn next(&mut self) -> Result<(u32, Value), IpcError> {
        loop {
            let (op, v) = read_frame(&mut self.stream).await?;
            match op {
                OP_PING => write_frame(&mut self.stream, OP_PONG, &v).await?,
                OP_CLOSE => return Err(IpcError::Closed(close_reason(&v))),
                OP_PONG => {}
                _ => return Ok((op, v)),
            }
        }
    }

    /// Set (or with None, clear) this app's activity.
    pub async fn set_activity(&mut self, pid: u32, activity: Option<&Value>) -> Result<Reply, IpcError> {
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        let mut args = json!({ "pid": pid });
        if let Some(a) = activity {
            args["activity"] = a.clone();
        }
        write_frame(&mut self.stream, OP_FRAME, &json!({ "cmd": "SET_ACTIVITY", "args": args, "nonce": nonce })).await?;
        loop {
            let (_, v) = self.next().await?;
            if v.get("nonce").and_then(Value::as_str) != Some(nonce.as_str()) {
                continue;
            }
            if v.get("evt").and_then(Value::as_str) == Some("ERROR") {
                let d = v.get("data").cloned().unwrap_or_default();
                return Ok(Reply::Refused {
                    code: d.get("code").and_then(Value::as_i64).unwrap_or_default(),
                    message: d.get("message").and_then(Value::as_str).unwrap_or_default().to_owned(),
                });
            }
            return Ok(Reply::Ok);
        }
    }
}

/// The Discord desktop client on this computer.
pub struct LocalDiscord;

#[cfg(windows)]
#[async_trait]
impl Connector for LocalDiscord {
    async fn connect(&self) -> io::Result<Box<dyn Stream>> {
        use tokio::net::windows::named_pipe::ClientOptions;
        let mut last = io::Error::new(io::ErrorKind::NotFound, "Discord isn't running");
        for n in 0..10 {
            match ClientOptions::new().open(format!(r"\\.\pipe\discord-ipc-{n}")) {
                Ok(p) => return Ok(Box::new(p)),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

#[cfg(unix)]
#[async_trait]
impl Connector for LocalDiscord {
    async fn connect(&self) -> io::Result<Box<dyn Stream>> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "Discord isn't running");
        for dir in socket_dirs() {
            for n in 0..10 {
                match tokio::net::UnixStream::connect(dir.join(format!("discord-ipc-{n}"))).await {
                    Ok(s) => return Ok(Box::new(s)),
                    Err(e) => last = e,
                }
            }
        }
        Err(last)
    }
}

/// Where Discord puts its socket on macOS and Linux, in the order it checks.
#[cfg(unix)]
fn socket_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .filter_map(|k| std::env::var_os(k))
        .filter(|v| !v.is_empty())
        .map(Into::into)
        .collect();
    dirs.push("/tmp".into());
    dirs
}

#[cfg(test)]
pub mod fake {
    //! An in-memory Discord: answers the handshake and every command, logs
    //! what it received, and can refuse activity types or hang up.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use parking_lot::Mutex;
    use tokio::io::DuplexStream;

    use super::*;

    #[derive(Default)]
    pub struct FakeDiscord {
        /// Discord isn't running: connects fail.
        pub down: AtomicBool,
        /// Refuse activities of type 2 (as a client without Listening would).
        pub refuse_listening: AtomicBool,
        /// Hang up after this many more commands (0 = never).
        pub hang_up_after: AtomicUsize,
        pub connects: AtomicUsize,
        /// Every frame body received, in order (handshakes included).
        pub frames: Mutex<Vec<Value>>,
    }

    impl FakeDiscord {
        pub fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        /// The activity of each SET_ACTIVITY, None for a clear.
        pub fn activities(&self) -> Vec<Option<Value>> {
            self.frames
                .lock()
                .iter()
                .filter(|f| f.get("cmd").and_then(Value::as_str) == Some("SET_ACTIVITY"))
                .map(|f| f["args"].get("activity").cloned())
                .collect()
        }

        /// Everything sent to Discord, as one string.
        pub fn wire_text(&self) -> String {
            self.frames.lock().iter().map(Value::to_string).collect::<Vec<_>>().join("\n")
        }

        async fn serve(self: Arc<Self>, mut s: DuplexStream) {
            loop {
                let Ok((op, v)) = read_frame(&mut s).await else { return };
                self.frames.lock().push(v.clone());
                if op == OP_HANDSHAKE {
                    let ready = json!({ "cmd": "DISPATCH", "evt": "READY", "data": { "v": 1 } });
                    if write_frame(&mut s, OP_FRAME, &ready).await.is_err() {
                        return;
                    }
                    continue;
                }
                let nonce = v.get("nonce").cloned().unwrap_or_default();
                let ty = v["args"]["activity"].get("type").and_then(Value::as_i64);
                let reply = if ty == Some(2) && self.refuse_listening.load(Ordering::SeqCst) {
                    json!({ "cmd": "SET_ACTIVITY", "evt": "ERROR", "nonce": nonce,
                            "data": { "code": 4000, "message": "child \"activity\" fails because [\"type\" must be one of [0]]" } })
                } else {
                    json!({ "cmd": "SET_ACTIVITY", "evt": null, "nonce": nonce, "data": v["args"]["activity"] })
                };
                if write_frame(&mut s, OP_FRAME, &reply).await.is_err() {
                    return;
                }
                let left = self.hang_up_after.load(Ordering::SeqCst);
                if left == 1 {
                    self.hang_up_after.store(0, Ordering::SeqCst);
                    self.down.store(true, Ordering::SeqCst);
                    return; // drops the pipe, like Discord quitting
                } else if left > 1 {
                    self.hang_up_after.store(left - 1, Ordering::SeqCst);
                }
            }
        }
    }

    pub struct FakeConnector(pub Arc<FakeDiscord>);

    #[async_trait]
    impl Connector for FakeConnector {
        async fn connect(&self) -> io::Result<Box<dyn Stream>> {
            if self.0.down.load(Ordering::SeqCst) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "pipe not found"));
            }
            self.0.connects.fetch_add(1, Ordering::SeqCst);
            let (client, server) = tokio::io::duplex(64 * 1024);
            tokio::spawn(self.0.clone().serve(server));
            Ok(Box::new(client))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    #[tokio::test]
    async fn frames_are_opcode_length_and_json_little_endian() {
        let mut buf = Vec::new();
        write_frame(&mut buf, OP_FRAME, &json!({ "a": 1 })).await.unwrap();
        assert_eq!(&buf[..4], &[1, 0, 0, 0]);
        assert_eq!(&buf[4..8], &[7, 0, 0, 0]);
        assert_eq!(&buf[8..], br#"{"a":1}"#);
        let (op, v) = read_frame(&mut buf.as_slice()).await.unwrap();
        assert_eq!((op, v), (OP_FRAME, json!({ "a": 1 })));
    }

    #[tokio::test]
    async fn an_oversized_frame_is_refused_not_allocated() {
        let mut buf = vec![1, 0, 0, 0];
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(read_frame(&mut buf.as_slice()).await, Err(IpcError::Protocol(_))));
    }

    #[tokio::test]
    async fn handshake_then_set_and_clear() {
        let d = FakeDiscord::new();
        let mut c = IpcClient::open(&FakeConnector(d.clone()), "123").await.unwrap();
        assert_eq!(c.set_activity(42, Some(&json!({ "type": 2, "details": "Song" }))).await.unwrap(), Reply::Ok);
        assert_eq!(c.set_activity(42, None).await.unwrap(), Reply::Ok);
        let frames = d.frames.lock().clone();
        assert_eq!(frames[0], json!({ "v": 1, "client_id": "123" }));
        assert_eq!(frames[1]["cmd"], "SET_ACTIVITY");
        assert_eq!(frames[1]["args"]["pid"], 42);
        assert!(frames[2]["args"].get("activity").is_none(), "a clear sends no activity");
    }

    #[tokio::test]
    async fn a_refused_command_reports_its_code() {
        let d = FakeDiscord::new();
        d.refuse_listening.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut c = IpcClient::open(&FakeConnector(d), "1").await.unwrap();
        let r = c.set_activity(1, Some(&json!({ "type": 2 }))).await.unwrap();
        assert!(matches!(r, Reply::Refused { code: 4000, .. }));
    }

    #[tokio::test]
    async fn pings_are_answered_and_close_is_an_error() {
        let (client, mut server) = tokio::io::duplex(4096);
        struct One(parking_lot::Mutex<Option<tokio::io::DuplexStream>>);
        #[async_trait]
        impl Connector for One {
            async fn connect(&self) -> io::Result<Box<dyn Stream>> {
                Ok(Box::new(self.0.lock().take().unwrap()))
            }
        }
        let srv = tokio::spawn(async move {
            read_frame(&mut server).await.unwrap();
            write_frame(&mut server, OP_PING, &json!({ "p": 1 })).await.unwrap();
            write_frame(&mut server, OP_FRAME, &json!({ "evt": "READY" })).await.unwrap();
            let (op, v) = read_frame(&mut server).await.unwrap();
            assert_eq!((op, v), (OP_PONG, json!({ "p": 1 })));
            read_frame(&mut server).await.unwrap();
            write_frame(&mut server, OP_CLOSE, &json!({ "code": 4000, "message": "bye" })).await.unwrap();
        });
        let mut c = IpcClient::open(&One(parking_lot::Mutex::new(Some(client))), "1").await.unwrap();
        let r = c.set_activity(1, None).await;
        assert_eq!(r, Err(IpcError::Closed("4000 bye".into())));
        srv.await.unwrap();
    }
}
