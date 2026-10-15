//! Playing on a renderer and following it. The renderer plays one song at a
//! time; when one ends, the link hands it the next.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;

use super::soap::{CallError, Soap};
use super::{DlnaDevice, text};
use crate::audio::queue::Repeat;
use crate::error::{AppError, AppResult};
use crate::remote::companion::Timeline;
use crate::remote::lan::LanLink;
use crate::remote::types::{RemoteCommand, repeat_code};
use crate::remote::xml::escape;

/// A song ended when it stopped this close to its end.
const END_SLACK_MS: u64 = 5_000;
/// After handing over the next song, how long it may take to start.
const START_WAIT: Duration = Duration::from_secs(10);

/// What a renderer is told it can't play.
pub const REFUSED: &str = "Doesn't play this song's format";

/// One song for the renderer. URLs carry a token: never logged.
#[derive(Debug, Clone, PartialEq)]
pub struct DlnaItem {
    pub rating_key: String,
    /// The original file and its MIME type, when the renderer takes it.
    pub original: Option<(String, String)>,
    /// The server's MP3 transcode.
    pub transcode: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub image: Option<String>,
    pub duration_ms: u64,
}

#[derive(Default)]
struct St {
    items: Vec<DlnaItem>,
    index: usize,
    /// The URL the renderer has now.
    url: String,
    seen_playing: bool,
    last_pos: u64,
    duration_ms: u64,
    repeat: Repeat,
    volume: Option<i64>,
    /// When the next song was handed over (until it plays).
    handed: Option<Instant>,
}

pub struct DlnaLink {
    dev: DlnaDevice,
    soap: Soap,
    st: Mutex<St>,
}

impl std::fmt::Debug for DlnaLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DlnaLink").field("device", &self.dev.name).finish()
    }
}

/// `H:MM:SS` (with optional fractions) in milliseconds.
pub fn parse_time(s: &str) -> Option<u64> {
    let mut parts = s.trim().split(':');
    let h: u64 = parts.next()?.parse().ok()?;
    let m: u64 = parts.next()?.parse().ok()?;
    let sec: f64 = parts.next()?.parse().ok()?;
    parts.next().is_none().then(|| h * 3_600_000 + m * 60_000 + (sec * 1000.0).round() as u64)
}

