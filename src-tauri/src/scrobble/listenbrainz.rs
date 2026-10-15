//! ListenBrainz: submit listens with the user token (the same token artist
//! info uses, kept in the OS keychain).

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use super::{Listen, ScrobbleService, SendError, Service};
use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;
use crate::web::{Web, WebRequest};

pub const API: &str = "https://api.listenbrainz.org/1/";
pub const TOKEN_ACCOUNT: &str = "listenbrainz-token";

pub struct ListenBrainz {
    web: Arc<dyn Web>,
    keys: Arc<dyn TokenStore>,
}

#[derive(Deserialize)]
struct Validate {
    #[serde(default)]
    valid: bool,
    #[serde(default)]
    user_name: Option<String>,
}

fn metadata(l: &Listen) -> serde_json::Value {
    let mut info = json!({
        "media_player": "Platter",
        "submission_client": "Platter",
        "submission_client_version": env!("CARGO_PKG_VERSION"),
    });
    if l.duration_s > 0 {
        info["duration_ms"] = json!(l.duration_s as u64 * 1000);
    }
    if l.track_number > 0 {
        info["tracknumber"] = json!(l.track_number);
    }
    let mut m = json!({ "artist_name": l.artist, "track_name": l.title, "additional_info": info });
    if !l.album.is_empty() {
        m["release_name"] = json!(l.album);
    }
    m
}

/// The JSON body for `submit-listens`.
pub fn body(kind: &str, ls: &[Listen]) -> String {
    let payload: Vec<serde_json::Value> = ls
        .iter()
        .map(|l| {
            let mut p = json!({ "track_metadata": metadata(l) });
            if kind != "playing_now" {
                p["listened_at"] = json!(l.started_at);
            }
            p
        })
        .collect();
    json!({ "listen_type": kind, "payload": payload }).to_string()
}

impl ListenBrainz {
    pub fn new(web: Arc<dyn Web>, keys: Arc<dyn TokenStore>) -> Self {
        Self { web, keys }
    }

    fn token(&self) -> Option<String> {
        self.keys.get(TOKEN_ACCOUNT).ok().flatten().map(|t| t.expose().to_string())
    }

    /// Whose token it is (None when it's refused).
    pub async fn user(&self) -> AppResult<Option<String>> {
        let Some(t) = self.token() else { return Ok(None) };
        let resp = self.web.send(WebRequest::get(format!("{API}validate-token")).header("Authorization", format!("Token {t}"))).await?;
        if resp.status == 401 {
            return Ok(None);
        }
        if !resp.ok() {
            return Err(AppError::Network(format!("ListenBrainz answered {}", resp.status)));
        }
        let v: Validate = resp.json()?;
        Ok(if v.valid { Some(v.user_name.unwrap_or_default()) } else { None })
    }

    async fn submit(&self, kind: &str, ls: &[Listen]) -> Result<(), SendError> {
        let t = self.token().ok_or(SendError::NotSignedIn)?;
        let req = WebRequest::post_json(format!("{API}submit-listens"), body(kind, ls)).header("Authorization", format!("Token {t}"));
        let resp = self.web.send(req).await.map_err(|e| SendError::Later(e.kind().to_string()))?;
        match resp.status {
            200..=299 => Ok(()),
            401 => Err(SendError::Rejected),
            429 | 500..=599 => Err(SendError::Later(format!("ListenBrainz answered {}", resp.status))),
            s => Err(SendError::Refused(format!("ListenBrainz answered {s}"))),
        }
    }
}

#[async_trait]
impl ScrobbleService for ListenBrainz {
    fn service(&self) -> Service {
        Service::Listenbrainz
    }

    fn signed_in(&self) -> bool {
        self.token().is_some()
    }

    async fn now_playing(&self, l: &Listen) -> Result<(), SendError> {
        self.submit("playing_now", std::slice::from_ref(l)).await
    }

    async fn scrobble(&self, ls: &[Listen]) -> Result<(), SendError> {
        let kind = if ls.len() == 1 { "single" } else { "import" };
        self.submit(kind, ls).await
    }
}
