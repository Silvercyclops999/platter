use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, params};

use super::detect::{LOOKBACK_MS, finished_sessions, first_play};
use super::export::{self, LocalTime};
use super::*;
use crate::db::DbHandle;
use crate::db::listens::{self, ContextKind, ImportSource, ImportedPlay, PlayStart};
use crate::sync::SyncStore;
use crate::sync::engine::Groups;
use crate::sync::store::DIARY;

const MIN: i64 = 60_000;
const HOUR: i64 = 3_600_000;
/// "Now" for these tests: 2026-10-17 12:00 UTC.
const NOW: i64 = 1_792_238_400_000;

/// An album `id` with `n` songs `{id}-1` .. `{id}-n`.
fn album(c: &Connection, id: &str, n: u32) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, thumb, year, track_count, added_at)
         VALUES (?1, ?2, ?2, 'ar1', 'The Band', ?3, 1997, ?4, 0)",
        params![id, format!("Record {id}"), format!("/al/{id}"), n],
    )
    .unwrap();
    for i in 1..=n {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, duration_ms, track_number)
             VALUES (?1, ?2, ?3, ?3, 'ar1', 'The Band', '/p', 240000, ?4)",
            params![format!("{id}-{i}"), format!("Song {i}"), id, i],
        )
        .unwrap();
    }
}

/// Song `n` of `album`, started from the album at `at`. `how`: "done",
/// "half" (stopped at half way), "skip".
fn play(c: &Connection, album: &str, n: u32, at: i64, how: &str) -> i64 {
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
        "half" => drop(listens::end_play(c, id, at + 2 * MIN, 120_000, 120_000, false).unwrap()),
        _ => drop(listens::end_play(c, id, at + 5_000, 5_000, 5_000, false).unwrap()),
    }
    id
}

/// Songs `from..=to` played through one after another from `at`.
fn through(c: &Connection, album: &str, from: u32, to: u32, at: i64) -> i64 {
    let mut last = 0;
    for i in from..=to {
        last = play(c, album, i, at + (i - from) as i64 * 4 * MIN, "done");
    }
    last
}

fn mem() -> Connection {
    let mut c = Connection::open_in_memory().unwrap();
    crate::db::schema::migrate(&mut c).unwrap();
    c
}

// ------------------------------------------------------------------ detection

#[test]
fn an_album_heard_through_is_one_sitting() {
    let c = mem();
    album(&c, "al", 10);
    let last = through(&c, "al", 1, 10, NOW - 2 * HOUR);
    let got = finished_sessions(&c, "al", 0).unwrap();
    assert_eq!(got.len(), 1);
    let f = &got[0];
    assert_eq!((f.songs_played, f.songs_total, f.play_id), (10, 10, last));
    assert_eq!(f.started_at, NOW - 2 * HOUR);
    assert_eq!(f.ms_listened, 10 * 240_000);
    assert_eq!(f.key(), format!("auto:al:s{}", NOW - 2 * HOUR), "keyed by when the sitting started");
    assert_eq!(f.legacy_key(), format!("auto:al:{last}"), "the key older builds used");
}

#[test]
fn a_sitting_cut_by_the_lookback_is_read_from_its_start() {
    let c = mem();
    album(&c, "al", 10);
    let since = NOW - LOOKBACK_MS;
    // Started ten minutes before the window: songs 1 to 3 fall outside it.
    let begun = since - 10 * MIN;
    through(&c, "al", 1, 10, begun);
    // An older sitting, well apart, stays out of it.
    through(&c, "al", 1, 4, begun - 6 * HOUR);
    let got = finished_sessions(&c, "al", since).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].started_at, got[0].songs_played, got[0].ms_listened), (begun, 10, 10 * 240_000));
}

#[test]
fn two_devices_sittings_get_different_keys() {
    // Each computer numbers its own listen log, so the same row id can come
    // up on both; the sitting's start time tells them apart.
    let (a, b) = (mem(), mem());
    album(&a, "al", 3);
    album(&b, "al", 3);
    let (la, lb) = (through(&a, "al", 1, 3, NOW - 3 * HOUR), through(&b, "al", 1, 3, NOW - HOUR));
    assert_eq!(la, lb, "same listen log row on both");
    let (ka, kb) = (finished_sessions(&a, "al", 0).unwrap()[0].key(), finished_sessions(&b, "al", 0).unwrap()[0].key());
    assert_ne!(ka, kb);
}

