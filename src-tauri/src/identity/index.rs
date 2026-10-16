//! The song index: every synced song, on every included server, under its
//! folded artist and title (`song_index`).
//!
//! A song has one row per artist name it goes by: its own artist, and the
//! album artist when that differs. [`rebuild`] runs after each library sync
//! and [`refresh_artists`] after file tags change, for just the artists
//! whose songs were read. [`lookup`] finds the candidates for a [`SongKey`]
//! through the indexes on ids and on (artist, title), then ranks them with
//! [`compare`](super::compare).

use std::collections::HashMap;

use rusqlite::{Connection, ToSql, params, params_from_iter};

use super::{Context, Match, SongKey, compare, fold};
use crate::db::merge::{Rank, quality};
use crate::db::release::track_unreleased;
use crate::error::AppResult;

/// What an album says about its songs' release status.
struct AlbumInfo {
    format: Option<String>,
    subformats: Vec<String>,
    matched: Option<bool>,
    studio: Option<String>,
    mb_found: Option<bool>,
    version: Option<String>,
}

const SOURCE: &str = "SELECT t.id, t.title, t.artist_id, t.artist_title, t.track_artist, t.album_title, t.duration_ms,
            t.mbid, f.isrc, f.recording_mbid, coalesce(f.version, f.release), t.album_id,
            al.format, al.subformats, al.matched, al.studio, mc.found, coalesce(at.version, at.release), al.id IS NOT NULL, si.recording_mbid
     FROM tracks t
     LEFT JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'
     LEFT JOIN albums al ON al.id = t.album_id
     LEFT JOIN album_mb_check mc ON mc.album_id = t.album_id
     LEFT JOIN album_tags at ON at.album_id = t.album_id
     LEFT JOIN sound_ids si ON si.track_id = t.id AND si.part_key = t.part_key AND si.status = 'ok'";

/// Adds the rows of the songs `SOURCE` + `filter` selects.
fn insert(c: &Connection, filter: &str, args: &[&dyn ToSql]) -> AppResult<usize> {
    let mut st = c.prepare(&format!("{SOURCE} {filter}"))?;
    let mut ins = c.prepare_cached(
        "INSERT OR REPLACE INTO song_index
             (track_id, artist_fold, title_fold, version_fold, album_fold, artist_id, duration_ms,
              isrc, recording_mbid, plex_mbid, unreleased, sound_mbid)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;
    let mut albums: HashMap<String, AlbumInfo> = HashMap::new();
    let mut folded_albums: HashMap<String, String> = HashMap::new();
    let mut rows = st.query(params_from_iter(args.iter()))?;
    let mut n = 0;
    while let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        let title: String = r.get(1)?;
        let artist_id: String = r.get(2)?;
        let album_artist: String = r.get(3)?;
        let own_artist: Option<String> = r.get(4)?;
        let album_title: String = r.get(5)?;
        let duration = r.get::<_, i64>(6)?.max(0);
        let plex_mbid: Option<String> = r.get(7)?;
        let isrc: Option<String> = r.get(8)?;
        let rec: Option<String> = r.get(9)?;
        let file_version: Option<String> = r.get(10)?;
        let album_id: String = r.get(11)?;
        let sound: Option<String> = r.get(19)?;
        if !albums.contains_key(&album_id) {
            let subs: Option<String> = r.get(13)?;
            let known: bool = r.get(18)?;
            albums.insert(
                album_id.clone(),
                AlbumInfo {
                    format: r.get(12)?,
                    subformats: serde_json::from_str(subs.as_deref().unwrap_or("[]")).unwrap_or_default(),
                    // An album not synced yet says nothing either way.
                    matched: if known { r.get(14)? } else { Some(true) },
                    studio: r.get(15)?,
                    mb_found: r.get(16)?,
                    version: r.get(17)?,
                },
            );
        }
        let al = &albums[&album_id];
        let version = file_version.or_else(|| al.version.clone());
        let unreleased = track_unreleased(
            &id,
            &album_id,
            al.format.as_deref(),
            &al.subformats,
            al.matched,
            al.studio.as_deref(),
            version.as_deref(),
            al.mb_found,
        );
        let album_fold = folded_albums.entry(album_title.clone()).or_insert_with(|| fold::fold_album(&album_title)).clone();
        let t = fold::fold_title(&title);
        let mut artists = vec![fold::fold_artist(own_artist.as_deref().unwrap_or(&album_artist))];
        if own_artist.is_some() {
            let a = fold::fold_artist(&album_artist);
            if !artists.contains(&a) {
                artists.push(a);
            }
        }
        // Names that say nothing ("Various Artists") are left out, unless
        // there's no other: the ids can still match.
        if artists.len() > 1 {
            artists.retain(|a| !a.is_empty());
        }
        for a in artists {
            n += ins.execute(params![
                id,
                a,
                t.title,
                t.version,
                album_fold,
                artist_id,
                duration,
                isrc.as_deref().and_then(crate::tags::clean_isrc),
                rec.as_deref().and_then(crate::tags::clean_mbid),
                plex_mbid.as_deref().map(|m| m.trim().to_ascii_lowercase()).filter(|m| !m.is_empty()),
                unreleased as i64,
                sound.as_deref().and_then(crate::tags::clean_mbid),
            ])?;
        }
    }
    Ok(n)
}

