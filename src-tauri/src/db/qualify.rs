//! Rewrite a cache from before server-qualified ids (`crate::sid`) in place.
//!
//! Migration v21 marks the cache (`kv` `ids.pending`). The rewrite itself
//! needs the tag of the server the library came from, which only the Plex
//! side knows, so it runs here once that's known: at startup from the saved
//! server choice, or when the first server is picked. One transaction puts
//! the tag on every plain rating key and server image path, in every table
//! that keeps them, and records the tag as `ids.legacyTag` so ids saved
//! elsewhere before the change (links, the saved queue, the sync file) still
//! resolve (`sid::resolve`).
//!
//! Untouched on purpose: part keys (paths used with their track), section
//! ids, Plex tag ids in `artists.similar`, pending playlist edits (their
//! plain ids still go to the right server) and the sync store (read through
//! `sid::resolve`).

use rusqlite::{Connection, params};

use super::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};

pub const KEY_PENDING: &str = "ids.pending";
pub const KEY_LEGACY: &str = "ids.legacyTag";

/// A plain rating key column (all digits) gets the tag.
fn key_expr(col: &str) -> String {
    format!("CASE WHEN {col} <> '' AND {col} NOT GLOB '*[^0-9]*' THEN ?1 || ':' || {col} ELSE {col} END")
}

/// A server image path column gets the tag; outside links don't.
fn path_expr(col: &str) -> String {
    format!(
        "CASE WHEN {col} LIKE '/%' AND {col} NOT LIKE '/ext/%' THEN ?1 || ':' || {col} ELSE {col} END"
    )
}

/// (table, key columns, path columns).
const COLUMNS: &[(&str, &[&str], &[&str])] = &[
    ("artists", &["id"], &["thumb", "art"]),
    ("albums", &["id", "artist_id"], &["thumb"]),
    ("tracks", &["id", "album_id", "artist_id"], &["thumb"]),
    ("playlists", &["id"], &["thumb"]),
    ("playlist_items", &["playlist_id", "track_id"], &[]),
    ("play_history", &["rating_key"], &[]),
    ("listen_log", &["rating_key"], &[]),
    ("report_queue", &["rating_key"], &[]),
    ("search_recents", &["id"], &[]),
    ("playlist_ops", &["playlist_id"], &[]),
    ("playlist_aliases", &["plex_id"], &[]),
    ("artist_info", &["artist_id"], &[]),
    ("rating_ops", &["track_id"], &[]),
    ("discord_art", &["album_id"], &[]),
    ("album_tags", &["album_id", "face_id"], &[]),
    ("user_lyrics", &["track_id"], &[]),
    ("lyrics_auto_empty", &["track_id"], &[]),
    ("lyrics_share", &["track_id"], &[]),
    ("download_items", &["id"], &[]),
    ("download_item_tracks", &["id", "track_id"], &[]),
    ("download_files", &["track_id"], &[]),
    ("mix_tracks", &["track_id"], &[]),
];

/// The tag the cache was rewritten with, if it was.
pub fn legacy_tag(c: &Connection) -> AppResult<Option<String>> {
    kv_get(c, KEY_LEGACY)
}

/// Whether the rewrite is still owed.
pub fn pending(c: &Connection) -> AppResult<bool> {
    Ok(kv_get(c, KEY_PENDING)?.is_some())
}