#[test]
fn most_songs_with_the_last_one_is_enough() {
    let c = mem();
    album(&c, "al", 10);
    // Songs 3 and 6 skipped: 8 of 10 heard, ending on the last.
    for i in 1..=10 {
        play(&c, "al", i, NOW - 3 * HOUR + i as i64 * 4 * MIN, if i == 3 || i == 6 { "skip" } else { "done" });
    }
    let got = finished_sessions(&c, "al", 0).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].songs_played, 8);
}

#[test]
fn a_partial_play_makes_no_entry() {
    let c = mem();
    album(&c, "al", 10);
    // Most of it, but never the last song.
    through(&c, "al", 1, 9, NOW - 3 * HOUR);
    assert!(finished_sessions(&c, "al", 0).unwrap().is_empty());
    // The last song, but too few before it.
    let c = mem();
    album(&c, "al", 10);
    through(&c, "al", 6, 10, NOW - 3 * HOUR);
    assert!(finished_sessions(&c, "al", 0).unwrap().is_empty());
    // Every song started but skipped.
    let c = mem();
    album(&c, "al", 4);
    for i in 1..=4 {
        play(&c, "al", i, NOW - HOUR + i as i64 * MIN, "skip");
    }
    assert!(finished_sessions(&c, "al", 0).unwrap().is_empty());
}

#[test]
fn half_a_song_counts_as_heard() {
    let c = mem();
    album(&c, "al", 3);
    play(&c, "al", 1, NOW - HOUR, "half");
    play(&c, "al", 2, NOW - HOUR + 4 * MIN, "done");
    play(&c, "al", 3, NOW - HOUR + 8 * MIN, "half");
    assert_eq!(finished_sessions(&c, "al", 0).unwrap().len(), 1);
}

#[test]
fn two_sittings_make_two_entries() {
    let c = mem();
    album(&c, "al", 5);
    through(&c, "al", 1, 5, NOW - 30 * HOUR);
    through(&c, "al", 1, 5, NOW - 3 * HOUR);
    let got = finished_sessions(&c, "al", 0).unwrap();
    assert_eq!(got.len(), 2);
    assert_ne!(got[0].key(), got[1].key());
    // Straight through twice in a row is two as well.
    let c = mem();
    album(&c, "al", 5);
    through(&c, "al", 1, 5, NOW - 4 * HOUR);
    through(&c, "al", 1, 5, NOW - 4 * HOUR + 20 * MIN);
    assert_eq!(finished_sessions(&c, "al", 0).unwrap().len(), 2);
}

#[test]
fn a_long_pause_splits_a_listen_in_two() {
    let c = mem();
    album(&c, "al", 6);
    through(&c, "al", 1, 3, NOW - 10 * HOUR);
    // Three hours later the rest: neither half heard it through.
    through(&c, "al", 4, 6, NOW - 10 * HOUR + 3 * HOUR + 12 * MIN);
    assert!(finished_sessions(&c, "al", 0).unwrap().is_empty());
}

#[test]
fn singles_and_plays_from_elsewhere_make_no_entry() {
    let c = mem();
    album(&c, "single", 2);
    through(&c, "single", 1, 2, NOW - HOUR);
    assert!(finished_sessions(&c, "single", 0).unwrap().is_empty(), "too short to be an album");
    // The album's songs played from a playlist don't count.
    album(&c, "al", 3);
    for i in 1..=3 {
        let id = listens::start_play(
            &c,
            &PlayStart {
                rating_key: format!("al-{i}"),
                part_key: None,
                started_at: NOW - HOUR + i as i64 * 4 * MIN,
                duration_ms: 240_000,
                context_kind: Some(ContextKind::Playlist),
                context_id: Some("pl".into()),
                device: None,
                offline: false,
            },
        )
        .unwrap();
        listens::end_play(&c, id, NOW, 240_000, 240_000, true).unwrap();
    }
    assert!(finished_sessions(&c, "al", 0).unwrap().is_empty());
}

