//! "Played a lot" and history-only imports, on synthetic plays and a
//! synthetic library. No real listening history anywhere.

use std::collections::HashMap;
use std::io::Write;

use rusqlite::{Connection, params};
use serde_json::json;

use super::played::{self, Kind, Period, Query};
use crate::db::DbHandle;
use crate::db::listens::{ImportSource, ImportedPlay, insert_imported};
use crate::db::schema;
use crate::identity::index;

const A: &str = "srv";
const DAY: i64 = 86_400_000;
/// 2026-06-01, "now" for these tests.
const NOW: i64 = 1_780_272_000_000;

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
    schema::migrate(&mut c).unwrap();
    (dir, c)
}

fn song(c: &Connection, id: &str, album: &str, artist: &str, title: &str) {
    c.execute(
        "INSERT OR IGNORE INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, matched, subformats)
         VALUES (?1, ?2, ?2, ?3, ?4, 1, ?5, 1, '[]')",
        params![format!("{A}:al:{album}"), album, format!("{A}:ar:{artist}"), artist, A],
    )
    .unwrap();
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, src)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 200000, ?7, ?8)",
        params![format!("{A}:{id}"), title, format!("{A}:al:{album}"), album, format!("{A}:ar:{artist}"), artist, format!("/p/{id}"), A],
    )
    .unwrap();
}

/// A play, `days_ago` before NOW, `secs` long.
fn play(n: usize, artist: &str, title: &str, album: &str, days_ago: i64, secs: u64, key: Option<&str>) -> ImportedPlay {
    ImportedPlay {
        source: ImportSource::File,
        source_id: format!("streaming:{n}:{artist}:{title}"),
        played_at: NOW - days_ago * DAY - n as i64 * 1000,
        ms_played: Some(secs * 1000),
        title: title.into(),
        artist: Some(artist.into()),
        album: Some(album.into()),
        rating_key: key.map(|k| format!("{A}:{k}")),
    }
}

fn plays(c: &mut Connection, list: Vec<ImportedPlay>) {
    insert_imported(c, &list, NOW).unwrap();
}

/// Harbor Lights' "Paper Boats" 8 times, "Undertow" 6 times (longer plays,
/// same album "Low Water"); Mira Vale's "Glass Orchard" 6 times (shorter);
/// Cass Morrow 2 times; a song in the library played 9 times; skips.
fn history(c: &mut Connection) {
    song(c, "1", "Shoreline", "Low Tide", "Golden Hour");
    song(c, "2", "Low Water", "Harbor Lights", "First Light");
    index::rebuild(c).unwrap();
    let mut v = Vec::new();
    let mut n = 0;
    let mut add = |v: &mut Vec<ImportedPlay>, a: &str, t: &str, al: &str, days: i64, secs: u64, key: Option<&str>| {
        n += 1;
        v.push(play(n, a, t, al, days, secs, key));
    };
    for i in 0..8 {
        // Most of them long ago, two this year.
        add(&mut v, "Harbor Lights", if i == 0 { "PAPER BOATS" } else { "Paper Boats" }, "Low Water", if i < 6 { 900 } else { 30 }, 200, None);
    }
    for _ in 0..6 {
        add(&mut v, "Harbor Lights", "Undertow", "Low Water", 400, 300, None);
    }
    for _ in 0..6 {
        add(&mut v, "Mira Vale", "Glass Orchard", "Second Bloom", 10, 100, None);
    }
    for _ in 0..2 {
        add(&mut v, "Cass Morrow", "Thin Ice Waltz", "Winter Rooms", 5, 200, None);
    }
    for _ in 0..9 {
        add(&mut v, "Low Tide", "Golden Hour", "Shoreline", 3, 200, Some("1"));
    }
    // Skips never count.
    for _ in 0..10 {
        add(&mut v, "Osa Lind", "Field Notes", "Field Notes", 2, 12, None);
    }
    plays(c, v);
}

fn q(kind: Kind, min: u32, period: &str) -> Query {
    Query { kind, offset: 0, min_plays: min, period: period.into() }
}

fn titles(p: &played::PlayedPage) -> Vec<(String, u64)> {
    p.rows.iter().map(|r| (r.title.clone(), r.plays)).collect()
}

