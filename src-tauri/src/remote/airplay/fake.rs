//! A fake AirPlay receiver for tests, on localhost: RTSP over TCP, audio,
//! control and timing over UDP. It records what it was sent.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use super::rtsp::digest_response;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
pub struct Recorded {
    pub requests: Vec<Request>,
    /// Audio packets as received.
    pub audio: Vec<Vec<u8>>,
    /// Sync packets from the control port.
    pub syncs: Vec<Vec<u8>>,
    /// The sender's timing port, from SETUP.
    pub client_timing: Option<u16>,
}

pub struct FakeRaop {
    pub addr: SocketAddr,
    pub rec: Arc<Mutex<Recorded>>,
    stop: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
pub struct FakeOptions {
    pub password: Option<String>,
    /// Answer 403 like a receiver that only takes paired senders.
    pub pairing: bool,
}

impl Drop for FakeRaop {
    fn drop(&mut self) {
        self.vanish();
    }
}

fn read_request(r: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    parts.next()?;
    let mut headers = Vec::new();
    loop {
        let mut l = String::new();
        if r.read_line(&mut l).ok()? == 0 {
            return None;
        }
        let l = l.trim_end().to_string();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).ok()?;
    Some(Request { method, headers, body: String::from_utf8_lossy(&body).into_owned() })
}

fn authorized(req: &Request, password: &str) -> bool {
    let Some(h) = req.header("authorization") else { return false };
    let get = |k: &str| {
        let at = h.find(&format!("{k}=\""))? + k.len() + 2;
        Some(h[at..].split('"').next()?.to_string())
    };
    let (Some(user), Some(realm), Some(nonce), Some(uri), Some(resp)) = (get("username"), get("realm"), get("nonce"), get("uri"), get("response")) else {
        return false;
    };
    digest_response(&user, &realm, password, &req.method, &uri, &nonce) == resp
}

impl FakeRaop {
    pub fn start(opts: FakeOptions) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let rec: Arc<Mutex<Recorded>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (rec, stop) = (rec.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((s, _)) => {
                            let (rec, stop, opts) = (rec.clone(), stop.clone(), opts.clone());
                            std::thread::spawn(move || serve(s, rec, stop, opts));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            });
        }
        Self { addr, rec, stop }
    }

    pub fn methods(&self) -> Vec<String> {
        self.rec.lock().requests.iter().map(|r| r.method.clone()).collect()
    }

    pub fn last(&self, method: &str) -> Option<Request> {
        self.rec.lock().requests.iter().rev().find(|r| r.method == method).cloned()
    }

    /// The receiver drops off the network.
    pub fn vanish(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn udp() -> UdpSocket {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    s.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    s
}

fn serve(stream: TcpStream, rec: Arc<Mutex<Recorded>>, stop: Arc<AtomicBool>, opts: FakeOptions) {
    stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    let mut w = stream.try_clone().unwrap();
    let mut r = BufReader::new(stream);
    while !stop.load(Ordering::SeqCst) {
        let Some(req) = read_request(&mut r) else {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            // Read timeout with nothing pending: keep waiting.
            if r.buffer().is_empty() {
                continue;
            }
            return;
        };
        rec.lock().requests.push(req.clone());
        let cseq = req.header("cseq").unwrap_or("0").to_string();
        let mut extra = String::new();
        let status = if opts.pairing {
            "403 Forbidden"
        } else if opts.password.as_deref().is_some_and(|p| !authorized(&req, p)) {
            extra.push_str("WWW-Authenticate: Digest realm=\"raop\", nonce=\"fake-nonce\"\r\n");
            "401 Unauthorized"
        } else {
            match req.method.as_str() {
                "SETUP" => {
                    let (audio, control, timing) = (udp(), udp(), udp());
                    let port = |s: &UdpSocket| s.local_addr().unwrap().port();
                    let t = req.header("transport").unwrap_or_default();
                    let client_timing = t.split(';').find_map(|p| p.strip_prefix("timing_port=")?.parse().ok());
                    rec.lock().client_timing = client_timing;
                    extra.push_str(&format!(
                        "Session: 1\r\nTransport: RTP/AVP/UDP;unicast;mode=record;server_port={};control_port={};timing_port={}\r\n",
                        port(&audio),
                        port(&control),
                        port(&timing)
                    ));
                    for (sock, is_audio) in [(audio, true), (control, false)] {
                        let (rec, stop) = (rec.clone(), stop.clone());
                        std::thread::spawn(move || {
                            let mut buf = [0u8; 2048];
                            while !stop.load(Ordering::SeqCst) {
                                if let Ok(n) = sock.recv(&mut buf) {
                                    let p = buf[..n].to_vec();
                                    if is_audio {
                                        rec.lock().audio.push(p);
                                    } else {
                                        rec.lock().syncs.push(p);
                                    }
                                }
                            }
                        });
                    }
                    drop(timing);
                    "200 OK"
                }
                "RECORD" => {
                    extra.push_str("Audio-Latency: 11025\r\n");
                    "200 OK"
                }
                _ => "200 OK",
            }
        };
        let resp = format!("RTSP/1.0 {status}\r\nCSeq: {cseq}\r\n{extra}Content-Length: 0\r\n\r\n");
        if w.write_all(resp.as_bytes()).is_err() {
            return;
        }
    }
}
