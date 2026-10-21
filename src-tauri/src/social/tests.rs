//! Friend activity from faked sessions and history, follows and What's New,
//! and the release announcement. Listen along has its own tests
//! (`listen/tests.rs`).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use parking_lot::Mutex;

use super::activity::{self, Account, ActivitySource, ActivityState, LiveRow, PlayedRow};
use super::follows::{self, FollowStore, FollowedArtist, KvFollows};
use super::{Notifier, Social, SocialDeps, SocialSwitches};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::plex::state::EventSink;

const NOW: i64 = 1_800_000_000;

#[derive(Default)]
struct Events(Mutex<Vec<(String, serde_json::Value)>>);

impl EventSink for Events {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().push((event.to_string(), payload));
    }
}

impl Events {
    fn last(&self, name: &str) -> Option<serde_json::Value> {
        self.0.lock().iter().rev().find(|(e, _)| e == name).map(|(_, v)| v.clone())
    }
}

// ---- friend activity --------------------------------------------------------

const SESSIONS: &str = r#"{"MediaContainer":{"size":2,"Metadata":[
 {"type":"track","ratingKey":"501","title":"Song A","grandparentTitle":"Band","parentTitle":"Record","parentRatingKey":"50",
  "parentThumb":"/library/metadata/50/thumb/1","User":{"id":"7","title":"Sam","thumb":"https://plex.tv/users/x/avatar?c=1"},"Player":{"state":"playing"}},
 {"type":"episode","ratingKey":"9","title":"Show","User":{"id":"8","title":"Kim"}}
]}}"#;

const HISTORY: &str = r#"{"MediaContainer":{"size":4,"Metadata":[
 {"type":"track","ratingKey":"601","title":"Old Song","grandparentTitle":"Other","parentTitle":"LP","parentKey":"/library/metadata/60","viewedAt":1799999000,"accountID":9},
 {"type":"track","ratingKey":"602","title":"Older","grandparentTitle":"Other","parentTitle":"LP","parentKey":"/library/metadata/60","viewedAt":1799990000,"accountID":9},
 {"type":"track","ratingKey":"603","title":"Mine","grandparentTitle":"Me","parentTitle":"LP","viewedAt":1799999900,"accountID":1},
 {"type":"track","ratingKey":"604","title":"Ancient","grandparentTitle":"X","parentTitle":"Y","viewedAt":1700000000,"accountID":10},
 {"type":"track","ratingKey":"605","title":"Earlier Sam","grandparentTitle":"X","parentTitle":"Y","viewedAt":1799999990,"accountID":7}
]}}"#;

const ACCOUNTS: &str = r#"{"MediaContainer":{"size":4,"Account":[
 {"id":1,"name":"owner","thumb":""},{"id":7,"name":"Sam","thumb":""},{"id":9,"name":"Alex","thumb":""},{"id":10,"name":"Pat"},{"id":11,"name":""}
]}}"#;

#[test]
fn sessions_history_and_accounts_parse() {
    let live = activity::parse_sessions(SESSIONS.as_bytes()).unwrap();
    assert_eq!(live.len(), 1, "only music sessions");
    let l = &live[0];
    assert_eq!((l.account_id, l.track_id.as_str(), l.album_id.as_deref(), l.playing), (7, "501", Some("50"), true));
    assert_eq!(l.user_thumb.as_deref(), Some("/ext/https://plex.tv/users/x/avatar?c=1"));
    let h = activity::parse_history(HISTORY.as_bytes()).unwrap();
    assert_eq!(h.len(), 5);
    assert_eq!(h[0].album_id.as_deref(), Some("60"), "the album id comes from parentKey");
    let a = activity::parse_accounts(ACCOUNTS.as_bytes()).unwrap();
    assert_eq!(a.iter().map(|a| a.id).collect::<Vec<_>>(), vec![1, 7, 9, 10], "unnamed accounts are left out");
    assert!(a.iter().all(|a| a.thumb.is_none()));
    assert!(activity::parse_sessions(b"not json").is_err());
}