/// Rebuilds the whole index from the synced songs (after a library sync).
/// Returns the rows written.
pub fn rebuild(c: &Connection) -> AppResult<usize> {
    c.execute("DELETE FROM song_index", [])?;
    insert(c, "", &[])
}

/// Rebuilds the rows of these album artists' songs (file tags changed).
pub fn refresh_artists(c: &Connection, artist_ids: &[String]) -> AppResult<usize> {
    let mut n = 0;
    for id in artist_ids {
        c.execute("DELETE FROM song_index WHERE artist_id = ?1", [id])?;
        n += insert(c, "WHERE t.artist_id = ?1", &[id])?;
    }
    Ok(n)
}

/// [`refresh_artists`] for the artists of these songs.
pub fn refresh_for_tracks(c: &Connection, track_ids: &[String]) -> AppResult<usize> {
    let mut st = c.prepare_cached("SELECT artist_id FROM tracks WHERE id = ?1")?;
    let mut artists: Vec<String> = Vec::new();
    for id in track_ids {
        let mut rows = st.query([id])?;
        if let Some(r) = rows.next()? {
            let a: String = r.get(0)?;
            if !artists.contains(&a) {
                artists.push(a);
            }
        }
    }
    refresh_artists(c, &artists)
}

/// A song in the library that matches a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub track_id: String,
    pub matched: Match,
    /// On an album with the same (folded) title.
    pub same_album: bool,
}

/// A candidate with what orders ties: the server's rank and the file quality.
type Ranked = (Candidate, Rank, (u8, u32, u32, u32));

struct Row {
    track_id: String,
    key: SongKey,
    quality: (u8, u32, u32, u32),
}

const COLS: &str = "SELECT s.track_id, s.artist_fold, s.title_fold, s.version_fold, s.album_fold, s.duration_ms,
            s.isrc, s.recording_mbid, s.plex_mbid, s.unreleased, t.codec, t.bit_depth, t.sample_rate, t.bitrate, s.sound_mbid
     FROM song_index s JOIN tracks t ON t.id = s.track_id";

