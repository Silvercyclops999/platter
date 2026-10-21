use std::sync::Arc;

use super::*;
use crate::db::DbHandle;
use crate::db::fake::{self, FakeSource};
use crate::db::source::LibrarySource;

async fn synced(artists: usize, albums_per: usize) -> (DbHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("library.db")).unwrap();
    let src = Arc::new(FakeSource::new(fake::generated(artists, albums_per, 3)));
    db.sync_once(src as Arc<dyn LibrarySource>, true).await.unwrap();
    (db, dir)
}

#[tokio::test]
async fn library_lists_play_what_they_show_and_stay_short() {
    let (db, _dir) = synced(30, 5).await;
    let all = db.read(|c| page(c, &CarList::Albums, 10.0, 0, 500)).unwrap();
    assert!(all.len() <= LIST_MAX, "150 albums never make a list longer than our limit");
    let CarAction::Turn { page: first, .. } = all[0].action else { panic!("150 albums open on their letters") };
    let albums = db.read(|c| page(c, &CarList::Albums, 10.0, first, 500)).unwrap();
    assert_eq!(albums[0].title, "All letters");
    let albums: Vec<_> = albums.iter().filter(|a| !matches!(a.action, CarAction::Turn { .. })).collect();
    assert!(!albums.is_empty());
    for a in albums {
        assert!(!a.title.is_empty());
        assert!(a.subtitle.as_deref().is_some_and(|s| !s.is_empty()), "albums say who made them");
        assert!(matches!(&a.action, CarAction::Open(CarList::Songs { kind: ContextKind::Album, .. })), "an album opens to its songs");
    }
    let artists = db.read(|c| list(c, &CarList::Artists, 10.0)).unwrap();
    assert_eq!(artists.len(), 30);
    assert!(artists.iter().all(|a| matches!(&a.action, CarAction::Open(CarList::Artist { id, name }) if !id.is_empty() && *name == a.title)));
    let names: Vec<&str> = artists.iter().map(|a| a.title.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_by_key(|n| n.to_lowercase());
    assert_eq!(names, sorted, "artists A to Z");
    let added = db.read(|c| list(c, &CarList::RecentlyAdded, 10.0)).unwrap();
    assert_eq!(added.len(), LIST_MAX);
}

#[tokio::test]
async fn an_empty_library_gives_empty_lists_not_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("library.db")).unwrap();
    for which in [CarList::RecentlyPlayed, CarList::RecentlyAdded, CarList::Playlists, CarList::Albums, CarList::Artists] {
        assert!(db.read(|c| list(c, &which, 10.0)).unwrap().is_empty(), "{which:?}");
    }
    // With no songs there's nothing to shuffle; likes and the shelves stay.
    let home = db.read(home).unwrap();
    let titles: Vec<_> = home.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["Liked songs", "Recently played", "Recently added"]);
}

#[tokio::test]
async fn home_opens_on_shuffle_and_likes_then_what_you_played_then_mixes() {
    let (db, _dir) = synced(4, 3).await;
    db.write(|c| {
        c.execute_batch("UPDATE albums SET last_viewed_at = 1000 + rowid, view_count = 1")?;
        Ok(())
    })
    .unwrap();
    let sections = db.read(home_sections).unwrap();
    let CarRow::Item(shuffle) = &sections[0].rows[0] else { panic!("a row first") };
    assert_eq!(shuffle.title, "Shuffle my library");
    assert_eq!(shuffle.action, CarAction::ShuffleLibrary, "the songs are drawn when it's picked");
    let CarRow::Item(liked) = &sections[0].rows[1] else { panic!("a row second") };
    assert_eq!(liked.title, "Liked songs");
    let ctx = db.read(shuffle_library_context).unwrap();
    assert_eq!(ctx.track_ids.as_ref().map(Vec::len), Some(4 * 3 * 3), "every song (under the cap)");
    let CarRow::Covers { title, items } = &sections[1].rows[0] else { panic!("covers second: {:?}", sections[1]) };
    assert_eq!(title, "Jump back in");
    assert!(!items.is_empty() && items.len() <= COVERS_MAX);
    assert!(items.iter().all(|i| matches!(&i.action, CarAction::Open(CarList::Songs { .. }))), "a cover opens its songs");
    assert_eq!(sections.last().unwrap().header.as_deref(), Some("More"));
}

#[tokio::test]
async fn shuffle_my_library_starts_with_at_most_the_cap() {
    let (db, _dir) = synced(40, 5).await;
    let ctx = db.read(shuffle_library_context).unwrap();
    assert_eq!(ctx.track_ids.as_ref().map(Vec::len), Some(SHUFFLE_MAX));
}

#[test]
fn library_tab_starts_with_liked_songs_then_the_shelves() {
    let tab = library();
    assert_eq!(tab[0].title, "Liked songs");
    assert!(opens(&tab[0].action, ContextKind::Liked, "liked"));
    let opens: Vec<_> = tab[1..].iter().map(|i| i.title.as_str()).collect();
    assert_eq!(opens, ["Playlists", "Albums", "Artists"]);
}

#[tokio::test]
async fn radio_offers_stations_from_whats_playing_then_albums() {
    let (db, _dir) = synced(3, 2).await;
    let none = db.read(|c| radio(c, None)).unwrap();
    assert_eq!(none.len(), 1, "a random album only, while nothing plays (nothing is old yet)");
    assert_eq!(none[0].title, "A random album");
    let playing = Playing { track_id: "t1".into(), title: "Song".into(), artist_id: "a1".into(), artist: "Band".into() };
    let with = db.read(|c| radio(c, Some(&playing))).unwrap();
    assert_eq!(with[0].title, "Radio from Song");
    assert!(matches!(&with[0].action, CarAction::Play(p) if p.kind == ContextKind::Radio && p.id.as_deref() == Some("track:t1")));
    assert!(matches!(&with[1].action, CarAction::Play(p) if p.id.as_deref() == Some("artist:a1")));
}

#[tokio::test]
async fn downloaded_is_empty_with_nothing_kept() {
    let (db, _dir) = synced(2, 2).await;
    assert!(db.read(downloaded).unwrap().is_empty());
}

