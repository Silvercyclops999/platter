use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use super::lrclib::{self, Query};
use super::*;
use crate::artistinfo::http::fake::FakeHttp;
use crate::error::AppError;

/// A canned Plex answer: lyrics, or an error by kind.
type Answer = Result<Option<Lyrics>, &'static str>;

/// Plex answers from a fixed map; ids not in it have no lyrics.
#[derive(Default)]
struct FakePlex {
    answers: Mutex<Vec<(String, Answer)>>,
    calls: Mutex<usize>,
}

impl FakePlex {
    fn set(&self, id: &str, r: Answer) {
        self.answers.lock().push((id.to_owned(), r));
    }
}

#[async_trait]
impl PlexLyrics for FakePlex {
    async fn lyrics(&self, track_id: &str) -> AppResult<Option<Lyrics>> {
        *self.calls.lock() += 1;
        match self.answers.lock().iter().find(|(id, _)| id == track_id) {
            Some((_, Ok(l))) => Ok(l.clone()),
            Some((_, Err("invalid"))) => Err(AppError::Invalid("track id".into())),
            Some((_, Err(e))) => Err(AppError::Network((*e).into())),
            None => Ok(None),
        }
    }
}

struct Fx {
    svc: LyricsService,
    http: Arc<FakeHttp>,
    plex: Arc<FakePlex>,
    on: Arc<AtomicBool>,
    prefer: Arc<AtomicBool>,
    db: DbHandle,
    _dir: tempfile::TempDir,
}

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("l.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            r#"
            INSERT INTO artists (id, title, sort_title) VALUES ('7', 'Test Artist', 'Test Artist');
            INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, track_count)
              VALUES ('50', 'First Light', 'First Light', '7', 'Test Artist', 'album', '[]', 10),
                     ('80', 'Basement Tapes', 'Basement Tapes', '7', 'Test Artist', 'album', '["demo"]', 4);
            INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key)
              VALUES ('501', 'Opening', '50', 'First Light', '7', 'Test Artist', 200500, '/p/1'),
                     ('801', 'Opening', '80', 'Basement Tapes', '7', 'Test Artist', 190000, '/p/2');
            "#,
        )?;
        Ok(())
    })
    .unwrap();
    let http = Arc::new(FakeHttp::new());
    let plex = Arc::new(FakePlex::default());
    let on = Arc::new(AtomicBool::new(true));
    let prefer = Arc::new(AtomicBool::new(false));
    let switches = Switches { lrclib: on.clone(), prefer_own: prefer.clone() };
    let svc = LyricsService::with_gap(db.clone(), http.clone(), plex.clone(), switches, Duration::from_millis(1));
    Fx { svc, http, plex, on, prefer, db, _dir: dir }
}

const SYNCED: &str = "[00:01.00]Hello\n[00:02.00]World";

fn record(artist: &str, title: &str, duration: f64, synced: Option<&str>, plain: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "id": 1, "trackName": title, "artistName": artist, "albumName": "Whatever",
        "duration": duration, "instrumental": false,
        "syncedLyrics": synced, "plainLyrics": plain
    })
}

// ---------------------------------------------------------------- rules

fn q(ms: i64) -> Query {
    Query { artist: "Test Artist".into(), title: "Opening".into(), album: "First Light".into(), duration_ms: ms }
}

#[test]
fn urls_carry_only_the_words_and_length() {
    let get = lrclib::get_url(&q(200_500));
    assert_eq!(
        get,
        "https://lrclib.net/api/get?artist_name=Test+Artist&track_name=Opening&album_name=First+Light&duration=201"
    );
    let search = lrclib::search_url(&q(200_500));
    assert_eq!(search, "https://lrclib.net/api/search?artist_name=Test+Artist&track_name=Opening");
}

