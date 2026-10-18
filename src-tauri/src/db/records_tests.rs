use rusqlite::{Connection, params};

use super::listens::{self, ContextKind, PlayStart};
use super::records::*;
use super::schema::migrate;

const MIN: i64 = 60_000;
const DAY: i64 = 86_400_000;
/// "Now" for these tests: 2026-10-17 12:00 UTC.
const NOW: i64 = 1_792_238_400_000;

fn db() -> Connection {
    let mut c = Connection::open_in_memory().unwrap();
    migrate(&mut c).unwrap();
    c
}

/// An album `id` with `n` songs `{id}-1` .. `{id}-n` (listed out of order, to
/// check the album order wins).
fn album(c: &Connection, id: &str, n: u32) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, thumb, track_count, added_at)
         VALUES (?1, ?1, ?1, 'ar1', 'Band', ?2, ?3, 0)",
        params![id, format!("/al/{id}"), n],
    )
    .unwrap();
    for i in (1..=n).rev() {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, duration_ms, track_number)
             VALUES (?1, ?2, ?3, ?3, 'ar1', 'Band', '/p', 240000, ?4)",
            params![format!("{id}-{i}"), format!("Song {i}"), id, i],
        )
        .unwrap();
    }
}

/// A play of song `n` of `album`, started from the album at `at`. `how`:
/// "done" (finished), "skip", "stop" (stopped partway), "on" (still playing).
fn play(c: &Connection, album: &str, n: u32, at: i64, how: &str) {
    let id = listens::start_play(
        c,
        &PlayStart {
            rating_key: format!("{album}-{n}"),
            part_key: None,
            started_at: at,
            duration_ms: 240_000,
            context_kind: Some(ContextKind::Album),
            context_id: Some(album.into()),
            device: None,
            offline: false,
        },
    )
    .unwrap();
    match how {
        "done" => drop(listens::end_play(c, id, at + 4 * MIN, 240_000, 240_000, true).unwrap()),
        "skip" => drop(listens::end_play(c, id, at + 5_000, 5_000, 5_000, false).unwrap()),
        "stop" => drop(listens::end_play(c, id, at + 2 * MIN, 120_000, 120_000, false).unwrap()),
        _ => {}
    }
}

/// Songs 1..=n of `album` played through, one after the other from `at`.
fn side(c: &Connection, album: &str, n: u32, at: i64) {
    for i in 1..=n {
        play(c, album, i, at + (i as i64 - 1) * 4 * MIN, "done");
    }
}

fn tracks(n: u32) -> Vec<(String, String)> {
    (1..=n).map(|i| (format!("t{i}"), format!("Song {i}"))).collect()
}

fn s(id: &str, moved_on: bool) -> (String, bool) {
    (id.to_string(), moved_on)
}

#[test]
fn stopped_at_picks_the_next_song_or_the_one_left_in_the_middle() {
    let t = tracks(8);
    // Two finished: pick up at the third.
    assert_eq!(stopped_at(&t, &[s("t1", true), s("t2", true)]), Some(2));
    // Stopped in the middle of the third: start it again.
    assert_eq!(stopped_at(&t, &[s("t1", true), s("t2", true), s("t3", false)]), Some(2));
    // A skip moves on too.
    assert_eq!(stopped_at(&t, &[s("t1", true), s("t4", true)]), Some(4));
}

#[test]
fn stopped_at_leaves_out_finished_barely_started_and_short_albums() {
    let t = tracks(8);
    // Played to the end.
    assert_eq!(stopped_at(&t, &[s("t7", true), s("t8", true)]), None);
    // One song is a sample, not a start.
    assert_eq!(stopped_at(&t, &[s("t1", true)]), None);
    // Three songs is a single, never "partway".
    assert_eq!(stopped_at(&tracks(3), &[s("t1", true), s("t2", true)]), None);
    // A song not on the album (removed since).
    assert_eq!(stopped_at(&t, &[s("t1", true), s("gone", true)]), None);
    assert_eq!(stopped_at(&t, &[]), None);
}

