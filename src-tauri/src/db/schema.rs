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
}
