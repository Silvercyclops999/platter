//! Friend activity: who else on the Plex server is listening, from the
//! server's current sessions (`/status/sessions`, live) and its play history
//! (`/status/sessions/history/all`, the last 7 days). Only the server
//! owner's token can read either, so for anyone else the panel says activity
//! isn't available.
//!
//! The parsing and the row building are pure so they can be tested with
//! faked sessions and history.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// History older than this isn't shown.
pub const WINDOW_S: i64 = 7 * 24 * 3600;
/// History rows read at most per refresh (newest first).
pub const HISTORY_PAGE: u64 = 500;

/// One person's latest listen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendActivity {
    /// The server's account id for the person.
    pub account_id: i64,
    pub name: String,
    /// Art path for the round avatar (through the art cache), if any.
    pub thumb: Option<String>,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album_id: Option<String>,
    pub album: String,
    /// Art path of the album cover.
    pub cover: Option<String>,
    /// Unix seconds: when it was played (or now, while live).
    pub at: i64,
    /// Listening right now.
    pub now: bool,
}

/// What the panel can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityState {
    Ok,
    /// This account doesn't own the server, so Plex won't share it.
    NotAdmin,
    /// Show friend activity is off.
    Off,
    /// The server can't be reached (or Go offline is on).
    Offline,
}

/// `friend_activity`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPanel {
    pub state: ActivityState,
    pub friends: Vec<FriendActivity>,
}

impl ActivityPanel {
    pub fn empty(state: ActivityState) -> Self {
        Self { state, friends: Vec::new() }
    }
}

/// A song someone is playing now (one `/status/sessions` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRow {
    pub account_id: i64,
    pub user_name: Option<String>,
    pub user_thumb: Option<String>,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album_id: Option<String>,
    pub album: String,
    pub cover: Option<String>,
    pub playing: bool,
}

/// A play in the server's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayedRow {
    pub account_id: i64,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album_id: Option<String>,
    pub album: String,
    pub cover: Option<String>,
    pub viewed_at: i64,
}

/// A server account (`/accounts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub thumb: Option<String>,
}

// ---- Plex JSON ------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct Envelope {
    #[serde(default)]
    media_container: Container,
}

