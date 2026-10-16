use rusqlite::Connection;

use super::tests::{MIN, add_track, db, library, local_ms, play};
use super::wrapped::*;
use super::{parse_day, refresh};
use crate::error::AppError;

fn day(s: &str) -> i64 {
    parse_day(s).unwrap()
}

/// `n` plays of `key` from a local time, ten minutes apart.
fn plays(c: &Connection, key: &str, from: &str, n: i64, ms: u64) {
    let at = local_ms(c, from);
    for i in 0..n {
        play(c, key, at + i * 600_000, ms);
    }
}

fn ids(l: &TopSongsList) -> Vec<&str> {
    l.rows.iter().map(|r| r.track.id.as_str()).collect()
}

#[test]
fn a_year_list_ranks_by_plays_then_minutes() {
    let mut c = db();
    library(&c);
    plays(&c, "t1", "2025-03-01 10:00:00", 3, 2 * MIN);
    plays(&c, "t2", "2025-03-02 10:00:00", 3, 3 * MIN);
    plays(&c, "t3", "2025-03-03 10:00:00", 24, 2 * MIN);
    refresh(&mut c).unwrap();
    let l = list(&c, Some(2025), day("2026-06-01")).unwrap();
    assert_eq!(l.title, "Your Top Songs 2025");
    assert_eq!(ids(&l), ["t3", "t2", "t1"], "most plays first; equal plays go to more minutes");
    assert_eq!(l.rows[1].plays, 3);
    assert_eq!(l.duration_ms, 3 * 200_000);
}

#[test]
fn equal_plays_and_minutes_keep_a_steady_order() {
    let mut c = db();
    library(&c);
    plays(&c, "t2", "2025-05-01 10:00:00", 15, 2 * MIN);
    plays(&c, "t1", "2025-05-02 10:00:00", 15, 2 * MIN);
    refresh(&mut c).unwrap();
    let a = list(&c, Some(2025), day("2026-06-01")).unwrap();
    let b = list(&c, Some(2025), day("2026-06-01")).unwrap();
    assert_eq!(ids(&a), ["t1", "t2"]);
    assert_eq!(ids(&a), ids(&b));
}

#[test]
fn years_follow_local_time() {
    let mut c = db();
    library(&c);
    // The last minutes of 2024 and the first of 2025, on this computer's clock.
    plays(&c, "t1", "2024-12-31 23:40:00", 1, 2 * MIN);
    plays(&c, "t2", "2025-01-01 00:05:00", 2, 2 * MIN);
    refresh(&mut c).unwrap();
    let ys = years(&c).unwrap();
    assert_eq!(ys.iter().map(|y| (y.0, y.1)).collect::<Vec<_>>(), [(2024, 1), (2025, 2)]);
    assert_eq!(wrapped(&c, 2024).unwrap().totals.plays, 1);
    let w = wrapped(&c, 2025).unwrap();
    assert_eq!(w.totals.plays, 2);
    assert_eq!(w.prev_ms, (2 * MIN) as i64, "the year before is 2024's listening");
    assert_eq!(w.first_song.unwrap().song.track_id.as_deref(), Some("t2"));
    assert_eq!(year_days(2024).hi + 1, year_days(2025).lo);
}

#[test]
fn a_year_needs_thirty_plays_for_its_list() {
    let mut c = db();
    library(&c);
    plays(&c, "t1", "2023-04-01 10:00:00", YEAR_LIST_MIN_PLAYS - 1, 2 * MIN);
    plays(&c, "t1", "2024-04-01 10:00:00", YEAR_LIST_MIN_PLAYS, 2 * MIN);
    refresh(&mut c).unwrap();
    let today = day("2025-06-01");
    let ix = index(&c, today).unwrap();
    assert_eq!(ix.years, [2024]);
    assert!(ix.all_time);
    assert_eq!(ix.wrapped_years, [2024, 2023], "Wrapped covers any year with plays");
    assert!(matches!(list(&c, Some(2023), today), Err(AppError::NotFound(_))));
    assert!(list(&c, Some(2024), today).is_ok());
    let all = list(&c, None, today).unwrap();
    assert_eq!(all.title, "All-Time Top Songs");
    assert_eq!(all.rows[0].plays, 2 * YEAR_LIST_MIN_PLAYS - 1);
    assert!(!all.rows[0].first_of_year, "All-Time has no first play of the year");
}