#[test]
fn one_row_per_person_newest_first_live_wins_own_and_old_left_out() {
    let live = activity::parse_sessions(SESSIONS.as_bytes()).unwrap();
    let played = activity::parse_history(HISTORY.as_bytes()).unwrap();
    let accounts = activity::parse_accounts(ACCOUNTS.as_bytes()).unwrap();
    let rows = activity::build_rows(&live, &played, &accounts, Some(1), NOW);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Sam", "Alex"], "own account and plays older than a week are left out");
    assert!(rows[0].now && rows[0].title == "Song A", "a live session beats Sam's history");
    assert_eq!((rows[1].title.as_str(), rows[1].at, rows[1].now), ("Old Song", 1_799_999_000, false), "Alex's latest play");
    // Without the live session, Sam's latest history row shows.
    let rows = activity::build_rows(&[], &played, &accounts, Some(1), NOW);
    assert_eq!(rows[0].name, "Sam");
    assert_eq!(rows[0].title, "Earlier Sam");
}

#[test]
fn a_paused_session_still_counts_but_a_playing_one_wins() {
    let row = |track: &str, playing: bool| LiveRow {
        account_id: 3,
        user_name: Some("Lee".into()),
        user_thumb: None,
        track_id: track.into(),
        title: track.into(),
        artist: String::new(),
        artist_id: None,
        album_artist: String::new(),
        album_id: None,
        album: String::new(),
        year: None,
        cover: None,
        playing,
    };
    let rows = activity::build_rows(&[row("1", true), row("2", false)], &[], &[], None, NOW);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].track_id.as_deref(), Some("1"));
    let only: Vec<PlayedRow> = Vec::new();
    let accounts = vec![Account { id: 3, name: "Lee".into(), thumb: None }];
    let rows = activity::build_rows(&[row("2", false)], &only, &accounts, None, NOW);
    assert!(rows[0].now);
}

struct FakeSource {
    owned: Option<bool>,
    refuse: bool,
}

#[async_trait]
impl ActivitySource for FakeSource {
    async fn owned(&self) -> Option<bool> {
        self.owned
    }
    async fn sessions(&self) -> AppResult<Vec<u8>> {
        if self.refuse {
            return Err(AppError::Auth("no".into()));
        }
        Ok(SESSIONS.as_bytes().to_vec())
    }
    async fn history(&self, _size: u64) -> AppResult<Vec<u8>> {
        Ok(HISTORY.as_bytes().to_vec())
    }
    async fn accounts(&self) -> AppResult<Vec<u8>> {
        Ok(ACCOUNTS.as_bytes().to_vec())
    }
}

#[tokio::test]
async fn admin_sees_activity_others_are_told_it_isnt_available() {
    let p = activity::read(&FakeSource { owned: Some(true), refuse: false }, NOW).await;
    assert_eq!(p.state, ActivityState::Ok);
    assert_eq!(p.friends.len(), 2);
    let p = activity::read(&FakeSource { owned: Some(false), refuse: false }, NOW).await;
    assert_eq!((p.state, p.friends.len()), (ActivityState::NotAdmin, 0));
    let p = activity::read(&FakeSource { owned: Some(true), refuse: true }, NOW).await;
    assert_eq!(p.state, ActivityState::NotAdmin, "the server refusing the read means no admin rights");
    let p = activity::read(&FakeSource { owned: None, refuse: false }, NOW).await;
    assert_eq!(p.state, ActivityState::Offline);
}

// ---- linking activity to this library -----------------------------------------

/// History as the Plex client hands it over: rating keys carry the server's
/// tag, but `parentKey` paths don't (and history rows have no
/// `parentRatingKey`).
const TAGGED_HISTORY: &str = r#"{"MediaContainer":{"size":2,"Metadata":[
 {"type":"track","ratingKey":"0a1b2c3d:30260","title":"Morning Light","grandparentTitle":"The Demo Band","parentTitle":"First Record","parentKey":"/library/metadata/30251","grandparentKey":"/library/metadata/30250","parentYear":2011,"viewedAt":1799999000,"accountID":9},
 {"type":"track","ratingKey":"0a1b2c3d:30261","title":"Evening","grandparentTitle":"The Demo Band","parentTitle":"First Record","parentRatingKey":"0a1b2c3d:30251","viewedAt":1799998000,"accountID":9}
]}}"#;

