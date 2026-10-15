//! A pretend DLNA renderer on 127.0.0.1 for tests: a device description,
//! AVTransport, RenderingControl and ConnectionManager over HTTP. Never a
//! real device.

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use super::text;

#[derive(Debug, Clone)]
pub struct FakeState {
    pub transport: String,
    pub uri: String,
    pub meta: String,
    pub pos_ms: u64,
    pub duration_ms: u64,
    pub volume: i64,
    /// Every action called, in order (`SetAVTransportURI`, `Play`...).
    pub actions: Vec<String>,
    /// MIME types `SetAVTransportURI` refuses (fault 714).
    pub refuse: Vec<String>,
    /// What `GetProtocolInfo` lists.
    pub sinks: String,
    /// Seek targets received.
    pub seeks: Vec<String>,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            transport: "NO_MEDIA_PRESENT".into(),
            uri: String::new(),
            meta: String::new(),
            pos_ms: 0,
            duration_ms: 200_000,
            volume: 30,
            actions: Vec::new(),
            refuse: Vec::new(),
            sinks: "http-get:*:audio/mpeg:*,http-get:*:audio/x-flac:*".into(),
            seeks: Vec::new(),
        }
    }
}

pub struct FakeRenderer {
    pub addr: SocketAddr,
    pub state: Arc<Mutex<FakeState>>,
    task: tokio::task::JoinHandle<()>,
}

const AV: &str = "urn:schemas-upnp-org:service:AVTransport:1";
const RC: &str = "urn:schemas-upnp-org:service:RenderingControl:1";
const CM: &str = "urn:schemas-upnp-org:service:ConnectionManager:1";

fn description() -> String {
    format!(
        "<?xml version=\"1.0\"?><root xmlns=\"urn:schemas-upnp-org:device-1-0\"><specVersion><major>1</major></specVersion>\
<device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType><friendlyName>Den Receiver</friendlyName>\
<manufacturer>Maker</manufacturer><modelName>AV-100</modelName><UDN>uuid:1234-abcd</UDN><serviceList>\
<service><serviceType>{AV}</serviceType><serviceId>urn:upnp-org:serviceId:AVTransport</serviceId><controlURL>/av</controlURL></service>\
<service><serviceType>{RC}</serviceType><serviceId>urn:upnp-org:serviceId:RenderingControl</serviceId><controlURL>/rc</controlURL></service>\
<service><serviceType>{CM}</serviceType><serviceId>urn:upnp-org:serviceId:ConnectionManager</serviceId><controlURL>/cm</controlURL></service>\
</serviceList></device></root>"
    )
}

fn time(ms: u64) -> String {
    super::link::format_time(ms)
}

fn ok(action: &str, ty: &str, inner: &str) -> (u16, String) {
    (
        200,
        format!("<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:{action}Response xmlns:u=\"{ty}\">{inner}</u:{action}Response></s:Body></s:Envelope>"),
    )
}

fn fault(code: u32) -> (u16, String) {
    (
        500,
        format!("<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\"><errorCode>{code}</errorCode><errorDescription>Illegal MIME-type</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>"),
    )
}

fn handle(st: &Mutex<FakeState>, path: &str, soapaction: &str, body: &str) -> (u16, String) {
    if path == "/desc.xml" {
        return (200, description());
    }
    let action = soapaction.trim_matches('"').rsplit('#').next().unwrap_or("").to_string();
    let mut s = st.lock();
    s.actions.push(action.clone());
    match (path, action.as_str()) {
        ("/cm", "GetProtocolInfo") => ok(&action, CM, &format!("<Source></Source><Sink>{}</Sink>", s.sinks)),
        ("/av", "SetAVTransportURI") => {
            let uri = text(body, "CurrentURI").unwrap_or_default();
            let meta = text(body, "CurrentURIMetaData").unwrap_or_default();
            if s.refuse.iter().any(|m| meta.contains(&format!(":{m}:"))) {
                return fault(714);
            }
            s.uri = uri;
            s.meta = meta;
            s.transport = "STOPPED".into();
            s.pos_ms = 0;
            ok(&action, AV, "")
        }
        ("/av", "Play") => {
            s.transport = "PLAYING".into();
            ok(&action, AV, "")
        }
        ("/av", "Pause") => {
            s.transport = "PAUSED_PLAYBACK".into();
            ok(&action, AV, "")
        }
        ("/av", "Stop") => {
            s.transport = "STOPPED".into();
            ok(&action, AV, "")
        }
        ("/av", "Seek") => {
            let target = text(body, "Target").unwrap_or_default();
            s.pos_ms = super::link::parse_time(&target).unwrap_or(0);
            s.seeks.push(target);
            ok(&action, AV, "")
        }
        ("/av", "GetTransportInfo") => ok(
            &action,
            AV,
            &format!("<CurrentTransportState>{}</CurrentTransportState><CurrentTransportStatus>OK</CurrentTransportStatus><CurrentSpeed>1</CurrentSpeed>", s.transport),
        ),
        ("/av", "GetPositionInfo") => ok(
            &action,
            AV,
            &format!(
                "<Track>1</Track><TrackDuration>{}</TrackDuration><TrackURI>{}</TrackURI><RelTime>{}</RelTime><AbsTime>NOT_IMPLEMENTED</AbsTime>",
                time(s.duration_ms),
                crate::remote::xml::escape(&s.uri),
                time(s.pos_ms)
            ),
        ),
        ("/rc", "GetVolume") => ok(&action, RC, &format!("<CurrentVolume>{}</CurrentVolume>", s.volume)),
        ("/rc", "SetVolume") => {
            s.volume = text(body, "DesiredVolume").and_then(|v| v.parse().ok()).unwrap_or(s.volume);
            ok(&action, RC, "")
        }
        _ => fault(401),
    }
}

impl FakeRenderer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state: Arc<Mutex<FakeState>> = Arc::default();
        let st = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let st = st.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let (head_end, len) = loop {
                        let Ok(n) = sock.read(&mut chunk).await else { return };
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buf[..i]).to_string();
                            let len = head
                                .lines()
                                .filter_map(|l| l.split_once(':'))
                                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            break (i + 4, len);
                        }
                    };
                    while buf.len() < head_end + len {
                        let Ok(n) = sock.read(&mut chunk).await else { return };
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                    let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let soapaction = head
                        .lines()
                        .filter_map(|l| l.split_once(':'))
                        .find(|(k, _)| k.eq_ignore_ascii_case("soapaction"))
                        .map(|(_, v)| v.trim().to_string())
                        .unwrap_or_default();
                    let (status, out) = handle(&st, &path, &soapaction, &body);
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                        out.len()
                    );
                    let _ = sock.write_all(reply.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { addr, state, task }
    }

    pub fn location(&self) -> Url {
        Url::parse(&format!("http://{}/desc.xml", self.addr)).unwrap()
    }

    /// The song plays to its end and the renderer stops.
    pub fn finish_song(&self) {
        let mut s = self.state.lock();
        s.pos_ms = s.duration_ms;
        s.transport = "STOPPED".into();
    }

    /// Playing along.
    pub fn at(&self, ms: u64) {
        self.state.lock().pos_ms = ms;
    }

    /// The renderer is switched off (stops answering).
    pub fn vanish(&self) {
        self.task.abort();
    }

    /// The actions called, without the polling ones.
    pub fn actions(&self) -> Vec<String> {
        self.state
            .lock()
            .actions
            .iter()
            .filter(|a| !matches!(a.as_str(), "GetTransportInfo" | "GetPositionInfo"))
            .cloned()
            .collect()
    }
}

impl Drop for FakeRenderer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
