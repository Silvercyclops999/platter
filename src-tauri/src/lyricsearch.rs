//! Find a song by a line of its lyrics. Search also looks in the lyrics this
//! computer already has: the ones a song showed (fetched, written or
//! transcribed) are noted in `lyrics_index` as they are shown or saved.
//! Nothing is ever fetched just to search.
//!
//! A lyric match needs three or more words, or words in quotes, so short
//! searches don't fill with lyric noise. Matches come after the songs found
//! by title and name (db::commands::search).
//!
//! The index is made the first time it's needed, filled from the user's own
//! lyrics (user_lyrics), and made again from them when it is damaged.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::types::SearchResults;
use crate::db::{DbHandle, searchtext};
use crate::error::AppResult;
use crate::plex::details::Lyrics;

/// Words a lyric search needs, unless they are in quotes.
pub const MIN_WORDS: usize = 3;
/// Words read from the query; more don't make a line easier to find.
const MAX_WORDS: usize = 12;
/// Characters of the matching line shown under the song.
const LINE_MAX: usize = 120;

const TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS lyrics_index (
  pk INTEGER PRIMARY KEY,
  track_id TEXT NOT NULL UNIQUE,
  auto INTEGER NOT NULL DEFAULT 0,
  body TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS lyrics_index_fts USING fts5 (body, content = 'lyrics_index', content_rowid = 'pk', tokenize = "unicode61 remove_diacritics 2");
CREATE TRIGGER IF NOT EXISTS lyrics_index_ai AFTER INSERT ON lyrics_index BEGIN
  INSERT INTO lyrics_index_fts (rowid, body) VALUES (new.pk, new.body);
END;
CREATE TRIGGER IF NOT EXISTS lyrics_index_ad AFTER DELETE ON lyrics_index BEGIN
  INSERT INTO lyrics_index_fts (lyrics_index_fts, rowid, body) VALUES ('delete', old.pk, old.body);
END;
CREATE TRIGGER IF NOT EXISTS lyrics_index_au AFTER UPDATE OF body ON lyrics_index BEGIN
  INSERT INTO lyrics_index_fts (lyrics_index_fts, rowid, body) VALUES ('delete', old.pk, old.body);
  INSERT INTO lyrics_index_fts (rowid, body) VALUES (new.pk, new.body);
END;
"#;

const DROP: &str = "
DROP TRIGGER IF EXISTS lyrics_index_ai;
DROP TRIGGER IF EXISTS lyrics_index_ad;
DROP TRIGGER IF EXISTS lyrics_index_au;
DROP TABLE IF EXISTS lyrics_index_fts;
DROP TABLE IF EXISTS lyrics_index;
";

/// Every piece of the index is there: both tables and the three triggers
/// that keep the text index in step.
fn exists(c: &Connection) -> AppResult<bool> {
    let n: i64 = c.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name IN
           ('lyrics_index', 'lyrics_index_fts', 'lyrics_index_ai', 'lyrics_index_ad', 'lyrics_index_au')",
        [],
        |r| r.get(0),
    )?;
    Ok(n == 5)
}

/// Run `f` as one step: all of it or none (a savepoint, so it nests in a
/// transaction a caller already has open).
fn whole<T>(c: &Connection, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
    c.execute_batch("SAVEPOINT lyrics_index_make")?;
    match f(c) {
        Ok(v) => {
            c.execute_batch("RELEASE lyrics_index_make")?;
            Ok(v)
        }
        Err(e) => {
            let _ = c.execute_batch("ROLLBACK TO lyrics_index_make; RELEASE lyrics_index_make");
            Err(e)
        }
    }
}

/// Make the index when it isn't all there, filled from the user's own
/// lyrics. Rows already noted keep their place in the text index.
pub fn ensure(c: &Connection) -> AppResult<()> {
    if exists(c)? {
        return Ok(());
    }
    whole(c, |c| {
        c.execute_batch(TABLES)?;
        c.execute_batch("INSERT INTO lyrics_index_fts (lyrics_index_fts) VALUES ('rebuild')")?;
        backfill(c)
    })
}