fn imported(at: i64, key: &str, source: ImportSource) -> ImportedPlay {
    ImportedPlay {
        source,
        source_id: format!("{key}@{at}"),
        played_at: at,
        ms_played: Some(240_000),
        title: key.into(),
        artist: Some("The Band".into()),
        album: Some("Record al".into()),
        rating_key: Some(key.into()),
    }
}

#[test]
fn imported_plays_never_make_entries() {
    let mut c = mem();
    album(&c, "al", 4);
    let plays: Vec<_> = (1..=4).map(|i| imported(NOW - HOUR + i * 4 * MIN, &format!("al-{i}"), ImportSource::PlexHistory)).collect();
    listens::insert_imported(&mut c, &plays, NOW).unwrap();
    assert!(finished_sessions(&c, "al", 0).unwrap().is_empty());
}

// ------------------------------------------------------------------ first heard

#[test]
fn first_heard_is_the_earliest_play_imported_or_here() {
    let mut c = mem();
    album(&c, "al", 4);
    album(&c, "other", 3);
    assert_eq!(first_play(&c, "al").unwrap(), None, "never played");
    play(&c, "al", 2, NOW - HOUR, "done");
    assert_eq!(first_play(&c, "al").unwrap(), Some(NOW - HOUR));
    let years_ago = NOW - 5 * 365 * 24 * HOUR;
    listens::insert_imported(&mut c, &[imported(years_ago, "al-3", ImportSource::PlexHistory)], NOW).unwrap();
    // Another album's older play doesn't count.
    listens::insert_imported(&mut c, &[imported(years_ago - HOUR, "other-1", ImportSource::PlexHistory)], NOW).unwrap();
    assert_eq!(first_play(&c, "al").unwrap(), Some(years_ago), "imported history counts");
    // A source left out of plays is left out here too.
    listens::set_excluded_sources(&mut c, &[ImportSource::PlexHistory]).unwrap();
    assert_eq!(first_play(&c, "al").unwrap(), Some(NOW - HOUR));
}

// ------------------------------------------------------------------ the rest

#[test]
fn ratings_are_half_steps_and_notes_are_plain_and_short() {
    assert_eq!(check_rating(None).unwrap(), None);
    for ok in [0.5, 1.0, 3.5, 5.0] {
        assert_eq!(check_rating(Some(ok)).unwrap(), Some(ok));
    }
    for bad in [0.0, 0.3, 5.5, 4.25, f32::NAN] {
        assert!(check_rating(Some(bad)).is_err(), "{bad}");
    }
    assert_eq!(clean_note("  Loved side two.\r\n\n\n\nThe bass!\u{7}  ").unwrap(), "Loved side two.\n\nThe bass!");
    assert!(clean_note(&"a".repeat(NOTE_MAX)).is_ok());
    assert!(clean_note(&"é".repeat(NOTE_MAX + 1)).is_err(), "counted in characters");
}

#[test]
fn the_output_name_says_headphones_or_speakers_when_it_can() {
    assert_eq!(listened_on("Headphones (WH-1000XM5)"), Some(ListenedOn::Headphones));
    assert_eq!(listened_on("AirPods Pro"), Some(ListenedOn::Headphones));
    assert_eq!(listened_on("Speakers (Realtek(R) Audio)"), Some(ListenedOn::Speakers));
    assert_eq!(listened_on("KEF LSX Studio Monitors"), Some(ListenedOn::Speakers));
    assert_eq!(listened_on("Topping D90"), None);
    assert_eq!(listened_on("Galaxy Buds2 Pro"), Some(ListenedOn::Headphones));
    assert_eq!(listened_on("Moondrop IEMs"), Some(ListenedOn::Headphones));
    assert_eq!(listened_on("In-Ear Monitors"), Some(ListenedOn::Headphones), "in-ear monitors are worn");
    assert_eq!(listened_on("Sennheiser HD650"), Some(ListenedOn::Headphones));
    assert_eq!(listened_on("Headphones (HD 600)"), Some(ListenedOn::Headphones));
    assert_eq!(listened_on("beyerdynamic DT 770 PRO"), Some(ListenedOn::Headphones));
}