#[tokio::test]
async fn an_artist_opens_popular_then_their_albums_newest_first() {
    let (db, _dir) = synced(3, 4).await;
    let artists = db.read(|c| list(c, &CarList::Artists, 10.0)).unwrap();
    let CarAction::Open(which) = &artists[0].action else { panic!("artists open a list") };
    let CarList::Artist { id, .. } = which else { panic!("an artist's list") };
    assert_eq!(which.title(), artists[0].title, "the list is titled with the artist's name");
    let rows = db.read(|c| list(c, which, 10.0)).unwrap();
    assert_eq!(rows.len(), 5, "Popular and four albums");
    assert_eq!(rows[0].title, "Popular");
    assert!(matches!(&rows[0].action, CarAction::Play(p) if p.kind == ContextKind::Artist && p.id.as_ref() == Some(id)));
    let mut own = db.read(|c| queries::artist_albums(c, id, LIST_MAX)).unwrap();
    own.sort_by_key(|a| std::cmp::Reverse(a.year));
    for (row, album) in rows[1..].iter().zip(&own) {
        assert_eq!(row.title, album.title);
        assert!(opens(&row.action, ContextKind::Album, &album.id));
    }
}

/// Every page of an artist's list for a car showing `room` rows, turning
/// with the Older releases rows from the first.
fn artist_pages(db: &crate::db::DbHandle, which: &CarList, room: usize) -> Vec<Vec<CarItem>> {
    let mut pages = vec![db.read(|c| page(c, which, 10.0, 0, room)).unwrap()];
    while let Some(CarAction::Turn { list, page: next }) = pages.last().unwrap().last().map(|r| r.action.clone()) {
        assert_eq!(&list, which, "a page turns to the same artist");
        assert_eq!(next, pages.len(), "Older releases turns to the next page");
        pages.push(db.read(|c| page(c, which, 10.0, next, room)).unwrap());
        assert!(pages.len() < 100, "the pages end");
    }
    pages
}

fn first_artist(db: &crate::db::DbHandle) -> CarList {
    let artists = db.read(|c| list(c, &CarList::Artists, 10.0)).unwrap();
    let CarAction::Open(which) = artists[0].action.clone() else { panic!("artists open a list") };
    which
}

#[tokio::test]
async fn a_car_that_shows_twelve_rows_pages_through_every_release_once() {
    let (db, _dir) = synced(1, 40).await;
    let which = first_artist(&db);
    let CarList::Artist { id, .. } = &which else { panic!("an artist's list") };
    let pages = artist_pages(&db, &which, LIST_SHORT);
    assert_eq!(pages.len(), 4, "10 on the first page, then 10, 10 and 10");
    let mut seen = Vec::new();
    for (n, rows) in pages.iter().enumerate() {
        assert!(rows.len() <= LIST_SHORT, "page {n} fits the car: {}", rows.len());
        if n == 0 {
            assert_eq!(rows[0].title, "Popular");
        } else {
            assert_eq!(rows[0].title, "Newer releases");
            assert!(matches!(&rows[0].action, CarAction::Turn { page, .. } if *page == n - 1), "Newer turns back a page");
        }
        let last = n + 1 == pages.len();
        let albums = if last { &rows[1..] } else { &rows[1..rows.len() - 1] };
        if !last {
            let older = rows.last().unwrap();
            assert_eq!(older.title, "Older releases");
            assert_eq!(older.subtitle.as_deref(), Some(format!("{} more", 40 - 10 * (n + 1)).as_str()));
        }
        for r in albums {
            let CarAction::Open(CarList::Songs { kind: ContextKind::Album, id, .. }) = &r.action else { panic!("{} opens its songs", r.title) };
            seen.push(id.clone());
        }
    }
    let mut own: Vec<_> = db.read(|c| queries::artist_albums(c, id, LIST_MAX)).unwrap();
    own.sort_by_key(|a| std::cmp::Reverse(a.year));
    let own: Vec<String> = own.into_iter().map(|a| a.id).collect();
    assert_eq!(seen, own, "every release once, newest first, across the pages");
}

#[tokio::test]
async fn an_artist_list_fills_a_roomy_car_and_turns_the_page_past_it() {
    let (db, _dir) = synced(1, 150).await;
    let which = first_artist(&db);
    let rows = db.read(|c| list(c, &which, 10.0)).unwrap();
    assert_eq!(rows.len(), LIST_MAX, "Popular, 98 releases and Older releases");
    assert_eq!(rows.last().unwrap().subtitle.as_deref(), Some("52 more"));
    let pages = artist_pages(&db, &which, LIST_MAX);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].len(), 53, "Newer releases and the last 52");
    let none = db.read(|c| list(c, &CarList::Artist { id: "nobody".into(), name: "Nobody".into() }, 10.0)).unwrap();
    assert_eq!(none.len(), 1, "an artist with nothing here is just Popular");
    assert_eq!(none[0].title, "Popular");
}

#[tokio::test]
async fn one_page_when_it_fits_and_two_when_one_more_does_not() {
    let (db, _dir) = synced(1, 11).await;
    let pages = artist_pages(&db, &first_artist(&db), LIST_SHORT);
    assert_eq!(pages.len(), 1, "Popular and 11 releases fill 12 rows");
    assert_eq!(pages[0].len(), 12);
    assert!(pages[0].iter().all(|r| !matches!(r.action, CarAction::Turn { .. })));
    let (db, _dir) = synced(1, 12).await;
    let pages = artist_pages(&db, &first_artist(&db), LIST_SHORT);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].len(), 12, "Popular, 10 releases, Older releases");
    assert_eq!(pages[1].len(), 3, "Newer releases and the last 2");
}

#[tokio::test]
async fn a_car_reporting_odd_room_still_gets_a_usable_list() {
    let (db, _dir) = synced(1, 300).await;
    let which = first_artist(&db);
    // A car that says it has no room, or one row, gets the fewest any car shows.
    for room in [0, 1, 2] {
        let rows = db.read(|c| page(c, &which, 10.0, 0, room)).unwrap();
        assert_eq!(rows.len(), LIST_SHORT, "room {room}");
        assert_eq!(rows.last().unwrap().title, "Older releases");
    }
    // A car with room for 500 still gets no more than our own limit.
    let rows = db.read(|c| page(c, &which, 10.0, 0, 500)).unwrap();
    assert_eq!(rows.len(), LIST_MAX);
}

