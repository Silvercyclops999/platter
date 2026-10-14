//! An AirPlay audio (RAOP) session: RTSP setup, then Apple Lossless frames
//! over RTP/UDP, sync packets on the control port once a second, answers to
//! the receiver's timing requests, and resends of packets it asks for.
//!
//! Only unencrypted streams are sent. Receivers that insist on RSA, FairPlay
//! or AirPlay 2 pairing are listed as not supported (see `mod.rs`).

use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use super::alac;
use super::rtsp::Rtsp;
use crate::audio::netsink::NetStream;
use crate::error::{AppError, AppResult};

pub const SAMPLE_RATE: u32 = 44_100;
/// The playback delay this sender asks for (2 s, the classic RAOP value).
pub const LATENCY_FRAMES: u32 = 88_200;
const RESEND_KEEP: usize = 512;

/// RAOP volume in dB: -30 (quietest) to 0, and -144 for mute.
pub fn volume_db(v: f32) -> f32 {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    if v <= 0.0 { -144.0 } else { -30.0 + 30.0 * v }
}

/// NTP time now (seconds since 1900 in the top 32 bits).
pub fn ntp_now() -> u64 {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() + 2_208_988_800;
    let frac = (u64::from(d.subsec_nanos()) << 32) / 1_000_000_000;
    (secs << 32) | frac
}

fn transport_port(transport: &str, key: &str) -> Option<u16> {
    transport.split(';').find_map(|p| p.trim().strip_prefix(key)?.strip_prefix('=')?.parse().ok())
}

/// State shared by the stream (sending audio) and the controls (flush,
/// volume, teardown).
pub struct RaopShared {
    rtsp: Mutex<Rtsp>,
    uri: String,
    seq: AtomicU16,
    rtptime: AtomicU32,
    /// The next packet starts a new run (after setup or a flush).
    first: AtomicBool,
    closed: AtomicBool,
    /// Extra delay the receiver reported (frames), on top of ours.
    pub receiver_latency: u32,
    stop: Arc<AtomicBool>,
}

impl RaopShared {
    /// Total delay between sending and hearing.
    pub fn delay(&self) -> Duration {
        Duration::from_secs_f64(f64::from(LATENCY_FRAMES + self.receiver_latency) / f64::from(SAMPLE_RATE))
    }

    /// Drops what the receiver has buffered (pause, seek, skip).
    pub fn flush(&self) -> AppResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(AppError::Network("AirPlay session ended".into()));
        }
        let info = format!(
            "seq={};rtptime={}",
            self.seq.load(Ordering::SeqCst),
            self.rtptime.load(Ordering::SeqCst)
        );
        self.rtsp.lock().request("FLUSH", &self.uri, &[("RTP-Info", info)], None)?;
        self.first.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Receiver volume, 0..1.
    pub fn set_volume(&self, v: f32) -> AppResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(AppError::Network("AirPlay session ended".into()));
        }
        let body = format!("volume: {:.6}\r\n", volume_db(v));
        self.rtsp
            .lock()
            .request("SET_PARAMETER", &self.uri, &[], Some(("text/parameters", body.as_bytes())))?;
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn teardown(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.rtsp.lock().request("TEARDOWN", &self.uri, &[], None);
    }
}

/// Audio sending side of a session.
pub struct RaopStream {
    shared: Arc<RaopShared>,
    audio: UdpSocket,
    control: Arc<UdpSocket>,
    server_control: SocketAddr,
    ssrc: u32,
    sent_since_sync: u32,
    resend: Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>,
    threads: Vec<JoinHandle<()>>,
}

/// Connection settings for one receiver.
#[derive(Clone)]
pub struct RaopConfig {
    pub addr: SocketAddr,
    pub password: Option<String>,
    pub timeout: Duration,
    /// Sender id (16 hex digits).
    pub client_id: String,
    /// Receiver volume to start at, 0..1.
    pub volume: f32,
}

fn bind_udp(ip: std::net::IpAddr) -> AppResult<UdpSocket> {
    let s = UdpSocket::bind(SocketAddr::new(ip, 0)).map_err(|e| AppError::Network(format!("udp: {e}")))?;
    s.set_read_timeout(Some(Duration::from_millis(200))).ok();
    Ok(s)
}