#[test]
fn the_output_name_matches_whole_words_only() {
    // A word inside a longer one isn't the word.
    assert_eq!(listened_on("Rosebuds Lounge DAC"), None);
    assert_eq!(listened_on("Siemens Monitor"), Some(ListenedOn::Speakers), "not headphones for the iem in Siemens");
    assert_eq!(listened_on("Studio Monitor (HD Audio)"), Some(ListenedOn::Speakers));
    assert_eq!(listened_on("Monitor"), Some(ListenedOn::Speakers));
    assert_eq!(listened_on("Monitoring Interface"), None);
    assert_eq!(listened_on("Loudspeakerless Box"), None);
    assert_eq!(listened_on("NVIDIA High Definition Audio"), None);
    assert_eq!(listened_on("Earbudsy Thing"), None);
}

struct Events(Mutex<usize>);
impl DiaryEvents for Events {
    fn changed(&self) {
        *self.0.lock() += 1;
    }
}

struct Output(&'static str);
#[async_trait]
impl ListenPlace for Output {
    async fn output_name(&self) -> Option<String> {
        Some(self.0.into())
    }
}

struct Rig {
    db: DbHandle,
    store: SyncStore,
    diary: Diary,
    events: Arc<Events>,
    clock: Arc<AtomicI64>,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("d.db")).unwrap();
    db.write(|c| {
        album(c, "al", 4);
        album(c, "b", 3);
        Ok(())
    })
    .unwrap();
    let store = SyncStore::new(db.clone(), "pc");
    let clock = Arc::new(AtomicI64::new(NOW));
    let tick = clock.clone();
    let diary = Diary::new(db.clone(), store.clone(), DiarySwitches::default(), "Den PC")
        .with_clock(Arc::new(move || tick.load(Ordering::SeqCst)));
    let events = Arc::new(Events(Mutex::new(0)));
    diary.set_events(events.clone());
    diary.set_place(Arc::new(Output("Headphones (WH-1000XM5)")));
    Rig { db, store, diary, events, clock, _dir: dir }
}

#[tokio::test]
async fn a_finished_album_writes_one_quiet_entry_with_where_it_was_heard() {
    let r = rig();
    r.db.write(|c| {
        through(c, "al", 1, 4, NOW - HOUR);
        Ok(())
    })
    .unwrap();
    assert_eq!(r.diary.album_played("al").await.unwrap(), 1);
    // Asking again (the next play's end) writes nothing new.
    assert_eq!(r.diary.album_played("al").await.unwrap(), 0);
    let list = r.diary.list(&DiaryQuery::default()).await.unwrap();
    assert_eq!(list.total, 1);
    let e = &list.entries[0];
    assert!(e.entry.auto && e.in_library);
    assert_eq!((e.entry.title.as_str(), e.entry.artist.as_str(), e.entry.year), ("Record al", "The Band", Some(1997)));
    assert_eq!((e.entry.songs_played, e.entry.songs_total, e.entry.minutes), (4, 4, 16));
    assert_eq!(e.entry.device.as_deref(), Some("Den PC"));
    assert_eq!(e.entry.output.as_deref(), Some("Headphones (WH-1000XM5)"));
    assert_eq!(e.listened_on, Some(ListenedOn::Headphones));
    assert_eq!(e.thumb.as_deref(), Some("/al/al"));
    assert_eq!(e.entry.at, NOW - HOUR + 3 * 4 * MIN + 4 * MIN, "dated when it finished");
    assert_eq!(*r.events.0.lock(), 1);
}

#[tokio::test]
async fn a_partial_play_writes_nothing_and_off_switches_stop_entries() {
    let r = rig();
    r.db.write(|c| {
        through(c, "al", 1, 2, NOW - HOUR);
        Ok(())
    })
    .unwrap();
    assert_eq!(r.diary.album_played("al").await.unwrap(), 0);
    r.db.write(|c| {
        through(c, "b", 1, 3, NOW - HOUR);
        Ok(())
    })
    .unwrap();
    r.diary.switches().auto.store(false, Ordering::Relaxed);
    assert_eq!(r.diary.album_played("b").await.unwrap(), 0, "automatic entries off");
    r.diary.switches().auto.store(true, Ordering::Relaxed);
    r.diary.switches().on.store(false, Ordering::Relaxed);
    assert_eq!(r.diary.album_played("b").await.unwrap(), 0, "no diary");
    assert!(r.diary.log("b").await.is_err(), "nothing logged by hand either");
    r.diary.switches().on.store(true, Ordering::Relaxed);
    assert_eq!(r.diary.album_played("b").await.unwrap(), 1);
}

