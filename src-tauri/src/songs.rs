//! Every song in the library as one table (Your Library > Songs): a page of
//! rows in any column's order, the count and running time of what's in
//! view, and the ids to play from a row. Albums the tag or server view
//! hides, fan and AI edits (unless shown), hidden songs and songs by hidden
//! artists are left out, as on every other list.

use rusqlite::{Connection, params_from_iter};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::DbHandle;
use crate::db::merge::ServerView;
use crate::db::store::{TRACK_COLS, TRACK_NCOLS, row_track};
use crate::error::AppResult;
use crate::hidden::HiddenSet;
use crate::model::Track;
use crate::tags::TagView;

/// Rows one page holds at most.
pub const MAX_PAGE: u32 = 500;
/// Songs a play from the table queues: this many before the row picked and
/// the rest after it, so Previous still steps back.
pub const PLAY_BEFORE: usize = 500;
pub const PLAY_MAX: usize = 2_000;
/// The `tracks` context id a play from the table uses, and the queue's name for it.
pub const CONTEXT_ID: &str = "songs";
pub const TITLE: &str = "Your songs";

/// The column the table is in order of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SongSort {
    #[default]
    Title,
    Artist,
    Album,
    Year,
    Time,
    Plays,
    Added,
    LastPlayed,
    Rating,
    TrackNumber,
    Format,
}

