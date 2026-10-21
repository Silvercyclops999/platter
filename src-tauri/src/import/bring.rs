//! Bringing music over from another app: one door, three steps.
//!
//! 1. Where from: [`read`] works out what a dropped or picked file is
//!    (`sources::detect`) and imports it the usual way (`store`). The source
//!    is then remembered as brought over ([`mark_brought`]); a source read
//!    another way (a connected account) is brought over the same way.
//! 2. What you already have: [`summary`], one sentence and two numbers.
//!    [`plan`] is what goes to the server at once: liked songs found become
//!    likes, and each playlist is made with the songs found. Missing songs
//!    show greyed until their album arrives. Unsure matches wait in the
//!    review queue and block nothing.
//! 3. Get the rest: [`albums`] are the missing songs' albums (and saved
//!    albums the library lacks), most loved first. Getting them has
//!    MusicBrainz pick each one ([`Bring::pick_pending`]) and puts it on the
//!    Get albums queue, which asks Lidarr a few a day (`lidarr::waves`), away
//!    from home through a home side or the Bridge like any request. An album
//!    that can't be found stays on the list with "Lidarr couldn't find it".
//!
//! As albums arrive ([`rematch`] after the library changes), the songs that
//! were liked are liked and the playlist songs are added, each only once and
//! never again after the user takes one out (settings `bringLikeArrivals`
//! and `bringFillPlaylists`). A newer file for the same source updates the
//! same playlists.
//!
//! Unlike the rest of import, this writes to the server without a further
//! click: bringing music over is the click.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::matching::{Matcher, Status, album_found, tier_name};
use super::{SongIn, files, played, sources, store};
use crate::db::DbHandle;
use crate::db::merge::Rank;
use crate::error::{AppError, AppResult};
use crate::identity::fold::{fold_album, fold_artist};
use crate::lidarr::waves::QueuedGet;

/// Where step 1 says to request a file: a streaming service's privacy page
/// (account data) and Google Takeout. The page opens these and no others.
pub const REQUEST_PAGES: [&str; 2] = ["https://www.spotify.com/account/privacy/", "https://takeout.google.com/"];

/// What the page says for an album nothing could be found for.
pub const NOT_FOUND: &str = "Lidarr couldn't find it";
/// How often the watcher looks for arrivals and albums to pick while Platter is open.
pub const LOOK_EVERY: Duration = Duration::from_secs(10 * 60);
/// Albums picked in one go at most (MusicBrainz is asked once a second).
const PICKS_PER_PASS: usize = 50;

/// Where an imported list's liked songs go.
pub fn liked_target(source: &str) -> String {
    format!("liked:{source}")
}

/// Where an imported playlist goes, by its source and folded name.
pub fn list_target(source: &str, name_key: &str) -> String {
    format!("pl:{source}:{name_key}")
}

/// The key an album is kept under: folded artist and title.
pub fn album_key(artist: &str, album: &str) -> String {
    format!("{}\u{1f}{}", fold_artist(artist), fold_album(album))
}

// ------------------------------------------------------------------ step 1

/// Reads, matches and stores what's at `path`, whatever it is (blocking).
/// Returns the source it came in as.
pub fn read(db: &DbHandle, path: &std::path::Path, now_ms: i64) -> AppResult<String> {
    let input = files::read_input(path)?;
    if input.is_empty() {
        return Err(AppError::Invalid(sources::UNKNOWN_FILE.into()));
    }
    let (src, parsed) = sources::detect(&input)?;
    drop(input);
    if parsed.is_empty() {
        return Err(AppError::Invalid(super::commands::nothing_usable(&parsed.skipped)));
    }
    let id = src.id;
    let prepared = db.read(|c| store::prepare(c, id, parsed, db.source_ranks()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    // A data download or a library file holds every playlist the source has.
    let whole = matches!(id, "streaming" | "apple");
    db.write(|c| store::write(c, id, name.as_deref(), prepared, whole, now_ms))?;
    let ranks = db.source_ranks();
    db.write(|c| {
        played::keep_up(c, ranks, now_ms);
        mark_brought(c, id, now_ms)
    })?;
    Ok(id.to_string())
}

/// Remember a source as brought over: its likes and playlists go to the server.
pub fn mark_brought(c: &Connection, source: &str, now_ms: i64) -> AppResult<()> {
    c.execute(
        "INSERT INTO import_bring_sources (source, brought_at) VALUES (?1, ?2)
         ON CONFLICT (source) DO UPDATE SET brought_at = excluded.brought_at",
        params![source, now_ms],
    )?;
    Ok(())
}

/// Sources brought over, oldest first.
pub fn brought(c: &Connection) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached("SELECT source FROM import_bring_sources ORDER BY brought_at, source")?;
    Ok(st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
}

/// Forget what bringing a source over kept (`store::forget` calls it).
pub fn forget(c: &Connection, source: &str) -> AppResult<()> {
    c.execute("DELETE FROM import_bring_sources WHERE source = ?1", [source])?;
    let lists = format!("pl:{source}:");
    let len = lists.len() as i64;
    c.execute("DELETE FROM import_bring_lists WHERE substr(target, 1, ?2) = ?1", params![lists, len])?;
    c.execute("DELETE FROM import_bring_applied WHERE substr(target, 1, ?2) = ?1 OR target = ?3", params![lists, len, liked_target(source)])?;
    // Albums asked for stay: Lidarr has them on its list either way.
    Ok(())
}

// ------------------------------------------------------------------ step 2

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub total: u64,
    pub found: u64,
}

/// What the files brought, against the library.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub sources: Vec<String>,
    pub liked: Counts,
    /// Every song the likes and playlists name, once each.
    pub songs: Counts,
    pub playlists: u64,
    pub missing_songs: u64,
    /// Albums to get (those of missing songs, and saved albums the library lacks).
    pub missing_albums: u64,
    /// Songs waiting in the review queue.
    pub unsure: u64,
    pub sentence: String,
}

