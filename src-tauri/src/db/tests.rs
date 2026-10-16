//! Integration tests for the db layer: sync, queries, search, listen log.
//! Every test uses a fresh tempfile database.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;

use super::fake::{self, FakeLibrary, FakeSource};
use super::listens::{self, ContextKind, NewReport, PlayStart, ReportKind};
use super::popular;
use super::queries::{self, now_s};
use super::search::{self, recent_add, recent_clear, recent_list, recent_remove};
use super::source::{HistoryEntry, ItemKind, LibrarySource, SimilarArtist};
use super::store;
use super::*;
use crate::model::ReleaseSection;

struct Db {
    db: DbHandle,
    _dir: tempfile::TempDir,
}

fn open() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("library.db")).unwrap();
    Db { db, _dir: dir }
}

type Hook = Box<dyn FnMut(&SyncStatus) + Send>;

#[derive(Default)]
struct Recorder {
    progress: Mutex<Vec<SyncStatus>>,
    changed: Mutex<u32>,
    outcomes: Mutex<Vec<SyncOutcome>>,
    /// Ids from `artist_changed`.
    artists: Mutex<Vec<String>>,
    /// Hook run on each progress event (e.g. read the DB mid-sync).
    hook: Mutex<Option<Hook>>,
}

impl DbEvents for Recorder {
    fn sync_progress(&self, s: &SyncStatus) {
        self.progress.lock().push(s.clone());
        if let Some(h) = self.hook.lock().as_mut() {
            h(s);
        }
    }
    fn library_changed(&self) {
        *self.changed.lock() += 1;
    }
    fn sync_outcome(&self, o: &SyncOutcome) {
        self.outcomes.lock().push(o.clone());
    }
    fn artist_changed(&self, id: &str) {
        self.artists.lock().push(id.to_string());
    }
}

fn count(db: &DbHandle, table: &str) -> i64 {
    db.read(|c| Ok(c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)).unwrap()
}

async fn sync(db: &DbHandle, src: &Arc<FakeSource>, full: bool) -> SyncOutcome {
    db.sync_once(src.clone() as Arc<dyn LibrarySource>, full).await.unwrap()
}

// ---------------------------------------------------------------- sync

#[tokio::test]
async fn first_sync_pages_everything_and_reports_progress() {
    let t = open();
    let rec = Arc::new(Recorder::default());
    t.db.set_events(rec.clone());
    // 12 artists × 5 albums × 20 tracks = 1,200 tracks → 3 track pages.
    let src = Arc::new(FakeSource::new(fake::generated(12, 5, 20)));
    let out = sync(&t.db, &src, false).await;
    assert_eq!(out, SyncOutcome::Full { artists: 12, albums: 60, tracks: 1200, deleted: 0 });
    assert_eq!(count(&t.db, "artists"), 12);
    assert_eq!(count(&t.db, "albums"), 60);
    assert_eq!(count(&t.db, "tracks"), 1200);

    let pages: Vec<_> = src.page_calls.lock().iter().filter(|c| c.0 == ItemKind::Track).cloned().collect();
    assert_eq!(pages, vec![(ItemKind::Track, 0, 500), (ItemKind::Track, 500, 500), (ItemKind::Track, 1000, 500)]);

    let prog = rec.progress.lock().clone();
    let tracks: Vec<_> = prog.iter().filter(|s| s.phase == SyncPhase::Tracks).map(|s| (s.done, s.total)).collect();
    assert_eq!(tracks, vec![(0, 1200), (500, 1200), (1000, 1200), (1200, 1200)]);
    assert!(prog.iter().any(|s| s.phase == SyncPhase::Albums && s.done == 60 && s.total == 60));
    assert_eq!(prog.last().unwrap().phase, SyncPhase::Idle);
    assert!(prog.last().unwrap().last_sync_at.is_some());
    assert!(*rec.changed.lock() >= 1);
    assert_eq!(rec.outcomes.lock().len(), 1);
    assert_eq!(t.db.sync_status().phase, SyncPhase::Idle);
    assert!(t.db.sync_status().last_sync_at.is_some());
}

#[tokio::test]
async fn pages_are_browsable_while_sync_runs() {
    let t = open();
    let rec = Arc::new(Recorder::default());
    let seen = Arc::new(Mutex::new(Vec::new()));
    {
        let db = t.db.clone();
        let seen = seen.clone();
        *rec.hook.lock() = Some(Box::new(move |s: &SyncStatus| {
            if s.phase == SyncPhase::Tracks && s.done == 500 {
                // Read through the pool while the sync is mid-flight.
                let page = db.read(|c| queries::albums(c, AlbumSort::Name, 0, 10, None)).unwrap();
                let tracks = count(&db, "tracks");
                seen.lock().push((page.total, tracks));
            }
        }));
    }
    t.db.set_events(rec.clone());
    let src = Arc::new(FakeSource::new(fake::generated(10, 5, 30)));
    sync(&t.db, &src, false).await;
    assert_eq!(*seen.lock(), vec![(50, 500)]);
}

#[tokio::test]
async fn likes_made_in_other_plex_apps_arrive_with_any_sync() {
    let t = open();
    let src = Arc::new(FakeSource::new(fake::generated(1, 1, 3)));
    sync(&t.db, &src, false).await;
    let (a, b) = {
        let l = src.lib.lock();
        (l.tracks[0].id.clone(), l.tracks[1].id.clone())
    };
    // Liked elsewhere: Plex's rated list has it, but its updatedAt didn't move.
    src.lib.lock().rated = Some(vec![super::likes::Rated { track_id: a.clone(), rating: 10.0, rated_at: Some(1234) }]);
    sync(&t.db, &src, false).await;
    assert_eq!(t.db.read(|c| super::likes::liked_ids(c, 10.0)).unwrap(), vec![a.clone()]);
    let liked = t.db.read(|c| super::likes::liked_tracks(c, 10.0)).unwrap();
    assert_eq!(liked[0].rated_at, Some(1234));
    // Unliked there and another liked.
    src.lib.lock().rated = Some(vec![super::likes::Rated { track_id: b.clone(), rating: 10.0, rated_at: Some(2000) }]);
    sync(&t.db, &src, false).await;
    assert_eq!(t.db.read(|c| super::likes::liked_ids(c, 10.0)).unwrap(), vec![b]);
}

#[tokio::test]
async fn incremental_update_applies_changes_and_reindexes() {
    let t = open();
    let src = Arc::new(FakeSource::new(fake::generated(3, 2, 5)));
    sync(&t.db, &src, false).await;
    let now = now_s();
    let new_album_id;
    {
        let mut l = src.lib.lock();
        let tr = &mut l.tracks[0];
        tr.title = "Completely Renamed Song".into();
        tr.view_count = 7;
        let id = tr.id.clone();
        l.updated.insert(id, now);
        let ar = l.artists[0].clone();
        let mut al = fake::album("9001", "Brand New Album", &ar, 2026, Some("ep"), &[]);
        al.added_at = now;
        al.track_count = 1;
        let tr = fake::track("9002", "Fresh Single", &al, 1, 1);
        l.updated.insert("9001".into(), now);
        l.updated.insert("9002".into(), now);
        l.albums.push(al);
        l.tracks.push(tr);
        new_album_id = "9001".to_string();
    }
    let calls_before = src.page_calls.lock().len();
    let out = sync(&t.db, &src, false).await;
    assert_eq!(out, SyncOutcome::Incremental { changed: 3, deleted: 0 });
    assert_eq!(src.page_calls.lock().len(), calls_before, "incremental does not page the whole library");

    let page = t.db.read(|c| queries::album_page(c, &new_album_id)).unwrap();
    assert_eq!(page.album.section, ReleaseSection::SinglesEps);
    assert_eq!(page.tracks.len(), 1);
    let home = t.db.read(|c| queries::home(c, now)).unwrap();
    assert_eq!(home.recently_added[0].id, new_album_id);

    let r = search::search(&t.db, "completely renamed", SearchFilter::Songs, None).unwrap();
    assert_eq!(r.songs.len(), 1);
    assert_eq!(r.songs[0].view_count, 7);
    let r = search::search(&t.db, "Song 0-0-0", SearchFilter::Songs, None).unwrap();
    assert!(r.songs.iter().all(|s| s.title != "Song 0-0-0"), "old title left the index");
}

#[tokio::test]
async fn deletions_are_detected_incrementally_and_on_full_sync() {
    let t = open();
    let src = Arc::new(FakeSource::new(fake::generated(3, 2, 5)));
    sync(&t.db, &src, false).await;
    let (gone_album, gone_track) = {
        let mut l = src.lib.lock();
        let al = l.albums.remove(0);
        l.tracks.retain(|t| t.album_id != al.id);
        let tr = l.tracks.remove(0);
        (al.id, tr.title)
    };
    let out = sync(&t.db, &src, false).await;
    assert_eq!(out, SyncOutcome::Incremental { changed: 0, deleted: 7 });
    assert_eq!(count(&t.db, "albums"), 5);
    assert_eq!(count(&t.db, "tracks"), 24);
    assert!(t.db.read(|c| queries::album_page(c, &gone_album)).is_err());
    let r = search::search(&t.db, &gone_track, SearchFilter::Songs, None).unwrap();
    assert!(r.songs.iter().all(|s| s.title != gone_track));

    // Full sync path: unseen rows are deleted after a complete pass.
    src.lib.lock().artists.pop();
    let out = sync(&t.db, &src, true).await;
    assert_eq!(out, SyncOutcome::Full { artists: 2, albums: 5, tracks: 24, deleted: 1 });
    assert_eq!(count(&t.db, "artists"), 2);
}

#[tokio::test]
async fn failed_full_sync_keeps_existing_rows() {
    let t = open();
    let src = Arc::new(FakeSource::new(fake::generated(10, 5, 20)));
    sync(&t.db, &src, false).await;
    *src.fail_tracks_at.lock() = Some(500);
    let err = t.db.sync_once(src.clone(), true).await.unwrap_err();
    assert_eq!(err.kind(), "network");
    assert_eq!(count(&t.db, "tracks"), 1000, "no deletion after a partial pass");
    assert_eq!(t.db.sync_status().phase, SyncPhase::Idle);
}

#[tokio::test]
async fn weekly_full_resync_when_due() {
    let t = open();
    let src = Arc::new(FakeSource::new(fake::generated(2, 2, 2)));
    sync(&t.db, &src, false).await;
    assert!(matches!(sync(&t.db, &src, false).await, SyncOutcome::Incremental { .. }));
    t.db.write(|c| {
        c.execute("UPDATE sync_state SET last_full_sync_at = last_full_sync_at - 8 * 86400", [])?;
        Ok(())
    })
    .unwrap();
    assert!(matches!(sync(&t.db, &src, false).await, SyncOutcome::Full { .. }));
}

#[tokio::test]
async fn library_replaced_wipes_and_rebuilds() {
    let t = open();
    let rec = Arc::new(Recorder::default());
    t.db.set_events(rec.clone());
    let src = Arc::new(FakeSource::new(fake::generated(4, 2, 3)));
    sync(&t.db, &src, false).await;
    // Remembered before the new library has the album: the entry survives the wipe.
    t.db.write(|c| search::recent_add(c, SearchKind::Album, "51")).unwrap();

    let mut new_lib = FakeLibrary { sections: vec![fake::section("7", "uuid-b")], ..Default::default() };
    let ar = fake::artist("50", "Fresh Artist");
    let al = fake::album("51", "Fresh Album", &ar, 2020, None, &[]);
    new_lib.tracks.push(fake::track("52", "Fresh Track", &al, 1, 1));
    new_lib.albums.push(al);
    new_lib.artists.push(ar);
    *src.lib.lock() = new_lib;

    let out = sync(&t.db, &src, false).await;
    assert_eq!(out, SyncOutcome::LibraryReplaced { artists: 1, albums: 1, tracks: 1 });
    assert_eq!(count(&t.db, "artists"), 1);
    assert_eq!(count(&t.db, "tracks"), 1);
    let st = t.db.read(store::sync_state).unwrap().unwrap();
    assert_eq!((st.section_id.as_str(), st.section_uuid.as_str()), ("7", "uuid-b"));
    let recents = t.db.read(search::recent_list).unwrap();
    assert_eq!(recents.len(), 1);
    assert_eq!(recents[0].album.as_ref().unwrap().title, "Fresh Album");
    // The old library's artists are gone from search too.
    let r = search::search(&t.db, "artist 0", SearchFilter::Artists, None).unwrap();
    assert_eq!(r.artists.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), ["Fresh Artist"]);
    assert_eq!(rec.outcomes.lock().last(), Some(&out));
}

#[tokio::test]
async fn no_music_library_is_reported() {
    let t = open();
    let src = Arc::new(FakeSource::new(FakeLibrary::default()));
    assert_eq!(sync(&t.db, &src, false).await, SyncOutcome::NoMusicLibrary);
    assert_eq!(count(&t.db, "albums"), 0);
}

#[tokio::test]
async fn sync_can_be_cancelled_between_pages() {
    let t = open();
    let rec = Arc::new(Recorder::default());
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = cancel.clone();
        *rec.hook.lock() = Some(Box::new(move |s: &SyncStatus| {
            if s.phase == SyncPhase::Tracks && s.done == 500 {
                cancel.store(true, Ordering::SeqCst);
            }
        }));
    }
    t.db.set_events(rec.clone());
    let src = Arc::new(FakeSource::new(fake::generated(10, 5, 30)));
    let out = sync::run(t.db.clone(), src.clone(), false, cancel).await.unwrap();
    assert_eq!(out, SyncOutcome::Cancelled);
    assert_eq!(count(&t.db, "tracks"), 500);
    assert!(t.db.read(store::sync_state).unwrap().unwrap().last_sync_at.is_none());
    // Next run finishes the job as a full sync.
    assert!(matches!(sync(&t.db, &src, false).await, SyncOutcome::Full { tracks: 1500, .. }));
}

