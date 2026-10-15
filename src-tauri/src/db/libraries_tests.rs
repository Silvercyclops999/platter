//! More than one library in the cache: each library syncs its own rows,
//! libraries left out go, and the same music on two servers shows once.

use std::collections::HashMap;
use std::sync::Arc;

use super::fake::{self, FakeLibrary, FakeSource};
use super::merge::{self, Rank, ServerView};
use super::source::LibrarySource;
use super::store;
use super::sync::ExtraLibrary;
use super::*;

const A: &str = "aaaaaaaa";
const B: &str = "bbbbbbbb";

struct Db {
    db: DbHandle,
    _dir: tempfile::TempDir,
}

fn open() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("library.db")).unwrap();
    Db { db, _dir: dir }
}

fn source(lib: FakeLibrary, tag: &str) -> Arc<FakeSource> {
    let s = Arc::new(FakeSource::new(fake::tagged(lib, tag)));
    *s.tag.lock() = Some(tag.to_string());
    s
}

fn extra(tag: &str, section: &str, src: &Arc<FakeSource>) -> ExtraLibrary {
    ExtraLibrary { src: store::src_of(tag, section), tag: tag.into(), source: src.clone() as Arc<dyn LibrarySource> }
}

fn n(db: &DbHandle, sql: &str) -> i64 {
    db.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
}

fn albums_in(db: &DbHandle, view: &ServerView) -> i64 {
    db.read(|c| queries::albums_view_on(c, AlbumSort::Name, 0, 500, None, crate::tags::TagView::default(), view).map(|p| p.total as i64))
        .unwrap()
}

/// The same albums on a second server (a shared copy), in a section 5.
fn copy_of(lib: &FakeLibrary) -> FakeLibrary {
    let mut l = lib.clone();
    l.sections = vec![fake::section("5", "uuid-b")];
    l.playlists = vec![(fake::playlist("900", "Theirs", 1), vec![l.tracks[0].id.clone()])];
    l
}

#[tokio::test]
async fn each_library_syncs_its_own_rows_and_the_same_music_shows_once() {
    let t = open();
    let mut lib = fake::generated(2, 2, 3);
    lib.playlists = vec![(fake::playlist("800", "Mine", 1), vec![lib.tracks[0].id.clone()])];
    let main = source(lib.clone(), A);
    let other = source(copy_of(&lib), B);
    t.db.set_extra_libraries(vec![extra(B, "5", &other)]);
    t.db.sync_once(main.clone(), true).await.unwrap();

    // Both libraries are in, each row marked with the library it came from.
    assert_eq!(n(&t.db, "SELECT count(*) FROM albums"), 8);
    assert_eq!(n(&t.db, "SELECT count(*) FROM albums WHERE src = ''"), 4);
    assert_eq!(n(&t.db, &format!("SELECT count(*) FROM tracks WHERE src = '{B}/5'")), 12);
    assert_eq!(n(&t.db, "SELECT count(*) FROM playlists"), 2, "each server's playlists");
    // Every album, song and artist on B is a copy of one on A.
    assert_eq!(n(&t.db, "SELECT count(*) FROM dupes WHERE kind = 'album'"), 4);
    assert_eq!(n(&t.db, "SELECT count(*) FROM dupes WHERE kind = 'track'"), 12);
    assert_eq!(n(&t.db, "SELECT count(*) FROM dupes WHERE kind = 'artist'"), 2);
    assert_eq!(n(&t.db, &format!("SELECT count(*) FROM dupes WHERE canon NOT LIKE '{A}:%'")), 0, "the main library's copy stands for each");
    assert_eq!(albums_in(&t.db, &ServerView::All), 4, "one card each");
    assert_eq!(albums_in(&t.db, &ServerView::Server(B.into())), 4);
    assert_eq!(albums_in(&t.db, &ServerView::Server(A.into())), 4);

    // An album gone from B goes from B's rows only.
    let gone = other.lib.lock().albums[0].id.clone();
    other.lib.lock().albums.retain(|a| a.id != gone);
    other.lib.lock().tracks.retain(|tr| tr.album_id != gone);
    t.db.sync_once(main.clone(), true).await.unwrap();
    assert_eq!(n(&t.db, "SELECT count(*) FROM albums WHERE src = ''"), 4);
    assert_eq!(n(&t.db, &format!("SELECT count(*) FROM albums WHERE src = '{B}/5'")), 3);
    assert_eq!(n(&t.db, "SELECT count(*) FROM dupes WHERE kind = 'album'"), 3);
    // A change on A doesn't touch B's rows either (incremental pass).
    main.lib.lock().albums.pop();
    t.db.sync_once(main.clone(), false).await.unwrap();
    assert_eq!(n(&t.db, "SELECT count(*) FROM albums WHERE src = ''"), 3);
    assert_eq!(n(&t.db, &format!("SELECT count(*) FROM albums WHERE src = '{B}/5'")), 3);

    // B left out: its rows, playlists and copies go; A's stay.
    t.db.set_extra_libraries(Vec::new());
    t.db.sync_once(main.clone(), false).await.unwrap();
    assert_eq!(n(&t.db, "SELECT count(*) FROM albums WHERE src <> ''"), 0);
    assert_eq!(n(&t.db, "SELECT count(*) FROM albums"), 3);
    assert_eq!(n(&t.db, "SELECT count(*) FROM dupes"), 0);
    assert_eq!(n(&t.db, &format!("SELECT count(*) FROM playlists WHERE id LIKE '{B}:%'")), 0);
    assert_eq!(n(&t.db, &format!("SELECT count(*) FROM playlists WHERE id LIKE '{A}:%'")), 1);
    assert_eq!(n(&t.db, "SELECT count(*) FROM kv WHERE key LIKE 'sync.src.%'"), 0);
}