/// Mend the index after a lookup failed: make what's missing, else rebuild
/// the text index from the noted lyrics, and only when that fails too,
/// start again from the user's own lyrics.
pub fn repair(c: &Connection) -> AppResult<()> {
    if !exists(c)? {
        return ensure(c);
    }
    let mended = whole(c, |c| Ok(c.execute_batch("INSERT INTO lyrics_index_fts (lyrics_index_fts) VALUES ('rebuild')")?));
    if mended.is_ok() {
        return Ok(());
    }
    whole(c, |c| {
        c.execute_batch(DROP)?;
        c.execute_batch(TABLES)?;
        backfill(c)
    })
}

/// Only the user's own lyrics: everything else fills in as songs are shown,
/// so nothing is fetched just for search.
fn backfill(c: &Connection) -> AppResult<()> {
    for id in crate::lyrics::user::ids(c)? {
        if let Some(own) = crate::lyrics::user::get(c, &id)?
            && let Some(l) = own.lyrics()
        {
            put(c, &id, &l, own.auto.is_some())?;
        }
    }
    Ok(())
}

/// The words of the lyrics, one line per line, without timing or breaks.
pub fn body(l: &Lyrics) -> String {
    let lines: Vec<&str> = if l.lines.iter().any(|s| !s.trim().is_empty()) {
        l.lines.iter().map(String::as_str).collect()
    } else {
        l.timed.iter().map(|t| t.text.as_str()).collect()
    };
    lines.into_iter().map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n")
}

/// True when the index already holds exactly these lyrics for the song (a
/// read, so a view that changes nothing doesn't wait for the writer).
pub fn holds(c: &Connection, track_id: &str, l: &Lyrics, auto: bool) -> bool {
    exists(c).unwrap_or(false) && same(c, track_id, &body(l), auto).unwrap_or(false)
}

fn same(c: &Connection, track_id: &str, body: &str, auto: bool) -> AppResult<bool> {
    let same: Option<bool> = c
        .query_row("SELECT body = ?2 AND auto = ?3 FROM lyrics_index WHERE track_id = ?1", params![track_id, body, auto as i64], |r| r.get(0))
        .optional()?;
    Ok(same == Some(true))
}

fn put(c: &Connection, track_id: &str, l: &Lyrics, auto: bool) -> AppResult<()> {
    let body = body(l);
    if body.is_empty() {
        return forget_in(c, track_id);
    }
    if same(c, track_id, &body, auto)? {
        return Ok(());
    }
    c.execute(
        "INSERT INTO lyrics_index (track_id, auto, body) VALUES (?1, ?2, ?3)
         ON CONFLICT (track_id) DO UPDATE SET auto = excluded.auto, body = excluded.body",
        params![track_id, auto as i64, body],
    )?;
    Ok(())
}

fn forget_in(c: &Connection, track_id: &str) -> AppResult<()> {
    c.execute("DELETE FROM lyrics_index WHERE track_id = ?1", params![track_id])?;
    Ok(())
}

/// Note the lyrics a song has. `auto`: transcribed on a computer, matched
/// only while transcribed lyrics are shown.
pub fn note(c: &Connection, track_id: &str, l: &Lyrics, auto: bool) -> AppResult<()> {
    ensure(c)?;
    put(c, track_id, l, auto)
}

/// [`note`], for callers whose own work must not fail when the index does.
pub fn note_or_log(c: &Connection, track_id: &str, l: &Lyrics, auto: bool) {
    if let Err(e) = note(c, track_id, l, auto) {
        tracing::warn!("lyric search index: {}", e.kind());
    }
}

/// [`forget`], logging instead of failing.
pub fn forget_or_log(c: &Connection, track_id: &str) {
    if let Err(e) = forget(c, track_id) {
        tracing::warn!("lyric search index: {}", e.kind());
    }
}

/// The song has no lyrics any more (the user picked none, or deleted theirs).
pub fn forget(c: &Connection, track_id: &str) -> AppResult<()> {
    if !exists(c)? {
        return Ok(());
    }
    forget_in(c, track_id)
}