/// "1,234" for 1234.
pub fn n(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn plural(v: u64, one: &str, many: &str) -> String {
    format!("{} {}", n(v), if v == 1 { one } else { many })
}

/// "812 of 900 liked songs are already in your library. 88 songs from 31
/// albums aren't."
pub fn sentence(s: &Summary) -> String {
    let (have, total, what, one) = if s.liked.total > 0 {
        (s.liked.found, s.liked.total, "liked songs", "liked song")
    } else {
        (s.songs.found, s.songs.total, "songs", "song")
    };
    let mut out = if total == 0 {
        String::new()
    } else if total == 1 {
        if have == 1 { format!("Your one {one} is already in your library.") } else { format!("Your one {one} isn't in your library yet.") }
    } else if have == total {
        format!("All {} {what} are already in your library.", n(total))
    } else if have == 0 {
        format!("None of the {} {what} are in your library yet.", n(total))
    } else {
        format!("{} of {} {what} are already in your library.", n(have), n(total))
    };
    let tail = if s.missing_songs > 0 {
        let verb = if s.missing_songs == 1 { "isn't" } else { "aren't" };
        if s.missing_albums > 0 {
            format!("{} from {} {verb}.", plural(s.missing_songs, "song", "songs"), plural(s.missing_albums, "album", "albums"))
        } else {
            format!("{} {verb}.", plural(s.missing_songs, "song", "songs"))
        }
    } else if s.missing_albums > 0 {
        format!("{} you saved {}.", plural(s.missing_albums, "album", "albums"), if s.missing_albums == 1 { "isn't" } else { "aren't" })
    } else {
        String::new()
    };
    if total == 0 && tail.is_empty() {
        return "Those files had no liked songs or playlists.".into();
    }
    if !out.is_empty() && !tail.is_empty() {
        out.push(' ');
    }
    out.push_str(&tail);
    out
}

const BROUGHT: &str = "(SELECT source FROM import_bring_sources)";

/// The songs a brought source's likes or playlists want.
const WANTED: &str = "(EXISTS (SELECT 1 FROM import_likes l WHERE l.song_id = s.id)
                      OR EXISTS (SELECT 1 FROM import_playlist_items i WHERE i.song_id = s.id))";

/// Step 2's numbers, for everything brought over.
pub fn summary(c: &Connection) -> AppResult<Summary> {
    let mut s = Summary { sources: brought(c)?, ..Default::default() };
    s.liked = c.query_row(
        &format!(
            "SELECT count(*), count(CASE WHEN s.status = 'matched' THEN 1 END)
             FROM import_likes l JOIN import_songs s ON s.id = l.song_id WHERE l.source IN {BROUGHT}"
        ),
        [],
        |r| Ok(Counts { total: r.get::<_, i64>(0)? as u64, found: r.get::<_, i64>(1)? as u64 }),
    )?;
    let mut st = c.prepare_cached(&format!(
        "SELECT status, reviewed, count(*) FROM import_songs s WHERE s.source IN {BROUGHT} AND {WANTED} GROUP BY status, reviewed"
    ))?;
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, i64>(2)? as u64)))? {
        let (status, reviewed, count) = r?;
        s.songs.total += count;
        match Status::parse(&status) {
            Status::Matched => s.songs.found += count,
            Status::Unsure if !reviewed => s.unsure += count,
            Status::Unsure => {}
            Status::Missing => s.missing_songs += count,
        }
    }
    s.playlists = c.query_row(&format!("SELECT count(*) FROM import_playlists WHERE source IN {BROUGHT}"), [], |r| r.get::<_, i64>(0))? as u64;
    s.missing_albums = albums(c)?.iter().filter(|a| a.state != AlbumState::Here).count() as u64;
    s.sentence = sentence(&s);
    Ok(s)
}

/// One playlist brought over, as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroughtList {
    pub name: String,
    /// The imported list (`pl:<n>`), for its page with the greyed rows.
    pub import_id: String,
    /// The server playlist it became; None until a song of it is found.
    pub playlist_id: Option<String>,
    /// The user deleted the server playlist: it isn't filled any more.
    pub gone: bool,
    pub total: u64,
    pub found: u64,
    /// The server playlist's cover.
    pub thumb: Option<String>,
}