const HERE: &str = "0a1b2c3d";
const THERE: &str = "ffffffff";

fn add_artist(db: &DbHandle, id: &str, title: &str) {
    let (id, title) = (id.to_string(), title.to_string());
    db.write(move |c| {
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES (?1, ?2, ?2)", rusqlite::params![id, title])?;
        Ok(())
    })
    .unwrap();
}

fn add_record(db: &DbHandle, id: &str, title: &str, artist: (&str, &str), year: Option<i64>) {
    let (id, title, aid, artist) = (id.to_string(), title.to_string(), artist.0.to_string(), artist.1.to_string());
    db.write(move |c| {
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, year) VALUES (?1, ?2, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, title, aid, artist, year],
        )?;
        Ok(())
    })
    .unwrap();
}

fn add_song(db: &DbHandle, id: &str, title: &str, album: (&str, &str), artist: (&str, &str)) {
    let (id, title, alid, album, arid, artist) = (id.to_string(), title.to_string(), album.0.to_string(), album.1.to_string(), artist.0.to_string(), artist.1.to_string());
    db.write(move |c| {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '/library/parts/1/f.flac')",
            rusqlite::params![id, title, alid, album, arid, artist],
        )?;
        Ok(())
    })
    .unwrap();
}

/// The demo library: one artist, one album from 2011, two songs, all on
/// the server tagged [`HERE`].
fn library() -> (DbHandle, tempfile::TempDir) {
    let (db, dir) = db();
    let artist_id = format!("{HERE}:30250");
    let artist = (artist_id.as_str(), "The Demo Band");
    let album = format!("{HERE}:30251");
    add_artist(&db, artist.0, artist.1);
    add_record(&db, &album, "First Record", (artist.0, artist.1), Some(2011));
    add_song(&db, &format!("{HERE}:30260"), "Morning Light", (&album, "First Record"), (artist.0, artist.1));
    add_song(&db, &format!("{HERE}:30261"), "Evening", (&album, "First Record"), (artist.0, artist.1));
    (db, dir)
}

fn friend(track: &str, title: &str, artist: &str, album_id: Option<&str>, album: &str, year: Option<i64>) -> activity::FriendActivity {
    activity::FriendActivity {
        account_id: 9,
        name: "Alex".into(),
        thumb: None,
        track_id: Some(track.into()),
        title: title.into(),
        artist: artist.into(),
        artist_id: None,
        album_id: album_id.map(str::to_string),
        album: album.into(),
        cover: None,
        at: NOW,
        now: false,
        in_library: false,
        album_artist: artist.into(),
        year,
    }
}

fn placed(db: &DbHandle, mut rows: Vec<activity::FriendActivity>) -> Vec<activity::FriendActivity> {
    db.read(move |c| activity::place(c, &mut rows).map(|_| rows)).unwrap()
}

#[test]
fn history_album_and_artist_ids_carry_the_songs_server_tag() {
    let h = activity::parse_history(TAGGED_HISTORY.as_bytes()).unwrap();
    assert_eq!(h[0].track_id, format!("{HERE}:30260"));
    assert_eq!(h[0].album_id.as_deref(), Some(&*format!("{HERE}:30251")), "never a bare 30251");
    assert_eq!(h[0].artist_id.as_deref(), Some(&*format!("{HERE}:30250")));
    assert_eq!(h[0].year, Some(2011));
    assert_eq!(h[1].album_id.as_deref(), Some(&*format!("{HERE}:30251")), "a tagged parentRatingKey stays as it is");
}