#[tokio::test]
async fn the_best_copy_plays_and_likes_stay_with_their_server() {
    let t = open();
    let lib = fake::generated(1, 1, 2);
    let main = source(lib.clone(), A);
    let mut theirs = copy_of(&lib);
    // B's copy of the album is lossy; A's is lossless.
    for tr in &mut theirs.tracks {
        tr.codec = Some("mp3".into());
        tr.bitrate = Some(320);
    }
    // A song liked on B only.
    theirs.rated = Some(vec![likes::Rated { track_id: crate::sid::qualify(B, &lib.tracks[1].id), rating: 10.0, rated_at: Some(5) }]);
    main.lib.lock().rated = Some(Vec::new());
    let other = source(theirs, B);
    t.db.set_extra_libraries(vec![extra(B, "5", &other)]);
    t.db.sync_once(main.clone(), true).await.unwrap();

    let liked = n(&t.db, "SELECT count(*) FROM tracks WHERE user_rating IS NOT NULL");
    assert_eq!(liked, 1, "A's sync didn't clear the like B has");
    let b_liked: String = t.db.read(|c| Ok(c.query_row("SELECT id FROM tracks WHERE user_rating IS NOT NULL", [], |r| r.get(0))?)).unwrap();
    assert!(b_liked.starts_with(B));

    let a_song = crate::sid::qualify(A, &lib.tracks[0].id);
    let play = |ranks: HashMap<String, Rank>| {
        let id = a_song.clone();
        t.db.read(move |c| {
            let mut tr = queries::tracks_by_ids(c, &[id])?;
            merge::best_copies(c, &mut tr, &ranks)?;
            Ok(tr[0].id.clone())
        })
        .unwrap()
    };
    // B on the local network, A over the relay: B's copy plays, so its
    // plays and play queue go to B, which has the file.
    let b_song = play([(A.to_string(), Rank::Relay), (B.to_string(), Rank::Local)].into());
    assert!(b_song.starts_with(B), "{b_song}");
    assert_eq!(crate::sid::owner(&b_song).as_deref(), Some(B));
    // Reached the same way: the better file.
    assert_eq!(play([(A.to_string(), Rank::Remote), (B.to_string(), Rank::Remote)].into()), a_song);
}

#[tokio::test]
async fn the_main_library_replaced_keeps_the_other_libraries() {
    let t = open();
    let lib = fake::generated(1, 2, 2);
    let main = source(lib.clone(), A);
    let other = source(copy_of(&lib), B);
    t.db.set_extra_libraries(vec![extra(B, "5", &other)]);
    t.db.sync_once(main.clone(), true).await.unwrap();
    // The main server's library was deleted and made again (new uuid).
    main.lib.lock().sections = vec![fake::section("1", "uuid-new")];
    t.db.sync_once(main.clone(), false).await.unwrap();
    assert_eq!(n(&t.db, "SELECT count(*) FROM albums WHERE src = ''"), 2);
    assert_eq!(n(&t.db, &format!("SELECT count(*) FROM albums WHERE src = '{B}/5'")), 2);
}

#[tokio::test]
async fn the_main_library_listed_again_isnt_synced_twice() {
    let t = open();
    let lib = fake::generated(1, 1, 2);
    let main = source(lib.clone(), A);
    // The main library itself, as a list entry would name it.
    t.db.set_extra_libraries(vec![extra(A, "1", &main)]);
    t.db.sync_once(main.clone(), true).await.unwrap();
    assert_eq!(n(&t.db, "SELECT count(*) FROM albums"), 1);
    assert_eq!(n(&t.db, "SELECT count(*) FROM albums WHERE src = ''"), 1);
}