fn sdp(local: &std::net::IpAddr, remote: &std::net::IpAddr, session: u32) -> String {
    let fam = |ip: &std::net::IpAddr| if ip.is_ipv4() { "IP4" } else { "IP6" };
    format!(
        "v=0\r\no=Platter {session} 0 IN {} {local}\r\ns=Platter\r\nc=IN {} {remote}\r\nt=0 0\r\n\
         m=audio 0 RTP/AVP 96\r\na=rtpmap:96 AppleLossless\r\n\
         a=fmtp:96 {} 0 16 40 10 14 2 255 0 0 {SAMPLE_RATE}\r\n",
        fam(local),
        fam(remote),
        alac::FRAMES,
    )
}

/// Answers timing requests: the receiver's clock sync.
fn timing_loop(sock: UdpSocket, stop: Arc<AtomicBool>) {
    let mut buf = [0u8; 128];
    while !stop.load(Ordering::SeqCst) {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        if n < 32 || buf[1] & 0x7f != 0x52 {
            continue;
        }
        let now = ntp_now().to_be_bytes();
        let mut r = [0u8; 32];
        r[0] = 0x80;
        r[1] = 0xd3;
        r[2..4].copy_from_slice(&7u16.to_be_bytes());
        r[8..16].copy_from_slice(&buf[24..32]);
        r[16..24].copy_from_slice(&now);
        r[24..32].copy_from_slice(&now);
        let _ = sock.send_to(&r, from);
    }
}

/// Resends packets the receiver missed.
fn control_loop(sock: Arc<UdpSocket>, to: SocketAddr, resend: Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>, stop: Arc<AtomicBool>) {
    let mut buf = [0u8; 128];
    while !stop.load(Ordering::SeqCst) {
        let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
        if n < 8 || buf[1] & 0x7f != 0x55 {
            continue;
        }
        let first = u16::from_be_bytes([buf[4], buf[5]]);
        let count = u16::from_be_bytes([buf[6], buf[7]]).min(64);
        let packets: Vec<Vec<u8>> = {
            let r = resend.lock();
            (0..count)
                .filter_map(|i| {
                    let seq = first.wrapping_add(i);
                    r.iter().find(|(s, _)| *s == seq).map(|(_, p)| p.clone())
                })
                .collect()
        };
        for p in packets {
            let mut out = Vec::with_capacity(p.len() + 4);
            out.extend_from_slice(&[0x80, 0xd6, 0x00, 0x01]);
            out.extend_from_slice(&p);
            let _ = sock.send_to(&out, to);
        }
    }
}

impl RaopStream {
    /// Sets up the session and starts the timing and control threads.
    pub fn open(cfg: &RaopConfig) -> AppResult<(Self, Arc<RaopShared>)> {
        let mut rtsp = Rtsp::connect(cfg.addr, cfg.timeout, cfg.password.clone(), cfg.client_id.clone())?;
        let session_num: u32 = rand::random();
        let local = rtsp.local_ip;
        let uri = format!("rtsp://{local}/{session_num}");
        rtsp.request("OPTIONS", "*", &[], None)?;
        let body = sdp(&local, &cfg.addr.ip(), session_num);
        rtsp.request("ANNOUNCE", &uri, &[], Some(("application/sdp", body.as_bytes())))?;

        let audio = bind_udp(local)?;
        let control = Arc::new(bind_udp(local)?);
        let timing = bind_udp(local)?;
        let port = |s: &UdpSocket| s.local_addr().map(|a| a.port()).unwrap_or(0);
        let transport = format!(
            "RTP/AVP/UDP;unicast;interleaved=0-1;mode=record;control_port={};timing_port={}",
            port(&control),
            port(&timing)
        );
        let r = rtsp.request("SETUP", &uri, &[("Transport", transport)], None)?;
        rtsp.session = r.header("session").map(|s| s.split(';').next().unwrap_or(s).trim().to_string());
        let t = r.header("transport").unwrap_or_default();
        let server_port = transport_port(t, "server_port").ok_or_else(|| AppError::Network("AirPlay receiver gave no audio port".into()))?;
        let server_control = transport_port(t, "control_port").unwrap_or(server_port);
        audio
            .connect(SocketAddr::new(cfg.addr.ip(), server_port))
            .map_err(|e| AppError::Network(format!("udp: {e}")))?;

        let seq: u16 = rand::random();
        let rtptime: u32 = rand::random();
        let r = rtsp.request(
            "RECORD",
            &uri,
            &[("Range", "npt=0-".into()), ("RTP-Info", format!("seq={seq};rtptime={rtptime}"))],
            None,
        )?;
        let receiver_latency = r.header("audio-latency").and_then(|v| v.parse().ok()).unwrap_or(0u32).min(SAMPLE_RATE * 5);

        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(RaopShared {
            rtsp: Mutex::new(rtsp),
            uri,
            seq: AtomicU16::new(seq),
            rtptime: AtomicU32::new(rtptime),
            first: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            receiver_latency,
            stop: stop.clone(),
        });
        if let Err(e) = shared.set_volume(cfg.volume) {
            tracing::debug!(kind = e.kind(), "AirPlay receiver didn't take the volume");
        }
        let server_control = SocketAddr::new(cfg.addr.ip(), server_control);
        let resend: Arc<Mutex<VecDeque<(u16, Vec<u8>)>>> = Arc::default();
        let mut threads = Vec::new();
        {
            let stop = stop.clone();
            threads.push(
                std::thread::Builder::new()
                    .name("airplay-timing".into())
                    .spawn(move || timing_loop(timing, stop))
                    .map_err(|e| AppError::Internal(e.to_string()))?,
            );
        }
        {
            let (sock, resend, stop) = (control.clone(), resend.clone(), stop.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("airplay-control".into())
                    .spawn(move || control_loop(sock, server_control, resend, stop))
                    .map_err(|e| AppError::Internal(e.to_string()))?,
            );
        }
        Ok((
            Self {
                shared: shared.clone(),
                audio,
                control,
                server_control,
                ssrc: rand::random(),
                sent_since_sync: 0,
                resend,
                threads,
            },
            shared,
        ))
    }