#[test]
fn songs_group_by_folded_names_rank_by_plays_then_time_and_leave_out_matches_and_skips() {
    let (_d, mut c) = db();
    history(&mut c);
    let p = played::list(&c, &q(Kind::Song, 5, "all"), &[], NOW).unwrap();
    // Paper Boats (8, two spellings), then Undertow (6, longer) before
    // Glass Orchard (6, shorter). Golden Hour is in the library; Cass
    // Morrow has too few plays; Field Notes was only ever skipped.
    assert_eq!(titles(&p), [("Paper Boats".into(), 8), ("Undertow".into(), 6), ("Glass Orchard".into(), 6)]);
    assert_eq!(p.total, 3);
    assert!(p.has_history);
    let top = &p.rows[0];
    assert_eq!(top.artist, "Harbor Lights");
    assert_eq!(top.album.as_deref(), Some("Low Water"));
    assert_eq!(top.ms, 8 * 200_000);
    assert!(top.first_played < top.last_played);
    assert!(top.links.bandcamp.contains("Harbor%20Lights%20Paper%20Boats"));
    // Fewer plays brings Cass Morrow in.
    let p = played::list(&c, &q(Kind::Song, 1, "all"), &[], NOW).unwrap();
    assert_eq!(p.total, 4);
}

#[test]
fn the_period_filter_counts_only_plays_inside_it() {
    let (_d, mut c) = db();
    history(&mut c);
    // Last year: Glass Orchard 6 times, Cass Morrow and Paper Boats twice
    // each for the same time (ties go by artist).
    let p = played::list(&c, &q(Kind::Song, 2, "year"), &[], NOW).unwrap();
    assert_eq!(titles(&p), [("Glass Orchard".into(), 6), ("Thin Ice Waltz".into(), 2), ("Paper Boats".into(), 2)]);
    let p = played::list(&c, &q(Kind::Song, 1, "days90"), &[], NOW).unwrap();
    assert_eq!(titles(&p), [("Glass Orchard".into(), 6), ("Thin Ice Waltz".into(), 2), ("Paper Boats".into(), 2)]);
    let p = played::list(&c, &q(Kind::Song, 1, "days90"), &[], NOW + 60 * DAY).unwrap();
    assert_eq!(titles(&p), [("Glass Orchard".into(), 6), ("Thin Ice Waltz".into(), 2)], "two months on, Paper Boats is outside the 90 days");
    assert!(played::list(&c, &q(Kind::Song, 1, "decade"), &[], NOW).is_err());
    assert_eq!(Period::parse("year").unwrap().since(NOW), NOW - 365 * DAY);
}

#[test]
fn albums_count_songs_played_and_tracks_already_in_the_library() {
    let (_d, mut c) = db();
    history(&mut c);
    let p = played::list(&c, &q(Kind::Album, 5, "all"), &[], NOW).unwrap();
    let rows: Vec<(String, u64, Option<u64>, Option<u64>)> = p.rows.iter().map(|r| (r.title.clone(), r.plays, r.songs, r.in_library)).collect();
    // Low Water: 14 plays of 2 songs, and the library has one of its songs.
    assert_eq!(rows, [("Low Water".into(), 14, Some(2), Some(1)), ("Second Bloom".into(), 6, Some(1), Some(0))]);
    assert!(p.rows[0].links.bandcamp.contains("item_type=a"));
    assert_eq!(p.rows[0].kind, Kind::Album);
}

#[test]
fn hidden_rows_and_artists_leave_the_list() {
    let (_d, mut c) = db();
    history(&mut c);
    let song = played::hide_key_for("song", "Harbor Lights", "Paper Boats").unwrap();
    let p = played::list(&c, &q(Kind::Song, 5, "all"), std::slice::from_ref(&song), NOW).unwrap();
    assert_eq!(titles(&p), [("Undertow".into(), 6), ("Glass Orchard".into(), 6)]);
    assert_eq!(p.total, 2);
    // The key the list gives a row is the one hiding it uses.
    let all = played::list(&c, &q(Kind::Song, 5, "all"), &[], NOW).unwrap();
    assert_eq!(all.rows[0].key, song);
    let artist = played::hide_key_for("artist", "harbor lights", "").unwrap();
    let p = played::list(&c, &q(Kind::Song, 5, "all"), std::slice::from_ref(&artist), NOW).unwrap();
    assert_eq!(titles(&p), [("Glass Orchard".into(), 6)]);
    let p = played::list(&c, &q(Kind::Album, 5, "all"), &[artist], NOW).unwrap();
    assert_eq!(p.rows.len(), 1);
    let album = played::hide_key_for("album", "Mira Vale", "Second Bloom").unwrap();
    let p = played::list(&c, &q(Kind::Album, 5, "all"), &[album], NOW).unwrap();
    assert_eq!(p.rows[0].title, "Low Water");
    assert!(played::hide_key_for("playlist", "x", "y").is_err());
    assert!(played::hide_key_for("song", "", "y").is_err());
    // Long names still make keys the sync store takes.
    let long = "a".repeat(400);
    assert!(played::hide_key_for("song", &long, &long).unwrap().chars().count() <= 190);
}

