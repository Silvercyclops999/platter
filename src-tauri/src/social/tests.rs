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
        album_id: None,
        album: String::new(),
        cover: None,
        playing,
    };
    let rows = activity::build_rows(&[row("1", true), row("2", false)], &[], &[], None, NOW);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].track_id, "1");
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
    let follows = vec![FollowedArtist { id: "7".into(), title: "A".into(), thumb: None, followed_at: NOW + 20 }];
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
