//! Explicit marks from Deezer: parsing and matching, the background round
//! (Deezer down, busy, no match, unreleased albums left alone, the setting
//! off), the file tag winning, and where the marks show.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::json;

use super::deezer::{self, DzAlbum, DzTrack, LocalTrack};
use super::*;
use crate::artistinfo::http::fake::FakeHttp;
use crate::model::{Album, ReleaseSection, Track};
use crate::tags::store::{self as tag_store, Status};
use crate::tags::{FileTags, TagView};

const BASE: &str = "https://dz.test/";

fn album(id: &str, title: &str, subformats: &[&str]) -> Album {
    Album {
        mbid: None,
        id: id.into(),
        title: title.into(),
        sort_title: title.into(),
        artist_id: "ar".into(),
        artist_title: "Rue Ellis".into(),
        year: Some(2020),
        thumb: None,
        format: Some("album".into()),
        subformats: subformats.iter().map(|s| s.to_string()).collect(),
        section: ReleaseSection::Albums,
        genres: vec![],
        track_count: 3,
        added_at: 1,
        view_count: 0,
        last_viewed_at: None,
        tint: None,
        released: None,
        studio: None,
        matched: Some(true),
        styles: vec![],
        moods: vec![],
    }
}

fn track(id: &str, album_id: &str, n: u32, title: &str) -> Track {
    let mut t = crate::audio::testutil::track(id, "Rue Ellis", &format!("/library/parts/{id}/100/file.flac"));
    t.album_id = album_id.into();
    t.artist_id = "ar".into();
    t.title = title.into();
    t.track_number = n;
    t.duration_ms = 200_000 + n as u64 * 1000;
    t
}