#[derive(Deserialize, Default)]
struct Container {
    #[serde(rename = "Metadata", default)]
    metadata: Vec<Meta>,
    #[serde(rename = "Account", default)]
    account: Vec<RawAccount>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Meta {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    rating_key: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    original_title: Option<String>,
    #[serde(default)]
    grandparent_title: Option<String>,
    #[serde(default)]
    parent_title: Option<String>,
    #[serde(default)]
    parent_rating_key: Option<String>,
    #[serde(default)]
    parent_key: Option<String>,
    #[serde(default)]
    parent_thumb: Option<String>,
    #[serde(default)]
    thumb: Option<String>,
    #[serde(default)]
    viewed_at: Option<i64>,
    #[serde(rename = "accountID", default)]
    account_id: Option<i64>,
    #[serde(rename = "User", default)]
    user: Option<RawUser>,
    #[serde(rename = "Player", default)]
    player: Option<RawPlayer>,
}

#[derive(Deserialize, Default)]
struct RawUser {
    #[serde(default, deserialize_with = "id_any")]
    id: Option<i64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    thumb: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawPlayer {
    #[serde(default)]
    state: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawAccount {
    #[serde(default, deserialize_with = "id_any")]
    id: Option<i64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    thumb: Option<String>,
}

/// Plex sends ids as numbers in some answers and strings in others.
fn id_any<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// An image the art cache can serve: a server path as it is, a plex.tv
/// picture through the outside-image route. Anything else is dropped.
pub fn art_path(raw: Option<String>) -> Option<String> {
    let raw = non_empty(raw)?;
    if raw.starts_with('/') {
        return Some(raw);
    }
    raw.starts_with("https://").then(|| format!("{}{raw}", crate::artistinfo::art::EXT_PREFIX))
}

/// `/library/metadata/123` → `123`.
fn key_id(key: &str) -> Option<String> {
    let id = key.rsplit('/').next()?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string())
}

fn album_of(m: &Meta) -> Option<String> {
    m.parent_rating_key.clone().filter(|k| !k.is_empty()).or_else(|| m.parent_key.as_deref().and_then(key_id))
}

fn artist_of(m: &Meta) -> String {
    non_empty(m.original_title.clone())
        .or_else(|| non_empty(m.grandparent_title.clone()))
        .unwrap_or_default()
}

fn parse(body: &[u8], what: &str) -> AppResult<Container> {
    serde_json::from_slice::<Envelope>(body)
        .map(|e| e.media_container)
        .map_err(|e| AppError::Decode(format!("{what}: {e}")))
}

/// Music sessions in a `/status/sessions` answer.
pub fn parse_sessions(body: &[u8]) -> AppResult<Vec<LiveRow>> {
    Ok(parse(body, "/status/sessions")?
        .metadata
        .into_iter()
        .filter(|m| m.kind == "track")
        .filter_map(|m| {
            let user = m.user.as_ref()?;
            let account_id = user.id?;
            Some(LiveRow {
                account_id,
                user_name: non_empty(user.title.clone()),
                user_thumb: art_path(user.thumb.clone()),
                track_id: m.rating_key.clone().filter(|k| !k.is_empty())?,
                title: m.title.clone(),
                artist: artist_of(&m),
                album_id: album_of(&m),
                album: m.parent_title.clone().unwrap_or_default(),
                cover: art_path(m.parent_thumb.clone().or_else(|| m.thumb.clone())),
                playing: m.player.as_ref().and_then(|p| p.state.as_deref()) != Some("paused"),
            })
        })
        .collect())
}

/// Track plays in a `/status/sessions/history/all` answer.
pub fn parse_history(body: &[u8]) -> AppResult<Vec<PlayedRow>> {
    Ok(parse(body, "/status/sessions/history/all")?
        .metadata
        .into_iter()
        .filter(|m| m.kind == "track")
        .filter_map(|m| {
            Some(PlayedRow {
                account_id: m.account_id?,
                track_id: m.rating_key.clone().filter(|k| !k.is_empty())?,
                title: m.title.clone(),
                artist: artist_of(&m),
                album_id: album_of(&m),
                album: m.parent_title.clone().unwrap_or_default(),
                cover: art_path(m.parent_thumb.clone().or_else(|| m.thumb.clone())),
                viewed_at: m.viewed_at?,
            })
        })
        .collect())
}

/// The server's accounts (`/accounts`). Unnamed ones are left out.
pub fn parse_accounts(body: &[u8]) -> AppResult<Vec<Account>> {
    Ok(parse(body, "/accounts")?
        .account
        .into_iter()
        .filter_map(|a| {
            Some(Account { id: a.id?, name: non_empty(a.name)?, thumb: art_path(a.thumb) })
        })
        .collect())
}

/// One row per person, newest first: a live session wins over history, and
/// history older than [`WINDOW_S`] is dropped. `own` (the viewer's account)
/// is never listed. People without a name are left out.
pub fn build_rows(live: &[LiveRow], played: &[PlayedRow], accounts: &[Account], own: Option<i64>, now: i64) -> Vec<FriendActivity> {
    let by_id: HashMap<i64, &Account> = accounts.iter().map(|a| (a.id, a)).collect();
    let mut rows: HashMap<i64, FriendActivity> = HashMap::new();
    for l in live {
        if Some(l.account_id) == own {
            continue;
        }
        let acct = by_id.get(&l.account_id);
        let Some(name) = l.user_name.clone().or_else(|| acct.map(|a| a.name.clone())) else { continue };
        let row = FriendActivity {
            account_id: l.account_id,
            name,
            thumb: l.user_thumb.clone().or_else(|| acct.and_then(|a| a.thumb.clone())),
            track_id: l.track_id.clone(),
            title: l.title.clone(),
            artist: l.artist.clone(),
            album_id: l.album_id.clone(),
            album: l.album.clone(),
            cover: l.cover.clone(),
            at: now,
            now: true,
        };
        // Two sessions for one person: the one playing wins.
        match rows.get(&l.account_id) {
            Some(_) if !l.playing => {}
            _ => {
                rows.insert(l.account_id, row);
            }
        }
    }
    for p in played {
        if Some(p.account_id) == own || rows.contains_key(&p.account_id) && rows[&p.account_id].now {
            continue;
        }
        if now - p.viewed_at > WINDOW_S {
            continue;
        }
        let Some(acct) = by_id.get(&p.account_id) else { continue };
        let newer = rows.get(&p.account_id).is_none_or(|r| p.viewed_at > r.at);
        if newer {
            rows.insert(
                p.account_id,
                FriendActivity {
                    account_id: p.account_id,
                    name: acct.name.clone(),
                    thumb: acct.thumb.clone(),
                    track_id: p.track_id.clone(),
                    title: p.title.clone(),
                    artist: p.artist.clone(),
                    album_id: p.album_id.clone(),
                    album: p.album.clone(),
                    cover: p.cover.clone(),
                    at: p.viewed_at.min(now),
                    now: false,
                },
            );
        }
    }
    let mut out: Vec<FriendActivity> = rows.into_values().collect();
    out.sort_by(|a, b| b.now.cmp(&a.now).then(b.at.cmp(&a.at)).then(a.name.cmp(&b.name)));
    out
}

// ---- reading the server ---------------------------------------------------

/// Where activity comes from (the connected Plex server in the app).
#[async_trait]
pub trait ActivitySource: Send + Sync {
    /// Whether the signed-in account owns the server. None: not connected.
    async fn owned(&self) -> Option<bool>;
    async fn sessions(&self) -> AppResult<Vec<u8>>;
    async fn history(&self, size: u64) -> AppResult<Vec<u8>>;
    async fn accounts(&self) -> AppResult<Vec<u8>>;
}

/// The owner's own account id on their server.
pub const OWNER_ACCOUNT: i64 = 1;

/// Read and build the panel. Refused reads (401/403) mean no admin rights.
pub async fn read(src: &dyn ActivitySource, now: i64) -> ActivityPanel {
    let owned = match src.owned().await {
        None => return ActivityPanel::empty(ActivityState::Offline),
        Some(false) => return ActivityPanel::empty(ActivityState::NotAdmin),
        Some(true) => true,
    };
    let refused = |e: &AppError| matches!(e, AppError::Auth(_));
    let (sessions, history, accounts) = tokio::join!(src.sessions(), src.history(HISTORY_PAGE), src.accounts());
    let (sessions, history) = match (sessions, history) {
        (Ok(s), Ok(h)) => (s, h),
        (Err(e), _) | (_, Err(e)) if refused(&e) => return ActivityPanel::empty(ActivityState::NotAdmin),
        (Err(e), _) | (_, Err(e)) => {
            tracing::debug!(kind = e.kind(), "friend activity unavailable");
            return ActivityPanel::empty(ActivityState::Offline);
        }
    };
    let live = parse_sessions(&sessions).unwrap_or_default();
    let played = parse_history(&history).unwrap_or_default();
    let accounts = accounts.ok().and_then(|b| parse_accounts(&b).ok()).unwrap_or_default();
    let own = owned.then_some(OWNER_ACCOUNT);
    ActivityPanel { state: ActivityState::Ok, friends: build_rows(&live, &played, &accounts, own, now) }
}
