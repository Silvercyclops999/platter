//! DLNA / UPnP media renderers (receivers, smart speakers, TVs) as Listen
//! on targets.
//!
//! Found with SSDP (an `M-SEARCH` for MediaRenderer devices, answered by
//! unicast), then each device's description is read for its AVTransport,
//! RenderingControl and ConnectionManager services, and ConnectionManager's
//! `GetProtocolInfo` says which formats it takes. Playback is the renderer
//! fetching a stream URL from the Plex server: the original file when the
//! renderer takes its format, else the server's MP3 transcode. Platter keeps
//! the queue and hands the renderer the next song when one ends.
//!
//! - [`soap`]: UPnP control calls and faults.
//! - [`link`]: following and controlling it as a [`super::lan::LanLink`].

pub mod link;
pub mod soap;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use url::Url;

use super::lan::lan_player;
use super::service::ServerCtx;
use super::types::{PlayerKind, PlayerVia, RemotePlayer};
use crate::model::Track;
use crate::plex::Token;
use link::DlnaItem;

pub use link::DlnaLink;

/// The device type searched for.
pub const RENDERER: &str = "urn:schemas-upnp-org:device:MediaRenderer:1";
/// The SSDP multicast group and port.
pub const SSDP_GROUP: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(239, 255, 255, 250)), 1900);
/// How long a search listens for answers.
pub const SEARCH_WAIT: Duration = Duration::from_millis(1500);
/// How long a device's description is kept before it's read again.
const DESCRIPTION_TTL: Duration = Duration::from_secs(600);
/// Songs handed over at most (Platter feeds the renderer one at a time).
pub const MAX_ITEMS: usize = 200;

/// One UPnP service on a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// The full service type, e.g. `urn:schemas-upnp-org:service:AVTransport:1`.
    pub service_type: String,
    pub control: Url,
}

/// A renderer found on the network.
#[derive(Debug, Clone, PartialEq)]
pub struct DlnaDevice {
    /// "dlna:<UDN>".
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub av_transport: Service,
    pub rendering: Option<Service>,
    pub connection: Option<Service>,
    /// The formats it takes (MIME types from `GetProtocolInfo`, lower case).
    /// Empty when it didn't say.
    pub sinks: Vec<String>,
}

impl DlnaDevice {
    pub fn player(&self) -> RemotePlayer {
        let hay = format!("{} {}", self.name, self.model.as_deref().unwrap_or("")).to_lowercase();
        let kind = if hay.contains("tv") || hay.contains("bravia") { PlayerKind::Tv } else { PlayerKind::Speaker };
        let product = self.model.clone().unwrap_or_else(|| "DLNA".into());
        lan_player(self.id.clone(), self.name.clone(), product, kind, PlayerVia::Dlna)
    }

    /// Whether it says it takes `mime` (a renderer that didn't say is
    /// assumed to take MP3 only).
    pub fn takes(&self, mime: &str) -> bool {
        if self.sinks.is_empty() {
            return mime == "audio/mpeg";
        }
        let alias: &[&str] = match mime {
            "audio/flac" => &["audio/flac", "audio/x-flac"],
            "audio/mp4" => &["audio/mp4", "audio/x-m4a", "audio/m4a", "audio/aac"],
            "audio/wav" => &["audio/wav", "audio/x-wav", "audio/wave"],
            "audio/mpeg" => &["audio/mpeg", "audio/mp3", "audio/x-mpeg"],
            other => return self.sinks.iter().any(|s| s == other),
        };
        self.sinks.iter().any(|s| alias.contains(&s.as_str()))
    }
}

// ---------------------------------------------------------------- XML text

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The local name of a tag (`u:Play` is `Play`).
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// The raw inner text of every `<name>` element (any namespace prefix),
/// in document order. Nested elements of the same name aren't expected.
pub fn blocks<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = xml[i..].find('<') {
        let start = i + off + 1;
        let Some(end_rel) = xml[start..].find('>') else { break };
        let tag = &xml[start..start + end_rel];
        i = start + end_rel + 1;
        if tag.starts_with('/') || tag.starts_with('?') || tag.starts_with('!') || tag.ends_with('/') {
            continue;
        }
        let tag_name = tag.split_whitespace().next().unwrap_or("");
        if local(tag_name) != name {
            continue;
        }
        // The matching close tag, with the same prefix.
        let close = format!("</{tag_name}>");
        if let Some(c) = xml[i..].find(&close) {
            out.push(&xml[i..i + c]);
            i += c + close.len();
        }
    }
    out
}

