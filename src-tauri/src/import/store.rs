//! The import tables: what each source brought in, how its songs matched,
//! the review queue, imported playlists and the Missing list.
//!
//! Importing again from a source updates rather than adds: songs are kept
//! once per source (by folded names), review decisions survive, a playlist
//! with the same name is replaced, the liked songs and saved albums are
//! replaced when the new files have them, and plays already imported are
//! skipped (`imported_plays` is unique on its source id).

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::matching::{Matcher, Outcome, Status, album_found, song_key, tier_name};
use super::{Parsed, Skipped, SongIn};
use crate::db::listens::{ImportSource, ImportedPlay, insert_imported};
use crate::db::merge::Rank;
use crate::error::{AppError, AppResult};
use crate::identity::fold;
use crate::model::Track;

/// Most rows the Missing list returns.
pub const MISSING_MAX: usize = 2_000;

/// What one run did beyond what the tables show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunStats {
    pub plays_read: u64,
    pub plays_added: u64,
    pub skipped: Skipped,
}

/// Everything matched, ready to write.
pub struct Prepared {
    songs: Vec<(String, SongIn, Outcome)>,
    by_key: HashMap<String, usize>,
    parsed: Parsed,
    albums_found: Vec<bool>,
    plays: Vec<ImportedPlay>,
}

/// Matches what was parsed against the library (on any connection; reads
/// only). The slow part, kept apart from the write.
pub fn prepare(c: &Connection, source: &str, parsed: Parsed, ranks: HashMap<String, Rank>) -> AppResult<Prepared> {
    let mut m = Matcher::new(c, ranks);
    let mut songs = Vec::new();
    let mut by_key = HashMap::new();
    let all = parsed.playlists.iter().flat_map(|p| p.songs.iter()).chain(parsed.liked.iter().flatten());
    for s in all {
        let k = song_key(s);
        if by_key.contains_key(&k) {
            continue;
        }
        let o = m.find(s)?;
        by_key.insert(k.clone(), songs.len());
        songs.push((k, s.clone(), o));
    }
    let mut albums_found = Vec::new();
    for a in parsed.albums.iter().flatten() {
        albums_found.push(album_found(c, &a.artist, &a.album)?);
    }
    let mut plays = Vec::with_capacity(parsed.plays.len());
    for p in &parsed.plays {
        let o = m.find(&p.song)?;
        plays.push(ImportedPlay {
            source: ImportSource::File,
            source_id: play_id(source, p.ended_at, &p.song),
            played_at: p.ended_at - p.ms_played as i64,
            ms_played: Some(p.ms_played),
            title: p.song.title.clone(),
            artist: Some(p.song.artist.clone()),
            album: p.song.album.clone(),
            rating_key: if o.status == Status::Matched { o.track_id.clone() } else { None },
        });
    }
    Ok(Prepared { songs, by_key, parsed, albums_found, plays })
}

/// A play's id within the source: the minute it ended and the folded names.
/// The short and the extended history of one account log the same play at
/// the same minute, so importing both counts it once.
pub fn play_id(source: &str, ended_at: i64, s: &SongIn) -> String {
    let t = fold::fold_title(&s.title);
    format!("{source}:{}:{}:{}", ended_at.div_euclid(60_000), fold::fold_artist(&s.artist), t.title)
}

fn name_key(name: &str) -> String {
    fold::fold_text(name.trim())
}