#[test]
fn a_play_from_this_librarys_server_links_by_id() {
    let (db, _d) = library();
    let played = activity::parse_history(TAGGED_HISTORY.as_bytes()).unwrap();
    let accounts = vec![Account { id: 9, name: "Alex".into(), thumb: None }];
    let rows = placed(&db, activity::build_rows(&[], &played, &accounts, Some(1), NOW));
    let f = &rows[0];
    assert!(f.in_library);
    assert_eq!(f.track_id.as_deref(), Some(&*format!("{HERE}:30260")));
    assert_eq!(f.album_id.as_deref(), Some(&*format!("{HERE}:30251")));
    assert_eq!(f.artist_id.as_deref(), Some(&*format!("{HERE}:30250")));
    // The album it links to opens.
    let album = f.album_id.clone().unwrap();
    assert!(db.read(move |c| crate::db::queries::album_page(c, &album)).is_ok());
    // The bare rating key the panel used to link to isn't an album here.
    assert!(matches!(db.read(|c| crate::db::queries::album_page(c, "30251")), Err(AppError::NotFound(_))));
}

#[test]
fn a_play_from_another_server_is_found_here_by_name() {
    let (db, _d) = library();
    let rows = placed(
        &db,
        vec![
            // Case and punctuation fold; the album and year agree.
            friend(&format!("{THERE}:9"), "morning light!", "the demo band", Some(&format!("{THERE}:8")), "First Record", Some(2011)),
            // The song isn't here, but its album is.
            friend(&format!("{THERE}:10"), "B-Side", "The Demo Band", Some(&format!("{THERE}:8")), "First Record", None),
        ],
    );
    assert!(rows[0].in_library);
    assert_eq!(rows[0].track_id.as_deref(), Some(&*format!("{HERE}:30260")));
    assert_eq!(rows[0].album_id.as_deref(), Some(&*format!("{HERE}:30251")));
    assert_eq!(rows[0].artist_id.as_deref(), Some(&*format!("{HERE}:30250")));
    assert!(!rows[1].in_library);
    assert_eq!(rows[1].track_id, None);
    assert_eq!(rows[1].album_id.as_deref(), Some(&*format!("{HERE}:30251")));
    assert_eq!(rows[1].artist_id.as_deref(), Some(&*format!("{HERE}:30250")));
}

#[test]
fn a_play_with_no_match_here_is_not_linked() {
    let (db, _d) = library();
    let rows = placed(
        &db,
        vec![
            friend(&format!("{THERE}:9"), "Morning Light", "Someone Else", Some(&format!("{THERE}:8")), "Other Record", None),
            // Same names, another year: another album.
            friend(&format!("{THERE}:11"), "Unknown", "The Demo Band", Some(&format!("{THERE}:8")), "First Record", Some(1999)),
            // A bare id that isn't in this library.
            friend("30251", "Praise", "Nobody", Some("30251"), "Nothing", None),
        ],
    );
    for f in &rows {
        assert!(!f.in_library, "{}", f.title);
        assert_eq!(f.track_id, None);
    }
    assert_eq!((rows[0].album_id.as_deref(), rows[0].artist_id.as_deref()), (None, None));
    assert_eq!(rows[1].album_id, None, "the year rules the album out");
    assert_eq!(rows[1].artist_id.as_deref(), Some(&*format!("{HERE}:30250")), "the artist is still here");
    assert_eq!((rows[2].album_id.as_deref(), rows[2].artist_id.as_deref()), (None, None));
    let json = serde_json::to_value(&rows[2]).unwrap();
    assert_eq!(json["inLibrary"], false);
    assert!(json["trackId"].is_null() && json["albumId"].is_null());
    assert!(json.get("albumArtist").is_none() && json.get("year").is_none(), "matching hints stay in the core");
}

#[tokio::test]
async fn the_panel_only_links_what_this_library_has() {
    let r = rig();
    // The fake server's history uses ids this library doesn't have.
    let p = r.social.activity(true).await;
    assert_eq!(p.state, ActivityState::Ok);
    assert!(p.friends.iter().all(|f| !f.in_library && f.track_id.is_none() && f.album_id.is_none()));
}

// ---- follows and What's New ---------------------------------------------------

#[derive(Default)]
struct Notes(Mutex<Vec<(String, String)>>);