/// What a query asks the lyrics for: its words in order, the last one still
/// being typed unless the words were quoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricQuery {
    pieces: Vec<String>,
    prefix: bool,
}

impl LyricQuery {
    /// None when the query is too short to look in lyrics.
    pub fn parse(raw: &str) -> Option<LyricQuery> {
        // Words in quotes once the quote is closed; an open quote is just typing.
        let quotes = raw.matches('"').count();
        let quoted = if quotes >= 2 { raw.split('"').nth(1).filter(|q| !q.trim().is_empty()) } else { None };
        let text = quoted.unwrap_or(raw);
        let words: Vec<searchtext::Word> = searchtext::words(text).into_iter().filter(|w| !w.pieces.is_empty()).take(MAX_WORDS).collect();
        if words.is_empty() || (quoted.is_none() && words.len() < MIN_WORDS) {
            return None;
        }
        // The last word is still being typed: not in quotes or after a space.
        let prefix = quoted.is_none() && !raw.ends_with(char::is_whitespace);
        let pieces = words.into_iter().flat_map(|w| w.pieces).collect();
        Some(LyricQuery { pieces, prefix })
    }

    fn fts(&self) -> String {
        let phrase = format!("\"{}\"", self.pieces.join(" ").replace('"', ""));
        if self.prefix { format!("body : {phrase}*") } else { format!("body : {phrase}") }
    }
}

/// One song found by its lyrics, with the line that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub track_id: String,
    pub line: String,
}