#[tokio::test]
async fn background_sync_is_single_flight() {
    let t = open();
    let src = Arc::new(FakeSource::new(fake::generated(10, 5, 30)));
    t.db.set_source(src.clone());
    assert!(t.db.start_sync(false).unwrap());
    assert!(!t.db.start_sync(false).unwrap(), "second start is a no-op while running");
    for _ in 0..200 {
        if t.db.sync_status().phase == SyncPhase::Idle && count(&t.db, "tracks") == 1500 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(count(&t.db, "tracks"), 1500);
}

#[tokio::test]
async fn playlists_sync_and_refetch_only_changed_items() {
    let t = open();
    let mut lib = fake::generated(2, 2, 3);
    let ids: Vec<String> = lib.tracks.iter().map(|t| t.id.clone()).collect();
    lib.playlists.push((fake::playlist("p1", "Road Trip", 3), vec![ids[5].clone(), ids[0].clone(), ids[3].clone()]));
    lib.playlists.push((fake::playlist("p2", "Focus", 1), vec![ids[1].clone()]));
    let src = Arc::new(FakeSource::new(lib));
    sync(&t.db, &src, false).await;
    let page = t.db.read(|c| queries::playlist_page(c, "p1")).unwrap();
    let got: Vec<_> = page.tracks.iter().map(|t| t.id.clone()).collect();
    assert_eq!(got, vec![ids[5].clone(), ids[0].clone(), ids[3].clone()]);
    assert_eq!(t.db.read(queries::playlists).unwrap().len(), 2);

    src.playlist_item_calls.lock().clear();
    {
        let mut l = src.lib.lock();
        l.playlists[1].0.track_count = 2;
        l.playlists[1].1.push(ids[2].clone());
        l.playlists.remove(0);
    }
    sync(&t.db, &src, false).await;
    assert_eq!(*src.playlist_item_calls.lock(), vec!["p2".to_string()]);
    let lists = t.db.read(queries::playlists).unwrap();
    assert_eq!(lists.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["p2"]);
    assert_eq!(count(&t.db, "playlist_items"), 2, "items of the removed playlist cascade away");
}

#[tokio::test]
async fn library_items_list_everything_with_added_and_played_dates() {
    let t = open();
    let mut lib = fake::generated(2, 2, 2);
    let ids: Vec<String> = lib.tracks.iter().map(|t| t.id.clone()).collect();
    let mut smart = fake::playlist("p2", "Most Played", 1);
    smart.smart = true;
    lib.playlists.push((fake::playlist("p1", "Road Trip", 1), vec![ids[0].clone()]));
    lib.playlists.push((smart, vec![ids[1].clone()]));
    let first = lib.albums[0].clone();
    let second = lib.albums[1].clone();
    let other_artist = lib.artists[1].id.clone();
    let second_track = lib.tracks.iter().find(|t| t.album_id == second.id).unwrap().id.clone();
    let src = Arc::new(FakeSource::new(lib));
    sync(&t.db, &src, false).await;

    // Plex history on the first album, a newer local play of the second
    // album started from playlist p1.
    t.db.write(|c| {
        c.execute("INSERT INTO play_history (rating_key, viewed_at) VALUES (?1, 5000)", [&ids[0]])?;
        listens::start_play(c, &play(&second_track, 9_000_000, 200_000, false))?;
        Ok(())
    })
    .unwrap();

    let items = t.db.read(queries::library_items).unwrap();
    let n = |k: LibraryKind| items.iter().filter(|i| i.kind == k).count();
    assert_eq!((n(LibraryKind::Album), n(LibraryKind::Artist), n(LibraryKind::Playlist)), (4, 2, 2));
    let get = |k: LibraryKind, id: &str| items.iter().find(|i| i.kind == k && i.id == id).unwrap().clone();

    let a1 = get(LibraryKind::Album, &first.id);
    assert_eq!(a1.creator.as_deref(), Some(first.artist_title.as_str()));
    assert_eq!(a1.creator_id.as_deref(), Some(first.artist_id.as_str()));
    assert_eq!(a1.added_at, Some(first.added_at));
    assert_eq!(a1.played_at, Some(5000));
    assert_eq!(get(LibraryKind::Album, &second.id).played_at, Some(9_000));

    // An artist: earliest album added, newest album played.
    let ar = get(LibraryKind::Artist, &first.artist_id);
    assert_eq!(ar.added_at, Some(first.added_at.min(second.added_at)));
    assert_eq!(ar.played_at, Some(9_000));
    assert_eq!(get(LibraryKind::Artist, &other_artist).played_at, None);

    let p1 = get(LibraryKind::Playlist, "p1");
    assert!(!p1.smart);
    assert_eq!(p1.played_at, Some(9_000), "played from the playlist");
    let p2 = get(LibraryKind::Playlist, "p2");
    assert!(p2.smart);
    assert_eq!(p2.played_at, None);
}

// ---------------------------------------------------------------- browse

fn sectioned_library() -> FakeLibrary {
    let mut lib = FakeLibrary { sections: vec![fake::section("1", "u")], ..Default::default() };
    let ar = fake::artist("1", "Archive Artist");
    let mk = |id: &str, title: &str, year: i32, f: Option<&str>, s: &[&str]| {
        let mut a = fake::album(id, title, &ar, year, f, s);
        a.track_count = 2;
        a
    };
    lib.albums = vec![
        mk("10", "Unreleased Build", 2021, None, &["Demo"]),
        mk("11", "Second Album", 2019, Some("Album"), &[]),
        mk("12", "First Album", 2015, Some("Album"), &[]),
        mk("13", "Untagged Thing", 2017, None, &[]),
        mk("14", "Lead Single", 2018, Some("Single"), &[]),
        mk("15", "Live at Home", 2020, Some("Album"), &["Live"]),
        mk("16", "Greatest Hits", 2022, Some("Album"), &["Compilation"]),
        mk("17", "Film Score", 2016, Some("Album"), &["Soundtrack"]),
        mk("18", "Remixed", 2023, Some("EP"), &["Remix"]),
        mk("19", "Street Tape", 2014, Some("Album"), &["Mixtape/Street"]),
        mk("20", "Club EP", 2024, Some("EP"), &[]),
    ];
    for al in lib.albums.clone() {
        // Inserted out of order on purpose: disc 2 first.
        lib.tracks.push(fake::track(&format!("{}2", al.id), &format!("{} B", al.title), &al, 2, 1));
        lib.tracks.push(fake::track(&format!("{}1", al.id), &format!("{} A", al.title), &al, 1, 1));
    }
    for (i, t) in lib.tracks.iter_mut().enumerate() {
        t.view_count = (i as u32 * 7) % 23;
    }
    lib.artists = vec![ar, fake::artist("2", "Other Artist")];
    lib
}

#[tokio::test]
async fn artist_page_groups_release_types_in_fixed_order() {
    let t = open();
    let src = Arc::new(FakeSource::new(sectioned_library()));
    sync(&t.db, &src, false).await;
    let page = t.db.read(|c| queries::artist_page(c, "1")).unwrap();
    let sections: Vec<(ReleaseSection, Vec<&str>)> = page
        .sections
        .iter()
        .map(|s| (s.section, s.albums.iter().map(|a| a.title.as_str()).collect()))
        .collect();
    assert_eq!(
        sections,
        vec![
            (ReleaseSection::Albums, vec!["Second Album", "Untagged Thing", "First Album"]),
            (ReleaseSection::SinglesEps, vec!["Club EP", "Lead Single"]),
            (ReleaseSection::Live, vec!["Live at Home"]),
            (ReleaseSection::Compilations, vec!["Greatest Hits"]),
            (ReleaseSection::Soundtracks, vec!["Film Score"]),
            (ReleaseSection::Remixes, vec!["Remixed"]),
            (ReleaseSection::Demos, vec!["Unreleased Build"]),
            (ReleaseSection::Mixtapes, vec!["Street Tape"]),
        ]
    );
    assert_eq!(page.popular.len(), 10);
    assert!(page.popular.windows(2).all(|w| w[0].view_count >= w[1].view_count));
    assert!(t.db.read(|c| queries::artist_page(c, "nope")).unwrap_err().kind() == "notFound");
}

fn sim(id: Option<&str>, name: &str) -> SimilarArtist {
    SimilarArtist { id: id.map(String::from), name: name.into() }
}

#[tokio::test]
async fn artist_page_has_plays_about_and_similar_in_library() {
    let t = open();
    let mut lib = sectioned_library();
    lib.artists[0].summary = Some("  A band from somewhere.  ".into());
    let expected: u64 = lib.tracks.iter().filter(|t| t.artist_id == "1").map(|t| t.view_count as u64).sum();
    assert!(expected > 0);
    let src = Arc::new(FakeSource::new(lib));
    sync(&t.db, &src, false).await;

    // Before the detail fetch: no similar artists yet, and sync never asked.
    let page = t.db.read(|c| queries::artist_page(c, "1")).unwrap();
    assert_eq!(page.plays, expected);
    assert_eq!(page.about.as_deref(), Some("A band from somewhere."));
    assert!(page.similar.is_empty());
    assert!(src.similar_calls.lock().is_empty());

    src.lib.lock().similar.insert(
        "1".into(),
        vec![
            sim(Some("501"), "other artist"),
            sim(Some("502"), "Not In Library"),
            sim(None, "Other Artist"),
            sim(Some("503"), "Archive Artist"),
        ],
    );
    let now = 1_800_000_000;
    assert!(similar::refresh_if_stale(&t.db, src.as_ref(), "1", now).await.unwrap());
    let page = t.db.read(|c| queries::artist_page(c, "1")).unwrap();
    // Only library artists, once each, never the artist itself.
    let names: Vec<&str> = page.similar.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(names, vec!["Other Artist"]);

    // An artist with nothing: no plays, no about, no similar.
    let other = t.db.read(|c| queries::artist_page(c, "2")).unwrap();
    assert_eq!((other.plays, other.about, other.similar.len()), (0, None, 0));

    let v = serde_json::to_value(&page).unwrap();
    assert_eq!(v["plays"], expected);
    assert_eq!(v["about"], "A band from somewhere.");
    assert_eq!(v["similar"][0]["title"], "Other Artist");
}

#[tokio::test]
async fn similar_artists_are_fetched_on_demand_and_reused_for_a_week() {
    let t = open();
    let rec = Arc::new(Recorder::default());
    t.db.set_events(rec.clone());
    let mut lib = sectioned_library();
    lib.similar.insert("1".into(), vec![sim(Some("9"), "Other Artist")]);
    let src = Arc::new(FakeSource::new(lib));
    sync(&t.db, &src, true).await;
    assert!(src.similar_calls.lock().is_empty(), "sync never fetches artist detail");
    let stored = |db: &DbHandle| {
        db.read(|c| {
            Ok(c.query_row("SELECT similar, similar_fetched_at FROM artists WHERE id = '1'", [], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
            })?)
        })
        .unwrap()
    };

    let now = 1_800_000_000;
    let before = *rec.changed.lock();
    assert!(similar::refresh_if_stale(&t.db, src.as_ref(), "1", now).await.unwrap());
    assert_eq!(stored(&t.db), (r#"[{"id":"9","name":"Other Artist"}]"#.to_string(), Some(now)));
    assert_eq!(*rec.artists.lock(), vec!["1".to_string()]);
    assert_eq!(*rec.changed.lock(), before, "no library-wide refresh");

    // Within the week: no call.
    let later = now + similar::SIMILAR_TTL_S - 1;
    assert!(!similar::refresh_if_stale(&t.db, src.as_ref(), "1", later).await.unwrap());
    assert_eq!(src.similar_calls.lock().len(), 1);

    // A full sync keeps what was fetched.
    sync(&t.db, &src, true).await;
    assert_eq!(stored(&t.db).1, Some(now));

    // After a week: fetched again; an unchanged list stores the new time but
    // doesn't tell the page.
    let week = now + similar::SIMILAR_TTL_S;
    assert!(!similar::refresh_if_stale(&t.db, src.as_ref(), "1", week).await.unwrap());
    assert_eq!(src.similar_calls.lock().len(), 2);
    assert_eq!(stored(&t.db).1, Some(week));
    assert_eq!(rec.artists.lock().len(), 1);

    // Server down: the cache stays as it was and the error comes back.
    *src.fail_similar.lock() = true;
    let much_later = week + similar::SIMILAR_TTL_S;
    assert!(similar::refresh_if_stale(&t.db, src.as_ref(), "1", much_later).await.is_err());
    assert_eq!(stored(&t.db), (r#"[{"id":"9","name":"Other Artist"}]"#.to_string(), Some(week)));

    // An empty answer is stored too, so it isn't asked again every visit.
    *src.fail_similar.lock() = false;
    assert!(similar::refresh_if_stale(&t.db, src.as_ref(), "2", now).await.is_ok());
    assert!(!similar::refresh_if_stale(&t.db, src.as_ref(), "2", now + 60).await.unwrap());
    assert_eq!(src.similar_calls.lock().iter().filter(|i| *i == "2").count(), 1);

    // Unknown artists are never fetched.
    assert!(!similar::refresh_if_stale(&t.db, src.as_ref(), "nope", now).await.unwrap());
    assert!(!src.similar_calls.lock().iter().any(|i| i == "nope"));
}

/// One track in the popular fixtures: (id, title, rating count, plays,
/// last played, track artist line, length in seconds).
type Pop<'a> = (&'a str, &'a str, Option<u32>, u32, Option<i64>, Option<&'a str>, u64);

fn popular_library(own: &[Pop], others: &[(&str, &[Pop])]) -> FakeLibrary {
    let mut lib = FakeLibrary { sections: vec![fake::section("1", "u")], ..Default::default() };
    let mut artists = vec![fake::artist("a1", "Odessa Wren")];
    let add = |lib: &mut FakeLibrary, ar: &crate::model::Artist, album_id: &str, specs: &[Pop]| {
        let mut al = fake::album(album_id, &format!("{} LP", ar.title), ar, 2020, Some("Album"), &[]);
        al.track_count = specs.len() as u32;
        for (n, &(id, title, count, plays, last, line, secs)) in specs.iter().enumerate() {
            let mut tr = fake::track(id, title, &al, 1, n as u32 + 1);
            tr.rating_count = count;
            tr.view_count = plays;
            tr.last_viewed_at = last;
            tr.track_artist = line.map(String::from);
            tr.duration_ms = secs * 1000;
            lib.tracks.push(tr);
        }
        lib.albums.push(al);
    };
    let me = artists[0].clone();
    add(&mut lib, &me, "al1", own);
    for (i, (name, specs)) in others.iter().enumerate() {
        let ar = fake::artist(&format!("a{}", i + 2), name);
        add(&mut lib, &ar, &format!("al{}", i + 2), specs);
        artists.push(ar);
    }
    lib.artists = artists;
    lib
}

async fn popular_of(lib: FakeLibrary, artist: &str) -> Vec<(String, Option<u32>)> {
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(lib)), false).await;
    let page = t.db.read(|c| queries::artist_page(c, artist)).unwrap();
    page.popular.into_iter().map(|p| (p.id, p.rating_count)).collect()
}

fn ids(v: &[(String, Option<u32>)]) -> Vec<&str> {
    v.iter().map(|(id, _)| id.as_str()).collect()
}

/// Store a global top list for a page id, as artist info would.
fn put_global(db: &DbHandle, id: &str, songs: &[(&str, u64)]) {
    let top = popular::GlobalTop::from_pairs("listenbrainz", songs.iter().map(|(t, n)| (t.to_string(), *n)));
    let id = id.to_owned();
    db.write(move |c| {
        crate::artistinfo::cache::put(c, &id, popular::GLOBAL_ROW, Some(&top), now_s(), 3600)?;
        Ok(())
    })
    .unwrap();
}

fn page_ids(db: &DbHandle, id: &str, source: PopularSource) -> Vec<String> {
    db.read(|c| queries::artist_page_by(c, id, source)).unwrap().popular.into_iter().map(|p| p.id).collect()
}

#[tokio::test]
async fn popular_without_a_global_list_ranks_by_plays_not_rating_counts() {
    let own: Vec<Pop> = vec![
        ("t1", "Low Count", Some(1_000), 50, Some(10), None, 200),
        ("t2", "Big Hit", Some(900_000), 0, None, None, 210),
        ("t3", "Played A Lot", None, 40, Some(5), None, 220),
        ("t4", "Mid", Some(50_000), 3, None, None, 230),
        ("t5", "Played Lately", None, 40, Some(99), None, 240),
        ("t6", "Never Played", None, 0, None, None, 250),
    ];
    let got = popular_of(popular_library(&own, &[]), "a1").await;
    // Rating counts only break the tie between unplayed songs.
    assert_eq!(ids(&got), ["t1", "t5", "t3", "t4", "t2", "t6"]);
}

#[tokio::test]
async fn popular_follows_the_global_list_over_rating_counts() {
    // One album has huge rating counts; the global list ranks the other one.
    let own: Vec<Pop> = vec![
        ("n1", "Deep Cut", Some(318_000), 0, None, None, 200),
        ("n2", "Filler", Some(133_000), 0, None, None, 210),
        ("o1", "Famous Single", Some(618), 1, None, None, 220),
        ("o2", "Fan Favourite (Remastered)", None, 0, None, None, 230),
        ("o3", "Mine", None, 30, Some(5), None, 240),
    ];
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(popular_library(&own, &[]))), false).await;
    // No list yet: plays order.
    assert_eq!(page_ids(&t.db, "a1", PopularSource::Global), ["o3", "o1", "n1", "n2", "o2"]);
    put_global(&t.db, "a1", &[("Famous Single", 9_000), ("Fan Favourite", 4_000), ("Fan Favourite - 2011 Remaster", 900), ("Deep Cut", 10)]);
    assert_eq!(page_ids(&t.db, "a1", PopularSource::Global), ["o1", "o2", "n1", "o3", "n2"]);
    // My plays ignores the list.
    assert_eq!(page_ids(&t.db, "a1", PopularSource::MyPlays), ["o3", "o1", "n1", "n2", "o2"]);

    // The handle carries the choice for the commands and play contexts.
    assert_eq!(t.db.popular_source(), PopularSource::Global);
    t.db.set_popular_source(PopularSource::MyPlays);
    assert_eq!(t.db.popular_source(), PopularSource::MyPlays);
}