impl Notifier for Notes {
    fn notify(&self, title: &str, body: &str) {
        self.0.lock().push((title.into(), body.into()));
    }
}

fn db() -> (DbHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("s.db")).unwrap();
    (db, dir)
}

fn add_album(db: &DbHandle, id: &str, artist: &str, added: i64) {
    let (id, artist) = (id.to_string(), artist.to_string());
    db.write(move |c| {
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, added_at) VALUES (?1, ?2, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, format!("Album {id}"), artist, format!("Artist {artist}"), added],
        )?;
        Ok(())
    })
    .unwrap();
}

struct Rig {
    social: Social,
    db: DbHandle,
    notes: Arc<Notes>,
    events: Arc<Events>,
    now: Arc<Mutex<i64>>,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let (db, dir) = db();
    let notes = Arc::new(Notes::default());
    let events = Arc::new(Events::default());
    let now = Arc::new(Mutex::new(NOW));
    let clock = now.clone();
    let social = Social::new(SocialDeps {
        db: db.clone(),
        follows: Arc::new(KvFollows(db.clone())),
        activity: Arc::new(FakeSource { owned: Some(true), refuse: false }),
        events: events.clone(),
        notifier: notes.clone(),
        switches: SocialSwitches::default(),
        clock: Arc::new(move || *clock.lock()),
    });
    Rig { social, db, notes, events, now, _dir: dir }
}

#[test]
fn follow_unfollow_and_follow_again_keep_one_entry() {
    let r = rig();
    r.social.set_follow("7".into(), "Band".into(), None, true).unwrap();
    r.social.set_follow("credit:guest".into(), "Guest".into(), None, true).unwrap();
    let all = r.social.set_follow("7".into(), "Band".into(), None, true).unwrap();
    assert_eq!(all.len(), 2, "following twice keeps one");
    let all = r.social.set_follow("7".into(), "Band".into(), None, false).unwrap();
    assert_eq!(all.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["credit:guest"]);
    // Kept in the database, and the UI was told.
    assert_eq!(KvFollows(r.db.clone()).list().unwrap().len(), 1);
    assert_eq!(r.events.last(super::EVENT_FOLLOWS).unwrap().as_array().unwrap().len(), 1);
    assert!(r.social.set_follow(" ".into(), "x".into(), None, true).is_err());
}

#[test]
fn a_follow_can_just_show_new_albums_and_keeps_that_through_the_store() {
    let r = rig();
    assert!(r.social.set_just_show("7", true).is_err(), "only a followed artist");
    r.social.set_follow("7".into(), "Band".into(), None, true).unwrap();
    let all = r.social.set_just_show("7", true).unwrap();
    assert!(all[0].just_show);
    assert!(KvFollows(r.db.clone()).list().unwrap()[0].just_show, "kept");
    assert_eq!(r.events.last(super::EVENT_FOLLOWS).unwrap()[0]["justShow"], true, "the UI was told");
    // Following again keeps the choice; the usual follow leaves the field out.
    assert!(r.social.set_follow("7".into(), "Band".into(), None, true).unwrap()[0].just_show);
    let all = r.social.set_just_show("7", false).unwrap();
    assert!(serde_json::to_value(&all[0]).unwrap().get("justShow").is_none());
}

#[test]
fn whats_new_splits_new_and_earlier_and_counts_unseen() {
    let r = rig();
    add_album(&r.db, "1", "7", NOW - 10 * 86400);
    add_album(&r.db, "2", "8", NOW - 86400);
    // Nothing followed: everything recently added, and the first look is "seen".
    let w = r.social.whats_new().unwrap();
    assert!(!w.following);
    assert_eq!(w.items.len(), 2);
    assert_eq!(w.unseen, 0);
    r.social.set_follow("7".into(), "Artist 7".into(), None, true).unwrap();
    *r.now.lock() = NOW + 100;
    add_album(&r.db, "3", "7", NOW + 50);
    add_album(&r.db, "4", "7", NOW - 200 * 86400);
    let w = r.social.whats_new().unwrap();
    assert!(w.following);
    let ids: Vec<(&str, bool)> = w.items.iter().map(|i| (i.album.id.as_str(), i.new)).collect();
    assert_eq!(ids, [("3", true), ("1", false)], "followed only, newest first, old ones left out");
    assert_eq!(w.unseen, 1);
    r.social.whats_new_seen().unwrap();
    assert_eq!(r.social.whats_new().unwrap().unseen, 0);
    assert_eq!(r.events.last(super::EVENT_WHATS_NEW).unwrap()["unseen"], 0);
}