#[tokio::test]
async fn two_sittings_write_two_entries_and_a_deleted_one_stays_gone() {
    let r = rig();
    r.db.write(|c| {
        through(c, "al", 1, 4, NOW - 20 * HOUR);
        through(c, "al", 1, 4, NOW - 2 * HOUR);
        Ok(())
    })
    .unwrap();
    assert_eq!(r.diary.album_played("al").await.unwrap(), 2);
    let list = r.diary.list(&DiaryQuery::default()).await.unwrap();
    assert_eq!(list.entries.len(), 2);
    assert!(list.entries[0].entry.at > list.entries[1].entry.at, "newest first");
    let gone = list.entries[0].id.clone();
    assert!(r.diary.delete(&gone).await.unwrap());
    assert!(!r.diary.delete(&gone).await.unwrap());
    assert_eq!(r.diary.album_played("al").await.unwrap(), 0, "a deleted entry isn't written again");
    assert_eq!(r.diary.list(&DiaryQuery::default()).await.unwrap().total, 1);
}

#[tokio::test]
async fn an_entry_or_tombstone_under_an_older_key_still_counts() {
    let r = rig();
    let last = r
        .db
        .write(|c| {
            let last = through(c, "al", 1, 4, NOW - 20 * HOUR);
            through(c, "b", 1, 3, NOW - 2 * HOUR);
            Ok(last)
        })
        .unwrap();
    // An older build wrote "al" under its listen log row, and deleted "b".
    let old = DiaryEntry { album_id: "al".into(), at: NOW - 19 * HOUR, auto: true, rating: Some(4.0), ..Default::default() };
    r.store.put(&DIARY, &format!("auto:al:{last}"), &old).unwrap();
    let b_last = r.db.read(|c| Ok(c.query_row("SELECT max(id) FROM listen_log", [], |row| row.get::<_, i64>(0))?)).unwrap();
    r.store.put(&DIARY, &format!("auto:b:{b_last}"), &old).unwrap();
    r.store.remove(&DIARY, &format!("auto:b:{b_last}")).unwrap();
    assert_eq!(r.diary.album_played("al").await.unwrap(), 0, "not written twice");
    assert_eq!(r.diary.album_played("b").await.unwrap(), 0, "a deleted entry stays gone");
    let list = r.diary.list(&DiaryQuery::default()).await.unwrap();
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].entry.rating, Some(4.0), "the old entry reads as before");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checks_after_plays_never_overlap_or_undo_an_edit() {
    let r = rig();
    r.db.write(|c| {
        through(c, "al", 1, 4, NOW - HOUR);
        Ok(())
    })
    .unwrap();
    // Two plays ending together: one entry between them.
    let (a, b) = tokio::join!(r.diary.album_played("al"), r.diary.album_played("al"));
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    let id = r.diary.list(&DiaryQuery::default()).await.unwrap().entries[0].id.clone();
    // A rating saved while another check runs is kept, and a delete holds.
    for _ in 0..20 {
        let edit = DiaryEdit { rating: Some(5.0), note: "Kept".into(), at: None };
        let (played, edited) = tokio::join!(r.diary.album_played("al"), r.diary.edit(&id, edit));
        assert_eq!(played.unwrap(), 0);
        edited.unwrap();
    }
    let e = &r.diary.list(&DiaryQuery::default()).await.unwrap().entries[0];
    assert_eq!((e.entry.rating, e.entry.note.as_str()), (Some(5.0), "Kept"));
    let (played, gone) = tokio::join!(r.diary.album_played("al"), r.diary.delete(&id));
    assert!(gone.unwrap());
    assert_eq!(played.unwrap(), 0);
    assert_eq!(r.diary.album_played("al").await.unwrap(), 0);
    assert_eq!(r.diary.list(&DiaryQuery::default()).await.unwrap().total, 0);
}