#[tokio::test]
async fn popular_includes_whole_name_credits_on_other_artists_tracks() {
    let own: Vec<Pop> = vec![("t1", "Own Song", Some(10), 0, None, None, 200)];
    let moth: Vec<Pop> = vec![
        ("m1", "Feature One", Some(500), 0, None, Some("Moth & Meridian feat. Odessa Wren"), 200),
        ("m2", "Not Her", Some(900), 0, None, Some("Moth & Meridian, Odessa Wrenfield"), 200),
        ("m3", "Bracketed", None, 7, None, Some("Moth & Meridian (FT. odessa wren)"), 200),
        ("m4", "No Credit", Some(999), 0, None, None, 200),
    ];
    let salt: Vec<Pop> = vec![("s1", "Duet", Some(20), 0, None, Some("Saltglass & Odessa Wren"), 200)];
    let lib = popular_library(&own, &[("Moth & Meridian", &moth), ("Saltglass", &salt)]);
    let got = popular_of(lib.clone(), "a1").await;
    assert_eq!(ids(&got), ["m3", "m1", "s1", "t1"]);

    // A name inside a longer known name is not a credit: "Wren" is its own
    // artist, but "Odessa Wren" lines don't credit it.
    let mut lib = lib;
    lib.artists.push(fake::artist("a9", "Wren"));
    let got = popular_of(lib, "a9").await;
    assert!(got.is_empty(), "{got:?}");

    // A name with "&" in it is found whole on someone else's track.
    let salt2: Vec<Pop> = vec![("s2", "Trio", Some(5), 0, None, Some("Saltglass, Moth & Meridian"), 200)];
    let lib = popular_library(&[], &[("Moth & Meridian", &moth), ("Saltglass", &salt2)]);
    let got = popular_of(lib, "a2").await;
    assert_eq!(ids(&got), ["m3", "m4", "m2", "m1", "s2"]);
}

#[tokio::test]
async fn popular_collapses_copies_of_the_same_song() {
    let own: Vec<Pop> = vec![
        ("t1", "Anthem", Some(100), 0, None, None, 200),
        ("t2", "ANTHEM", Some(5_000), 0, None, None, 201),
        ("t3", "Anthem", None, 90, None, None, 201),
        ("t4", "Anthem", Some(50), 0, None, None, 260),
        ("t5", "Other", None, 1, None, None, 200),
    ];
    let comp: Vec<Pop> = vec![("c1", "anthem", Some(9_000), 0, None, Some("Various, Odessa Wren"), 202)];
    let got = popular_of(popular_library(&own, &[("Various", &comp)]), "a1").await;
    // t3 (most played) stands for the copies within 2 s; t4 is a different length.
    assert_eq!(ids(&got), ["t3", "t5", "t4"]);
}

#[tokio::test]
async fn popular_fills_to_ten_and_handles_small_catalogs() {
    let names: Vec<String> = (0..14).map(|i| format!("Song {i}")).collect();
    let tids: Vec<String> = (0..14).map(|i| format!("t{i}")).collect();
    let own: Vec<Pop> = (0..14)
        .map(|i| (tids[i].as_str(), names[i].as_str(), (i % 3 == 0).then_some(i as u32 * 10), 0, None, None, 200))
        .collect();
    let got = popular_of(popular_library(&own, &[]), "a1").await;
    assert_eq!(got.len(), 10, "unplayed songs fill the list");
    assert_eq!(ids(&got)[..5], ["t12", "t9", "t6", "t3", "t0"]);

    let got = popular_of(popular_library(&own[..3], &[]), "a1").await;
    assert_eq!(ids(&got), ["t0", "t1", "t2"]);
}

/// Odessa Wren (a1) plus two artists whose tracks credit a guest with no
/// library entry, "Rue Ellis".
fn credited_library() -> FakeLibrary {
    let own: Vec<Pop> = vec![("t1", "Own Song", None, 2, None, None, 200)];
    let moth: Vec<Pop> = vec![
        ("m1", "Night Drive", None, 4, None, Some("Moth & Meridian feat. Rue Ellis"), 200),
        ("m2", "Close Call", None, 0, None, Some("Moth & Meridian, Rue Ellison"), 200),
        ("m3", "With Wren", None, 1, None, Some("Moth & Meridian, Odessa Wren"), 200),
    ];
    let salt: Vec<Pop> = vec![
        ("s1", "Duet", None, 10, None, Some("Saltglass & rue ellis"), 200),
        ("s2", "Money", None, 0, None, Some("Saltglass feat. $"), 200),
        ("s3", "Tyler Song", None, 0, None, Some("Tyler, The Maker feat. Rue Ellis"), 200),
    ];
    popular_library(&own, &[("Moth & Meridian", &moth), ("Saltglass", &salt)])
}

#[tokio::test]
async fn credited_artist_page_is_built_from_the_tracks_that_credit_them() {
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(credited_library())), false).await;
    let page = t.db.read(|c| queries::artist_page(c, "credit:rue ellis")).unwrap();
    assert_eq!(page.artist.title, "Rue Ellis", "as written on the first credit");
    assert_eq!(page.artist.id, "credit:rue ellis");
    let got: Vec<_> = page.popular.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(got, ["s1", "m1", "s3"], "plays order, Rue Ellison is someone else");
    assert_eq!(page.plays, 14);
    let albums: Vec<_> = page.appears_on.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(albums.len(), 2);
    assert!(albums.contains(&"al2") && albums.contains(&"al3"));
    assert!(page.sections.is_empty() && page.similar.is_empty() && page.about.is_none());

    // The global list ranks a credited page too, under its own id.
    put_global(&t.db, "credit:rue ellis", &[("Tyler Song", 50), ("Night Drive", 10)]);
    assert_eq!(page_ids(&t.db, "credit:rue ellis", PopularSource::Global), ["s3", "m1", "s1"]);

    // Partial names and names nobody credits: not found.
    for id in ["credit:rue", "credit:nobody", "credit:"] {
        let err = t.db.read(|c| queries::artist_page(c, id)).unwrap_err();
        assert!(matches!(err, crate::error::AppError::NotFound(_)), "{id}: {err:?}");
    }

    // A name with no letters or digits.
    let page = t.db.read(|c| queries::artist_page(c, "credit:$")).unwrap();
    assert_eq!(page.popular.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["s2"]);

    // The play context covers every credited track.
    let all = t.db.read(|c| queries::credited_context_tracks(c, "credit:rue ellis")).unwrap();
    assert_eq!(all.len(), 3);
    assert!(t.db.read(|c| queries::credited_context_tracks(c, "a1")).unwrap().is_empty());
}

#[tokio::test]
async fn library_artist_pages_list_albums_they_appear_on() {
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(credited_library())), false).await;
    let page = t.db.read(|c| queries::artist_page(c, "a1")).unwrap();
    assert_eq!(page.appears_on.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["al2"]);
    assert_eq!(page.popular.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["t1", "m3"]);
}

#[tokio::test]
async fn learned_names_with_separators_stay_whole() {
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(credited_library())), false).await;
    // Before: "Tyler, The Maker" splits into two names.
    assert!(t.db.read(|c| queries::artist_page(c, "credit:tyler")).is_ok());
    t.db.write(|c| {
        assert_eq!(popular::learn_names(c, ["Tyler, The Maker", "Plain Name", "Odessa Wren"])?, 1, "only names with separators");
        Ok(())
    })
    .unwrap();
    let page = t.db.read(|c| queries::artist_page(c, "credit:tyler, the maker")).unwrap();
    assert_eq!(page.artist.title, "Tyler, The Maker");
    assert!(t.db.read(|c| queries::artist_page(c, "credit:tyler")).is_err());
    // Rue Ellis on that track is still credited.
    assert_eq!(page_ids(&t.db, "credit:rue ellis", PopularSource::MyPlays).len(), 3);
    let names = t.db.read(queries::artist_names).unwrap();
    assert!(names.contains(&ArtistName { id: "credit:tyler, the maker".into(), title: "Tyler, The Maker".into() }));
}

#[tokio::test]
async fn search_lists_credited_artists() {
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(credited_library())), false).await;
    let r = search::search(&t.db, "rue ellis", SearchFilter::All, None).unwrap();
    let ids: Vec<_> = r.artists.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"credit:rue ellis"), "{ids:?}");
    assert!(!ids.contains(&"credit:moth & meridian"), "library artists keep their own entry");
    let hit = r.artists.iter().find(|a| a.id == "credit:rue ellis").unwrap();
    assert_eq!(hit.title, "Rue Ellis");
    // Library names still come up as library artists.
    let r = search::search(&t.db, "saltglass", SearchFilter::All, None).unwrap();
    assert_eq!(r.artists.first().map(|a| a.id.as_str()), Some("a3"));
    // Off in Settings: no credited artists.
    t.db.set_link_featured(false);
    let r = search::search(&t.db, "rue ellis", SearchFilter::All, None).unwrap();
    assert!(r.artists.iter().all(|a| !a.id.starts_with("credit:")));
}

#[tokio::test]
async fn artist_names_lists_every_artist() {
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(sectioned_library())), false).await;
    let names = t.db.read(queries::artist_names).unwrap();
    assert_eq!(
        names,
        vec![
            ArtistName { id: "1".into(), title: "Archive Artist".into() },
            ArtistName { id: "2".into(), title: "Other Artist".into() },
        ]
    );
}

#[tokio::test]
async fn full_sync_stores_rating_count_and_keeps_it_when_missing_later() {
    let t = open();
    let mut lib = sectioned_library();
    lib.tracks[0].rating_count = Some(77);
    let id = lib.tracks[0].id.clone();
    sync(&t.db, &Arc::new(FakeSource::new(lib.clone())), true).await;
    let stored = |db: &DbHandle| -> Option<u32> {
        db.read(|c| Ok(c.query_row("SELECT rating_count FROM tracks WHERE id = ?1", [&id], |r| r.get(0))?)).unwrap()
    };
    assert_eq!(stored(&t.db), Some(77));
    // An answer without the field (another endpoint) doesn't wipe it.
    lib.tracks[0].rating_count = None;
    sync(&t.db, &Arc::new(FakeSource::new(lib)), true).await;
    assert_eq!(stored(&t.db), Some(77));
}

#[tokio::test]
async fn album_page_orders_tracks_and_totals_duration() {
    let t = open();
    let src = Arc::new(FakeSource::new(sectioned_library()));
    sync(&t.db, &src, false).await;
    let page = t.db.read(|c| queries::album_page(c, "11")).unwrap();
    let order: Vec<_> = page.tracks.iter().map(|t| (t.disc_number, t.track_number)).collect();
    assert_eq!(order, vec![(1, 1), (2, 1)]);
    assert_eq!(page.total_duration_ms, 400_000);
    assert_eq!(page.more_by_artist.len(), 10);
    assert!(page.more_by_artist.iter().all(|a| a.id != "11"));
    assert_eq!(page.more_by_artist[0].title, "Club EP", "newest first");
}

