//! One TLS connection to a Cast device: a reader task that answers the
//! heartbeat, matches replies to requests by `requestId` and keeps the
//! latest receiver and media status; a writer task; a heartbeat every 5 s.
//!
//! Cast devices present a self-signed certificate (Google's device
//! certificates aren't checked by senders on the LAN either), so the
//! certificate's identity isn't verified. The handshake signature still is.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};

use super::proto::{self, CastMessage, NS_CONNECTION, NS_HEARTBEAT, RECEIVER, SENDER};
use crate::error::{AppError, AppResult};

pub const HEARTBEAT: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct AnyCert(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// TLS settings for Cast devices.
pub fn tls_config() -> AppResult<Arc<rustls::ClientConfig>> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| AppError::Internal(format!("cast tls: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCert(provider)))
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

#[derive(Default)]
struct Shared {
    pending: Mutex<HashMap<i64, oneshot::Sender<Value>>>,
    /// Latest RECEIVER_STATUS `status`.
    receiver: Mutex<Option<Value>>,
    /// Latest MEDIA_STATUS `status[0]` (None: no media session).
    media: Mutex<Option<Value>>,
    /// The app's transport sent CLOSE (the app ended or was replaced).
    app_closed: AtomicBool,
    closed: AtomicBool,
    /// The transport id whose CLOSE counts as the app ending.
    transport: Mutex<Option<String>>,
}

/// A connection to one Cast device. Dropping it closes the socket.
pub struct CastConn {
    tx: mpsc::UnboundedSender<CastMessage>,
    shared: Arc<Shared>,
    next_id: AtomicI64,
    timeout: Duration,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for CastConn {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

fn io_err(e: impl std::fmt::Display) -> AppError {
    AppError::Network(format!("cast device: {e}"))
}

impl CastConn {
    /// TLS connect and open the virtual connection to the platform receiver.
    pub async fn connect(addr: SocketAddr, tls: Arc<rustls::ClientConfig>, timeout: Duration) -> AppResult<Self> {
        let tcp = tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr))
            .await
            .map_err(|_| io_err("timed out"))?
            .map_err(io_err)?;
        let _ = tcp.set_nodelay(true);
        let name = rustls::pki_types::ServerName::IpAddress(addr.ip().into());
        let stream = tokio::time::timeout(timeout, tokio_rustls::TlsConnector::from(tls).connect(name, tcp))
            .await
            .map_err(|_| io_err("tls timed out"))?
            .map_err(io_err)?;
        let (mut rd, mut wr) = tokio::io::split(stream);
        let shared: Arc<Shared> = Arc::default();
        let (tx, mut rx) = mpsc::unbounded_channel::<CastMessage>();

        let writer = {
            let shared = shared.clone();
            tokio::spawn(async move {
                while let Some(m) = rx.recv().await {
                    if wr.write_all(&m.frame()).await.is_err() || wr.flush().await.is_err() {
                        break;
                    }
                }
                shared.closed.store(true, Ordering::SeqCst);
            })
        };
        let reader = {
            let shared = shared.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                while let Ok(Some(m)) = proto::read_frame(&mut rd).await {
                    handle_incoming(&shared, &tx, m);
                }
                shared.closed.store(true, Ordering::SeqCst);
                shared.pending.lock().clear();
            })
        };
        let heartbeat = {
            let tx = tx.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(HEARTBEAT).await;
                    let ping = CastMessage::new(SENDER, RECEIVER, NS_HEARTBEAT, json!({"type": "PING"}).to_string());
                    if tx.send(ping).is_err() {
                        return;
                    }
                }
            })
        };
        let conn = Self {
            tx,
            shared,
            next_id: AtomicI64::new(1),
            timeout,
            tasks: vec![writer, reader, heartbeat],
        };
        conn.open_channel(RECEIVER)?;
        Ok(conn)
    }

    /// Virtual connection to a receiver or an app's transport.
    pub fn open_channel(&self, destination: &str) -> AppResult<()> {
        self.send(destination, NS_CONNECTION, json!({"type": "CONNECT", "origin": {}}))
    }

    /// Replies from this transport's CLOSE mean the app went away.
    pub fn watch_transport(&self, transport: &str) {
        *self.shared.transport.lock() = Some(transport.to_string());
        self.shared.app_closed.store(false, Ordering::SeqCst);
    }

    pub fn send(&self, destination: &str, namespace: &str, payload: Value) -> AppResult<()> {
        if self.is_closed() {
            return Err(io_err("connection closed"));
        }
        self.tx
            .send(CastMessage::new(SENDER, destination, namespace, payload.to_string()))
            .map_err(|_| io_err("connection closed"))
    }

    /// Sends with a fresh `requestId` and waits for the reply carrying it.
    pub async fn request(&self, destination: &str, namespace: &str, mut payload: Value) -> AppResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        payload["requestId"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.shared.pending.lock().insert(id, tx);
        if let Err(e) = self.send(destination, namespace, payload) {
            self.shared.pending.lock().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => Err(io_err("connection closed")),
            Err(_) => {
                self.shared.pending.lock().remove(&id);
                Err(io_err("no reply"))
            }
        }
    }

    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }
    pub fn app_closed(&self) -> bool {
        self.shared.app_closed.load(Ordering::SeqCst)
    }
    pub fn receiver_status(&self) -> Option<Value> {
        self.shared.receiver.lock().clone()
    }
    pub fn media_status(&self) -> Option<Value> {
        self.shared.media.lock().clone()
    }
}

fn handle_incoming(shared: &Shared, tx: &mpsc::UnboundedSender<CastMessage>, m: CastMessage) {
    let Ok(v) = serde_json::from_str::<Value>(&m.payload) else {
        return;
    };
    let ty = v.get("type").and_then(Value::as_str).unwrap_or_default();
    match (m.namespace.as_str(), ty) {
        (NS_HEARTBEAT, "PING") => {
            let _ = tx.send(CastMessage::new(SENDER, &m.source, NS_HEARTBEAT, json!({"type": "PONG"}).to_string()));
            return;
        }
        (NS_CONNECTION, "CLOSE") => {
            if shared.transport.lock().as_deref() == Some(m.source.as_str()) {
                shared.app_closed.store(true, Ordering::SeqCst);
            }
            return;
        }
        (_, "RECEIVER_STATUS") => {
            *shared.receiver.lock() = v.get("status").cloned();
        }
        (_, "MEDIA_STATUS") => {
            let first = v.get("status").and_then(Value::as_array).and_then(|a| a.first()).cloned();
            *shared.media.lock() = first;
        }
        _ => {}
    }
    if let Some(id) = v.get("requestId").and_then(Value::as_i64).filter(|id| *id != 0)
        && let Some(waiter) = shared.pending.lock().remove(&id)
    {
        let _ = waiter.send(v);
    }
}