fn db_with(albums: &[Album], tracks: &[Track]) -> (DbHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    db.write(|c| {
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('ar', 'Rue Ellis', 'Rue Ellis')", [])?;
        let tx = c.transaction()?;
        crate::db::store::upsert_albums(&tx, albums, 1)?;
        crate::db::store::upsert_tracks(&tx, tracks, 1)?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    (db, dir)
}

/// One album, "Night Drive", three songs.
fn library() -> (DbHandle, tempfile::TempDir) {
    db_with(
        &[album("a1", "Night Drive", &[])],
        &[track("t1", "a1", 1, "Neon"), track("t2", "a1", 2, "Slow Lane (feat. Mara)"), track("t3", "a1", 3, "Exit")],
    )
}

fn search_answer() -> String {
    json!({ "data": [
        { "id": 7, "title": "Night Drive", "nb_tracks": 3, "explicit_lyrics": false, "artist": { "name": "Rue Ellis" } },
        { "id": 8, "title": "Night Drive", "nb_tracks": 3, "explicit_lyrics": true, "artist": { "name": "Rue Ellis" } },
        { "id": 9, "title": "Night Drive", "nb_tracks": 3, "explicit_lyrics": true, "artist": { "name": "Someone Else" } }
    ]})
    .to_string()
}

fn tracks_answer() -> String {
    json!({ "data": [
        { "title": "Neon", "title_short": "Neon", "disk_number": 1, "track_position": 1, "duration": 201,
          "explicit_lyrics": true, "explicit_content_lyrics": 1 },
        { "title": "Slow Lane", "title_short": "Slow Lane", "disk_number": 1, "track_position": 2, "duration": 202,
          "explicit_lyrics": false, "explicit_content_lyrics": 0 },
        { "title": "Exit", "title_short": "Exit", "disk_number": 1, "track_position": 3, "duration": 203,
          "explicit_lyrics": false, "explicit_content_lyrics": 2 }
    ]})
    .to_string()
}

fn deezer_ok() -> Arc<FakeHttp> {
    let http = Arc::new(FakeHttp::new());
    http.route("search/album", 200, &search_answer());
    http.route("album/8/tracks", 200, &tracks_answer());
    http
}

async fn round(db: &DbHandle, http: &FakeHttp) -> (usize, usize) {
    let told = Arc::new(AtomicUsize::new(0));
    let t = told.clone();
    let n = run_round(db, http, BASE, Duration::ZERO, &move || {
        t.fetch_add(1, Ordering::SeqCst);
    })
    .await;
    (n, told.load(Ordering::SeqCst))
}

fn file_tag(db: &DbHandle, track_id: &str, explicit: Option<bool>) {
    db.write(|c| {
        let part = format!("/library/parts/{track_id}/100/file.flac");
        tag_store::save(c, &part, Status::Ok, &FileTags { explicit, ..Default::default() }, 1)?;
        tag_store::refresh_albums(c)
    })
    .unwrap();
}

// ------------------------------------------------------------------ parsing and matching

#[test]
fn reads_albums_and_each_songs_advisory() {
    let v: serde_json::Value = serde_json::from_str(&search_answer()).unwrap();
    let a = deezer::albums(&v);
    assert_eq!(a.len(), 3);
    assert_eq!(a[1], DzAlbum { id: 8, title: "Night Drive".into(), artist: "Rue Ellis".into(), tracks: 3, explicit: true });
    let v: serde_json::Value = serde_json::from_str(&tracks_answer()).unwrap();
    let t: Vec<Option<bool>> = deezer::tracks(&v).into_iter().map(|t| t.explicit).collect();
    // Unknown (2) stays unknown rather than counting as clean.
    assert_eq!(t, vec![Some(true), Some(false), None]);
    assert!(deezer::albums(&json!({ "error": { "code": 4 } })).is_empty());
    assert!(deezer::is_error(&json!({ "error": { "code": 4, "message": "Quota limit exceeded" } })));
}

#[test]
fn edited_songs_are_clean_and_no_advice_is_unknown() {
    let v = json!({ "data": [
        { "title": "A", "explicit_lyrics": false, "explicit_content_lyrics": 3 },
        { "title": "B", "explicit_lyrics": false, "explicit_content_lyrics": 6 },
        { "title": "C", "explicit_lyrics": true }
    ]});
    let t: Vec<Option<bool>> = deezer::tracks(&v).into_iter().map(|t| t.explicit).collect();
    assert_eq!(t, vec![Some(false), None, Some(true)]);
}

fn dz(id: u64, title: &str, artist: &str, tracks: u32, explicit: bool) -> DzAlbum {
    DzAlbum { id, title: title.into(), artist: artist.into(), tracks, explicit }
}

#[test]
fn picks_the_explicit_edition_unless_ours_says_clean() {
    let found = [dz(1, "Night Drive", "Rue Ellis", 12, false), dz(2, "Night Drive", "Rue Ellis", 12, true)];
    assert_eq!(deezer::pick("Night Drive", "Rue Ellis", 12, &found).map(|a| a.id), Some(2));
    assert_eq!(deezer::pick("Night Drive (Clean)", "Rue Ellis", 12, &found).map(|a| a.id), Some(1));
    assert_eq!(deezer::pick("Night Drive [Edited Version]", "Rue Ellis", 12, &found).map(|a| a.id), Some(1));
}

#[test]
fn picks_by_artist_title_and_song_count() {
    let found = [
        dz(1, "Night Drive", "Somebody Else", 12, true),
        dz(2, "Night Drive (Deluxe Edition)", "Rue Ellis", 18, true),
        dz(3, "Night Drive (Explicit)", "Rue Ellis", 12, true),
        dz(4, "Night Moves", "Rue Ellis", 12, true),
    ];
    assert_eq!(deezer::pick("Night Drive", "Rue Ellis", 12, &found).map(|a| a.id), Some(3));
    assert_eq!(deezer::pick("Night Drive (Deluxe Edition)", "Rue Ellis", 18, &found).map(|a| a.id), Some(2));
    assert_eq!(deezer::pick("Night Drive", "The Rue Ellis feat. Mara", 12, &found).map(|a| a.id), Some(3));
    assert_eq!(deezer::pick("Daylight", "Rue Ellis", 12, &found), None);
    assert_eq!(deezer::pick("Night Drive", "Nobody", 12, &found), None);
}

fn lt(id: &str, title: &str, n: u32, secs: u64) -> LocalTrack {
    LocalTrack { id: id.into(), title: title.into(), disc: 1, number: n, duration_ms: secs * 1000 }
}

fn dt(title: &str, n: u32, secs: u64, explicit: Option<bool>) -> DzTrack {
    DzTrack { title: title.into(), disc: 1, position: n, duration_s: secs, explicit }
}

#[test]
fn matches_songs_by_title_then_by_place() {
    let local = [lt("x", "Neon (feat. Mara)", 1, 200), lt("y", "Track 2", 2, 180), lt("z", "Gone", 3, 100)];
    let dz = [dt("Neon", 1, 201, Some(true)), dt("Slow Lane", 2, 182, Some(true)), dt("Exit", 3, 240, Some(true))];
    // "Gone" has no title match and its length is far from song 3's.
    assert_eq!(deezer::match_tracks(&local, &dz), vec![("x".to_string(), true), ("y".to_string(), true)]);
    // Different song counts: titles only.
    assert_eq!(deezer::match_tracks(&local[..2], &dz), vec![("x".to_string(), true)]);
}

// ------------------------------------------------------------------ the round

#[tokio::test]
async fn marks_explicit_songs_and_their_album() {
    let (db, _d) = library();
    let http = deezer_ok();
    assert_eq!(round(&db, &http).await, (1, 1));
    // The explicit edition's songs, not the clean one's or another artist's.
    assert_eq!(http.calls("album/8/tracks"), 1);
    db.read(|c| {
        assert!(track_explicit(c, "t1")?);
        assert!(!track_explicit(c, "t2")?);
        assert!(!track_explicit(c, "t3")?);
        assert!(album_explicit(c, "a1")?);
        let ix = tag_store::index(c)?;
        assert!(ix.tracks["t1"].explicit);
        assert!(!ix.tracks.contains_key("t2"));
        assert!(ix.albums["a1"].explicit);
        // Unknown songs aren't stored as clean.
        let stored: i64 = c.query_row("SELECT count(*) FROM explicit_found", [], |r| r.get(0))?;
        assert_eq!(stored, 2);
        Ok(())
    })
    .unwrap();
    // Answered: not asked again until it's due.
    assert_eq!(round(&db, &http).await, (0, 0));
    assert_eq!(http.calls("search/album"), 1);
}

#[tokio::test]
async fn the_files_own_tag_wins() {
    let (db, _d) = library();
    file_tag(&db, "t1", Some(false));
    file_tag(&db, "t2", Some(true));
    round(&db, &deezer_ok()).await;
    db.read(|c| {
        assert!(!track_explicit(c, "t1")?, "the file says clean");
        assert!(track_explicit(c, "t2")?, "the file says explicit");
        let ix = tag_store::index(c)?;
        assert!(!ix.tracks.get("t1").is_some_and(|b| b.explicit));
        assert!(ix.tracks["t2"].explicit);
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn albums_every_file_settles_are_never_asked() {
    let (db, _d) = library();
    for t in ["t1", "t2", "t3"] {
        file_tag(&db, t, Some(false));
    }
    let http = deezer_ok();
    assert_eq!(round(&db, &http).await, (0, 0));
    assert!(http.urls().is_empty());
}

#[tokio::test]
async fn unreleased_albums_are_never_asked() {
    let mut demo = album("a1", "Night Drive", &["demo"]);
    demo.section = ReleaseSection::Demos;
    let (db, _d) = db_with(&[demo], &[track("t1", "a1", 1, "Neon")]);
    let http = deezer_ok();
    assert_eq!(round(&db, &http).await, (0, 0));
    assert!(http.urls().is_empty(), "nothing about unreleased music leaves the device");
}

#[tokio::test]
async fn deezer_down_stores_nothing_and_asks_again_next_round() {
    let (db, _d) = library();
    let http = Arc::new(FakeHttp::new());
    http.fail("search/album");
    assert_eq!(round(&db, &http).await, (0, 0));
    let checked: i64 = db.read(|c| Ok(c.query_row("SELECT count(*) FROM explicit_check", [], |r| r.get(0))?)).unwrap();
    assert_eq!(checked, 0);
    // Back up: the album is asked.
    http.route("search/album", 200, &search_answer());
    http.route("album/8/tracks", 200, &tracks_answer());
    assert_eq!(round(&db, &http).await, (1, 1));
}

#[tokio::test]
async fn deezer_busy_or_answering_an_error_stops_the_round() {
    for (status, body) in [(429, "{}".to_string()), (200, json!({ "error": { "code": 4 } }).to_string()), (200, "not json".to_string())] {
        let (db, _d) = db_with(
            &[album("a1", "Night Drive", &[]), album("a2", "Daylight", &[])],
            &[track("t1", "a1", 1, "Neon"), track("t2", "a2", 1, "Sun")],
        );
        let http = Arc::new(FakeHttp::new());
        http.route("search/album", status, &body);
        assert_eq!(round(&db, &http).await, (0, 0), "{status} {body}");
        assert_eq!(http.calls("search/album"), 1, "the rest wait for the next round");
    }
}

#[tokio::test]
async fn no_match_is_kept_and_asked_again_in_two_weeks() {
    let (db, _d) = library();
    let http = Arc::new(FakeHttp::new());
    http.route("search/album", 200, &json!({ "data": [] }).to_string());
    assert_eq!(round(&db, &http).await, (1, 0));
    // The field search, then the looser one.
    assert_eq!(http.calls("search/album"), 2);
    let (found, ttl): (bool, i64) = db
        .read(|c| Ok(c.query_row("SELECT found, expires_at - checked_at FROM explicit_check WHERE album_id = 'a1'", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .unwrap();
    assert!(!found);
    assert_eq!(ttl, MISS_TTL);
    assert_eq!(round(&db, &http).await, (0, 0));
}

#[tokio::test]
async fn the_setting_off_asks_nothing() {
    let (db, _d) = library();
    let http = deezer_ok();
    set_enabled(false);
    assert_eq!(round(&db, &http).await, (0, 0));
    set_enabled(true);
    assert!(http.urls().is_empty());
}

#[tokio::test]
async fn filtered_explicit_content_leaves_out_songs_deezer_marks() {
    let (db, _d) = library();
    round(&db, &deezer_ok()).await;
    let view = TagView { filter_explicit: true, ..TagView::default() };
    let mut list = vec![track("t1", "a1", 1, "Neon"), track("t2", "a1", 2, "Slow Lane")];
    let skipped = db.read(|c| tag_store::drop_hidden(c, &mut list, view)).unwrap();
    assert_eq!(skipped.explicit, 1);
    assert_eq!(list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["t2"]);
}

#[tokio::test]
async fn a_new_answer_replaces_the_old_marks() {
    let (db, _d) = library();
    round(&db, &deezer_ok()).await;
    let changed = db.write(|c| put(c, "a1", Some(&[("t2".to_string(), true)]), now_s())).unwrap();
    assert!(changed);
    db.read(|c| {
        assert!(!track_explicit(c, "t1")?);
        assert!(track_explicit(c, "t2")?);
        Ok(())
    })
    .unwrap();
    let same = db.write(|c| put(c, "a1", Some(&[("t2".to_string(), true)]), now_s())).unwrap();
    assert!(!same, "nothing to read again");
}

// ------------------------------------------------------------------ the car

#[tokio::test]
async fn carplay_rows_get_the_explicit_mark() {
    use crate::audio::source::{ContextKind, PlayContext};
    use crate::carplay::{CarAction, CarItem, CarList};
    let (db, _d) = library();
    round(&db, &deezer_ok()).await;
    let ctx = PlayContext { kind: ContextKind::Album, id: Some("a1".into()), track_ids: None };
    let row = |action| CarItem { title: "x".into(), subtitle: None, thumb: None, action };
    let neon = row(CarAction::PlayFrom { context: ctx.clone(), track_id: "t1".into(), index: 0 });
    let slow = row(CarAction::PlayFrom { context: ctx.clone(), track_id: "t2".into(), index: 1 });
    let album = row(CarAction::Open(CarList::Songs { kind: ContextKind::Album, id: "a1".into(), title: "Night Drive".into() }));
    let start = row(CarAction::Start { context: ctx, shuffle: false });
    let items = [neon, slow, album, start];
    let marks = db.read(|c| crate::carplay::explicit_rows(c, &items)).unwrap();
    assert_eq!(items.iter().map(|i| marks.marks(i)).collect::<Vec<_>>(), vec![true, false, true, false]);
}