    fn sync(&self, first: bool, rtptime: u32) {
        let mut p = [0u8; 20];
        p[0] = if first { 0x90 } else { 0x80 };
        p[1] = 0xd4;
        p[2..4].copy_from_slice(&7u16.to_be_bytes());
        p[4..8].copy_from_slice(&rtptime.wrapping_sub(LATENCY_FRAMES).to_be_bytes());
        p[8..16].copy_from_slice(&ntp_now().to_be_bytes());
        p[16..20].copy_from_slice(&rtptime.to_be_bytes());
        let _ = self.control.send_to(&p, self.server_control);
    }
}

impl NetStream for RaopStream {
    fn block_frames(&self) -> usize {
        alac::FRAMES
    }

    fn send(&mut self, pcm: &[i16]) -> std::io::Result<()> {
        if self.shared.is_closed() {
            return Err(std::io::Error::other("session ended"));
        }
        let first = self.shared.first.swap(false, Ordering::SeqCst);
        let seq = self.shared.seq.fetch_add(1, Ordering::SeqCst);
        let ts = self.shared.rtptime.fetch_add(alac::FRAMES as u32, Ordering::SeqCst);
        if first || self.sent_since_sync >= SAMPLE_RATE {
            self.sync(first, ts);
            self.sent_since_sync = 0;
        }
        let mut p = Vec::with_capacity(12 + alac::FRAMES * 4 + 4);
        p.push(0x80);
        p.push(if first { 0xe0 } else { 0x60 });
        p.extend_from_slice(&seq.to_be_bytes());
        p.extend_from_slice(&ts.to_be_bytes());
        p.extend_from_slice(&self.ssrc.to_be_bytes());
        p.extend_from_slice(&alac::encode_stereo(pcm));
        self.audio.send(&p)?;
        self.sent_since_sync += alac::FRAMES as u32;
        let mut r = self.resend.lock();
        if r.len() >= RESEND_KEEP {
            r.pop_front();
        }
        r.push_back((seq, p));
        Ok(())
    }

    fn close(&mut self) {
        self.shared.teardown();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for RaopStream {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_maps_to_decibels() {
        assert_eq!(volume_db(0.0), -144.0);
        assert_eq!(volume_db(1.0), 0.0);
        assert_eq!(volume_db(0.5), -15.0);
        assert_eq!(volume_db(f32::NAN), -144.0);
    }

    #[test]
    fn transport_ports_parse() {
        let t = "RTP/AVP/UDP;unicast;mode=record;server_port=6000;control_port=6001;timing_port=6002";
        assert_eq!(transport_port(t, "server_port"), Some(6000));
        assert_eq!(transport_port(t, "control_port"), Some(6001));
        assert_eq!(transport_port(t, "nope"), None);
    }

    #[test]
    fn ntp_is_after_1900_by_seventy_years() {
        assert!(ntp_now() >> 32 > 2_208_988_800 + 1_600_000_000);
    }
}