#[test]
fn exact_reply_prefers_synced_and_skips_instrumentals() {
    let r = record("A", "B", 1.0, Some(SYNCED), Some("Hello\nWorld"));
    assert_eq!(lrclib::parse_get(r.to_string().as_bytes()).unwrap().as_deref(), Some(SYNCED));
    let plain = record("A", "B", 1.0, None, Some("Hello"));
    assert_eq!(lrclib::parse_get(plain.to_string().as_bytes()).unwrap().as_deref(), Some("Hello"));
    let mut inst = record("A", "B", 1.0, Some(SYNCED), None);
    inst["instrumental"] = true.into();
    assert_eq!(lrclib::parse_get(inst.to_string().as_bytes()).unwrap(), None);
    assert!(lrclib::parse_get(b"not json").is_err());
}

#[test]
fn search_needs_the_same_artist_and_title() {
    let body = serde_json::json!([
        record("Someone Else", "Opening", 200.0, Some(SYNCED), None),
        record("Test Artist", "Opening Night", 200.0, Some(SYNCED), None),
    ]);
    assert_eq!(lrclib::best_search_hit(&q(200_000), body.to_string().as_bytes()).unwrap(), None);
    // Case, accents and a featured artist don't matter.
    let body = serde_json::json!([record("TEST ARTIST", "Opening (feat. Guest)", 200.0, None, Some("Words"))]);
    assert_eq!(lrclib::best_search_hit(&q(200_000), body.to_string().as_bytes()).unwrap().as_deref(), Some("Words"));
}

#[test]
fn search_prefers_synced_then_the_closest_length() {
    let body = serde_json::json!([
        record("Test Artist", "Opening", 200.0, None, Some("plain, exact length")),
        record("Test Artist", "Opening", 206.0, Some("[00:01.00]far"), None),
        record("Test Artist", "Opening", 203.0, Some("[00:01.00]near"), None),
    ]);
    assert_eq!(lrclib::best_search_hit(&q(200_000), body.to_string().as_bytes()).unwrap().as_deref(), Some("[00:01.00]near"));
    // Synced lyrics more than 10 s off lose their times (they'd drift).
    let body = serde_json::json!([record("Test Artist", "Opening", 260.0, Some("[00:01.00]Late line"), None)]);
    assert_eq!(lrclib::best_search_hit(&q(200_000), body.to_string().as_bytes()).unwrap().as_deref(), Some("Late line"));
    // Then plain text with the closest length wins over synced text far off.
    let body = serde_json::json!([
        record("Test Artist", "Opening", 290.0, Some("[00:01.00]far"), Some("far plain")),
        record("Test Artist", "Opening", 201.0, None, Some("close plain")),
    ]);
    assert_eq!(lrclib::best_search_hit(&q(200_000), body.to_string().as_bytes()).unwrap().as_deref(), Some("close plain"));
}

// ---------------------------------------------------------------- service

fn plex_lyrics() -> Lyrics {
    Lyrics { lines: vec!["From Plex".into()], ..Default::default() }
}

#[tokio::test]
async fn plex_comes_first_and_lrclib_is_not_asked() {
    let f = fx();
    f.plex.set("501", Ok(Some(plex_lyrics())));
    let l = f.svc.get("501").await.unwrap().unwrap();
    assert_eq!(l.lines, vec!["From Plex"]);
    assert_eq!(l.source, LyricsSource::Plex);
    assert!(f.http.urls().is_empty());
}

#[tokio::test]
async fn lrclib_only_when_the_setting_is_on() {
    let f = fx();
    f.on.store(false, Ordering::Relaxed);
    f.http.route("api/get", 200, &record("Test Artist", "Opening", 200.0, Some(SYNCED), None).to_string());
    assert_eq!(f.svc.get("501").await.unwrap(), None);
    assert!(f.http.urls().is_empty(), "nothing leaves the computer while it's off");
    f.on.store(true, Ordering::Relaxed);
    let l = f.svc.get("501").await.unwrap().unwrap();
    assert!(l.synced);
    assert_eq!(l.source, LyricsSource::Lrclib);
    assert_eq!(l.credit.as_deref(), Some(CREDIT));
    assert_eq!(l.timed.len(), 2);
}

