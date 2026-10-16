//! The music section's play history on the connected Plex server
//! (`/status/sessions/history/all`), the signed-in user's plays only.
//!
//! Every history entry names the server account that played it. Only the
//! signed-in user's account is asked for (`accountID`), and every entry is
//! checked again, so another Home user's or a friend's plays never count.
//! When Platter can't tell which account is the user's, nothing is read.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Deserialize;

use super::store::Fetched;
use super::{Feed, Page};
use crate::db::DbHandle;
use crate::db::listens::ImportSource;
use crate::error::{AppError, AppResult};
use crate::plex::PlexState;
use crate::plex::lenient;

/// The owner's account id on their own server.
pub const OWNER_ACCOUNT: i64 = 1;

/// What the history import needs from the Plex connection.
#[async_trait]
pub trait PlexServer: Send + Sync {
    /// Connected to a Plex server with a music library.
    async fn ready(&self) -> bool;
    /// The signed-in user's account id on the server; None when it can't be told.
    async fn own_account(&self) -> AppResult<Option<i64>>;
    /// The signed-in user's Plex user name.
    async fn username(&self) -> Option<String>;
    /// The server's id tag (`crate::sid`).
    async fn tag(&self) -> Option<String>;
    /// One page of the music section's history for `account`, newest first,
    /// as the server answered (ids qualified).
    async fn history(&self, account: i64, start: u64, size: u64) -> AppResult<Vec<u8>>;
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "MediaContainer")]
    media_container: Container,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Container {
    #[serde(rename = "totalSize", deserialize_with = "lenient::opt_i64")]
    total_size: Option<i64>,
    #[serde(rename = "Metadata")]
    metadata: Vec<Entry>,
    #[serde(rename = "Account")]
    account: Vec<AccountRow>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Entry {
    #[serde(rename = "historyKey", deserialize_with = "lenient::string")]
    history_key: String,
    #[serde(deserialize_with = "lenient::string")]
    id: String,
    #[serde(rename = "ratingKey", deserialize_with = "lenient::string")]
    rating_key: String,
    #[serde(rename = "type", deserialize_with = "lenient::string")]
    kind: String,
    #[serde(deserialize_with = "lenient::string")]
    title: String,
    #[serde(rename = "grandparentTitle", deserialize_with = "lenient::string")]
    artist: String,
    #[serde(rename = "originalTitle", deserialize_with = "lenient::string")]
    original_title: String,
    #[serde(rename = "parentTitle", deserialize_with = "lenient::string")]
    album: String,
    #[serde(rename = "viewedAt", deserialize_with = "lenient::opt_i64")]
    viewed_at: Option<i64>,
    #[serde(rename = "accountID", deserialize_with = "lenient::opt_i64")]
    account_id: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AccountRow {
    #[serde(deserialize_with = "lenient::opt_i64")]
    id: Option<i64>,
    #[serde(deserialize_with = "lenient::string")]
    name: String,
}

fn container(body: &[u8], what: &str) -> AppResult<Container> {
    serde_json::from_slice::<Envelope>(body).map(|e| e.media_container).map_err(|e| AppError::Decode(format!("{what}: {e}")))
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// A page of history: `account`'s song plays (others are counted, never
/// kept), and the total the server reported.
pub fn parse_history(body: &[u8], account: i64) -> AppResult<(Vec<Fetched>, usize, Option<u64>, usize)> {
    let c = container(body, "/status/sessions/history/all")?;
    let read = c.metadata.len();
    let mut others = 0;
    let mut out = Vec::new();
    for e in c.metadata {
        if e.account_id != Some(account) {
            others += 1;
            continue;
        }
        let Some(at) = e.viewed_at.filter(|v| *v > 0) else {
            continue;
        };
        if (!e.kind.is_empty() && e.kind != "track") || e.title.trim().is_empty() {
            continue;
        }
        let (tag, _) = crate::sid::split(&e.rating_key);
        let own = e.history_key.rsplit('/').next().filter(|s| !s.is_empty()).map(str::to_string).or_else(|| non_empty(&e.id));
        let id = own.unwrap_or_else(|| format!("{account}.{}.{at}", e.rating_key));
        let source_id = match tag {
            Some(t) => format!("{t}:{id}"),
            None => id,
        };
        out.push(Fetched {
            source_id,
            played_at: at * 1000,
            ms_played: None,
            rating_key: non_empty(&e.rating_key),
            title: e.title.trim().to_string(),
            artist: non_empty(&e.original_title).or_else(|| non_empty(&e.artist)),
            album: non_empty(&e.album),
        });
    }
    Ok((out, read, c.total_size.map(|t| t.max(0) as u64), others))
}

/// The account in a server's `/accounts` answer named like the user.
pub fn account_named(body: &[u8], names: &[&str]) -> AppResult<Option<i64>> {
    let c = container(body, "/accounts")?;
    Ok(c.account
        .into_iter()
        .find(|a| names.iter().any(|n| !n.trim().is_empty() && a.name.trim().eq_ignore_ascii_case(n.trim())))
        .and_then(|a| a.id))
}

/// The Plex history feed.
pub struct PlexFeed {
    server: Arc<dyn PlexServer>,
    /// The account found at the start of this run.
    account: Mutex<Option<i64>>,
}

impl PlexFeed {
    pub fn new(server: Arc<dyn PlexServer>) -> Self {
        Self { server, account: Mutex::new(None) }
    }
}

#[async_trait]
impl Feed for PlexFeed {
    fn source(&self) -> ImportSource {
        ImportSource::PlexHistory
    }

    async fn ready(&self) -> bool {
        self.server.ready().await
    }

    async fn page(&self, page: u64, size: u64, _since_ms: Option<i64>) -> AppResult<Page> {
        let cached = *self.account.lock();
        let account = match cached {
            Some(a) if page > 0 => a,
            _ => {
                let a = self.server.own_account().await?.ok_or_else(|| {
                    AppError::Invalid("Platter can't tell which plays on your Plex server are yours, so none were brought in.".into())
                })?;
                *self.account.lock() = Some(a);
                a
            }
        };
        let body = self.server.history(account, page * size, size).await?;
        let (plays, read, total, others) = parse_history(&body, account)?;
        if others > 0 {
            tracing::debug!(others, "plex history: plays from other accounts left out");
        }
        Ok(Page { plays, more: read as u64 >= size, total, short_plays: 0 })
    }
}

/// The real server, through the app's Plex connection.
pub struct LivePlex {
    pub plex: PlexState,
    pub db: DbHandle,
}

#[async_trait]
impl PlexServer for LivePlex {
    async fn ready(&self) -> bool {
        self.plex.client().await.is_ok() && self.section().await.is_some()
    }

    async fn own_account(&self) -> AppResult<Option<i64>> {
        let status = self.plex.auth_status().await;
        let owned = status.server.as_ref().is_some_and(|s| s.owned);
        let c = self.plex.client().await?;
        let by_name = match (&status.username, c.get_bytes("/accounts", &[]).await) {
            (Some(name), Ok(body)) => account_named(&body, &[name]).unwrap_or(None),
            _ => None,
        };
        Ok(by_name.or(owned.then_some(OWNER_ACCOUNT)))
    }

    async fn username(&self) -> Option<String> {
        self.plex.auth_status().await.username.filter(|u| !u.trim().is_empty())
    }

    async fn tag(&self) -> Option<String> {
        self.plex.tag().await
    }

    async fn history(&self, account: i64, start: u64, size: u64) -> AppResult<Vec<u8>> {
        let section = self.section().await.ok_or_else(|| AppError::Invalid("no music library yet".into()))?;
        let q = [
            ("librarySectionID", section),
            ("accountID", account.to_string()),
            ("sort", "viewedAt:desc".to_string()),
            ("X-Plex-Container-Start", start.to_string()),
            ("X-Plex-Container-Size", size.to_string()),
        ];
        let c = self.plex.client().await?;
        Ok(c.qualify_json(&c.get_bytes("/status/sessions/history/all", &q).await?))
    }
}

impl LivePlex {
    async fn section(&self) -> Option<String> {
        self.db.read_async(crate::db::store::sync_state).await.ok().flatten().map(|s| s.section_id)
    }
}