#[tokio::test]
async fn a_page_past_the_end_shows_the_last_page() {
    // The library shrank, or the car gave more room, between two turns.
    let (db, _dir) = synced(1, 15).await;
    let which = first_artist(&db);
    let rows = db.read(|c| page(c, &which, 10.0, 7, LIST_SHORT)).unwrap();
    assert_eq!(rows[0].title, "Newer releases");
    assert!(matches!(&rows[0].action, CarAction::Turn { page: 0, .. }));
    assert_eq!(rows.len(), 6, "Newer releases and the last 5");
    let roomy = db.read(|c| page(c, &which, 10.0, 1, LIST_MAX)).unwrap();
    assert_eq!(roomy[0].title, "Popular", "with room for everything there's one page");
    assert_eq!(roomy.len(), 16);
}

#[test]
fn releases_sort_newest_first_with_unreleased_and_undated_last() {
    use crate::db::fake::{album, artist};
    use crate::model::ReleaseSection;
    let ar = artist("1", "A");
    let mut demo = album("d", "Demo", &ar, 2024, Some("album"), &["Demo"]);
    demo.section = ReleaseSection::Demos;
    let mut leak = album("u", "Leak", &ar, 2025, Some("album"), &[]);
    leak.section = ReleaseSection::Unreleased;
    let mut undated = album("n", "Undated", &ar, 2000, Some("album"), &[]);
    undated.year = None;
    let mut albums = vec![
        album("o", "Old", &ar, 1999, Some("album"), &[]),
        demo,
        undated,
        album("s", "Single", &ar, 2021, Some("single"), &[]),
        leak,
        album("a", "Album", &ar, 2019, Some("album"), &[]),
    ];
    sort_newest_first(&mut albums);
    let order: Vec<&str> = albums.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(order, ["Single", "Album", "Old", "Undated", "Leak", "Demo"]);
}

#[test]
fn names_file_under_their_first_letter_without_accents() {
    assert_eq!(letter("Abba"), 'A');
    assert_eq!(letter("élan"), 'E');
    assert_eq!(letter("\u{201c}Heroes\u{201d}"), 'H', "quotes are skipped");
    assert_eq!(letter("2Pac"), '#');
    assert_eq!(letter("坂本龍一"), '#');
    assert_eq!(letter(""), '#', "a blank name still files somewhere");
}

fn rows_titled(names: &[&str]) -> Vec<(char, CarItem)> {
    names.iter().map(|n| (letter(n), info(n, None))).collect()
}

#[test]
fn a_short_a_to_z_list_is_one_list() {
    let rows = a_to_z(&CarList::Albums, rows_titled(&["b", "1999", "a", "c"]), ("album", "albums"), 0, LIST_SHORT);
    let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["a", "b", "c", "1999"], "A to Z, then # last");
}

/// A fake library of `per` names under each of `letters`.
fn names(letters: &str, per: usize) -> Vec<String> {
    letters.chars().flat_map(|l| (0..per).map(move |i| format!("{l}{i:03}"))).collect()
}

#[test]
fn a_long_a_to_z_list_opens_on_its_letters_and_reaches_every_name_once() {
    let all = names("ABCDEFGHIJKLMNOPQRSTUVWXYZ1", 9);
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    let page_of = |p| a_to_z(&CarList::Artists, rows_titled(&refs), ("artist", "artists"), p, LIST_SHORT);
    let index = page_of(0);
    assert!(index.len() <= LIST_SHORT, "letters fit the car: {}", index.len());
    assert_eq!(index[0].title, "A to C");
    assert_eq!(index[0].subtitle.as_deref(), Some("27 artists"));
    assert!(index.last().unwrap().title.ends_with("and #"), "{}", index.last().unwrap().title);
    let mut seen = Vec::new();
    for row in &index {
        let CarAction::Turn { page: mut p, .. } = row.action else { panic!("a run of letters turns the page") };
        loop {
            let rows = page_of(p);
            assert!(rows.len() <= LIST_SHORT);
            assert_eq!(rows[0].title, "All letters");
            assert!(matches!(rows[0].action, CarAction::Turn { page: 0, .. }));
            match &rows.last().unwrap().action {
                CarAction::Turn { page: next, .. } if rows.last().unwrap().title == "More" => {
                    seen.extend(rows[1..rows.len() - 1].iter().map(|r| r.title.clone()));
                    assert_eq!(*next, p + 1);
                    p = *next;
                }
                _ => {
                    seen.extend(rows[1..].iter().map(|r| r.title.clone()));
                    break;
                }
            }
        }
    }
    assert_eq!(seen, all, "every name once, A to Z then #");
}

#[test]
fn every_letter_gets_its_own_row_when_there_is_room() {
    let all = names("ABC", 50);
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    let index = a_to_z(&CarList::Albums, rows_titled(&refs), ("album", "albums"), 0, LIST_MAX);
    let titles: Vec<&str> = index.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["A", "B", "C"]);
    let a = a_to_z(&CarList::Albums, rows_titled(&refs), ("album", "albums"), 1, LIST_MAX);
    assert_eq!(a.len(), 51, "All letters and the 50 under A");
    let more = a_to_z(&CarList::Albums, rows_titled(&refs), ("album", "albums"), 1, LIST_SHORT);
    assert_eq!(more.last().unwrap().subtitle.as_deref(), Some("40 more albums in A"));
}

#[test]
fn a_page_past_the_letters_goes_back_to_them() {
    // The library shrank, or the car gave more room, between two turns.
    let all = names("ABCDEFGHIJKLMN", 3);
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    let lost = a_to_z(&CarList::Albums, rows_titled(&refs), ("album", "albums"), 99, LIST_SHORT);
    assert_eq!(lost, a_to_z(&CarList::Albums, rows_titled(&refs), ("album", "albums"), 0, LIST_SHORT));
}