#[test]
fn new_releases_are_announced_once_only_when_on_and_not_private() {
    let r = rig();
    r.social.set_follow("7".into(), "Artist 7".into(), None, true).unwrap();
    add_album(&r.db, "1", "7", NOW - 5);
    // The first check only sets the mark.
    r.social.switches().notify_releases.store(true, Ordering::Relaxed);
    r.social.library_changed();
    assert!(r.notes.0.lock().is_empty());
    *r.now.lock() = NOW + 100;
    add_album(&r.db, "2", "7", NOW + 50);
    add_album(&r.db, "3", "8", NOW + 60);
    r.social.library_changed();
    assert_eq!(*r.notes.0.lock(), vec![("New from Artist 7".to_string(), "Album 2".to_string())]);
    // Once only.
    r.social.library_changed();
    assert_eq!(r.notes.0.lock().len(), 1);
    // Off, or a private session: nothing shows (and the mark still moves).
    add_album(&r.db, "5", "7", NOW + 70);
    r.social.switches().private_session.store(true, Ordering::Relaxed);
    r.social.library_changed();
    r.social.switches().private_session.store(false, Ordering::Relaxed);
    r.social.switches().notify_releases.store(false, Ordering::Relaxed);
    add_album(&r.db, "6", "7", NOW + 80);
    r.social.library_changed();
    assert_eq!(r.notes.0.lock().len(), 1);
    // The badge counts what's unseen while What's New announcements are on.
    assert!(r.events.last(super::EVENT_WHATS_NEW).unwrap()["unseen"].as_u64().unwrap() >= 1);
    r.social.switches().whats_new.store(false, Ordering::Relaxed);
    r.social.badge();
    assert_eq!(r.events.last(super::EVENT_WHATS_NEW).unwrap()["unseen"], 0);
}

#[test]
fn an_album_added_before_the_follow_isnt_announced() {
    let (db, _d) = db();
    add_album(&db, "1", "7", NOW + 10);
    let follows = vec![FollowedArtist { id: "7".into(), title: "A".into(), thumb: None, followed_at: NOW + 20, just_show: false }];
    let found = db.read(|c| follows::to_announce(c, &follows, NOW)).unwrap();
    assert!(found.albums.is_empty());
    add_album(&db, "2", "7", NOW + 30);
    let found = db.read(|c| follows::to_announce(c, &follows, NOW)).unwrap();
    assert_eq!(found.albums.len(), 1);
    assert_eq!(found.mark, NOW + 30);
}

#[test]
fn notices_name_one_artist_or_count_several() {
    let mk = |id: &str, artist: &str| {
        let (d, _x) = db();
        add_album(&d, id, artist, NOW);
        d.read(|c| follows::releases(c, &[], 0, 10)).unwrap().remove(0)
    };
    let a = mk("1", "7");
    let b = mk("2", "7");
    let c = mk("3", "8");
    assert_eq!(follows::notice(&[]), None);
    assert_eq!(follows::notice(std::slice::from_ref(&a)).unwrap().0, "New from Artist 7");
    assert_eq!(follows::notice(&[a.clone(), b]).unwrap().1, "Album 1 and 1 more");
    assert_eq!(follows::notice(&[a, c]).unwrap(), ("New releases".into(), "2 new releases from artists you follow".into()));
}

#[tokio::test]
async fn friend_activity_off_reads_nothing() {
    let r = rig();
    assert_eq!(r.social.activity(false).await.state, ActivityState::Ok);
    r.social.switches().show_activity.store(false, Ordering::Relaxed);
    assert_eq!(r.social.activity(false).await.state, ActivityState::Off);
}