#[tokio::test]
async fn album_page_has_release_date_label_and_artist_photo() {
    let t = open();
    let mut lib = sectioned_library();
    lib.albums[1].released = Some("2019-06-18".into());
    lib.albums[1].studio = Some("Night Records".into());
    lib.artists[0].thumb = Some("/library/metadata/1/thumb/9".into());
    sync(&t.db, &Arc::new(FakeSource::new(lib.clone())), false).await;
    let page = t.db.read(|c| queries::album_page(c, "11")).unwrap();
    assert_eq!(page.released.as_deref(), Some("2019-06-18"));
    assert_eq!(page.studio.as_deref(), Some("Night Records"));
    assert_eq!(page.artist_thumb.as_deref(), Some("/library/metadata/1/thumb/9"));
    // None when Plex has nothing, and a later sync can clear them.
    let other = t.db.read(|c| queries::album_page(c, "12")).unwrap();
    assert_eq!((other.released, other.studio), (None, None));
    lib.albums[1].studio = None;
    sync(&t.db, &Arc::new(FakeSource::new(lib)), true).await;
    let page = t.db.read(|c| queries::album_page(c, "11")).unwrap();
    assert_eq!(page.studio, None);
    // Not sent with every album: only the page carries them.
    let json = serde_json::to_value(&page.album).unwrap();
    assert!(json.get("released").is_none() && json.get("studio").is_none());
}

#[tokio::test]
async fn sync_keeps_whether_plex_matched_each_album() {
    let t = open();
    let mut lib = sectioned_library();
    lib.albums[0].matched = Some(true);
    lib.albums[1].matched = Some(false);
    sync(&t.db, &Arc::new(FakeSource::new(lib)), false).await;
    let matched = |id: &'static str| {
        t.db.read(move |c| Ok(c.query_row("SELECT matched FROM albums WHERE id = ?1", [id], |r| r.get::<_, Option<bool>>(0))?))
            .unwrap()
    };
    assert_eq!(matched("10"), Some(true));
    assert_eq!(matched("11"), Some(false));
    assert_eq!(matched("12"), None);
    let json = serde_json::to_value(t.db.read(|c| queries::album_page(c, "11")).unwrap().album).unwrap();
    assert!(json.get("matched").is_none(), "not sent to the UI");
}

#[tokio::test]
async fn recent_tracks_are_newest_first_with_repeats() {
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(sectioned_library())), false).await;
    t.db.write(|c| {
        for (key, at) in [("111", 1_000), ("121", 2_000), ("111", 3_000), ("gone", 4_000), ("131", 5_000)] {
            listens::start_play(c, &play(key, at, 200_000, false))?;
        }
        Ok(())
    })
    .unwrap();
    let ids: Vec<_> = t.db.read(|c| queries::recent_tracks(c, 10)).unwrap().into_iter().map(|t| t.id).collect();
    assert_eq!(ids, vec!["131", "111", "121", "111"], "newest first, repeats kept, missing songs skipped");
    let ids: Vec<_> = t.db.read(|c| queries::recent_tracks(c, 2)).unwrap().into_iter().map(|t| t.id).collect();
    assert_eq!(ids, vec!["131", "111"]);
}

#[tokio::test]
async fn recents_group_plays_into_sessions_by_context_with_likes() {
    use super::history::{self, RecentsKind};
    let t = open();
    let mut lib = sectioned_library();
    let min = 60_000;
    let base = 1_700_000_000_000i64;
    // Two songs liked a minute apart, an hour before the first play.
    for (id, at) in [("101", base / 1000 - 3600), ("102", base / 1000 - 3540)] {
        let tr = lib.tracks.iter_mut().find(|t| t.id == id).unwrap();
        tr.user_rating = Some(10.0);
        tr.rated_at = Some(at);
    }
    sync(&t.db, &Arc::new(FakeSource::new(lib)), false).await;
    let ctx = |kind: Option<ContextKind>, id: Option<&str>, key: &str, at: i64, skipped: bool| {
        t.db.write(|c| {
            let p = listens::start_play(c, &PlayStart {
                rating_key: key.into(),
                part_key: None,
                started_at: at,
                duration_ms: 3 * min as u64,
                context_kind: kind,
                context_id: id.map(str::to_string),
                device: None,
                offline: false,
            })?;
            let heard = if skipped { 5_000 } else { 3 * min as u64 };
            listens::end_play(c, p, at + heard as i64, heard, heard, !skipped).map(|_| ())
        })
        .unwrap();
    };
    ctx(Some(ContextKind::Album), Some("11"), "111", base, false);
    ctx(Some(ContextKind::Album), Some("11"), "112", base + 3 * min, false);
    ctx(Some(ContextKind::Album), Some("11"), "112", base + 6 * min, true); // a skip: not a play
    ctx(Some(ContextKind::Search), None, "121", base + 10 * min, false);
    ctx(None, None, "gone", base + 13 * min, false); // not in the library
    ctx(Some(ContextKind::Artist), Some("1"), "131", base + 14 * min, false);
    ctx(Some(ContextKind::Album), Some("11"), "111", base + 120 * min, false); // back later: a new session
    ctx(Some(ContextKind::Liked), None, "101", base + 124 * min, false);
    ctx(Some(ContextKind::Playlist), Some("nope"), "141", base + 127 * min, false); // playlist gone: loose songs

    let got = t.db.read(|c| history::recents(c, base - 86_400_000, 10.0, 50)).unwrap();
    let summary: Vec<_> = got
        .iter()
        .map(|e| {
            let what = match (&e.kind, &e.item) {
                (RecentsKind::Context, Some(i)) => format!("{}:{}", i.key().0, i.key().1),
                (k, _) => format!("{k:?}"),
            };
            (what, e.tracks.iter().map(|t| t.id.clone()).collect::<Vec<_>>())
        })
        .collect();
    let s = |w: &str, ids: &[&str]| (w.to_string(), ids.iter().map(|i| i.to_string()).collect::<Vec<_>>());
    assert_eq!(
        summary,
        vec![
            s("Songs", &["141"]),
            s("Liked", &["101"]),
            s("album:11", &["111"]),
            s("artist:1", &["131"]),
            s("Songs", &["121"]),
            s("album:11", &["112", "111"]),
            s("LikedAdded", &["102", "101"]),
        ],
    );
    assert_eq!(got[0].at, base + 127 * min);
    // The limit keeps the newest.
    assert_eq!(t.db.read(|c| history::recents(c, base - 86_400_000, 10.0, 2)).unwrap().len(), 2);
    // Older than the window: left out.
    assert!(t.db.read(|c| history::recents(c, base + 200 * min, 10.0, 50)).unwrap().is_empty());
}

#[tokio::test]
async fn library_lists_page_sort_and_filter() {
    let t = open();
    let src = Arc::new(FakeSource::new(sectioned_library()));
    sync(&t.db, &src, false).await;

    let p = t.db.read(|c| queries::albums(c, AlbumSort::Name, 0, 4, None)).unwrap();
    assert_eq!(p.total, 11);
    assert_eq!(
        p.items.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        vec!["Club EP", "Film Score", "First Album", "Greatest Hits"]
    );
    let p2 = t.db.read(|c| queries::albums(c, AlbumSort::Name, 4, 4, None)).unwrap();
    assert_eq!(p2.items[0].title, "Lead Single");
    let all: Vec<String> = (0..3)
        .flat_map(|i| t.db.read(|c| queries::albums(c, AlbumSort::Name, i * 4, 4, None)).unwrap().items)
        .map(|a| a.id)
        .collect();
    assert_eq!(all.len(), 11);

    let y = t.db.read(|c| queries::albums(c, AlbumSort::Year, 0, 2, None)).unwrap();
    assert_eq!(y.items[0].year, Some(2024));
    let singles = t.db.read(|c| queries::albums(c, AlbumSort::Name, 0, 50, Some(ReleaseSection::SinglesEps))).unwrap();
    assert_eq!(singles.total, 2);
    assert!(singles.items.iter().all(|a| a.section == ReleaseSection::SinglesEps));
    for sort in [AlbumSort::Artist, AlbumSort::Recent, AlbumSort::Plays] {
        assert_eq!(t.db.read(|c| queries::albums(c, sort, 0, 100, None)).unwrap().items.len(), 11);
    }

    let a = t.db.read(|c| queries::artists(c, ArtistSort::Name, 0, 1)).unwrap();
    assert_eq!((a.total, a.items[0].title.as_str()), (2, "Archive Artist"));
    let r = t.db.read(|c| queries::artists(c, ArtistSort::Recent, 0, 10)).unwrap();
    assert_eq!(r.items[0].id, "1", "artist with the newest album first");
    t.db.read(|c| queries::artists(c, ArtistSort::Plays, 0, 10)).unwrap();
}

#[tokio::test]
async fn home_rows_from_history_and_listens() {
    let t = open();
    let mut lib = fake::generated(4, 3, 4);
    let now = now_s();
    let tr = |a: usize, b: usize, n: usize| lib.tracks[(a * 3 + b) * 4 + n].id.clone();
    let mut h = Vec::new();
    // Album (0,0): finished long ago. Album (1,1): played lately, unfinished.
    for n in 0..4 {
        h.push(HistoryEntry { track_id: tr(0, 0, n), viewed_at: now - 60 * 86400 });
    }
    for i in 0..5 {
        h.push(HistoryEntry { track_id: tr(1, 1, 0), viewed_at: now - 86400 * (i + 1) });
    }
    h.push(HistoryEntry { track_id: tr(2, 0, 1), viewed_at: now - 3600 });
    lib.history = h;
    let local_track = tr(3, 2, 0);
    let src = Arc::new(FakeSource::new(lib.clone()));
    sync(&t.db, &src, false).await;
    // An offline local play is the newest of all.
    t.db.write(|c| {
        listens::start_play(c, &PlayStart {
            rating_key: local_track.clone(),
            part_key: None,
            started_at: now * 1000 - 10_000,
            duration_ms: 200_000,
            context_kind: Some(ContextKind::Album),
            context_id: None,
            device: None,
            offline: true,
        })
    })
    .unwrap();

    let home = t.db.read(|c| queries::home(c, now)).unwrap();
    let album_of = |track_id: &str| lib.tracks.iter().find(|t| t.id == track_id).unwrap().album_id.clone();
    let played: Vec<_> = home.recently_played.iter().map(|a| a.key().1.to_string()).collect();
    assert!(home.recently_played.iter().all(|i| i.key().0 == "album"));
    assert_eq!(played, vec![album_of(&local_track), album_of(&tr(2, 0, 1)), album_of(&tr(1, 1, 0)), album_of(&tr(0, 0, 0))]);
    assert_eq!(home.recently_added.len(), queries::HOME_ROW);
    assert!(home.recently_added.windows(2).all(|w| w[0].added_at >= w[1].added_at));
    // Jump back in: (1,1) qualifies (5 plays, unfinished) but it is already
    // in the first six recently played, so the row stays empty here.
    assert!(home.jump_back_in.is_empty());
}

fn keys(items: &[HomeItem]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|i| {
            let (k, id) = i.key();
            (k.to_string(), id.to_string())
        })
        .collect()
}

fn local_play(db: &DbHandle, track: &str, at_s: i64, kind: ContextKind, id: Option<&str>) {
    db.write(|c| {
        let play = listens::start_play(c, &PlayStart {
            rating_key: track.to_string(),
            part_key: None,
            started_at: at_s * 1000,
            duration_ms: 200_000,
            context_kind: Some(kind),
            context_id: id.map(str::to_string),
            device: None,
            offline: false,
        })?;
        listens::end_play(c, play, at_s * 1000 + 200_000, 200_000, 200_000, true).map(|_| ())
    })
    .unwrap();
}

#[tokio::test]
async fn home_recents_credit_the_artist_or_playlist_a_play_started_from() {
    let t = open();
    let mut lib = fake::generated(3, 2, 3);
    let now = now_s();
    let tr = |a: usize, b: usize, n: usize| lib.tracks[(a * 2 + b) * 3 + n].id.clone();
    let pl_tracks = vec![tr(0, 0, 0), tr(1, 0, 0)];
    lib.playlists.push((fake::playlist("p1", "Mix", 2), pl_tracks.clone()));
    lib.playlists.push((fake::playlist("p2", "Another", 0), vec![]));
    lib.artists[2].view_count = 40;
    lib.artists[0].view_count = 90;
    lib.albums[3].view_count = 12;
    // Plex history: album (1,1) an hour ago, plus the playlist's tracks as
    // reported to Plex from this computer.
    lib.history = vec![
        HistoryEntry { track_id: tr(1, 1, 0), viewed_at: now - 3600 },
        HistoryEntry { track_id: pl_tracks[0].clone(), viewed_at: now - 600 + 200 },
        HistoryEntry { track_id: pl_tracks[1].clone(), viewed_at: now - 300 + 200 },
    ];
    let artist2 = lib.artists[2].id.clone();
    let src = Arc::new(FakeSource::new(lib.clone()));
    sync(&t.db, &src, false).await;
    local_play(&t.db, &pl_tracks[0], now - 600, ContextKind::Playlist, Some("p1"));
    local_play(&t.db, &pl_tracks[1], now - 300, ContextKind::Playlist, Some("p1"));
    local_play(&t.db, &tr(2, 1, 2), now - 60, ContextKind::Artist, Some(&artist2));

    let home = t.db.read(|c| queries::home(c, now)).unwrap();
    let s = |k: &str, id: &str| (k.to_string(), id.to_string());
    assert_eq!(
        keys(&home.recently_played),
        vec![s("artist", &artist2), s("playlist", "p1"), s("album", &lib.albums[3].id)],
        "the playlist's and the artist's albums don't show on their own",
    );
    // Shortcuts: what's hot (the playlist played twice beats the artist
    // played once, a bit more recently), then most played (album, artist,
    // playlist in turn), without repeats.
    assert_eq!(
        keys(&home.shortcuts),
        vec![s("playlist", "p1"), s("artist", &artist2), s("album", &lib.albums[3].id), s("artist", &lib.artists[0].id)],
    );
    assert_eq!(home.top_artists.iter().map(|a| a.id.clone()).collect::<Vec<_>>(), vec![lib.artists[0].id.clone(), artist2.clone()]);
    assert_eq!(home.playlists.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["p1", "p2"], "played first, then by title");

    let all = t.db.read(|c| queries::home_shelf(c, now, HomeShelf::RecentlyAdded, 500)).unwrap();
    assert_eq!(all.len(), 6, "every album, capped by the library");
    let top = t.db.read(|c| queries::home_shelf(c, now, HomeShelf::TopArtists, 1)).unwrap();
    assert_eq!(keys(&top), vec![s("artist", &lib.artists[0].id)]);
}