#[tokio::test]
async fn now_playings_album_button_opens_the_playing_songs_album() {
    let (db, _dir) = synced(1, 2).await;
    let album = db.read(|c| queries::albums(c, AlbumSort::Name, 0, 1, None)).unwrap().items.remove(0);
    let now = NowKey { album_id: Some(album.id.clone()), ..Default::default() };
    let list = db.read(|c| now_album(c, &now)).unwrap();
    assert_eq!(list, Some(CarList::Songs { kind: ContextKind::Album, id: album.id.clone(), title: album.title.clone() }));
    let rows = db.read(|c| super::list(c, list.as_ref().unwrap(), 10.0)).unwrap();
    assert_eq!(rows[0].title, "Play", "the album opens to Play, Shuffle and its songs");
    // Nothing playing, or a song whose album the library no longer has.
    assert_eq!(db.read(|c| now_album(c, &NowKey::default())).unwrap(), None);
    let gone = NowKey { album_id: Some("gone".into()), ..Default::default() };
    assert_eq!(db.read(|c| now_album(c, &gone)).unwrap(), None);
}

#[test]
fn page_turns_stay_when_offline() {
    let turn = CarItem {
        title: "Older releases".into(),
        subtitle: Some("3 more".into()),
        thumb: None,
        action: CarAction::Turn { list: CarList::Artist { id: "1".into(), name: "A".into() }, page: 1 },
    };
    let kept = Kept::default();
    assert_eq!(offline_rows(vec![turn.clone()], &kept, true), vec![turn.clone()], "turning a page needs no signal");
    assert_eq!(offline_rows(vec![turn.clone()], &kept, false), vec![turn]);
}

fn health(status: crate::plex::types::HealthStatus, cached_only: bool) -> crate::plex::types::Health {
    crate::plex::types::Health {
        status,
        server_name: None,
        connection: None,
        latency_ms: None,
        server_version: None,
        last_seen_at: None,
        scan: None,
        banner: None,
        files_available: true,
        cached_only,
    }
}

#[test]
fn reach_follows_sign_in_and_the_server() {
    use crate::plex::types::HealthStatus as H;
    assert_eq!(reach(&health(H::NoAuth, false), false), Reach::SignedOut);
    assert_eq!(reach(&health(H::NoAuth, false), true), Reach::Online, "another server is set up");
    assert_eq!(reach(&health(H::NoAuth, true), false), Reach::Unreachable, "the server signed us out: downloads still play");
    assert_eq!(reach(&health(H::Offline, true), false), Reach::Unreachable);
    assert_eq!(reach(&health(H::Local, false), false), Reach::Online);
    assert_eq!(reach(&health(H::Starting, false), false), Reach::Online);
}

