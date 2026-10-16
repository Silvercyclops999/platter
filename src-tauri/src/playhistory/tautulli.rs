//! Tautulli: its play history for the signed-in user's music plays
//! (`get_history`). Tautulli is set up in Settings > Integrations: its
//! address in Settings, its API key in the OS keychain.
//!
//! Tautulli knows how long each play lasted, so a play too short to count
//! (a skip, by the same rule as Platter's own plays) is left out.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::plex::PlexServer;
use super::store::Fetched;
use super::{Feed, Page};
use crate::db::DbHandle;
use crate::db::listens::{ImportSource, is_skip};
use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;
use crate::plex::lenient;
use crate::plex::token::Token;
use crate::web::{Web, WebRequest};

/// Keychain account for the API key.
pub const KEY_ACCOUNT: &str = "tautulli-api-key";
/// A play this far through counts as heard to the end.
const COMPLETE_PERCENT: i64 = 90;

/// Whether Tautulli is set up (never the key itself).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TautulliStatus {
    pub url_set: bool,
    pub has_key: bool,
}

impl TautulliStatus {
    pub fn ready(&self) -> bool {
        self.url_set && self.has_key
    }
}

pub struct Tautulli {
    pub web: Arc<dyn Web>,
    pub keys: Arc<dyn TokenStore>,
    pub db: DbHandle,
    pub plex: Arc<dyn PlexServer>,
}

impl Tautulli {
    fn url(&self) -> String {
        crate::settings::load(&self.db).tautulli_url
    }

    pub fn status(&self) -> TautulliStatus {
        TautulliStatus { url_set: !self.url().trim().is_empty(), has_key: self.keys.get(KEY_ACCOUNT).ok().flatten().is_some() }
    }

    /// Save (or with None remove) the API key in the keychain.
    pub fn set_key(&self, key: Option<&str>) -> AppResult<TautulliStatus> {
        match key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => {
                if k.len() > 200 || !k.chars().all(|c| c.is_ascii_alphanumeric()) {
                    return Err(AppError::Invalid(
                        "A Tautulli API key is letters and numbers, from Tautulli's Settings > Web Interface.".into(),
                    ));
                }
                self.keys.set(KEY_ACCOUNT, &Token::new(k)?)?
            }
            None => self.keys.delete(KEY_ACCOUNT)?,
        }
        Ok(self.status())
    }
}

