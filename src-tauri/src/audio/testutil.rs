//! Test helpers: generated audio, a tiny range-capable HTTP server that can
//! drop connections, and a fake resolver. No binary fixtures are committed.

use std::collections::HashMap;
use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::source::{PlayContext, ResolvedContext, StreamSource, TrackLookup, TrackResolver};
use crate::error::{AppError, AppResult};
use crate::model::Track;

pub fn track(id: &str, artist: &str, part: &str) -> Track {
    Track { mbid: None,
        id: id.into(),
        title: format!("Track {id}"),
        album_id: "al".into(),
        album_title: "Album".into(),
        artist_id: format!("ar-{artist}"),
        artist_title: artist.into(),
        track_artist: None,
        track_number: 1,
        disc_number: 1,
        duration_ms: 1000,
        thumb: None,
        part_key: part.into(),
        codec: Some("wav".into()),
        container: Some("wav".into()),
        bitrate: None,
        bit_depth: None,
        sample_rate: None,
        user_rating: None,
        view_count: 0,
        last_viewed_at: None,
        rating_count: None,
        file: None,
        other_versions: None,
        song_status: None,
        file_release: None,
        rated_at: None,
        genres: Vec::new(),
    }
}

/// A 16-bit WAV of a sine.
pub fn wav_tone(rate: u32, channels: u16, frames: usize, freq: f32, amp: f32) -> Vec<u8> {
    wav_tone_from(rate, channels, 0, frames, freq, amp)
}

/// Like [`wav_tone`] but starting at sine phase `start` frames, so two
/// files can be exact continuations of each other.
pub fn wav_tone_from(
    rate: u32,
    channels: u16,
    start: usize,
    frames: usize,
    freq: f32,
    amp: f32,
) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cur = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
        for i in start..start + frames {
            let t = i as f64 / rate as f64;
            let s = ((2.0 * std::f64::consts::PI * freq as f64 * t).sin() * amp as f64) as f32;
            for c in 0..channels {
                let v = s * if c == 0 { 1.0 } else { 0.8 };
                w.write_sample((v * i16::MAX as f32) as i16).unwrap();
            }
        }
        w.finalize().unwrap();
    }
    cur.into_inner()
}

/// Decodes a whole in-memory file with symphonia (the reference decode).
pub fn decode_all(bytes: &[u8], hint: &str) -> (Vec<f32>, u32, usize) {
    let mut d = super::decode::TrackDecoder::open(Box::new(Cursor::new(bytes.to_vec())), Some(hint))
        .expect("open");
    let mut out = Vec::new();
    while let Some(b) = d.next_block().expect("decode") {
        out.extend_from_slice(b);
    }
    (out, d.rate, d.channels)
}

