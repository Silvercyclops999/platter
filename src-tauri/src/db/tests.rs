//! Integration tests for the db layer: sync, queries, search, listen log.
//! Every test uses a fresh tempfile database.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;

use super::fake::{self, FakeLibrary, FakeSource};
use super::listens::{self, ContextKind, NewReport, PlayStart, ReportKind};
use super::queries::{self, now_s};
use super::search::{self, recent_add, recent_clear, recent_list};
use super::source::{HistoryEntry, ItemKind, LibrarySource};
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
    t.db.write(|c| search::recent_add(c, "keep me")).unwrap();

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
    assert_eq!(t.db.read(search::recent_list).unwrap(), vec!["keep me"]);
    assert!(search::search(&t.db, "artist 0", SearchFilter::Artists, None).unwrap().artists.is_empty());
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
        h.push(HistoryEntry { rating_key: tr(0, 0, n), viewed_at: now - 60 * 86400 });
    }
    for i in 0..5 {
        h.push(HistoryEntry { rating_key: tr(1, 1, 0), viewed_at: now - 86400 * (i + 1) });
    }
    h.push(HistoryEntry { rating_key: tr(2, 0, 1), viewed_at: now - 3600 });
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
    let played: Vec<_> = home.recently_played.iter().map(|a| a.id.clone()).collect();
    assert_eq!(played, vec![album_of(&local_track), album_of(&tr(2, 0, 1)), album_of(&tr(1, 1, 0)), album_of(&tr(0, 0, 0))]);
    assert_eq!(home.recently_added.len(), queries::HOME_ROW);
    assert!(home.recently_added.windows(2).all(|w| w[0].added_at >= w[1].added_at));
    // Jump back in: (1,1) qualifies (5 plays, unfinished) but it is already
    // in the first six recently played, so the row stays empty here.
    assert!(home.jump_back_in.is_empty());
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
    // Short words (< 5 chars) are not fuzzed: "lova" ≠ "love".
    assert!(search::search(&t.db, "lova", SearchFilter::All, None).unwrap().songs.is_empty());
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
    assert_eq!(r.songs.len(), 4);
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
async fn recent_searches_dedupe_cap_and_clear() {
    let t = open();
    t.db.write(|c| {
        recent_add(c, "Beyoncé")?;
        recent_add(c, "abbey road")?;
        recent_add(c, "  beyonce ")?; // same as the first once folded
        recent_add(c, "")?;
        Ok(())
    })
    .unwrap();
    assert_eq!(t.db.read(recent_list).unwrap(), vec!["beyonce", "abbey road"]);
    t.db.write(|c| {
        for i in 0..30 {
            recent_add(c, &format!("query {i}"))?;
        }
        Ok(())
    })
    .unwrap();
    let l = t.db.read(recent_list).unwrap();
    assert_eq!(l.len(), 20);
    assert_eq!(l[0], "query 29");
    t.db.write(|c| recent_clear(c)).unwrap();
    assert!(t.db.read(recent_list).unwrap().is_empty());
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
            lib.history.push(HistoryEntry { rating_key: k, viewed_at: now - rng.below(40 * 86400) as i64 });
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