#[tokio::test]
async fn the_computer_name_is_asked_for_when_an_entry_is_written() {
    let r = rig();
    let name = Arc::new(Mutex::new("Studio Mac".to_owned()));
    let n = name.clone();
    let diary = r.diary.clone().with_device_name(Arc::new(move || n.lock().clone()));
    r.db.write(|c| {
        through(c, "al", 1, 4, NOW - 5 * HOUR);
        Ok(())
    })
    .unwrap();
    assert_eq!(diary.album_played("al").await.unwrap(), 1);
    *name.lock() = "  ".into();
    r.db.write(|c| {
        through(c, "b", 1, 3, NOW - HOUR);
        Ok(())
    })
    .unwrap();
    assert_eq!(diary.album_played("b").await.unwrap(), 1);
    let list = diary.list(&DiaryQuery::default()).await.unwrap();
    let device = |album: &str| list.entries.iter().find(|e| e.entry.album_id == album).unwrap().entry.device.clone();
    assert_eq!(device("al").as_deref(), Some("Studio Mac"));
    assert_eq!(device("b"), None, "no blank name");
}

#[test]
fn a_failed_export_names_the_file() {
    let e = super::service::export_error("Listening diary.csv", &std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied"));
    let text = e.to_string();
    assert!(text.contains("Listening diary.csv") && text.contains("access denied"), "{text}");
}

#[tokio::test]
async fn entries_take_a_rating_a_note_and_a_new_date() {
    let r = rig();
    let logged = r.diary.log("b").await.unwrap();
    assert!(!logged.entry.auto && logged.entry.output.is_none(), "logged by hand: no place guessed");
    assert_eq!(logged.entry.songs_total, 3);
    let id = logged.id.clone();
    let edited = r
        .diary
        .edit(&id, DiaryEdit { rating: Some(4.5), note: "  Warm and close.  ".into(), at: Some(NOW - 48 * HOUR) })
        .await
        .unwrap();
    assert_eq!((edited.entry.rating, edited.entry.note.as_str(), edited.entry.at), (Some(4.5), "Warm and close.", NOW - 48 * HOUR));
    // Clearing the rating keeps the note.
    let cleared = r.diary.edit(&id, DiaryEdit { rating: None, note: "Warm and close.".into(), at: None }).await.unwrap();
    assert_eq!((cleared.entry.rating, cleared.entry.at), (None, NOW - 48 * HOUR));
    assert!(r.diary.edit(&id, DiaryEdit { rating: Some(6.0), ..Default::default() }).await.is_err());
    assert!(r.diary.edit(&id, DiaryEdit { note: "x".repeat(NOTE_MAX + 1), ..Default::default() }).await.is_err());
    let future = r.diary.edit(&id, DiaryEdit { at: Some(NOW + 3 * 24 * HOUR), ..Default::default() }).await;
    assert!(matches!(&future, Err(crate::error::AppError::Invalid(m)) if m.contains("future")), "{future:?}");
    for bad in [0, -5] {
        let got = r.diary.edit(&id, DiaryEdit { at: Some(bad), ..Default::default() }).await;
        assert!(matches!(&got, Err(crate::error::AppError::Invalid(m)) if !m.contains("future")), "{got:?}");
    }
    assert!(matches!(r.diary.edit("log:nope", DiaryEdit::default()).await, Err(crate::error::AppError::NotFound(_))));
    assert!(r.diary.delete(&id).await.unwrap());
    assert_eq!(r.diary.list(&DiaryQuery::default()).await.unwrap().total, 0);
}

#[tokio::test]
async fn filters_find_rated_noted_dated_and_searched_entries() {
    let r = rig();
    let a = r.diary.log("al").await.unwrap();
    r.clock.store(NOW + MIN, Ordering::SeqCst);
    let b = r.diary.log("b").await.unwrap();
    r.diary.edit(&a.id, DiaryEdit { rating: Some(3.0), note: "Rainy day record".into(), at: Some(NOW - 400 * 24 * HOUR) }).await.unwrap();
    r.diary.edit(&b.id, DiaryEdit { rating: Some(5.0), note: String::new(), at: None }).await.unwrap();
    let ids = |l: &DiaryList| l.entries.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
    let all = r.diary.list(&DiaryQuery::default()).await.unwrap();
    assert_eq!(ids(&all), [b.id.clone(), a.id.clone()]);
    assert_eq!((all.first_at, all.last_at), (Some(NOW - 400 * 24 * HOUR), Some(NOW + MIN)));
    let noted = r.diary.list(&DiaryQuery { with_notes: true, ..Default::default() }).await.unwrap();
    assert_eq!(ids(&noted), std::slice::from_ref(&a.id));
    assert_eq!(noted.total, 2, "the total is the whole diary");
    let rated = r.diary.list(&DiaryQuery { rated: true, ..Default::default() }).await.unwrap();
    assert_eq!(rated.entries.len(), 2);
    let recent = r.diary.list(&DiaryQuery { from: Some(NOW - 24 * HOUR), until: Some(NOW + HOUR), ..Default::default() }).await.unwrap();
    assert_eq!(ids(&recent), std::slice::from_ref(&b.id));
    let found = r.diary.list(&DiaryQuery { text: Some("rainy RECORD".into()), ..Default::default() }).await.unwrap();
    assert_eq!(ids(&found), std::slice::from_ref(&a.id));
    let by_album = r.diary.list(&DiaryQuery { album_id: Some("b".into()), ..Default::default() }).await.unwrap();
    assert_eq!(ids(&by_album), std::slice::from_ref(&b.id));
}

#[tokio::test]
async fn first_heard_counts_imported_plays_and_earlier_diary_entries() {
    let r = rig();
    assert_eq!(r.diary.first_heard("al").await.unwrap(), None);
    let old = NOW - 3 * 365 * 24 * HOUR;
    r.db.write(|c| {
        play(c, "al", 1, NOW - HOUR, "done");
        listens::insert_imported(c, &[imported(old, "al-2", ImportSource::Lastfm)], NOW)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(r.diary.first_heard("al").await.unwrap(), Some(old));
    let e = r.diary.log("al").await.unwrap();
    r.diary.edit(&e.id, DiaryEdit { at: Some(old - HOUR), ..Default::default() }).await.unwrap();
    assert_eq!(r.diary.first_heard("al").await.unwrap(), Some(old - HOUR), "heard on the record player before");
}

#[tokio::test]
async fn the_diary_stays_on_this_device_unless_its_sync_is_on() {
    let r = rig();
    r.diary.log("al").await.unwrap();
    assert!(!Groups::default().namespaces().contains(&DIARY.name()), "off by default");
    let entries = r.store.entries(&Groups::default().namespaces()).unwrap();
    assert!(!entries.contains_key(DIARY.name()), "nothing for the sync file");
    let on = Groups { diary: true, ..Default::default() };
    let entries = r.store.entries(&on.namespaces()).unwrap();
    assert_eq!(entries.get(DIARY.name()).map(|m| m.len()), Some(1), "goes out once turned on");
}

#[tokio::test]
async fn a_synced_entry_from_another_device_shows_here() {
    let r = rig();
    let dir = tempfile::tempdir().unwrap();
    let db2 = DbHandle::open(dir.path().join("laptop.db")).unwrap();
    let laptop = SyncStore::new(db2.clone(), "laptop");
    let entry = DiaryEntry {
        album_id: "al".into(),
        title: "Record al".into(),
        artist: "The Band".into(),
        at: NOW - HOUR,
        auto: true,
        songs_played: 4,
        songs_total: 4,
        device: Some("Laptop".into()),
        output: Some("Speakers".into()),
        ..Default::default()
    };
    laptop.put(&DIARY, "auto:al:7", &entry).unwrap();
    let on = Groups { diary: true, ..Default::default() }.namespaces();
    let theirs = laptop.entries(&on).unwrap();
    r.store.merge(&theirs, &on).unwrap();
    let list = r.diary.list(&DiaryQuery::default()).await.unwrap();
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].entry.device.as_deref(), Some("Laptop"));
    assert_eq!(list.entries[0].listened_on, Some(ListenedOn::Speakers));
}

// ------------------------------------------------------------------ export

fn t(year: i32, month: u32, day: u32) -> LocalTime {
    LocalTime { year, month, day, hour: 21, minute: 5 }
}

fn item(id: &str, f: impl FnOnce(&mut DiaryEntry)) -> DiaryItem {
    let mut e = DiaryEntry {
        album_id: "al".into(),
        title: "Lantern Songs".into(),
        artist: "The Band".into(),
        year: Some(1997),
        at: NOW,
        songs_played: 9,
        songs_total: 10,
        minutes: 41,
        device: Some("Den PC".into()),
        output: Some("Headphones (WH-1000XM5)".into()),
        auto: true,
        ..Default::default()
    };
    f(&mut e);
    let listened_on = e.output.as_deref().and_then(listened_on);
    DiaryItem { id: id.into(), entry: e, thumb: None, in_library: true, listened_on }
}

#[test]
fn text_export_reads_like_a_journal() {
    let items = vec![
        (
            item("a", |e| {
                e.rating = Some(4.5);
                e.note = "Side two at night.\nThe bass!".into();
            }),
            t(2026, 10, 17),
        ),
        (
            item("b", |e| {
                e.auto = false;
                e.device = None;
                e.output = None;
                e.rating = Some(3.0);
            }),
            t(2026, 9, 2),
        ),
    ];
    let text = export::to_text(&items, t(2026, 10, 18));
    let want = "Listening diary\n\
Exported from Platter on 18 October 2026. 2 entries.\n\
\n\
October 2026\n\
------------\n\
\n\
Oct 17  Lantern Songs, The Band (1997)  4.5/5\n        Played 9 of 10 songs, 41 min. On Den PC, Headphones (WH-1000XM5).\n        Side two at night.\n        The bass!\n\
\n\
September 2026\n\
--------------\n\
\n\
Sep  2  Lantern Songs, The Band (1997)  3/5\n        Logged by hand.\n";
    assert_eq!(text, want);
}

#[test]
fn csv_export_has_one_row_per_entry_and_safe_fields() {
    let items = vec![
        (item("a", |e| e.note = "Loud, \"live\" feel\nyes".into()), t(2026, 10, 17)),
        (
            item("b", |e| {
                e.auto = false;
                e.title = "=SUM(1)".into();
                e.rating = Some(2.5);
                e.output = None;
            }),
            t(2026, 1, 5),
        ),
    ];
    let csv = export::to_csv(&items);
    assert!(csv.starts_with('\u{feff}'), "a byte order mark for spreadsheets");
    let body = csv.trim_start_matches('\u{feff}');
    assert!(body.starts_with(&format!("{}\r\n", export::CSV_HEADER)));
    assert!(
        body.contains("\r\n2026-10-17,21:05,Lantern Songs,The Band,1997,,9,10,41,Den PC,Headphones (WH-1000XM5),headphones,played,\"Loud, \"\"live\"\" feel\nyes\"\r\n"),
        "{body}"
    );
    assert!(body.ends_with("\r\n2026-01-05,21:05,'=SUM(1),The Band,1997,2.5,,,,Den PC,,,logged,\r\n"), "{body}");
}

#[test]
fn local_times_come_from_this_computer_clock() {
    let c = mem();
    let got = export::local_times(&c, &[NOW]).unwrap();
    let want: String = c.query_row("SELECT strftime('%Y-%m-%d', ?1 / 1000, 'unixepoch', 'localtime')", [NOW], |r| r.get(0)).unwrap();
    assert_eq!(format!("{:04}-{:02}-{:02}", got[0].year, got[0].month, got[0].day), want);
}

#[tokio::test]
async fn export_lists_the_query_in_both_formats() {
    let r = rig();
    let a = r.diary.log("al").await.unwrap();
    r.diary.edit(&a.id, DiaryEdit { rating: Some(4.0), note: "Good".into(), at: None }).await.unwrap();
    r.diary.log("b").await.unwrap();
    let text = r.diary.export_body(&DiaryQuery::default(), export::ExportFormat::Text).await.unwrap();
    assert!(text.contains("2 entries") && text.contains("Record al, The Band (1997)  4/5") && text.contains("Good"));
    let csv = r.diary.export_body(&DiaryQuery { rated: true, ..Default::default() }, export::ExportFormat::Csv).await.unwrap();
    assert_eq!(csv.trim_end().split("\r\n").count(), 2, "header and the one rated entry");
}