/// Milliseconds as `H:MM:SS`.
pub fn format_time(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// The DIDL-Lite description sent with a song.
pub fn didl(item: &DlnaItem, url: &str, mime: &str) -> String {
    let art = item.image.as_deref().map(|u| format!("<upnp:albumArtURI>{}</upnp:albumArtURI>", escape(u))).unwrap_or_default();
    format!(
        "<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\">\
<item id=\"{id}\" parentID=\"0\" restricted=\"1\"><dc:title>{title}</dc:title><dc:creator>{artist}</dc:creator><upnp:artist>{artist}</upnp:artist>\
<upnp:album>{album}</upnp:album>{art}<upnp:class>object.item.audioItem.musicTrack</upnp:class>\
<res protocolInfo=\"http-get:*:{mime}:*\" duration=\"{dur}.000\">{url}</res></item></DIDL-Lite>",
        id = escape(&item.rating_key),
        title = escape(&item.title),
        artist = escape(&item.artist),
        album = escape(&item.album),
        dur = format_time(item.duration_ms),
        url = escape(url),
    )
}

fn unreachable(dev: &DlnaDevice) -> AppError {
    AppError::Network(format!("{} didn't answer", dev.name))
}

impl DlnaLink {
    /// Hand the renderer `items[start]` at `offset_ms` and play.
    pub async fn start(dev: DlnaDevice, items: Vec<DlnaItem>, start: usize, offset_ms: u64, repeat: Repeat, soap: Soap) -> AppResult<Self> {
        if items.get(start).is_none() {
            return Err(AppError::Invalid("Nothing to play there.".into()));
        }
        let link = Self { dev, soap, st: Mutex::new(St { items, repeat, ..St::default() }) };
        link.load(start, offset_ms).await?;
        if let Some(rc) = &link.dev.rendering
            && let Ok(r) = link.soap.call(rc, "GetVolume", &[("InstanceID", "0".into()), ("Channel", "Master".into())]).await
        {
            link.st.lock().volume = text(&r, "CurrentVolume").and_then(|v| v.parse().ok());
        }
        Ok(link)
    }

    async fn av(&self, action: &str, args: &[(&str, String)]) -> Result<String, CallError> {
        let mut all = vec![("InstanceID", "0".to_string())];
        all.extend(args.iter().cloned());
        self.soap.call(&self.dev.av_transport, action, &all).await
    }

    /// Put song `index` on the renderer: the original first, the transcode
    /// when the renderer refuses it.
    async fn load(&self, index: usize, offset_ms: u64) -> AppResult<()> {
        let item = self.st.lock().items.get(index).cloned().ok_or_else(|| AppError::Invalid("Nothing to play there.".into()))?;
        let mut tries: Vec<(String, String)> = Vec::new();
        if let Some(o) = &item.original {
            tries.push(o.clone());
        }
        if let Some(t) = &item.transcode {
            tries.push((t.clone(), "audio/mpeg".into()));
        }
        let mut sent = None;
        for (url, mime) in tries {
            match self.av("SetAVTransportURI", &[("CurrentURI", url.clone()), ("CurrentURIMetaData", didl(&item, &url, &mime))]).await {
                Ok(_) => {
                    sent = Some(url);
                    break;
                }
                Err(CallError::Unreachable) => return Err(unreachable(&self.dev)),
                Err(CallError::Fault(code)) => tracing::info!(?code, "renderer refused a stream"),
            }
        }
        let url = sent.ok_or_else(|| AppError::Invalid(REFUSED.into()))?;
        match self.av("Play", &[("Speed", "1".into())]).await {
            Ok(_) => {}
            Err(CallError::Unreachable) => return Err(unreachable(&self.dev)),
            Err(CallError::Fault(code)) => return Err(AppError::Server(format!("renderer refused play ({code:?})"))),
        }
        if offset_ms > 0 && self.av("Seek", &[("Unit", "REL_TIME".into()), ("Target", format_time(offset_ms))]).await.is_err() {
            tracing::info!("renderer didn't seek to the start position");
        }
        let mut st = self.st.lock();
        st.index = index;
        st.url = url;
        st.seen_playing = false;
        st.last_pos = offset_ms;
        st.duration_ms = item.duration_ms;
        st.handed = Some(Instant::now());
        Ok(())
    }

    /// The song after this one, by the repeat mode.
    fn next_index(&self, by_user: bool) -> Option<usize> {
        let st = self.st.lock();
        let n = st.items.len();
        match st.repeat {
            Repeat::One if !by_user => Some(st.index),
            Repeat::All => Some((st.index + 1) % n.max(1)),
            _ => (st.index + 1 < n).then_some(st.index + 1),
        }
    }

    fn timeline(&self, state: &str, pos: u64) -> Timeline {
        let st = self.st.lock();
        Timeline {
            state: state.into(),
            time_ms: pos,
            duration_ms: st.duration_ms,
            rating_key: st.items.get(st.index).map(|i| i.rating_key.clone()),
            volume: st.volume,
            repeat: repeat_code(st.repeat) as i64,
            ..Timeline::default()
        }
    }

    /// The current song's index (tests).
    pub fn index(&self) -> usize {
        self.st.lock().index
    }
}

#[async_trait]
impl LanLink for DlnaLink {
    async fn poll(&self) -> AppResult<Option<Timeline>> {
        let info = self.av("GetTransportInfo", &[]).await.map_err(|_| unreachable(&self.dev))?;
        let pos = self.av("GetPositionInfo", &[]).await.map_err(|_| unreachable(&self.dev))?;
        let state = text(&info, "CurrentTransportState").unwrap_or_default();
        let rel = text(&pos, "RelTime").and_then(|t| parse_time(&t)).unwrap_or(0);
        let uri = text(&pos, "TrackURI").unwrap_or_default();
        {
            let mut st = self.st.lock();
            if let Some(d) = text(&pos, "TrackDuration").and_then(|t| parse_time(&t)).filter(|d| *d > 0) {
                st.duration_ms = d;
            }
            // Something else is playing there now: it was taken over.
            let ours = st.url.split('?').next().unwrap_or("").to_string();
            if !uri.is_empty() && uri != st.url && !uri.starts_with(&ours) && state == "PLAYING" {
                return Ok(None);
            }
        }
        match state.as_str() {
            "PLAYING" => {
                let mut st = self.st.lock();
                st.seen_playing = true;
                st.handed = None;
                st.last_pos = rel;
                drop(st);
                Ok(Some(self.timeline("playing", rel)))
            }
            "PAUSED_PLAYBACK" => {
                self.st.lock().last_pos = rel;
                Ok(Some(self.timeline("paused", rel)))
            }
            "TRANSITIONING" => Ok(Some(self.timeline("buffering", rel))),
            _ => {
                let (seen, last, dur, handed) = {
                    let st = self.st.lock();
                    (st.seen_playing, st.last_pos, st.duration_ms, st.handed)
                };
                let ended = seen && dur > 0 && last + END_SLACK_MS >= dur;
                if ended {
                    if let Some(next) = self.next_index(false) {
                        self.load(next, 0).await?;
                        return Ok(Some(self.timeline("buffering", 0)));
                    }
                    return Ok(Some(self.timeline("stopped", last)));
                }
                // A song just handed over may take a moment to start.
                if !seen && handed.is_some_and(|h| h.elapsed() < START_WAIT) {
                    return Ok(Some(self.timeline("buffering", last)));
                }
                Ok(Some(self.timeline("stopped", last)))
            }
        }
    }

    async fn command(&self, cmd: &RemoteCommand) -> AppResult<()> {
        let simple = |r: Result<String, CallError>| match r {
            Ok(_) => Ok(()),
            Err(CallError::Unreachable) => Err(unreachable(&self.dev)),
            Err(CallError::Fault(c)) => Err(AppError::Server(format!("renderer refused ({c:?})"))),
        };
        match cmd {
            RemoteCommand::Play => simple(self.av("Play", &[("Speed", "1".into())]).await),
            RemoteCommand::Pause => simple(self.av("Pause", &[]).await),
            RemoteCommand::Next => match self.next_index(true) {
                Some(i) => self.load(i, 0).await,
                None => Err(AppError::Invalid("That's the end of the queue.".into())),
            },
            RemoteCommand::Previous => {
                let (index, pos) = {
                    let st = self.st.lock();
                    (st.index, st.last_pos)
                };
                if pos > 3_000 || index == 0 {
                    self.load(index, 0).await
                } else {
                    self.load(index - 1, 0).await
                }
            }
            RemoteCommand::Seek { position_ms } => {
                let r = simple(self.av("Seek", &[("Unit", "REL_TIME".into()), ("Target", format_time(*position_ms))]).await);
                if r.is_ok() {
                    self.st.lock().last_pos = *position_ms;
                }
                r
            }
            RemoteCommand::Volume { volume } => {
                let rc = self.dev.rendering.as_ref().ok_or_else(|| AppError::Invalid(format!("{} doesn't let Platter set its volume.", self.dev.name)))?;
                let v = if volume.is_finite() { (volume.clamp(0.0, 1.0) * 100.0).round() as i64 } else { 0 };
                let r = self
                    .soap
                    .call(rc, "SetVolume", &[("InstanceID", "0".into()), ("Channel", "Master".into()), ("DesiredVolume", v.to_string())])
                    .await;
                let r = simple(r);
                if r.is_ok() {
                    self.st.lock().volume = Some(v);
                }
                r
            }
            RemoteCommand::Shuffle { .. } => Err(AppError::Invalid("Shuffle can't change while playing on this device.".into())),
            RemoteCommand::Repeat { mode } => {
                self.st.lock().repeat = *mode;
                Ok(())
            }
        }
    }

    async fn leave(&self, stop: bool) {
        if stop {
            let _ = self.av("Stop", &[]).await;
        }
    }
}