#[test]
fn shortcuts_take_eight_hot_first_then_favourites_then_the_rest() {
    let al = |id: &str| HomeItem::Album { album: fake::album(id, id, &fake::artist("a", "A"), 2000, None, &[]) };
    let ar = |id: &str| HomeItem::Artist { artist: fake::artist(id, id) };
    let hot: Vec<_> = (0..6).map(|i| al(&format!("h{i}"))).collect();
    let most = vec![al("h0"), ar("x"), al("m1")];
    let recent = vec![al("r0"), al("h5")];
    let got: Vec<_> = queries::shortcuts(&hot, &most, &recent).iter().map(|i| i.key().1.to_string()).collect();
    assert_eq!(got, vec!["h0", "h1", "h2", "h3", "x", "m1", "h4", "h5"]);
    // Few hot items: favourites, then older recent ones fill.
    let got: Vec<_> = queries::shortcuts(&hot[..1], &most, &recent).iter().map(|i| i.key().1.to_string()).collect();
    assert_eq!(got, vec!["h0", "x", "m1", "r0", "h5"]);
    assert!(queries::shortcuts(&[], &[], &[]).is_empty());
}

#[tokio::test]
async fn shortcuts_rank_by_recent_plays_with_frequency_beating_a_single_play() {
    let t = open();
    let lib = fake::generated(4, 1, 4);
    let now = now_s();
    sync(&t.db, &Arc::new(FakeSource::new(lib.clone())), false).await;
    let tracks_of = |i: usize| lib.tracks.iter().filter(|t| t.album_id == lib.albums[i].id).map(|t| t.id.clone()).collect::<Vec<_>>();
    // Album 0: one play a minute ago. Album 1: four plays two hours ago.
    // Album 2: four plays five days ago (fading). Album 3: one play from
    // Liked Songs (doesn't count) and one skip (doesn't count).
    local_play(&t.db, &tracks_of(0)[0], now - 60, ContextKind::Album, Some(&lib.albums[0].id));
    for id in tracks_of(1) {
        local_play(&t.db, &id, now - 7200, ContextKind::Album, Some(&lib.albums[1].id));
    }
    for id in tracks_of(2) {
        local_play(&t.db, &id, now - 5 * 86400, ContextKind::Album, Some(&lib.albums[2].id));
    }
    local_play(&t.db, &tracks_of(3)[0], now - 30, ContextKind::Liked, None);
    t.db.write(|c| {
        let p = listens::start_play(c, &play(&tracks_of(3)[1], now * 1000 - 20_000, 200_000, false))?;
        listens::end_play(c, p, now * 1000 - 15_000, 5_000, 5_000, false).map(|_| ())
    })
    .unwrap();
    let home = t.db.read(|c| queries::home(c, now)).unwrap();
    let ids: Vec<_> = home.shortcuts.iter().map(|i| i.key().1.to_string()).collect();
    assert_eq!(&ids[..3], &[lib.albums[1].id.clone(), lib.albums[0].id.clone(), lib.albums[2].id.clone()]);
    // Another round of album 0 today moves it to the front.
    for id in tracks_of(0) {
        local_play(&t.db, &id, now - 30, ContextKind::Album, Some(&lib.albums[0].id));
    }
    let home = t.db.read(|c| queries::home(c, now)).unwrap();
    assert_eq!(home.shortcuts[0].key().1, lib.albums[0].id);
}

// ---------------------------------------------------------------- search

fn search_library() -> FakeLibrary {
    let mut lib = FakeLibrary { sections: vec![fake::section("1", "u")], ..Default::default() };
    let bey = fake::artist("1", "Beyoncé");
    let beat = fake::artist("2", "The Beatles");
    let mut love_artist = fake::artist("3", "Love");
    love_artist.view_count = 0;
    let other = fake::artist("4", "Lovers Rock Collective");
    let sigur = fake::artist("5", "Sigur Rós");
    let lemon = fake::album("10", "Lemonade", &bey, 2016, Some("album"), &[]);
    let abbey = fake::album("11", "Abbey Road", &beat, 1969, Some("album"), &[]);
    let forever = fake::album("12", "Forever Changes", &love_artist, 1967, Some("album"), &[]);
    let takk = fake::album("13", "Takk...", &sigur, 2005, Some("album"), &[]);
    let lovely = fake::album("14", "Lovely Day", &other, 1999, Some("album"), &[]);
    let mut t = |id: &str, title: &str, al: &crate::model::Album, plays: u32| {
        let mut tr = fake::track(id, title, al, 1, 1);
        tr.view_count = plays;
        lib.tracks.push(tr);
    };
    t("100", "Formation", &lemon, 50);
    t("101", "Hold Up", &lemon, 3);
    t("102", "Come Together", &abbey, 9);
    t("103", "Something", &abbey, 1);
    t("104", "Alone Again Or", &forever, 0);
    t("105", "Hoppípolla", &takk, 4);
    t("106", "Lovely Day", &lovely, 0);
    t("107", "Love Song", &lovely, 0);
    t("108", "Love", &lovely, 0);
    t("109", "Glove Box", &lovely, 100);
    t("110", "Lovesick Blues", &lovely, 40);
    t("111", "Lovesick Blues (Live)", &lovely, 1);
    lib.albums = vec![lemon, abbey, forever, takk, lovely];
    lib.artists = vec![bey, beat, love_artist, other, sigur];
    lib.playlists.push((fake::playlist("p1", "Love Songs Mix", 2), vec!["107".into(), "108".into()]));
    lib
}

async fn search_db() -> Db {
    let t = open();
    let src = Arc::new(FakeSource::new(search_library()));
    sync(&t.db, &src, false).await;
    t
}

#[tokio::test]
async fn search_is_diacritic_insensitive_both_ways() {
    let t = search_db().await;
    let r = search::search(&t.db, "Beyonce", SearchFilter::All, None).unwrap();
    assert_eq!(r.artists[0].title, "Beyoncé");
    assert_eq!(r.top.as_ref().unwrap().kind, SearchKind::Artist);
    let r = search::search(&t.db, "hoppipolla", SearchFilter::Songs, None).unwrap();
    assert_eq!(r.songs[0].title, "Hoppípolla");
    let r = search::search(&t.db, "SIGUR RÓS", SearchFilter::Artists, None).unwrap();
    assert_eq!(r.artists[0].title, "Sigur Rós");
    assert_eq!(r.source, SearchSource::Local);
}

#[tokio::test]
async fn search_prefix_matching_and_track_artist_fields() {
    let t = search_db().await;
    let r = search::search(&t.db, "abb", SearchFilter::Albums, None).unwrap();
    assert_eq!(r.albums[0].title, "Abbey Road");
    // Tracks match on their album artist too.
    let r = search::search(&t.db, "beatles", SearchFilter::Songs, None).unwrap();
    assert_eq!(r.songs.len(), 2);
    // Multi-word: every word must match (as a prefix).
    let r = search::search(&t.db, "come tog", SearchFilter::Songs, None).unwrap();
    assert_eq!(r.songs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), vec!["Come Together"]);
}

#[tokio::test]
async fn search_ranks_exact_then_prefix_then_fuzzy_with_boosts() {
    let t = search_db().await;
    let r = search::search(&t.db, "love", SearchFilter::Songs, Some(10)).unwrap();
    let titles: Vec<&str> = r.songs.iter().map(|s| s.title.as_str()).collect();
    // Exact title first, then title prefixes boosted by plays
    // ("Lovesick Blues" 40 plays beats the 0-play ones), and word matches
    // elsewhere after. "Glove Box" (100 plays) only matches through its
    // album "Lovely Day", so it ranks below every title prefix.
    assert_eq!(titles[0], "Love");
    assert_eq!(titles[1], "Lovesick Blues");
    let pos = |t: &str| titles.iter().position(|x| *x == t).unwrap();
    assert!(pos("Love Song") < pos("Alone Again Or"), "title prefix beats album/artist match");
    assert!(pos("Lovely Day") < pos("Glove Box"), "plays never lift a weaker tier");

    // Top result across kinds: the exact-title artist "Love".
    let r = search::search(&t.db, "love", SearchFilter::All, None).unwrap();
    let top = r.top.unwrap();
    assert_eq!(top.kind, SearchKind::Artist);
    assert_eq!(top.artist.unwrap().title, "Love");
    assert_eq!(r.songs.len(), 4, "All shows four songs");
    assert_eq!(r.playlists[0].title, "Love Songs Mix");

    // Fuzzy matches rank below anything that matched as typed.
    let r = search::search(&t.db, "lovesik", SearchFilter::Songs, None).unwrap();
    assert_eq!(r.songs[0].title, "Lovesick Blues", "fuzzy hit with more plays first");

    // First keystrokes (1–2 chars): titles starting with it, most played first.
    let r = search::search(&t.db, "Lo", SearchFilter::Songs, Some(10)).unwrap();
    let titles: Vec<&str> = r.songs.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles[0], "Lovesick Blues");
    assert!(titles.iter().all(|t| t.to_lowercase().starts_with("lo")), "{titles:?}");
    let r = search::search(&t.db, "b", SearchFilter::All, None).unwrap();
    assert_eq!(r.artists[0].title, "Beyoncé");
    assert!(r.artists.iter().all(|a| a.title != "The Beatles"), "short queries match title starts only");
}

#[tokio::test]
async fn search_tolerates_one_typo_on_long_words_only() {
    let t = search_db().await;
    for typo in ["beatls", "beatels", "beetles", "beatlesz"] {
        let r = search::search(&t.db, typo, SearchFilter::Artists, None).unwrap();
        assert_eq!(r.artists.first().map(|a| a.title.as_str()), Some("The Beatles"), "{typo}");
    }
    let r = search::search(&t.db, "lemonaed", SearchFilter::Albums, None).unwrap();
    assert_eq!(r.albums[0].title, "Lemonade");
    // Two edits: no match.
    assert!(search::search(&t.db, "btaels", SearchFilter::Artists, None).unwrap().artists.is_empty());
    // Words under 5 letters forgive no typos: "lova" ≠ "love".
    assert!(search::search(&t.db, "lova", SearchFilter::All, None).unwrap().songs.is_empty());
}

fn forgiving_library() -> FakeLibrary {
    let mut lib = FakeLibrary { sections: vec![fake::section("1", "u")], ..Default::default() };
    let marlo = fake::artist("1", "Marlo Vane");
    let risky = fake::artist("2", "Ri$ky Tide");
    let cafe = fake::artist("3", "Café Señorita");
    let skies = fake::album("10", "Open Skies", &marlo, 2001, Some("album"), &[]);
    let vol = fake::album("11", "Volume 4", &risky, 2010, Some("album"), &[]);
    let late = fake::album("12", "Late Hours", &cafe, 2015, Some("album"), &[]);
    let mut t = |id: &str, title: &str, al: &crate::model::Album, plays: u32| {
        let mut tr = fake::track(id, title, al, 1, 1);
        tr.view_count = plays;
        lib.tracks.push(tr);
    };
    t("100", "Through the High Grass", &skies, 2);
    t("101", "Thru the Nite", &skies, 1);
    t("102", "Light Tho", &skies, 0);
    t("103", "Part II", &skies, 0);
    t("104", "No. 9 Dream", &skies, 0);
    t("105", "Me & You", &skies, 0);
    t("106", "Talk w/ Me", &skies, 0);
    t("107", "Lullaby ft. Somebody", &skies, 0);
    t("108", "Rock 'n' Roll Heart", &skies, 0);
    t("109", "U Got It", &skies, 0);
    t("110", "Your Turn", &skies, 0);
    t("111", "Don't Look Down", &vol, 0);
    t("112", "Dont Stop Now", &vol, 0);
    t("113", "Sky-High", &vol, 0);
    t("114", "Wanderlust", &vol, 5);
    t("115", "Wanderlast", &vol, 0);
    t("116", "And They Came Back / This One Here", &late, 0);
    t("117", "This Is One Way Here", &late, 10);
    for (i, title) in ["Lamp Light", "Lamplighter", "Lamp Post", "Lamps", "Lamp Oil", "Lampshade"].iter().enumerate() {
        t(&format!("12{i}"), title, &late, 0);
    }
    t("130", "Lamb Chop", &late, 100);
    t("140", "Tonight", &late, 0);
    t("141", "Tonite", &late, 0);
    t("150", "Mother Reach My Arms Pt. 1", &late, 0);
    t("151", "Arms Reach", &late, 0);
    t("160", "Shore Lights", &late, 0);
    t("161", "Harbor Song / Shore Lights - Live", &late, 0);
    t("170", "Glass Harbor", &late, 0);
    t("172", "Glass Harbor Lights (Demo)", &late, 0);
    {
        let mut tr = fake::track("171", "Glass Harbor Lights", &late, 1, 1);
        tr.view_count = 900;
        tr.user_rating = Some(10.0);
        lib.tracks.push(tr);
    }
    lib.albums = vec![skies, vol, late];
    lib.artists = vec![marlo, risky, cafe];
    lib
}

async fn forgiving_db() -> Db {
    let t = open();
    let src = Arc::new(FakeSource::new(forgiving_library()));
    sync(&t.db, &src, false).await;
    t
}

fn song_titles(r: &SearchResults) -> Vec<&str> {
    r.songs.iter().map(|s| s.title.as_str()).collect()
}

fn first_song(db: &Db, q: &str) -> Option<String> {
    search::search(&db.db, q, SearchFilter::Songs, Some(20)).unwrap().songs.first().map(|s| s.title.clone())
}

#[tokio::test]
async fn search_shorthand_finds_the_long_spelling() {
    let t = forgiving_db().await;
    for q in ["thru the high", "Thru The High Grass", "grass thru"] {
        assert_eq!(first_song(&t, q).as_deref(), Some("Through the High Grass"), "{q}");
    }
    // Both spellings match; the one spelled as typed comes first.
    let r = search::search(&t.db, "thru the", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r)[..2], ["Thru the Nite", "Through the High Grass"]);
}