#[test]
fn signed_out_every_tab_says_to_sign_in() {
    for tab in [Tab::Home, Tab::Library, Tab::Downloaded, Tab::Radio] {
        let rows = tab_rows(tab, Reach::SignedOut, || panic!("nothing is read while signed out"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Open Platter on your phone to sign in");
        assert_eq!(rows[0].action, CarAction::Info);
    }
}

#[test]
fn out_of_reach_only_downloaded_lists_its_rows() {
    let kept = || library();
    for tab in [Tab::Home, Tab::Library, Tab::Radio] {
        let rows = tab_rows(tab, Reach::Unreachable, || panic!("{tab:?} is not read"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, UNREACHABLE);
        assert_eq!(rows[0].action, CarAction::Info);
    }
    assert_eq!(tab_rows(Tab::Downloaded, Reach::Unreachable, kept), library());
    assert_eq!(tab_rows(Tab::Home, Reach::Online, kept), library());
}

fn queued(id: &str, title: &str) -> crate::audio::queue::QueueItem {
    let mut track = fake::generated(1, 1, 1).tracks.remove(0);
    track.title = title.into();
    track.track_artist = None;
    crate::audio::queue::QueueItem { item_id: id.into(), track, source: crate::audio::queue::ItemSource::Context }
}

#[test]
fn up_next_lists_the_hand_queued_songs_first_and_jumps() {
    let q = crate::audio::queue::QueueView {
        current: Some(queued("now", "Playing")),
        user_queue: vec![queued("u1", "Queued")],
        next_up: (0..150).map(|i| queued(&format!("n{i}"), &format!("Next {i}"))).collect(),
        history: vec![queued("h", "Earlier")],
        context: None,
    };
    let rows = up_next(&q);
    assert_eq!(rows.len(), LIST_MAX);
    assert_eq!(rows[0].title, "Queued");
    assert_eq!(rows[0].action, CarAction::Jump("u1".into()));
    assert_eq!(rows[1].action, CarAction::Jump("n0".into()));
    assert!(rows[0].subtitle.as_deref().is_some_and(|s| !s.is_empty()), "rows say who it's by");
    assert!(up_next(&crate::audio::queue::QueueView::default()).is_empty());
}

#[test]
fn repeat_button_cycles_off_all_one() {
    use crate::audio::queue::Repeat;
    assert_eq!(next_repeat(Repeat::Off), Repeat::All);
    assert_eq!(next_repeat(Repeat::All), Repeat::One);
    assert_eq!(next_repeat(Repeat::One), Repeat::Off);
}

#[test]
fn rows_playing_what_plays_now_are_marked() {
    let album = |id: &str| CarItem { title: "A".into(), subtitle: None, thumb: None, action: play(ContextKind::Album, id) };
    let playlist = CarItem { title: "P".into(), subtitle: None, thumb: None, action: play(ContextKind::Playlist, "p1") };
    let now = NowKey { context: Some((ContextKind::Playlist, "p1".into())), album_id: Some("al9".into()), track_id: None };
    assert!(is_playing(&playlist, &now), "the playing playlist");
    assert!(is_playing(&album("al9"), &now), "the album the playing song is on");
    assert!(!is_playing(&album("al1"), &now));
    // The same id under another kind isn't it.
    let mix = CarItem { title: "M".into(), subtitle: None, thumb: None, action: play(ContextKind::Mix, "p1") };
    assert!(!is_playing(&mix, &now));
    // Lists to open, and nothing playing, mark nothing.
    assert!(!is_playing(&open(CarList::Albums), &now));
    assert!(!is_playing(&playlist, &NowKey::default()));
}

#[tokio::test]
async fn home_lists_the_mixes_and_leaves_artists_out_of_the_covers() {
    let (db, _dir) = synced(4, 3).await;
    db.write(|c| {
        // An artist and an album played lately; only the album is a cover.
        c.execute_batch("UPDATE albums SET last_viewed_at = 1000 + rowid, view_count = 1")?;
        let ids: Vec<String> = c
            .prepare("SELECT id FROM tracks LIMIT 30")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mix = crate::mixes::store::Stored {
            id: "daily-1".into(),
            kind: crate::mixes::MixKind::Daily,
            title: "Daily Mix 1".into(),
            description: "Some artists and more".into(),
            color: "#334455".into(),
            artist_ids: vec![],
            tag: None,
            tracks: ids.into_iter().map(|id| (id, false)).collect(),
        };
        crate::mixes::store::save_one(c, &mix, 0, "2026-10-20", 0)?;
        Ok(())
    })
    .unwrap();
    let sections = db.read(home_sections).unwrap();
    let made = sections.iter().find(|s| s.header.as_deref() == Some("Made for you")).expect("a mixes section");
    let CarRow::Item(m) = &made.rows[0] else { panic!("a mix row") };
    assert_eq!(m.title, "Daily Mix 1");
    assert!(opens(&m.action, ContextKind::Mix, "daily-1"));
    let CarRow::Covers { items, .. } = &sections[1].rows[0] else { panic!("covers") };
    assert!(items.iter().all(|i| !matches!(i.action, CarAction::Open(CarList::Artist { .. }))), "no artists among the covers");
}

#[test]
fn home_out_of_reach_or_signed_out_says_so() {
    let signed_out = tab_rows(Tab::Home, Reach::SignedOut, Vec::new);
    assert_eq!(signed_out[0].title, SIGN_IN);
    let away = tab_rows(Tab::Home, Reach::Unreachable, Vec::new);
    assert_eq!(away[0].title, UNREACHABLE);
}

#[test]
fn release_lines_say_year_and_kind() {
    assert_eq!(release_line(Some(2011), Some("album")).as_deref(), Some("2011"));
    assert_eq!(release_line(Some(2019), Some("ep")).as_deref(), Some("EP · 2019"));
    assert_eq!(release_line(None, Some("single")).as_deref(), Some("Single"));
    assert_eq!(release_line(None, None), None);
}

#[test]
fn covers_crop_to_a_centred_whole_pixel_square() {
    assert_eq!(square_crop(600, 600), None);
    assert_eq!(square_crop(0, 10), None);
    assert_eq!(square_crop(800, 600), Some((100, 0, 600)));
    assert_eq!(square_crop(600, 901), Some((0, 150, 600)), "odd sizes stay square");
}


fn connect(status: crate::audio::events::PlayerStatus) -> Connect {
    Connect {
        resume_on: true,
        reach: Reach::Online,
        status,
        track: Some(("t1".into(), false)),
        since_disconnect_s: None,
    }
}

#[test]
fn a_car_connecting_plays_on_where_you_left_off() {
    use crate::audio::events::PlayerStatus;
    assert_eq!(on_connect(&connect(PlayerStatus::Paused)), OnConnect::Resume);
    assert_eq!(on_connect(&connect(PlayerStatus::Idle)), OnConnect::Resume, "a fresh start with the saved queue");
    for busy in [PlayerStatus::Playing, PlayerStatus::Loading, PlayerStatus::Buffering, PlayerStatus::Error] {
        assert_eq!(on_connect(&connect(busy)), OnConnect::Nothing, "{busy:?}");
    }
    let off = Connect { resume_on: false, ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&off), OnConnect::Nothing, "the setting is off");
    let empty = Connect { track: None, ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&empty), OnConnect::Nothing, "nothing new is started");
    let out = Connect { reach: Reach::SignedOut, ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&out), OnConnect::Nothing);
}

#[test]
fn a_car_that_only_blinked_changes_nothing() {
    use crate::audio::events::PlayerStatus;
    let blink = Connect { since_disconnect_s: Some(RECONNECT_GRACE_S - 1), ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&blink), OnConnect::Nothing, "paused in the car, the cable wobbled");
    let later = Connect { since_disconnect_s: Some(RECONNECT_GRACE_S), ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&later), OnConnect::Resume, "a new drive");
}

#[test]
fn the_blink_line_is_thirty_seconds() {
    use crate::audio::events::PlayerStatus;
    assert_eq!(RECONNECT_GRACE_S, 30);
    let at = |s| on_connect(&Connect { since_disconnect_s: Some(s), ..connect(PlayerStatus::Paused) });
    assert_eq!(at(29), OnConnect::Nothing);
    assert_eq!(at(30), OnConnect::Resume);
    assert_eq!(at(0), OnConnect::Nothing);
}

#[test]
fn time_since_the_car_left_counts_the_phones_sleep() {
    assert_eq!(since_left(5, Some(5)), 5, "a blink on both clocks");
    assert_eq!(since_left(5, Some(8 * 3600)), 8 * 3600, "a short drive, then a night asleep");
    assert_eq!(since_left(40, Some(3)), 40, "the wall clock set back a little");
    assert_eq!(since_left(5, None), u64::MAX, "a wall clock that went backwards resumes");
    use crate::audio::events::PlayerStatus;
    let slept = Connect { since_disconnect_s: Some(since_left(5, Some(8 * 3600))), ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&slept), OnConnect::Resume);
}

#[test]
fn signed_out_never_resumes_even_a_kept_song() {
    use crate::audio::events::PlayerStatus;
    let c = Connect { reach: Reach::SignedOut, track: Some(("t1".into(), true)), ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&c), OnConnect::Nothing);
    let idle = Connect { status: PlayerStatus::Idle, ..c };
    assert_eq!(on_connect(&idle), OnConnect::Nothing);
}