#[test]
fn this_years_list_and_wrapped_open_on_december_first() {
    let mut c = db();
    library(&c);
    plays(&c, "t1", "2026-02-01 10:00:00", 40, 2 * MIN);
    refresh(&mut c).unwrap();
    let nov = index(&c, day("2026-11-30")).unwrap();
    assert!(nov.years.is_empty() && nov.wrapped_years.is_empty());
    assert_eq!(nov.current_year, 2026);
    assert!(matches!(list(&c, Some(2026), day("2026-11-30")), Err(AppError::NotFound(_))));
    let dec = index(&c, day("2026-12-01")).unwrap();
    assert_eq!((dec.years.clone(), dec.wrapped_years.clone()), (vec![2026], vec![2026]));
    assert_eq!(dec.promoted, Some(2026));
}

#[test]
fn home_shows_the_new_list_from_december_to_mid_january() {
    assert_eq!(promoted_year(day("2026-11-30")), None);
    assert_eq!(promoted_year(day("2026-12-01")), Some(2026));
    assert_eq!(promoted_year(day("2026-12-31")), Some(2026));
    assert_eq!(promoted_year(day("2027-01-01")), Some(2026));
    assert_eq!(promoted_year(day("2027-01-15")), Some(2026));
    assert_eq!(promoted_year(day("2027-01-16")), None);
    assert_eq!(promoted_year(day("2027-07-04")), None);
}

#[test]
fn home_promotes_only_a_year_that_has_a_list() {
    let mut c = db();
    library(&c);
    plays(&c, "t1", "2026-02-01 10:00:00", 5, 2 * MIN);
    refresh(&mut c).unwrap();
    assert_eq!(index(&c, day("2026-12-10")).unwrap().promoted, None, "five plays make no list");
}

#[test]
fn the_years_first_play_is_marked() {
    let mut c = db();
    library(&c);
    plays(&c, "t3", "2025-01-02 08:00:00", 1, 2 * MIN);
    plays(&c, "t1", "2025-01-02 09:00:00", 30, 2 * MIN);
    refresh(&mut c).unwrap();
    let l = list(&c, Some(2025), day("2026-01-05")).unwrap();
    let marked: Vec<&str> = l.rows.iter().filter(|r| r.first_of_year).map(|r| r.track.id.as_str()).collect();
    assert_eq!(marked, ["t3"]);
}

