use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, params};

use super::plex::{PlexFeed, PlexServer, account_named, parse_history};
use super::store::{self, Fetched};
use super::tautulli::{self, Tautulli};
use super::*;
use crate::db::listens::{ALL_PLAYS, ImportSource, PlayStart, all_plays, end_play, start_play};
use crate::identity::index;
use crate::plex::keychain::{MemoryStore, TokenStore};
use crate::plex::token::Token;
use crate::web::fake::FakeWeb;

const T: &str = "aaaaaaaa";
/// 2026-10-01 12:00 UTC, in ms.
const NOW: i64 = 1_790_856_000_000;

fn tid(n: &str) -> String {
    format!("{T}:{n}")
}

fn song(c: &Connection, id: &str, artist: &str, album: &str, title: &str) {
    c.execute(
        "INSERT OR IGNORE INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, matched)
         VALUES (?1, ?2, ?2, ?3, ?4, 1, ?5, 1)",
        params![format!("{T}:al:{album}"), album, format!("{T}:ar:{artist}"), artist, T],
    )
    .unwrap();
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, src)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 200000, ?7, ?8)",
        params![tid(id), title, format!("{T}:al:{album}"), album, format!("{T}:ar:{artist}"), artist, format!("/p/{id}"), T],
    )
    .unwrap();
}

/// A small library: three songs.
fn library(c: &Connection) {
    song(c, "1", "Low Tide", "Shoreline", "Golden Hour");
    song(c, "2", "Low Tide", "Shoreline", "Undertow");
    song(c, "3", "Echo Field", "Static", "Ghost Town");
    index::rebuild(c).unwrap();
}

fn db() -> (tempfile::TempDir, DbHandle) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("h.db")).unwrap();
    db.write(|c| {
        library(c);
        Ok(())
    })
    .unwrap();
    (dir, db)
}

fn count(db: &DbHandle, sql: &str) -> i64 {
    db.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
}