#[test]
fn finish_what_you_started_finds_albums_stopped_partway() {
    let c = db();
    album(&c, "half", 10);
    album(&c, "whole", 4);
    album(&c, "sample", 10);
    // Half: five of ten, three days ago.
    side(&c, "half", 5, NOW - 3 * DAY);
    // Whole: all four, yesterday.
    side(&c, "whole", 4, NOW - DAY);
    // Sample: one song.
    play(&c, "sample", 1, NOW - 2 * DAY, "done");
    let got = unfinished(&c, NOW).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    let p = &got[0];
    assert_eq!((p.album.id.as_str(), p.next_track_id.as_str(), p.next_number, p.total), ("half", "half-6", 6, 10));
    assert_eq!(p.next_title, "Song 6");
}

#[test]
fn only_the_latest_session_counts() {
    let c = db();
    album(&c, "a", 6);
    album(&c, "b", 6);
    // A: three songs a week ago, then played through yesterday: finished.
    side(&c, "a", 3, NOW - 7 * DAY);
    side(&c, "a", 6, NOW - DAY);
    // B: played through a week ago, then two songs today: partway again.
    side(&c, "b", 6, NOW - 7 * DAY);
    side(&c, "b", 2, NOW - 3 * 3_600_000);
    let got = unfinished(&c, NOW).unwrap();
    assert_eq!(got.iter().map(|p| p.album.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
    assert_eq!(got[0].next_number, 3);
}

#[test]
fn old_sessions_fall_off_the_shelf() {
    let c = db();
    album(&c, "old", 8);
    side(&c, "old", 3, NOW - 90 * DAY);
    assert!(unfinished(&c, NOW).unwrap().is_empty());
}

#[test]
fn the_resume_target_is_the_newest_unfinished_album_and_leaves_the_shelf() {
    let c = db();
    album(&c, "older", 8);
    album(&c, "newer", 8);
    side(&c, "older", 3, NOW - 5 * DAY);
    side(&c, "newer", 2, NOW - 2 * 3_600_000);
    // Left in the middle of song 3.
    play(&c, "newer", 3, NOW - 2 * 3_600_000 + 8 * MIN, "stop");
    let home = record_home(&c, NOW, 0).unwrap();
    let r = home.resume.expect("a record to resume");
    assert_eq!((r.album.id.as_str(), r.next_track_id.as_str(), r.next_number), ("newer", "newer-3", 3));
    assert_eq!(home.unfinished.iter().map(|p| p.album.id.as_str()).collect::<Vec<_>>(), vec!["older"]);
}

#[test]
fn nothing_recent_to_resume_means_no_hero() {
    let c = db();
    album(&c, "a", 8);
    side(&c, "a", 3, NOW - 30 * DAY);
    let home = record_home(&c, NOW, 0).unwrap();
    assert!(home.resume.is_none());
    // Still on the shelf: a month is inside its window.
    assert_eq!(home.unfinished.len(), 1);
}

#[test]
fn the_song_playing_now_is_where_resume_starts() {
    let c = db();
    album(&c, "a", 8);
    side(&c, "a", 2, NOW - 10 * MIN);
    play(&c, "a", 3, NOW - MIN, "on");
    let r = record_home(&c, NOW, 0).unwrap().resume.unwrap();
    assert_eq!(r.next_track_id, "a-3");
}

#[test]
fn a_long_pause_starts_a_new_session() {
    let c = db();
    album(&c, "a", 8);
    // Two songs last night, one this morning after a long gap: this
    // morning's lone song is the latest session, too short to count.
    side(&c, "a", 2, NOW - 14 * 3_600_000);
    play(&c, "a", 5, NOW - 3_600_000, "done");
    assert!(unfinished(&c, NOW).unwrap().is_empty());
}

#[test]
fn spun_this_week_counts_the_last_seven_days() {
    let c = db();
    album(&c, "hot", 6);
    album(&c, "warm", 6);
    album(&c, "once", 6);
    album(&c, "last_month", 6);
    let today = crate::stats::today(&c).unwrap();
    let now = crate::db::queries::now_s() * 1000;
    side(&c, "hot", 5, now - DAY);
    side(&c, "warm", 3, now - 3 * DAY);
    play(&c, "once", 1, now - DAY, "done");
    side(&c, "last_month", 6, now - 30 * DAY);
    let mut c = c;
    crate::stats::refresh(&mut c).unwrap();
    let got = spun_this_week(&c, today).unwrap();
    assert_eq!(got.iter().map(|s| (s.album.id.as_str(), s.plays)).collect::<Vec<_>>(), vec![("hot", 5), ("warm", 3)]);
}

#[test]
fn not_played_in_a_year_finds_old_favourites() {
    let mut c = db();
    album(&c, "fave", 6);
    album(&c, "recent", 6);
    album(&c, "barely", 6);
    album(&c, "plex_fave", 6);
    album(&c, "plex_recent", 6);
    let now = crate::db::queries::now_s() * 1000;
    // Played a lot two years ago.
    side(&c, "fave", 6, now - 730 * DAY);
    side(&c, "fave", 6, now - 700 * DAY);
    // Played a lot, the last time last month.
    side(&c, "recent", 6, now - 400 * DAY);
    play(&c, "recent", 1, now - 30 * DAY, "done");
    // Played twice, long ago: never a favourite.
    side(&c, "barely", 2, now - 500 * DAY);
    // Plex knows plays from other devices.
    c.execute("UPDATE albums SET view_count = 20, last_viewed_at = ?1 WHERE id = 'plex_fave'", [(now - 400 * DAY) / 1000]).unwrap();
    c.execute("UPDATE albums SET view_count = 20, last_viewed_at = ?1 WHERE id = 'plex_recent'", [(now - 10 * DAY) / 1000]).unwrap();
    // Recent here, but Plex's count says it was a favourite: the later play wins.
    crate::stats::refresh(&mut c).unwrap();
    let got = not_in_a_year(&c, now).unwrap();
    let ids: Vec<_> = got.iter().map(|l| l.album.id.as_str()).collect();
    assert_eq!(ids, vec!["plex_fave", "fave"], "{got:?}");
    assert_eq!(got[1].plays, 12);
    assert!(got[1].last_played < now - 365 * DAY);
}

#[test]
fn new_arrivals_are_recent_adds_newest_first() {
    let c = db();
    album(&c, "new", 4);
    album(&c, "newer", 4);
    album(&c, "old", 4);
    let now_s = NOW / 1000;
    c.execute("UPDATE albums SET added_at = ?1 WHERE id = 'new'", [now_s - 10 * 86_400]).unwrap();
    c.execute("UPDATE albums SET added_at = ?1 WHERE id = 'newer'", [now_s - 86_400]).unwrap();
    c.execute("UPDATE albums SET added_at = ?1 WHERE id = 'old'", [now_s - 400 * 86_400]).unwrap();
    let got = new_arrivals(&c, now_s).unwrap();
    assert_eq!(got.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["newer", "new"]);
}

#[test]
fn an_empty_library_has_nothing_to_show() {
    let c = db();
    let home = record_home(&c, NOW, 0).unwrap();
    assert!(home.shown && home.resume.is_none());
    assert!(home.new_arrivals.is_empty() && home.spun_this_week.is_empty() && home.unfinished.is_empty() && home.not_in_a_year.is_empty());
}

#[test]
fn the_stamp_moves_when_a_play_ends() {
    let c = db();
    album(&c, "a", 6);
    let now = crate::db::queries::now_s() * 1000;
    play(&c, "a", 1, now - MIN, "on");
    let before = stamp(&c, now).unwrap();
    c.execute("UPDATE listen_log SET ended_at = ?1, completed = 1", [now]).unwrap();
    assert_ne!(stamp(&c, now).unwrap(), before);
}