/// Songs whose lyrics hold the words, best first. Transcribed lyrics only
/// when `with_auto`.
pub fn find(c: &Connection, q: &LyricQuery, with_auto: bool, limit: usize) -> AppResult<Vec<Match>> {
    if !exists(c)? {
        return Err(crate::error::AppError::Internal("lyrics index missing".into()));
    }
    let mut st = c.prepare_cached(
        "SELECT i.track_id, i.body FROM lyrics_index_fts f JOIN lyrics_index i ON i.pk = f.rowid
         WHERE lyrics_index_fts MATCH ?1 AND (?2 OR i.auto = 0)
         ORDER BY bm25(lyrics_index_fts) LIMIT ?3",
    )?;
    let rows = st.query_map(params![q.fts(), with_auto, limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (track_id, body) = row?;
        out.push(Match { line: line_for(&body, q), track_id });
    }
    Ok(out)
}

/// Indexes being mended, by database file, so a broken one isn't mended
/// again on every keystroke.
static MENDING: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Add the songs whose lyrics hold the query's words to `out.songs`, after
/// the ones already there, up to `room` more. Returns the songs that were
/// found by name. A search never fails or waits for lyrics: a missing or
/// damaged index finds nothing this time and is mended in the background.
pub async fn add_songs(db: &DbHandle, out: &mut SearchResults, query: &str, with_auto: bool, room: usize) -> HashSet<String> {
    let have: HashSet<String> = out.songs.iter().map(|t| t.id.clone()).collect();
    let Some(q) = LyricQuery::parse(query).filter(|_| room > 0) else {
        return have;
    };
    let want = room + have.len();
    let found = match db.read_async(move |c| find(c, &q, with_auto, want)).await {
        Ok(f) => f,
        Err(e) => {
            tracing::debug!("lyric search: {}", e.kind());
            mend(db);
            return have;
        }
    };
    let found: Vec<Match> = found.into_iter().filter(|m| !have.contains(&m.track_id)).take(room).collect();
    if found.is_empty() {
        return have;
    }
    let ids: Vec<String> = found.iter().map(|m| m.track_id.clone()).collect();
    let Ok(tracks) = db.read_async(move |c| crate::db::queries::tracks_by_ids(c, &ids)).await else {
        return have;
    };
    let mut lines: HashMap<String, String> = found.into_iter().map(|m| (m.track_id, m.line)).collect();
    for t in tracks {
        if let Some(line) = lines.remove(&t.id) {
            out.lyric_lines.insert(t.id.clone(), line);
            out.songs.push(t);
        }
    }
    have
}

/// Mend the index off the search's path, once at a time per database.
fn mend(db: &DbHandle) {
    let path = db.path().to_path_buf();
    {
        let Ok(mut m) = MENDING.lock() else { return };
        if m.contains(&path) {
            return;
        }
        m.push(path.clone());
    }
    let db = db.clone();
    tokio::spawn(async move {
        if let Err(e) = db.write_async(|c| repair(c)).await {
            tracing::warn!("lyric search index: {}", e.kind());
        }
        if let Ok(mut m) = MENDING.lock() {
            m.retain(|p| p != &path);
        }
    });
}

/// After songs are filtered and grouped by version: a line follows its song
/// to the version listed for it (unless that song was found by name), and
/// lines for songs no longer listed go.
pub fn tidy(out: &mut SearchResults, by_name: &HashSet<String>) {
    if out.lyric_lines.is_empty() {
        return;
    }
    let mut lines = std::mem::take(&mut out.lyric_lines);
    for s in &out.songs {
        let line = lines.remove(&s.id).or_else(|| {
            if by_name.contains(&s.id) {
                return None;
            }
            out.versions.get(&s.id).into_iter().flatten().find_map(|v| lines.remove(&v.id))
        });
        if let Some(line) = line {
            out.lyric_lines.insert(s.id.clone(), line);
        }
    }
}

/// The line (or lines, when the words run across a break) that matched.
pub fn line_for(body: &str, q: &LyricQuery) -> String {
    let lines: Vec<&str> = body.lines().collect();
    // Every piece of every line, with the line it is on.
    let mut all: Vec<(usize, String)> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        for w in searchtext::words(l) {
            all.extend(w.pieces.into_iter().map(|p| (i, p)));
        }
    }
    let n = q.pieces.len();
    let at = (0..all.len().saturating_sub(n - 1)).find(|&s| {
        q.pieces.iter().enumerate().all(|(k, p)| {
            let have = &all[s + k].1;
            if k + 1 == n && q.prefix { have.starts_with(p.as_str()) } else { have == p }
        })
    });
    let text = match at {
        Some(s) => lines[all[s].0..=all[s + n - 1].0].join(" / "),
        None => lines.first().copied().unwrap_or_default().to_string(),
    };
    if text.chars().count() > LINE_MAX {
        let mut t: String = text.chars().take(LINE_MAX - 1).collect();
        t.push('…');
        t
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::details::parse_lyrics_text;

    fn db() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        crate::db::schema::migrate(&mut c).unwrap();
        c
    }

    const RIVER: &str = "[00:01.00]We walked along the water\n[00:05.00]And the river took the light\n[00:09.00]Away from us tonight";

    fn ids(m: &[Match]) -> Vec<&str> {
        m.iter().map(|m| m.track_id.as_str()).collect()
    }

    #[test]
    fn a_phrase_finds_the_song_and_its_line() {
        let c = db();
        note(&c, "1", &parse_lyrics_text(RIVER), false).unwrap();
        note(&c, "2", &parse_lyrics_text("Nothing about water here\nOnly light"), false).unwrap();
        let q = LyricQuery::parse("the river took").unwrap();
        let m = find(&c, &q, true, 10).unwrap();
        assert_eq!(ids(&m), ["1"]);
        assert_eq!(m[0].line, "And the river took the light");
        // Still typing the last word; accents and case don't matter.
        let m = find(&c, &LyricQuery::parse("THE RÍVER to").unwrap(), true, 10).unwrap();
        assert_eq!(ids(&m), ["1"]);
        // Words across a line break show both lines.
        let m = find(&c, &LyricQuery::parse("the light away from").unwrap(), true, 10).unwrap();
        assert_eq!(m[0].line, "And the river took the light / Away from us tonight");
    }

    #[test]
    fn short_searches_dont_look_in_lyrics_unless_quoted() {
        assert!(LyricQuery::parse("river light").is_none());
        assert!(LyricQuery::parse("a").is_none());
        assert!(LyricQuery::parse("   ").is_none());
        let q = LyricQuery::parse("\"river\" songs").unwrap();
        assert_eq!(q, LyricQuery { pieces: vec!["river".into()], prefix: false });
        assert!(LyricQuery::parse("walk along the").is_some());
        // An open quote is just typing: the usual three words apply.
        assert!(LyricQuery::parse("\"a").is_none());
        assert_eq!(LyricQuery::parse("\"walk along th").unwrap(), LyricQuery { pieces: vec!["walk".into(), "along".into(), "th".into()], prefix: true });
    }

    #[test]
    fn transcribed_lyrics_match_only_while_they_are_shown() {
        let c = db();
        note(&c, "1", &parse_lyrics_text(RIVER), true).unwrap();
        let q = LyricQuery::parse("the river took").unwrap();
        assert_eq!(ids(&find(&c, &q, true, 10).unwrap()), ["1"]);
        assert!(find(&c, &q, false, 10).unwrap().is_empty());
    }

    #[test]
    fn changed_and_forgotten_lyrics_leave_the_index() {
        let c = db();
        note(&c, "1", &parse_lyrics_text(RIVER), false).unwrap();
        note(&c, "1", &parse_lyrics_text("Something else entirely now"), false).unwrap();
        assert!(find(&c, &LyricQuery::parse("the river took").unwrap(), true, 10).unwrap().is_empty());
        assert_eq!(ids(&find(&c, &LyricQuery::parse("something else entirely").unwrap(), true, 10).unwrap()), ["1"]);
        forget(&c, "1").unwrap();
        assert!(find(&c, &LyricQuery::parse("something else entirely").unwrap(), true, 10).unwrap().is_empty());
    }

    #[test]
    fn the_index_starts_from_the_users_own_lyrics_and_a_damaged_one_is_made_again() {
        let c = db();
        let q = LyricQuery::parse("the river took").unwrap();
        // Lyrics written before the index existed.
        c.execute("INSERT INTO user_lyrics (track_id, body, prefer, updated_at) VALUES ('7', ?1, 1, 1)", [RIVER]).unwrap();
        assert!(find(&c, &q, true, 10).is_err(), "no index yet");
        ensure(&c).unwrap();
        assert_eq!(ids(&find(&c, &q, true, 10).unwrap()), ["7"]);
        // Lyrics a song showed, then the text index is lost: mending keeps them.
        note(&c, "8", &parse_lyrics_text("[00:01.00]Across the river took the light"), false).unwrap();
        c.execute_batch("DROP TABLE lyrics_index_fts").unwrap();
        assert!(find(&c, &q, true, 10).is_err());
        repair(&c).unwrap();
        let mut found = ids(&find(&c, &q, true, 10).unwrap()).into_iter().map(String::from).collect::<Vec<_>>();
        found.sort();
        assert_eq!(found, ["7", "8"]);
        // A missing trigger is made again too, and the rows stay.
        c.execute_batch("DROP TRIGGER lyrics_index_ai").unwrap();
        repair(&c).unwrap();
        assert!(exists(&c).unwrap());
        let mut again = ids(&find(&c, &q, true, 10).unwrap()).into_iter().map(String::from).collect::<Vec<_>>();
        again.sort();
        assert_eq!(again, found);
    }

    #[test]
    fn saving_and_deleting_lyrics_keeps_the_index_in_step() {
        use crate::lyrics::user;
        let c = db();
        let q = LyricQuery::parse("the river took").unwrap();
        user::put(&c, "7", RIVER, true, 1).unwrap();
        assert_eq!(ids(&find(&c, &q, true, 10).unwrap()), ["7"]);
        user::delete(&c, "7").unwrap();
        assert!(find(&c, &q, true, 10).unwrap().is_empty());
        // A transcription is noted as one.
        assert!(user::put_auto(&c, "8", RIVER, "small", 1).unwrap());
        assert_eq!(ids(&find(&c, &q, true, 10).unwrap()), ["8"]);
        assert!(find(&c, &q, false, 10).unwrap().is_empty());
        // Re-timed text replaces it.
        assert!(user::retimed(&c, "8", RIVER, Some("[00:02.00]A different song entirely"), 9).unwrap());
        assert!(find(&c, &q, true, 10).unwrap().is_empty());
        assert_eq!(ids(&find(&c, &LyricQuery::parse("a different song").unwrap(), true, 10).unwrap()), ["8"]);
        // Removing a transcription takes it out too.
        crate::lyrics::transcribe::store::forget(&c, "8").unwrap();
        assert!(find(&c, &LyricQuery::parse("a different song").unwrap(), true, 10).unwrap().is_empty());
    }

    #[test]
    fn a_query_cant_break_out_of_its_phrase() {
        let c = db();
        note(&c, "1", &parse_lyrics_text(RIVER), false).unwrap();
        for q in ["\" OR body : *", "river) OR (took NEAR", "the - river : took"] {
            if let Some(q) = LyricQuery::parse(q) {
                find(&c, &q, true, 10).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn lyric_matches_fill_in_after_the_songs_found_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("t.db")).unwrap();
        db.write(|c| {
            c.execute_batch(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key)
                 VALUES ('1', 'The River Took It', 'a', 'A', 'r', 'R', 1000, '/p/1'),
                        ('2', 'Quiet Song', 'a', 'A', 'r', 'R', 1000, '/p/2'),
                        ('3', 'Another', 'a', 'A', 'r', 'R', 1000, '/p/3');",
            )?;
            crate::lyrics::user::put(c, "2", RIVER, true, 1)?;
            crate::lyrics::user::put(c, "1", RIVER, true, 1)
        })
        .unwrap();
        let by_name = db.read(|c| crate::db::queries::tracks_by_ids(c, &["1".into()])).unwrap();
        let mut out = SearchResults { songs: by_name.clone(), ..Default::default() };
        add_songs(&db, &mut out, "the river took", true, 3).await;
        assert_eq!(out.songs.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["1", "2"], "found by name first, then by lyrics, once each");
        assert_eq!(out.lyric_lines.keys().collect::<Vec<_>>(), ["2"]);
        assert_eq!(out.lyric_lines["2"], "And the river took the light");

        // No room, or too few words: nothing added.
        let mut out = SearchResults { songs: by_name.clone(), ..Default::default() };
        add_songs(&db, &mut out, "the river took", true, 0).await;
        add_songs(&db, &mut out, "river took", true, 3).await;
        assert_eq!(out.songs.len(), 1);

        // A damaged index finds nothing at once (the search doesn't wait),
        // and is mended in the background.
        db.write(|c| Ok(c.execute_batch("DROP TABLE lyrics_index_fts")?)).unwrap();
        let mut out = SearchResults::default();
        add_songs(&db, &mut out, "\"took the light\"", true, 3).await;
        assert!(out.songs.is_empty());
        let mut n = 0;
        while out.songs.len() < 2 && n < 100 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            out = SearchResults::default();
            add_songs(&db, &mut out, "\"took the light\"", true, 3).await;
            n += 1;
        }
        assert_eq!(out.songs.len(), 2);
    }

    #[test]
    fn a_line_follows_its_song_to_the_version_listed() {
        let mut t = crate::platform::media::tests::track("10");
        let mut out = SearchResults::default();
        out.lyric_lines.insert("11".into(), "a line".into());
        out.lyric_lines.insert("99".into(), "gone".into());
        out.versions.insert("10".into(), vec![crate::platform::media::tests::track("11")]);
        t.title = "Released".into();
        out.songs.push(t);
        let mut found_by_name = out.clone();
        tidy(&mut out, &HashSet::new());
        assert_eq!(out.lyric_lines.into_iter().collect::<Vec<_>>(), [("10".to_string(), "a line".to_string())]);
        // A song found by its name doesn't take a version's lyric line.
        tidy(&mut found_by_name, &HashSet::from(["10".to_string()]));
        assert!(found_by_name.lyric_lines.is_empty());
    }
}
