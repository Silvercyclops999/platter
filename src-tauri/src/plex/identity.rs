//! Stable per-install client identity and the `X-Plex-*` headers sent on
//! every request.

use std::path::Path;

use reqwest::header::{ACCEPT, HeaderMap, HeaderName, HeaderValue};

use crate::error::{AppError, AppResult};

pub const PRODUCT: &str = "Platter";
pub(crate) const ID_FILE: &str = "plex-client-id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIdentity {
    pub client_id: String,
    pub product: String,
    pub version: String,
    pub platform: String,
    pub platform_version: String,
    pub device_name: String,
}

impl ClientIdentity {
    /// Load the client id from `<dir>/plex-client-id`, generating and saving
    /// a new UUID the first time.
    pub fn load_or_create(dir: &Path) -> AppResult<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(ID_FILE);
        let id = match std::fs::read_to_string(&path) {
            Ok(s) if uuid::Uuid::parse_str(s.trim()).is_ok() => s.trim().to_string(),
            _ => {
                let id = uuid::Uuid::new_v4().to_string();
                std::fs::write(&path, &id)?;
                id
            }
        };
        Ok(Self::with_id(id))
    }

    /// Identity with a fixed id (tests).
    pub fn with_id(client_id: impl Into<String>) -> Self {
        ClientIdentity {
            client_id: client_id.into(),
            product: PRODUCT.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            platform: platform_name().to_string(),
            platform_version: std::env::consts::ARCH.to_string(),
            device_name: device_name(),
        }
    }

    /// Headers sent on every plex.tv and server request (no token).
    pub fn headers(&self) -> AppResult<HeaderMap> {
        let mut h = HeaderMap::new();
        h.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let pairs = [
            ("x-plex-client-identifier", self.client_id.as_str()),
            ("x-plex-product", self.product.as_str()),
            ("x-plex-version", self.version.as_str()),
            ("x-plex-platform", self.platform.as_str()),
            ("x-plex-platform-version", self.platform_version.as_str()),
            ("x-plex-device", self.platform.as_str()),
            ("x-plex-device-name", self.device_name.as_str()),
            ("x-plex-provides", "player"),
        ];
        for (k, v) in pairs {
            let value = HeaderValue::from_str(&sanitize_header(v))
                .map_err(|e| AppError::Internal(format!("bad header {k}: {e}")))?;
            h.insert(HeaderName::from_static(k), value);
        }
        Ok(h)
    }
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "ios") {
        "iOS"
    } else if cfg!(target_os = "android") {
        "Android"
    } else {
        "Linux"
    }
}

fn device_name() -> String {
    // iOS hands apps only the model name; "iPhone" reads better in other
    // devices' lists than the product and platform.
    if cfg!(target_os = "ios") {
        return "iPhone".into();
    }
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    format!("{PRODUCT} ({})", platform_name())
}

/// Header values must be visible ASCII; replace anything else.
fn sanitize_header(v: &str) -> String {
    v.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() {
                c
            } else {
                '?'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_generated_once_and_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let a = ClientIdentity::load_or_create(dir.path()).unwrap();
        let b = ClientIdentity::load_or_create(dir.path()).unwrap();
        assert_eq!(a.client_id, b.client_id);
        assert!(uuid::Uuid::parse_str(&a.client_id).is_ok());
        let other = tempfile::tempdir().unwrap();
        let c = ClientIdentity::load_or_create(other.path()).unwrap();
        assert_ne!(a.client_id, c.client_id);
    }

    #[test]
    fn headers_include_product_and_accept_json() {
        let id = ClientIdentity::with_id("abc");
        let h = id.headers().unwrap();
        assert_eq!(h["accept"], "application/json");
        assert_eq!(h["x-plex-client-identifier"], "abc");
        assert_eq!(h["x-plex-product"], "Platter");
        assert_eq!(h["x-plex-version"], env!("CARGO_PKG_VERSION"));
        assert!(h.contains_key("x-plex-platform"));
        assert!(h.contains_key("x-plex-device-name"));
        assert!(!h.contains_key("x-plex-token"));
    }

    #[test]
    fn non_ascii_device_names_are_sanitized() {
        assert_eq!(sanitize_header("Café"), "Caf?");
    }
}
