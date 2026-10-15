//! Following artists, What's New, and the new-release announcement.
//!
//! Plex has no follow, so Platter keeps its own list behind [`FollowStore`]:
//! on this computer for now ([`KvFollows`]), so a synced store can take over
//! later without the rest changing. What's New lists albums added to the
//! library by followed artists (or everything recently added while nothing
//! is followed), newest first, split into New (added since the page was
//! last seen) and Earlier.

use std::collections::HashSet;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::db::DbHandle;
use crate::db::popular;
use crate::db::store::{ALBUM_COLS, kv_get, kv_set, row_album};
use crate::error::{AppError, AppResult};
use crate::model::Album;

/// Albums older than this aren't in What's New.
pub const WINDOW_S: i64 = 90 * 24 * 3600;
/// Most releases the page lists.
pub const MAX_ITEMS: usize = 60;
/// Most artists one person can follow (keeps the stored list small).
pub const MAX_FOLLOWS: usize = 5000;

const KEY_FOLLOWS: &str = "social.follows.v1";
const KEY_SEEN: &str = "social.whatsnew.seen.v1";
const KEY_ANNOUNCED: &str = "social.whatsnew.announced.v1";

/// An artist the user follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowedArtist {
    /// A library artist id, or a credited artist's `credit:` id.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub thumb: Option<String>,
    /// Unix seconds.
    pub followed_at: i64,
}

/// Where the follow list lives.
pub trait FollowStore: Send + Sync {
    fn list(&self) -> AppResult<Vec<FollowedArtist>>;
    fn save(&self, all: &[FollowedArtist]) -> AppResult<()>;
}

/// The follow list in this computer's database.
pub struct KvFollows(pub DbHandle);

impl FollowStore for KvFollows {
    fn list(&self) -> AppResult<Vec<FollowedArtist>> {
        let raw = self.0.read(|c| kv_get(c, KEY_FOLLOWS))?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    fn save(&self, all: &[FollowedArtist]) -> AppResult<()> {
        let json = serde_json::to_string(all).map_err(|e| AppError::Internal(e.to_string()))?;
        self.0.write(|c| kv_set(c, KEY_FOLLOWS, &json))
    }
}

fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 300
}

/// Follow or unfollow. Following again keeps the first date.
pub fn set_follow(store: &dyn FollowStore, artist: FollowedArtist, on: bool) -> AppResult<Vec<FollowedArtist>> {
    if !valid_id(&artist.id) {
        return Err(AppError::Invalid("Not an artist.".into()));
    }
    let mut all = store.list()?;
    let had = all.iter().any(|f| f.id == artist.id);
    match (on, had) {
        (true, false) => {
            if all.len() >= MAX_FOLLOWS {
                return Err(AppError::Invalid("You follow as many artists as Platter can keep.".into()));
            }
            all.push(artist);
        }
        (false, true) => all.retain(|f| f.id != artist.id),
        _ => return Ok(all),
    }
    store.save(&all)?;
    Ok(all)
}

// ---- What's New -------------------------------------------------------------

/// One row on What's New.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewRelease {
    pub album: Album,
    /// Added since What's New was last seen.
    pub new: bool,
}

/// `whats_new`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNew {
    /// Some artist is followed (otherwise the list is everything recently added).
    pub following: bool,
    pub items: Vec<NewRelease>,
    /// How many are new.
    pub unseen: u32,
}

/// Albums by these artists added at or after `since`, newest first.
/// Credited artists bring the albums of songs that credit them.
pub fn releases(c: &Connection, follows: &[FollowedArtist], since: i64, limit: usize) -> AppResult<Vec<Album>> {
    releases_from(c, follows, since, limit, false)
}

/// [`releases`]; with `after_follow`, only albums added after each artist
/// was followed count.
fn releases_from(c: &Connection, follows: &[FollowedArtist], since: i64, limit: usize, after_follow: bool) -> AppResult<Vec<Album>> {
    let mut out: Vec<Album> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if follows.is_empty() {
        let mut st = c.prepare_cached(&format!(
            "SELECT {ALBUM_COLS} FROM albums al WHERE al.added_at >= ?1 ORDER BY al.added_at DESC, al.pk DESC LIMIT ?2"
        ))?;
        return Ok(st.query_map(params![since, limit as i64], row_album)?.collect::<Result<Vec<_>, _>>()?);
    }
    let mut by_artist = c.prepare_cached(&format!(
        "SELECT {ALBUM_COLS} FROM albums al WHERE al.artist_id = ?1 AND al.added_at >= ?2 ORDER BY al.added_at DESC LIMIT ?3"
    ))?;
    let mut by_id = c.prepare_cached(&format!("SELECT {ALBUM_COLS} FROM albums al WHERE al.id = ?1 AND al.added_at >= ?2"))?;
    for f in follows {
        let since = if after_follow { since.max(f.followed_at + 1) } else { since };
        if let Some(key) = popular::credit_key(&f.id) {
            let ids: HashSet<String> = popular::credited_tracks(c, key, None)?.into_iter().map(|t| t.album_id).collect();
            for id in ids {
                if let Some(a) = by_id.query_map(params![id, since], row_album)?.next().transpose()?
                    && seen.insert(a.id.clone())
                {
                    out.push(a);
                }
            }
        } else {
            for a in by_artist.query_map(params![f.id, since, limit as i64], row_album)? {
                let a = a?;
                if seen.insert(a.id.clone()) {
                    out.push(a);
                }
            }
        }
    }
    out.sort_by(|a, b| b.added_at.cmp(&a.added_at).then(a.title.cmp(&b.title)));
    out.truncate(limit);
    Ok(out)
}