/// Captured samples once at least `len` have arrived. The engine reports
/// "ended"/"drained" from inside the render call, slightly before the
/// capture thread stores that block, so tests must wait for the data.
/// The wait only gives up once the capture has stopped growing for 20 s, so
/// a paced (real-time) capture on a slow machine is never cut short.
pub async fn settled_samples(cap: &super::output::CaptureHandle, len: usize) -> Vec<f32> {
    let stall = std::time::Duration::from_secs(20);
    let mut seen = cap.len_frames(1);
    let mut deadline = std::time::Instant::now() + stall;
    loop {
        let now_len = cap.len_frames(1);
        if now_len != seen {
            seen = now_len;
            deadline = std::time::Instant::now() + stall;
        }
        if now_len >= len || std::time::Instant::now() > deadline {
            // Give any unexpected extra audio a moment to show up too.
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            return cap.samples();
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Waits until the capture stops growing (e.g. after a pause ramp).
pub async fn wait_capture_still(cap: &super::output::CaptureHandle) -> usize {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut last = cap.len_frames(1);
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let now = cap.len_frames(1);
        if now == last {
            return now;
        }
        assert!(std::time::Instant::now() < deadline, "capture never went quiet");
        last = now;
    }
}

#[derive(Clone, Default)]
pub struct Route {
    pub body: Vec<u8>,
    /// Drop the connection after this many body bytes, for the first
    /// `drops` requests.
    pub drop_after: Option<usize>,
    pub drops: usize,
    pub status: Option<u16>,
    /// Pause between 16 KiB body chunks (a slow transfer).
    pub chunk_delay: Option<std::time::Duration>,
    /// Once the `drops` are used up, answer every open-ended request with
    /// this status (a server that went away part way through a song).
    pub then_status: Option<u16>,
    /// Answer without a length and ignore ranges (a live transcode).
    pub no_length: bool,
}

#[derive(Default)]
pub struct ServerState {
    pub routes: Mutex<HashMap<String, Route>>,
    pub requests: AtomicUsize,
    pub ranges: Mutex<Vec<String>>,
    /// (path, range) of every request.
    pub log: Mutex<Vec<(String, Option<String>)>>,
    /// Emulate a server that allows one transfer at a time: starting a body
    /// aborts any other body still being sent.
    pub exclusive: std::sync::atomic::AtomicBool,
    pub transfer_gen: std::sync::atomic::AtomicU64,
    /// Bodies aborted because a newer transfer started (exclusive mode).
    pub killed: AtomicUsize,
}

impl ServerState {
    pub fn ranges_for(&self, path: &str) -> Vec<Option<String>> {
        self.log
            .lock()
            .iter()
            .filter(|l| l.0 == path)
            .map(|l| l.1.clone())
            .collect()
    }
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub state: Arc<ServerState>,
}

impl TestServer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(ServerState::default());
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let st = st.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 2048];
                    loop {
                        let Ok(n) = sock.read(&mut tmp).await else { return };
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let path = path.split('?').next().unwrap_or("").to_string();
                    let range = req
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("range:"))
                        .map(|l| l[6..].trim().to_string());
                    st.requests.fetch_add(1, Ordering::SeqCst);
                    if let Some(r) = &range {
                        st.ranges.lock().push(r.clone());
                    }
                    st.log.lock().push((path.clone(), range.clone()));
                    let route = {
                        let mut routes = st.routes.lock();
                        match routes.get_mut(&path) {
                            Some(r) => {
                                let mut snapshot = r.clone();
                                // A request for a closed range (the file's
                                // tail) still gets through.
                                let closed = range.as_deref().is_some_and(|r| !r.ends_with('-'));
                                if r.drops == 0 && r.then_status.is_some() && !closed {
                                    snapshot.status = r.then_status;
                                }
                                if r.drops > 0 {
                                    r.drops -= 1;
                                }
                                Some(snapshot)
                            }
                            None => None,
                        }
                    };
                    let Some(route) = route else {
                        let _ = sock
                            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            .await;
                        return;
                    };
                    if let Some(code) = route.status {
                        let _ = sock
                            .write_all(
                                format!("HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                                    .as_bytes(),
                            )
                            .await;
                        return;
                    }
                    if route.no_length {
                        let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n").await;
                        for chunk in route.body.chunks(16 * 1024) {
                            if sock.write_all(chunk).await.is_err() {
                                return;
                            }
                        }
                        let _ = sock.flush().await;
                        return;
                    }
                    let total = route.body.len();
                    let (start, end) = match range.as_deref().and_then(|r| r.strip_prefix("bytes=")) {
                        Some(spec) => {
                            let mut it = spec.split('-');
                            let s: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                            let e: usize = it
                                .next()
                                .and_then(|x| x.parse().ok())
                                .unwrap_or(total.saturating_sub(1));
                            (s.min(total), (e + 1).min(total))
                        }
                        None => (0, total),
                    };
                    let body = &route.body[start..end];
                    let head = if range.is_some() {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                            body.len(),
                            start,
                            end.saturating_sub(1),
                            total
                        )
                    } else {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                    };
                    if sock.write_all(head.as_bytes()).await.is_err() {
                        return;
                    }
                    let send = match route.drop_after {
                        Some(n) if route.drops > 0 => n.min(body.len()),
                        _ => body.len(),
                    };
                    let my_gen = st.transfer_gen.fetch_add(1, Ordering::SeqCst) + 1;
                    for chunk in body[..send].chunks(16 * 1024) {
                        if st.exclusive.load(Ordering::SeqCst)
                            && st.transfer_gen.load(Ordering::SeqCst) != my_gen
                        {
                            // A newer transfer started: the newest one wins.
                            // Only count it if the client was still reading
                            // (not if it had already hung up on us).
                            let mut probe = [0u8; 1];
                            let client_gone = match sock.try_read(&mut probe) {
                                Ok(0) => true,
                                Ok(_) => false,
                                Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
                            };
                            if !client_gone {
                                st.killed.fetch_add(1, Ordering::SeqCst);
                            }
                            return;
                        }
                        if sock.write_all(chunk).await.is_err() {
                            return;
                        }
                        if let Some(d) = route.chunk_delay {
                            tokio::time::sleep(d).await;
                        }
                    }
                    let _ = sock.flush().await;
                    // Dropping the socket here closes the connection; if we
                    // sent less than Content-Length the client sees an error.
                });
            }
        });
        Self { addr, state }
    }

    pub fn add(&self, path: &str, route: Route) {
        self.state.routes.lock().insert(path.into(), route);
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

/// Resolver backed by the test server: track id → `/t/{id}`, transcode →
/// `/x/{id}`.
pub struct FakeResolver {
    pub base: String,
    pub transcode_calls: AtomicUsize,
    pub tracks: Mutex<HashMap<String, Track>>,
    /// `Some(ids)`: only these can play now (server out of reach).
    pub playable: Mutex<Option<std::collections::HashSet<String>>>,
    /// Every `left_out_offline` call.
    pub left_out: Mutex<Vec<usize>>,
    /// Batches `more` hands out, first first (ids of known tracks).
    pub more_batches: Mutex<Vec<Vec<String>>>,
    /// Every `more` request.
    pub more_calls: Mutex<Vec<super::source::MoreRequest>>,
    /// Hidden songs and artists (shuffle play skips them).
    pub hidden: Mutex<Option<Arc<crate::hidden::HiddenSet>>>,
    /// Track ids whose server can't be reached (stream and transcode fail
    /// with a network error).
    pub unreachable: Mutex<std::collections::HashSet<String>>,
    /// Contexts starting with this track id take this long to look up.
    pub slow_context: Mutex<Option<(String, std::time::Duration)>>,
    /// `more` takes this long to answer.
    pub slow_more: Mutex<Option<std::time::Duration>>,
    /// This many `more` calls fail (network) before answering.
    pub more_fails: AtomicUsize,
}

impl FakeResolver {
    pub fn new(server: &TestServer) -> Arc<Self> {
        Arc::new(Self {
            base: format!("http://{}", server.addr),
            transcode_calls: AtomicUsize::new(0),
            tracks: Mutex::new(HashMap::new()),
            playable: Mutex::new(None),
            left_out: Mutex::new(Vec::new()),
            more_batches: Mutex::new(Vec::new()),
            more_calls: Mutex::new(Vec::new()),
            hidden: Mutex::new(None),
            unreachable: Mutex::new(std::collections::HashSet::new()),
            slow_context: Mutex::new(None),
            slow_more: Mutex::new(None),
            more_fails: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl TrackResolver for FakeResolver {
    async fn stream(&self, track: &Track) -> AppResult<StreamSource> {
        if self.unreachable.lock().contains(&track.id) {
            return Err(AppError::Network("server unreachable".into()));
        }
        Ok(StreamSource {
            url: format!("{}/t/{}", self.base, track.id),
            headers: vec![("X-Plex-Token".into(), "test-token".into())],
            size: None,
            codec_hint: None,
        })
    }
    async fn transcode(&self, track: &Track) -> AppResult<StreamSource> {
        self.transcode_calls.fetch_add(1, Ordering::SeqCst);
        if self.unreachable.lock().contains(&track.id) {
            return Err(AppError::Network("server unreachable".into()));
        }
        Ok(StreamSource {
            url: format!("{}/x/{}", self.base, track.id),
            headers: vec![],
            size: None,
            codec_hint: Some("wav".into()),
        })
    }
}

#[async_trait]
impl TrackLookup for FakeResolver {
    async fn tracks(&self, ids: &[String]) -> AppResult<Vec<Track>> {
        let t = self.tracks.lock();
        Ok(ids.iter().filter_map(|i| t.get(i).cloned()).collect())
    }
    async fn context(&self, ctx: &PlayContext) -> AppResult<ResolvedContext> {
        let ids = ctx
            .track_ids
            .clone()
            .ok_or_else(|| AppError::Invalid("no ids".into()))?;
        let slow = self.slow_context.lock().clone();
        if let Some((first, wait)) = slow
            && ids.first() == Some(&first)
        {
            tokio::time::sleep(wait).await;
        }
        Ok(ResolvedContext {
            tracks: self.tracks(&ids).await?,
            title: Some("Test context".into()),
        })
    }
    async fn playable_now(&self) -> Option<std::collections::HashSet<String>> {
        self.playable.lock().clone()
    }
    fn left_out_offline(&self, n: usize) {
        self.left_out.lock().push(n);
    }
    fn hidden(&self) -> Option<Arc<crate::hidden::HiddenSet>> {
        self.hidden.lock().clone()
    }
    async fn more(&self, req: &super::source::MoreRequest) -> AppResult<Vec<Track>> {
        self.more_calls.lock().push(req.clone());
        let slow = *self.slow_more.lock();
        if let Some(wait) = slow {
            tokio::time::sleep(wait).await;
        }
        if self.more_fails.load(Ordering::SeqCst) > 0 {
            self.more_fails.fetch_sub(1, Ordering::SeqCst);
            return Err(AppError::Network("lookup failed".into()));
        }
        let ids = {
            let mut b = self.more_batches.lock();
            if b.is_empty() { Vec::new() } else { b.remove(0) }
        };
        self.tracks(&ids).await
    }
}
