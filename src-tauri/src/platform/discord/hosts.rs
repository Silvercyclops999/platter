//! The keyless image hosts covers can be uploaded to, one small adapter each:
//! how to send the file, how to read the answer, which links count as a
//! direct image on that host, and how long the host keeps the file.
//!
//! In the automatic order the hosts whose links Discord's image proxy takes
//! on the first try come first: Litterbox (72 hours), x0.at (up to 100 days
//! by size), uguu.se (3 hours), and catbox.moe (kept) last, since Discord's
//! proxy often refuses a new catbox.moe link for several minutes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::activity::image_allowed;
use super::art::multipart_form;
use crate::artistinfo::http::HttpResponse;
use crate::error::{AppError, AppResult};

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

/// Where a cover is uploaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Host {
    /// With the user's own client id.
    Imgur,
    Litterbox,
    X0,
    Uguu,
    Catbox,
}

impl Host {
    /// The name kept in the cache (`discord_art.source` and `host`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Imgur => "imgur",
            Self::Litterbox => "litterbox",
            Self::X0 => "x0",
            Self::Uguu => "uguu",
            Self::Catbox => "catbox",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "imgur" => Self::Imgur,
            "litterbox" => Self::Litterbox,
            "x0" => Self::X0,
            "uguu" => Self::Uguu,
            "catbox" => Self::Catbox,
            _ => return None,
        })
    }

    /// The upload host a cover link is on, if it's one of these.
    pub fn from_url(url: &str) -> Option<Self> {
        let host = url::Url::parse(url).ok()?.host_str()?.to_ascii_lowercase();
        Some(match host.as_str() {
            "i.imgur.com" => Self::Imgur,
            "litter.catbox.moe" => Self::Litterbox,
            "x0.at" => Self::X0,
            "files.catbox.moe" => Self::Catbox,
            h if h.ends_with(".uguu.se") => Self::Uguu,
            _ => return None,
        })
    }

    /// The keyless adapter (Imgur needs a key and lives in `art`).
    pub fn adapter(self) -> Option<&'static dyn HostAdapter> {
        match self {
            Self::Imgur => None,
            Self::Litterbox => Some(&Litterbox),
            Self::X0 => Some(&X0),
            Self::Uguu => Some(&Uguu),
            Self::Catbox => Some(&Catbox),
        }
    }

    /// For logs and errors.
    pub fn label(self) -> &'static str {
        match self {
            Self::Imgur => "Imgur",
            Self::Litterbox => "Litterbox",
            Self::X0 => "x0.at",
            Self::Uguu => "uguu.se",
            Self::Catbox => "catbox.moe",
        }
    }
}

/// The "Where covers are uploaded" setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverHost {
    /// Litterbox, then x0.at, then uguu.se, then catbox.moe.
    #[default]
    Auto,
    Litterbox,
    X0,
    Uguu,
    Catbox,
}

/// The automatic order.
pub const AUTO_CHAIN: [Host; 4] = [Host::Litterbox, Host::X0, Host::Uguu, Host::Catbox];

impl CoverHost {
    /// The keyless hosts to try, in order.
    pub fn chain(self) -> &'static [Host] {
        match self {
            Self::Auto => &AUTO_CHAIN,
            Self::Litterbox => &[Host::Litterbox],
            Self::X0 => &[Host::X0],
            Self::Uguu => &[Host::Uguu],
            Self::Catbox => &[Host::Catbox],
        }
    }
}

