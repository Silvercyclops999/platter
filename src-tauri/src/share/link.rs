//! `platter://` links: `platter://<kind>/<id>[?t=<seconds>][&s=<server>]`.
//!
//! Kinds are `song`, `album`, `artist` and `playlist` (`track` is read as
//! `song`). Ids are Plex rating keys, which only mean something on the
//! server they came from, so a link carries a short tag of that server's id
//! (`s`): a hash, not the id itself. A link from another server says so
//! instead of opening whatever has the same number here.

use serde::{Deserialize, Serialize};

pub const SCHEME: &str = "platter";
/// Longest start offset a link may ask for (a day).
pub const MAX_T: u32 = 24 * 3600;
const MAX_ID: usize = 64;

/// What a share or a link points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShareKind {
    Track,
    Album,
    Artist,
    Playlist,
}

impl ShareKind {
    /// The word in a link.
    pub fn link_word(self) -> &'static str {
        match self {
            Self::Track => "song",
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
        }
    }

    fn from_word(w: &str) -> Option<Self> {
        match w.to_ascii_lowercase().as_str() {
            "song" | "track" => Some(Self::Track),
            "album" => Some(Self::Album),
            "artist" => Some(Self::Artist),
            "playlist" => Some(Self::Playlist),
            _ => None,
        }
    }
}

/// A parsed link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppLink {
    pub kind: ShareKind,
    pub id: String,
    /// Start offset in seconds (songs only).
    pub t: Option<u32>,
    /// Server tag, when the link has one.
    pub server: Option<String>,
}

/// Short tag for a server id: 8 hex digits of its FNV-1a hash.
pub fn server_tag(machine_id: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in machine_id.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Build a link. `t` is kept for songs only.
pub fn build(kind: ShareKind, id: &str, t: Option<u32>, machine_id: Option<&str>) -> String {
    let mut q = Vec::new();
    if kind == ShareKind::Track
        && let Some(t) = t
    {
        q.push(format!("t={}", t.min(MAX_T)));
    }
    if let Some(m) = machine_id.filter(|m| !m.is_empty()) {
        q.push(format!("s={}", server_tag(m)));
    }
    let id: String = url::form_urlencoded::byte_serialize(id.as_bytes()).collect();
    let mut s = format!("{SCHEME}://{}/{id}", kind.link_word());
    if !q.is_empty() {
        s.push('?');
        s.push_str(&q.join("&"));
    }
    s
}

/// A start offset: whole seconds, or `m:ss` / `h:mm:ss`.
fn parse_t(v: &str) -> Option<u32> {
    let v = v.trim().trim_end_matches('s');
    let mut total: u32 = 0;
    let parts: Vec<&str> = v.split(':').collect();
    if parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) || p.len() > 6) {
        return None;
    }
    for p in parts {
        total = total.checked_mul(60)?.checked_add(p.parse().ok()?)?;
    }
    (total <= MAX_T).then_some(total)
}

/// Read a link. None when it isn't a `platter://` link Platter can open.
pub fn parse(raw: &str) -> Option<AppLink> {
    let u = url::Url::parse(raw.trim()).ok()?;
    if !u.scheme().eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    // platter://album/1 has the kind as the host; platter:album/1 and
    // platter:///album/1 have it in the path.
    let mut segs: Vec<String> = Vec::new();
    if let Some(h) = u.host_str().filter(|h| !h.is_empty()) {
        segs.push(h.to_string());
    }
    segs.extend(u.path().split('/').filter(|s| !s.is_empty()).map(|s| {
        percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
    }));
    let [kind, id] = segs.as_slice() else { return None };
    let kind = ShareKind::from_word(kind)?;
    if !valid_id(id) {
        return None;
    }
    let mut t = None;
    let mut server = None;
    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "t" if kind == ShareKind::Track => t = Some(parse_t(&v)?),
            "s" if v.len() == 8 && v.bytes().all(|b| b.is_ascii_hexdigit()) => server = Some(v.to_ascii_lowercase()),
            _ => {}
        }
    }
    Some(AppLink { kind, id: id.clone(), t, server })
}