/// The text of the first `<name>` element, unescaped and trimmed.
pub fn text(xml: &str, name: &str) -> Option<String> {
    blocks(xml, name).first().map(|t| unescape(t.trim())).filter(|t| !t.is_empty())
}

// ---------------------------------------------------------------- discovery

/// The `M-SEARCH` request for renderers.
pub fn search_request() -> String {
    format!("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: {RENDERER}\r\n\r\n")
}

/// The description URL in a search answer from `from`. Only plain http on
/// the address that answered: a device can't point Platter elsewhere.
pub fn answer_location(answer: &str, from: IpAddr) -> Option<Url> {
    let mut lines = answer.split("\r\n");
    if !lines.next()?.to_ascii_uppercase().starts_with("HTTP/1.1 200") {
        return None;
    }
    let loc = lines.filter_map(|l| l.split_once(':')).find(|(k, _)| k.trim().eq_ignore_ascii_case("location")).map(|(_, v)| v.trim())?;
    let u = Url::parse(loc).ok()?;
    let host_ip: IpAddr = u.host_str()?.trim_matches(['[', ']']).parse().ok()?;
    (u.scheme() == "http" && host_ip == from).then_some(u)
}

/// A device description: the renderer and its services. None when it isn't
/// a renderer with AVTransport.
pub fn parse_description(xml: &str, location: &Url) -> Option<DlnaDevice> {
    if !blocks(xml, "deviceType").iter().any(|t| t.contains("MediaRenderer")) {
        return None;
    }
    let base = text(xml, "URLBase").and_then(|b| Url::parse(&b).ok()).unwrap_or_else(|| location.clone());
    let service = |kind: &str| -> Option<Service> {
        blocks(xml, "service").into_iter().find_map(|s| {
            let ty = text(s, "serviceType")?;
            if !ty.contains(&format!(":service:{kind}:")) {
                return None;
            }
            let control = base.join(&text(s, "controlURL")?).ok()?;
            // Control calls stay on the device that described itself.
            (control.host_str() == location.host_str() && control.scheme() == "http").then_some(Service { service_type: ty, control })
        })
    };
    let udn = text(xml, "UDN")?;
    Some(DlnaDevice {
        id: format!("dlna:{}", udn.trim_start_matches("uuid:")),
        name: text(xml, "friendlyName").unwrap_or_else(|| "DLNA device".into()),
        model: text(xml, "modelName"),
        av_transport: service("AVTransport")?,
        rendering: service("RenderingControl"),
        connection: service("ConnectionManager"),
        sinks: Vec::new(),
    })
}