#[derive(Deserialize)]
struct Reply {
    response: Response,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Response {
    #[serde(deserialize_with = "lenient::string")]
    result: String,
    #[serde(deserialize_with = "lenient::string")]
    message: String,
    data: Option<Data>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Data {
    #[serde(rename = "recordsFiltered", deserialize_with = "lenient::opt_i64")]
    filtered: Option<i64>,
    data: Vec<Row>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Row {
    #[serde(deserialize_with = "lenient::opt_i64")]
    id: Option<i64>,
    #[serde(deserialize_with = "lenient::opt_i64")]
    row_id: Option<i64>,
    #[serde(deserialize_with = "lenient::opt_i64")]
    started: Option<i64>,
    #[serde(deserialize_with = "lenient::opt_i64")]
    date: Option<i64>,
    /// Seconds played (paused time left out).
    #[serde(deserialize_with = "lenient::opt_i64")]
    play_duration: Option<i64>,
    #[serde(deserialize_with = "lenient::opt_i64")]
    duration: Option<i64>,
    #[serde(deserialize_with = "lenient::opt_i64")]
    percent_complete: Option<i64>,
    #[serde(deserialize_with = "lenient::string")]
    user: String,
    #[serde(deserialize_with = "lenient::string")]
    media_type: String,
    #[serde(deserialize_with = "lenient::opt_i64")]
    rating_key: Option<i64>,
    #[serde(deserialize_with = "lenient::string")]
    title: String,
    #[serde(deserialize_with = "lenient::string")]
    parent_title: String,
    #[serde(deserialize_with = "lenient::string")]
    grandparent_title: String,
    #[serde(deserialize_with = "lenient::string")]
    original_title: String,
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

const BAD_KEY: &str = "Tautulli didn't accept the API key. Check it in Settings > Integrations.";
const UNREACHABLE: &str = "Couldn't reach Tautulli. Platter will try again later.";

/// A `get_history` answer: `user`'s song plays long enough to count, how
/// many were too short, how many rows came back, and the total.
pub fn parse_history(body: &[u8], user: &str, tag: Option<&str>) -> AppResult<(Vec<Fetched>, u64, usize, Option<u64>)> {
    let reply: Reply = serde_json::from_slice(body).map_err(|_| AppError::Server(UNREACHABLE.into()))?;
    let r = reply.response;
    if r.result != "success" {
        let m = r.message.to_ascii_lowercase();
        return Err(if m.contains("apikey") || m.contains("api key") {
            AppError::Auth(BAD_KEY.into())
        } else {
            AppError::Server(format!("Tautulli couldn't list your history ({}). Platter will try again later.", r.message.trim()))
        });
    }
    let data = r.data.unwrap_or_default();
    let read = data.data.len();
    let (mut out, mut short) = (Vec::new(), 0);
    for row in data.data {
        if !row.user.trim().eq_ignore_ascii_case(user.trim()) || row.media_type != "track" || row.title.trim().is_empty() {
            continue;
        }
        let Some(id) = row.id.or(row.row_id) else {
            continue;
        };
        let Some(at) = row.started.or(row.date).filter(|v| *v > 0) else {
            continue;
        };
        let played = row.play_duration.or(row.duration).map(|s| (s.max(0) * 1000) as u64);
        let completed = row.percent_complete.is_some_and(|p| p >= COMPLETE_PERCENT);
        if played.is_some_and(|ms| is_skip(ms, completed)) {
            short += 1;
            continue;
        }
        let key = row.rating_key.filter(|k| *k > 0).map(|k| match tag {
            Some(t) => crate::sid::qualify(t, &k.to_string()),
            None => k.to_string(),
        });
        out.push(Fetched {
            source_id: id.to_string(),
            played_at: at * 1000,
            ms_played: played,
            rating_key: key,
            title: row.title.trim().to_string(),
            artist: non_empty(&row.original_title).or_else(|| non_empty(&row.grandparent_title)),
            album: non_empty(&row.parent_title),
        });
    }
    Ok((out, short, read, data.filtered.map(|t| t.max(0) as u64)))
}

#[async_trait]
impl Feed for Tautulli {
    fn source(&self) -> ImportSource {
        ImportSource::Tautulli
    }

    async fn ready(&self) -> bool {
        self.status().ready()
    }

    async fn page(&self, page: u64, size: u64, since_ms: Option<i64>) -> AppResult<Page> {
        let url = self.url();
        let key = self.keys.get(KEY_ACCOUNT)?.ok_or_else(|| AppError::Invalid("Add Tautulli's API key first.".into()))?;
        let user = self
            .plex
            .username()
            .await
            .ok_or_else(|| AppError::Invalid("Sign in to Plex so Platter knows which plays in Tautulli are yours.".into()))?;
        let tag = self.plex.tag().await;
        let query = {
            let mut q = url::form_urlencoded::Serializer::new(String::new());
            q.append_pair("apikey", key.expose())
                .append_pair("cmd", "get_history")
                .append_pair("media_type", "track")
                .append_pair("user", &user)
                .append_pair("grouping", "0")
                .append_pair("order_column", "date")
                .append_pair("order_dir", "desc")
                .append_pair("start", &(page * size).to_string())
                .append_pair("length", &size.to_string());
            if let Some(s) = since_ms {
                // Whole days; plays already saved are skipped.
                let day = crate::stats::format_day((s / 1000).div_euclid(86_400) - 1);
                q.append_pair("after", &day);
            }
            q.finish()
        };
        let req = WebRequest::get(format!("{}/api/v2?{query}", url.trim_end_matches('/')));
        let resp = self.web.send(req).await.map_err(|_| AppError::Network(UNREACHABLE.into()))?;
        if resp.status == 401 || resp.status == 403 {
            return Err(AppError::Auth(BAD_KEY.into()));
        }
        if !resp.ok() && resp.status != 400 {
            return Err(AppError::Network(UNREACHABLE.into()));
        }
        let (plays, short_plays, read, total) = parse_history(&resp.body, &user, tag.as_deref())?;
        Ok(Page { plays, more: read as u64 >= size, total, short_plays })
    }
}