/// Writes a prepared import. `replace_playlists`: the files are the whole
/// set of the source's playlists (an account download), so playlists no
/// longer in it are dropped; otherwise only same-named ones are replaced.
pub fn write(c: &mut Connection, source: &str, file_name: Option<&str>, p: Prepared, replace_playlists: bool, now_ms: i64) -> AppResult<RunStats> {
    let tx = c.transaction()?;
    let mut ids: Vec<i64> = Vec::with_capacity(p.songs.len());
    {
        let mut up = tx.prepare_cached(
            "INSERT INTO import_songs (source, song_key, artist, title, album, duration_ms, isrc, service_id, artist_fold, title_fold,
                                       status, track_id, tier, confidence, candidates, matched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT (source, song_key) DO UPDATE SET
                 artist = excluded.artist, title = excluded.title, album = excluded.album,
                 duration_ms = coalesce(excluded.duration_ms, duration_ms), isrc = coalesce(excluded.isrc, isrc),
                 service_id = coalesce(excluded.service_id, service_id),
                 status = CASE WHEN reviewed = 1 THEN status ELSE excluded.status END,
                 track_id = CASE WHEN reviewed = 1 THEN track_id ELSE excluded.track_id END,
                 tier = CASE WHEN reviewed = 1 THEN tier ELSE excluded.tier END,
                 confidence = CASE WHEN reviewed = 1 THEN confidence ELSE excluded.confidence END,
                 candidates = CASE WHEN reviewed = 1 THEN candidates ELSE excluded.candidates END,
                 matched_at = CASE WHEN reviewed = 1 THEN matched_at ELSE excluded.matched_at END
             RETURNING id",
        )?;
        for (k, s, o) in &p.songs {
            let key = super::matching::key_of(s);
            let id: i64 = up.query_row(
                params![
                    source,
                    k,
                    s.artist,
                    s.title,
                    s.album,
                    s.duration_ms.map(|d| d as i64),
                    s.isrc,
                    s.service_id,
                    key.artist_fold,
                    key.title_fold,
                    o.status.as_str(),
                    o.track_id,
                    o.tier.map(tier_name),
                    o.confidence as i64,
                    serde_json::to_string(&o.candidates).unwrap_or_else(|_| "[]".into()),
                    o.track_id.as_ref().map(|_| now_ms),
                ],
                |r| r.get(0),
            )?;
            ids.push(id);
        }
    }
    let song_id = |s: &SongIn| p.by_key.get(&song_key(s)).map(|i| ids[*i]);

    // Playlists.
    if replace_playlists && !p.parsed.playlists.is_empty() {
        tx.execute("DELETE FROM import_playlist_items WHERE playlist_id IN (SELECT id FROM import_playlists WHERE source = ?1)", [source])?;
        tx.execute("DELETE FROM import_playlists WHERE source = ?1", [source])?;
    }
    let mut seen_names: HashSet<String> = HashSet::new();
    for pl in &p.parsed.playlists {
        // Two lists with one name in the same files stay two lists.
        let mut name = pl.name.clone();
        let mut n = 2;
        while !seen_names.insert(name_key(&name)) {
            name = format!("{} ({n})", pl.name);
            n += 1;
        }
        let id: i64 = tx.query_row(
            "INSERT INTO import_playlists (source, name, name_key, imported_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (source, name_key) DO UPDATE SET name = excluded.name, imported_at = excluded.imported_at
             RETURNING id",
            params![source, name, name_key(&name), now_ms],
            |r| r.get(0),
        )?;
        tx.execute("DELETE FROM import_playlist_items WHERE playlist_id = ?1", [id])?;
        let mut ins = tx.prepare_cached("INSERT INTO import_playlist_items (playlist_id, position, song_id) VALUES (?1, ?2, ?3)")?;
        for (pos, s) in pl.songs.iter().enumerate() {
            if let Some(sid) = song_id(s) {
                ins.execute(params![id, pos as i64, sid])?;
            }
        }
    }

    // Liked songs.
    if let Some(liked) = &p.parsed.liked {
        tx.execute("DELETE FROM import_likes WHERE source = ?1", [source])?;
        let mut ins = tx.prepare_cached("INSERT OR IGNORE INTO import_likes (source, song_id, position) VALUES (?1, ?2, ?3)")?;
        for (pos, s) in liked.iter().enumerate() {
            if let Some(sid) = song_id(s) {
                ins.execute(params![source, sid, pos as i64])?;
            }
        }
    }

    // Saved albums.
    if let Some(albums) = &p.parsed.albums {
        tx.execute("DELETE FROM import_albums WHERE source = ?1", [source])?;
        let mut ins = tx.prepare_cached(
            "INSERT OR REPLACE INTO import_albums (source, artist_fold, album_fold, artist, album, found) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for (a, found) in albums.iter().zip(&p.albums_found) {
            ins.execute(params![source, fold::fold_artist(&a.artist), fold::fold_album(&a.album), a.artist, a.album, *found as i64])?;
        }
    }

    // Songs nothing wants any more.
    tx.execute(
        "DELETE FROM import_songs WHERE source = ?1
           AND NOT EXISTS (SELECT 1 FROM import_playlist_items i WHERE i.song_id = import_songs.id)
           AND NOT EXISTS (SELECT 1 FROM import_likes l WHERE l.song_id = import_songs.id)",
        [source],
    )?;

    tx.commit()?;

    let added = insert_imported(c, &p.plays, now_ms)? as u64;
    let stats = RunStats {
        plays_read: p.plays.len() as u64,
        plays_added: added,
        skipped: p.parsed.skipped,
    };
    c.execute(
        "INSERT INTO import_sources (source, imported_at, file_name, stats) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (source) DO UPDATE SET imported_at = excluded.imported_at, file_name = excluded.file_name, stats = excluded.stats",
        params![source, now_ms, file_name, serde_json::to_string(&stats).unwrap_or_default()],
    )?;
    Ok(stats)
}