/// What to POST for one upload.
pub struct UploadForm {
    pub url: &'static str,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// One keyless host.
pub trait HostAdapter: Sync {
    fn host(&self) -> Host;
    /// The multipart upload of `bytes` (a JPEG or PNG).
    fn form(&self, bytes: &[u8], content_type: &str) -> UploadForm;
    /// The file's link from the host's answer, only if it's a direct image
    /// link on that host.
    fn link(&self, r: &HttpResponse) -> AppResult<String>;
    /// How long the host keeps a file of this size, in seconds (None: kept).
    fn keeps(&self, size: usize) -> Option<i64>;
}

/// A direct image link on `host`: https, an allowed cover host, and one
/// path segment that's a file name with an image extension.
pub fn direct_image(link: &str, host: Host) -> bool {
    if !image_allowed(link) || Host::from_url(link) != Some(host) {
        return false;
    }
    let Ok(u) = url::Url::parse(link) else { return false };
    let path = u.path().trim_start_matches('/');
    if path.is_empty() || path.contains('/') || u.fragment().is_some() {
        return false;
    }
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    matches!(ext.as_deref(), Some("jpg" | "jpeg" | "png" | "webp" | "gif"))
}

fn text_link(r: &HttpResponse, host: Host) -> AppResult<String> {
    if !r.ok() {
        return Err(AppError::Server(format!("{} answered {}", host.label(), r.status)));
    }
    let link = String::from_utf8_lossy(&r.body).trim().replace("http://", "https://");
    if !direct_image(&link, host) {
        return Err(AppError::Decode(format!("{} gave no usable link", host.label())));
    }
    Ok(link)
}

/// Litterbox, catbox.moe's temporary sibling: no key, kept 72 hours.
pub struct Litterbox;
pub const LITTERBOX_UPLOAD: &str = "https://litterbox.catbox.moe/resources/internals/api.php";

impl HostAdapter for Litterbox {
    fn host(&self) -> Host {
        Host::Litterbox
    }
    fn form(&self, bytes: &[u8], ct: &str) -> UploadForm {
        let (body, content_type) = multipart_form(&[("reqtype", "fileupload"), ("time", "72h")], "fileToUpload", bytes, ct);
        UploadForm { url: LITTERBOX_UPLOAD, content_type, body }
    }
    fn link(&self, r: &HttpResponse) -> AppResult<String> {
        text_link(r, Host::Litterbox)
    }
    fn keeps(&self, _: usize) -> Option<i64> {
        Some(72 * HOUR)
    }
}

/// x0.at: no key; a file is kept 3 to 100 days, longer the smaller it is.
pub struct X0;
pub const X0_UPLOAD: &str = "https://x0.at/";
/// x0.at's largest file, which gets the shortest time.
const X0_MAX_BYTES: f64 = 222.0 * 1024.0 * 1024.0;

impl HostAdapter for X0 {
    fn host(&self) -> Host {
        Host::X0
    }
    fn form(&self, bytes: &[u8], ct: &str) -> UploadForm {
        let (body, content_type) = multipart_form(&[], "file", bytes, ct);
        UploadForm { url: X0_UPLOAD, content_type, body }
    }
    fn link(&self, r: &HttpResponse) -> AppResult<String> {
        text_link(r, Host::X0)
    }
    /// The host's published rule: min + (min - max) * (size / max_size - 1)^3.
    fn keeps(&self, size: usize) -> Option<i64> {
        let (min, max) = (3.0, 100.0);
        let ratio = (size as f64 / X0_MAX_BYTES).min(1.0);
        let days = min + (min - max) * (ratio - 1.0).powi(3);
        Some((days.clamp(min, max) * DAY as f64) as i64)
    }
}

/// uguu.se: no key, kept 3 hours. Answers in JSON.
pub struct Uguu;
pub const UGUU_UPLOAD: &str = "https://uguu.se/upload";

impl HostAdapter for Uguu {
    fn host(&self) -> Host {
        Host::Uguu
    }
    fn form(&self, bytes: &[u8], ct: &str) -> UploadForm {
        let (body, content_type) = multipart_form(&[], "files[]", bytes, ct);
        UploadForm { url: UGUU_UPLOAD, content_type, body }
    }
    fn link(&self, r: &HttpResponse) -> AppResult<String> {
        if !r.ok() {
            return Err(AppError::Server(format!("uguu.se answered {}", r.status)));
        }
        let v: Value = r.json()?;
        let link = v["files"][0]["url"].as_str().unwrap_or_default().replace("http://", "https://");
        if v["success"].as_bool() != Some(true) || !direct_image(&link, Host::Uguu) {
            return Err(AppError::Decode("uguu.se gave no usable link".into()));
        }
        Ok(link)
    }
    fn keeps(&self, _: usize) -> Option<i64> {
        Some(3 * HOUR)
    }
}

/// catbox.moe: no key, kept for good.
pub struct Catbox;
pub const CATBOX_UPLOAD: &str = "https://catbox.moe/user/api.php";

impl HostAdapter for Catbox {
    fn host(&self) -> Host {
        Host::Catbox
    }
    fn form(&self, bytes: &[u8], ct: &str) -> UploadForm {
        let (body, content_type) = multipart_form(&[("reqtype", "fileupload")], "fileToUpload", bytes, ct);
        UploadForm { url: CATBOX_UPLOAD, content_type, body }
    }
    fn link(&self, r: &HttpResponse) -> AppResult<String> {
        text_link(r, Host::Catbox)
    }
    fn keeps(&self, _: usize) -> Option<i64> {
        None
    }
}