#[tokio::test]
async fn found_lyrics_are_cached_for_30_days() {
    let f = fx();
    f.http.route("api/get", 200, &record("Test Artist", "Opening", 200.0, Some(SYNCED), None).to_string());
    f.svc.get("501").await.unwrap().unwrap();
    f.svc.get("501").await.unwrap().unwrap();
    assert_eq!(f.http.calls("lrclib"), 1);
    let ttl: i64 = f.db.read(|c| Ok(c.query_row("SELECT expires_at - fetched_at FROM lyrics_cache", [], |r| r.get(0))?)).unwrap();
    assert_eq!(ttl, cache::TTL_S);
}

#[tokio::test]
async fn a_miss_searches_by_title_and_is_cached_for_7_days() {
    let f = fx();
    f.http.route("api/get", 404, r#"{"code":404,"name":"TrackNotFound"}"#);
    f.http.route("api/search", 200, "[]");
    assert_eq!(f.svc.get("501").await.unwrap(), None);
    assert_eq!(f.http.calls("api/get"), 1);
    assert_eq!(f.http.calls("api/search"), 1);
    assert_eq!(f.svc.get("501").await.unwrap(), None);
    assert_eq!(f.http.calls("lrclib"), 2, "the miss is remembered");
    let ttl: i64 = f.db.read(|c| Ok(c.query_row("SELECT expires_at - fetched_at FROM lyrics_cache", [], |r| r.get(0))?)).unwrap();
    assert_eq!(ttl, cache::MISS_TTL_S);
    // Expired rows are asked again.
    f.db.write(|c| Ok(c.execute("UPDATE lyrics_cache SET expires_at = 0", [])?)).unwrap();
    f.svc.get("501").await.unwrap();
    assert_eq!(f.http.calls("lrclib"), 4);
}

#[tokio::test]
async fn unreleased_songs_get_lyrics_matched_by_title() {
    let f = fx();
    f.http.route("api/get", 404, "{}");
    f.http.route(
        "api/search",
        200,
        &serde_json::json!([record("Test Artist", "Opening", 191.0, Some(SYNCED), None)]).to_string(),
    );
    let l = f.svc.get("801").await.unwrap().unwrap();
    assert!(l.synced);
    assert_eq!(l.credit.as_deref(), Some(CREDIT_MATCHED));
    // From the cache it still says how it was found.
    let again = f.svc.get("801").await.unwrap().unwrap();
    assert_eq!(again.credit.as_deref(), Some(CREDIT_MATCHED));
    assert_eq!(f.http.calls("lrclib"), 2);
    // The search sent only the artist and title.
    let search = f.http.urls().into_iter().find(|u| u.contains("search")).unwrap();
    assert!(!search.contains("Basement") && !search.contains("duration"), "{search}");
}

#[tokio::test]
async fn sources_down_are_silent_and_not_cached() {
    let f = fx();
    // Plex is down, LRCLIB is down: no lyrics, no error.
    f.plex.set("501", Err("offline"));
    f.http.fail("lrclib");
    assert_eq!(f.svc.get("501").await.unwrap(), None);
    let rows: i64 = f.db.read(|c| Ok(c.query_row("SELECT count(*) FROM lyrics_cache", [], |r| r.get(0))?)).unwrap();
    assert_eq!(rows, 0, "trouble is not remembered as a miss");
    // LRCLIB answering 500 is the same.
    f.http.route("lrclib", 500, "");
    let f2 = fx();
    f2.http.route("lrclib", 503, "");
    assert_eq!(f2.svc.get("501").await.unwrap(), None);
}

#[tokio::test]
async fn unknown_tracks_and_bad_ids() {
    let f = fx();
    assert_eq!(f.svc.get("999").await.unwrap(), None);
    assert!(f.http.urls().is_empty());
    f.plex.set("../x", Err("invalid"));
    assert_eq!(f.svc.get("../x").await.unwrap_err().kind(), "invalid");
    assert_eq!(*f.plex.calls.lock(), 2);
}

// ---------------------------------------------------------------- own lyrics

const MINE: &str = "[00:01.50]My first line\n[00:04.00]My second line";

#[tokio::test]
async fn own_lyrics_come_after_plex_and_before_lrclib() {
    let f = fx();
    f.http.route("api/get", 200, &record("Test Artist", "Opening", 200.0, Some(SYNCED), None).to_string());
    f.svc.save_own("501", MINE, false).await.unwrap();
    // No Plex lyrics: the user's own, and LRCLIB isn't asked.
    let l = f.svc.get("501").await.unwrap().unwrap();
    assert_eq!(l.source, LyricsSource::User);
    assert!(l.synced);
    assert_eq!(l.timed[0].start_ms, 1500);
    assert!(f.http.urls().is_empty());
    // Plex has some: Plex's.
    f.plex.set("501", Ok(Some(plex_lyrics())));
    assert_eq!(f.svc.get("501").await.unwrap().unwrap().source, LyricsSource::Plex);
}

#[tokio::test]
async fn preferring_own_lyrics_per_song_or_for_every_song() {
    let f = fx();
    f.plex.set("501", Ok(Some(plex_lyrics())));
    f.svc.save_own("501", MINE, true).await.unwrap();
    assert_eq!(f.svc.get("501").await.unwrap().unwrap().source, LyricsSource::User);
    assert_eq!(*f.plex.calls.lock(), 0, "Plex isn't asked when the song prefers its own");
    f.svc.save_own("501", MINE, false).await.unwrap();
    assert_eq!(f.svc.get("501").await.unwrap().unwrap().source, LyricsSource::Plex);
    f.prefer.store(true, Ordering::Relaxed);
    assert_eq!(f.svc.get("501").await.unwrap().unwrap().source, LyricsSource::User);
    // The setting alone shows nothing new for songs without their own.
    f.plex.set("801", Ok(Some(plex_lyrics())));
    assert_eq!(f.svc.get("801").await.unwrap().unwrap().source, LyricsSource::Plex);
}

#[tokio::test]
async fn own_lyrics_save_replace_list_and_delete() {
    let f = fx();
    assert_eq!(f.svc.own("501").await.unwrap(), None);
    f.svc.save_own("501", "\u{feff}\r\n  Plain line one\r\nPlain line two\r\n\r\n", false).await.unwrap();
    let own = f.svc.own("501").await.unwrap().unwrap();
    assert_eq!(own.text, "Plain line one\nPlain line two");
    assert!(!own.prefer);
    let l = f.svc.get("501").await.unwrap().unwrap();
    assert!(!l.synced, "plain lines save without timing");
    assert_eq!(l.lines, vec!["Plain line one", "Plain line two"]);
    f.svc.save_own("501", MINE, true).await.unwrap();
    f.svc.save_own("801", "Words", false).await.unwrap();
    assert!(f.svc.own("501").await.unwrap().unwrap().prefer);
    assert_eq!(f.svc.own_ids().await.unwrap(), vec!["501", "801"]);
    assert!(f.svc.delete_own("501").await.unwrap());
    assert!(!f.svc.delete_own("501").await.unwrap());
    assert_eq!(f.svc.own_ids().await.unwrap(), vec!["801"]);
}

#[tokio::test]
async fn own_lyrics_refuse_empty_long_or_bad_ids() {
    let f = fx();
    assert_eq!(f.svc.save_own("501", "  \n\n ", false).await.unwrap_err().kind(), "invalid");
    assert_eq!(f.svc.save_own("501", "[00:01.00]\n[ar:Someone]", false).await.unwrap_err().kind(), "invalid");
    let long = "la ".repeat(user::MAX_BYTES);
    assert_eq!(f.svc.save_own("501", &long, false).await.unwrap_err().kind(), "invalid");
    assert_eq!(f.svc.save_own("../x", "Words", false).await.unwrap_err().kind(), "invalid");
    assert_eq!(f.svc.own("../x").await.unwrap(), None);
    assert!(f.svc.own_ids().await.unwrap().is_empty());
}

#[test]
fn export_names_are_safe_file_names() {
    assert_eq!(user::export_file_name("Test Artist - Opening"), "Test Artist - Opening.lrc");
    assert_eq!(user::export_file_name("AC/DC: What? *Live*"), "AC_DC_ What_ _Live_.lrc");
    assert_eq!(user::export_file_name("  ...  "), "Lyrics.lrc");
    assert_eq!(user::export_file_name("song.LRC"), "song.LRC");
    assert!(user::export_file_name(&"x".repeat(500)).chars().count() <= 124);
}
