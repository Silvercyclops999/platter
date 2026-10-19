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
    let albums = db.read(|c| list(c, CarList::Albums)).unwrap();
    assert_eq!(albums.len(), LIST_MAX, "150 albums are cut to the car's limit");
    for a in &albums {
        assert!(!a.title.is_empty());
        assert!(a.subtitle.as_deref().is_some_and(|s| !s.is_empty()), "albums say who made them");
        assert!(matches!(&a.action, CarAction::Play(p) if p.kind == ContextKind::Album && p.id.is_some()));
    }
    let artists = db.read(|c| list(c, CarList::Artists)).unwrap();
    assert_eq!(artists.len(), 30);
    assert!(artists.iter().all(|a| matches!(&a.action, CarAction::Play(p) if p.kind == ContextKind::Artist)));
    let added = db.read(|c| list(c, CarList::RecentlyAdded)).unwrap();
    assert_eq!(added.len(), LIST_MAX);
}

#[tokio::test]
async fn an_empty_library_gives_empty_lists_not_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("library.db")).unwrap();
    for which in [CarList::RecentlyPlayed, CarList::RecentlyAdded, CarList::Playlists, CarList::Albums, CarList::Artists] {
        assert!(db.read(|c| list(c, which)).unwrap().is_empty(), "{which:?}");
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