fn read_rows(c: &Connection, filter: &str, args: &[&dyn ToSql], out: &mut Vec<Row>) -> AppResult<()> {
    let mut st = c.prepare_cached(&format!("{COLS} WHERE {filter}"))?;
    let rows = st.query_map(params_from_iter(args.iter()), |r| {
        Ok(Row {
            track_id: r.get(0)?,
            key: SongKey {
                artist_fold: r.get(1)?,
                title_fold: r.get(2)?,
                version_fold: r.get(3)?,
                album_fold: r.get(4)?,
                duration_ms: r.get::<_, i64>(5)?.max(0) as u64,
                isrc: r.get(6)?,
                recording_mbid: r.get(7)?,
                plex_track_mbid: r.get(8)?,
                unreleased: r.get::<_, i64>(9)? != 0,
                album_artist_fold: None,
                sound_mbid: r.get(14)?,
            },
            quality: quality(r.get::<_, Option<String>>(10)?.as_deref(), r.get(11)?, r.get(12)?, r.get(13)?),
        })
    })?;
    for r in rows {
        out.push(r?);
    }
    Ok(())
}

/// Every song in the index that matches `key`, best first: the strongest
/// tier, then confidence, then a song on an album of the same title, then
/// the order songs play in (`db::merge::best_copies`: the server reached the
/// best way, then the better file). `ranks` maps server tags to how they're
/// reached now. An unreleased key matches nothing.
pub fn lookup(c: &Connection, key: &SongKey, ctx: Context, ranks: &HashMap<String, Rank>) -> AppResult<Vec<Candidate>> {
    if key.unreleased {
        return Ok(Vec::new());
    }
    let mut rows = Vec::new();
    if let Some(v) = &key.isrc {
        read_rows(c, "s.isrc = ?1", &[v], &mut rows)?;
    }
    if let Some(v) = &key.recording_mbid {
        read_rows(c, "s.recording_mbid = ?1", &[v], &mut rows)?;
    }
    if let Some(v) = &key.plex_track_mbid {
        read_rows(c, "s.plex_mbid = ?1", &[v], &mut rows)?;
    }
    if let Some(v) = &key.recording_mbid {
        read_rows(c, "s.sound_mbid = ?1", &[v], &mut rows)?;
    }
    if let Some(v) = &key.sound_mbid {
        read_rows(c, "s.recording_mbid = ?1 OR s.sound_mbid = ?1", &[v], &mut rows)?;
    }
    if !key.title_fold.is_empty() {
        for a in key.artists() {
            read_rows(c, "s.artist_fold = ?1 AND s.title_fold = ?2", &[&a, &key.title_fold], &mut rows)?;
        }
    }
    let rank = |id: &str| crate::sid::owner(id).and_then(|t| ranks.get(&t).copied()).unwrap_or(Rank::Unknown);
    let mut best: HashMap<String, Ranked> = HashMap::new();
    for row in rows {
        let Some(m) = compare(key, &row.key, ctx) else { continue };
        let same_album = !key.album_fold.is_empty() && key.album_fold == row.key.album_fold;
        let cand = Candidate { track_id: row.track_id.clone(), matched: m, same_album };
        let better = match best.get(&row.track_id) {
            None => true,
            Some((old, _, _)) => (m.tier, std::cmp::Reverse(m.confidence)) < (old.matched.tier, std::cmp::Reverse(old.matched.confidence)),
        };
        if better {
            let r = rank(&row.track_id);
            best.insert(row.track_id, (cand, r, row.quality));
        }
    }
    let mut out: Vec<Ranked> = best.into_values().collect();
    out.sort_by(|a, b| {
        let k = |x: &Ranked| {
            (x.0.matched.tier, std::cmp::Reverse(x.0.matched.confidence), !x.0.same_album, x.1, std::cmp::Reverse(x.2))
        };
        k(a).cmp(&k(b)).then_with(|| a.0.track_id.cmp(&b.0.track_id))
    });
    Ok(out.into_iter().map(|x| x.0).collect())
}

/// The best match for `key` among songs `keep` allows (another server's,
/// say), if any.
pub fn best(
    c: &Connection,
    key: &SongKey,
    ctx: Context,
    ranks: &HashMap<String, Rank>,
    keep: impl Fn(&str) -> bool,
) -> AppResult<Option<Candidate>> {
    Ok(lookup(c, key, ctx, ranks)?.into_iter().find(|x| keep(&x.track_id)))
}