/// Put `tag` on every plain id in the cache. Does nothing when it already
/// ran. Returns the number of rows changed.
pub fn qualify_legacy(c: &mut Connection, tag: &str) -> AppResult<usize> {
    if !crate::sid::is_tag(tag) {
        return Err(AppError::Invalid(format!("not a server tag: {tag:?}")));
    }
    if !pending(c)? {
        return Ok(0);
    }
    let tx = c.transaction()?;
    // Playlist rows and mix songs point at their parents; those move together.
    tx.pragma_update(None, "defer_foreign_keys", true)?;
    let mut changed = 0;
    for (table, keys, paths) in COLUMNS {
        let mut sets: Vec<String> = keys.iter().map(|k| format!("{k} = {}", key_expr(k))).collect();
        sets.extend(paths.iter().map(|p| format!("{p} = {}", path_expr(p))));
        let mut wheres: Vec<String> = keys
            .iter()
            .map(|k| format!("({k} <> '' AND {k} NOT GLOB '*[^0-9]*')"))
            .collect();
        wheres.extend(paths.iter().map(|p| format!("({p} LIKE '/%' AND {p} NOT LIKE '/ext/%')")));
        let sql = format!("UPDATE {table} SET {} WHERE {}", sets.join(", "), wheres.join(" OR "));
        changed += tx.execute(&sql, params![tag])?;
    }
    // Play contexts: albums, artists and playlists by id; radios by seed.
    changed += tx.execute(
        &format!(
            "UPDATE listen_log SET context_id = {} WHERE context_kind IN ('album', 'artist', 'playlist')
             AND context_id <> '' AND context_id NOT GLOB '*[^0-9]*'",
            key_expr("context_id")
        ),
        params![tag],
    )?;
    changed += tx.execute(
        "UPDATE listen_log SET context_id =
             substr(context_id, 1, instr(context_id, ':')) || ?1 || ':' || substr(context_id, instr(context_id, ':') + 1)
         WHERE context_kind = 'radio' AND (context_id GLOB 'track:[0-9]*' OR context_id GLOB 'artist:[0-9]*')
           AND substr(context_id, instr(context_id, ':') + 1) NOT GLOB '*[^0-9]*'",
        params![tag],
    )?;
    // Mixes keep their artists as a JSON list.
    changed += tx.execute(
        "UPDATE mixes SET artist_ids = (
             SELECT json_group_array(CASE WHEN value <> '' AND value NOT GLOB '*[^0-9]*' THEN ?1 || ':' || value ELSE value END)
             FROM json_each(mixes.artist_ids))
         WHERE json_valid(artist_ids) AND artist_ids <> '[]'",
        params![tag],
    )?;
    kv_set(&tx, KEY_LEGACY, tag)?;
    tx.execute("DELETE FROM kv WHERE key = ?1", [KEY_PENDING])?;
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
        schema::migrate(&mut c).unwrap();
        (dir, c)
    }

    fn one(c: &Connection, sql: &str) -> String {
        c.query_row(sql, [], |r| r.get::<_, String>(0)).unwrap()
    }

    #[test]
    fn rewrites_every_plain_id_and_server_path_once() {
        let (_d, mut c) = db();
        c.execute_batch(
            r#"
            INSERT INTO artists (id, title, sort_title, thumb, art) VALUES ('7', 'A', 'A', '/library/metadata/7/thumb/1', '/ext/https://x/y.jpg');
            INSERT INTO albums (id, title, sort_title, artist_id, artist_title, thumb) VALUES ('8', 'B', 'B', '7', 'A', '/library/metadata/8/thumb/1');
            INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, thumb)
                VALUES ('9', 'C', '8', 'B', '7', 'A', '/library/parts/1/2/file.flac', '/library/metadata/8/thumb/1');
            INSERT INTO playlists (id, title) VALUES ('50', 'P'), ('local:abc', 'Q');
            INSERT INTO playlist_items (playlist_id, position, track_id, item_id) VALUES ('50', 0, '9', '77'), ('local:abc', 0, '9', 'local:x');
            INSERT INTO play_history (rating_key, viewed_at) VALUES ('9', 1);
            INSERT INTO listen_log (rating_key, started_at, context_kind, context_id) VALUES
                ('9', 1, 'album', '8'), ('9', 2, 'radio', 'track:9'), ('9', 3, 'mix', 'daily:1'), ('9', 4, 'liked', 'liked');
            INSERT INTO search_recents (kind, id, opened_at) VALUES ('album', '8', 1);
            INSERT INTO download_items (kind, id, title, added_at) VALUES ('album', '8', 'B', 1), ('liked', '', 'Liked', 1), ('mix', 'daily:1', 'M', 1);
            INSERT INTO download_item_tracks (kind, id, track_id, position) VALUES ('album', '8', '9', 0);
            INSERT INTO download_files (track_id, seq, state) VALUES ('9', 1, 'done');
            INSERT INTO user_lyrics (track_id, body, updated_at) VALUES ('9', 'la', 1);
            INSERT INTO album_tags (album_id, face_id) VALUES ('8', '8');
            INSERT INTO mixes (id, kind, position, title, description, color, artist_ids, day, generated_at)
                VALUES ('daily:1', 'daily', 0, 'D', 'd', '#fff', '["7","x"]', '2026-10-14', 1);
            INSERT INTO mix_tracks (mix_id, position, track_id) VALUES ('daily:1', 0, '9');
            INSERT INTO file_tags (part_key, status, scanned_at) VALUES ('/library/parts/1/2/file.flac', 'ok', 1);
            "#,
        )
        .unwrap();
        assert!(pending(&c).unwrap());
        let n = qualify_legacy(&mut c, "0a1b2c3d").unwrap();
        assert!(n > 15, "{n}");
        assert_eq!(one(&c, "SELECT id || '|' || thumb || '|' || art FROM artists"), "0a1b2c3d:7|0a1b2c3d:/library/metadata/7/thumb/1|/ext/https://x/y.jpg");
        assert_eq!(one(&c, "SELECT id || '|' || artist_id FROM albums"), "0a1b2c3d:8|0a1b2c3d:7");
        assert_eq!(
            one(&c, "SELECT id || '|' || album_id || '|' || artist_id || '|' || part_key FROM tracks"),
            "0a1b2c3d:9|0a1b2c3d:8|0a1b2c3d:7|/library/parts/1/2/file.flac",
            "part keys stay as Plex sends them"
        );
        assert_eq!(one(&c, "SELECT group_concat(id, ',') FROM (SELECT id FROM playlists ORDER BY id)"), "0a1b2c3d:50,local:abc");
        assert_eq!(
            one(&c, "SELECT group_concat(playlist_id || '/' || track_id || '/' || item_id, ',') FROM (SELECT * FROM playlist_items ORDER BY playlist_id)"),
            "0a1b2c3d:50/0a1b2c3d:9/77,local:abc/0a1b2c3d:9/local:x"
        );
        assert_eq!(
            one(&c, "SELECT group_concat(context_id, ',') FROM (SELECT context_id FROM listen_log ORDER BY started_at)"),
            "0a1b2c3d:8,track:0a1b2c3d:9,daily:1,liked"
        );
        assert_eq!(one(&c, "SELECT rating_key FROM play_history"), "0a1b2c3d:9");
        assert_eq!(one(&c, "SELECT id FROM search_recents"), "0a1b2c3d:8");
        assert_eq!(
            one(&c, "SELECT group_concat(kind || '=' || id, ',') FROM (SELECT * FROM download_items ORDER BY kind)"),
            "album=0a1b2c3d:8,liked=,mix=daily:1"
        );
        assert_eq!(one(&c, "SELECT id || '/' || track_id FROM download_item_tracks"), "0a1b2c3d:8/0a1b2c3d:9");
        assert_eq!(one(&c, "SELECT track_id FROM download_files"), "0a1b2c3d:9");
        assert_eq!(one(&c, "SELECT track_id FROM user_lyrics"), "0a1b2c3d:9");
        assert_eq!(one(&c, "SELECT album_id || '/' || face_id FROM album_tags"), "0a1b2c3d:8/0a1b2c3d:8");
        assert_eq!(one(&c, "SELECT artist_ids FROM mixes"), r#"["0a1b2c3d:7","x"]"#);
        assert_eq!(one(&c, "SELECT track_id FROM mix_tracks"), "0a1b2c3d:9");
        assert_eq!(one(&c, "SELECT part_key FROM file_tags"), "/library/parts/1/2/file.flac");
        // The search index follows the rows (same pk), so search still finds them.
        let n: i64 = c.query_row("SELECT count(*) FROM tracks_fts WHERE tracks_fts MATCH 'c'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        // Recorded, and never again.
        assert_eq!(legacy_tag(&c).unwrap().as_deref(), Some("0a1b2c3d"));
        assert!(!pending(&c).unwrap());
        assert_eq!(qualify_legacy(&mut c, "ffffffff").unwrap(), 0);
        assert_eq!(one(&c, "SELECT id FROM albums"), "0a1b2c3d:8");
        // Foreign keys hold.
        let bad: i64 = c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
        assert_eq!(bad, 0);
    }

    /// A cache as the last build before server tags left it (schema v19),
    /// shaped like a real one: library rows, a playlist, plays, a download,
    /// own lyrics, a mix, saved settings and a sync store entry.
    fn old_cache(path: &std::path::Path) {
        let mut c = schema::open_writer(path).unwrap();
        schema::migrate_to(&mut c, 19).unwrap();
        c.execute_batch(
            r#"
            INSERT INTO artists (id, title, sort_title, thumb) VALUES ('7', 'A', 'A', '/library/metadata/7/thumb/1');
            INSERT INTO albums (id, title, sort_title, artist_id, artist_title, thumb, track_count) VALUES ('8', 'B', 'B', '7', 'A', '/library/metadata/8/thumb/1', 2);
            INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, track_number, user_rating)
                VALUES ('9', 'C', '8', 'B', '7', 'A', '/library/parts/1/2/c.flac', 1, 10.0),
                       ('10', 'D', '8', 'B', '7', 'A', '/library/parts/3/4/d.flac', 2, NULL);
            INSERT INTO playlists (id, title) VALUES ('50', 'P');
            INSERT INTO playlist_items (playlist_id, position, track_id, item_id) VALUES ('50', 0, '9', '77');
            INSERT INTO sync_state (id, section_id, section_uuid, last_sync_at, last_full_sync_at, generation) VALUES (1, '1', 'u', 100, 100, 3);
            INSERT INTO play_history (rating_key, viewed_at) VALUES ('9', 1);
            INSERT INTO listen_log (rating_key, started_at, context_kind, context_id) VALUES ('9', 1, 'album', '8');
            INSERT INTO download_items (kind, id, title, added_at) VALUES ('album', '8', 'B', 1);
            INSERT INTO download_item_tracks (kind, id, track_id, position) VALUES ('album', '8', '9', 0);
            INSERT INTO download_files (track_id, seq, state) VALUES ('9', 1, 'done');
            INSERT INTO user_lyrics (track_id, body, updated_at) VALUES ('10', 'la', 1);
            INSERT INTO mixes (id, kind, position, title, description, color, artist_ids, day, generated_at)
                VALUES ('daily:1', 'daily', 0, 'D', 'd', '#fff', '["7"]', '2026-10-14', 1);
            INSERT INTO mix_tracks (mix_id, position, track_id) VALUES ('daily:1', 0, '10');
            INSERT INTO kv (key, value) VALUES ('app.settings', '{"mixExcludeArtists":["7"],"mixExcludeAlbums":["8"]}');
            INSERT INTO sync_entries (ns, key, value, ts, device) VALUES ('pins', 'album:8', '{"id":"8"}', 1, 'd1');
            "#,
        )
        .unwrap();
    }

    #[test]
    fn an_older_builds_cache_upgrades_in_place_and_old_ids_keep_working() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");
        old_cache(&path);
        crate::sid::set_legacy(None);

        // Opening it runs the new migrations; the rewrite waits for the tag.
        let db = crate::db::DbHandle::open(&path).unwrap();
        assert!(db.ids_pending());
        assert_eq!(crate::sid::legacy(), None);
        assert_eq!(db.read(|c| Ok(one(c, "SELECT id || '|' || src FROM albums"))).unwrap(), "8|", "every old row is the main library's");

        let t = crate::sid::tag("machine-a");
        assert!(db.qualify_legacy(&t).unwrap() > 10);
        assert!(!db.ids_pending());
        assert_eq!(crate::sid::legacy().as_deref(), Some(t.as_str()));
        let q = |sql: &str| db.read(|c| Ok(one(c, sql))).unwrap();
        assert_eq!(q("SELECT group_concat(id, ',') FROM (SELECT id FROM tracks ORDER BY id)"), format!("{t}:10,{t}:9"));
        assert_eq!(q("SELECT album_id FROM tracks LIMIT 1"), format!("{t}:8"));
        assert_eq!(q("SELECT part_key FROM tracks WHERE id LIKE '%:9'"), "/library/parts/1/2/c.flac");
        assert_eq!(q("SELECT track_id FROM playlist_items"), format!("{t}:9"));
        assert_eq!(q("SELECT context_id FROM listen_log"), format!("{t}:8"));
        assert_eq!(q("SELECT track_id FROM download_files"), format!("{t}:9"));
        assert_eq!(q("SELECT track_id FROM user_lyrics"), format!("{t}:10"));
        assert_eq!(q("SELECT artist_ids FROM mixes"), format!(r#"["{t}:7"]"#));
        // Likes and the section stay as they were.
        assert_eq!(q("SELECT id FROM tracks WHERE user_rating = 10.0"), format!("{t}:9"));
        assert_eq!(q("SELECT section_id FROM sync_state"), "1");

        // Ids saved elsewhere before the change map to the same items.
        let s = crate::settings::load(&db);
        assert_eq!(s.mix_exclude_artists, vec![format!("{t}:7")]);
        assert_eq!(s.mix_exclude_albums, vec![format!("{t}:8")]);
        assert_eq!(crate::sid::resolve("9"), format!("{t}:9"));
        assert_eq!(crate::sid::resolve_key("album:8"), format!("album:{t}:8"));
        assert_eq!(crate::sid::legacy_variant(&format!("album:{t}:8")).as_deref(), Some("album:8"));
        let found = db.read(|c| crate::db::queries::tracks_by_ids(c, &crate::sid::resolve_all(&["9".to_string()]))).unwrap();
        assert_eq!(found.len(), 1);
        drop(db);

        // Opened again (the next start): nothing to do, the tag is known.
        crate::sid::set_legacy(None);
        let db = crate::db::DbHandle::open(&path).unwrap();
        assert_eq!(crate::sid::legacy().as_deref(), Some(t.as_str()));
        assert!(!db.ids_pending());
        assert_eq!(db.qualify_legacy(&crate::sid::tag("machine-b")).unwrap(), 0);
        assert_eq!(db.read(|c| Ok(one(c, "SELECT id FROM albums"))).unwrap(), format!("{t}:8"));
        crate::sid::set_legacy(None);
    }

    #[test]
    fn a_rewrite_cut_short_leaves_the_cache_as_it_was_and_runs_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");
        old_cache(&path);
        let mut c = schema::open_writer(&path).unwrap();
        schema::migrate(&mut c).unwrap();
        // The last table the rewrite touches fails (a full disk, a crash).
        c.execute_batch("CREATE TRIGGER boom BEFORE UPDATE ON mixes BEGIN SELECT RAISE(ABORT, 'disk full'); END;")
            .unwrap();
        assert!(qualify_legacy(&mut c, "0a1b2c3d").is_err());
        assert!(pending(&c).unwrap(), "still owed");
        assert_eq!(legacy_tag(&c).unwrap(), None);
        assert_eq!(one(&c, "SELECT group_concat(id, ',') FROM (SELECT id FROM tracks ORDER BY id)"), "10,9", "nothing half done");
        assert_eq!(one(&c, "SELECT track_id FROM playlist_items"), "9");
        c.execute_batch("DROP TRIGGER boom;").unwrap();
        assert!(qualify_legacy(&mut c, "0a1b2c3d").unwrap() > 10);
        assert_eq!(one(&c, "SELECT group_concat(id, ',') FROM (SELECT id FROM tracks ORDER BY id)"), "0a1b2c3d:10,0a1b2c3d:9");
        // Run twice: the second time changes nothing.
        assert_eq!(qualify_legacy(&mut c, "0a1b2c3d").unwrap(), 0);
        assert_eq!(one(&c, "SELECT track_id FROM playlist_items"), "0a1b2c3d:9");
    }

    #[test]
    fn a_new_cache_just_records_the_tag() {
        let (_d, mut c) = db();
        assert_eq!(qualify_legacy(&mut c, "0a1b2c3d").unwrap(), 0);
        assert_eq!(legacy_tag(&c).unwrap().as_deref(), Some("0a1b2c3d"));
        assert!(qualify_legacy(&mut c, "nope").is_err());
    }
}