/// What the table shows: a column's order, and words to find.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SongsQuery {
    #[serde(default)]
    pub sort: SongSort,
    #[serde(default)]
    pub desc: bool,
    /// Words that must each be in the title, artist or album.
    #[serde(default)]
    pub find: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongRow {
    pub track: Track,
    pub year: Option<i32>,
    /// When the album came into the library (unix seconds).
    pub added_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongsPage {
    pub rows: Vec<SongRow>,
    /// Songs in view, and their running time.
    pub total: u64,
    pub total_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SongsPlay {
    pub track_ids: Vec<String>,
}

/// The column's value, and the tie-breaks after it (always ascending).
fn order(q: &SongsQuery) -> String {
    let dir = if q.desc { "DESC" } else { "ASC" };
    let album = "al.sort_title COLLATE NOCASE, al.pk, t.disc_number, t.track_number, t.pk";
    // Songs with nothing in the column go last either way.
    let (key, then) = match q.sort {
        SongSort::Title => ("t.title COLLATE NOCASE", "t.artist_title COLLATE NOCASE, t.pk"),
        SongSort::Artist => (
            "coalesce(nullif(t.track_artist, ''), t.artist_title) COLLATE NOCASE",
            "al.year, al.sort_title COLLATE NOCASE, al.pk, t.disc_number, t.track_number, t.pk",
        ),
        SongSort::Album => ("al.sort_title COLLATE NOCASE", "al.pk, t.disc_number, t.track_number, t.pk"),
        SongSort::Year => ("al.year", album),
        SongSort::Time => ("t.duration_ms", "t.title COLLATE NOCASE, t.pk"),
        SongSort::Plays => ("t.view_count", "t.title COLLATE NOCASE, t.pk"),
        SongSort::Added => ("al.added_at", album),
        SongSort::LastPlayed => ("t.last_viewed_at", "t.title COLLATE NOCASE, t.pk"),
        SongSort::Rating => ("t.user_rating", "t.title COLLATE NOCASE, t.pk"),
        SongSort::TrackNumber => ("t.track_number", "t.title COLLATE NOCASE, t.pk"),
        // Higher resolution after lower: depth, then rate, then bitrate.
        SongSort::Format => (
            "coalesce(t.bit_depth, 0) * 1000000000 + coalesce(t.sample_rate, 0) * 10 + coalesce(t.bitrate, 0) / 10000.0",
            "t.title COLLATE NOCASE, t.pk",
        ),
    };
    let empty = match q.sort {
        SongSort::Year => "al.year IS NULL, ",
        SongSort::LastPlayed => "t.last_viewed_at IS NULL, ",
        SongSort::Rating => "t.user_rating IS NULL, ",
        SongSort::Format => "(t.bit_depth IS NULL AND t.sample_rate IS NULL AND t.bitrate IS NULL), ",
        _ => "",
    };
    format!("{empty}{key} {dir}, {then}")
}

/// The rows in view, and the values the `?` marks in it take.
fn filter(q: &SongsQuery, view: TagView, server: &ServerView, hidden: &HiddenSet) -> (String, Vec<String>) {
    let mut sql = format!(
        "{} AND {} AND t.id NOT IN (SELECT value FROM json_each(?)) AND t.artist_id NOT IN (SELECT value FROM json_each(?))",
        crate::tags::store::visible_clause(view),
        server.clause("al", "album"),
    );
    let mut args = vec![
        serde_json::to_string(&hidden.tracks).unwrap_or_else(|_| "[]".into()),
        serde_json::to_string(&hidden.artists).unwrap_or_else(|_| "[]".into()),
    ];
    if !view.show_edits {
        sql.push_str(
            " AND NOT EXISTS (SELECT 1 FROM file_tags f WHERE f.part_key = t.part_key AND f.status = 'ok'
                AND (',' || lower(replace(coalesce(f.flags, ''), ' ', '')) || ',' LIKE '%,fanedit,%'
                     OR ',' || lower(replace(coalesce(f.flags, ''), ' ', '')) || ',' LIKE '%,aiedit,%'))",
        );
    }
    for word in q.find.split_whitespace().take(8) {
        let like = format!("%{}%", word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        sql.push_str(
            " AND (t.title LIKE ? ESCAPE '\\' OR t.artist_title LIKE ? ESCAPE '\\'
                   OR coalesce(t.track_artist, '') LIKE ? ESCAPE '\\' OR al.title LIKE ? ESCAPE '\\')",
        );
        args.extend(std::iter::repeat_n(like, 4));
    }
    (sql, args)
}

const FROM: &str = "FROM tracks t JOIN albums al ON al.id = t.album_id";

/// One page of the table, with the count and running time of everything in view.
pub fn page(
    c: &Connection,
    q: &SongsQuery,
    offset: u32,
    limit: u32,
    view: TagView,
    server: &ServerView,
    hidden: &HiddenSet,
) -> AppResult<SongsPage> {
    let (wh, args) = filter(q, view, server, hidden);
    let limit = limit.clamp(1, MAX_PAGE);
    let sql = format!(
        "SELECT {TRACK_COLS}, al.year, al.added_at {FROM} WHERE {wh} ORDER BY {} LIMIT {limit} OFFSET {offset}",
        order(q)
    );
    let mut st = c.prepare_cached(&sql)?;
    let rows = st
        .query_map(params_from_iter(args.iter()), |r| {
            Ok(SongRow { track: row_track(r)?, year: r.get(TRACK_NCOLS)?, added_at: r.get(TRACK_NCOLS + 1)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let (total, total_ms): (i64, i64) = c
        .prepare_cached(&format!("SELECT count(*), coalesce(sum(t.duration_ms), 0) {FROM} WHERE {wh}"))?
        .query_row(params_from_iter(args.iter()), |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(SongsPage { rows, total: total as u64, total_ms: total_ms as u64 })
}

/// The ids to play from `start`: the table's order around it, at most
/// [`PLAY_MAX`] songs with up to [`PLAY_BEFORE`] before it. From the top
/// when `start` isn't in view.
pub fn play_ids(
    c: &Connection,
    q: &SongsQuery,
    start: Option<&str>,
    view: TagView,
    server: &ServerView,
    hidden: &HiddenSet,
) -> AppResult<Vec<String>> {
    let (wh, args) = filter(q, view, server, hidden);
    let mut st = c.prepare_cached(&format!("SELECT t.id {FROM} WHERE {wh} ORDER BY {}", order(q)))?;
    let ids: Vec<String> = st.query_map(params_from_iter(args.iter()), |r| r.get(0))?.collect::<Result<_, _>>()?;
    let at = start.and_then(|s| ids.iter().position(|i| i == s)).unwrap_or(0);
    let from = at.saturating_sub(PLAY_BEFORE);
    Ok(ids.into_iter().skip(from).take(PLAY_MAX).collect())
}

/// `songs_page`: one page of Your Library > Songs.
#[tauri::command]
pub async fn songs_page(
    db: State<'_, DbHandle>,
    hidden: State<'_, crate::hidden::Hidden>,
    query: SongsQuery,
    offset: Option<u32>,
    limit: Option<u32>,
) -> AppResult<SongsPage> {
    let (view, server, hidden) = (db.tag_view(), db.server_view(), hidden.ids().get());
    let (offset, limit) = (offset.unwrap_or(0), limit.unwrap_or(200));
    db.read_async(move |c| page(c, &query, offset, limit, view, &server, &hidden)).await
}

/// `songs_play_ids`: what plays from a row of the table, in its order.
#[tauri::command]
pub async fn songs_play_ids(
    db: State<'_, DbHandle>,
    hidden: State<'_, crate::hidden::Hidden>,
    query: SongsQuery,
    start_track_id: Option<String>,
) -> AppResult<SongsPlay> {
    let (view, server, hidden) = (db.tag_view(), db.server_view(), hidden.ids().get());
    let track_ids = db
        .read_async(move |c| play_ids(c, &query, start_track_id.as_deref(), view, &server, &hidden))
        .await?;
    Ok(SongsPlay { track_ids })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// Two albums (b: 1999, added first; a: 2005) with three songs each,
    /// and one album with no year.
    fn db() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        crate::db::schema::migrate(&mut c).unwrap();
        for (id, title, artist, year, added) in [
            ("a", "Alpha", "Xavier", Some(2005), 200),
            ("b", "Bravo", "Yolanda", Some(1999), 100),
            ("c", "Charlie", "Zed", None, 300),
        ] {
            c.execute(
                "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, year, added_at) VALUES (?1, ?2, ?2, ?3, ?3, ?4, ?5)",
                params![id, title, artist, year, added],
            )
            .unwrap();
        }
        // (id, title, album, artist, n, ms, plays, last played)
        let songs = [
            ("a1", "dawn", "a", "Xavier", 1, 3000, 5, Some(50)),
            ("a2", "Echo", "a", "Xavier", 2, 1000, 0, None),
            ("a3", "fog", "a", "Xavier", 3, 2000, 9, Some(90)),
            ("b1", "Glow", "b", "Yolanda", 1, 4000, 1, Some(10)),
            ("b2", "haze", "b", "Yolanda", 2, 6000, 0, None),
            ("c1", "Iris", "c", "Zed", 1, 5000, 2, Some(20)),
        ];
        for (id, title, album, artist, n, ms, plays, last) in songs {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, duration_ms, track_number, view_count, last_viewed_at)
                 VALUES (?1, ?2, ?3, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id, title, album, artist, format!("/p/{id}"), ms, n, plays, last],
            )
            .unwrap();
        }
        c
    }

    fn ids(c: &Connection, sort: SongSort, desc: bool) -> Vec<String> {
        let q = SongsQuery { sort, desc, find: String::new() };
        page(c, &q, 0, 50, TagView::default(), &ServerView::All, &HiddenSet::default())
            .unwrap()
            .rows
            .into_iter()
            .map(|r| r.track.id)
            .collect()
    }

    #[test]
    fn sorts_by_each_column_both_ways() {
        let c = db();
        assert_eq!(ids(&c, SongSort::Title, false), ["a1", "a2", "a3", "b1", "b2", "c1"]);
        assert_eq!(ids(&c, SongSort::Title, true), ["c1", "b2", "b1", "a3", "a2", "a1"]);
        assert_eq!(ids(&c, SongSort::Time, false), ["a2", "a3", "a1", "b1", "c1", "b2"]);
        assert_eq!(ids(&c, SongSort::Plays, true), ["a3", "a1", "c1", "b1", "a2", "b2"]);
        assert_eq!(ids(&c, SongSort::Album, true), ["c1", "b1", "b2", "a1", "a2", "a3"]);
        assert_eq!(ids(&c, SongSort::Added, true), ["c1", "a1", "a2", "a3", "b1", "b2"]);
        assert_eq!(ids(&c, SongSort::Artist, false), ["a1", "a2", "a3", "b1", "b2", "c1"]);
        assert_eq!(ids(&c, SongSort::TrackNumber, false), ["a1", "b1", "c1", "a2", "b2", "a3"]);
    }

    #[test]
    fn songs_with_nothing_in_the_column_go_last_either_way() {
        let c = db();
        assert_eq!(ids(&c, SongSort::Year, false), ["b1", "b2", "a1", "a2", "a3", "c1"]);
        assert_eq!(ids(&c, SongSort::Year, true), ["a1", "a2", "a3", "b1", "b2", "c1"]);
        assert_eq!(ids(&c, SongSort::LastPlayed, true), ["a3", "a1", "c1", "b1", "a2", "b2"]);
        assert_eq!(ids(&c, SongSort::LastPlayed, false), ["b1", "c1", "a1", "a3", "a2", "b2"]);
    }

    #[test]
    fn pages_and_totals_cover_everything_in_view() {
        let c = db();
        let q = SongsQuery::default();
        let p = page(&c, &q, 2, 3, TagView::default(), &ServerView::All, &HiddenSet::default()).unwrap();
        assert_eq!(p.rows.iter().map(|r| r.track.id.as_str()).collect::<Vec<_>>(), ["a3", "b1", "b2"]);
        assert_eq!((p.total, p.total_ms), (6, 21_000));
        assert_eq!(p.rows[1].year, Some(1999));
        assert_eq!(p.rows[1].added_at, 100);
    }

    #[test]
    fn finds_by_title_artist_or_album_with_every_word() {
        let c = db();
        let find = |s: &str| {
            let q = SongsQuery { find: s.into(), ..Default::default() };
            let p = page(&c, &q, 0, 50, TagView::default(), &ServerView::All, &HiddenSet::default()).unwrap();
            (p.rows.into_iter().map(|r| r.track.id).collect::<Vec<_>>(), p.total)
        };
        assert_eq!(find("GLOW").0, ["b1"]);
        assert_eq!(find("bravo").0, ["b1", "b2"]);
        assert_eq!(find("bravo haze"), (vec!["b2".to_string()], 1));
        assert_eq!(find("zed").0, ["c1"]);
        // Wildcards are plain letters.
        assert!(find("%").0.is_empty());
        assert!(find("_").0.is_empty());
    }

    #[test]
    fn hidden_songs_artists_and_edits_stay_out() {
        let c = db();
        let mut hidden = HiddenSet::default();
        hidden.tracks.insert("a2".into());
        hidden.artists.insert("Yolanda".into());
        c.execute(
            "INSERT INTO file_tags (part_key, status, flags, scanned_at) VALUES ('/p/c1', 'ok', 'live, FanEdit', 1)",
            [],
        )
        .unwrap();
        let q = SongsQuery::default();
        let p = page(&c, &q, 0, 50, TagView::default(), &ServerView::All, &hidden).unwrap();
        let got: Vec<_> = p.rows.iter().map(|r| r.track.id.as_str()).collect();
        assert_eq!(got, ["a1", "a3"]);
        assert_eq!(p.total, 2);
        let shown = TagView { show_edits: true, ..TagView::default() };
        let p = page(&c, &q, 0, 50, shown, &ServerView::All, &hidden).unwrap();
        assert_eq!(p.rows.iter().map(|r| r.track.id.as_str()).collect::<Vec<_>>(), ["a1", "a3", "c1"]);
    }

    #[test]
    fn plays_from_the_row_in_the_table_order() {
        let c = db();
        let q = SongsQuery { sort: SongSort::Time, ..Default::default() };
        let got = play_ids(&c, &q, Some("b1"), TagView::default(), &ServerView::All, &HiddenSet::default()).unwrap();
        assert_eq!(got, ["a2", "a3", "a1", "b1", "c1", "b2"]);
        // Not in view: from the top.
        let got = play_ids(&c, &q, Some("zzz"), TagView::default(), &ServerView::All, &HiddenSet::default()).unwrap();
        assert_eq!(got[0], "a2");
    }

    #[test]
    fn a_long_table_plays_a_window_around_the_row() {
        let mut c = Connection::open_in_memory().unwrap();
        crate::db::schema::migrate(&mut c).unwrap();
        c.execute("INSERT INTO albums (id, title, sort_title, artist_id, artist_title) VALUES ('x', 'x', 'x', 'x', 'x')", [])
            .unwrap();
        let tx = c.transaction().unwrap();
        for i in 0..3000 {
            tx.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, track_number)
                 VALUES (?1, ?1, 'x', 'x', 'x', 'x', ?1, ?2)",
                params![format!("s{i:04}"), i],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let q = SongsQuery::default();
        let got = play_ids(&c, &q, Some("s1200"), TagView::default(), &ServerView::All, &HiddenSet::default()).unwrap();
        assert_eq!(got.len(), PLAY_MAX);
        assert_eq!(got[0], "s0700");
        assert_eq!(got[PLAY_BEFORE], "s1200");
    }
}