#[test]
fn each_version_counts_as_the_file_heard() {
    let mut c = db();
    library(&c);
    add_track(&c, "t1live", "One", ("al1", "First"), ("ar1", "Band"), r#"["Rock"]"#);
    plays(&c, "t1", "2025-06-01 10:00:00", 20, 2 * MIN);
    plays(&c, "t1live", "2025-06-02 10:00:00", 12, 2 * MIN);
    refresh(&mut c).unwrap();
    let l = list(&c, Some(2025), day("2026-01-05")).unwrap();
    let got: Vec<(&str, i64)> = l.rows.iter().map(|r| (r.track.id.as_str(), r.plays)).collect();
    assert_eq!(got, [("t1", 20), ("t1live", 12)]);
}

#[test]
fn unreleased_songs_count_and_are_marked() {
    let mut c = db();
    library(&c);
    c.execute("INSERT INTO song_index (track_id, artist_fold, title_fold, artist_id, unreleased) VALUES ('t2', 'band', 'two', 'ar1', 1)", [])
        .unwrap();
    plays(&c, "t2", "2025-06-01 10:00:00", 20, 2 * MIN);
    plays(&c, "t1", "2025-06-02 10:00:00", 10, 2 * MIN);
    refresh(&mut c).unwrap();
    let l = list(&c, Some(2025), day("2026-01-05")).unwrap();
    assert_eq!(ids(&l), ["t2", "t1"]);
    assert!(l.rows[0].unreleased && !l.rows[1].unreleased);
    let w = wrapped(&c, 2025).unwrap();
    assert_eq!(w.totals.plays, 30, "unreleased plays count");
    assert!(w.top_songs[0].unreleased);
    assert!(w.top_albums[0].unreleased, "its album holds an unreleased song");
}

#[test]
fn wrapped_finds_the_years_numbers() {
    let mut c = db();
    library(&c);
    plays(&c, "t1", "2024-05-01 21:00:00", 2, 3 * MIN);
    plays(&c, "t1", "2025-02-10 22:30:00", 6, 3 * MIN);
    plays(&c, "t3", "2025-02-11 08:00:00", 1, 4 * MIN);
    plays(&c, "t2", "2025-08-20 23:00:00", 4, 2 * MIN);
    refresh(&mut c).unwrap();
    let w = wrapped(&c, 2025).unwrap();
    assert_eq!(w.year, 2025);
    assert_eq!((w.totals.plays, w.totals.songs, w.totals.days), (11, 3, 3));
    assert_eq!(w.prev_ms, (6 * MIN) as i64);
    assert_eq!(w.top_songs[0].track_id.as_deref(), Some("t1"));
    assert_eq!(w.top_artists[0].name, "Band");
    assert_eq!(w.top_genres[0].name, "Rock");
    let big = w.biggest_day.unwrap();
    assert_eq!((big.key.as_str(), big.plays), ("2025-02-10", 6));
    let rep = w.replayed.unwrap();
    assert_eq!((rep.song.track_id.as_deref(), rep.song.plays, rep.day.as_str()), (Some("t1"), 6, "2025-02-10"));
    assert_eq!(w.new_artists, 1, "Other is new in 2025; Band was played in 2024");
    let p = w.personality.unwrap();
    assert_eq!(p.time, TimeOfDay::Night);
    assert!(p.repeats, "six of eleven plays went to a song played five times or more");
    assert!(!p.mostly_new, "t1 was already a favourite");
    assert_eq!(w.hours.len(), 24);
    assert!(wrapped(&c, 12).is_err());
}

#[test]
fn an_empty_year_wraps_to_nothing() {
    let c = db();
    let w = wrapped(&c, 2025).unwrap();
    assert_eq!(w.totals.plays, 0);
    assert!(w.personality.is_none() && w.biggest_day.is_none() && w.top_songs.is_empty());
}

#[test]
fn personality_rules() {
    let mut hours = vec![0i64; 24];
    assert!(personality(&hours, 10, 0, 0).is_none(), "no listening, no personality");
    hours[7] = 100;
    hours[23] = 100;
    hours[2] = 50;
    let p = personality(&hours, 10, 5, 4).unwrap();
    assert_eq!(p.time, TimeOfDay::Night, "11 PM and 2 AM are both night");
    assert!((p.time_share - 0.6).abs() < 1e-9);
    assert!(p.repeats, "exactly half is a repeater");
    assert!(!p.mostly_new);
    assert!((p.new_share - 0.4).abs() < 1e-9);
    let p = personality(&hours, 10, 4, 5).unwrap();
    assert!(!p.repeats && p.mostly_new, "deep cuts, mostly new finds");
    // A tie between parts of the day goes to the earlier one.
    let mut tie = vec![0i64; 24];
    tie[9] = 60;
    tie[14] = 60;
    assert_eq!(personality(&tie, 1, 0, 0).unwrap().time, TimeOfDay::Morning);
    // Shares never pass 1.
    assert_eq!(personality(&tie, 2, 9, 9).unwrap().repeat_share, 1.0);
    for (h, want) in [(4, TimeOfDay::Night), (5, TimeOfDay::Morning), (12, TimeOfDay::Afternoon), (17, TimeOfDay::Evening), (22, TimeOfDay::Night)] {
        assert_eq!(time_of_day(h), want, "{h}:00");
    }
}
