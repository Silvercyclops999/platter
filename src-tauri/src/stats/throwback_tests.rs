use rusqlite::{Connection, params};

use super::tests::{MIN, add_track, db, local_ms, play};
use super::throwback::*;
use super::{parse_day, refresh};
use crate::db::listens::{self, ImportSource, ImportedPlay};

fn day(s: &str) -> i64 {
    parse_day(s).unwrap()
}

fn album(c: &Connection, id: &str, title: &str, year: i64) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, year, thumb) VALUES (?1, ?2, ?2, 'ar1', 'Band', ?3, ?4)",
        params![id, title, year, format!("/al/{id}")],
    )
    .unwrap();
}

/// Songs t1 and t2 on a 1994 album, t3 on a 2003 album, t4 on a 1968 album.
fn library(c: &Connection) {
    album(c, "a90", "Nineties", 1994);
    album(c, "a00", "Noughties", 2003);
    album(c, "a60", "Sixties", 1968);
    add_track(c, "t1", "One", ("a90", "Nineties"), ("ar1", "Band"), "[]");
    add_track(c, "t2", "Two", ("a90", "Nineties"), ("ar1", "Band"), "[]");
    add_track(c, "t3", "Three", ("a00", "Noughties"), ("ar1", "Band"), "[]");
    add_track(c, "t4", "Four", ("a60", "Sixties"), ("ar1", "Band"), "[]");
}

fn plays(c: &Connection, key: &str, at: &str, n: usize) {
    let at = local_ms(c, at);
    for i in 0..n {
        play(c, key, at + i as i64 * 600_000, 4 * MIN);
    }
}

fn imported(source: ImportSource, id: &str, at: i64, title: &str, key: Option<&str>) -> ImportedPlay {
    ImportedPlay {
        source,
        source_id: id.into(),
        played_at: at,
        ms_played: Some(200_000),
        title: title.into(),
        artist: Some("Band".into()),
        album: None,
        rating_key: key.map(str::to_string),
    }
}

#[test]
fn matching_days_follow_the_calendar_and_feb_29() {
    assert_eq!(matching_days(10, 17, 2026, 2021), vec![day("2021-10-17")]);
    // Feb 29 only has Feb 29s.
    assert_eq!(matching_days(2, 29, 2028, 2024), vec![day("2024-02-29")]);
    assert!(matching_days(2, 29, 2028, 2023).is_empty());
    // Feb 28 in a year without Feb 29 takes in the leap day too.
    assert_eq!(matching_days(2, 28, 2026, 2024), vec![day("2024-02-28"), day("2024-02-29")]);
    assert_eq!(matching_days(2, 28, 2026, 2023), vec![day("2023-02-28")]);
    // In a leap year Feb 28 is just Feb 28 (the 29th gets its own day).
    assert_eq!(matching_days(2, 28, 2028, 2024), vec![day("2024-02-28")]);
}

#[test]
fn on_this_day_groups_by_year_most_played_first() {
    let mut c = db();
    library(&c);
    plays(&c, "t1", "2021-10-17 20:00:00", 2);
    plays(&c, "t2", "2021-10-17 22:00:00", 1);
    plays(&c, "t3", "2023-10-17 09:00:00", 5);
    // Other days and this year's own date don't count.
    plays(&c, "t4", "2023-10-16 12:00:00", 9);
    plays(&c, "t4", "2026-10-17 08:00:00", 9);
    refresh(&mut c).unwrap();
    let o = on_this_day(&c, day("2026-10-17")).unwrap();
    assert!(o.shown);
    assert_eq!(o.date, "2026-10-17");
    let years: Vec<i64> = o.years.iter().map(|y| y.year).collect();
    assert_eq!(years, vec![2023, 2021], "the busier day first");
    let y21 = &o.years[1];
    assert_eq!((y21.date.as_str(), y21.plays), ("2021-10-17", 3));
    let order: Vec<&str> = y21.songs.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(order, vec!["One", "Two"], "most played first");
    assert_eq!(y21.track_ids, vec!["t1", "t2"]);
    assert_eq!(y21.albums[0].title, "Nineties");
    assert_eq!(y21.covers, vec!["/al/a90"]);
    assert_eq!(o.years[0].songs.len(), 1);
}

#[test]
fn on_this_day_feb_29_plays_come_up_on_feb_28_between_leap_years() {
    let mut c = db();
    library(&c);
    plays(&c, "t1", "2024-02-29 12:00:00", 1);
    plays(&c, "t2", "2024-02-28 12:00:00", 1);
    refresh(&mut c).unwrap();
    let o = on_this_day(&c, day("2026-02-28")).unwrap();
    assert_eq!(o.years.len(), 1);
    assert_eq!(o.years[0].plays, 2);
    let o = on_this_day(&c, day("2028-02-29")).unwrap();
    assert_eq!(o.years[0].track_ids, vec!["t1"]);
    let o = on_this_day(&c, day("2028-02-28")).unwrap();
    assert_eq!(o.years[0].track_ids, vec!["t2"]);
}