/// MIME types in a `GetProtocolInfo` sink list
/// (`http-get:*:audio/flac:*,http-get:*:audio/mpeg:DLNA.ORG_PN=MP3`).
pub fn sink_types(sink: &str) -> Vec<String> {
    let mut out: Vec<String> = sink
        .split(',')
        .filter_map(|p| {
            let mut parts = p.trim().splitn(4, ':');
            let proto = parts.next()?;
            parts.next()?;
            let mime = parts.next()?.trim().to_ascii_lowercase();
            (proto == "http-get" && !mime.is_empty() && mime != "*").then_some(mime)
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Read a renderer's description and the formats it takes.
pub async fn describe(http: &reqwest::Client, soap: &soap::Soap, location: &Url) -> Option<DlnaDevice> {
    let body = http.get(location.clone()).timeout(Duration::from_secs(4)).send().await.ok()?.text().await.ok()?;
    let mut dev = parse_description(&body, location)?;
    if let Some(cm) = &dev.connection
        && let Ok(r) = soap.call(cm, "GetProtocolInfo", &[]).await
    {
        dev.sinks = sink_types(&text(&r, "Sink").unwrap_or_default());
    }
    Some(dev)
}

/// Where renderers come from.
#[async_trait]
pub trait DlnaSource: Send + Sync {
    /// Renderers on the network now.
    async fn renderers(&self) -> Vec<DlnaDevice>;
    /// The setting turned off: forget what was found.
    fn stop(&self) {}
}

/// SSDP on this computer's network: one search each time the picker asks,
/// descriptions kept for a while.
pub struct Ssdp {
    http: reqwest::Client,
    soap: soap::Soap,
    /// Description URLs to use instead of searching (tests).
    fixed: Option<Vec<Url>>,
    seen: Mutex<HashMap<Url, (Instant, Option<DlnaDevice>)>>,
}

impl Ssdp {
    pub fn new(http: reqwest::Client) -> Self {
        Self { soap: soap::Soap::new(http.clone(), Duration::from_secs(4)), http, fixed: None, seen: Mutex::default() }
    }

    /// Devices at these description URLs, without searching.
    pub fn fixed(http: reqwest::Client, urls: Vec<Url>) -> Self {
        Self { fixed: Some(urls), ..Self::new(http) }
    }

    async fn search(&self) -> Vec<Url> {
        if let Some(f) = &self.fixed {
            return f.clone();
        }
        let Ok(sock) = tokio::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await else {
            return Vec::new();
        };
        let req = search_request();
        // UDP may drop one; ask twice.
        for _ in 0..2 {
            if let Err(e) = sock.send_to(req.as_bytes(), SSDP_GROUP).await {
                tracing::debug!("ssdp search not sent: {e}");
                return Vec::new();
            }
        }
        let mut out: Vec<Url> = Vec::new();
        let until = tokio::time::Instant::now() + SEARCH_WAIT;
        let mut buf = vec![0u8; 4096];
        while let Ok(Ok((n, from))) = tokio::time::timeout_at(until, sock.recv_from(&mut buf)).await {
            if let Some(u) = answer_location(&String::from_utf8_lossy(&buf[..n]), from.ip())
                && !out.contains(&u)
            {
                out.push(u);
            }
        }
        out
    }
}

#[async_trait]
impl DlnaSource for Ssdp {
    async fn renderers(&self) -> Vec<DlnaDevice> {
        let mut out: Vec<DlnaDevice> = Vec::new();
        for loc in self.search().await {
            let cached = self.seen.lock().get(&loc).filter(|(at, _)| at.elapsed() < DESCRIPTION_TTL).map(|(_, d)| d.clone());
            let dev = match cached {
                Some(d) => d,
                None => {
                    let d = describe(&self.http, &self.soap, &loc).await;
                    self.seen.lock().insert(loc, (Instant::now(), d.clone()));
                    d
                }
            };
            if let Some(d) = dev
                && !out.iter().any(|o| o.id == d.id)
            {
                out.push(d);
            }
        }
        out
    }

    fn stop(&self) {
        self.seen.lock().clear();
    }
}

/// A shared source (cheap to clone into the service).
pub type SharedSource = Arc<dyn DlnaSource>;

// ---------------------------------------------------------------- plan

/// What the renderer is handed for each song: the original file when it
/// takes the format, and always the server's MP3 transcode to fall back on.
pub fn plan(server: &ServerCtx, token: &Token, client_id: &str, dev: &DlnaDevice, tracks: &[Track]) -> Vec<DlnaItem> {
    use super::cast::plan::{cast_type, on_server, transcode_mp3, with_token};
    tracks
        .iter()
        .filter(|t| on_server(server, t))
        .take(MAX_ITEMS)
        .map(|t| {
            let key = crate::sid::key(&t.id).to_string();
            let part = crate::sid::key(&t.part_key);
            let original = cast_type(t)
                .filter(|m| dev.takes(m) && part.starts_with('/'))
                .and_then(|m| Some((with_token(server.url(part).ok()?, token), m.to_string())));
            let image = t
                .thumb
                .as_deref()
                .map(crate::sid::key)
                .filter(|p| p.starts_with('/'))
                .and_then(|p| server.url(p).ok())
                .map(|u| with_token(u, token));
            DlnaItem {
                transcode: transcode_mp3(server, token, client_id, &key),
                rating_key: key,
                original,
                title: t.title.clone(),
                artist: t.track_artist.clone().unwrap_or_else(|| t.artist_title.clone()),
                album: t.album_title.clone(),
                image,
                duration_ms: t.duration_ms,
            }
        })
        .collect()
}