#[test]
fn out_of_reach_only_a_kept_song_resumes() {
    use crate::audio::events::PlayerStatus;
    let streamed = Connect { reach: Reach::Unreachable, ..connect(PlayerStatus::Paused) };
    assert_eq!(on_connect(&streamed), OnConnect::Nothing, "it would only fail");
    let kept = Connect { track: Some(("t1".into(), true)), ..streamed };
    assert_eq!(on_connect(&kept), OnConnect::Resume);
}

fn row(title: &str, action: CarAction) -> CarItem {
    CarItem { title: title.into(), subtitle: None, thumb: None, action }
}

fn ctx(kind: ContextKind, id: &str) -> CarAction {
    CarAction::Play(PlayContext { kind, id: Some(id.into()), track_ids: None })
}

fn kept_album_a1() -> Kept {
    Kept {
        items: [(crate::downloads::store::ItemKind::Album, "a1".to_string())].into(),
        tracks: ["t1".to_string()].into(),
    }
}

#[test]
fn kept_knows_what_plays_with_no_signal() {
    let k = kept_album_a1();
    assert!(k.plays(&ctx(ContextKind::Album, "a1")));
    assert!(!k.plays(&ctx(ContextKind::Album, "a2")));
    assert!(!k.plays(&ctx(ContextKind::Playlist, "a1")), "kinds don't mix");
    assert!(!k.plays(&ctx(ContextKind::Liked, "liked")));
    assert!(!k.plays(&ctx(ContextKind::Artist, "x")), "Popular comes from the server");
    assert!(!k.plays(&ctx(ContextKind::Radio, "track:t1")));
    let some = CarAction::Play(PlayContext {
        kind: ContextKind::Tracks,
        id: None,
        track_ids: Some(vec!["t9".into(), "t1".into()]),
    });
    assert!(k.plays(&some), "a list with a kept song plays it");
    assert!(k.plays(&CarAction::ShuffleLibrary));
    assert!(!Kept::default().plays(&CarAction::ShuffleLibrary), "nothing kept, nothing to shuffle");
    for always in [CarAction::Open(CarList::Albums), CarAction::Jump("q".into()), CarAction::Info] {
        assert!(Kept::default().plays(&always), "{always:?}");
    }
    let liked = Kept { items: [(crate::downloads::store::ItemKind::Liked, String::new())].into(), ..Kept::default() };
    assert!(liked.plays(&ctx(ContextKind::Liked, "liked")), "Liked songs is kept under an empty id");
}

#[test]
fn offline_rows_hide_or_say_what_needs_a_connection() {
    let k = kept_album_a1();
    let rows = || {
        vec![
            row("Kept", ctx(ContextKind::Album, "a1")),
            row("Streamed", ctx(ContextKind::Album, "a2")),
            row("Albums", CarAction::Open(CarList::Albums)),
        ]
    };
    let only: Vec<_> = offline_rows(rows(), &k, true).into_iter().map(|r| r.title).collect();
    assert_eq!(only, ["Kept", "Albums"], "lists stay: they're filtered when opened");
    let all = offline_rows(rows(), &k, false);
    assert_eq!(all.len(), 3);
    assert_eq!(all[0], rows()[0], "a kept row is untouched");
    assert_eq!(all[1].subtitle.as_deref(), Some(NEEDS_CONNECTION));
    assert_eq!(all[1].action, CarAction::Info, "picking it does nothing");
}

#[test]
fn offline_home_keeps_covers_that_play_and_drops_empty_sections() {
    let k = kept_album_a1();
    let sections = vec![
        CarSection { header: None, rows: vec![CarRow::Item(row("Shuffle my library", CarAction::ShuffleLibrary))] },
        CarSection {
            header: None,
            rows: vec![CarRow::Covers {
                title: "Jump back in".into(),
                items: vec![row("a1", ctx(ContextKind::Album, "a1")), row("a2", ctx(ContextKind::Album, "a2"))],
            }],
        },
        CarSection { header: Some("Made for you".into()), rows: vec![CarRow::Item(row("Daily Mix 1", ctx(ContextKind::Mix, "m1")))] },
    ];
    let out = offline_sections(sections.clone(), &k, true);
    assert_eq!(out.len(), 2, "the mixes section had nothing kept");
    let CarRow::Covers { items, .. } = &out[1].rows[0] else { panic!("covers stay") };
    assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["a1"]);
    // Without the setting the mix stays and says why it won't play; covers
    // still only show what plays.
    let out = offline_sections(sections, &k, false);
    assert_eq!(out.len(), 3);
    let CarRow::Item(mix) = &out[2].rows[0] else { panic!() };
    assert_eq!(mix.subtitle.as_deref(), Some(NEEDS_CONNECTION));
    let CarRow::Covers { items, .. } = &out[1].rows[0] else { panic!() };
    assert_eq!(items.len(), 1);
}

#[test]
fn offline_tab_with_nothing_playable_says_so() {
    let rows = vec![row("Streamed", ctx(ContextKind::Album, "a2"))];
    for only in [true, false] {
        let out = offline_tab(rows.clone(), &Kept::default(), only);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, UNREACHABLE);
    }
    let out = offline_tab(rows, &kept_album_a1(), true);
    assert_eq!(out[0].title, UNREACHABLE, "a2 isn't kept");
    let out = offline_tab(vec![row("Kept", ctx(ContextKind::Album, "a1"))], &kept_album_a1(), true);
    assert_eq!(out[0].title, "Kept");
}