#[test]
fn on_this_day_merges_imports_once_and_keeps_songs_not_in_the_library() {
    let mut c = db();
    library(&c);
    let at = local_ms(&c, "2022-10-17 21:00:00");
    play(&c, "t1", at, 4 * MIN);
    let imports = vec![
        // The same play from the server's history: counted once.
        imported(ImportSource::PlexHistory, "h1", at + 60_000, "One", Some("t1")),
        // A play from another app of a song the library doesn't have.
        imported(ImportSource::File, "f1", at + 3_600_000, "Gone Song", None),
        imported(ImportSource::File, "f2", at + 4_000_000, "Gone Song", None),
    ];
    listens::insert_imported(&mut c, &imports, at).unwrap();
    refresh(&mut c).unwrap();
    let o = on_this_day(&c, day("2026-10-17")).unwrap();
    let y = &o.years[0];
    assert_eq!(y.plays, 3, "one library play, two imported");
    let gone = y.songs.iter().find(|s| s.title == "Gone Song").unwrap();
    assert_eq!((gone.plays, gone.track_id.as_deref()), (2, None), "listed, not playable");
    let one = y.songs.iter().find(|s| s.title == "One").unwrap();
    assert_eq!(one.plays, 1, "not double counted");
    assert_eq!(y.track_ids, vec!["t1"], "only library songs play");
}

#[test]
fn on_this_day_is_empty_without_history() {
    let mut c = db();
    library(&c);
    refresh(&mut c).unwrap();
    let o = on_this_day(&c, day("2026-10-17")).unwrap();
    assert!(o.shown && o.years.is_empty());
    // Plays on other dates only.
    plays(&c, "t1", "2024-05-01 12:00:00", 3);
    refresh(&mut c).unwrap();
    assert!(on_this_day(&c, day("2026-10-17")).unwrap().years.is_empty());
}

#[test]
fn time_travel_mixes_each_decade_from_your_most_played() {
    let mut c = db();
    library(&c);
    plays(&c, "t2", "2025-03-01 12:00:00", 4);
    plays(&c, "t1", "2025-03-02 12:00:00", 2);
    plays(&c, "t3", "2025-03-03 12:00:00", 1);
    refresh(&mut c).unwrap();
    let t = time_travel(&c, day("2026-10-17")).unwrap();
    assert!(t.shown);
    let decs: Vec<(i64, &str)> = t.decades.iter().map(|d| (d.year, d.id.as_str())).collect();
    assert_eq!(decs, vec![(1960, "decade:1960"), (1990, "decade:1990"), (2000, "decade:2000")], "oldest first");
    let d90 = &t.decades[1];
    assert_eq!(d90.track_ids, vec!["t2", "t1"], "most played first");
    assert_eq!((d90.plays, d90.covers.clone()), (6, vec!["/al/a90".to_string()]));
    // A decade nobody played still gets a card from its songs.
    assert_eq!(t.decades[0].track_ids, vec!["t4"]);
    assert_eq!(t.decades[0].plays, 0);
}

#[test]
fn your_year_is_the_year_with_most_plays() {
    let mut c = db();
    library(&c);
    plays(&c, "t1", "2021-06-01 12:00:00", 3);
    plays(&c, "t3", "2022-06-01 12:00:00", 5);
    plays(&c, "t2", "2022-07-01 12:00:00", 2);
    plays(&c, "t4", "2024-01-01 12:00:00", 4);
    refresh(&mut c).unwrap();
    let y = your_year(&c).unwrap().unwrap();
    assert_eq!((y.year, y.plays, y.id.as_str(), y.kind.as_str()), (2022, 7, "year:2022", "year"));
    assert_eq!(y.track_ids, vec!["t3", "t2"]);
    // Ties go to the later year.
    plays(&c, "t4", "2024-02-01 12:00:00", 3);
    refresh(&mut c).unwrap();
    assert_eq!(your_year(&c).unwrap().unwrap().year, 2024);
}

#[test]
fn time_travel_is_empty_without_music_and_your_year_needs_library_songs() {
    let mut c = db();
    refresh(&mut c).unwrap();
    let t = time_travel(&c, day("2026-10-17")).unwrap();
    assert!(t.decades.is_empty() && t.your_year.is_none());
    // Only plays of songs the library doesn't have: no year to play.
    let at = local_ms(&c, "2023-01-01 12:00:00");
    listens::insert_imported(&mut c, &[imported(ImportSource::File, "f1", at, "Gone", None)], at).unwrap();
    refresh(&mut c).unwrap();
    assert!(your_year(&c).unwrap().is_none());
}

#[test]
fn stamp_moves_when_plays_are_counted() {
    let mut c = db();
    library(&c);
    let before = stamp(&c).unwrap();
    plays(&c, "t1", "2024-05-01 12:00:00", 1);
    refresh(&mut c).unwrap();
    assert_ne!(stamp(&c).unwrap(), before);
}