#[tokio::test]
async fn search_spelling_variants_match_both_ways() {
    let t = forgiving_db().await;
    let cases: &[(&str, &str)] = &[
        ("through the nite", "Thru the Nite"),
        ("thru the night", "Thru the Nite"),
        ("through the high", "Through the High Grass"),
        ("lite tho", "Light Tho"),
        ("light though", "Light Tho"),
        ("part 2", "Part II"),
        ("pt two", "Part II"),
        ("number nine dream", "No. 9 Dream"),
        ("no. 9", "No. 9 Dream"),
        ("me and you", "Me & You"),
        ("me n u", "Me & You"),
        ("me + you", "Me & You"),
        ("talk with me", "Talk w/ Me"),
        ("lullaby featuring somebody", "Lullaby ft. Somebody"),
        ("lullaby feat somebody", "Lullaby ft. Somebody"),
        ("rock and roll heart", "Rock 'n' Roll Heart"),
        ("rock n roll", "Rock 'n' Roll Heart"),
        ("you got it", "U Got It"),
        ("ur turn", "Your Turn"),
    ];
    for (q, want) in cases {
        assert_eq!(first_song(&t, q).as_deref(), Some(*want), "{q}");
    }
    for q in ["volume four", "vol 4", "vol iv"] {
        let r = search::search(&t.db, q, SearchFilter::Albums, None).unwrap();
        assert_eq!(r.albums.first().map(|a| a.title.as_str()), Some("Volume 4"), "{q}");
    }
}

#[tokio::test]
async fn search_ignores_accents_apostrophes_dollar_and_hyphens() {
    let t = forgiving_db().await;
    for q in ["dont look down", "don't look down", "Don’t look"] {
        assert_eq!(first_song(&t, q).as_deref(), Some("Don't Look Down"), "{q}");
    }
    assert_eq!(first_song(&t, "don't stop now").as_deref(), Some("Dont Stop Now"));
    for q in ["skyhigh", "sky high", "sky-high"] {
        assert_eq!(first_song(&t, q).as_deref(), Some("Sky-High"), "{q}");
    }
    for q in ["risky tide", "ri$ky", "RISKY"] {
        let r = search::search(&t.db, q, SearchFilter::Artists, None).unwrap();
        assert_eq!(r.artists.first().map(|a| a.title.as_str()), Some("Ri$ky Tide"), "{q}");
    }
    let r = search::search(&t.db, "cafe senorita", SearchFilter::Artists, None).unwrap();
    assert_eq!(r.artists[0].title, "Café Señorita");
}

#[tokio::test]
async fn search_forgives_typos_below_exact_matches() {
    let t = forgiving_db().await;
    // Wrong, missing, extra and swapped letters.
    for q in ["wanderlusk", "wandrlust", "wanderluust", "wanderlsut"] {
        let r = search::search(&t.db, q, SearchFilter::Songs, None).unwrap();
        assert!(song_titles(&r).contains(&"Wanderlust"), "{q}: {:?}", song_titles(&r));
        assert!(r.corrected.is_some(), "{q}");
    }
    let r = search::search(&t.db, "wanderlsut", SearchFilter::Songs, None).unwrap();
    assert_eq!(r.corrected.as_deref(), Some("wanderlust"));
    // One typo only.
    assert!(search::search(&t.db, "wnaderlsut", SearchFilter::Songs, None).unwrap().songs.is_empty());
    // Words that are in the library as typed are not corrected, even with
    // few results ("Wanderlast" is a real title one letter from another).
    let r = search::search(&t.db, "wanderlast", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r), vec!["Wanderlast"]);
    assert_eq!(r.corrected, None);
    // The setting turns the typo fallback off.
    t.db.set_search_typos(false);
    assert!(search::search(&t.db, "wanderlsut", SearchFilter::Songs, None).unwrap().songs.is_empty());
}

#[tokio::test]
async fn search_adds_no_typo_matches_when_exact_ones_are_plenty() {
    let t = forgiving_db().await;
    let r = search::search(&t.db, "lamp", SearchFilter::Songs, Some(20)).unwrap();
    let titles = song_titles(&r);
    assert!(titles.len() >= search::FUZZY_BELOW, "{titles:?}");
    assert!(!titles.contains(&"Lamb Chop"), "{titles:?}");
    assert_eq!(r.corrected, None);
    // A real library word is not corrected to another one.
    let r = search::search(&t.db, "lamb", SearchFilter::Songs, Some(20)).unwrap();
    assert_eq!(song_titles(&r), vec!["Lamb Chop"]);
}

#[tokio::test]
async fn search_drops_a_word_only_when_nothing_matches_them_all() {
    let t = forgiving_db().await;
    let r = search::search(&t.db, "high grass zebra", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r), vec!["Through the High Grass"]);
    // Words in any order still need all of them.
    let r = search::search(&t.db, "grass high", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r), vec!["Through the High Grass"]);
    // Nothing found with every word: the one-word-dropped matches.
    let r = search::search(&t.db, "wanderlust zebra", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r)[0], "Wanderlust");
}

#[tokio::test]
async fn search_puts_the_typed_spelling_above_its_variant() {
    let t = forgiving_db().await;
    let r = search::search(&t.db, "tonite", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r), vec!["Tonite", "Tonight"]);
    let r = search::search(&t.db, "tonight", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r), vec!["Tonight", "Tonite"]);
}

#[tokio::test]
async fn search_small_and_unknown_words_do_not_block_a_match() {
    let t = forgiving_db().await;
    assert_eq!(first_song(&t, "the lamp of post").as_deref(), Some("Lamp Post"));
    assert_eq!(first_song(&t, "grass high the").as_deref(), Some("Through the High Grass"));
    // A word the library has nowhere is left out.
    let r = search::search(&t.db, "zebra qqzzx wanderlust", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r)[0], "Wanderlust");
}

#[tokio::test]
async fn search_ranks_mid_title_words_medley_parts_and_plays() {
    let t = forgiving_db().await;
    // Words mid-title count, and in a row beat the same words scattered.
    assert_eq!(first_song(&t, "reach my arms").as_deref(), Some("Mother Reach My Arms Pt. 1"));
    // A plain query: the song titled exactly that first, the medley after.
    let r = search::search(&t.db, "shore lights", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r)[..2], ["Shore Lights", "Harbor Song / Shore Lights - Live"]);
    // An extra word that hits a later part lifts that version.
    assert_eq!(first_song(&t, "shore lights live").as_deref(), Some("Harbor Song / Shore Lights - Live"));
    // A much played, top rated song with every word passes a never played
    // exact title; an unplayed one does not.
    let r = search::search(&t.db, "glass harbor", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r), vec!["Glass Harbor Lights", "Glass Harbor", "Glass Harbor Lights (Demo)"]);
}

#[tokio::test]
async fn search_matches_a_part_of_a_two_part_title() {
    let t = forgiving_db().await;
    // "/" separates words; a query matching one part ranks like a title
    // match, above the same words scattered (with a few more plays).
    let r = search::search(&t.db, "this one here", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r), vec!["And They Came Back / This One Here", "This Is One Way Here"]);
    let r = search::search(&t.db, "came back", SearchFilter::Songs, None).unwrap();
    assert_eq!(song_titles(&r)[0], "And They Came Back / This One Here");
    let r = search::search(&t.db, "this one here", SearchFilter::All, None).unwrap();
    assert_eq!(r.top.unwrap().track.unwrap().title, "And They Came Back / This One Here");
}

#[tokio::test]
async fn search_empty_long_and_hostile_queries_are_safe() {
    let t = search_db().await;
    for q in ["", "   ", "\"", "*", "-", "--", "NEAR(", "NEAR(love abbey)", "love OR", "AND", "title:love",
        "\"love\" NOT x", "((((", "^", "love*", "-love", "'; DROP TABLE tracks; --", "\u{0}"]
    {
        let r = search::search(&t.db, q, SearchFilter::All, None);
        assert!(r.is_ok(), "query {q:?} errored: {:?}", r.err());
    }
    let r = search::search(&t.db, "", SearchFilter::All, None).unwrap();
    assert!(r.top.is_none() && r.songs.is_empty() && r.artists.is_empty());
    let long = "love ".repeat(2_000); // 10k chars
    let r = search::search(&t.db, &long, SearchFilter::All, None).unwrap();
    assert!(r.took_ms < 500.0);
    let r = search::search(&t.db, &"x".repeat(10_000), SearchFilter::All, None).unwrap();
    assert!(r.songs.is_empty());
    // Operators are treated as words: "love OR" needs a word starting "or".
    let r = search::search(&t.db, "alone or", SearchFilter::Songs, None).unwrap();
    assert_eq!(r.songs[0].title, "Alone Again Or");
    assert_eq!(count(&t.db, "tracks"), 12);
}

#[tokio::test]
async fn search_filter_limits_kinds() {
    let t = search_db().await;
    let r = search::search(&t.db, "love", SearchFilter::Albums, None).unwrap();
    assert!(r.songs.is_empty() && r.artists.is_empty() && r.playlists.is_empty() && r.top.is_none());
    assert!(!r.albums.is_empty());
    let r = search::search(&t.db, "love", SearchFilter::Songs, Some(2)).unwrap();
    assert_eq!(r.songs.len(), 2);
}

// ---------------------------------------------------------------- server fallback

#[derive(Default)]
struct FakeRemote {
    hits: RemoteHits,
    fail: bool,
    calls: Mutex<Vec<(String, Option<String>)>>,
}

#[async_trait::async_trait]
impl RemoteSearch for FakeRemote {
    async fn search(&self, query: &str, section_id: Option<&str>, _limit: u32) -> crate::error::AppResult<RemoteHits> {
        self.calls.lock().push((query.to_string(), section_id.map(str::to_string)));
        if self.fail {
            return Err(crate::error::AppError::Network("offline".into()));
        }
        Ok(self.hits.clone())
    }
}

fn remote_hits() -> RemoteHits {
    let lib = search_library();
    RemoteHits {
        artists: lib.artists.clone(),
        albums: lib.albums.clone(),
        // Server order is not ours: reversed on purpose.
        tracks: lib.tracks.iter().rev().cloned().collect(),
        playlists: lib.playlists.iter().map(|(p, _)| p.clone()).collect(),
    }
}

#[tokio::test]
async fn search_uses_server_until_first_sync_finishes() {
    let t = open();
    // A first sync has started (section known) but not finished.
    t.db.write(|c| {
        store::put_sync_state(
            c,
            &store::SyncState {
                section_id: "7".into(),
                section_uuid: "u".into(),
                section_title: None,
                last_sync_at: None,
                last_full_sync_at: None,
                generation: 1,
            },
        )
    })
    .unwrap();
    let remote = Arc::new(FakeRemote { hits: remote_hits(), ..Default::default() });
    t.db.set_remote_search(remote.clone());

    let r = search::search_with_fallback(&t.db, "love", SearchFilter::All, Some(10)).await.unwrap();
    assert_eq!(r.source, SearchSource::Server);
    // Same ranking rules as the local index.
    let top = r.top.unwrap();
    assert_eq!(top.kind, SearchKind::Artist);
    assert_eq!(top.artist.unwrap().title, "Love");
    // An explicit limit applies to every kind, songs included (the results page mixes them).
    assert_eq!(r.songs.len(), 10);
    assert_eq!(r.songs[0].title, "Love");
    assert_eq!(remote.calls.lock()[0], ("love".to_string(), Some("7".to_string())));

    // Filters apply to server results too.
    let r = search::search_with_fallback(&t.db, "love", SearchFilter::Albums, None).await.unwrap();
    assert!(r.songs.is_empty() && r.artists.is_empty() && r.playlists.is_empty() && r.top.is_none());
    assert_eq!(r.albums[0].title, "Lovely Day");
}

#[tokio::test]
async fn search_uses_cache_once_synced_and_never_asks_server() {
    let t = search_db().await;
    let remote = Arc::new(FakeRemote { hits: remote_hits(), ..Default::default() });
    t.db.set_remote_search(remote.clone());
    let r = search::search_with_fallback(&t.db, "love", SearchFilter::All, None).await.unwrap();
    assert_eq!(r.source, SearchSource::Local);
    assert!(remote.calls.lock().is_empty());
}

#[tokio::test]
async fn server_search_failure_falls_back_to_cache() {
    let t = open();
    let remote = Arc::new(FakeRemote { fail: true, ..Default::default() });
    t.db.set_remote_search(remote.clone());
    let r = search::search_with_fallback(&t.db, "love", SearchFilter::All, None).await.unwrap();
    assert_eq!(r.source, SearchSource::Local);
    assert!(r.songs.is_empty());
    assert_eq!(remote.calls.lock().len(), 1);
}

#[tokio::test]
async fn empty_and_long_queries_are_safe_with_fallback() {
    let t = open();
    let remote = Arc::new(FakeRemote { hits: remote_hits(), ..Default::default() });
    t.db.set_remote_search(remote.clone());
    for q in ["", "   ", "--"] {
        let r = search::search_with_fallback(&t.db, q, SearchFilter::All, None).await.unwrap();
        assert!(r.top.is_none() && r.songs.is_empty());
    }
    assert!(remote.calls.lock().is_empty(), "nothing to search for, no server call");
    let long = "love ".repeat(2_000);
    let r = search::search_with_fallback(&t.db, &long, SearchFilter::All, None).await.unwrap();
    assert_eq!(r.source, SearchSource::Server);
    assert!(remote.calls.lock()[0].0.chars().count() <= 200, "query is clipped before it leaves");
}

#[test]
fn rank_remote_orders_exact_then_prefix_then_other() {
    let r = search::rank_remote("love", SearchFilter::Songs, Some(20), remote_hits(), now_s());
    let titles: Vec<&str> = r.songs.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles[0], "Love");
    assert_eq!(titles[1], "Lovesick Blues");
    let pos = |t: &str| titles.iter().position(|x| *x == t).unwrap();
    assert!(pos("Lovely Day") < pos("Glove Box"));
    assert_eq!(r.source, SearchSource::Server);
}