#[test]
fn hidden_rows_sync_through_the_store() {
    use crate::sync::store::{NAMESPACES, PLAYED_HIDDEN};
    assert!(NAMESPACES.contains(&PLAYED_HIDDEN.name()));
    let g = crate::sync::engine::Groups { follows: true, settings: false, library: false, lyrics: false, diary: false, looks: false, downloads: false, plays: false };
    assert!(g.namespaces().contains(&PLAYED_HIDDEN.name()));
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("s.db")).unwrap();
    let s = crate::sync::SyncStore::new(db, "dev");
    let key = played::hide_key_for("album", "Mira Vale", "Second Bloom").unwrap();
    s.put(&PLAYED_HIDDEN, &key, &played::HiddenRow { kind: "album".into(), artist: "Mira Vale".into(), name: "Second Bloom".into(), hidden_at: 5 })
        .unwrap();
    let back: Vec<(String, played::HiddenRow)> = s.list(&PLAYED_HIDDEN).unwrap();
    assert_eq!(back[0].0, key);
    assert_eq!(back[0].1.name, "Second Bloom");
}

#[test]
fn pages_of_a_hundred() {
    let (_d, mut c) = db();
    let mut v = Vec::new();
    let mut n = 0;
    for s in 0..150 {
        for _ in 0..(1 + s % 3) {
            n += 1;
            v.push(play(n, "Paper Boats", &format!("Song {s}"), "Long Album", 1, 60, None));
        }
    }
    plays(&mut c, v);
    let p = played::list(&c, &q(Kind::Song, 1, "all"), &[], NOW).unwrap();
    assert_eq!((p.rows.len(), p.total), (100, 150));
    let next = played::list(&c, &Query { offset: 100, ..q(Kind::Song, 1, "all") }, &[], NOW).unwrap();
    assert_eq!((next.rows.len(), next.total, next.offset), (50, 150, 100));
    let past = played::list(&c, &Query { offset: 400, ..q(Kind::Song, 1, "all") }, &[], NOW).unwrap();
    assert_eq!((past.rows.len(), past.total), (0, 150));
    assert!(p.rows.windows(2).all(|w| w[0].plays >= w[1].plays));
}

#[test]
fn no_history_says_so() {
    let (_d, c) = db();
    let p = played::list(&c, &q(Kind::Song, 5, "all"), &[], NOW).unwrap();
    assert!(!p.has_history);
    assert!(p.rows.is_empty());
}

#[test]
fn sources_left_out_of_every_count_are_left_out_here() {
    let (_d, mut c) = db();
    history(&mut c);
    crate::db::listens::set_excluded_sources(&mut c, &[ImportSource::File]).unwrap();
    let p = played::list(&c, &q(Kind::Song, 1, "all"), &[], NOW).unwrap();
    assert!(p.rows.is_empty());
}

#[test]
fn rows_leave_when_the_library_gains_the_song_and_stats_count_the_plays() {
    let (_d, mut c) = db();
    history(&mut c);
    assert_eq!(played::rematch(&mut c, HashMap::new(), NOW).unwrap(), 0, "nothing new in the library");
    c.execute("DELETE FROM stats_dirty", []).unwrap();
    song(&c, "3", "Second Bloom", "Mira Vale", "Glass Orchard");
    index::rebuild(&c).unwrap();
    assert_eq!(played::rematch(&mut c, HashMap::new(), NOW).unwrap(), 6);
    let keys: i64 = c.query_row("SELECT count(*) FROM imported_plays WHERE rating_key = ?1", [format!("{A}:3")], |r| r.get(0)).unwrap();
    assert_eq!(keys, 6);
    // Stats count those days again.
    let dirty: i64 = c.query_row("SELECT count(*) FROM stats_dirty", [], |r| r.get(0)).unwrap();
    assert!(dirty > 0);
    let p = played::list(&c, &q(Kind::Song, 5, "all"), &[], NOW).unwrap();
    assert_eq!(titles(&p), [("Paper Boats".into(), 8), ("Undertow".into(), 6)]);
    // Nothing changed since: the check is skipped.
    assert_eq!(played::rematch(&mut c, HashMap::new(), NOW).unwrap(), 0);
}

