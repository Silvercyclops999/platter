//! Connection setup and versioned migrations (`PRAGMA user_version`).

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::AppResult;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Open the single writer connection: WAL, foreign keys, busy timeout.
pub fn open_writer(path: &Path) -> AppResult<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let _mode: String =
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "cache_size", -16_000)?; // ~16 MB
    Ok(conn)
}

/// Open a read-only connection for the read pool.
pub fn open_reader(path: &Path) -> AppResult<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "cache_size", -16_000)?;
    Ok(conn)
}

/// Apply every migration newer than the stored `user_version`.
pub fn migrate(conn: &mut Connection) -> AppResult<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = i as i64 + 1;
        if version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        tracing::info!(version, "db migrated");
    }
    Ok(())
}

pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// A database as an older build left it: migrations up to `version` only.
#[cfg(test)]
pub(crate) fn migrate_to(conn: &mut Connection, version: usize) -> AppResult<()> {
    for (i, sql) in MIGRATIONS[..version].iter().enumerate() {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

const MIGRATIONS: &[&str] = &[
    // v1 — library cache, search index, listen log, settings.
    // FTS tables: case- and diacritic-insensitive, prefix indexes for
    // search-as-you-type.
    r#"
CREATE TABLE artists (
    pk              INTEGER PRIMARY KEY,
    id              TEXT NOT NULL UNIQUE,
    title           TEXT NOT NULL,
    sort_title      TEXT NOT NULL,
    thumb           TEXT,
    art             TEXT,
    view_count      INTEGER NOT NULL DEFAULT 0,
    latest_added_at INTEGER NOT NULL DEFAULT 0,
    seen_gen        INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX artists_sort ON artists (sort_title COLLATE NOCASE);
CREATE INDEX artists_plays ON artists (view_count DESC);
CREATE INDEX artists_recent ON artists (latest_added_at DESC);

CREATE TABLE albums (
    pk             INTEGER PRIMARY KEY,
    id             TEXT NOT NULL UNIQUE,
    title          TEXT NOT NULL,
    sort_title     TEXT NOT NULL,
    artist_id      TEXT NOT NULL,
    artist_title   TEXT NOT NULL,
    year           INTEGER,
    thumb          TEXT,
    format         TEXT,
    subformats     TEXT NOT NULL DEFAULT '[]',
    section        TEXT NOT NULL DEFAULT 'albums',
    section_rank   INTEGER NOT NULL DEFAULT 0,
    genres         TEXT NOT NULL DEFAULT '[]',
    track_count    INTEGER NOT NULL DEFAULT 0,
    added_at       INTEGER NOT NULL DEFAULT 0,
    view_count     INTEGER NOT NULL DEFAULT 0,
    last_viewed_at INTEGER,
    tint           TEXT,
    seen_gen       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX albums_sort ON albums (sort_title COLLATE NOCASE);
CREATE INDEX albums_artist_sort ON albums (artist_title COLLATE NOCASE, year, sort_title COLLATE NOCASE);
CREATE INDEX albums_artist ON albums (artist_id, section_rank, year DESC);
CREATE INDEX albums_year ON albums (year DESC);
CREATE INDEX albums_added ON albums (added_at DESC);
CREATE INDEX albums_plays ON albums (view_count DESC);
CREATE INDEX albums_section ON albums (section, sort_title COLLATE NOCASE);

CREATE TABLE tracks (
    pk             INTEGER PRIMARY KEY,
    id             TEXT NOT NULL UNIQUE,
    title          TEXT NOT NULL,
    album_id       TEXT NOT NULL,
    album_title    TEXT NOT NULL,
    artist_id      TEXT NOT NULL,
    artist_title   TEXT NOT NULL,
    track_artist   TEXT,
    track_number   INTEGER NOT NULL DEFAULT 0,
    disc_number    INTEGER NOT NULL DEFAULT 1,
    duration_ms    INTEGER NOT NULL DEFAULT 0,
    thumb          TEXT,
    part_key       TEXT NOT NULL,
    codec          TEXT,
    container      TEXT,
    bitrate        INTEGER,
    bit_depth      INTEGER,
    sample_rate    INTEGER,
    user_rating    REAL,
    view_count     INTEGER NOT NULL DEFAULT 0,
    last_viewed_at INTEGER,
    seen_gen       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX tracks_album ON tracks (album_id, disc_number, track_number);
CREATE INDEX tracks_artist_plays ON tracks (artist_id, view_count DESC);

CREATE TABLE playlists (
    pk          INTEGER PRIMARY KEY,
    id          TEXT NOT NULL UNIQUE,
    title       TEXT NOT NULL,
    smart       INTEGER NOT NULL DEFAULT 0,
    track_count INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    thumb       TEXT,
    seen_gen    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE playlist_items (
    playlist_id TEXT NOT NULL REFERENCES playlists (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    track_id    TEXT NOT NULL,
    PRIMARY KEY (playlist_id, position)
) WITHOUT ROWID;

-- Plex play history (/status/sessions/history/all): feeds Home.
CREATE TABLE play_history (
    rating_key TEXT NOT NULL,
    viewed_at  INTEGER NOT NULL,
    PRIMARY KEY (rating_key, viewed_at)
) WITHOUT ROWID;
CREATE INDEX play_history_at ON play_history (viewed_at DESC);

CREATE TABLE sync_state (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    section_id        TEXT NOT NULL,
    section_uuid      TEXT NOT NULL,
    section_title     TEXT,
    last_sync_at      INTEGER,
    last_full_sync_at INTEGER,
    generation        INTEGER NOT NULL DEFAULT 0
);

-- Spec 06 local listen log. Times are unix ms.
CREATE TABLE listen_log (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    rating_key        TEXT NOT NULL,
    part_key          TEXT,
    started_at        INTEGER NOT NULL,
    ended_at          INTEGER,
    duration_ms       INTEGER NOT NULL DEFAULT 0,
    ms_listened       INTEGER NOT NULL DEFAULT 0,
    completed         INTEGER NOT NULL DEFAULT 0,
    skipped           INTEGER NOT NULL DEFAULT 0,
    skipped_at_ms     INTEGER,
    context_kind      TEXT,
    context_id        TEXT,
    device            TEXT,
    offline           INTEGER NOT NULL DEFAULT 0,
    scrobbled         INTEGER NOT NULL DEFAULT 0,
    reported_to_plex  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX listen_log_started ON listen_log (started_at DESC);
CREATE INDEX listen_log_key ON listen_log (rating_key);

-- Timeline / scrobble calls waiting to be sent (offline replay), in order.
CREATE TABLE report_queue (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    play_id     INTEGER REFERENCES listen_log (id) ON DELETE SET NULL,
    kind        TEXT NOT NULL,
    rating_key  TEXT NOT NULL,
    state       TEXT,
    time_ms     INTEGER,
    duration_ms INTEGER,
    at          INTEGER NOT NULL,
    sent_at     INTEGER
);
CREATE INDEX report_queue_pending ON report_queue (sent_at, at, id);

CREATE TABLE recent_searches (
    query      TEXT PRIMARY KEY,
    norm       TEXT NOT NULL UNIQUE,
    searched_at INTEGER NOT NULL
);

CREATE TABLE kv (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Search index: external-content FTS5 tables kept in step by triggers.
CREATE VIRTUAL TABLE artists_fts USING fts5 (title, content = 'artists', content_rowid = 'pk', tokenize = "unicode61 remove_diacritics 2", prefix = '1 2 3');
CREATE VIRTUAL TABLE albums_fts USING fts5 (title, artist_title, content = 'albums', content_rowid = 'pk', tokenize = "unicode61 remove_diacritics 2", prefix = '1 2 3');
CREATE VIRTUAL TABLE tracks_fts USING fts5 (title, artist_title, album_title, track_artist, content = 'tracks', content_rowid = 'pk', tokenize = "unicode61 remove_diacritics 2", prefix = '1 2 3');
CREATE VIRTUAL TABLE playlists_fts USING fts5 (title, content = 'playlists', content_rowid = 'pk', tokenize = "unicode61 remove_diacritics 2", prefix = '1 2 3');

CREATE VIRTUAL TABLE artists_vocab USING fts5vocab (artists_fts, row);
CREATE VIRTUAL TABLE albums_vocab USING fts5vocab (albums_fts, row);
CREATE VIRTUAL TABLE tracks_vocab USING fts5vocab (tracks_fts, row);
CREATE VIRTUAL TABLE playlists_vocab USING fts5vocab (playlists_fts, row);

CREATE TRIGGER artists_ai AFTER INSERT ON artists BEGIN
    INSERT INTO artists_fts (rowid, title) VALUES (new.pk, new.title);
END;
CREATE TRIGGER artists_ad AFTER DELETE ON artists BEGIN
    INSERT INTO artists_fts (artists_fts, rowid, title) VALUES ('delete', old.pk, old.title);
END;
CREATE TRIGGER artists_au AFTER UPDATE OF title ON artists
    WHEN old.title IS NOT new.title BEGIN
    INSERT INTO artists_fts (artists_fts, rowid, title) VALUES ('delete', old.pk, old.title);
    INSERT INTO artists_fts (rowid, title) VALUES (new.pk, new.title);
END;

CREATE TRIGGER albums_ai AFTER INSERT ON albums BEGIN
    INSERT INTO albums_fts (rowid, title, artist_title) VALUES (new.pk, new.title, new.artist_title);
END;
CREATE TRIGGER albums_ad AFTER DELETE ON albums BEGIN
    INSERT INTO albums_fts (albums_fts, rowid, title, artist_title) VALUES ('delete', old.pk, old.title, old.artist_title);
END;
CREATE TRIGGER albums_au AFTER UPDATE OF title, artist_title ON albums
    WHEN old.title IS NOT new.title OR old.artist_title IS NOT new.artist_title BEGIN
    INSERT INTO albums_fts (albums_fts, rowid, title, artist_title) VALUES ('delete', old.pk, old.title, old.artist_title);
    INSERT INTO albums_fts (rowid, title, artist_title) VALUES (new.pk, new.title, new.artist_title);
END;

CREATE TRIGGER tracks_ai AFTER INSERT ON tracks BEGIN
    INSERT INTO tracks_fts (rowid, title, artist_title, album_title, track_artist)
    VALUES (new.pk, new.title, new.artist_title, new.album_title, new.track_artist);
END;
CREATE TRIGGER tracks_ad AFTER DELETE ON tracks BEGIN
    INSERT INTO tracks_fts (tracks_fts, rowid, title, artist_title, album_title, track_artist)
    VALUES ('delete', old.pk, old.title, old.artist_title, old.album_title, old.track_artist);
END;
CREATE TRIGGER tracks_au AFTER UPDATE OF title, artist_title, album_title, track_artist ON tracks
    WHEN old.title IS NOT new.title OR old.artist_title IS NOT new.artist_title
      OR old.album_title IS NOT new.album_title OR old.track_artist IS NOT new.track_artist BEGIN
    INSERT INTO tracks_fts (tracks_fts, rowid, title, artist_title, album_title, track_artist)
    VALUES ('delete', old.pk, old.title, old.artist_title, old.album_title, old.track_artist);
    INSERT INTO tracks_fts (rowid, title, artist_title, album_title, track_artist)
    VALUES (new.pk, new.title, new.artist_title, new.album_title, new.track_artist);
END;

CREATE TRIGGER playlists_ai AFTER INSERT ON playlists BEGIN
    INSERT INTO playlists_fts (rowid, title) VALUES (new.pk, new.title);
END;
CREATE TRIGGER playlists_ad AFTER DELETE ON playlists BEGIN
    INSERT INTO playlists_fts (playlists_fts, rowid, title) VALUES ('delete', old.pk, old.title);
END;
CREATE TRIGGER playlists_au AFTER UPDATE OF title ON playlists
    WHEN old.title IS NOT new.title BEGIN
    INSERT INTO playlists_fts (playlists_fts, rowid, title) VALUES ('delete', old.pk, old.title);
    INSERT INTO playlists_fts (rowid, title) VALUES (new.pk, new.title);
END;
"#,
    // v2: global listener count per track (Plex `ratingCount`). Existing rows
    // have none yet, so the next sync runs as a full pass to fill it.
    r#"
ALTER TABLE tracks ADD COLUMN rating_count INTEGER;
UPDATE sync_state SET last_full_sync_at = NULL;
"#,
    // v3: artist biography (Plex `summary`, on list items; the next sync runs
    // as a full pass to fill it) and similar artists (Plex `Similar`, only in
    // artist detail, fetched when an artist page opens; see db::similar).
    r#"
ALTER TABLE artists ADD COLUMN summary TEXT;
ALTER TABLE artists ADD COLUMN similar TEXT NOT NULL DEFAULT '[]';
ALTER TABLE artists ADD COLUMN similar_fetched_at INTEGER;
UPDATE sync_state SET last_full_sync_at = NULL;
"#,
    // v4: recent searches are the items opened from search (artist, album,
    // track or playlist), not query text.
    r#"
DROP TABLE recent_searches;
CREATE TABLE search_recents (
    kind      TEXT NOT NULL,
    id        TEXT NOT NULL,
    opened_at INTEGER NOT NULL,
    PRIMARY KEY (kind, id)
);
"#,
    // v5: playlist editing. Playlists keep their description and Plex
    // updatedAt; items keep Plex's playlistItemID (needed to remove or move
    // one). Playlists made here start local only until their first song
    // reaches Plex. Edits queue in playlist_ops and replay in order; an id
    // made here maps to the Plex id once created (playlist_aliases).
    // duration_ms = -1 makes the next sync refetch every playlist's items so
    // the item ids fill in.
    r#"
ALTER TABLE playlists ADD COLUMN summary TEXT;
ALTER TABLE playlists ADD COLUMN updated_at INTEGER;
ALTER TABLE playlists ADD COLUMN local_only INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playlist_items ADD COLUMN item_id TEXT;
UPDATE playlist_items SET item_id = 'local:' || playlist_id || '.' || position;
UPDATE playlists SET duration_ms = -1;
CREATE TABLE playlist_ops (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id TEXT NOT NULL,
    op          TEXT NOT NULL,
    attempts    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);
CREATE INDEX playlist_ops_playlist ON playlist_ops (playlist_id);
CREATE TABLE playlist_aliases (
    local_id TEXT PRIMARY KEY,
    plex_id  TEXT NOT NULL
) WITHOUT ROWID;
"#,
    // v6: artist info from outside sources (artistinfo::cache). One row per
    // artist and source; data is the source's JSON, NULL when it had nothing.
    r#"
CREATE TABLE artist_info (
    artist_id  TEXT NOT NULL,
    source     TEXT NOT NULL,
    data       TEXT,
    fetched_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (artist_id, source)
) WITHOUT ROWID;
"#,
    // v7: likes. Tracks keep when they were rated (Plex lastRatedAt: the
    // Liked Songs "date added") and their own genre and mood tags (its
    // filter chips); the next sync runs as a full pass to fill both. Rating
    // writes queue in rating_ops, one per track, until Plex has them; each
    // keeps the rating it replaced so a refused write can be rolled back.
    r#"
ALTER TABLE tracks ADD COLUMN rated_at INTEGER;
ALTER TABLE tracks ADD COLUMN genres TEXT NOT NULL DEFAULT '[]';
CREATE INDEX tracks_rated ON tracks (rated_at DESC) WHERE user_rating IS NOT NULL;
CREATE TABLE rating_ops (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id      TEXT NOT NULL UNIQUE,
    rating        REAL,
    prev_rating   REAL,
    prev_rated_at INTEGER,
    attempts      INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
);
UPDATE sync_state SET last_full_sync_at = NULL;
"#,
    // v8: loudness and silence per track (db::trackaudio). Plex's gain,
    // albumGain, peak and albumPeak come from track detail on first play
    // (loudness_at marks the fetch, even when Plex had none); the quiet
    // lead-in and run-out are measured while a track decodes, for smart
    // crossfades. Sync never touches these columns.
    r#"
ALTER TABLE tracks ADD COLUMN gain_db REAL;
ALTER TABLE tracks ADD COLUMN album_gain_db REAL;
ALTER TABLE tracks ADD COLUMN peak REAL;
ALTER TABLE tracks ADD COLUMN album_peak REAL;
ALTER TABLE tracks ADD COLUMN loudness_at INTEGER;
ALTER TABLE tracks ADD COLUMN head_silence_ms INTEGER;
ALTER TABLE tracks ADD COLUMN tail_silence_ms INTEGER;
"#,
    // v9: public share links found for songs and albums (share::cache). One
    // row per lookup, keyed by the folded search fields; url NULL = no match.
    r#"
CREATE TABLE share_links (
    key        TEXT PRIMARY KEY,
    url        TEXT,
    fetched_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
"#,
    // v10: public cover URLs for Discord status, per album
    // (platform::discord::art). source is "caa" (url NULL: Cover Art Archive
    // has none) or "imgur" (kept for good).
    r#"
CREATE TABLE IF NOT EXISTS discord_art (
    album_id   TEXT PRIMARY KEY,
    url        TEXT,
    source     TEXT NOT NULL,
    fetched_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
"#,
    // v11: artist names that contain a separator ("Tyler, The Maker"),
    // learned from ListenBrainz artist credits (db::popular), so splitting a
    // track's artist line keeps them whole. key is the lower-cased name.
    r#"
CREATE TABLE IF NOT EXISTS credit_names (
    key  TEXT PRIMARY KEY,
    name TEXT NOT NULL
) WITHOUT ROWID;
"#,
    // v12: LRCLIB lyrics (lyrics::cache). One row per lookup, keyed by the
    // folded artist, title, album and duration; body NULL = none found.
    r#"
CREATE TABLE IF NOT EXISTS lyrics_cache (
    key        TEXT PRIMARY KEY,
    body       TEXT,
    matched    INTEGER NOT NULL DEFAULT 0,
    fetched_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
"#,
    // v13: tags read from the files themselves (tags::). One row per media
    // part key; the key carries the part's update stamp, so a changed file
    // gets a new key and is read again. status: ok, failed (read error, tried
    // again when the file changes) or skipped (no Range support and too big).
    // album_tags is derived from file_tags after each scan batch.
    r#"
CREATE TABLE IF NOT EXISTS file_tags (
    part_key   TEXT PRIMARY KEY,
    status     TEXT NOT NULL,
    explicit   INTEGER,
    version    TEXT,
    era        TEXT,
    quality    TEXT,
    date       TEXT,
    edition    TEXT,
    grp        TEXT,
    source     TEXT,
    flags      TEXT,
    scanned_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS tracks_part_key ON tracks (part_key);
CREATE TABLE IF NOT EXISTS album_tags (
    album_id TEXT PRIMARY KEY,
    explicit INTEGER NOT NULL DEFAULT 0,
    version  TEXT,
    edition  TEXT,
    grp      TEXT,
    flagged  INTEGER NOT NULL DEFAULT 0,
    face_id  TEXT
);
CREATE INDEX IF NOT EXISTS album_tags_grp ON album_tags (grp);
"#,
    // v14: lyrics the user wrote in the app (lyrics::user), as LRC text (or
    // plain lines), one row per track. prefer: show them ahead of Plex's.
    r#"
CREATE TABLE IF NOT EXISTS user_lyrics (
    track_id   TEXT PRIMARY KEY,
    body       TEXT NOT NULL,
    prefer     INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);
"#,
    // v15: lyrics transcribed on this computer (lyrics::transcribe) go into
    // user_lyrics with auto set to the model's name, so the editor opens them;
    // saving in the editor clears it. lyrics_auto_empty remembers songs that
    // were transcribed and had no words, so the batch doesn't redo them.
    r#"
ALTER TABLE user_lyrics ADD COLUMN auto TEXT;
CREATE TABLE IF NOT EXISTS lyrics_auto_empty (
    track_id   TEXT PRIMARY KEY,
    model      TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
"#,
    // v16: album release date (Plex originallyAvailableAt) and label (studio)
    // for the album page footer. Both are on list items; the next sync runs as
    // a full pass to fill them.
    r#"
ALTER TABLE albums ADD COLUMN released TEXT;
ALTER TABLE albums ADD COLUMN studio TEXT;
UPDATE sync_state SET last_full_sync_at = NULL;
"#,
    // v17: downloads (downloads::store). download_items are what the user
    // turned on (album, playlist, liked, mix), with their songs in
    // download_item_tracks; download_files has one row per song file, shared
    // by every item that holds the song. state: queued, done, failed or
    // missing (gone from disk). seq is the queue order.
    r#"
CREATE TABLE IF NOT EXISTS download_items (
    kind     TEXT NOT NULL,
    id       TEXT NOT NULL,
    title    TEXT NOT NULL DEFAULT '',
    added_at INTEGER NOT NULL,
    PRIMARY KEY (kind, id)
);
CREATE TABLE IF NOT EXISTS download_item_tracks (
    kind     TEXT NOT NULL,
    id       TEXT NOT NULL,
    track_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (kind, id, track_id)
);
CREATE INDEX IF NOT EXISTS download_item_tracks_track ON download_item_tracks (track_id);
CREATE TABLE IF NOT EXISTS download_files (
    track_id    TEXT PRIMARY KEY,
    seq         INTEGER NOT NULL,
    state       TEXT NOT NULL,
    part_key    TEXT NOT NULL DEFAULT '',
    quality     TEXT NOT NULL DEFAULT 'original',
    file        TEXT,
    size        INTEGER NOT NULL DEFAULT 0,
    error       TEXT,
    attempts    INTEGER NOT NULL DEFAULT 0,
    next_try_at INTEGER NOT NULL DEFAULT 0,
    done_at     INTEGER
);
CREATE INDEX IF NOT EXISTS download_files_queue ON download_files (state, seq);
"#,
    // v18: made-for-you mixes (mixes::). Albums keep their Plex style and
    // mood tags (collected per tag value, like format); the next sync runs as
    // a full pass to fill them. Generated mixes are cached with their songs
    // in order; `day` is the local day they were made for (after 4 am).
    r#"
ALTER TABLE albums ADD COLUMN styles TEXT NOT NULL DEFAULT '[]';
ALTER TABLE albums ADD COLUMN moods TEXT NOT NULL DEFAULT '[]';
CREATE TABLE IF NOT EXISTS mixes (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,
    position     INTEGER NOT NULL,
    title        TEXT NOT NULL,
    description  TEXT NOT NULL,
    color        TEXT NOT NULL,
    artist_ids   TEXT NOT NULL DEFAULT '[]',
    tag          TEXT,
    day          TEXT NOT NULL,
    generated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS mix_tracks (
    mix_id   TEXT NOT NULL REFERENCES mixes (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    track_id TEXT NOT NULL,
    familiar INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (mix_id, position)
) WITHOUT ROWID;
UPDATE sync_state SET last_full_sync_at = NULL;
"#,
    // v19: app-only state that can travel in the sync file (sync::). One row
    // per entry: a namespace (pins, folders, ...), a key, the value as JSON
    // (NULL for a deletion, kept as a tombstone), when it changed (ms) and on
    // which device. Last writer wins per entry.
    r#"
CREATE TABLE IF NOT EXISTS sync_entries (
    ns     TEXT NOT NULL,
    key    TEXT NOT NULL,
    value  TEXT,
    ts     INTEGER NOT NULL,
    device TEXT NOT NULL,
    PRIMARY KEY (ns, key)
) WITHOUT ROWID;
"#,
    // v20: whether Plex matched each album (guid plex://... = 1, local://... =
    // 0, NULL until the next sync fills it). Feeds the unreleased guess.
    r#"
ALTER TABLE albums ADD COLUMN matched INTEGER;
UPDATE sync_state SET last_full_sync_at = NULL;
"#,
    // v21: server-qualified ids (crate::sid). Every Plex id and server image
    // path in the cache gets its server's tag. That needs the server's
    // machine id, so this only marks the rewrite as owed; db::qualify does
    // it in place once the server is known and keeps that tag as the one
    // older plain ids map to.
    r#"
INSERT OR REPLACE INTO kv (key, value) VALUES ('ids.pending', '1');
"#,
    // v22: more than one library. `src` names the library a row was read
    // from ('' for the connected server's main library, which every row
    // before this came from; `<server tag>/<section id>` for the others), so
    // each library's sync only adds and removes its own rows. `mbid` holds
    // the MusicBrainz id Plex matched, and `dupes` the albums, songs and
    // artists that are the same as one on another server (db::merge).
    r#"
ALTER TABLE artists ADD COLUMN src TEXT NOT NULL DEFAULT '';
ALTER TABLE albums ADD COLUMN src TEXT NOT NULL DEFAULT '';
ALTER TABLE tracks ADD COLUMN src TEXT NOT NULL DEFAULT '';
ALTER TABLE albums ADD COLUMN mbid TEXT;
ALTER TABLE tracks ADD COLUMN mbid TEXT;
CREATE INDEX IF NOT EXISTS artists_src ON artists (src);
CREATE INDEX IF NOT EXISTS albums_src ON albums (src);
CREATE INDEX IF NOT EXISTS tracks_src ON tracks (src);
CREATE TABLE IF NOT EXISTS dupes (
    id    TEXT PRIMARY KEY,
    kind  TEXT NOT NULL,
    canon TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS dupes_canon ON dupes (canon);
"#,
    // v23: what MusicBrainz said about albums Plex couldn't match
    // (artistinfo::releasecheck). found = 1 with the release group and
    // release ids, 0 when nothing matched. Checked again after expires_at.
    r#"
CREATE TABLE IF NOT EXISTS album_mb_check (
    album_id         TEXT PRIMARY KEY,
    found            INTEGER NOT NULL,
    release_group_id TEXT,
    release_id       TEXT,
    checked_at       INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL
) WITHOUT ROWID;
"#,
    // v24: Discord covers go to more than one upload host, some of which
    // delete files after hours or days (platform::discord::hosts). `host`
    // names the upload host and `link_expires_at` is when it deletes the
    // file (NULL: kept). Earlier Imgur and catbox.moe links are kept.
    r#"
ALTER TABLE discord_art ADD COLUMN host TEXT;
ALTER TABLE discord_art ADD COLUMN link_expires_at INTEGER;
UPDATE discord_art SET host = source WHERE source IN ('imgur', 'catbox');
"#,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_fresh_db_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let mut c = open_writer(&path).unwrap();
        migrate(&mut c).unwrap();
        migrate(&mut c).unwrap();
        let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        let mode: String = c.pragma_query_value(None, "journal_mode", |r| r.get(0)).unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        // FTS5 is compiled into the bundled SQLite.
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('1', 'Beyoncé', 'Beyonce')", [])
            .unwrap();
        let n: i64 = c
            .query_row("SELECT count(*) FROM artists_fts WHERE artists_fts MATCH 'beyon*'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let r = open_reader(&path).unwrap();
        assert!(r.execute("DELETE FROM artists", []).is_err(), "reader is read-only");
    }

    #[test]
    fn v3_adds_artist_summary_and_similar_and_makes_the_next_sync_full() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        c.execute_batch(MIGRATIONS[0]).unwrap();
        c.execute_batch(MIGRATIONS[1]).unwrap();
        c.pragma_update(None, "user_version", 2).unwrap();
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('1', 'A', 'A')", []).unwrap();
        c.execute(
            "INSERT INTO sync_state (id, section_id, section_uuid, last_sync_at, last_full_sync_at)
             VALUES (1, '1', 'u', 100, 100)",
            [],
        )
        .unwrap();
        migrate(&mut c).unwrap();
        let (summary, similar, at): (Option<String>, String, Option<i64>) = c
            .query_row("SELECT summary, similar, similar_fetched_at FROM artists", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((summary, similar.as_str(), at), (None, "[]", None));
        let full: Option<i64> = c.query_row("SELECT last_full_sync_at FROM sync_state", [], |r| r.get(0)).unwrap();
        assert_eq!(full, None);
    }

    #[test]
    fn v5_names_every_playlist_row_and_refetches_items() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        for m in &MIGRATIONS[..4] {
            c.execute_batch(m).unwrap();
        }
        c.pragma_update(None, "user_version", 4).unwrap();
        c.execute("INSERT INTO playlists (id, title, track_count, duration_ms) VALUES ('p', 'P', 2, 500)", []).unwrap();
        c.execute("INSERT INTO playlist_items (playlist_id, position, track_id) VALUES ('p', 0, 't'), ('p', 1, 't')", [])
            .unwrap();
        migrate(&mut c).unwrap();
        let ids: Vec<String> = c
            .prepare("SELECT item_id FROM playlist_items ORDER BY position")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1], "the same song twice still has two names");
        assert!(ids.iter().all(|i| i.starts_with("local:")));
        let d: i64 = c.query_row("SELECT duration_ms FROM playlists", [], |r| r.get(0)).unwrap();
        assert_eq!(d, -1, "differs from Plex, so the next sync refetches the items");
    }

    #[test]
    fn v8_adds_loudness_and_silence_columns_to_tracks() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        for m in &MIGRATIONS[..7] {
            c.execute_batch(m).unwrap();
        }
        c.pragma_update(None, "user_version", 7).unwrap();
        migrate(&mut c).unwrap();
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM pragma_table_info('tracks') WHERE name IN
                 ('gain_db', 'album_gain_db', 'peak', 'album_peak', 'loudness_at', 'head_silence_ms', 'tail_silence_ms')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 7);
    }

    #[test]
    fn v6_adds_the_artist_info_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        for m in &MIGRATIONS[..5] {
            c.execute_batch(m).unwrap();
        }
        c.pragma_update(None, "user_version", 5).unwrap();
        migrate(&mut c).unwrap();
        c.execute(
            "INSERT INTO artist_info (artist_id, source, data, fetched_at, expires_at) VALUES ('1', 'wikipedia', NULL, 1, 2)",
            [],
        )
        .unwrap();
        let n: i64 = c.query_row("SELECT count(*) FROM artist_info", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn v7_likes_migration_adds_rating_columns_and_queue_and_makes_the_next_sync_full() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        let before = 6; // v7 is the likes migration
        for m in &MIGRATIONS[..before] {
            c.execute_batch(m).unwrap();
        }
        c.pragma_update(None, "user_version", before as i64).unwrap();
        c.execute(
            "INSERT INTO sync_state (id, section_id, section_uuid, last_sync_at, last_full_sync_at)
             VALUES (1, '1', 'u', 100, 100)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, user_rating)
             VALUES ('t', 'T', 'a', 'A', 'r', 'R', '/p', 10)",
            [],
        )
        .unwrap();
        migrate(&mut c).unwrap();
        let (at, genres): (Option<i64>, String) =
            c.query_row("SELECT rated_at, genres FROM tracks", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((at, genres.as_str()), (None, "[]"));
        let full: Option<i64> = c.query_row("SELECT last_full_sync_at FROM sync_state", [], |r| r.get(0)).unwrap();
        assert_eq!(full, None);
        c.execute("INSERT INTO rating_ops (track_id, rating, created_at) VALUES ('t', NULL, 1)", []).unwrap();
        assert!(
            c.execute("INSERT INTO rating_ops (track_id, rating, created_at) VALUES ('t', 10, 2)", []).is_err(),
            "one queued rating per track"
        );
    }

    #[test]
    fn share_links_migration_adds_the_link_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        migrate(&mut c).unwrap();
        c.execute("INSERT INTO share_links (key, url, fetched_at, expires_at) VALUES ('k', NULL, 1, 2)", []).unwrap();
        assert!(
            c.execute("INSERT INTO share_links (key, url, fetched_at, expires_at) VALUES ('k', 'u', 1, 2)", []).is_err(),
            "one row per lookup"
        );
    }

    #[test]
    fn v10_adds_the_discord_art_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        migrate(&mut c).unwrap();
        const { assert!(SCHEMA_VERSION >= 10) };
        c.execute(
            "INSERT INTO discord_art (album_id, url, source, fetched_at, expires_at) VALUES ('1', NULL, 'caa', 1, 2)",
            [],
        )
        .unwrap();
        assert!(
            c.execute("INSERT INTO discord_art (album_id, url, source, fetched_at, expires_at) VALUES ('1', 'u', 'imgur', 1, 2)", [])
                .is_err(),
            "one row per album"
        );
    }

    #[test]
    fn discord_art_gets_upload_hosts_and_keeps_earlier_links() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        let before = MIGRATIONS.iter().position(|m| m.contains("link_expires_at")).unwrap();
        migrate_to(&mut c, before).unwrap();
        c.execute_batch(
            "INSERT INTO discord_art (album_id, url, source, fetched_at, expires_at) VALUES
                 ('1', 'https://files.catbox.moe/a.jpg', 'catbox', 1, 9223372036854775807),
                 ('2', NULL, 'caa', 1, 2)",
        )
        .unwrap();
        migrate(&mut c).unwrap();
        let row = |id: &str| {
            c.query_row(
                "SELECT url, host, link_expires_at, expires_at FROM discord_art WHERE album_id = ?1",
                [id],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, i64>(3)?)),
            )
            .unwrap()
        };
        assert_eq!(row("1"), (Some("https://files.catbox.moe/a.jpg".into()), Some("catbox".into()), None, i64::MAX));
        assert_eq!(row("2"), (None, None, None, 2));
    }

    #[test]
    fn v11_adds_learned_credit_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        for m in &MIGRATIONS[..10] {
            c.execute_batch(m).unwrap();
        }
        c.pragma_update(None, "user_version", 10).unwrap();
        migrate(&mut c).unwrap();
        c.execute("INSERT INTO credit_names (key, name) VALUES ('a, b', 'A, B')", []).unwrap();
        assert!(c.execute("INSERT INTO credit_names (key, name) VALUES ('a, b', 'a, B')", []).is_err(), "one row per name");
    }

    #[test]
    fn v12_adds_the_lyrics_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        migrate(&mut c).unwrap();
        const { assert!(SCHEMA_VERSION >= 12) };
        c.execute("INSERT INTO lyrics_cache (key, body, fetched_at, expires_at) VALUES ('k', NULL, 1, 2)", []).unwrap();
        assert!(
            c.execute("INSERT INTO lyrics_cache (key, body, fetched_at, expires_at) VALUES ('k', 'x', 1, 2)", []).is_err(),
            "one row per lookup"
        );
    }

    #[test]
    fn v13_adds_the_file_tag_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        migrate(&mut c).unwrap();
        const { assert!(SCHEMA_VERSION >= 13) };
        let row = "INSERT INTO file_tags (part_key, status, scanned_at) VALUES ('/library/parts/1/2/file.flac', 'ok', 1)";
        c.execute(row, []).unwrap();
        assert!(c.execute(row, []).is_err(), "one row per part key");
        c.execute("INSERT INTO album_tags (album_id, grp, face_id) VALUES ('1', 'g', '1')", []).unwrap();
    }

    #[test]
    fn v14_adds_user_lyrics() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        migrate(&mut c).unwrap();
        const { assert!(SCHEMA_VERSION >= 14) };
        let row = "INSERT INTO user_lyrics (track_id, body, updated_at) VALUES ('1', 'Hello', 1)";
        c.execute(row, []).unwrap();
        assert!(c.execute(row, []).is_err(), "one row per track");
    }

    #[test]
    fn v18_adds_album_styles_and_moods_and_the_mix_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        for m in &MIGRATIONS[..17] {
            c.execute_batch(m).unwrap();
        }
        c.pragma_update(None, "user_version", 17).unwrap();
        c.execute(
            "INSERT INTO sync_state (id, section_id, section_uuid, last_sync_at, last_full_sync_at)
             VALUES (1, '1', 'u', 100, 100)",
            [],
        )
        .unwrap();
        c.execute("INSERT INTO albums (id, title, sort_title, artist_id, artist_title) VALUES ('a', 'A', 'A', 'r', 'R')", [])
            .unwrap();
        migrate(&mut c).unwrap();
        let (styles, moods): (String, String) =
            c.query_row("SELECT styles, moods FROM albums", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((styles.as_str(), moods.as_str()), ("[]", "[]"));
        let full: Option<i64> = c.query_row("SELECT last_full_sync_at FROM sync_state", [], |r| r.get(0)).unwrap();
        assert_eq!(full, None);
        c.execute(
            "INSERT INTO mixes (id, kind, position, title, description, color, day, generated_at)
             VALUES ('daily:1', 'daily', 0, 'Daily Mix 1', 'd', '#fff', '2026-10-14', 1)",
            [],
        )
        .unwrap();
        c.execute("INSERT INTO mix_tracks (mix_id, position, track_id) VALUES ('daily:1', 0, 't')", []).unwrap();
        c.execute("DELETE FROM mixes", []).unwrap();
        let n: i64 = c.query_row("SELECT count(*) FROM mix_tracks", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "a mix's songs go with it");
    }

    #[test]
    fn v2_adds_rating_count_and_makes_the_next_sync_full() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("t.db")).unwrap();
        c.execute_batch(MIGRATIONS[0]).unwrap();
        c.pragma_update(None, "user_version", 1).unwrap();
        c.execute(
            "INSERT INTO sync_state (id, section_id, section_uuid, last_sync_at, last_full_sync_at)
             VALUES (1, '1', 'u', 100, 100)",
            [],
        )
        .unwrap();
        migrate(&mut c).unwrap();
        let full: Option<i64> = c.query_row("SELECT last_full_sync_at FROM sync_state", [], |r| r.get(0)).unwrap();
        assert_eq!(full, None);
        let n: i64 = c
            .query_row("SELECT count(*) FROM pragma_table_info('tracks') WHERE name = 'rating_count'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