#[tokio::test]
async fn search_recents_add_dedupe_remove_cap_and_clear() {
    let t = search_db().await;
    t.db.write(|c| {
        recent_add(c, SearchKind::Album, "10")?;
        recent_add(c, SearchKind::Artist, "2")?;
        recent_add(c, SearchKind::Track, "100")?;
        recent_add(c, SearchKind::Playlist, "p1")?;
        recent_add(c, SearchKind::Album, "10")?; // opened again: moves to the top
        recent_add(c, SearchKind::Album, "gone")?; // not in the library
        recent_add(c, SearchKind::Album, "")?;
        Ok(())
    })
    .unwrap();
    let l = t.db.read(recent_list).unwrap();
    let keys: Vec<(SearchKind, String)> = l
        .iter()
        .map(|h| {
            let id = match h.kind {
                SearchKind::Artist => h.artist.as_ref().unwrap().id.clone(),
                SearchKind::Album => h.album.as_ref().unwrap().id.clone(),
                SearchKind::Track => h.track.as_ref().unwrap().id.clone(),
                SearchKind::Playlist => h.playlist.as_ref().unwrap().id.clone(),
            };
            (h.kind, id)
        })
        .collect();
    assert_eq!(
        keys,
        vec![
            (SearchKind::Album, "10".to_string()),
            (SearchKind::Playlist, "p1".to_string()),
            (SearchKind::Track, "100".to_string()),
            (SearchKind::Artist, "2".to_string()),
        ],
        "newest first, one entry per item, missing items skipped"
    );
    assert_eq!(l[0].album.as_ref().unwrap().title, "Lemonade");

    t.db.write(|c| recent_remove(c, SearchKind::Playlist, "p1")).unwrap();
    assert_eq!(t.db.read(recent_list).unwrap().len(), 3);

    t.db.write(|c| {
        for i in 0..30 {
            recent_add(c, SearchKind::Track, &format!("x{i}"))?;
        }
        Ok(())
    })
    .unwrap();
    let n: i64 = t.db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM search_recents", [], |r| r.get(0))?)).unwrap();
    assert_eq!(n, search::RECENTS_MAX);
    t.db.write(|c| recent_clear(c)).unwrap();
    assert!(t.db.read(recent_list).unwrap().is_empty());
}

#[tokio::test]
async fn search_completions_are_whole_names_starting_with_the_words() {
    let t = search_db().await;
    let c = |q: &str| t.db.read(|c| search::completions(c, q, search::COMPLETIONS_MAX)).unwrap();
    // Most played first, then shorter names; the artist "Love" and the song
    // "Love" give one completion, and four is the most.
    assert_eq!(c("love"), vec!["Lovesick Blues", "Lovesick Blues (Live)", "Love", "Love Song"]);
    // Several words, the last one a prefix; diacritics and case don't matter.
    assert_eq!(c("lovesick bl"), vec!["Lovesick Blues", "Lovesick Blues (Live)"]);
    assert_eq!(c("BEYONCE"), vec!["Beyoncé"]);
    assert_eq!(c("hoppipol"), vec!["Hoppípolla"]);
    // Only names that start with the words: "Glove Box" and "The Beatles" don't.
    assert!(c("glo").contains(&"Glove Box".to_string()));
    assert!(c("beat").is_empty());
    assert!(c("").is_empty());
    assert!(c("\"*:(").is_empty());
}

// ---------------------------------------------------------------- listens

fn play(key: &str, at: i64, duration_ms: u64, offline: bool) -> PlayStart {
    PlayStart {
        rating_key: key.into(),
        part_key: Some(format!("/library/parts/{key}/1/f.flac")),
        started_at: at,
        duration_ms,
        context_kind: Some(ContextKind::Playlist),
        context_id: Some("p1".into()),
        device: Some("desk".into()),
        offline,
    }
}

#[test]
fn listen_log_records_skips_and_scrobbles_once() {
    let t = open();
    t.db.write(|c| {
        // A skip at 12 s: logged as a skip, never scrobbled.
        let id = listens::start_play(c, &play("1", 1_000, 300_000, false))?;
        let u = listens::progress(c, id, 12_000)?;
        assert!(!u.scrobble_now);
        let u = listens::end_play(c, id, 13_000, 12_000, 12_500, false)?;
        assert!(u.skipped && !u.scrobble_now);
        let row = listens::get_play(c, id)?.unwrap();
        assert!(row.skipped && !row.completed && !row.scrobbled);
        assert_eq!(row.skipped_at_ms, Some(12_500));
        assert_eq!(row.context_kind.as_deref(), Some("playlist"));

        // A full play: scrobble fires exactly once at the threshold.
        let id = listens::start_play(c, &play("2", 2_000, 300_000, false))?;
        assert!(!listens::progress(c, id, 100_000)?.scrobble_now);
        assert!(listens::progress(c, id, 150_000)?.scrobble_now);
        assert!(!listens::progress(c, id, 200_000)?.scrobble_now);
        let u = listens::end_play(c, id, 302_000, 300_000, 300_000, true)?;
        assert!(!u.skipped && !u.scrobble_now);
        let row = listens::get_play(c, id)?.unwrap();
        assert!(row.completed && row.scrobbled && !row.skipped);
        assert_eq!(row.ms_listened, 300_000);

        // Stopped after 35 s of a 60 s track: not a skip, threshold 30 s → scrobble at end.
        let id = listens::start_play(c, &play("3", 3_000, 60_000, false))?;
        let u = listens::end_play(c, id, 40_000, 35_000, 35_000, false)?;
        assert!(!u.skipped && u.scrobble_now);
        Ok(())
    })
    .unwrap();
    assert!(t.db.write(|c| listens::progress(c, 999, 1)).is_err());
}

#[test]
fn offline_reports_replay_in_original_order() {
    let t = open();
    let pending = t
        .db
        .write(|c| {
            let p1 = listens::start_play(c, &play("1", 10_000, 200_000, true))?;
            let p2 = listens::start_play(c, &play("2", 20_000, 200_000, true))?;
            let rep = |play_id, kind, key: &str, state: Option<&str>, at| NewReport {
                play_id: Some(play_id),
                kind,
                rating_key: key.into(),
                state: state.map(Into::into),
                time_ms: Some(0),
                duration_ms: Some(200_000),
                at,
            };
            // Enqueued out of order; replay must follow `at`, then insertion.
            listens::enqueue_report(c, &rep(p2, ReportKind::Timeline, "2", Some("playing"), 20_000))?;
            listens::enqueue_report(c, &rep(p1, ReportKind::Timeline, "1", Some("playing"), 10_000))?;
            listens::enqueue_report(c, &rep(p1, ReportKind::Scrobble, "1", None, 15_000))?;
            listens::enqueue_report(c, &rep(p1, ReportKind::Timeline, "1", Some("stopped"), 15_000))?;
            listens::pending_reports(c, 100)
        })
        .unwrap();
    let order: Vec<_> = pending.iter().map(|p| (p.at, p.kind, p.state.clone())).collect();
    assert_eq!(
        order,
        vec![
            (10_000, ReportKind::Timeline, Some("playing".into())),
            (15_000, ReportKind::Scrobble, None),
            (15_000, ReportKind::Timeline, Some("stopped".into())),
            (20_000, ReportKind::Timeline, Some("playing".into())),
        ]
    );
    // Send the first two; the rest stay pending, in order.
    let sent: Vec<i64> = pending.iter().take(2).map(|p| p.id).collect();
    t.db.write(|c| listens::mark_sent(c, &sent, 100_000)).unwrap();
    let left = t.db.read(|c| listens::pending_reports(c, 100)).unwrap();
    assert_eq!(left.iter().map(|p| p.at).collect::<Vec<_>>(), vec![15_000, 20_000]);
    let p1 = t.db.read(|c| listens::get_play(c, pending[0].play_id.unwrap())).unwrap().unwrap();
    assert!(p1.reported_to_plex && p1.offline, "scrobble sent → play marked reported");
}

// ---------------------------------------------------------------- benchmarks

mod bench {
    use super::*;
    use std::time::Instant;

