use std::time::Instant;

use rusqlite::{Connection, params};

use super::compare::{self, CompareSummary, WeightedArtist, WeightedSong};
use super::*;
use crate::db::listens::{self, ContextKind, ImportSource, ImportedPlay, PlayStart};
use crate::db::schema::migrate;

fn db() -> Connection {
    let mut c = Connection::open_in_memory().unwrap();
    migrate(&mut c).unwrap();
    c
}

/// Unix ms for a local date and time.
fn local_ms(c: &Connection, at: &str) -> i64 {
    c.query_row("SELECT CAST(strftime('%s', ?1, 'utc') AS INTEGER) * 1000", [at], |r| r.get(0)).unwrap()
}

fn day(s: &str) -> i64 {
    parse_day(s).unwrap()
}

fn range(from: &str, to: &str) -> Days {
    Days { lo: day(from), hi: day(to), all: false }
}

fn add_artist(c: &Connection, id: &str, title: &str) {
    c.execute("INSERT INTO artists (id, title, sort_title, thumb) VALUES (?1, ?2, ?2, ?3)", params![id, title, format!("/a/{id}")]).unwrap();
}

fn add_album(c: &Connection, id: &str, title: &str, artist_id: &str, artist: &str) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, thumb) VALUES (?1, ?2, ?2, ?3, ?4, ?5)",
        params![id, title, artist_id, artist, format!("/al/{id}")],
    )
    .unwrap();
}

/// A song on album `al1` by artist `ar1` ("Band") unless told otherwise.
fn add_track(c: &Connection, id: &str, title: &str, album: (&str, &str), artist: (&str, &str), genres: &str) {
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, duration_ms, genres)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '/p', 200000, ?7)",
        params![id, title, album.0, album.1, artist.0, artist.1, genres],
    )
    .unwrap();
}