// ------------------------------------------------------------------ reports

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub total: u64,
    pub matched: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongCounts {
    pub total: u64,
    pub matched: u64,
    pub unsure: u64,
    pub missing: u64,
}

/// How one source's import went: "812 of 900 songs found".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReport {
    pub source: String,
    pub imported_at: i64,
    pub file_name: Option<String>,
    pub songs: SongCounts,
    pub liked: Counts,
    pub playlists: u64,
    pub albums: Counts,
    /// Plays from this source in the listen history, and how many are songs
    /// in the library.
    pub plays: Counts,
    /// New plays the last import added.
    pub plays_added: u64,
    pub skipped: Skipped,
}

/// Every source imported from, with its report.
pub fn reports(c: &Connection) -> AppResult<Vec<SourceReport>> {
    let mut st = c.prepare_cached("SELECT source, imported_at, file_name, stats FROM import_sources ORDER BY source")?;
    let heads: Vec<(String, i64, Option<String>, String)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (source, imported_at, file_name, stats) in heads {
        let stats: RunStats = serde_json::from_str(&stats).unwrap_or_default();
        let mut songs = SongCounts::default();
        let mut q = c.prepare_cached("SELECT status, count(*) FROM import_songs WHERE source = ?1 GROUP BY status")?;
        for row in q.query_map([&source], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))? {
            let (s, n) = row?;
            songs.total += n;
            match Status::parse(&s) {
                Status::Matched => songs.matched += n,
                Status::Unsure => songs.unsure += n,
                Status::Missing => songs.missing += n,
            }
        }
        let liked = c.query_row(
            "SELECT count(*), count(CASE WHEN s.status = 'matched' THEN 1 END) FROM import_likes l JOIN import_songs s ON s.id = l.song_id WHERE l.source = ?1",
            [&source],
            |r| Ok(Counts { total: r.get::<_, i64>(0)? as u64, matched: r.get::<_, i64>(1)? as u64 }),
        )?;
        let playlists: i64 = c.query_row("SELECT count(*) FROM import_playlists WHERE source = ?1", [&source], |r| r.get(0))?;
        let albums = c.query_row("SELECT count(*), coalesce(sum(found), 0) FROM import_albums WHERE source = ?1", [&source], |r| {
            Ok(Counts { total: r.get::<_, i64>(0)? as u64, matched: r.get::<_, i64>(1)? as u64 })
        })?;
        let plays = c.query_row(
            "SELECT count(*), count(rating_key) FROM imported_plays WHERE source = 'file' AND substr(source_id, 1, ?2) = ?1",
            params![format!("{source}:"), source.len() as i64 + 1],
            |r| Ok(Counts { total: r.get::<_, i64>(0)? as u64, matched: r.get::<_, i64>(1)? as u64 }),
        )?;
        out.push(SourceReport {
            source,
            imported_at,
            file_name,
            songs,
            liked,
            playlists: playlists as u64,
            albums,
            plays,
            plays_added: stats.plays_added,
            skipped: stats.skipped,
        });
    }
    Ok(out)
}