#[test]
fn the_index_migration_folds_older_rows_and_the_list_uses_its_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
    // A database from before the folded columns.
    let before = schema::MIGRATIONS_FOR_TESTS.iter().position(|m| m.contains("imported_plays_unmatched_song")).unwrap();
    schema::migrate_to(&mut c, before).unwrap();
    c.execute(
        "INSERT INTO imported_plays (source, source_id, played_at, ms_played, title, artist, album)
         VALUES ('file', 'streaming:1', ?1, 200000, 'Paper Boats (2019 Remaster)', 'Harbor Lights', 'Low Water')",
        [NOW],
    )
    .unwrap();
    schema::migrate(&mut c).unwrap();
    let names: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'imported_plays'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for n in ["imported_plays_unmatched_song", "imported_plays_unmatched_album", "imported_plays_unfolded"] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
    assert_eq!(played::fill_folds(&mut c).unwrap(), 1);
    assert_eq!(played::fill_folds(&mut c).unwrap(), 0);
    let (a, t, al): (String, String, String) =
        c.query_row("SELECT artist_fold, title_fold, album_fold FROM imported_plays", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!((a.as_str(), t.as_str(), al.as_str()), ("harbor lights", "paper boats", "low water"));
    let p = played::list(&c, &q(Kind::Song, 1, "all"), &[], NOW).unwrap();
    assert_eq!(p.rows[0].title, "Paper Boats (2019 Remaster)");
    // The grouping reads the partial index, not the whole table.
    let plan: Vec<String> = c
        .prepare(
            "EXPLAIN QUERY PLAN SELECT artist_fold, title_fold, count(*) FROM imported_plays INDEXED BY imported_plays_unmatched_song
             WHERE rating_key IS NULL AND title_fold IS NOT NULL GROUP BY artist_fold, title_fold",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(plan.iter().any(|l| l.contains("COVERING INDEX imported_plays_unmatched_song")), "{plan:?}");
}

// ------------------------------------------------------------------ history-only imports

/// An extended history file, made up: plays from 2020 and 2026 of one song
/// in the library and two that aren't, a podcast and a skip.
fn history_zip() -> Vec<u8> {
    let rec = |ts: &str, ms: u64, artist: &str, title: &str, album: &str| {
        json!({
            "ts": ts, "ms_played": ms, "master_metadata_track_name": title,
            "master_metadata_album_artist_name": artist, "master_metadata_album_album_name": album,
            "episode_name": null, "incognito_mode": false
        })
    };
    let records = json!([
        rec("2020-02-01T10:00:00Z", 200_000, "Low Tide", "Golden Hour", "Shoreline"),
        rec("2021-02-01T10:00:00Z", 200_000, "Low Tide", "Golden Hour", "Shoreline"),
        rec("2022-02-01T10:00:00Z", 190_000, "Harbor Lights", "Paper Boats", "Low Water"),
        rec("2023-02-01T10:00:00Z", 190_000, "Harbor Lights", "Paper Boats", "Low Water"),
        rec("2026-02-01T10:00:00Z", 180_000, "Mira Vale", "Glass Orchard", "Second Bloom"),
        rec("2026-02-02T10:00:00Z", 5_000, "Mira Vale", "Glass Orchard", "Second Bloom"),
        json!({"ts": "2026-02-03T10:00:00Z", "ms_played": 900000, "master_metadata_track_name": null, "episode_name": "Ep 1", "episode_show_name": "A Show"}),
    ]);
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("Extended Streaming History/Streaming_History_Audio_2020-2026_0.json", opts).unwrap();
        z.write_all(records.to_string().as_bytes()).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn a_history_only_zip_reports_plays_years_and_songs_through_the_whole_command() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("h.db")).unwrap();
    db.write(|c| {
        song(c, "1", "Shoreline", "Low Tide", "Golden Hour");
        index::rebuild(c)?;
        Ok(())
    })
    .unwrap();
    let zip = dir.path().join("my_extended_history.zip");
    std::fs::write(&zip, history_zip()).unwrap();
    let src = super::sources::source("streaming").unwrap();
    let r = super::commands::run(&db, src, &zip, NOW).unwrap();
    // No playlists or likes: the song counts are zero, the history isn't.
    assert_eq!(r.songs.total, 0);
    assert_eq!(r.playlists, 0);
    let h = &r.history;
    assert_eq!((h.plays, h.plays_matched, h.songs, h.songs_matched), (5, 2, 3, 1));
    assert_eq!(h.first_played.map(|t| t / 1000 / 86_400 / 365 + 1970), Some(2020));
    assert_eq!(h.last_played.map(|t| t / 1000 / 86_400 / 365 + 1970), Some(2026));
    assert_eq!(r.plays_added, 5);
    assert_eq!(r.skipped.short_plays, 1);
    assert_eq!(r.skipped.podcasts, 1);
    // The unmatched plays are on "Played a lot".
    let p = db.read(|c| played::list(c, &q(Kind::Song, 1, "all"), &[], NOW)).unwrap();
    assert_eq!(titles(&p), [("Paper Boats".into(), 2), ("Glass Orchard".into(), 1)]);
    // And stats count every play, matched or not.
    db.write(|c| crate::stats::refresh(c).map(|_| ())).unwrap();
    let o = db
        .read(|c| {
            let d = crate::stats::resolve(c, &crate::stats::StatsRange::default())?;
            crate::stats::overview(c, d, 10)
        })
        .unwrap();
    assert_eq!(o.totals.plays, 5);
    assert_eq!(o.top_songs.iter().map(|s| s.plays).sum::<i64>(), 5);
    assert!(o.top_songs.iter().any(|s| s.title == "Golden Hour" && s.track_id.as_deref() == Some("srv:1")));
}