/// Split into New and Earlier by the last time the page was seen.
pub fn whats_new(albums: Vec<Album>, following: bool, seen_at: i64) -> WhatsNew {
    let items: Vec<NewRelease> = albums.into_iter().map(|a| NewRelease { new: a.added_at > seen_at, album: a }).collect();
    let unseen = items.iter().filter(|i| i.new).count() as u32;
    WhatsNew { following, items, unseen }
}

fn kv_i64(c: &Connection, key: &str) -> AppResult<Option<i64>> {
    Ok(kv_get(c, key)?.and_then(|s| s.parse().ok()))
}

/// When What's New was last seen. The first time it's asked, that's now, so
/// a new install doesn't start with a dot for everything already there.
pub fn seen_at(db: &DbHandle, now: i64) -> AppResult<i64> {
    if let Some(v) = db.read(|c| kv_i64(c, KEY_SEEN))? {
        return Ok(v);
    }
    db.write(|c| kv_set(c, KEY_SEEN, &now.to_string()))?;
    Ok(now)
}

pub fn mark_seen(db: &DbHandle, now: i64) -> AppResult<()> {
    db.write(|c| kv_set(c, KEY_SEEN, &now.to_string()))
}

/// The page, as it is now.
pub fn load(db: &DbHandle, store: &dyn FollowStore, now: i64) -> AppResult<WhatsNew> {
    let follows = store.list()?;
    let seen = seen_at(db, now)?;
    let albums = db.read(|c| releases(c, &follows, now - WINDOW_S, MAX_ITEMS))?;
    Ok(whats_new(albums, !follows.is_empty(), seen))
}

// ---- announcements ------------------------------------------------------------

/// What a new-release check found.
#[derive(Debug, Clone, PartialEq)]
pub struct Announcement {
    pub albums: Vec<Album>,
    /// The mark to store: the newest album announced (or the old mark).
    pub mark: i64,
}

/// Albums by followed artists added after `mark` (the newest album already
/// announced) and after the artist was followed. Nothing is followed:
/// nothing to announce.
pub fn to_announce(c: &Connection, follows: &[FollowedArtist], mark: i64) -> AppResult<Announcement> {
    if follows.is_empty() {
        return Ok(Announcement { albums: Vec::new(), mark });
    }
    let albums = releases_from(c, follows, mark + 1, MAX_ITEMS, true)?;
    let newest = albums.iter().map(|a| a.added_at).max().unwrap_or(mark).max(mark);
    Ok(Announcement { albums, mark: newest })
}

/// The notification for new releases: (title, body).
pub fn notice(albums: &[Album]) -> Option<(String, String)> {
    match albums {
        [] => None,
        [a] => Some((format!("New from {}", a.artist_title), a.title.clone())),
        [a, rest @ ..] => {
            let artists: HashSet<&str> = albums.iter().map(|a| a.artist_title.as_str()).collect();
            if artists.len() == 1 {
                Some((format!("New from {}", a.artist_title), format!("{} and {} more", a.title, rest.len())))
            } else {
                Some(("New releases".into(), format!("{} new releases from artists you follow", albums.len())))
            }
        }
    }
}

/// Run one check: find what's new since the last announcement and move the
/// mark. The first check only sets the mark (to now).
pub fn check(db: &DbHandle, store: &dyn FollowStore, now: i64) -> AppResult<Vec<Album>> {
    let Some(mark) = db.read(|c| kv_i64(c, KEY_ANNOUNCED))? else {
        db.write(|c| kv_set(c, KEY_ANNOUNCED, &now.to_string()))?;
        return Ok(Vec::new());
    };
    let follows = store.list()?;
    let found = db.read(|c| to_announce(c, &follows, mark))?;
    if found.mark != mark {
        db.write(|c| kv_set(c, KEY_ANNOUNCED, &found.mark.to_string()))?;
    }
    Ok(found.albums)
}