fn library(c: &Connection) {
    add_artist(c, "ar1", "Band");
    add_artist(c, "ar2", "Other");
    add_album(c, "al1", "First", "ar1", "Band");
    add_album(c, "al2", "Second", "ar2", "Other");
    add_track(c, "t1", "One", ("al1", "First"), ("ar1", "Band"), r#"["Rock"]"#);
    add_track(c, "t2", "Two", ("al1", "First"), ("ar1", "Band"), r#"["Rock","Indie"]"#);
    add_track(c, "t3", "Three", ("al2", "Second"), ("ar2", "Other"), r#"["Jazz"]"#);
}

/// A finished play of `ms` listened, from an album.
fn play(c: &Connection, key: &str, at: i64, ms: u64) -> i64 {
    let id = listens::start_play(
        c,
        &PlayStart {
            rating_key: key.into(),
            part_key: None,
            started_at: at,
            duration_ms: 200_000,
            context_kind: Some(ContextKind::Album),
            context_id: Some("al1".into()),
            device: None,
            offline: false,
        },
    )
    .unwrap();
    listens::end_play(c, id, at + ms as i64, ms, ms, ms >= 200_000).unwrap();
    id
}

const MIN: u64 = 60_000;

#[test]
fn calendar_round_trips() {
    for d in [-1000, 0, 1, 365, 10_957, 20_000, 30_000] {
        let (y, m, dd) = civil_from_days(d);
        assert_eq!(days_from_civil(y, m, dd), d);
    }
    assert_eq!(format_day(0), "1970-01-01");
    assert_eq!(weekday(0), 4, "1970-01-01 was a Thursday");
    assert_eq!(day("2024-02-29") + 1, day("2024-03-01"));
    assert!(parse_day("2023-02-29").is_err());
    assert!(parse_day("nope").is_err());
    assert_eq!(parse_month("2026-02").unwrap(), (day("2026-02-01"), day("2026-02-28")));
}

#[test]
fn counts_plays_and_minutes_and_skips_never_count() {
    let mut c = db();
    library(&c);
    let at = local_ms(&c, "2026-10-01 12:00:00");
    play(&c, "t1", at, 3 * MIN);
    play(&c, "t1", at + 600_000, 2 * MIN);
    play(&c, "t2", at + 1_200_000, 4 * MIN);
    play(&c, "t3", at + 1_800_000, 10_000); // a skip
    assert!(needs_refresh(&c).unwrap());
    assert_eq!(refresh(&mut c).unwrap(), 1);
    assert!(!needs_refresh(&c).unwrap());
    let o = overview(&c, range("2026-10-01", "2026-10-01"), 10).unwrap();
    assert_eq!((o.totals.plays, o.totals.ms), (3, (9 * MIN) as i64));
    assert_eq!((o.totals.songs, o.totals.artists, o.totals.albums, o.totals.days), (2, 1, 1, 1));
    assert_eq!(o.top_songs[0].track_id.as_deref(), Some("t1"));
    assert_eq!(o.top_songs[0].plays, 2);
    assert_eq!(o.top_artists[0].artist_id.as_deref(), Some("ar1"));
    assert_eq!(o.top_artists[0].thumb.as_deref(), Some("/a/ar1"));
    assert_eq!(o.top_albums[0].album_id.as_deref(), Some("al1"));
    assert_eq!(o.top_genres[0].name, "Rock");
    assert_eq!(o.top_genres[0].plays, 3);
    assert_eq!(o.busiest_hour, Some(12));
}

#[test]
fn ranks_by_plays_then_minutes() {
    let mut c = db();
    library(&c);
    let at = local_ms(&c, "2026-09-10 09:00:00");
    for i in 0..3 {
        play(&c, "t1", at + i * 600_000, 2 * MIN);
        play(&c, "t2", at + i * 600_000 + 300_000, 3 * MIN);
    }
    play(&c, "t3", at + 4_000_000, 3 * MIN);
    refresh(&mut c).unwrap();
    let d = range("2026-09-01", "2026-09-30");
    let ids: Vec<_> = top_songs(&c, d, 10).unwrap().into_iter().map(|s| s.track_id.unwrap()).collect();
    assert_eq!(ids, ["t2", "t1", "t3"], "same plays: more minutes first");
    let all = resolve(&c, &StatsRange::default()).unwrap();
    assert!(all.all);
    let ids: Vec<_> = top_songs(&c, all, 10).unwrap().into_iter().map(|s| s.track_id.unwrap()).collect();
    assert_eq!(ids, ["t2", "t1", "t3"], "all time reads the totals and ranks the same");
    let s = item(&c, ItemKind::Song, "t1", day("2026-10-15")).unwrap();
    assert_eq!((s.plays, s.rank), (3, Some(2)));
}

#[test]
fn year_boundaries_follow_local_time() {
    let mut c = db();
    library(&c);
    play(&c, "t1", local_ms(&c, "2025-12-31 23:30:00"), 3 * MIN);
    play(&c, "t2", local_ms(&c, "2026-01-01 00:30:00"), 3 * MIN);
    refresh(&mut c).unwrap();
    let y25 = overview(&c, range("2025-01-01", "2025-12-31"), 10).unwrap();
    let y26 = overview(&c, range("2026-01-01", "2026-12-31"), 10).unwrap();
    assert_eq!(y25.top_songs.iter().map(|s| s.track_id.clone().unwrap()).collect::<Vec<_>>(), ["t1"]);
    assert_eq!(y26.top_songs.iter().map(|s| s.track_id.clone().unwrap()).collect::<Vec<_>>(), ["t2"]);
    assert_eq!(y25.busiest_hour, Some(23));
    assert_eq!(y26.busiest_hour, Some(0));
}

#[test]
fn refresh_recounts_only_changed_days() {
    let mut c = db();
    library(&c);
    let d1 = local_ms(&c, "2026-10-01 10:00:00");
    let d2 = local_ms(&c, "2026-10-05 10:00:00");
    play(&c, "t1", d1, 3 * MIN);
    refresh(&mut c).unwrap();

    // A new play marks only its own day.
    let id = listens::start_play(
        &c,
        &PlayStart {
            rating_key: "t2".into(),
            part_key: None,
            started_at: d2,
            duration_ms: 200_000,
            context_kind: None,
            context_id: None,
            device: None,
            offline: false,
        },
    )
    .unwrap();
    let dirty: Vec<i64> = c.prepare("SELECT day FROM stats_dirty").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(dirty, [day("2026-10-05")]);
    assert_eq!(refresh(&mut c).unwrap(), 1);
    // Playing on adds minutes.
    listens::progress(&c, id, 2 * MIN).unwrap();
    assert!(needs_refresh(&c).unwrap());
    refresh(&mut c).unwrap();
    let o = overview(&c, range("2026-10-01", "2026-10-31"), 10).unwrap();
    assert_eq!((o.totals.plays, o.totals.ms), (2, (5 * MIN) as i64));
    // Ending as a skip takes it out again.
    listens::end_play(&c, id, d2 + 10_000, 10_000, 10_000, false).unwrap();
    refresh(&mut c).unwrap();
    let o = overview(&c, range("2026-10-01", "2026-10-31"), 10).unwrap();
    assert_eq!(o.totals.plays, 1);
    let s = item(&c, ItemKind::Song, "t2", day("2026-10-15")).unwrap();
    assert_eq!(s.plays, 0, "the song's all-time total follows");
    // A removed play goes too.
    c.execute("DELETE FROM listen_log", []).unwrap();
    refresh(&mut c).unwrap();
    let o = overview(&c, range("2026-10-01", "2026-10-31"), 10).unwrap();
    assert_eq!(o.totals.plays, 0);
    assert!(o.top_songs.is_empty());
    assert_eq!(item(&c, ItemKind::Artist, "ar1", day("2026-10-15")).unwrap().plays, 0);
}

#[test]
fn imported_plays_count_once_and_follow_their_match() {
    let mut c = db();
    library(&c);
    let at = local_ms(&c, "2026-08-01 18:00:00");
    play(&c, "t1", at, 3 * MIN);
    let imports = [
        // The same play as reported by the server: left out.
        ImportedPlay {
            source: ImportSource::PlexHistory,
            source_id: "h1".into(),
            played_at: at + 120_000,
            ms_played: None,
            title: "One".into(),
            artist: Some("Band".into()),
            album: Some("First".into()),
            rating_key: Some("t1".into()),
        },
        // An older play nobody matched yet.
        ImportedPlay {
            source: ImportSource::File,
            source_id: "f1".into(),
            played_at: at - 86_400_000,
            ms_played: Some(150_000),
            title: "Gone Song".into(),
            artist: Some("Old Band".into()),
            album: None,
            rating_key: None,
        },
    ];
    listens::insert_imported(&mut c, &imports, at).unwrap();
    refresh(&mut c).unwrap();
    let all = resolve(&c, &StatsRange::default()).unwrap();
    let o = overview(&c, all, 10).unwrap();
    assert_eq!(o.totals.plays, 2, "the server's copy of a local play counts once");
    let gone = o.top_songs.iter().find(|s| s.title == "Gone Song").unwrap();
    assert!(gone.track_id.is_none() && gone.key.starts_with("n:"));
    assert_eq!(gone.ms, 150_000);
    let first = item(&c, ItemKind::Song, "t1", day("2026-10-15")).unwrap().first.unwrap();
    assert_eq!(first.from.unwrap().title.as_deref(), Some("First"), "first listen names what it was played from");

    // Matching it later moves it onto the library song.
    listens::set_imported_match(&c, ImportSource::File, "f1", Some("t3"), at).unwrap();
    refresh(&mut c).unwrap();
    let o = overview(&c, all, 10).unwrap();
    assert!(o.top_songs.iter().all(|s| !s.key.starts_with("n:")), "the unmatched entry is gone");
    let s3 = item(&c, ItemKind::Song, "t3", day("2026-10-15")).unwrap();
    assert_eq!(s3.plays, 1);
    assert_eq!(s3.first.unwrap().from.unwrap().kind, "imported");
}

#[test]
fn streaks_series_and_busiest_weekday() {
    let mut c = db();
    library(&c);
    for d in ["2026-03-01", "2026-03-02", "2026-03-03", "2026-03-06", "2026-03-07"] {
        play(&c, "t1", local_ms(&c, &format!("{d} 20:00:00")), 3 * MIN);
    }
    play(&c, "t2", local_ms(&c, "2026-03-07 21:00:00"), 3 * MIN);
    refresh(&mut c).unwrap();
    let o = overview(&c, range("2026-03-01", "2026-03-31"), 10).unwrap();
    let st = o.streak.unwrap();
    assert_eq!((st.days, st.from.as_str(), st.to.as_str()), (3, "2026-03-01", "2026-03-03"));
    assert_eq!(o.bucket, Bucket::Day);
    assert_eq!(o.series.len(), 31);
    assert_eq!(o.series[6].plays, 2);
    assert_eq!(o.busiest_weekday, Some(weekday(day("2026-03-07"))));
    let y = overview(&c, range("2026-01-01", "2026-12-31"), 10).unwrap();
    assert_eq!(y.bucket, Bucket::Month);
    assert_eq!(y.series.len(), 12);
    assert_eq!(y.series[2].key, "2026-03");
    assert_eq!(y.series[2].plays, 6);
    assert_eq!(longest_streak(&[]), None);
    let sp = span(&c).unwrap();
    assert_eq!(sp.months, ["2026-03"]);
}

#[test]
fn item_stats_for_artists_and_albums() {
    let mut c = db();
    library(&c);
    play(&c, "t2", local_ms(&c, "2026-05-02 10:00:00"), 3 * MIN);
    play(&c, "t1", local_ms(&c, "2026-06-02 10:00:00"), 3 * MIN);
    play(&c, "t3", local_ms(&c, "2026-06-03 10:00:00"), 3 * MIN);
    refresh(&mut c).unwrap();
    let today = day("2026-10-15");
    let a = item(&c, ItemKind::Artist, "ar1", today).unwrap();
    assert_eq!((a.plays, a.rank), (2, Some(1)));
    let first = a.first.unwrap();
    assert_eq!((first.song.as_str(), first.track_id.as_deref()), ("Two", Some("t2")));
    assert_eq!(a.last_at, Some(local_ms(&c, "2026-06-02 10:00:00")));
    assert_eq!(a.series.len(), ITEM_SERIES_MONTHS as usize);
    assert_eq!(a.series.last().unwrap().key, "2026-10");
    let may = a.series.iter().find(|p| p.key == "2026-05").unwrap();
    assert_eq!(may.plays, 1);
    let al = item(&c, ItemKind::Album, "al2", today).unwrap();
    assert_eq!((al.plays, al.rank), (1, Some(2)));
    let none = item(&c, ItemKind::Album, "nope", today).unwrap();
    assert_eq!((none.plays, none.rank), (0, None));
}

#[test]
fn compilations_count_for_the_songs_own_artist() {
    let mut c = db();
    add_artist(&c, "va", "Various Artists");
    add_album(&c, "comp", "Hits", "va", "Various Artists");
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_artist, part_key, duration_ms)
         VALUES ('c1', 'Hit', 'comp', 'Hits', 'va', 'Various Artists', 'Singer', '/p', 200000)",
        [],
    )
    .unwrap();
    play(&c, "c1", local_ms(&c, "2026-04-01 10:00:00"), 3 * MIN);
    refresh(&mut c).unwrap();
    let o = overview(&c, range("2026-04-01", "2026-04-30"), 10).unwrap();
    assert_eq!(o.top_artists[0].name, "Singer");
    assert_eq!(o.top_albums[0].title, "Hits");
    assert_eq!(o.top_albums[0].album_id.as_deref(), Some("comp"));
}

#[test]
fn monthly_recap_and_play_these() {
    let mut c = db();
    library(&c);
    play(&c, "t3", local_ms(&c, "2026-08-20 10:00:00"), 5 * MIN);
    for i in 0..3 {
        play(&c, "t1", local_ms(&c, &format!("2026-09-0{} 10:00:00", i + 1)), 3 * MIN);
    }
    play(&c, "t2", local_ms(&c, "2026-09-02 11:00:00"), 3 * MIN);
    refresh(&mut c).unwrap();
    let r = month_recap(&c, "2026-09").unwrap();
    assert_eq!(r.month, "2026-09");
    assert_eq!((r.totals.plays, r.totals.ms, r.prev_ms), (4, (12 * MIN) as i64, (5 * MIN) as i64));
    assert_eq!(r.top_songs[0].track_id.as_deref(), Some("t1"));
    assert_eq!(r.top_artists[0].name, "Band");
    assert_eq!(r.top_album.as_ref().unwrap().title, "First");
    assert_eq!(r.top_genre.as_deref(), Some("Rock"));
    assert_eq!(r.daily.len(), 30);
    assert_eq!(r.biggest_day.unwrap().key, "2026-09-02");

    let d = range("2026-08-01", "2026-09-30");
    assert_eq!(play_ids(&c, d, TopKind::Songs, 10).unwrap(), ["t1", "t3", "t2"]);
    assert_eq!(play_ids(&c, d, TopKind::Songs, 1).unwrap(), ["t1"]);
    let by_artist = play_ids(&c, d, TopKind::Artists, 10).unwrap();
    assert_eq!(by_artist.len(), 3);
    let jazz = play_ids(&c, range("2026-08-01", "2026-08-31"), TopKind::Genres, 10).unwrap();
    assert_eq!(jazz, ["t3"]);
}

#[test]
fn unreleased_songs_count_for_you_but_stay_out_of_summaries() {
    let mut c = db();
    library(&c);
    c.execute(
        "INSERT INTO song_index (track_id, artist_fold, title_fold, artist_id, unreleased) VALUES ('t2', 'band', 'two', 'ar1', 1)",
        [],
    )
    .unwrap();
    let today = day("2026-10-15");
    for i in 0..4 {
        play(&c, "t2", local_ms(&c, &format!("2026-10-0{} 10:00:00", i + 1)), 3 * MIN);
    }
    play(&c, "t1", local_ms(&c, "2026-10-06 10:00:00"), 3 * MIN);
    play(&c, "t3", local_ms(&c, "2026-10-07 10:00:00"), 3 * MIN);
    refresh(&mut c).unwrap();
    let o = overview(&c, range("2026-10-01", "2026-10-31"), 10).unwrap();
    assert!(o.top_songs[0].unreleased && o.top_songs[0].track_id.as_deref() == Some("t2"), "counts and is marked");
    assert!(o.top_albums[0].unreleased);

    let s = compare::summary(&c, today, Some("Me".into()), false, false).unwrap();
    assert!(s.songs.iter().all(|x| x.title != "Two"), "unreleased stays out");
    let w: f64 = s.artists.iter().map(|a| a.weight).sum();
    assert!((w - 1.0).abs() < 0.01);
    assert_eq!(s.artists.len(), 2);
    let both = compare::summary(&c, today, None, true, true).unwrap();
    assert!(both.songs.iter().any(|x| x.title == "Two"), "both opted in");
    let json = serde_json::to_string(&s).unwrap();
    assert!(!json.contains("playedAt") && !json.contains("plays"), "no history or counts leave");
}

#[test]
fn compare_finds_overlap_common_artists_and_new_songs() {
    let mut c = db();
    library(&c);
    c.execute(
        "INSERT INTO song_index (track_id, artist_fold, title_fold, artist_id, unreleased) VALUES ('t3', 'other', 'three', 'ar2', 0)",
        [],
    )
    .unwrap();
    play(&c, "t1", local_ms(&c, "2026-10-01 10:00:00"), 3 * MIN);
    refresh(&mut c).unwrap();
    let mine = compare::summary(&c, day("2026-10-15"), None, false, false).unwrap();
    let theirs = CompareSummary {
        version: compare::SUMMARY_VERSION,
        name: Some("Sam".into()),
        artists: vec![WeightedArtist { name: "The Band".into(), weight: 0.5 }, WeightedArtist { name: "Other".into(), weight: 0.5 }],
        songs: vec![
            WeightedSong { artist: "Band".into(), title: "One".into(), weight: 0.4 },
            WeightedSong { artist: "Other".into(), title: "Three".into(), weight: 0.3 },
            WeightedSong { artist: "Nobody".into(), title: "Elsewhere".into(), weight: 0.3 },
        ],
        unreleased_ok: false,
    };
    let r = compare::compare(&c, &mine, &theirs).unwrap();
    assert_eq!(r.overlap, 50);
    assert_eq!(r.common_artists.len(), 1);
    assert_eq!(r.common_artists[0].artist_id.as_deref(), Some("ar1"));
    let titles: Vec<_> = r.their_songs_new.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Three", "Elsewhere"], "songs you've played are left out");
    assert_eq!(r.their_songs_new[0].track.as_ref().map(|t| t.id.as_str()), Some("t3"));
    assert!(r.their_songs_new[1].track.is_none());
    let bad = CompareSummary { version: 99, ..theirs };
    assert!(compare::compare(&c, &mine, &bad).is_err());
    assert_eq!(compare::overlap(&[], &[]), 0);
}

#[test]
fn friends_summaries_are_kept_by_name() {
    let c = db();
    assert!(compare::friends(&c).unwrap().is_empty());
    let s = |name: &str, w: f64| CompareSummary {
        version: compare::SUMMARY_VERSION,
        name: Some(name.into()),
        artists: vec![WeightedArtist { name: "Band".into(), weight: w }],
        songs: vec![],
        unreleased_ok: false,
    };
    compare::friend_add(&c, s("Sam", 1.0)).unwrap();
    compare::friend_add(&c, s("Alex", 1.0)).unwrap();
    let all = compare::friend_add(&c, s("Sam", 0.5)).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!((all[0].name.as_deref(), all[0].artists[0].weight), (Some("Sam"), 0.5));
    assert!(compare::friend_add(&c, CompareSummary { version: 7, ..s("Bad", 1.0) }).is_err());
    assert_eq!(compare::friend_remove(&c, "Sam").unwrap().len(), 1);
}

#[test]
fn moments_first_song_replays_discovery_and_comeback() {
    let mut c = db();
    library(&c);
    // Before the range: Band, and "One" last heard in May.
    play(&c, "t1", local_ms(&c, "2026-05-01 10:00:00"), 3 * MIN);
    // In September: the first song is Two at 08:00, One comes back after
    // 123 days and plays four times that day, Other is new.
    play(&c, "t2", local_ms(&c, "2026-09-01 08:00:00"), 3 * MIN);
    play(&c, "t3", local_ms(&c, "2026-09-01 09:00:00"), 3 * MIN);
    for i in 0..4 {
        play(&c, "t1", local_ms(&c, &format!("2026-09-02 1{i}:00:00")), 3 * MIN);
    }
    play(&c, "t3", local_ms(&c, "2026-09-05 10:00:00"), 3 * MIN);
    refresh(&mut c).unwrap();

    let m = moments(&c, range("2026-09-01", "2026-09-30")).unwrap();
    let first = m.first_song.unwrap();
    assert_eq!((first.song.track_id.as_deref(), first.day.as_str()), (Some("t2"), "2026-09-01"));
    assert_eq!(first.at, Some(local_ms(&c, "2026-09-01 08:00:00")));
    let rep = m.replayed.unwrap();
    assert_eq!((rep.song.track_id.as_deref(), rep.day.as_str(), rep.song.plays), (Some("t1"), "2026-09-02", 4));
    let disc = m.discovery.unwrap();
    assert_eq!((disc.artist.name.as_str(), disc.artist.plays, disc.day.as_str()), ("Other", 2, "2026-09-01"));
    assert_eq!(m.new_artists, Some(1), "Band was heard before the range");
    let back = m.comeback.unwrap();
    assert_eq!((back.song.song.track_id.as_deref(), back.song.day.as_str()), (Some("t1"), "2026-09-02"));
    assert_eq!(back.gap_days, day("2026-09-02") - day("2026-05-01"));

    // A range with nothing in it has no moments; all time has no new artists.
    let none = moments(&c, range("2026-01-01", "2026-01-31")).unwrap();
    assert!(none.first_song.is_none() && none.replayed.is_none() && none.discovery.is_none() && none.comeback.is_none());
    assert_eq!(none.new_artists, Some(0));
    let all = moments(&c, resolve(&c, &StatsRange::default()).unwrap()).unwrap();
    assert_eq!(all.new_artists, None);
    assert!(all.discovery.is_none());
    assert_eq!(all.first_song.unwrap().day, "2026-05-01");
    // Two plays of a song in a day isn't a replay moment; a short break isn't a comeback.
    let short = moments(&c, range("2026-09-05", "2026-09-05")).unwrap();
    assert!(short.replayed.is_none());
    assert!(short.comeback.is_none(), "four days since Other's last play");
}

#[test]
fn calendar_days_genre_covers_and_recap_covers() {
    let mut c = db();
    library(&c);
    play(&c, "t1", local_ms(&c, "2026-09-01 10:00:00"), 3 * MIN);
    play(&c, "t1", local_ms(&c, "2026-09-01 11:00:00"), 2 * MIN);
    play(&c, "t3", local_ms(&c, "2026-09-03 10:00:00"), 4 * MIN);
    refresh(&mut c).unwrap();
    let ds = days(&c, day("2026-08-30"), day("2026-09-10")).unwrap();
    assert_eq!(ds.iter().map(|p| (p.key.as_str(), p.plays, p.ms)).collect::<Vec<_>>(), [
        ("2026-09-01", 2, (5 * MIN) as i64),
        ("2026-09-03", 1, (4 * MIN) as i64)
    ]);
    let o = overview(&c, range("2026-09-01", "2026-09-30"), 10).unwrap();
    let rock = o.top_genres.iter().find(|g| g.name == "Rock").unwrap();
    assert_eq!(rock.thumb.as_deref(), Some("/al/al1"));
    let jazz = o.top_genres.iter().find(|g| g.name == "Jazz").unwrap();
    assert_eq!(jazz.thumb.as_deref(), Some("/al/al2"));
    let r = month_recap(&c, "2026-09").unwrap();
    assert_eq!(r.top_albums.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
    assert_eq!(r.top_album.unwrap().title, "First");
}

/// 100k plays over three years, plus 30k imported from before them (and
/// imported copies of plays already counted): the first count and every
/// range query stay fast.
#[test]
fn fast_at_100k_plays() {
    let mut c = db();
    let songs = 4_000;
    {
        let tx = c.transaction().unwrap();
        for a in 0..400 {
            tx.execute("INSERT INTO artists (id, title, sort_title) VALUES (?1, ?2, ?2)", params![format!("a{a}"), format!("Artist {a}")]).unwrap();
            for b in 0..2 {
                tx.execute(
                    "INSERT INTO albums (id, title, sort_title, artist_id, artist_title) VALUES (?1, ?2, ?2, ?3, ?4)",
                    params![format!("al{a}-{b}"), format!("Album {a}-{b}"), format!("a{a}"), format!("Artist {a}")],
                )
                .unwrap();
            }
        }
        for s in 0..songs {
            let a = s % 400;
            let b = (s / 400) % 2;
            tx.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, duration_ms, genres)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, '/p', 200000, ?7)",
                params![
                    format!("s{s}"),
                    format!("Song {s}"),
                    format!("al{a}-{b}"),
                    format!("Album {a}-{b}"),
                    format!("a{a}"),
                    format!("Artist {a}"),
                    format!(r#"["Genre {}"]"#, s % 12)
                ],
            )
            .unwrap();
        }
        let start = local_ms(&tx, "2023-10-15 00:00:00");
        let mut st = tx
            .prepare(
                "INSERT INTO listen_log (rating_key, started_at, ended_at, duration_ms, ms_listened, completed,
                   snap_title, snap_artist, snap_album, snap_album_artist)
                 SELECT id, ?2, ?2 + 180000, duration_ms, 180000, 1, title, artist_title, album_title, artist_title
                 FROM tracks WHERE id = ?1",
            )
            .unwrap();
        let mut x: u64 = 7;
        let mut pick = || {
            // A skewed pick, so some songs are favourites.
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let r = (x >> 33) % (songs as u64);
            (r * r) / songs as u64
        };
        let mut picked = Vec::with_capacity(100_000);
        for i in 0..100_000i64 {
            let s = pick();
            picked.push(s);
            st.execute(params![format!("s{s}"), start + i * 945_000]).unwrap();
        }
        drop(st);
        // Imported plays: 30k from the server's history before the listen
        // log starts, each third one scrobbled to Last.fm too, and every
        // tenth local play as the server and Last.fm saw it. Only the 30k
        // count.
        let mut imp = tx
            .prepare(
                "INSERT INTO imported_plays (source, source_id, played_at, title, artist, album, rating_key)
                 SELECT ?1, ?2, ?3, title, artist_title, album_title, id FROM tracks WHERE id = ?4",
            )
            .unwrap();
        for i in 0..30_000i64 {
            let s = format!("s{}", pick());
            let at = start - (i + 1) * 945_000;
            imp.execute(params!["plex_history", format!("h{i}"), at + 120_000, s]).unwrap();
            if i % 3 == 0 {
                imp.execute(params!["lastfm", format!("l{i}"), at, s]).unwrap();
            }
        }
        for (i, s) in picked.iter().enumerate().step_by(10) {
            let at = start + i as i64 * 945_000;
            imp.execute(params!["plex_history", format!("own{i}"), at + 170_000, format!("s{s}")]).unwrap();
            imp.execute(params!["lastfm", format!("own{i}"), at, format!("s{s}")]).unwrap();
        }
        drop(imp);
        tx.commit().unwrap();
    }
    let t = Instant::now();
    refresh(&mut c).unwrap();
    let first = t.elapsed();
    eprintln!("first count of 100k plays: {first:?}");

    // Release-style limits, loosened for unoptimised debug builds.
    let budget_ms: u128 = if cfg!(debug_assertions) { 400 } else { 100 };
    // Moments over all time scans every play several times; unoptimised builds on shared CI
    // machines land around 400-450 ms, release builds far below the 100 ms limit.
    let moments_ms: u128 = if cfg!(debug_assertions) { 800 } else { 100 };
    let ranges = [
        StatsRange { from: Some("2026-09-17".into()), to: Some("2026-10-14".into()) },
        StatsRange { from: Some("2026-04-15".into()), to: Some("2026-10-14".into()) },
        StatsRange { from: Some("2026-01-01".into()), to: Some("2026-12-31".into()) },
        StatsRange { from: Some("2024-01-01".into()), to: Some("2024-12-31".into()) },
        StatsRange::default(),
    ];
    let all = resolve(&c, &StatsRange::default()).unwrap();
    assert_eq!(overview(&c, all, 10).unwrap().totals.plays, 130_000, "an imported copy of a play never counts twice");
    for r in &ranges {
        let d = resolve(&c, r).unwrap();
        let took = fastest(|| assert!(overview(&c, d, 50).unwrap().totals.plays > 0));
        eprintln!("overview {r:?}: {took:?}");
        assert!(took.as_millis() < budget_ms, "{r:?} took {took:?}");
        // Best of five, like the overview: one run on a busy CI machine can stall.
        let took = fastest(|| assert!(moments(&c, d).unwrap().first_song.is_some()));
        eprintln!("moments {r:?}: {took:?}");
        assert!(took.as_millis() < moments_ms, "moments {r:?} took {took:?}");
    }
    let t = Instant::now();
    let cal = days(&c, day("2025-10-12"), day("2026-10-17")).unwrap();
    assert!(!cal.is_empty());
    assert!(t.elapsed().as_millis() < budget_ms);
    let today = day("2026-10-15");
    let took = fastest(|| {
        let a = item(&c, ItemKind::Artist, "a0", today).unwrap();
        let s = item(&c, ItemKind::Song, "s0", today).unwrap();
        let al = item(&c, ItemKind::Album, "al0-0", today).unwrap();
        let r = month_recap(&c, "2026-09").unwrap();
        assert!(a.plays > 0 && s.plays > 0 && al.plays > 0 && r.totals.plays > 0);
    });
    eprintln!("item stats x3 + recap: {took:?}");
    assert!(took.as_millis() < budget_ms);

    // A new play recounts one day, not the history.
    let mut n = 0;
    let took = fastest(|| {
        n += 1;
        play(&c, "s5", local_ms(&c, &format!("2026-10-14 12:00:{n:02}")), 3 * MIN);
        let t = Instant::now();
        assert_eq!(refresh(&mut c).unwrap(), 1);
        t.elapsed()
    });
    eprintln!("one-day recount: {took:?}");
    assert!(took.as_millis() < budget_ms);
}

/// The fastest of a few runs of `f`. Another process taking the CPU slows
/// some runs, not all of them; slower code slows every run. `f` may time
/// itself (returning the part that counts) or be timed whole.
fn fastest<R: Timed>(mut f: impl FnMut() -> R) -> std::time::Duration {
    (0..5)
        .map(|_| {
            let t = Instant::now();
            let r = f();
            r.took().unwrap_or_else(|| t.elapsed())
        })
        .min()
        .unwrap()
}

trait Timed {
    fn took(&self) -> Option<std::time::Duration>;
}
impl Timed for () {
    fn took(&self) -> Option<std::time::Duration> {
        None
    }
}
impl Timed for std::time::Duration {
    fn took(&self) -> Option<std::time::Duration> {
        Some(*self)
    }
}