// ------------------------------------------------------------------ review queue

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewItem {
    pub song_id: i64,
    pub source: String,
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    /// The library songs it could be, best first.
    pub candidates: Vec<Track>,
}

/// Unsure matches waiting for a decision.
pub fn review_queue(c: &Connection) -> AppResult<Vec<ReviewItem>> {
    let mut st = c.prepare_cached(
        "SELECT id, source, artist, title, album, duration_ms, candidates FROM import_songs
         WHERE status = 'unsure' AND reviewed = 0 ORDER BY lower(artist), lower(title), id",
    )?;
    type Raw = (i64, String, String, String, Option<String>, Option<i64>, String);
    let rows: Vec<Raw> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, source, artist, title, album, dur, cands) in rows {
        let ids: Vec<String> = serde_json::from_str(&cands).unwrap_or_default();
        out.push(ReviewItem {
            song_id: id,
            source,
            artist,
            title,
            album,
            duration_ms: dur.map(|d| d as u64),
            candidates: crate::db::queries::tracks_by_ids(c, &ids)?,
        });
    }
    Ok(out)
}

/// A decision on an unsure match.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Decision {
    /// The best candidate is right.
    Accept,
    /// Another library song is.
    Pick {
        #[serde(rename = "trackId")]
        track_id: String,
    },
    /// None of them: the song is missing.
    None,
}

/// Records a decision. Imports after this keep it.
pub fn review(c: &mut Connection, song_id: i64, d: &Decision, now_ms: i64) -> AppResult<()> {
    let cands: Option<String> = c.query_row("SELECT candidates FROM import_songs WHERE id = ?1", [song_id], |r| r.get(0)).optional()?;
    let Some(cands) = cands else {
        return Err(AppError::NotFound(format!("import song {song_id}")));
    };
    let ids: Vec<String> = serde_json::from_str(&cands).unwrap_or_default();
    let pick = match d {
        Decision::Accept => Some(ids.first().cloned().ok_or_else(|| AppError::Invalid("There's nothing to accept.".into()))?),
        Decision::Pick { track_id } => {
            let known: bool = c.query_row("SELECT count(*) FROM tracks WHERE id = ?1", [track_id], |r| r.get::<_, i64>(0))? > 0;
            if !known {
                return Err(AppError::NotFound(format!("track {track_id}")));
            }
            Some(track_id.clone())
        }
        Decision::None => None,
    };
    let tx = c.transaction()?;
    tx.execute(
        "UPDATE import_songs SET status = ?2, track_id = ?3, reviewed = 1, confidence = ?4, tier = CASE WHEN ?3 IS NULL THEN NULL ELSE 'reviewed' END,
                matched_at = ?5 WHERE id = ?1",
        params![
            song_id,
            if pick.is_some() { "matched" } else { "missing" },
            pick,
            if pick.is_some() { 100 } else { 0 },
            pick.as_ref().map(|_| now_ms)
        ],
    )?;
    tx.commit()?;
    Ok(())
}

// ------------------------------------------------------------------ playlists

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedList {
    /// "pl:<n>" for a playlist, "liked:<source>" for liked songs.
    pub id: String,
    pub source: String,
    pub name: String,
    pub liked: bool,
    pub total: u64,
    pub matched: u64,
}

