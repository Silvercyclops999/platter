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
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::merge::norm;
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
    /// The song in this library (None when it isn't here).
    pub track_id: Option<String>,
    pub title: String,
    pub artist: String,
    /// The artist in this library, if found.
    pub artist_id: Option<String>,
    /// The album in this library, if found.
    pub album_id: Option<String>,
    pub album: String,
    /// Art path of the album cover.
    pub cover: Option<String>,
    /// Unix seconds: when it was played (or now, while live).
    pub at: i64,
    /// Listening right now.
    pub now: bool,
    /// The song is in this library ([`place`] found it).
    pub in_library: bool,
    /// The album's artist and year, for finding it here by name.
    #[serde(skip)]
    pub album_artist: String,
    #[serde(skip)]
    pub year: Option<i64>,
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
    pub artist_id: Option<String>,
    pub album_artist: String,
    pub album_id: Option<String>,
    pub album: String,
    pub year: Option<i64>,
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
    pub artist_id: Option<String>,
    pub album_artist: String,
    pub album_id: Option<String>,
    pub album: String,
    pub year: Option<i64>,
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
    grandparent_rating_key: Option<String>,
    #[serde(default)]
    grandparent_key: Option<String>,
    #[serde(default)]
    parent_year: Option<i64>,
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
    if raw.starts_with('/') || crate::sid::tag_of(&raw).is_some() {
        return Some(raw);
    }
    raw.starts_with("https://").then(|| format!("{}{raw}", crate::artistinfo::art::EXT_PREFIX))
}

/// `/library/metadata/123` → `123`.
fn key_id(key: &str) -> Option<String> {
    let id = key.rsplit('/').next()?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string())
}

/// A parent's id: its rating key (already tagged by the Plex client), else
/// the id in its key path, tagged like the song (history rows only carry
/// `parentKey`, and Platter's ids name their server, crate::sid).
fn parent_id(m: &Meta, rating_key: &Option<String>, key: &Option<String>) -> Option<String> {
    if let Some(k) = rating_key.clone().filter(|k| !k.is_empty()) {
        return Some(k);
    }
    let id = key.as_deref().and_then(key_id)?;
    Some(match m.rating_key.as_deref().and_then(crate::sid::tag_of) {
        Some(t) => crate::sid::qualify(t, &id),
        None => crate::sid::resolve(&id),
    })
}

fn album_of(m: &Meta) -> Option<String> {
    parent_id(m, &m.parent_rating_key, &m.parent_key)
}

fn artist_id_of(m: &Meta) -> Option<String> {
    parent_id(m, &m.grandparent_rating_key, &m.grandparent_key)
}

fn album_artist_of(m: &Meta) -> String {
    non_empty(m.grandparent_title.clone()).unwrap_or_default()
}

fn artist_of(m: &Meta) -> String {
    non_empty(m.original_title.clone())
        .or_else(|| non_empty(m.grandparent_title.clone()))
        .unwrap_or_default()
}

fn parse(body: &[u8], what: &str) -> AppResult<Container> {
    crate::web::decode_json::<Envelope>(body)
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
                artist_id: artist_id_of(&m),
                album_artist: album_artist_of(&m),
                album_id: album_of(&m),
                album: m.parent_title.clone().unwrap_or_default(),
                year: m.parent_year.filter(|y| *y > 0),
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
                artist_id: artist_id_of(&m),
                album_artist: album_artist_of(&m),
                album_id: album_of(&m),
                album: m.parent_title.clone().unwrap_or_default(),
                year: m.parent_year.filter(|y| *y > 0),
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
            track_id: Some(l.track_id.clone()),
            title: l.title.clone(),
            artist: l.artist.clone(),
            artist_id: l.artist_id.clone(),
            album_id: l.album_id.clone(),
            album: l.album.clone(),
            cover: l.cover.clone(),
            at: now,
            now: true,
            in_library: false,
            album_artist: l.album_artist.clone(),
            year: l.year,
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
                    track_id: Some(p.track_id.clone()),
                    title: p.title.clone(),
                    artist: p.artist.clone(),
                    artist_id: p.artist_id.clone(),
                    album_id: p.album_id.clone(),
                    album: p.album.clone(),
                    cover: p.cover.clone(),
                    at: p.viewed_at.min(now),
                    now: false,
                    in_library: false,
                    album_artist: p.album_artist.clone(),
                    year: p.year,
                },
            );
        }
    }
    let mut out: Vec<FriendActivity> = rows.into_values().collect();
    out.sort_by(|a, b| b.now.cmp(&a.now).then(b.at.cmp(&a.at)).then(a.name.cmp(&b.name)));
    out
}

// ---- finding it in this library -------------------------------------------

fn exists(c: &Connection, table: &str, id: &str) -> AppResult<bool> {
    Ok(c.query_row(&format!("SELECT 1 FROM {table} WHERE id = ?1"), [id], |_| Ok(())).optional()?.is_some())
}

/// Years agree when either is unknown.
fn same_year(a: Option<i64>, b: Option<i64>) -> bool {
    a.is_none() || b.is_none() || a == b
}