#[test]
fn a_file_with_nothing_usable_says_what_was_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("e.db")).unwrap();
    let f = dir.path().join("Streaming_History_Audio_2024.json");
    std::fs::write(
        &f,
        json!([
            {"ts": "2024-01-01T00:00:00Z", "ms_played": 4000, "master_metadata_track_name": "Song", "master_metadata_album_artist_name": "Someone"},
            {"ts": "2024-01-02T00:00:00Z", "ms_played": 900000, "master_metadata_track_name": null, "episode_name": "Ep", "episode_show_name": "Show"}
        ])
        .to_string(),
    )
    .unwrap();
    let e = super::commands::run(&db, super::sources::source("streaming").unwrap(), &f, NOW).unwrap_err();
    let m = e.message();
    assert!(m.contains("no songs Platter could use"), "{m}");
    assert!(m.contains("1 play under 30 seconds") && m.contains("1 podcast or audiobook record"), "{m}");
    assert_eq!(super::commands::nothing_usable(&super::Skipped::default()), "Those files had no songs, playlists or plays in them.");
}

#[test]
fn played_settings_default_validate_and_sync() {
    let s = crate::settings::Settings::default();
    assert!(s.show_played_a_lot);
    assert_eq!((s.played_min_plays, s.played_period.as_str()), (5, "all"));
    for k in ["showPlayedALot", "playedMinPlays", "playedPeriod"] {
        assert!(crate::settings::SYNCED_SETTINGS.contains(&k), "{k} is synced");
    }
    let mut s = crate::settings::Settings::default();
    let p: crate::settings::SettingsPatch = serde_json::from_value(json!({"playedMinPlays": 0, "playedPeriod": "days90", "showPlayedALot": false})).unwrap();
    s.apply(&p).unwrap();
    assert_eq!((s.played_min_plays, s.played_period.as_str(), s.show_played_a_lot), (1, "days90", false));
    let bad: crate::settings::SettingsPatch = serde_json::from_value(json!({"playedPeriod": "decade"})).unwrap();
    assert!(s.apply(&bad).is_err());
}

#[test]
fn a_library_that_swaps_one_song_for_another_is_matched_again() {
    let (_d, mut c) = db();
    history(&mut c);
    assert_eq!(played::rematch(&mut c, HashMap::new(), NOW).unwrap(), 0);
    let before: i64 = c.query_row("SELECT count(*) FROM song_index", [], |r| r.get(0)).unwrap();
    // One song goes, the one the plays name comes: the same count.
    c.execute("DELETE FROM tracks WHERE id = (SELECT min(id) FROM tracks)", []).unwrap();
    song(&c, "9", "Second Bloom", "Mira Vale", "Glass Orchard");
    index::rebuild(&c).unwrap();
    let after: i64 = c.query_row("SELECT count(*) FROM song_index", [], |r| r.get(0)).unwrap();
    assert_eq!(before, after);
    assert_eq!(played::rematch(&mut c, HashMap::new(), NOW).unwrap(), 6);
}