pub fn lists(c: &Connection) -> AppResult<Vec<BroughtList>> {
    let mut st = c.prepare_cached(&format!(
        "SELECT p.id, p.source, p.name, p.name_key, count(i.song_id), count(CASE WHEN s.status = 'matched' THEN 1 END)
         FROM import_playlists p LEFT JOIN import_playlist_items i ON i.playlist_id = p.id LEFT JOIN import_songs s ON s.id = i.song_id
         WHERE p.source IN {BROUGHT} GROUP BY p.id ORDER BY lower(p.name), p.id"
    ))?;
    type Raw = (i64, String, String, String, i64, i64);
    let raw: Vec<Raw> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::with_capacity(raw.len());
    for (id, source, name, key, total, found) in raw {
        let made: Option<(Option<String>, bool)> = c
            .query_row("SELECT playlist_id, gone FROM import_bring_lists WHERE target = ?1", [list_target(&source, &key)], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let (playlist_id, gone) = made.unwrap_or((None, false));
        let playlist_id = match playlist_id {
            Some(p) => Some(crate::db::playlists::resolve_id(c, &p)?),
            None => None,
        };
        let thumb: Option<String> = match &playlist_id {
            Some(p) => c.query_row("SELECT thumb FROM playlists WHERE id = ?1", [p], |r| r.get(0)).optional()?.flatten(),
            None => None,
        };
        out.push(BroughtList { name, import_id: format!("pl:{id}"), playlist_id, gone, total: total as u64, found: found as u64, thumb });
    }
    Ok(out)
}

/// The missing songs of a server playlist that was brought over, in the
/// imported list's order, for its greyed "not in your library" rows.
pub fn waiting_rows(c: &Connection, playlist_id: &str) -> AppResult<Vec<store::ImportedRow>> {
    let mut st = c.prepare_cached("SELECT target, playlist_id FROM import_bring_lists WHERE playlist_id IS NOT NULL AND gone = 0")?;
    let rows: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let want = crate::db::playlists::resolve_id(c, playlist_id)?;
    let mut target = None;
    for (t, p) in rows {
        if crate::db::playlists::resolve_id(c, &p)? == want {
            target = Some(t);
            break;
        }
    }
    let Some(target) = target else { return Ok(Vec::new()) };
    let Some((source, key)) = target.strip_prefix("pl:").and_then(|t| t.split_once(':')) else { return Ok(Vec::new()) };
    let mut st = c.prepare_cached(
        "SELECT s.id, s.artist, s.title, s.album, s.duration_ms
         FROM import_playlists p JOIN import_playlist_items i ON i.playlist_id = p.id JOIN import_songs s ON s.id = i.song_id
         WHERE p.source = ?1 AND p.name_key = ?2 AND s.status <> 'matched' ORDER BY i.position",
    )?;
    let mut seen = HashSet::new();
    let rows = st.query_map(params![source, key], |r| {
        Ok(store::ImportedRow {
            song_id: r.get(0)?,
            artist: r.get(1)?,
            title: r.get(2)?,
            album: r.get(3)?,
            duration_ms: r.get::<_, Option<i64>>(4)?.map(|d| d as u64),
            status: Status::Missing,
            track: None,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        let r = r?;
        if seen.insert(r.song_id) {
            out.push(r);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ what goes to the server

/// One server playlist to make or fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPlan {
    pub target: String,
    pub name: String,
    /// None: make it.
    pub playlist_id: Option<String>,
    pub add: Vec<String>,
}

/// What goes to the server now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Songs to like, by target (a source's liked list).
    pub likes: Vec<(String, Vec<String>)>,
    pub lists: Vec<ListPlan>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.likes.iter().all(|(_, v)| v.is_empty()) && self.lists.iter().all(|l| l.add.is_empty())
    }
}

fn applied(c: &Connection, target: &str) -> AppResult<HashSet<String>> {
    let mut st = c.prepare_cached("SELECT track_id FROM import_bring_applied WHERE target = ?1")?;
    Ok(st.query_map([target], |r| r.get(0))?.collect::<Result<_, _>>()?)
}

/// Matched songs still in the library, in order, each once, leaving out
/// those already sent to `target`.
fn fresh(c: &Connection, sql: &str, args: impl rusqlite::Params, target: &str, explicit_ok: bool) -> AppResult<Vec<String>> {
    let done = applied(c, target)?;
    let mut st = c.prepare_cached(sql)?;
    let ids: Vec<String> = st.query_map(args, |r| r.get(0))?.collect::<Result<_, _>>()?;
    let ids = ids.into_iter().map(|id| rated_copy(c, &id, explicit_ok)).collect::<AppResult<Vec<_>>>()?;
    let mut seen = HashSet::new();
    Ok(ids.into_iter().filter(|id| !done.contains(id) && seen.insert(id.clone())).collect())
}

/// Whether a library song is the explicit copy (Some(true)), the clean one
/// (Some(false)) or doesn't say: its file tag first, then a version note.
fn rating(explicit_tag: Option<bool>, version_fold: &str) -> Option<bool> {
    explicit_tag.or_else(|| {
        let words: Vec<&str> = version_fold.split([' ', ';']).collect();
        if words.iter().any(|w| *w == "explicit" || *w == "dirty") {
            Some(true)
        } else if words.contains(&"clean") {
            Some(false)
        } else {
            None
        }
    })
}

/// Clean and explicit copies of a song match the same, and the file names
/// neither. When the library has both, the copy the Explicit content setting
/// plays: explicit unless it's filtered. Same album first.
pub fn rated_copy(c: &Connection, track_id: &str, explicit_ok: bool) -> AppResult<String> {
    let mut st = c.prepare_cached(
        "SELECT b.track_id, b.version_fold, b.album_fold, b.duration_ms,
                coalesce(f.explicit, (SELECT x.explicit FROM explicit_found x WHERE x.track_id = b.track_id)), b.track_id = a.track_id
         FROM song_index a JOIN song_index b ON b.artist_fold = a.artist_fold AND b.title_fold = a.title_fold AND b.unreleased = 0
         LEFT JOIN tracks t ON t.id = b.track_id LEFT JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'
         WHERE a.track_id = ?1",
    )?;
    type Row = (String, String, String, i64, Option<bool>, bool);
    let rows: Vec<Row> = st.query_map([track_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<Result<_, _>>()?;
    let Some(me) = rows.iter().find(|r| r.5) else { return Ok(track_id.to_string()) };
    // A copy that doesn't say counts as either; one that says the other way doesn't play.
    let plays = |r: &Row| rating(r.4, &r.1) != Some(!explicit_ok);
    if plays(me) {
        return Ok(track_id.to_string());
    }
    let core = crate::identity::fold::version_core(&me.1);
    let close = |d: i64| me.3 == 0 || d == 0 || (me.3 - d).abs() <= 5_000;
    let mut other: Vec<&Row> = rows
        .iter()
        .filter(|r| !r.5 && close(r.3) && plays(r) && crate::identity::fold::version_core(&r.1) == core)
        .collect();
    other.sort_by_key(|r| (rating(r.4, &r.1) != Some(explicit_ok), r.2 != me.2, r.0.clone()));
    Ok(other.first().map_or_else(|| track_id.to_string(), |r| r.0.clone()))
}

/// What to send: with `likes`, the liked songs found and not liked by this
/// before; with `lists`, the playlists to make and the songs to add.
pub fn plan(c: &Connection, likes: bool, lists: bool, explicit_ok: bool) -> AppResult<Plan> {
    let mut out = Plan::default();
    let sources = brought(c)?;
    if likes {
        for src in &sources {
            let target = liked_target(src);
            let ids = fresh(
                c,
                "SELECT s.track_id FROM import_likes l JOIN import_songs s ON s.id = l.song_id JOIN tracks t ON t.id = s.track_id
                 WHERE l.source = ?1 AND s.status = 'matched' ORDER BY l.position",
                [src],
                &target,
                explicit_ok,
            )?;
            if !ids.is_empty() {
                out.likes.push((target, ids));
            }
        }
    }
    if lists {
        let mut st = c.prepare_cached(&format!("SELECT id, source, name, name_key FROM import_playlists WHERE source IN {BROUGHT} ORDER BY id"))?;
        let rows: Vec<(i64, String, String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        for (id, source, name, key) in rows {
            let target = list_target(&source, &key);
            let made: Option<(Option<String>, bool)> = c
                .query_row("SELECT playlist_id, gone FROM import_bring_lists WHERE target = ?1", [&target], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let (playlist_id, gone) = made.unwrap_or((None, false));
            if gone {
                continue;
            }
            let add = fresh(
                c,
                "SELECT s.track_id FROM import_playlist_items i JOIN import_songs s ON s.id = i.song_id JOIN tracks t ON t.id = s.track_id
                 WHERE i.playlist_id = ?1 AND s.status = 'matched' ORDER BY i.position",
                [id],
                &target,
                explicit_ok,
            )?;
            if !add.is_empty() {
                out.lists.push(ListPlan { target, name, playlist_id, add });
            }
        }
    }
    Ok(out)
}

/// Remember songs sent to a target, and the playlist a list became.
pub fn record(c: &mut Connection, target: &str, list: Option<(&str, &str)>, ids: &[String]) -> AppResult<()> {
    let tx = c.transaction()?;
    if let Some((name, playlist_id)) = list {
        tx.execute(
            "INSERT INTO import_bring_lists (target, name, playlist_id) VALUES (?1, ?2, ?3)
             ON CONFLICT (target) DO UPDATE SET name = excluded.name, playlist_id = excluded.playlist_id",
            params![target, name, playlist_id],
        )?;
    }
    {
        let mut ins = tx.prepare_cached("INSERT OR IGNORE INTO import_bring_applied (target, track_id) VALUES (?1, ?2)")?;
        for id in ids {
            ins.execute(params![target, id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// The user deleted the playlist a list became: leave it be from now on.
pub fn mark_gone(c: &Connection, target: &str) -> AppResult<()> {
    c.execute("UPDATE import_bring_lists SET gone = 1 WHERE target = ?1", [target])?;
    Ok(())
}

// ------------------------------------------------------------------ arrivals

/// Looks again for the brought sources' missing songs (after the library
/// changed). Returns how many are found or unsure now.
pub fn rematch(c: &mut Connection, ranks: HashMap<String, Rank>, now_ms: i64) -> AppResult<usize> {
    let rows: Vec<(i64, SongIn)> = {
        let mut st = c.prepare_cached(&format!(
            "SELECT id, artist, title, album, duration_ms, isrc FROM import_songs s
             WHERE source IN {BROUGHT} AND status = 'missing' AND reviewed = 0"
        ))?;
        st.query_map([], |r| {
            Ok((
                r.get(0)?,
                SongIn {
                    artist: r.get(1)?,
                    title: r.get(2)?,
                    album: r.get(3)?,
                    duration_ms: r.get::<_, Option<i64>>(4)?.map(|d| d.max(0) as u64),
                    isrc: r.get(5)?,
                    service_id: None,
                },
            ))
        })?
        .collect::<Result<_, _>>()?
    };
    if rows.is_empty() {
        return Ok(0);
    }
    let mut found = Vec::new();
    {
        let mut m = Matcher::new(c, ranks);
        for (id, s) in &rows {
            let o = m.find(s)?;
            if o.status != Status::Missing {
                found.push((*id, o));
            }
        }
    }
    if found.is_empty() {
        return Ok(0);
    }
    let tx = c.transaction()?;
    {
        let mut up = tx.prepare_cached(
            "UPDATE import_songs SET status = ?2, track_id = ?3, tier = ?4, confidence = ?5, candidates = ?6, matched_at = ?7 WHERE id = ?1",
        )?;
        for (id, o) in &found {
            up.execute(params![
                id,
                o.status.as_str(),
                o.track_id,
                o.tier.map(tier_name),
                o.confidence as i64,
                serde_json::to_string(&o.candidates).unwrap_or_else(|_| "[]".into()),
                o.track_id.as_ref().map(|_| now_ms),
            ])?;
        }
    }
    tx.commit()?;
    Ok(found.len())
}

// ------------------------------------------------------------------ step 3

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AlbumState {
    /// Not asked for yet.
    Missing,
    /// MusicBrainz is picking which release it is.
    Picking,
    /// On the Get albums queue, or asked of Lidarr.
    Asked,
    /// Nothing could be found for it.
    NotFound,
    /// In the library now.
    Here,
}

impl AlbumState {
    fn as_str(self) -> &'static str {
        match self {
            AlbumState::Missing => "missing",
            AlbumState::Picking => "picking",
            AlbumState::Asked => "asked",
            AlbumState::NotFound => "notFound",
            AlbumState::Here => "here",
        }
    }

    fn parse(s: &str) -> AlbumState {
        match s {
            "picking" => AlbumState::Picking,
            "asked" => AlbumState::Asked,
            "notFound" => AlbumState::NotFound,
            "here" => AlbumState::Here,
            _ => AlbumState::Missing,
        }
    }
}

/// An album to get.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BringAlbum {
    pub key: String,
    pub artist: String,
    pub album: String,
    /// Its songs the likes and playlists want.
    pub songs: u64,
    /// How many of those are liked.
    pub liked: u64,
    pub state: AlbumState,
    pub reason: Option<String>,
    pub release_group_id: Option<String>,
    /// Its cover, once MusicBrainz has picked it.
    pub thumb: Option<String>,
}

/// Every album to get, most loved first (liked songs, then songs wanted),
/// with how asking for it went. Albums that arrived stay, as Here.
pub fn albums(c: &Connection) -> AppResult<Vec<BringAlbum>> {
    let mut by_key: HashMap<String, BringAlbum> = HashMap::new();
    let mut st = c.prepare_cached(&format!(
        "SELECT s.artist, s.album, EXISTS (SELECT 1 FROM import_likes l WHERE l.song_id = s.id) FROM import_songs s
         WHERE s.source IN {BROUGHT} AND s.status = 'missing' AND trim(coalesce(s.album, '')) <> '' AND {WANTED}"
    ))?;
    let rows: Vec<(String, String, bool)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
    for (artist, album, liked) in rows {
        let key = album_key(&artist, &album);
        if key.ends_with('\u{1f}') {
            continue;
        }
        let a = by_key.entry(key.clone()).or_insert_with(|| BringAlbum {
            key,
            artist,
            album,
            songs: 0,
            liked: 0,
            state: AlbumState::Missing,
            reason: None,
            release_group_id: None,
            thumb: None,
        });
        a.songs += 1;
        a.liked += liked as u64;
    }
    // Saved albums the library lacks.
    let mut st = c.prepare_cached(&format!("SELECT artist, album FROM import_albums WHERE source IN {BROUGHT} AND found = 0"))?;
    let saved: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    for (artist, album) in saved {
        let key = album_key(&artist, &album);
        if key.ends_with('\u{1f}') || album_found(c, &artist, &album)? {
            continue;
        }
        by_key.entry(key.clone()).or_insert_with(|| BringAlbum {
            key,
            artist,
            album,
            songs: 0,
            liked: 0,
            state: AlbumState::Missing,
            reason: None,
            release_group_id: None,
            thumb: None,
        });
    }
    // How asking went, and albums that arrived.
    let mut st = c.prepare_cached("SELECT key, artist, album, state, release_group_id, reason FROM import_bring_albums")?;
    type Raw = (String, String, String, String, Option<String>, Option<String>);
    let asked: Vec<Raw> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<Result<_, _>>()?;
    for (key, artist, album, state, rg, reason) in asked {
        let here = album_found(c, &artist, &album)?;
        let a = by_key.entry(key.clone()).or_insert_with(|| BringAlbum {
            key,
            artist,
            album,
            songs: 0,
            liked: 0,
            state: AlbumState::Missing,
            reason: None,
            release_group_id: None,
            thumb: None,
        });
        a.state = if here { AlbumState::Here } else { AlbumState::parse(&state) };
        a.thumb = rg.as_deref().and_then(crate::lidarr::request::cover_thumb);
        a.release_group_id = rg;
        a.reason = if a.state == AlbumState::NotFound { reason } else { None };
    }
    let mut out: Vec<BringAlbum> = by_key.into_values().collect();
    out.sort_by(|a, b| {
        b.liked
            .cmp(&a.liked)
            .then(b.songs.cmp(&a.songs))
            .then_with(|| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()))
            .then_with(|| a.album.to_lowercase().cmp(&b.album.to_lowercase()))
    });
    Ok(out)
}

/// "31 albums on the way, 4 here so far".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// Not asked for yet.
    pub to_get: u64,
    /// Asked for (those here included).
    pub on_the_way: u64,
    pub here: u64,
    pub not_found: u64,
}

pub fn progress(list: &[BringAlbum]) -> Progress {
    let mut p = Progress::default();
    for a in list {
        match a.state {
            AlbumState::Missing => p.to_get += 1,
            AlbumState::Picking | AlbumState::Asked => p.on_the_way += 1,
            AlbumState::Here => {
                p.on_the_way += 1;
                p.here += 1;
            }
            AlbumState::NotFound => p.not_found += 1,
        }
    }
    p
}

/// Put albums on the list to pick: those named (any not here yet, so a
/// not-found one can be tried again), or with None every one not asked for.
/// Most loved first. Returns how many.
pub fn mark_wanted(c: &mut Connection, keys: Option<&[String]>, now_ms: i64) -> AppResult<usize> {
    let list = albums(c)?;
    let tx = c.transaction()?;
    let mut n = 0;
    {
        let mut ins = tx.prepare_cached(
            "INSERT INTO import_bring_albums (key, artist, album, asked_at, state, release_group_id, reason) VALUES (?1, ?2, ?3, ?4, 'picking', NULL, NULL)
             ON CONFLICT (key) DO UPDATE SET asked_at = excluded.asked_at, state = 'picking', release_group_id = NULL, reason = NULL",
        )?;
        for a in &list {
            let pick = match keys {
                Some(k) => k.contains(&a.key) && a.state != AlbumState::Here,
                None => a.state == AlbumState::Missing,
            };
            if pick {
                // Later ones a little later, so they're picked in this order.
                ins.execute(params![a.key, a.artist, a.album, now_ms + n as i64])?;
                n += 1;
            }
        }
    }
    tx.commit()?;
    Ok(n)
}

/// Albums waiting for MusicBrainz, in the order asked.
pub fn picking(c: &Connection) -> AppResult<Vec<(String, String, String)>> {
    let mut st = c.prepare_cached("SELECT key, artist, album FROM import_bring_albums WHERE state = 'picking' ORDER BY asked_at, rowid")?;
    Ok(st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?)
}

pub fn set_album(c: &Connection, key: &str, state: AlbumState, release_group_id: Option<&str>, reason: Option<&str>) -> AppResult<()> {
    c.execute(
        "UPDATE import_bring_albums SET state = ?2, release_group_id = coalesce(?3, release_group_id), reason = ?4 WHERE key = ?1",
        params![key, state.as_str(), release_group_id, reason],
    )?;
    Ok(())
}

// ------------------------------------------------------------------ the service

/// Where likes and playlists go: the app's like and playlist queues, which
/// reach the server when it's online.
#[async_trait]
pub trait Server: Send + Sync {
    async fn like(&self, track_ids: Vec<String>) -> AppResult<()>;
    /// Make a playlist; returns its id.
    async fn create(&self, name: &str, track_ids: Vec<String>) -> AppResult<String>;
    /// Add at the end. NotFound when the playlist is gone.
    async fn add(&self, playlist_id: &str, track_ids: Vec<String>) -> AppResult<()>;
}

/// What a pick came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// The release group, and its title.
    Group(String, String),
    /// Nothing to ask for, and why.
    None(String),
}

/// Lidarr, through the request path and the Get albums queue.
#[async_trait]
pub trait Getter: Send + Sync {
    /// Lidarr is set up (here or through a home side) and its check passed.
    fn ready(&self) -> bool;
    /// Until when practice mode only lists what it would get (unix seconds).
    fn practice_until(&self) -> Option<i64>;
    /// Which release group to ask for.
    async fn pick(&self, artist: &str, album: &str) -> AppResult<Picked>;
    /// Onto the Get albums queue (paced, most loved first).
    fn enqueue(&self, albums: Vec<QueuedGet>) -> AppResult<()>;
    /// Albums Lidarr refused once asked, by release group id: why.
    fn refused(&self) -> HashMap<String, String>;
}

/// Lidarr as the page shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LidarrState {
    pub ready: bool,
    pub practice_until: Option<i64>,
}

/// Everything the page shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BringStatus {
    pub summary: Summary,
    pub lists: Vec<BroughtList>,
    pub albums: Vec<BringAlbum>,
    pub progress: Progress,
    pub lidarr: LidarrState,
    /// Likes and songs added the last time anything was sent.
    pub sent: Sent,
}

/// What went to the server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sent {
    pub liked: u64,
    pub playlists_made: u64,
    pub songs_added: u64,
}

pub struct BringDeps {
    pub db: DbHandle,
    pub server: Arc<dyn Server>,
    pub getter: Arc<dyn Getter>,
    pub clock_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
}

#[derive(Clone)]
pub struct Bring {
    inner: Arc<BringDeps>,
    lock: Arc<tokio::sync::Mutex<()>>,
    wake: Arc<tokio::sync::Notify>,
    last: Arc<parking_lot::Mutex<Sent>>,
}

impl Bring {
    pub fn new(deps: BringDeps) -> Self {
        Self { inner: Arc::new(deps), lock: Default::default(), wake: Default::default(), last: Default::default() }
    }

    fn db(&self) -> &DbHandle {
        &self.inner.db
    }

    fn now(&self) -> i64 {
        (self.inner.clock_ms)()
    }

    /// Step 1 with a file: read it, then send what's found.
    pub async fn read(&self, path: PathBuf) -> AppResult<BringStatus> {
        let (db, now) = (self.db().clone(), self.now());
        tokio::task::spawn_blocking(move || read(&db, &path, now))
            .await
            .map_err(|e| AppError::Internal(format!("import task: {e}")))??;
        self.send(false).await?;
        self.status().await
    }

    /// A source imported another way (a connected account): bring it over.
    pub async fn adopt(&self, source: &str) -> AppResult<BringStatus> {
        let (src, now) = (source.to_string(), self.now());
        let known: bool = self
            .db()
            .read_async({
                let src = src.clone();
                move |c| Ok(c.query_row("SELECT EXISTS (SELECT 1 FROM import_sources WHERE source = ?1)", [&src], |r| r.get(0))?)
            })
            .await?;
        if !known {
            return Err(AppError::Invalid("Nothing has come in from there yet.".into()));
        }
        self.db().write_async(move |c| mark_brought(c, &src, now)).await?;
        self.send(false).await?;
        self.status().await
    }

    /// Send what's found and not sent yet. With `arrivals`, only what the
    /// settings say to do as albums arrive.
    pub async fn send(&self, arrivals: bool) -> AppResult<Sent> {
        let _g = self.lock.lock().await;
        let s = crate::settings::load(self.db());
        let (likes, lists) = if arrivals { (s.bring_like_arrivals, s.bring_fill_playlists) } else { (true, true) };
        let explicit_ok = s.explicit_content == crate::settings::ExplicitContent::Allowed;
        let plan = self.db().read_async(move |c| plan(c, likes, lists, explicit_ok)).await?;
        let mut sent = Sent::default();
        for (target, ids) in plan.likes {
            self.inner.server.like(ids.clone()).await?;
            sent.liked += ids.len() as u64;
            self.db().write_async(move |c| record(c, &target, None, &ids)).await?;
        }
        for l in plan.lists {
            let ListPlan { target, name, playlist_id, add } = l;
            match playlist_id {
                None => {
                    let id = self.inner.server.create(&name, add.clone()).await?;
                    sent.playlists_made += 1;
                    sent.songs_added += add.len() as u64;
                    self.db().write_async(move |c| record(c, &target, Some((&name, &id)), &add)).await?;
                }
                Some(id) => match self.inner.server.add(&id, add.clone()).await {
                    Ok(()) => {
                        sent.songs_added += add.len() as u64;
                        self.db().write_async(move |c| record(c, &target, None, &add)).await?;
                    }
                    Err(AppError::NotFound(_)) => {
                        tracing::info!(playlist = %name, "bring over: the playlist was deleted, leaving it be");
                        self.db().write_async(move |c| mark_gone(c, &target)).await?;
                    }
                    Err(e) => return Err(e),
                },
            }
        }
        if sent != Sent::default() {
            *self.last.lock() = sent;
        }
        Ok(sent)
    }

    /// Step 3: get albums (all not asked for yet, or those named).
    pub async fn get(&self, keys: Option<Vec<String>>) -> AppResult<BringStatus> {
        if !self.inner.getter.ready() {
            return Err(AppError::Invalid("Set up Lidarr first.".into()));
        }
        let now = self.now();
        self.db().write_async(move |c| mark_wanted(c, keys.as_deref(), now)).await?;
        self.poke();
        self.status().await
    }

    /// Picks the albums waiting for MusicBrainz and queues them. Stops when
    /// MusicBrainz or Lidarr can't be reached (the next pass carries on).
    pub async fn pick_pending(&self) -> AppResult<usize> {
        let _g = self.lock.lock().await;
        if !self.inner.getter.ready() {
            return Ok(0);
        }
        let waiting = self.db().read_async(picking).await?;
        let mut queued = 0;
        for (key, artist, album) in waiting.into_iter().take(PICKS_PER_PASS) {
            match self.inner.getter.pick(&artist, &album).await {
                Ok(Picked::Group(rg, title)) => {
                    self.inner.getter.enqueue(vec![QueuedGet {
                        release_group_id: rg.clone(),
                        title,
                        artist: artist.clone(),
                        artist_mbid: None,
                        artist_id: None,
                    }])?;
                    self.db().write_async(move |c| set_album(c, &key, AlbumState::Asked, Some(&rg), None)).await?;
                    queued += 1;
                }
                Ok(Picked::None(why)) => {
                    tracing::info!(%album, "bring over: nothing to ask for ({why})");
                    let reason = format!("{NOT_FOUND}. {why}");
                    self.db().write_async(move |c| set_album(c, &key, AlbumState::NotFound, None, Some(&reason))).await?;
                }
                Err(e) if matches!(e.kind(), "network" | "server") => {
                    tracing::info!("bring over: picking paused ({})", e.message());
                    break;
                }
                Err(e) => {
                    let reason = format!("{NOT_FOUND}. {}", e.message());
                    self.db().write_async(move |c| set_album(c, &key, AlbumState::NotFound, None, Some(&reason))).await?;
                }
            }
        }
        Ok(queued)
    }

    /// The library changed: look for the missing songs again and send what
    /// arrived. Returns how many songs turned up.
    pub async fn catch_up(&self) -> AppResult<usize> {
        let (db, now) = (self.db().clone(), self.now());
        let ranks = db.source_ranks();
        let found = db.write_async(move |c| rematch(c, ranks, now)).await?;
        if found > 0 {
            self.send(true).await?;
        }
        Ok(found)
    }

    pub async fn status(&self) -> AppResult<BringStatus> {
        let refused = self.inner.getter.refused();
        let mut st = self
            .db()
            .read_async(|c| Ok(BringStatus { summary: summary(c)?, lists: lists(c)?, albums: albums(c)?, ..Default::default() }))
            .await?;
        for a in &mut st.albums {
            if a.state != AlbumState::Asked {
                continue;
            }
            if let Some(why) = a.release_group_id.as_ref().and_then(|rg| refused.get(&rg.to_ascii_lowercase())) {
                a.state = AlbumState::NotFound;
                a.reason = Some(format!("{NOT_FOUND}. {why}"));
            }
        }
        st.progress = progress(&st.albums);
        st.lidarr = LidarrState { ready: self.inner.getter.ready(), practice_until: self.inner.getter.practice_until() };
        st.sent = *self.last.lock();
        Ok(st)
    }

    /// Look soon (the library changed, albums were asked for).
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    /// One look: pick what's waiting, then catch up with the library.
    pub async fn pass(&self) {
        if let Err(e) = self.pick_pending().await {
            tracing::warn!("bring over: picking: {e}");
        }
        if let Err(e) = self.catch_up().await {
            tracing::warn!("bring over: catching up: {e}");
        }
    }
}

/// While Platter is open: a look every [`LOOK_EVERY`], or soon after a poke.
pub async fn watch(b: Bring) {
    tokio::time::sleep(Duration::from_secs(60)).await;
    loop {
        let any = b.db().read_async(|c| Ok(!brought(c)?.is_empty())).await.unwrap_or(false);
        if any {
            b.pass().await;
        }
        tokio::select! {
            _ = tokio::time::sleep(LOOK_EVERY) => {}
            _ = b.wake.notified() => {
                // Library changes come in bursts during a sync.
                tokio::time::sleep(Duration::from_secs(20)).await;
            }
        }
    }
}

// ------------------------------------------------------------------ the app's parts

/// Likes and playlists through the app's queues.
pub struct AppServer {
    pub likes: crate::likes::Likes,
    pub playlists: crate::playlists::Playlists,
}

#[async_trait]
impl Server for AppServer {
    async fn like(&self, track_ids: Vec<String>) -> AppResult<()> {
        self.likes.set(track_ids, true).await.map(|_| ())
    }

    async fn create(&self, name: &str, track_ids: Vec<String>) -> AppResult<String> {
        Ok(self.playlists.create(Some(name.to_string()), track_ids).await?.id)
    }

    async fn add(&self, playlist_id: &str, track_ids: Vec<String>) -> AppResult<()> {
        self.playlists.add(playlist_id.to_string(), track_ids, crate::db::playlists::Place::End).await.map(|_| ())
    }
}

/// Lidarr's request path and the Get albums queue.
pub struct AppGetter {
    pub lidarr: crate::lidarr::Lidarr,
    pub waves: crate::lidarr::waves::Waves,
}

#[async_trait]
impl Getter for AppGetter {
    fn ready(&self) -> bool {
        self.lidarr.status().ready
    }

    fn practice_until(&self) -> Option<i64> {
        if self.lidarr.practice_active() { self.lidarr.practice_until() } else { None }
    }

    async fn pick(&self, artist: &str, album: &str) -> AppResult<Picked> {
        let p = self.lidarr.pick(artist, None, album, false, None).await?;
        Ok(match p.group {
            Some(g) => Picked::Group(g.id, g.title),
            None => Picked::None(p.reason.unwrap_or_else(|| "MusicBrainz has no official release of it.".into())),
        })
    }

    fn enqueue(&self, albums: Vec<QueuedGet>) -> AppResult<()> {
        self.waves.enqueue_for(albums, "Bring music over").map(|_| ())
    }

    fn refused(&self) -> HashMap<String, String> {
        self.waves
            .state()
            .needs_you
            .into_iter()
            .filter_map(|n| n.get.map(|g| (g.release_group_id.to_ascii_lowercase(), n.reason)))
            .collect()
    }
}

/// Where Lidarr probably is, best first: the Plex server's machine on
/// Lidarr's usual port (a plex.direct name gives its address), then this
/// computer.
pub fn lidarr_guesses(plex_url: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(host) = plex_url.and_then(|u| url::Url::parse(u).ok()).and_then(|u| u.host_str().map(str::to_string)) {
        let host = match host.strip_suffix(".plex.direct") {
            Some(rest) => {
                let first = rest.split('.').next().unwrap_or_default().replace('-', ".");
                first.parse::<std::net::Ipv4Addr>().ok().map(|ip| ip.to_string())
            }
            None => Some(host),
        };
        // Only an address at home: the key never goes over the internet unasked.
        if let Some(h) = host.filter(|h| crate::lidarr::paths::is_private_host(h)) {
            out.push(format!("http://{h}:8686"));
        }
    }
    let local = "http://localhost:8686".to_string();
    if !out.contains(&local) {
        out.push(local);
    }
    out
}

/// A dropped file's name, safe as a file name here: its last part, with
/// anything a file system might refuse replaced. Its extension is kept, as
/// that's how the file is read.
pub fn safe_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = last.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') { c } else { '_' }).collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    if clean.is_empty() {
        return "dropped".into();
    }
    match clean.rsplit_once('.') {
        Some((stem, ext)) if clean.chars().count() > 120 => format!("{}.{}", stem.chars().take(100).collect::<String>(), ext.chars().take(10).collect::<String>()),
        _ => clean,
    }
}