    const SYLLABLES: &[&str] = &[
        "ka", "lo", "mi", "ra", "ne", "so", "tu", "vi", "an", "el", "or", "us", "be", "da", "fe", "go", "hi", "jo",
        "ké", "lu", "ma", "no", "pé", "qu", "ri", "sa", "té", "va", "wo", "xy", "zé", "bri", "cla", "dre", "fro",
    ];

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn word(&mut self) -> String {
            let n = 2 + self.below(3);
            let mut w: String = (0..n).map(|_| SYLLABLES[self.below(SYLLABLES.len())]).collect();
            if let Some(f) = w.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            w
        }
        fn title(&mut self, max_words: usize) -> String {
            let n = 1 + self.below(max_words);
            (0..n).map(|_| self.word()).collect::<Vec<_>>().join(" ")
        }
    }

    /// 2,000 artists, 10,000 albums, 100,000 tracks.
    fn big_library() -> FakeLibrary {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let mut lib = FakeLibrary { sections: vec![fake::section("1", "u")], ..Default::default() };
        let mut next = 1u64;
        let mut id = || {
            next += 1;
            next.to_string()
        };
        for _ in 0..2_000 {
            let mut ar = fake::artist(&id(), &rng.title(2));
            ar.view_count = rng.below(500) as u32;
            for _ in 0..5 {
                let fmt = ["album", "ep", "single", "album"][rng.below(4)];
                let subs: &[&str] = [&[][..], &["Live"][..], &["Demo"][..], &[][..], &[][..]][rng.below(5)];
                let mut al = fake::album(&id(), &rng.title(3), &ar, 1960 + rng.below(66) as i32, Some(fmt), subs);
                al.track_count = 10;
                al.view_count = rng.below(200) as u32;
                al.added_at = 1_500_000_000 + rng.below(200_000_000) as i64;
                for n in 0..10 {
                    let mut tr = fake::track(&id(), &rng.title(4), &al, 1 + (n / 8) as u32, n as u32 + 1);
                    tr.view_count = rng.below(100) as u32;
                    if rng.below(5) == 0 {
                        tr.track_artist = Some(rng.title(2));
                    }
                    lib.tracks.push(tr);
                }
                lib.albums.push(al);
            }
            lib.artists.push(ar);
        }
        for i in 0..200 {
            let items: Vec<String> = (0..50).map(|_| lib.tracks[rng.below(lib.tracks.len())].id.clone()).collect();
            lib.playlists.push((fake::playlist(&format!("pl{i}"), &rng.title(3), 50), items));
        }
        let now = now_s();
        for _ in 0..1_000 {
            let k = lib.tracks[rng.below(lib.tracks.len())].id.clone();
            lib.history.push(HistoryEntry { track_id: k, viewed_at: now - rng.below(40 * 86400) as i64 });
        }
        lib
    }

    fn p95(mut v: Vec<f64>) -> (f64, f64, f64) {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let at = |q: f64| v[((v.len() as f64 - 1.0) * q).round() as usize];
        (at(0.5), at(0.95), *v.last().unwrap())
    }

    /// Run with: `cargo test --release db::tests::bench -- --ignored --nocapture`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "benchmark: ~100k tracks; run explicitly"]
    async fn bench_library_queries_and_search() {
        let t = open();
        let lib = big_library();
        let src = Arc::new(FakeSource::new(lib.clone()));
        let started = Instant::now();
        sync(&t.db, &src, false).await;
        println!("sync of 2k artists / 10k albums / 100k tracks: {:.1} s", started.elapsed().as_secs_f64());
        assert_eq!(count(&t.db, "tracks"), 100_000);

        let mut rng = Rng(42);
        let now = now_s();
        let mut report = Vec::new();
        let mut time = |name: &str, n: usize, f: &mut dyn FnMut(usize)| {
            f(0); // warm statement cache
            let samples: Vec<f64> = (0..n)
                .map(|i| {
                    let s = Instant::now();
                    f(i);
                    s.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            let (p50, p95, max) = p95(samples);
            println!("{name:<28} p50 {p50:>7.2} ms   p95 {p95:>7.2} ms   max {max:>7.2} ms");
            report.push((name.to_string(), p95));
        };

        let album_ids: Vec<String> = lib.albums.iter().map(|a| a.id.clone()).collect();
        let artist_ids: Vec<String> = lib.artists.iter().map(|a| a.id.clone()).collect();
        let db = &t.db;
        time("home", 50, &mut |_| {
            db.read(|c| queries::home(c, now)).unwrap();
        });
        for sort in [AlbumSort::Name, AlbumSort::Artist, AlbumSort::Year, AlbumSort::Recent, AlbumSort::Plays] {
            time(&format!("albums {sort:?}"), 50, &mut |i| {
                db.read(|c| queries::albums(c, sort, (i as u32 * 397) % 9_900, 100, None)).unwrap();
            });
        }
        time("albums section filter", 50, &mut |i| {
            db.read(|c| queries::albums(c, AlbumSort::Name, (i as u32 * 37) % 1_000, 100, Some(ReleaseSection::Live)))
                .unwrap();
        });
        for sort in [ArtistSort::Name, ArtistSort::Recent, ArtistSort::Plays] {
            time(&format!("artists {sort:?}"), 50, &mut |i| {
                db.read(|c| queries::artists(c, sort, (i as u32 * 37) % 1_900, 100)).unwrap();
            });
        }
        time("playlists", 50, &mut |_| {
            db.read(queries::playlists).unwrap();
        });
        time("artist_page", 200, &mut |i| {
            db.read(|c| queries::artist_page(c, &artist_ids[(i * 7) % artist_ids.len()])).unwrap();
        });
        time("album_page", 200, &mut |i| {
            db.read(|c| queries::album_page(c, &album_ids[(i * 13) % album_ids.len()])).unwrap();
        });
        time("playlist_page", 100, &mut |i| {
            db.read(|c| queries::playlist_page(c, &format!("pl{}", i % 200))).unwrap();
        });

        // Search: a mix of 1-3 char prefixes, full words, multi-word,
        // diacritic-free spellings and typos taken from real titles.
        let mut queries_: Vec<String> = Vec::new();
        for _ in 0..300 {
            let tr = &lib.tracks[rng.below(lib.tracks.len())];
            let norm = search::normalize(&tr.title);
            let words: Vec<&str> = norm.split(' ').collect();
            let w = words[0];
            let q = match rng.below(6) {
                0 => w.chars().take(1 + rng.below(3)).collect(),
                1 => w.to_string(),
                2 => words.join(" "),
                3 => format!("{} {}", search::normalize(&tr.artist_title), w),
                4 if w.chars().count() >= 6 => {
                    let mut c: Vec<char> = w.chars().collect();
                    c.remove(2);
                    c.into_iter().collect()
                }
                _ => tr.title.clone(),
            };
            queries_.push(q);
        }
        let mut lat = Vec::new();
        let mut sizes = Vec::new();
        time("search (all)", 300, &mut |i| {
            let r = search::search(db, &queries_[i % queries_.len()], SearchFilter::All, None).unwrap();
            lat.push((r.took_ms, queries_[i % queries_.len()].clone()));
            sizes.push(r.songs.len());
        });
        time("search (songs filter)", 150, &mut |i| {
            search::search(db, &queries_[(i * 7) % queries_.len()], SearchFilter::Songs, None).unwrap();
        });
        time("search 1-char prefix", 30, &mut |i| {
            let q = ["k", "l", "m", "b", "s", "a"][i % 6];
            search::search(db, q, SearchFilter::All, None).unwrap();
        });
        // Fallbacks: a typo in a long title word (the typo pass runs), a
        // word that is nowhere (typo pass, then one word dropped) and
        // shorthand that expands to its spellings.
        let mut typos: Vec<String> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        for _ in 0..100 {
            let tr = &lib.tracks[rng.below(lib.tracks.len())];
            let norm = search::normalize(&tr.title);
            let w = norm.split(' ').max_by_key(|w| w.len()).unwrap_or_default();
            let mut c: Vec<char> = w.chars().collect();
            if c.len() >= 5 {
                c.swap(1, 2);
            }
            typos.push(c.into_iter().collect());
            missing.push(format!("{norm} qqzzx"));
        }
        let mut fuzzy_hits = 0;
        time("search typo fallback", 100, &mut |i| {
            let r = search::search(db, &typos[i % typos.len()], SearchFilter::All, None).unwrap();
            fuzzy_hits += usize::from(r.corrected.is_some() || !r.songs.is_empty());
        });
        time("search missing-word fallback", 100, &mut |i| {
            search::search(db, &missing[i % missing.len()], SearchFilter::All, None).unwrap();
        });
        time("search shorthand variants", 60, &mut |i| {
            let q = ["thru the nite", "pt 2 vol iv", "me & u", "don't stop", "ft somebody", "no. 9"][i % 6];
            search::search(db, q, SearchFilter::All, None).unwrap();
        });
        println!("typo queries with results: {fuzzy_hits}/101");
        let mut slow = lat.clone();
        slow.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        println!("slowest searches: {:?}", &slow[..8]);
        let hits = sizes.iter().filter(|n| **n > 0).count();
        println!("search queries with song hits: {hits}/{}", sizes.len());

        for (name, p) in &report {
            let budget = if name.starts_with("search") { 50.0 } else { 10.0 };
            assert!(*p < budget, "{name}: p95 {p:.2} ms ≥ {budget} ms");
        }
    }
}

#[tokio::test]
async fn a_playlist_the_server_cannot_list_is_dropped_not_fatal() {
    let t = open();
    let mut lib = fake::generated(2, 2, 3);
    let ids: Vec<String> = lib.tracks.iter().map(|t| t.id.clone()).collect();
    lib.playlists.push((fake::playlist("ok", "Fine", 1), vec![ids[0].clone()]));
    lib.playlists.push((fake::playlist("orphan", "From a deleted library", 5), vec![]));
    lib.broken_playlists.push("orphan".into());
    let src = Arc::new(FakeSource::new(lib));
    sync(&t.db, &src, false).await;
    let lists = t.db.read(queries::playlists).unwrap();
    assert_eq!(lists.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["ok"]);
    assert_eq!(count(&t.db, "tracks"), 12, "the rest of the sync still lands");
}

// ------------------------------------------------- popular: which copy

/// An artist with the same songs on a studio album, a live album, an
/// unreleased tape and a session compilation, plus an interlude.
fn copies_library() -> FakeLibrary {
    let mut lib = FakeLibrary { sections: vec![fake::section("1", "u")], ..Default::default() };
    let ar = fake::artist("a1", "Odessa Wren");
    let albums = [
        fake::album("debut", "First Light", &ar, 2004, Some("Album"), &[]),
        fake::album("fifth", "Fifth Room", &ar, 2010, Some("Album"), &[]),
        fake::album("strings", "Live with Strings", &ar, 2006, Some("Album"), &["Live"]),
        fake::album("tape", "Room Tapes", &ar, 2009, Some("Album"), &[]),
        fake::album("sessions", "Radio Sessions", &ar, 2007, Some("Album"), &["Compilation"]),
    ];
    // id, title, album, plays, seconds
    let tracks: &[(&str, &str, &str, u32, u64)] = &[
        ("walk-live", "Walking Song", "strings", 30, 260),
        ("walk", "Walking Song", "debut", 0, 250),
        ("power-tape", "Power Song", "tape", 25, 290),
        ("power", "Power Song", "fifth", 1, 292),
        ("gold-session", "Gold Song (Radio Sessions)", "sessions", 40, 230),
        ("gold", "Gold Song", "debut", 2, 210),
        ("lights-interlude", "Bright Lights (Interlude)", "fifth", 50, 60),
        ("lights", "Bright Lights (feat. Guest Singer)", "fifth", 3, 300),
        ("old", "Old Song - 2011 Remaster", "debut", 0, 200),
    ];
    for (n, &(id, title, album, plays, secs)) in tracks.iter().enumerate() {
        let al = albums.iter().find(|a| a.id == album).unwrap();
        let mut t = fake::track(id, title, al, 1, n as u32 + 1);
        t.view_count = plays;
        t.duration_ms = secs * 1000;
        lib.tracks.push(t);
    }
    lib.albums = albums.into();
    lib.artists = vec![ar];
    lib
}

fn put_global_with_releases(db: &DbHandle, id: &str, songs: &[(&str, u64, &str)]) {
    let top = popular::GlobalTop::from_entries(
        "listenbrainz",
        songs.iter().map(|(t, n, r)| (t.to_string(), *n, Some(r.to_string()))),
    );
    let id = id.to_owned();
    db.write(move |c| {
        crate::artistinfo::cache::put(c, &id, popular::GLOBAL_ROW, Some(&top), now_s(), 3600)?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn popular_picks_the_studio_copy_of_each_listed_song() {
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(copies_library())), false).await;
    // The tape is marked unreleased by its files' ARCHIVE_VERSION.
    t.db.write(|c| {
        c.execute(
            "INSERT INTO file_tags (part_key, status, version, scanned_at)
             SELECT part_key, 'ok', 'Leak', 0 FROM tracks WHERE album_id = 'tape'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    // No list yet: the page waits for one rather than showing plays order.
    let page = t.db.read(|c| queries::artist_page(c, "a1")).unwrap();
    assert!(page.popular_pending);
    put_global_with_releases(
        &t.db,
        "a1",
        &[
            ("Bright Lights", 900, "Fifth Room"),
            ("Power Song", 800, "Fifth Room"),
            ("Gold Song", 700, "First Light"),
            ("Walking Song", 600, "First Light"),
            ("Old Song", 500, "First Light"),
        ],
    );
    let page = t.db.read(|c| queries::artist_page(c, "a1")).unwrap();
    assert!(!page.popular_pending);
    let got: Vec<&str> = page.popular.iter().map(|p| p.id.as_str()).collect();
    // Listed songs in list order, one copy each; then the unmatched by
    // plays, variants of listed songs (the interlude, the session take) last.
    assert_eq!(got, ["lights", "power", "gold", "walk", "old", "lights-interlude", "gold-session"]);

    // Without release names the rules alone still pick the same copies.
    let plain: Vec<(&str, u64)> =
        vec![("Bright Lights", 900), ("Power Song", 800), ("Gold Song", 700), ("Walking Song", 600), ("Old Song", 500)];
    put_global(&t.db, "a1", &plain);
    let page = t.db.read(|c| queries::artist_page(c, "a1")).unwrap();
    let got: Vec<&str> = page.popular.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(got, ["lights", "power", "gold", "walk", "old", "lights-interlude", "gold-session"]);

    // My plays: no waiting for a list.
    let page = t.db.read(|c| queries::artist_page_by(c, "a1", PopularSource::MyPlays)).unwrap();
    assert!(!page.popular_pending);
}

#[tokio::test]
async fn popular_with_a_warm_cache_is_quick() {
    // A big artist: 40 albums of 25 tracks, songs on several albums.
    let mut lib = FakeLibrary { sections: vec![fake::section("1", "u")], ..Default::default() };
    let ar = fake::artist("a1", "Odessa Wren");
    for a in 0..40i32 {
        let live = a % 3 == 0;
        let subs: &[&str] = if live { &["Live"] } else { &[] };
        let mut al = fake::album(&format!("al{a}"), &format!("Record {a}"), &ar, 1990 + a, Some("Album"), subs);
        al.track_count = 25;
        for n in 0..25u32 {
            let song = (a as u32 * 7 + n) % 300;
            let title = if live { format!("Song {song} (Live)") } else { format!("Song {song}") };
            let mut t = fake::track(&format!("t{a}-{n}"), &title, &al, 1, n + 1);
            t.view_count = n;
            lib.tracks.push(t);
        }
        lib.albums.push(al);
    }
    lib.artists = vec![ar];
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(lib)), false).await;
    let songs: Vec<(String, u64)> = (0..150).map(|i| (format!("Song {i}"), 10_000 - i as u64)).collect();
    let top = popular::GlobalTop::from_pairs("listenbrainz", songs);
    t.db.write(move |c| {
        crate::artistinfo::cache::put(c, "a1", popular::GLOBAL_ROW, Some(&top), now_s(), 3600)?;
        Ok(())
    })
    .unwrap();
    // Warm the statement cache, then time it.
    t.db.read(|c| queries::artist_page(c, "a1")).unwrap();
    // The fastest of a few reads: another process taking the CPU slows
    // some, slower code slows all of them.
    let mut ms = u128::MAX;
    let mut page = None;
    for _ in 0..5 {
        let started = std::time::Instant::now();
        page = Some(t.db.read(|c| queries::artist_page(c, "a1")).unwrap());
        ms = ms.min(started.elapsed().as_millis());
    }
    let page = page.unwrap();
    eprintln!("artist page, 1000 tracks, warm list: {ms} ms");
    assert_eq!(page.popular.len(), popular::POPULAR_N);
    assert!(page.popular.iter().all(|t| !t.title.contains("(Live)")), "studio copies");
    // Under 50 ms in a release build; a debug build gets room.
    let budget = if cfg!(debug_assertions) { 250 } else { 50 };
    assert!(ms < budget, "{ms} ms");
}

#[tokio::test]
async fn popular_leaves_out_hidden_songs_and_artists() {
    let own: Vec<Pop> = vec![("t1", "Own Song", None, 9, None, None, 200), ("t2", "Other Song", None, 5, None, None, 200)];
    let moth: Vec<Pop> = vec![("m1", "Feature One", None, 7, None, Some("Moth & Meridian feat. Odessa Wren"), 200)];
    let t = open();
    sync(&t.db, &Arc::new(FakeSource::new(popular_library(&own, &[("Moth & Meridian", &moth)]))), false).await;
    let page_ids = |h: &crate::hidden::HiddenSet| -> Vec<String> {
        t.db.read(|c| queries::artist_page_with(c, "a1", PopularSource::MyPlays, Some(h)))
            .unwrap()
            .popular
            .into_iter()
            .map(|p| p.id)
            .collect()
    };
    let none = crate::hidden::HiddenSet::default();
    assert_eq!(page_ids(&none), ["t1", "m1", "t2"]);
    let mut h = crate::hidden::HiddenSet::default();
    h.tracks.insert("t1".into());
    h.artists.insert("a2".into());
    assert_eq!(page_ids(&h), ["t2"], "the hidden song and the hidden artist's feature are gone");
    // Hiding the page's own artist doesn't empty its page.
    let mut h = crate::hidden::HiddenSet::default();
    h.artists.insert("a1".into());
    assert_eq!(page_ids(&h), ["t1", "m1", "t2"]);
    // The guest's credited page leaves the hidden artist's songs out too.
    let mut h = crate::hidden::HiddenSet::default();
    h.artists.insert("a2".into());
    let guest = t.db.read(|c| queries::artist_page_with(c, "credit:odessa wren", PopularSource::MyPlays, Some(&h)));
    assert!(guest.unwrap().popular.is_empty());
}

// ---------------------------------------------------------------- art fallback

/// Plex sends some albums' tracks with empty `thumb` and `parentThumb`. Their
/// rows, the queue and now playing (all read through `TRACK_COLS`) show the
/// album's cover, then the artist's, with the server tag kept.
#[tokio::test]
async fn songs_without_art_of_their_own_show_their_albums_cover() {
    let t = open();
    let mut lib = fake::generated(1, 2, 2);
    let plain: Vec<String> = lib.tracks.iter().map(|t| t.id.clone()).collect();
    lib.playlists.push((fake::playlist("p1", "Mix", 2), vec![plain[0].clone(), plain[2].clone()]));
    let mut lib = fake::tagged(lib, "0a1b2c3d");
    let bare = lib.albums[0].id.clone();
    let no_cover = lib.albums[1].id.clone();
    for tr in lib.tracks.iter_mut() {
        if tr.album_id == bare {
            tr.thumb = None;
        }
        if tr.album_id == no_cover {
            tr.thumb = Some(String::new());
        }
    }
    lib.albums[1].thumb = None;
    let ids: Vec<String> = lib.tracks.iter().map(|t| t.id.clone()).collect();
    let playlist = lib.playlists[0].0.id.clone();
    let album_cover = lib.albums[0].thumb.clone().unwrap();
    let artist_photo = lib.artists[0].thumb.clone().unwrap();
    assert!(album_cover.starts_with("0a1b2c3d:/library/metadata/"));
    let src = Arc::new(FakeSource::new(lib));
    sync(&t.db, &src, false).await;
    // The cache itself holds no art for these songs.
    let stored: Option<String> = t
        .db
        .read(|c| Ok(c.query_row("SELECT thumb FROM tracks WHERE id = ?1", [&ids[0]], |r| r.get(0))?))
        .unwrap();
    assert_eq!(stored, None);

    let album = t.db.read(|c| queries::album_tracks(c, &bare)).unwrap();
    assert_eq!(album.len(), 2);
    assert!(album.iter().all(|s| s.thumb.as_deref() == Some(album_cover.as_str())), "album rows");
    let page = t.db.read(|c| queries::playlist_page(c, &playlist)).unwrap();
    let thumbs: Vec<_> = page.tracks.iter().map(|s| s.thumb.clone().unwrap_or_default()).collect();
    assert_eq!(thumbs, vec![album_cover.clone(), artist_photo.clone()], "playlist rows; no album cover falls to the artist");
    // What the player, queue, now playing and media controls look songs up by.
    let looked_up = t.db.read(|c| queries::tracks_by_ids(c, &ids)).unwrap();
    assert_eq!(looked_up[0].thumb.as_deref(), Some(album_cover.as_str()));
    assert_eq!(looked_up[2].thumb.as_deref(), Some(artist_photo.as_str()));
}
