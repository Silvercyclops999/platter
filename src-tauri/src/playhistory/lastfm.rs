//! Last.fm: the scrobbles of the account connected for scrobbling
//! (`user.getRecentTracks`), read a page at a time with a pause between
//! pages. Scrobbles are matched to library songs by artist, title and
//! album; the ones that don't match are kept by their names (top artists
//! and the like still count them).

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use super::store::Fetched;
use super::{Feed, Page};
use crate::db::DbHandle;
use crate::db::listens::ImportSource;
use crate::db::store::kv_get;
use crate::error::{AppError, AppResult};
use crate::identity::fold;
use crate::plex::keychain::TokenStore;
use crate::scrobble::lastfm::{API, KEY_ACCOUNT, SESSION_ACCOUNT};
use crate::web::{Web, WebRequest};

pub struct LastfmFeed {
    pub web: Arc<dyn Web>,
    pub keys: Arc<dyn TokenStore>,
    pub db: DbHandle,
}

const BAD_KEY: &str = "Last.fm didn't accept your API key. Check it in Settings > Integrations.";
const UNREACHABLE: &str = "Couldn't reach Last.fm. Platter will try again later.";

#[derive(Deserialize)]
struct Reply {
    recenttracks: Recent,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Recent {
    track: OneOrMany,
    #[serde(rename = "@attr")]
    attr: Option<Attr>,
}

/// Last.fm sends one track as an object, several as a list.
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    Many(Vec<Scrobble>),
    One(Box<Scrobble>),
}

impl Default for OneOrMany {
    fn default() -> Self {
        OneOrMany::Many(Vec::new())
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Attr {
    #[serde(rename = "totalPages")]
    total_pages: Text,
    total: Text,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Scrobble {
    name: String,
    artist: Named,
    album: Named,
    date: Option<Date>,
    #[serde(rename = "@attr")]
    attr: Option<NowPlaying>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Named {
    #[serde(rename = "#text")]
    text: String,
    name: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Date {
    uts: Text,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct NowPlaying {
    nowplaying: Text,
}

/// Numbers come as strings.
#[derive(Deserialize, Default)]
#[serde(untagged)]
enum Text {
    S(String),
    N(i64),
    #[default]
    None,
}

impl Text {
    fn num(&self) -> Option<i64> {
        match self {
            Text::S(s) => s.trim().parse().ok(),
            Text::N(n) => Some(*n),
            Text::None => None,
        }
    }
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// A `user.getRecentTracks` page: the scrobbles (the one playing now has no
/// time yet and is left out), whether more pages follow, and the total.
pub fn parse_recent(body: &[u8], page: u64) -> AppResult<(Vec<Fetched>, bool, Option<u64>)> {
    let reply: Reply = serde_json::from_slice(body).map_err(|e| AppError::Decode(format!("Last.fm recent tracks: {e}")))?;
    let r = reply.recenttracks;
    let list = match r.track {
        OneOrMany::Many(v) => v,
        OneOrMany::One(s) => vec![*s],
    };
    let mut out = Vec::with_capacity(list.len());
    for s in list {
        if s.attr.as_ref().is_some_and(|a| matches!(&a.nowplaying, Text::S(v) if v == "true")) {
            continue;
        }
        let Some(at) = s.date.as_ref().and_then(|d| d.uts.num()).filter(|v| *v > 0) else {
            continue;
        };
        let artist = non_empty(&s.artist.text).or_else(|| non_empty(&s.artist.name));
        let Some(title) = non_empty(&s.name) else {
            continue;
        };
        // Last.fm has no id for a scrobble; its time and song name it.
        let id = format!("{at}:{}:{}", fold::fold_artist(artist.as_deref().unwrap_or("")), title.to_lowercase());
        out.push(Fetched {
            source_id: id,
            played_at: at * 1000,
            ms_played: None,
            rating_key: None,
            title,
            artist,
            album: non_empty(&s.album.text).or_else(|| non_empty(&s.album.name)),
        });
    }
    let attr = r.attr.unwrap_or_default();
    let pages = attr.total_pages.num().unwrap_or(0).max(0) as u64;
    let total = attr.total.num().map(|t| t.max(0) as u64);
    Ok((out, page + 1 < pages, total))
}

#[derive(Deserialize)]
struct ErrorReply {
    error: i64,
}

impl LastfmFeed {
    fn user(&self) -> Option<String> {
        self.db.read(|c| kv_get(c, crate::scrobble::commands::KEY_LASTFM_USER)).ok().flatten().filter(|u| !u.trim().is_empty())
    }
}

#[async_trait]
impl Feed for LastfmFeed {
    fn source(&self) -> ImportSource {
        ImportSource::Lastfm
    }

    /// Connected for scrobbling: a session, the API key and the user name.
    async fn ready(&self) -> bool {
        let has = |a: &str| self.keys.get(a).ok().flatten().is_some();
        has(SESSION_ACCOUNT) && has(KEY_ACCOUNT) && self.user().is_some()
    }

    async fn page(&self, page: u64, size: u64, since_ms: Option<i64>) -> AppResult<Page> {
        let key = self.keys.get(KEY_ACCOUNT)?.ok_or_else(|| AppError::Invalid("Add your Last.fm API key first.".into()))?;
        let user = self.user().ok_or_else(|| AppError::Invalid("Connect Last.fm in Settings first.".into()))?;
        let query = {
            let mut q = url::form_urlencoded::Serializer::new(String::new());
            q.append_pair("method", "user.getrecenttracks")
                .append_pair("user", &user)
                .append_pair("api_key", key.expose())
                .append_pair("format", "json")
                .append_pair("limit", &size.min(200).to_string())
                .append_pair("page", &(page + 1).to_string());
            if let Some(s) = since_ms {
                q.append_pair("from", &(s / 1000 + 1).to_string());
            }
            q.finish()
        };
        let resp = self.web.send(WebRequest::get(format!("{API}?{query}"))).await.map_err(|_| AppError::Network(UNREACHABLE.into()))?;
        if let Ok(e) = serde_json::from_slice::<ErrorReply>(&resp.body) {
            return Err(match e.error {
                10 | 26 => AppError::Auth(BAD_KEY.into()),
                17 => AppError::Auth(
                    "Last.fm keeps your listening private, so Platter can't read it. Make it public on last.fm to bring it in.".into(),
                ),
                6 => AppError::Auth("Last.fm doesn't know your account any more. Connect Last.fm again in Settings.".into()),
                _ => AppError::Network(UNREACHABLE.into()),
            });
        }
        if !resp.ok() {
            return Err(AppError::Network(UNREACHABLE.into()));
        }
        let (plays, more, total) = parse_recent(&resp.body, page)?;
        Ok(Page { plays, more, total, short_plays: 0 })
    }
}