/// `count` plays of `artist` starting at `utc_hour` on days before NOW.
fn at_hour(c: &mut Connection, artist: &str, count: usize, utc_hour: i64, secs: u64) {
    let v = (0..count)
        .map(|i| ImportedPlay {
            source: ImportSource::File,
            source_id: format!("streaming:{artist}:{utc_hour}:{i}"),
            played_at: NOW - (i as i64 + 1) * DAY + utc_hour * 3_600_000,
            ms_played: Some(secs * 1000),
            title: format!("Song {}", i % 3),
            artist: Some(artist.into()),
            album: Some(format!("{artist} Album")),
            rating_key: None,
        })
        .collect();
    plays(c, v);
}

#[test]
fn artists_played_mostly_late_at_night_are_sleep_music() {
    let (_d, mut c) = db();
    // 7 hours behind UTC: local 23:30 is UTC 06:30, local 13:00 is UTC 20:00.
    let off = -7 * 3600;
    at_hour(&mut c, "Quiet Harbor", 22, 6, 200);
    at_hour(&mut c, "Quiet Harbor", 3, 20, 200);
    at_hour(&mut c, "Day Shift", 30, 20, 200);
    // Too few plays to say.
    at_hour(&mut c, "Few Nights", 10, 6, 200);
    // 75% at night is not enough.
    at_hour(&mut c, "Edge Case", 15, 6, 200);
    at_hour(&mut c, "Edge Case", 5, 20, 200);
    // Skips don't count.
    at_hour(&mut c, "Skipped Lull", 25, 6, 10);
    // UTC 03:00 is 20:00 here, but night in UTC.
    at_hour(&mut c, "Morning Run", 25, 3, 200);
    let sleep = played::sleep_artists(&c, &[], off).unwrap();
    assert_eq!(sleep.iter().map(|s| (s.artist.as_str(), s.plays, s.night_percent)).collect::<Vec<_>>(), [("Quiet Harbor", 25, 88)]);
    let utc: Vec<String> = played::sleep_artists(&c, &[], 0).unwrap().into_iter().map(|s| s.artist).collect();
    // Same plays: by name.
    assert_eq!(utc, ["Morning Run", "Quiet Harbor"]);
    // One the user kept stays in.
    let kept = played::keep_key("quiet  HARBOR").unwrap();
    assert!(played::sleep_artists(&c, &[kept], off).unwrap().is_empty());
    assert!(played::keep_key(" ").is_err());
}

#[test]
fn sleep_music_leaves_the_list_and_bring_in_only_with_the_setting_on() {
    let (_d, mut c) = db();
    at_hour(&mut c, "Quiet Harbor", 30, 2, 200);
    at_hour(&mut c, "Day Shift", 30, 14, 200);
    let artists = |p: &played::PlayedPage| {
        let mut v: Vec<String> = p.rows.iter().map(|r| r.artist.clone()).collect();
        v.dedup();
        v
    };
    // The test clock's zone is whatever the machine says; both hours stay on their side of night within 2 hours.
    let off = played::local_offset_now(&c);
    if off.abs() > 2 * 3600 {
        return;
    }
    let keys = played::with_sleep(&c, Vec::new(), true);
    assert_eq!(keys, ["artist:quiet harbor"]);
    let p = played::list(&c, &q(Kind::Album, 1, "all"), &keys, NOW).unwrap();
    assert_eq!(artists(&p), ["Day Shift"]);
    // Off: nothing is left out.
    assert!(played::with_sleep(&c, Vec::new(), false).is_empty());
    // Kept: back in, and the keep key hides no row.
    let kept = vec![played::keep_key("Quiet Harbor").unwrap()];
    let keys = played::with_sleep(&c, kept.clone(), true);
    assert_eq!(keys, kept);
    let p = played::list(&c, &q(Kind::Album, 1, "all"), &keys, NOW).unwrap();
    assert_eq!(p.rows.len(), 2);
    // Sources left out of every count are left out of the check too.
    crate::db::listens::set_excluded_sources(&mut c, &[ImportSource::File]).unwrap();
    assert!(played::with_sleep(&c, Vec::new(), true).is_empty());
}
