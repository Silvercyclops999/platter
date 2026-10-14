//! A fake Cast device for tests: TLS on localhost with a throwaway
//! self-signed certificate, the Cast framing, and just enough of the
//! receiver, media and Plex namespaces to play, report and be controlled.

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::proto::{self, CastMessage, NS_CONNECTION, NS_HEARTBEAT, NS_MEDIA, NS_PLEX, NS_RECEIVER, RECEIVER};
use super::{DEFAULT_APP, PLEX_APP};

#[derive(Default)]
pub struct FakeState {
    /// Running app: (app id, session id, transport id).
    pub app: Option<(String, String, String)>,
    pub player_state: String,
    pub current_time: f64,
    pub volume: f64,
    pub queue: Vec<String>,
    pub index: usize,
    pub repeat: String,
    pub media_session: i64,
    /// Every (namespace, payload) the fake received, except heartbeats.
    pub log: Vec<(String, Value)>,
    /// Plex's receiver isn't installed (LAUNCH fails).
    pub no_plex_app: bool,
    /// LOAD and QUEUE_LOAD are refused.
    pub refuse_load: bool,
    /// The last LOAD for Plex's receiver.
    pub plex_load: Option<Value>,
}

pub struct FakeCast {
    pub addr: SocketAddr,
    pub state: Arc<Mutex<FakeState>>,
    clients: Arc<Mutex<Vec<mpsc::UnboundedSender<CastMessage>>>>,
    conns: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeCast {
    fn drop(&mut self) {
        self.vanish();
    }
}

fn tls_acceptor() -> tokio_rustls::TlsAcceptor {
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["fake-cast".to_string()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .unwrap();
    tokio_rustls::TlsAcceptor::from(Arc::new(cfg))
}

fn receiver_status(st: &FakeState, request_id: i64) -> Value {
    let apps: Vec<Value> = st
        .app
        .iter()
        .map(|(id, s, t)| json!({"appId": id, "sessionId": s, "transportId": t, "displayName": "app"}))
        .collect();
    json!({"type": "RECEIVER_STATUS", "requestId": request_id,
           "status": {"applications": apps, "volume": {"level": st.volume, "muted": false}}})
}

fn media_status(st: &FakeState, request_id: i64) -> Value {
    if st.app.is_none() || st.queue.is_empty() {
        return json!({"type": "MEDIA_STATUS", "requestId": request_id, "status": []});
    }
    let key = st.queue.get(st.index).cloned().unwrap_or_default();
    let plex = st.app.as_ref().is_some_and(|a| a.0 == PLEX_APP);
    let media = if plex {
        json!({"contentId": format!("/library/metadata/{key}"), "duration": 200.0})
    } else {
        json!({"contentId": "http://stream", "duration": 200.0, "customData": {"ratingKey": key}})
    };
    let mut s = json!({"mediaSessionId": st.media_session, "playerState": st.player_state,
                       "currentTime": st.current_time, "media": media, "repeatMode": st.repeat});
    if st.player_state == "IDLE" {
        s["idleReason"] = json!("FINISHED");
    }
    json!({"type": "MEDIA_STATUS", "requestId": request_id, "status": [s]})
}

impl FakeCast {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = tls_acceptor();
        let state = Arc::new(Mutex::new(FakeState {
            volume: 0.4,
            repeat: "REPEAT_OFF".into(),
            ..Default::default()
        }));
        let clients: Arc<Mutex<Vec<mpsc::UnboundedSender<CastMessage>>>> = Arc::default();
        let conns: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::default();
        let task = {
            let state = state.clone();
            let clients = clients.clone();
            let conns = conns.clone();
            tokio::spawn(async move {
                while let Ok((tcp, _)) = listener.accept().await {
                    let Ok(tls) = acceptor.accept(tcp).await else { continue };
                    let (mut rd, mut wr) = tokio::io::split(tls);
                    let (tx, mut rx) = mpsc::unbounded_channel::<CastMessage>();
                    clients.lock().push(tx.clone());
                    let w = tokio::spawn(async move {
                        while let Some(m) = rx.recv().await {
                            if wr.write_all(&m.frame()).await.is_err() {
                                return;
                            }
                        }
                        let _ = wr.shutdown().await;
                    });
                    let state = state.clone();
                    let r = tokio::spawn(async move {
                        while let Ok(Some(m)) = proto::read_frame(&mut rd).await {
                            for reply in handle(&state, &m) {
                                let _ = tx.send(reply);
                            }
                        }
                    });
                    conns.lock().extend([w, r]);
                }
            })
        };
        Self { addr, state, clients, conns, task }
    }

    pub fn log(&self) -> Vec<(String, Value)> {
        self.state.lock().log.clone()
    }

    /// Types of the messages sent on a namespace, in order.
    pub fn types(&self, ns: &str) -> Vec<String> {
        self.log()
            .into_iter()
            .filter(|(n, _)| n == ns)
            .filter_map(|(_, v)| v["type"].as_str().map(str::to_string))
            .collect()
    }

    /// Someone else stops the app on the device.
    pub fn stop_elsewhere(&self) {
        let msg = {
            let mut st = self.state.lock();
            st.app = None;
            receiver_status(&st, 0)
        };
        self.broadcast(RECEIVER, NS_RECEIVER, msg);
    }

    /// The device drops off the network.
    pub fn vanish(&self) {
        self.task.abort();
        self.clients.lock().clear();
        for c in self.conns.lock().drain(..) {
            c.abort();
        }
    }

    fn broadcast(&self, from: &str, ns: &str, v: Value) {
        for c in self.clients.lock().iter() {
            let _ = c.send(CastMessage::new(from, "*", ns, v.to_string()));
        }
    }
}

fn handle(state: &Mutex<FakeState>, m: &CastMessage) -> Vec<CastMessage> {
    let Ok(v) = serde_json::from_str::<Value>(&m.payload) else { return vec![] };
    let ty = v["type"].as_str().unwrap_or_default().to_string();
    let rid = v["requestId"].as_i64().unwrap_or(0);
    let reply = |ns: &str, body: Value| CastMessage::new(&m.destination, &m.source, ns, body.to_string());
    let mut st = state.lock();
    if m.namespace != NS_HEARTBEAT {
        st.log.push((m.namespace.clone(), v.clone()));
    }
    match (m.namespace.as_str(), ty.as_str()) {
        (NS_HEARTBEAT, "PING") => vec![reply(NS_HEARTBEAT, json!({"type": "PONG"}))],
        (NS_CONNECTION, _) => vec![],
        (NS_RECEIVER, "GET_STATUS") => vec![reply(NS_RECEIVER, receiver_status(&st, rid))],
        (NS_RECEIVER, "LAUNCH") => {
            let app = v["appId"].as_str().unwrap_or_default().to_string();
            if (app == PLEX_APP && st.no_plex_app) || (app != PLEX_APP && app != DEFAULT_APP) {
                return vec![reply(NS_RECEIVER, json!({"type": "LAUNCH_ERROR", "requestId": rid, "reason": "NOT_FOUND"}))];
            }
            st.app = Some((app.clone(), format!("session-{app}"), format!("transport-{app}")));
            st.queue.clear();
            vec![reply(NS_RECEIVER, receiver_status(&st, rid))]
        }
        (NS_RECEIVER, "STOP") => {
            st.app = None;
            vec![reply(NS_RECEIVER, receiver_status(&st, rid))]
        }
        (NS_RECEIVER, "SET_VOLUME") => {
            st.volume = v["volume"]["level"].as_f64().unwrap_or(st.volume);
            vec![reply(NS_RECEIVER, receiver_status(&st, rid))]
        }
        (NS_MEDIA, "LOAD") | (NS_MEDIA, "QUEUE_LOAD") => {
            if st.refuse_load {
                return vec![reply(NS_MEDIA, json!({"type": "LOAD_FAILED", "requestId": rid}))];
            }
            if ty == "LOAD" {
                let key = v["media"]["contentId"].as_str().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
                st.plex_load = Some(v.clone());
                st.queue = vec![key, "next-in-server-queue".into()];
                st.index = 0;
            } else {
                st.queue = v["items"]
                    .as_array()
                    .map(|a| a.iter().map(|i| i["media"]["customData"]["ratingKey"].as_str().unwrap_or("").to_string()).collect())
                    .unwrap_or_default();
                st.index = v["startIndex"].as_u64().unwrap_or(0) as usize;
                st.repeat = v["repeatMode"].as_str().unwrap_or("REPEAT_OFF").to_string();
            }
            st.current_time = v["currentTime"].as_f64().unwrap_or(0.0);
            st.player_state = "PLAYING".into();
            st.media_session += 1;
            vec![reply(NS_MEDIA, media_status(&st, rid))]
        }
        (NS_MEDIA, "GET_STATUS") => vec![reply(NS_MEDIA, media_status(&st, rid))],
        (NS_MEDIA | NS_PLEX, "PLAY") => {
            st.player_state = "PLAYING".into();
            vec![reply(NS_MEDIA, media_status(&st, 0))]
        }
        (NS_MEDIA | NS_PLEX, "PAUSE") => {
            st.player_state = "PAUSED".into();
            vec![reply(NS_MEDIA, media_status(&st, 0))]
        }
        (NS_MEDIA | NS_PLEX, "SEEK") => {
            st.current_time = v["currentTime"].as_f64().unwrap_or(0.0);
            vec![reply(NS_MEDIA, media_status(&st, 0))]
        }
        (NS_PLEX, "NEXT") => {
            st.index = (st.index + 1).min(st.queue.len().saturating_sub(1));
            st.current_time = 0.0;
            vec![reply(NS_MEDIA, media_status(&st, 0))]
        }
        (NS_PLEX, "PREVIOUS") => {
            st.index = st.index.saturating_sub(1);
            st.current_time = 0.0;
            vec![reply(NS_MEDIA, media_status(&st, 0))]
        }
        (NS_MEDIA, "QUEUE_UPDATE") => {
            if let Some(j) = v["jump"].as_i64() {
                st.index = (st.index as i64 + j).clamp(0, st.queue.len() as i64 - 1) as usize;
                st.current_time = 0.0;
            }
            if let Some(r) = v["repeatMode"].as_str() {
                st.repeat = r.to_string();
            }
            vec![reply(NS_MEDIA, media_status(&st, 0))]
        }
        _ => vec![],
    }
}
