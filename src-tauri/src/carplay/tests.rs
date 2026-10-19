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
    let albums = db.read(|c| list(c, &CarList::Albums)).unwrap();
    assert_eq!(albums.len(), LIST_MAX, "150 albums are cut to the car's limit");
    for a in &albums {
        assert!(!a.title.is_empty());
        assert!(a.subtitle.as_deref().is_some_and(|s| !s.is_empty()), "albums say who made them");
        assert!(matches!(&a.action, CarAction::Play(p) if p.kind == ContextKind::Album && p.id.is_some()));
    }
    let artists = db.read(|c| list(c, &CarList::Artists)).unwrap();
    assert_eq!(artists.len(), 30);
    assert!(artists.iter().all(|a| matches!(&a.action, CarAction::Open(CarList::Artist { id, name }) if !id.is_empty() && *name == a.title)));
    let added = db.read(|c| list(c, &CarList::RecentlyAdded)).unwrap();
    assert_eq!(added.len(), LIST_MAX);
}

#[tokio::test]
async fn an_empty_library_gives_empty_lists_not_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("library.db")).unwrap();
    for which in [CarList::RecentlyPlayed, CarList::RecentlyAdded, CarList::Playlists, CarList::Albums, CarList::Artists] {
        assert!(db.read(|c| list(c, &which)).unwrap().is_empty(), "{which:?}");
    }
    // Home still offers the shelves to open.
    let home = db.read(home).unwrap();
    assert_eq!(home.len(), 2);
    assert!(home.iter().all(|i| matches!(i.action, CarAction::Open(_))));
}

#[test]
fn library_tab_starts_with_liked_songs_then_the_shelves() {
    let tab = library();
    assert_eq!(tab[0].title, "Liked songs");
    assert!(matches!(&tab[0].action, CarAction::Play(p) if p.kind == ContextKind::Liked));
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
async fn an_artist_opens_popular_then_their_albums() {
    let (db, _dir) = synced(3, 4).await;
    let artists = db.read(|c| list(c, &CarList::Artists)).unwrap();
    let CarAction::Open(which) = &artists[0].action else { panic!("artists open a list") };
    let CarList::Artist { id, .. } = which else { panic!("an artist's list") };
    assert_eq!(which.title(), artists[0].title, "the list is titled with the artist's name");
    let rows = db.read(|c| list(c, which)).unwrap();
    assert_eq!(rows.len(), 5, "Popular and four albums");
    assert_eq!(rows[0].title, "Popular");
    assert!(matches!(&rows[0].action, CarAction::Play(p) if p.kind == ContextKind::Artist && p.id.as_ref() == Some(id)));
    let own = db.read(|c| queries::artist_albums(c, id, LIST_MAX)).unwrap();
    for (row, album) in rows[1..].iter().zip(&own) {
        assert_eq!(row.title, album.title);
        assert!(matches!(&row.action, CarAction::Play(p) if p.kind == ContextKind::Album && p.id.as_ref() == Some(&album.id)));
    }
}

#[tokio::test]
async fn an_artist_list_stays_short_and_an_unknown_artist_is_just_popular() {
    let (db, _dir) = synced(1, 150).await;
    let id = db.read(|c| list(c, &CarList::Artists)).unwrap().into_iter().find_map(|a| match a.action {
        CarAction::Open(CarList::Artist { id, .. }) => Some(id),
        _ => None,
    });
    let rows = db.read(|c| list(c, &CarList::Artist { id: id.unwrap(), name: "A".into() })).unwrap();
    assert_eq!(rows.len(), LIST_MAX, "Popular plus albums up to the car's limit");
    let none = db.read(|c| list(c, &CarList::Artist { id: "nobody".into(), name: "Nobody".into() })).unwrap();
    assert_eq!(none.len(), 1);
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