/// Every imported playlist, liked lists first.
pub fn lists(c: &Connection) -> AppResult<Vec<ImportedList>> {
    let mut out = Vec::new();
    let mut st = c.prepare_cached(
        "SELECT l.source, count(*), count(CASE WHEN s.status = 'matched' THEN 1 END)
         FROM import_likes l JOIN import_songs s ON s.id = l.song_id GROUP BY l.source ORDER BY l.source",
    )?;
    for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
        let (source, total, matched) = r?;
        out.push(ImportedList { id: format!("liked:{source}"), source, name: "Liked songs".into(), liked: true, total: total as u64, matched: matched as u64 });
    }
    let mut st = c.prepare_cached(
        "SELECT p.id, p.source, p.name, count(i.song_id), count(CASE WHEN s.status = 'matched' THEN 1 END)
         FROM import_playlists p LEFT JOIN import_playlist_items i ON i.playlist_id = p.id LEFT JOIN import_songs s ON s.id = i.song_id
         GROUP BY p.id ORDER BY lower(p.name), p.id",
    )?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?)))? {
        let (id, source, name, total, matched) = r?;
        out.push(ImportedList { id: format!("pl:{id}"), source, name, liked: false, total: total as u64, matched: matched as u64 });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedRow {
    pub song_id: i64,
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub status: Status,
    /// The library song, when matched and still in the library.
    pub track: Option<Track>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedListPage {
    pub list: ImportedList,
    pub rows: Vec<ImportedRow>,
}

/// One imported playlist (or liked list) with its songs in order.
pub fn list_page(c: &Connection, id: &str) -> AppResult<ImportedListPage> {
    let list = lists(c)?.into_iter().find(|l| l.id == id).ok_or_else(|| AppError::NotFound(format!("imported list {id}")))?;
    let (sql, arg): (&str, String) = if let Some(source) = id.strip_prefix("liked:") {
        (
            "SELECT s.id, s.artist, s.title, s.album, s.duration_ms, s.status, s.track_id
             FROM import_likes l JOIN import_songs s ON s.id = l.song_id WHERE l.source = ?1 ORDER BY l.position",
            source.to_string(),
        )
    } else {
        (
            "SELECT s.id, s.artist, s.title, s.album, s.duration_ms, s.status, s.track_id
             FROM import_playlist_items i JOIN import_songs s ON s.id = i.song_id WHERE i.playlist_id = ?1 ORDER BY i.position",
            id.trim_start_matches("pl:").to_string(),
        )
    };
    let mut st = c.prepare_cached(sql)?;
    type Raw = (i64, String, String, Option<String>, Option<i64>, String, Option<String>);
    let raw: Vec<Raw> = st
        .query_map([arg], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
        .collect::<Result<_, _>>()?;
    let mut rows = Vec::with_capacity(raw.len());
    for (song_id, artist, title, album, dur, status, track_id) in raw {
        let status = Status::parse(&status);
        let track = match (&track_id, status) {
            (Some(t), Status::Matched) => crate::db::queries::tracks_by_ids(c, std::slice::from_ref(t))?.pop(),
            _ => None,
        };
        // A matched song that left the library shows as missing.
        let status = if status == Status::Matched && track.is_none() { Status::Missing } else { status };
        rows.push(ImportedRow { song_id, artist, title, album, duration_ms: dur.map(|d| d as u64), status, track });
    }
    Ok(ImportedListPage { list, rows })
}

// ------------------------------------------------------------------ missing

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreLinks {
    pub bandcamp: String,
    pub qobuz: String,
    pub itunes: String,
}

/// Store search pages for a song.
pub fn store_links(artist: &str, title: &str) -> StoreLinks {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    let q = utf8_percent_encode(&format!("{artist} {title}"), NON_ALPHANUMERIC).to_string();
    StoreLinks {
        bandcamp: format!("https://bandcamp.com/search?q={q}&item_type=t"),
        qobuz: format!("https://www.qobuz.com/search?q={q}"),
        itunes: format!("https://music.apple.com/search?term={q}"),
    }
}

/// Only store search pages open from the Missing list.
pub fn link_allowed(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    u.scheme() == "https" && matches!(u.host_str(), Some("bandcamp.com" | "www.qobuz.com" | "music.apple.com")) && u.port().is_none()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingSong {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    /// How many imported playlists and liked lists want it.
    pub wants: u64,
    /// The playlists that want it, by name ("Liked songs" for likes).
    pub wanted_by: Vec<String>,
    pub sources: Vec<String>,
    pub links: StoreLinks,
}

/// Every missing song from imported playlists and likes, the most wanted
/// first. The same song from two sources is one row.
pub fn missing(c: &Connection, limit: usize) -> AppResult<Vec<MissingSong>> {
    let mut st = c.prepare_cached(
        "WITH m AS (SELECT id, source, artist, title, album, artist_fold, title_fold FROM import_songs WHERE status = 'missing'),
              w AS (SELECT m.artist_fold AS a, m.title_fold AS t, 'p' || i.playlist_id AS want, p.name AS name
                    FROM m JOIN import_playlist_items i ON i.song_id = m.id JOIN import_playlists p ON p.id = i.playlist_id
                    UNION
                    SELECT m.artist_fold, m.title_fold, 'l' || l.source, 'Liked songs'
                    FROM m JOIN import_likes l ON l.song_id = m.id),
              g AS (SELECT a, t, count(DISTINCT want) AS wants, group_concat(DISTINCT name) AS names FROM w GROUP BY a, t)
         SELECT min(m.artist), min(m.title), max(m.album), group_concat(DISTINCT m.source), g.wants, g.names
         FROM m JOIN g ON g.a = m.artist_fold AND g.t = m.title_fold
         GROUP BY m.artist_fold, m.title_fold
         ORDER BY g.wants DESC, lower(min(m.artist)), lower(min(m.title))
         LIMIT ?1",
    )?;
    let rows = st.query_map([limit as i64], |r| {
        let artist: String = r.get(0)?;
        let title: String = r.get(1)?;
        let split = |s: Option<String>| -> Vec<String> {
            let mut v: Vec<String> = s.unwrap_or_default().split(',').filter(|x| !x.is_empty()).map(String::from).collect();
            v.sort();
            v.dedup();
            v
        };
        Ok(MissingSong {
            links: store_links(&artist, &title),
            artist,
            title,
            album: r.get(2)?,
            sources: split(r.get(3)?),
            wants: r.get::<_, i64>(4)? as u64,
            wanted_by: split(r.get(5)?),
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// How many songs wait in the review queue, and how many are missing.
pub fn queue_counts(c: &Connection) -> AppResult<(u64, u64)> {
    c.query_row(
        "SELECT count(CASE WHEN status = 'unsure' AND reviewed = 0 THEN 1 END), count(CASE WHEN status = 'missing' THEN 1 END) FROM import_songs",
        [],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
    )
    .map_err(Into::into)
}

/// Forgets everything imported from a source, plays included.
pub fn forget(c: &mut Connection, source: &str) -> AppResult<()> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM import_playlist_items WHERE playlist_id IN (SELECT id FROM import_playlists WHERE source = ?1)", [source])?;
    tx.execute("DELETE FROM import_playlists WHERE source = ?1", [source])?;
    tx.execute("DELETE FROM import_likes WHERE source = ?1", [source])?;
    tx.execute("DELETE FROM import_albums WHERE source = ?1", [source])?;
    tx.execute("DELETE FROM import_songs WHERE source = ?1", [source])?;
    tx.execute(
        "DELETE FROM imported_plays WHERE source = 'file' AND substr(source_id, 1, ?2) = ?1",
        params![format!("{source}:"), source.len() as i64 + 1],
    )?;
    tx.execute("DELETE FROM import_sources WHERE source = ?1", [source])?;
    tx.commit()?;
    Ok(())
}