fn plays_of(db: &DbHandle, source: &str) -> Vec<(String, Option<String>, String)> {
    db.read(|c| {
        let mut st =
            c.prepare("SELECT source_id, rating_key, title FROM imported_plays WHERE source = ?1 ORDER BY played_at, source_id")?;
        let r = st.query_map([source], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(r)
    })
    .unwrap()
}

// ---------------------------------------------------------------- a fake Plex server

/// One history entry: id, account, rating key (raw), title, artist, album, viewed (s).
type Entry = (u64, i64, &'static str, &'static str, &'static str, &'static str, i64);

#[derive(Default)]
struct FakePlex {
    entries: Mutex<Vec<Entry>>,
    account: Mutex<Option<i64>>,
    fail: Mutex<Option<crate::error::AppError>>,
    asked: Mutex<Vec<(i64, u64, u64)>>,
}

impl FakePlex {
    fn new(account: i64, entries: Vec<Entry>) -> Arc<Self> {
        Arc::new(Self { entries: Mutex::new(entries), account: Mutex::new(Some(account)), ..Default::default() })
    }
}

fn body(entries: &[Entry], total: usize) -> Vec<u8> {
    let meta: Vec<serde_json::Value> = entries
        .iter()
        .map(|(id, acct, key, title, artist, album, at)| {
            serde_json::json!({
                "historyKey": format!("/status/sessions/history/{id}"),
                "ratingKey": if key.is_empty() { String::new() } else { format!("{T}:{key}") },
                "type": "track", "title": title, "grandparentTitle": artist, "parentTitle": album,
                "viewedAt": at, "accountID": acct, "deviceID": 3
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({"MediaContainer": {"size": meta.len(), "totalSize": total, "Metadata": meta}})).unwrap()
}

#[async_trait]
impl PlexServer for FakePlex {
    async fn ready(&self) -> bool {
        true
    }
    async fn own_account(&self) -> AppResult<Option<i64>> {
        Ok(*self.account.lock())
    }
    async fn username(&self) -> Option<String> {
        Some("rue".into())
    }
    async fn tag(&self) -> Option<String> {
        Some(T.into())
    }
    async fn history(&self, account: i64, start: u64, size: u64) -> AppResult<Vec<u8>> {
        self.asked.lock().push((account, start, size));
        if let Some(e) = self.fail.lock().take() {
            return Err(e);
        }
        // Newest first, as Plex sorts with viewedAt:desc. The account filter
        // on the server is ignored here, so the import's own check is what
        // keeps other people's plays out.
        let mut all = self.entries.lock().clone();
        all.sort_by_key(|e| std::cmp::Reverse(e.6));
        let page: Vec<Entry> = all.iter().skip(start as usize).take(size as usize).cloned().collect();
        Ok(body(&page, all.len()))
    }
}

struct Clock(AtomicI64);

impl Clock {
    fn at(ms: i64) -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(ms)))
    }
    fn add(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

struct Sink(Mutex<Vec<String>>);

impl crate::plex::state::EventSink for Sink {
    fn emit(&self, event: &str, _: serde_json::Value) {
        self.0.lock().push(event.to_string());
    }
}

fn service(db: &DbHandle, feeds: Vec<Arc<dyn Feed>>, clock: &Arc<Clock>, page_size: u64) -> PlayHistory {
    let c = clock.clone();
    PlayHistory::new(Deps {
        db: db.clone(),
        feeds,
        events: Arc::new(Sink(Mutex::new(Vec::new()))),
        clock: Arc::new(move || c.0.load(Ordering::SeqCst)),
        pause: Duration::ZERO,
        page_size,
    })
}

const S: i64 = 1_790_000_000; // seconds, a little before NOW

fn plex_entries() -> Vec<Entry> {
    vec![
        (101, 1, "1", "Golden Hour", "Low Tide", "Shoreline", S),
        (102, 1, "2", "Undertow", "Low Tide", "Shoreline", S + 600),
        // Another Home user's plays: never counted.
        (103, 7, "1", "Golden Hour", "Low Tide", "Shoreline", S + 700),
        (104, 7, "3", "Ghost Town", "Echo Field", "Static", S + 800),
        // Deleted from the library since; its names still match a song.
        (105, 1, "999", "Ghost Town", "Echo Field", "Static", S + 900),
        // Deleted and nothing like it in the library: kept by its names.
        (106, 1, "998", "Gone Song", "Nobody", "Lost", S + 1000),
    ]
}

#[tokio::test]
async fn plex_history_pages_through_the_users_own_plays_only() {
    let (_d, db) = db();
    let plex = FakePlex::new(1, plex_entries());
    let clock = Clock::at(NOW);
    let ph = service(&db, vec![Arc::new(PlexFeed::new(plex.clone()))], &clock, 2);
    assert_eq!(ph.run(ImportSource::PlexHistory).await, Outcome::Done(4));
    // Three full pages of two, then an empty one ends it.
    let asked = plex.asked.lock().clone();
    assert_eq!(asked, vec![(1, 0, 2), (1, 2, 2), (1, 4, 2), (1, 6, 2)], "the user's account is asked for, page by page");
    let got = plays_of(&db, "plex_history");
    assert_eq!(
        got,
        vec![
            (format!("{T}:101"), Some(tid("1")), "Golden Hour".to_string()),
            (format!("{T}:102"), Some(tid("2")), "Undertow".to_string()),
            (format!("{T}:105"), Some(tid("3")), "Ghost Town".to_string()),
            (format!("{T}:106"), None, "Gone Song".to_string()),
        ],
        "other accounts left out; a deleted song maps by its names or stays unmatched"
    );
    let p = db.read(|c| store::progress(c, ImportSource::PlexHistory)).unwrap();
    assert!(p.full_done && p.error.is_none() && p.updated_at == Some(NOW));
}

#[test]
fn plex_parsing_checks_the_account_and_finds_it_by_name() {
    let page = body(&plex_entries(), 6);
    let (plays, read, total, others) = parse_history(&page, 7).unwrap();
    assert_eq!((plays.len(), read, total, others), (2, 6, Some(6), 4));
    let accounts = br#"{"MediaContainer": {"Account": [{"id": 1, "name": "owner"}, {"id": 7, "name": "Rue"}, {"id": 9, "name": ""}]}}"#;
    assert_eq!(account_named(accounts, &["rue"]).unwrap(), Some(7));
    assert_eq!(account_named(accounts, &["nobody"]).unwrap(), None);
    assert_eq!(account_named(accounts, &[""]).unwrap(), None, "an empty name never matches");
}

#[tokio::test]
async fn an_unknown_account_brings_nothing_in() {
    let (_d, db) = db();
    let plex = FakePlex::new(1, plex_entries());
    *plex.account.lock() = None;
    let clock = Clock::at(NOW);
    let ph = service(&db, vec![Arc::new(PlexFeed::new(plex.clone()))], &clock, 50);
    assert_eq!(ph.run(ImportSource::PlexHistory).await, Outcome::Failed);
    assert!(plex.asked.lock().is_empty(), "no history is read without knowing whose it is");
    let st = ph.status().await.unwrap();
    assert!(st.sources[0].error.as_deref().unwrap().contains("which plays"));
}

#[tokio::test]
async fn later_runs_read_only_new_plays_and_a_second_import_adds_nothing() {
    let (_d, db) = db();
    let plex = FakePlex::new(1, plex_entries());
    let clock = Clock::at(NOW);
    let ph = service(&db, vec![Arc::new(PlexFeed::new(plex.clone()))], &clock, 2);
    assert_eq!(ph.run(ImportSource::PlexHistory).await, Outcome::Done(4));
    // The same history again adds nothing, and stops at the first page.
    plex.asked.lock().clear();
    assert_eq!(ph.run(ImportSource::PlexHistory).await, Outcome::Done(0));
    assert_eq!(plex.asked.lock().len(), 1, "stops where it reached what's saved");
    // Two new plays: only they come in, from the first page.
    plex.entries.lock().push((107, 1, "3", "Ghost Town", "Echo Field", "Static", S + 2000));
    plex.entries.lock().push((108, 7, "3", "Ghost Town", "Echo Field", "Static", S + 2100));
    plex.asked.lock().clear();
    assert_eq!(ph.run(ImportSource::PlexHistory).await, Outcome::Done(1));
    assert_eq!(plex.asked.lock().clone(), vec![(1, 0, 2), (1, 2, 2)], "up to the page that reaches what's saved");
    assert_eq!(count(&db, "SELECT count(*) FROM imported_plays"), 5);
}

#[tokio::test]
async fn a_stopped_first_read_goes_on_from_the_same_page() {
    let (_d, db) = db();
    let plex = FakePlex::new(1, plex_entries());
    let clock = Clock::at(NOW);
    // Stop the run after its first page.
    let feed = Arc::new(PlexFeed::new(plex.clone()));
    let stopper = Arc::new(Stopper { inner: feed, ph: Mutex::new(None) });
    let ph = service(&db, vec![stopper.clone()], &clock, 2);
    *stopper.ph.lock() = Some(ph.clone());
    assert_eq!(ph.run(ImportSource::PlexHistory).await, Outcome::Stopped);
    let p = db.read(|c| store::progress(c, ImportSource::PlexHistory)).unwrap();
    assert_eq!((p.full_done, p.resume, p.paused), (false, 1, true));
    *stopper.ph.lock() = None;
    plex.asked.lock().clear();
    // Stopped stays stopped until the user asks for an update.
    clock.add(REFRESH_MS);
    assert!(ph.run_due(true, false).await.is_empty());
    assert!(ph.status().await.unwrap().sources[0].paused);
    assert!(matches!(ph.run_due(false, true).await[..], [(ImportSource::PlexHistory, Outcome::Done(_))]));
    assert!(!ph.status().await.unwrap().sources[0].paused);
    assert_eq!(plex.asked.lock()[0].1, 2, "goes on from page 2");
    assert_eq!(plays_of(&db, "plex_history").len(), 4);
}

/// Stops the run after its first page.
struct Stopper {
    inner: Arc<PlexFeed>,
    ph: Mutex<Option<PlayHistory>>,
}

#[async_trait]
impl Feed for Stopper {
    fn source(&self) -> ImportSource {
        ImportSource::PlexHistory
    }
    async fn ready(&self) -> bool {
        true
    }
    async fn page(&self, page: u64, size: u64, since: Option<i64>) -> AppResult<Page> {
        let r = self.inner.page(page, size, since).await;
        if let Some(ph) = self.ph.lock().as_ref() {
            ph.cancel();
        }
        r
    }
}

// ---------------------------------------------------------------- one play, counted once

fn local(c: &Connection, key: &str, at: i64, listened: u64, completed: bool) -> i64 {
    let id = start_play(
        c,
        &PlayStart {
            rating_key: key.into(),
            part_key: None,
            started_at: at,
            duration_ms: 200_000,
            context_kind: None,
            context_id: None,
            device: None,
            offline: false,
        },
    )
    .unwrap();
    end_play(c, id, at + listened as i64, listened, listened, completed).unwrap();
    id
}

fn fetched(id: &str, at: i64, key: Option<&str>, title: &str, artist: &str) -> Fetched {
    Fetched {
        source_id: id.into(),
        played_at: at,
        ms_played: None,
        rating_key: key.map(tid),
        title: title.into(),
        artist: Some(artist.into()),
        album: None,
    }
}

fn sources(c: &Connection) -> Vec<(String, String)> {
    all_plays(c, 0, i64::MAX).unwrap().into_iter().filter(|p| !p.skipped).map(|p| (p.source, p.source_id)).collect()
}

#[test]
fn a_play_reported_from_here_counts_once_whichever_arrives_first() {
    let (_d, db) = db();
    db.write(|c| {
        // Imported first, the local play logged after it (an old play
        // replayed from the offline queue, say): still one play.
        store::save(c, ImportSource::PlexHistory, &[fetched("a", NOW + 100_000, Some("1"), "Golden Hour", "Low Tide")], HashMap::new(), 0)?;
        assert_eq!(sources(c).len(), 1);
        let l = local(c, &tid("1"), NOW, 200_000, true);
        assert_eq!(sources(c), vec![("local".to_string(), l.to_string())], "the import is the same play");

        // Local first, then imports around it: inside about 3 minutes
        // before it started or after it ended is the same play.
        let at = NOW + 3_600_000;
        let l2 = local(c, &tid("2"), at, 200_000, true);
        let near = [
            fetched("before", at - 179_000, Some("2"), "Undertow", "Low Tide"),
            fetched("after", at + 200_000 + 179_000, Some("2"), "Undertow", "Low Tide"),
        ];
        let far = [
            fetched("too-early", at - 181_000 - 200_000, Some("2"), "Undertow", "Low Tide"),
            fetched("too-late", at + 200_000 + 181_000, Some("2"), "Undertow", "Low Tide"),
        ];
        store::save(c, ImportSource::Lastfm, &near, HashMap::new(), 0)?;
        store::save(c, ImportSource::Tautulli, &far, HashMap::new(), 0)?;
        let got = sources(c);
        assert!(got.contains(&("local".into(), l2.to_string())));
        assert!(!got.iter().any(|(_, id)| id == "before" || id == "after"), "{got:?}");
        assert!(got.contains(&("tautulli".into(), "too-early".into())) && got.contains(&("tautulli".into(), "too-late".into())));
        Ok(())
    })
    .unwrap();
}

#[test]
fn the_same_play_from_two_sources_counts_once() {
    let (_d, db) = db();
    db.write(|c| {
        // Another app's play: the server logs it as it ends, Tautulli and
        // Last.fm as it starts.
        store::save(c, ImportSource::PlexHistory, &[fetched("p", NOW + 190_000, Some("1"), "Golden Hour", "Low Tide")], HashMap::new(), 0)?;
        store::save(c, ImportSource::Tautulli, &[fetched("t", NOW, Some("1"), "Golden Hour", "Low Tide")], HashMap::new(), 0)?;
        store::save(c, ImportSource::Lastfm, &[fetched("l", NOW + 1_000, Some("1"), "Golden Hour", "Low Tide")], HashMap::new(), 0)?;
        // Played again an hour later, only on Last.fm: that one stands.
        store::save(c, ImportSource::Lastfm, &[fetched("l2", NOW + 3_600_000, Some("1"), "Golden Hour", "Low Tide")], HashMap::new(), 0)?;
        // Unmatched on both sides: compared by names.
        store::save(c, ImportSource::Tautulli, &[fetched("tu", NOW + 7_200_000, None, "Gone Song", "Nobody")], HashMap::new(), 0)?;
        store::save(c, ImportSource::Lastfm, &[fetched("lu", NOW + 7_210_000, None, "gone song", "nobody")], HashMap::new(), 0)?;
        assert_eq!(sources(c), vec![("plex_history".into(), "p".into()), ("lastfm".into(), "l2".into()), ("tautulli".into(), "tu".into())]);
        Ok(())
    })
    .unwrap();
}

// ---------------------------------------------------------------- Tautulli

fn tautulli_rows() -> String {
    serde_json::json!({"response": {"result": "success", "message": null, "data": {"recordsFiltered": 4, "data": [
        {"id": 51, "started": S, "play_duration": 190, "percent_complete": 95, "user": "rue", "media_type": "track",
         "rating_key": 1, "title": "Golden Hour", "parent_title": "Shoreline", "grandparent_title": "Low Tide"},
        {"id": 52, "started": S + 300, "play_duration": 12, "percent_complete": 6, "user": "rue", "media_type": "track",
         "rating_key": 2, "title": "Undertow", "parent_title": "Shoreline", "grandparent_title": "Low Tide"},
        {"id": 53, "started": S + 400, "play_duration": 200, "percent_complete": 100, "user": "someone", "media_type": "track",
         "rating_key": 3, "title": "Ghost Town", "parent_title": "Static", "grandparent_title": "Echo Field"},
        {"id": 54, "started": S + 500, "play_duration": 25, "percent_complete": 100, "user": "Rue", "media_type": "track",
         "rating_key": 3, "title": "Ghost Town", "parent_title": "Static", "grandparent_title": "Echo Field"}
    ]}}})
    .to_string()
}

fn tautulli(db: &DbHandle, web: &Arc<FakeWeb>, plex: &Arc<FakePlex>) -> Arc<Tautulli> {
    let mut s = crate::settings::load(db);
    s.tautulli_url = "http://tautulli.test:8181".into();
    crate::settings::save(db, &s).unwrap();
    let keys = Arc::new(MemoryStore::new());
    keys.set(tautulli::KEY_ACCOUNT, &Token::new("abc123").unwrap()).unwrap();
    Arc::new(Tautulli { web: web.clone(), keys, db: db.clone(), plex: plex.clone() })
}

#[tokio::test]
async fn tautulli_keeps_the_users_plays_long_enough_to_count() {
    let (_d, db) = db();
    let web = Arc::new(FakeWeb::new());
    web.route("tautulli.test:8181/api/v2", None, 200, &tautulli_rows());
    let plex = FakePlex::new(1, vec![]);
    let t = tautulli(&db, &web, &plex);
    assert!(t.status().ready());
    let clock = Clock::at(NOW);
    let ph = service(&db, vec![t.clone()], &clock, 50);
    assert_eq!(ph.run(ImportSource::Tautulli).await, Outcome::Done(2));
    let got = plays_of(&db, "tautulli");
    assert_eq!(
        got,
        vec![("51".into(), Some(tid("1")), "Golden Hour".into()), ("54".into(), Some(tid("3")), "Ghost Town".into())],
        "the skip is left out, someone else's play too; a short song heard to the end counts"
    );
    let url = web.log.lock()[0].url.clone();
    assert!(url.contains("cmd=get_history") && url.contains("media_type=track") && url.contains("user=rue"));
    assert!(!url.contains("abc123"), "the key stays out of the address");
    assert_eq!(crate::web::fake::body_text(&web.log.lock()[0]), "apikey=abc123");
    let st = ph.status().await.unwrap();
    assert_eq!((st.sources[0].plays, st.sources[0].short_plays), (2, 1));
    // The key never shows in a request's debug text.
    assert!(!format!("{:?}", web.log.lock()[0]).contains("abc123"));
}

#[tokio::test]
async fn tautulli_and_lastfm_failures_are_shown_plainly_and_tried_again_later() {
    let (_d, db) = db();
    let web = Arc::new(FakeWeb::new());
    let plex = FakePlex::new(1, vec![]);
    let t = tautulli(&db, &web, &plex);
    let keys = lastfm_keys(&db);
    let lf = Arc::new(super::lastfm::LastfmFeed { web: web.clone(), keys, db: db.clone() });
    let clock = Clock::at(NOW);
    let ph = service(&db, vec![t.clone(), lf.clone()], &clock, 50);

    // Tautulli refuses the key; Last.fm can't be reached.
    web.route("tautulli.test", None, 401, r#"{"response": {"result": "error", "message": "Invalid apikey"}}"#);
    web.fail("audioscrobbler.com");
    let ran = ph.run_due(false, false).await;
    assert_eq!(ran, vec![(ImportSource::Tautulli, Outcome::Failed), (ImportSource::Lastfm, Outcome::Failed)]);
    let st = ph.status().await.unwrap();
    assert_eq!(st.sources[0].error.as_deref(), Some("Tautulli didn't accept the API key. Check it in Settings > Integrations."));
    assert_eq!(st.sources[1].error.as_deref(), Some("Couldn't reach Last.fm. Platter will try again later."));

    // Not tried again straight away, even after a sync...
    assert!(ph.run_due(true, false).await.is_empty());
    // ...but later, and a bad Last.fm key is said plainly too.
    clock.add(RETRY_MS);
    web.clear();
    web.route("tautulli.test", None, 200, &tautulli_rows());
    web.route("audioscrobbler.com", None, 403, r#"{"error": 10, "message": "Invalid API key"}"#);
    let ran = ph.run_due(false, false).await;
    assert_eq!(ran, vec![(ImportSource::Tautulli, Outcome::Done(2)), (ImportSource::Lastfm, Outcome::Failed)]);
    let st = ph.status().await.unwrap();
    assert_eq!(st.sources[0].error, None, "a good run clears the message");
    assert!(st.sources[1].error.as_deref().unwrap().contains("didn't accept your API key"));
    // The user's Update button tries again at once.
    web.route("audioscrobbler.com", None, 200, &recent(&[], 0));
    let ran = ph.run_due(false, true).await;
    assert!(ran.contains(&(ImportSource::Lastfm, Outcome::Done(0))));
}

#[tokio::test]
async fn plex_history_unavailable_is_shown_and_tried_again_on_a_later_sync() {
    let (_d, db) = db();
    let plex = FakePlex::new(1, plex_entries());
    *plex.fail.lock() = Some(AppError::Network("connection refused".into()));
    let clock = Clock::at(NOW);
    let ph = service(&db, vec![Arc::new(PlexFeed::new(plex.clone()))], &clock, 50);
    assert_eq!(ph.run_due(false, false).await, vec![(ImportSource::PlexHistory, Outcome::Failed)]);
    let st = ph.status().await.unwrap();
    assert_eq!(st.sources[0].error.as_deref(), Some("Your Plex history isn't available right now. Platter will try again later."));
    assert_eq!(st.sources[0].plays, 0);
    // Refused (a shared server's user, say) reads differently.
    *plex.fail.lock() = Some(AppError::Auth("401".into()));
    clock.add(RETRY_MS);
    ph.run_due(true, false).await;
    assert!(ph.status().await.unwrap().sources[0].error.as_deref().unwrap().contains("didn't let Platter"));
    clock.add(RETRY_MS);
    assert_eq!(ph.run_due(true, false).await, vec![(ImportSource::PlexHistory, Outcome::Done(4))]);
    assert_eq!(ph.status().await.unwrap().sources[0].error, None);
}

// ---------------------------------------------------------------- Last.fm

fn lastfm_keys(db: &DbHandle) -> Arc<MemoryStore> {
    let keys = Arc::new(MemoryStore::new());
    keys.set(crate::scrobble::lastfm::KEY_ACCOUNT, &Token::new("lfkey").unwrap()).unwrap();
    keys.set(crate::scrobble::lastfm::SESSION_ACCOUNT, &Token::new("sess").unwrap()).unwrap();
    db.write(|c| crate::db::store::kv_set(c, crate::scrobble::commands::KEY_LASTFM_USER, "rue")).unwrap();
    keys
}

/// A recent tracks page: (artist, title, album, uts; 0 is playing now).
fn recent(list: &[(&str, &str, &str, i64)], pages: u64) -> String {
    let tracks: Vec<serde_json::Value> = list
        .iter()
        .map(|(a, t, al, uts)| {
            if *uts == 0 {
                serde_json::json!({"artist": {"#text": a}, "name": t, "album": {"#text": al}, "@attr": {"nowplaying": "true"}})
            } else {
                serde_json::json!({"artist": {"#text": a}, "name": t, "album": {"#text": al}, "date": {"uts": uts.to_string()}})
            }
        })
        .collect();
    serde_json::json!({"recenttracks": {"track": tracks, "@attr": {"page": "1", "totalPages": pages.to_string(), "total": list.len().to_string()}}})
        .to_string()
}

#[tokio::test]
async fn lastfm_scrobbles_match_by_names_and_count_once() {
    let (_d, db) = db();
    let web = Arc::new(FakeWeb::new());
    let keys = lastfm_keys(&db);
    let lf = Arc::new(super::lastfm::LastfmFeed { web: web.clone(), keys, db: db.clone() });
    assert!(lf.ready().await);
    // Platter's own play, which it scrobbled too.
    let own = db.write(|c| Ok(local(c, &tid("2"), S * 1000, 200_000, true))).unwrap();
    web.route(
        "page=1",
        None,
        200,
        &recent(
            &[
                ("Low Tide", "Undertow", "Shoreline", 0),
                ("Low Tide", "Golden Hour", "Shoreline", S + 900),
                ("LOW TIDE", "Undertow", "Shoreline", S),
            ],
            2,
        ),
    );
    web.route("page=2", None, 200, &recent(&[("Somebody Else", "Not Here", "Elsewhere", S - 900)], 2));
    let clock = Clock::at(NOW);
    let ph = service(&db, vec![lf.clone()], &clock, 200);
    assert_eq!(ph.run(ImportSource::Lastfm).await, Outcome::Done(3), "the song playing now has no time yet");
    let got = plays_of(&db, "lastfm");
    assert_eq!(
        got.iter().map(|g| (g.1.clone(), g.2.as_str())).collect::<Vec<_>>(),
        vec![(None, "Not Here"), (Some(tid("2")), "Undertow"), (Some(tid("1")), "Golden Hour"),]
    );
    let urls: Vec<String> = web.log.lock().iter().map(|r| r.url.clone()).collect();
    assert!(urls[0].contains("method=user.getrecenttracks") && urls[0].contains("user=rue") && urls[0].contains("page=1"));
    assert!(!urls[0].contains("from="), "the first read is the whole history");
    // Platter's own scrobble is its local play.
    db.read(|c| {
        let s = sources(c);
        assert!(s.contains(&("local".into(), own.to_string())));
        assert_eq!(s.iter().filter(|(src, _)| src == "lastfm").count(), 2);
        Ok(())
    })
    .unwrap();
    // Later runs ask only for what's newer.
    web.log.lock().clear();
    ph.run(ImportSource::Lastfm).await;
    let url = web.log.lock()[0].url.clone();
    assert!(url.contains(&format!("from={}", S + 901)), "{url}");
}

// ---------------------------------------------------------------- stats, settings, removing

#[tokio::test]
async fn imported_plays_feed_stats_and_leave_when_turned_off_or_removed() {
    let (_d, db) = db();
    let plex = FakePlex::new(1, plex_entries());
    let web = Arc::new(FakeWeb::new());
    web.route("page=1", None, 200, &recent(&[("Somebody Else", "Not Here", "Elsewhere", S - 900)], 1));
    let lf = Arc::new(super::lastfm::LastfmFeed { web: web.clone(), keys: lastfm_keys(&db), db: db.clone() });
    let clock = Clock::at(NOW);
    let ph = service(&db, vec![Arc::new(PlexFeed::new(plex.clone())), lf], &clock, 50);
    ph.run_due(false, false).await;
    let overview = |db: &DbHandle| {
        db.write(|c| {
            crate::stats::refresh(c)?;
            let d = crate::stats::resolve(c, &crate::stats::StatsRange::default())?;
            crate::stats::overview(c, d, 10)
        })
        .unwrap()
    };
    let o = overview(&db);
    assert_eq!(o.totals.plays, 5, "four from Plex, one from Last.fm");
    assert!(o.top_artists.iter().any(|a| a.name == "Somebody Else"), "a scrobble not in the library still counts for its artist");
    assert!(o.top_artists.iter().any(|a| a.name == "Low Tide" && a.plays == 2));

    // Include off: out of every count, still saved.
    let mut s = crate::settings::load(&db);
    s.include_lastfm_history = false;
    follow_settings(&db, &s);
    assert_eq!(overview(&db).totals.plays, 4);
    assert_eq!(count(&db, "SELECT count(*) FROM imported_plays WHERE source = 'lastfm'"), 1);
    s.include_lastfm_history = true;
    follow_settings(&db, &s);
    assert_eq!(overview(&db).totals.plays, 5);

    // Remove deletes only that source's plays (a file import stays).
    db.write(|c| store::save(c, ImportSource::File, &[fetched("f", NOW, None, "From a file", "Someone")], HashMap::new(), 0)).unwrap();
    let n = ph.remove(ImportSource::PlexHistory).await.unwrap();
    assert_eq!(n, 4);
    assert_eq!(count(&db, "SELECT count(*) FROM imported_plays WHERE source = 'plex_history'"), 0);
    assert_eq!(count(&db, "SELECT count(*) FROM imported_plays"), 2);
    assert_eq!(overview(&db).totals.plays, 2);
    // Nothing comes back by itself...
    clock.add(REFRESH_MS);
    assert!(!ph.run_due(true, false).await.iter().any(|(s, _)| *s == ImportSource::PlexHistory));
    assert!(ph.status().await.unwrap().sources[0].removed);
    // ...until the source is turned off and on again.
    s.include_plex_history = false;
    follow_settings(&db, &s);
    s.include_plex_history = true;
    follow_settings(&db, &s);
    assert!(ph.run_due(false, false).await.contains(&(ImportSource::PlexHistory, Outcome::Done(4))));
    assert!(ph.remove(ImportSource::File).await.is_err(), "history files are removed from the import page");
}

#[tokio::test]
async fn sources_wait_for_their_setup_and_their_setting() {
    let (_d, db) = db();
    let web = Arc::new(FakeWeb::new());
    let lf = Arc::new(super::lastfm::LastfmFeed { web: web.clone(), keys: Arc::new(MemoryStore::new()), db: db.clone() });
    let t = Arc::new(Tautulli { web: web.clone(), keys: Arc::new(MemoryStore::new()), db: db.clone(), plex: FakePlex::new(1, vec![]) });
    let plex = FakePlex::new(1, plex_entries());
    let clock = Clock::at(NOW);
    let ph = service(&db, vec![Arc::new(PlexFeed::new(plex.clone())), t.clone(), lf], &clock, 50);
    let mut s = crate::settings::load(&db);
    s.include_plex_history = false;
    crate::settings::save(&db, &s).unwrap();
    assert!(ph.run_due(false, true).await.is_empty(), "Plex is off, Tautulli and Last.fm aren't set up");
    assert!(web.log.lock().is_empty());
    let st = ph.status().await.unwrap();
    assert_eq!(st.sources.iter().map(|x| (x.set_up, x.included)).collect::<Vec<_>>(), vec![(true, false), (false, true), (false, true)]);
    assert!(t.set_key(Some("not a key!")).is_err());
    assert!(t.set_key(Some("abc123")).unwrap().has_key);
    assert!(!t.set_key(None).unwrap().has_key);
}

#[test]
fn imported_plays_keep_the_speed_of_the_combined_view() {
    // The view's checks against local plays and other sources use the time
    // indexes, so a long history of both stays quick to read.
    let (_d, db) = db();
    db.write(|c| {
        let tx = c.transaction()?;
        for i in 0..20_000i64 {
            tx.execute(
                "INSERT INTO listen_log (rating_key, started_at, ended_at, duration_ms, ms_listened, completed) VALUES (?1, ?2, ?2 + 200000, 200000, 200000, 1)",
                params![tid(&format!("{}", 1 + i % 3)), NOW - i * 900_000],
            )?;
            tx.execute(
                "INSERT INTO imported_plays (source, source_id, played_at, title, rating_key) VALUES ('lastfm', ?1, ?2, 'x', ?3)",
                params![i.to_string(), NOW - i * 900_000 - 450_000, tid(&format!("{}", 1 + i % 3))],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let t = std::time::Instant::now();
    let n: i64 =
        db.read(|c| Ok(c.query_row(&format!("WITH plays AS ({ALL_PLAYS}) SELECT count(*) FROM plays"), [], |r| r.get(0))?)).unwrap();
    assert_eq!(n, 40_000);
    let limit = if cfg!(debug_assertions) { 3_000 } else { 500 };
    assert!(t.elapsed().as_millis() < limit, "{:?}", t.elapsed());
}

#[test]
fn tautulli_parsing_reports_a_bad_key() {
    let e = tautulli::parse_history(br#"{"response": {"result": "error", "message": "Invalid apikey"}}"#, "rue", None).unwrap_err();
    assert_eq!(e.kind(), "auth");
}