struct TrackRow {
    id: String,
    title: String,
    album_id: String,
    album_title: String,
    artist_id: String,
    artist_title: String,
    track_artist: Option<String>,
    year: Option<i64>,
}

struct AlbumRow {
    id: String,
    title: String,
    artist_id: String,
    artist_title: String,
    year: Option<i64>,
}

/// The library's songs, albums and artists, read once when a row needs
/// finding by name.
#[derive(Default)]
struct ByName {
    tracks: Vec<TrackRow>,
    albums: Vec<AlbumRow>,
    artists: Vec<(String, String)>,
}

impl ByName {
    fn read(c: &Connection) -> AppResult<Self> {
        let mut st = c.prepare(
            "SELECT t.id, t.title, t.album_id, t.album_title, t.artist_id, t.artist_title, t.track_artist, a.year
             FROM tracks t LEFT JOIN albums a ON a.id = t.album_id ORDER BY t.id",
        )?;
        let tracks = st
            .query_map([], |r| {
                Ok(TrackRow {
                    id: r.get(0)?,
                    title: norm(&r.get::<_, String>(1)?),
                    album_id: r.get(2)?,
                    album_title: norm(&r.get::<_, String>(3)?),
                    artist_id: r.get(4)?,
                    artist_title: norm(&r.get::<_, String>(5)?),
                    track_artist: r.get::<_, Option<String>>(6)?.map(|a| norm(&a)),
                    year: r.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut st = c.prepare("SELECT id, title, artist_id, artist_title, year FROM albums ORDER BY id")?;
        let albums = st
            .query_map([], |r| {
                Ok(AlbumRow {
                    id: r.get(0)?,
                    title: norm(&r.get::<_, String>(1)?),
                    artist_id: r.get(2)?,
                    artist_title: norm(&r.get::<_, String>(3)?),
                    year: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut st = c.prepare("SELECT id, title FROM artists ORDER BY id")?;
        let artists = st.query_map([], |r| Ok((r.get::<_, String>(0)?, norm(&r.get::<_, String>(1)?))))?.collect::<Result<Vec<_>, _>>()?;
        Ok(Self { tracks, albums, artists })
    }

    /// The same song: title and artist, and the album and year when known.
    fn track(&self, f: &FriendActivity) -> Option<&TrackRow> {
        let (title, artist, album) = (norm(&f.title), norm(&f.artist), norm(&f.album));
        if title.is_empty() || artist.is_empty() {
            return None;
        }
        self.tracks.iter().find(|t| {
            t.title == title
                && (t.artist_title == artist || t.track_artist.as_deref() == Some(artist.as_str()))
                && (album.is_empty() || t.album_title == album)
                && same_year(t.year, f.year)
        })
    }

    /// The same album: title and artist (the album's, or the song's), and
    /// the year when known.
    fn album(&self, f: &FriendActivity) -> Option<&AlbumRow> {
        let title = norm(&f.album);
        let artists = [norm(&f.album_artist), norm(&f.artist)];
        if title.is_empty() {
            return None;
        }
        self.albums.iter().find(|a| a.title == title && artists.iter().any(|n| !n.is_empty() && *n == a.artist_title) && same_year(a.year, f.year))
    }

    fn artist(&self, name: &str) -> Option<&str> {
        let name = norm(name);
        (!name.is_empty()).then(|| self.artists.iter().find(|(_, t)| *t == name).map(|(id, _)| id.as_str()))?
    }
}

/// Point each row at this library: by id when the song (or album, or
/// artist) is here, as it is when it came from this library's server; else
/// by name (a play on another server, or ids this library doesn't have).
/// What isn't found isn't linked, and a song not found is marked not in
/// the library.
pub fn place(c: &Connection, rows: &mut [FriendActivity]) -> AppResult<()> {
    let mut by_name: Option<ByName> = None;
    for f in rows.iter_mut() {
        let here = match f.track_id.as_deref() {
            Some(id) => c
                .query_row("SELECT album_id, artist_id FROM tracks WHERE id = ?1", [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .optional()?,
            None => None,
        };
        if let Some((album, artist)) = here {
            f.in_library = true;
            f.album_id = Some(album);
            f.artist_id = Some(artist);
            continue;
        }
        f.track_id = None;
        f.in_library = false;
        if let Some(id) = f.album_id.take()
            && exists(c, "albums", &id)?
        {
            f.album_id = Some(id);
        }
        if let Some(id) = f.artist_id.take()
            && exists(c, "artists", &id)?
        {
            f.artist_id = Some(id);
        }
        let names = match &mut by_name {
            Some(n) => n,
            None => by_name.insert(ByName::read(c)?),
        };
        if let Some(t) = names.track(f) {
            f.track_id = Some(t.id.clone());
            f.in_library = true;
            f.album_id = Some(t.album_id.clone());
            f.artist_id = Some(t.artist_id.clone());
            continue;
        }
        if f.album_id.is_none()
            && let Some(a) = names.album(f)
        {
            f.album_id = Some(a.id.clone());
            f.artist_id.get_or_insert_with(|| a.artist_id.clone());
        }
        if f.artist_id.is_none() {
            f.artist_id = names.artist(&f.artist).or_else(|| names.artist(&f.album_artist)).map(str::to_string);
        }
    }
    Ok(())
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