#[tokio::test]
async fn kept_reads_finished_downloads_and_shuffle_draws_from_them() {
    use crate::downloads::store::{self, ItemKey, ItemKind};
    let (db, _dir) = synced(2, 2).await;
    let (album, tracks): (String, Vec<String>) = db
        .read(|c| {
            let album: String = c.query_row("SELECT id FROM albums ORDER BY id LIMIT 1", [], |r| r.get(0))?;
            let mut st = c.prepare("SELECT id FROM tracks WHERE album_id = ?1 ORDER BY id")?;
            let t = st.query_map([&album], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok((album, t))
        })
        .unwrap();
    db.write(|c| {
        let key = ItemKey::new(ItemKind::Album, &album);
        store::set_item(c, &key, "A", &tracks, 1)?;
        store::reconcile(c)?;
        Ok(())
    })
    .unwrap();
    let k = db.read(Kept::read).unwrap();
    assert!(k.items.is_empty() && k.tracks.is_empty(), "queued, not finished");
    assert!(db.read(shuffle_downloads_context).unwrap().track_ids.unwrap().is_empty());
    db.write(|c| {
        store::mark_done(c, &tracks[0], "/p", "original", "f0.flac", 10, 2)?;
        Ok(())
    })
    .unwrap();
    let k = db.read(Kept::read).unwrap();
    assert!(k.plays(&CarAction::Play(PlayContext { kind: ContextKind::Album, id: Some(album), track_ids: None })));
    assert_eq!(k.tracks, [tracks[0].clone()].into());
    assert_eq!(db.read(shuffle_downloads_context).unwrap().track_ids, Some(vec![tracks[0].clone()]));
    // Shuffle draws exactly the songs Kept counts: finished with a file.
    db.write(|c| {
        for t in &tracks[1..] {
            store::mark_done(c, t, "/p", "original", &format!("{t}.flac"), 10, 3)?;
        }
        Ok(())
    })
    .unwrap();
    let k = db.read(Kept::read).unwrap();
    let drawn: std::collections::HashSet<String> =
        db.read(shuffle_downloads_context).unwrap().track_ids.unwrap().into_iter().collect();
    assert_eq!(drawn, k.tracks);
}

#[tokio::test]
async fn downloaded_lists_what_is_kept_newest_first_and_plays_each() {
    use crate::downloads::store::{self, ItemKey, ItemKind};
    let (db, _dir) = synced(2, 2).await;
    let (album, tracks): (String, Vec<String>) = db
        .read(|c| {
            let album: String = c.query_row("SELECT id FROM albums ORDER BY id LIMIT 1", [], |r| r.get(0))?;
            let t = c.prepare("SELECT id FROM tracks ORDER BY id LIMIT 4")?.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok((album, t))
        })
        .unwrap();
    db.write(|c| {
        store::set_item(c, &ItemKey::new(ItemKind::Album, &album), "Ignored for albums", &tracks[..1], 1)?;
        store::set_item(c, &ItemKey::new(ItemKind::Liked, "liked"), "Liked", &tracks[1..2], 2)?;
        store::set_item(c, &ItemKey::new(ItemKind::Playlist, "p1"), "Road trip", &tracks[2..3], 3)?;
        store::set_item(c, &ItemKey::new(ItemKind::Mix, "daily-1"), "Daily Mix 1", &tracks[2..4], 4)?;
        Ok(())
    })
    .unwrap();
    let rows = db.read(downloaded).unwrap();
    let titles: Vec<_> = rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(&titles[..3], ["Daily Mix 1", "Road trip", "Liked songs"], "newest first");
    assert!(matches!(&rows[0].action, CarAction::Play(p) if p.kind == ContextKind::Tracks && p.track_ids.as_deref() == Some(&tracks[2..4])), "a mix plays the songs it was kept with");
    assert_eq!(rows[1].subtitle.as_deref(), Some("Playlist"));
    assert!(opens(&rows[1].action, ContextKind::Playlist, "p1"));
    assert!(opens(&rows[2].action, ContextKind::Liked, "liked"));
    // An album shows as the library has it, with who made it.
    assert_ne!(rows[3].title, "Ignored for albums");
    assert!(rows[3].subtitle.as_deref().is_some_and(|s| !s.is_empty()));
    assert!(opens(&rows[3].action, ContextKind::Album, &album));
}

#[tokio::test]
async fn downloaded_keeps_an_album_the_library_lost() {
    use crate::downloads::store::{self, ItemKey, ItemKind};
    let (db, _dir) = synced(1, 1).await;
    db.write(|c| store::set_item(c, &ItemKey::new(ItemKind::Album, "gone"), "Old album", &[], 1)).unwrap();
    let rows = db.read(downloaded).unwrap();
    assert_eq!(rows[0].title, "Old album", "its kept title, not a blank row");
    assert!(opens(&rows[0].action, ContextKind::Album, "gone"));
}

#[tokio::test]
async fn jump_back_in_holds_one_row_of_covers() {
    let (db, _dir) = synced(10, 3).await;
    db.write(|c| Ok(c.execute_batch("UPDATE albums SET last_viewed_at = 1000 + rowid, view_count = 1")?)).unwrap();
    let sections = db.read(home_sections).unwrap();
    let CarRow::Covers { items, .. } = &sections[1].rows[0] else { panic!("covers") };
    assert_eq!(items.len(), COVERS_MAX, "30 played albums, one row");
}

#[tokio::test]
async fn made_for_you_shows_at_most_the_cap_in_shelf_order() {
    let (db, _dir) = synced(6, 3).await;
    db.write(|c| {
        let ids: Vec<String> = c.prepare("SELECT id FROM tracks")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        for i in 0..MIXES_MAX + 3 {
            let mix = crate::mixes::store::Stored {
                id: format!("daily-{i}"),
                kind: crate::mixes::MixKind::Daily,
                title: format!("Daily Mix {i}"),
                description: String::new(),
                color: "#334455".into(),
                artist_ids: vec![],
                tag: None,
                tracks: ids.iter().map(|id| (id.clone(), false)).collect(),
            };
            crate::mixes::store::save_one(c, &mix, i, "2026-10-20", 0)?;
        }
        Ok(())
    })
    .unwrap();
    let sections = db.read(home_sections).unwrap();
    let made = sections.iter().find(|s| s.header.as_deref() == Some("Made for you")).expect("mixes");
    assert_eq!(made.rows.len(), MIXES_MAX);
    let CarRow::Item(first) = &made.rows[0] else { panic!() };
    assert_eq!(first.title, "Daily Mix 0");
    assert_eq!(first.subtitle, None, "no empty line under a mix without a description");
}

#[tokio::test]
async fn radio_offers_an_album_not_played_in_a_while() {
    let (db, _dir) = synced(3, 2).await;
    db.write(|c| Ok(c.execute_batch("UPDATE albums SET view_count = 1, last_viewed_at = 1000 WHERE rowid = 1")?)).unwrap();
    let rows = db.read(|c| radio(c, None)).unwrap();
    let old = rows.iter().find(|r| r.title == "One you haven't played in a while").expect("the forgotten album");
    assert!(old.subtitle.as_deref().is_some_and(|s| s.contains(" by ")));
    assert!(matches!(&old.action, CarAction::Play(p) if p.kind == ContextKind::Album));
}

fn opens(a: &CarAction, kind: ContextKind, id: &str) -> bool {
    matches!(a, CarAction::Open(CarList::Songs { kind: k, id: i, .. }) if *k == kind && i == id)
}

#[tokio::test]
async fn an_album_opens_to_play_shuffle_then_its_songs() {
    let (db, _dir) = synced(1, 1).await;
    let albums = db.read(|c| list(c, &CarList::Albums, 10.0)).unwrap();
    let CarAction::Open(which) = &albums[0].action else { panic!("opens") };
    assert_eq!(which.title(), albums[0].title, "titled with the album");
    let rows = db.read(|c| list(c, which, 10.0)).unwrap();
    assert_eq!(rows[0].title, "Play");
    assert_eq!(rows[0].subtitle.as_deref(), Some("3 songs"));
    assert!(matches!(&rows[0].action, CarAction::Start { context, shuffle: false } if context.kind == ContextKind::Album));
    assert_eq!(rows[1].title, "Shuffle");
    assert!(matches!(&rows[1].action, CarAction::Start { context, shuffle: true } if context.kind == ContextKind::Album));
    assert_eq!(rows.len(), 2 + 3, "then its three songs");
    for (n, r) in rows[2..].iter().enumerate() {
        assert!(
            matches!(&r.action, CarAction::PlayFrom { context, track_id, index } if context.kind == ContextKind::Album && !track_id.is_empty() && *index == n),
            "a song plays the album from its own place"
        );
        assert!(r.thumb.is_none(), "an album's songs share its cover");
    }
}

#[test]
fn an_opened_album_is_marked_while_it_plays() {
    let row = CarItem { title: "A".into(), subtitle: None, thumb: None, action: songs(ContextKind::Album, "al1", "A") };
    let now = NowKey { context: Some((ContextKind::Album, "al1".into())), album_id: None, track_id: Some("t2".into()) };
    assert!(is_playing(&row, &now));
    assert!(is_playing(&row, &NowKey { album_id: Some("al1".into()), ..Default::default() }));
    assert!(!is_playing(&row, &NowKey::default()));
    // Inside the opened album: the playing song is marked, Play and Shuffle aren't.
    let ctx = PlayContext { kind: ContextKind::Album, id: Some("al1".into()), track_ids: None };
    let song = |t: &str| CarItem { title: t.into(), subtitle: None, thumb: None, action: CarAction::PlayFrom { context: ctx.clone(), track_id: t.into(), index: 0 } };
    assert!(is_playing(&song("t2"), &now));
    assert!(!is_playing(&song("t1"), &now));
    let start = CarItem { title: "Play".into(), subtitle: None, thumb: None, action: CarAction::Start { context: ctx.clone(), shuffle: false } };
    assert!(!is_playing(&start, &now));
    // The same song playing from another list isn't this row.
    let elsewhere = NowKey { context: Some((ContextKind::Playlist, "p1".into())), track_id: Some("t2".into()), ..Default::default() };
    assert!(!is_playing(&song("t2"), &elsewhere));
}

#[test]
fn offline_a_kept_album_opens_and_its_kept_songs_play() {
    let mut kept = Kept::default();
    kept.items.insert((crate::downloads::store::ItemKind::Album, "al1".into()));
    kept.tracks.insert("t1".into());
    assert!(kept.plays(&songs(ContextKind::Album, "al1", "A")));
    assert!(!kept.plays(&songs(ContextKind::Album, "al2", "B")));
    let ctx = PlayContext { kind: ContextKind::Album, id: Some("al1".into()), track_ids: None };
    assert!(kept.plays(&CarAction::PlayFrom { context: ctx.clone(), track_id: "t1".into(), index: 0 }));
    assert!(!kept.plays(&CarAction::PlayFrom { context: ctx, track_id: "t2".into(), index: 1 }));
    // Kept playlists, Liked songs and mixes open with no signal too.
    use crate::downloads::store::ItemKind;
    kept.items.insert((ItemKind::Playlist, "p1".into()));
    kept.items.insert((ItemKind::Liked, String::new()));
    kept.items.insert((ItemKind::Mix, "m1".into()));
    assert!(kept.plays(&songs(ContextKind::Playlist, "p1", "P")));
    assert!(kept.plays(&songs(ContextKind::Liked, "liked", "Liked songs")));
    assert!(kept.plays(&songs(ContextKind::Mix, "m1", "M")));
    assert!(!kept.plays(&songs(ContextKind::Playlist, "p2", "Q")));
}

#[tokio::test]
async fn a_long_list_says_how_many_it_leaves_out() {
    let (db, _dir) = synced(1, 1).await;
    let ids: Vec<String> = db.read(|c| Ok(c.prepare("SELECT id FROM tracks")?.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?)).unwrap();
    let many: Vec<String> = ids.iter().cycle().take(150).cloned().collect();
    let mix = crate::mixes::store::Stored {
        id: "big".into(),
        kind: crate::mixes::MixKind::Daily,
        title: "Big".into(),
        description: String::new(),
        color: "#000000".into(),
        artist_ids: vec![],
        tag: None,
        tracks: many.into_iter().map(|id| (id, false)).collect(),
    };
    db.write(|c| crate::mixes::store::save_one(c, &mix, 0, "2026-10-20", 0)).unwrap();
    let rows = db.read(|c| list(c, &CarList::Songs { kind: ContextKind::Mix, id: "big".into(), title: "Big".into() }, 10.0)).unwrap();
    assert_eq!(rows.len(), LIST_MAX, "stays within the car's limit");
    let last = rows.last().unwrap();
    assert!(last.title.starts_with("Showing ") && last.title.ends_with(" songs"), "{}", last.title);
    assert_eq!(last.action, CarAction::Info);
}

