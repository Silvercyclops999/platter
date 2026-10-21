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
    show_auto: Arc<AtomicBool>,
    live_timing: Arc<AtomicBool>,
    released_ok: Arc<AtomicBool>,
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
                     ('80', 'Basement Tapes', 'Basement Tapes', '7', 'Test Artist', 'album', '["demo"]', 4),
                     ('60', 'Late Show', 'Late Show', '7', 'Test Artist', 'album', '["Live"]', 12);
            INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key)
              VALUES ('501', 'Opening', '50', 'First Light', '7', 'Test Artist', 200500, '/p/1'),
                     ('801', 'Opening', '80', 'Basement Tapes', '7', 'Test Artist', 190000, '/p/2'),
                     ('601', 'Opening', '60', 'Late Show', '7', 'Test Artist', 245000, '/p/3');
            "#,
        )?;
        Ok(())
    })
    .unwrap();
    let http = Arc::new(FakeHttp::new());
    let plex = Arc::new(FakePlex::default());
    let on = Arc::new(AtomicBool::new(true));
    let prefer = Arc::new(AtomicBool::new(false));
    let show_auto = Arc::new(AtomicBool::new(true));
    let live_timing = Arc::new(AtomicBool::new(false));
    let released_ok = Arc::new(AtomicBool::new(false));
    let switches = Switches {
        lrclib: on.clone(),
        prefer_own: prefer.clone(),
        show_auto: show_auto.clone(),
        live_timing: live_timing.clone(),
        fix_timing: Arc::new(AtomicBool::new(true)),
        released_ok: released_ok.clone(),
        ..Default::default()
    };
    let svc = LyricsService::with_gap(db.clone(), http.clone(), plex.clone(), switches, Duration::from_millis(1));
    Fx { svc, http, plex, on, prefer, show_auto, live_timing, released_ok, db, _dir: dir }
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
    assert_eq!(lrclib::parse_get(r.to_string().as_bytes()).unwrap().and_then(|r| r.text(true)).as_deref(), Some(SYNCED));
    let plain = record("A", "B", 1.0, None, Some("Hello"));
    assert_eq!(lrclib::parse_get(plain.to_string().as_bytes()).unwrap().and_then(|r| r.text(true)).as_deref(), Some("Hello"));
    let mut inst = record("A", "B", 1.0, Some(SYNCED), None);
    inst["instrumental"] = true.into();
    // An instrumental is kept, flagged, with no text to show.
    let got = lrclib::parse_get(inst.to_string().as_bytes()).unwrap().unwrap();
    assert!(got.instrumental && got.text(true).is_none());
    assert!(lrclib::parse_get(b"not json").is_err());
}

#[test]
fn search_needs_the_same_artist_and_title() {
    let body = serde_json::json!([
        record("Someone Else", "Opening", 200.0, Some(SYNCED), None),
        record("Test Artist", "Opening Night", 200.0, Some(SYNCED), None),
    ]);
    assert!(lrclib::search_hits(&q(200_000), body.to_string().as_bytes()).unwrap().is_empty());
    // Case, accents and a featured artist don't matter.
    let body = serde_json::json!([record("TEST ARTIST", "Opening (feat. Guest)", 200.0, None, Some("Words"))]);
    let hits = lrclib::search_hits(&q(200_000), body.to_string().as_bytes()).unwrap();
    assert_eq!(hits[0].text(true).as_deref(), Some("Words"));
}

fn texts(hits: &[lrclib::Record]) -> Vec<String> {
    hits.iter().filter_map(|r| r.text(true)).collect()
}

#[test]
fn search_hits_come_closest_length_first_synced_first_on_a_tie() {
    let body = serde_json::json!([
        record("Test Artist", "Opening", 206.0, Some("[00:01.00]far"), None),
        record("Test Artist", "Opening", 200.0, None, Some("plain, exact length")),
        record("Test Artist", "Opening", 203.0, Some("[00:01.00]near"), None),
        record("Test Artist", "Opening", 203.0, None, Some("near plain")),
    ]);
    let hits = lrclib::search_hits(&q(200_000), body.to_string().as_bytes()).unwrap();
    assert_eq!(texts(&hits), vec!["plain, exact length", "[00:01.00]near", "near plain", "[00:01.00]far"]);
    // Instrumentals and empty records are left out.
    let mut inst = record("Test Artist", "Opening", 200.0, Some(SYNCED), None);
    inst["instrumental"] = true.into();
    let body = serde_json::json!([inst, record("Test Artist", "Opening", 200.0, None, None)]);
    assert!(lrclib::search_hits(&q(200_000), body.to_string().as_bytes()).unwrap().is_empty());
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
    // Found for another album: held back from an unreleased song...
    let s = f.svc.shown("801").await.unwrap().unwrap();
    assert_eq!(s.notice, Some(Notice::VersionHeld));
    assert!(s.lyrics.lines.is_empty());
    assert_eq!(f.svc.get("801").await.unwrap(), None);
    // ...until "Use released lyrics for other versions" is on.
    f.released_ok.store(true, Ordering::Relaxed);
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
    // The setting alone shows nothing new for songs without their own (801
    // is unreleased: its server lyrics need "Use released lyrics" too).
    f.released_ok.store(true, Ordering::Relaxed);
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

const AUTO: &str = "[00:01.00]<00:01.00>Heard <00:01.40>words\n";

#[tokio::test]
async fn lyrics_found_without_timing_are_lined_up_to_the_transcription() {
    let f = fx();
    let plain = Lyrics { lines: vec!["Hello darkness, my old friend".into(), "".into(), "I've come to talk with you again".into()], ..Default::default() };
    f.plex.set("501", Ok(Some(plain)));
    assert!(!f.svc.has_other("501").await, "words without timing still get written down");
    let l = f.svc.get("501").await.unwrap().unwrap();
    assert!(!l.synced, "nothing heard yet");
    let heard = "[00:10.00]<00:10.00>Hello <00:10.40>darkness <00:10.80>my <00:11.20>oh <00:11.60>friend\n\
                 [00:15.00]<00:15.00>I've <00:15.40>come <00:15.80>to <00:16.20>talk <00:16.60>with <00:17.00>you <00:17.40>again\n";
    let now = crate::db::queries::now_s();
    assert!(f.db.write(|c| user::put_auto(c, "501", heard, "turbo", now)).unwrap());
    let s = f.svc.shown("501").await.unwrap().unwrap();
    assert!(s.timed_here);
    let l = s.lyrics;
    assert!(l.synced);
    assert_eq!(l.source, LyricsSource::Plex, "the server's words");
    let starts: Vec<(u32, &str)> = l.timed.iter().map(|t| (t.start_ms, t.text.as_str())).collect();
    assert_eq!(starts, vec![(10_000, "Hello darkness, my old friend"), (15_000, "I've come to talk with you again")]);
    assert_eq!(l.timed[0].words.iter().map(|w| w.text.as_str()).collect::<String>(), "Hello darkness, my old friend");
}

#[tokio::test]
async fn transcriptions_show_after_plex_and_never_replace_the_users_own() {
    let f = fx();
    f.on.store(false, Ordering::Relaxed);
    f.prefer.store(true, Ordering::Relaxed);
    let now = crate::db::queries::now_s();
    assert!(f.db.write(|c| user::put_auto(c, "801", AUTO, "small", now)).unwrap());
    let l = f.svc.get("801").await.unwrap().unwrap();
    assert_eq!(l.source, LyricsSource::Auto);
    assert!(l.synced && l.timed[0].words.len() == 2);
    // The editor sees them, marked.
    assert_eq!(f.svc.own("801").await.unwrap().unwrap().auto.as_deref(), Some("small"));
    assert!(!f.svc.has_other("801").await, "a transcription isn't another source");
    // On this unreleased song, Plex's lyrics (found for the released
    // version) don't beat the transcription of this recording...
    f.plex.set("801", Ok(Some(plex_lyrics())));
    assert_eq!(f.svc.get("801").await.unwrap().unwrap().source, LyricsSource::Auto);
    // ...unless other versions take released lyrics: then they win, even
    // with "prefer my own" on.
    f.released_ok.store(true, Ordering::Relaxed);
    assert_eq!(f.svc.get("801").await.unwrap().unwrap().source, LyricsSource::Plex);
    // Saving in the editor makes them the user's own.
    f.svc.save_own("801", "Fixed words", false).await.unwrap();
    let own = f.svc.own("801").await.unwrap().unwrap();
    assert_eq!(own.auto, None);
    assert!(f.svc.has_other("801").await);
    // A later transcription leaves the user's lyrics alone.
    assert!(!f.db.write(|c| user::put_auto(c, "801", AUTO, "medium", now)).unwrap());
    assert_eq!(f.svc.own("801").await.unwrap().unwrap().text, "Fixed words");
}

#[tokio::test]
async fn hidden_transcriptions_leave_no_lyrics() {
    let f = fx();
    f.on.store(false, Ordering::Relaxed);
    let now = crate::db::queries::now_s();
    f.db.write(|c| user::put_auto(c, "501", AUTO, "small", now)).unwrap();
    f.show_auto.store(false, Ordering::Relaxed);
    assert_eq!(f.svc.get("501").await.unwrap(), None);
    f.show_auto.store(true, Ordering::Relaxed);
    assert_eq!(f.svc.get("501").await.unwrap().unwrap().source, LyricsSource::Auto);
}

// ---------------------------------------------------------------- live versions

fn facts(title: &str, album: &str, format: Option<&str>, subs: &[&str], version: Option<&str>) -> live::LiveFacts {
    live::LiveFacts {
        title: title.into(),
        album_title: album.into(),
        format: format.map(Into::into),
        subformats: subs.iter().map(|s| (*s).to_owned()).collect(),
        version: version.map(Into::into),
    }
}

#[test]
fn live_versions_are_recognised() {
    // The release type, from Plex's album format or subformat.
    assert!(live::is_live(&facts("The Song", "Late Orchestration", Some("album"), &["Live"], None)));
    assert!(live::is_live(&facts("The Song", "Concert", Some("Live"), &[], None)));
    assert!(live::is_live(&facts("The Song", "Concert", Some("album"), &["demo", "live"], None)));
    // ARCHIVE_VERSION.
    assert!(live::is_live(&facts("The Song", "Tapes", None, &[], Some("live"))));
    assert!(live::is_live(&facts("The Song", "Tapes", None, &[], Some(" LIVE "))));
    // What the titles say.
    for t in ["The Song (Live)", "The Song [Live at the Hall]", "The Song - Live", "The Song (MTV Unplugged)"] {
        assert!(live::is_live(&facts(t, "Studio Album", Some("album"), &[], None)), "{t}");
    }
    for t in ["The Song - Live at the Hall", "The Song - Live 1986", "The Song - Live (Remastered)"] {
        assert!(live::is_live(&facts(t, "Studio Album", Some("album"), &[], None)), "{t}");
    }
    for a in ["Live at the Hall", "Live From Somewhere", "Live in Paris 1999", "Unplugged", "The Tour (Live)", "The Tour: Live in Paris"] {
        assert!(live::is_live(&facts("The Song", a, Some("album"), &[], None)), "{a}");
    }
    // Not live.
    for (t, a) in [
        ("Live Forever", "Studio Album"),
        ("The Song (Lively Mix)", "Studio Album"),
        ("Where We Live", "Alive"),
        ("The Song", "Delivered"),
        ("Song - Live and Let Die", "Studio Album"),
        ("The Song", "Songs We Live In Between"),
    ] {
        assert!(!live::is_live(&facts(t, a, Some("album"), &[], None)), "{t} / {a}");
    }
    assert!(!live::is_live(&facts("The Song", "Album", Some("album"), &["remix"], Some("released"))));
}

#[test]
fn live_check_reads_the_cache() {
    let f = fx();
    f.db.read(|c| {
        assert!(live::track_is_live(c, "601")?);
        assert!(!live::track_is_live(c, "501")?);
        assert!(!live::track_is_live(c, "999")?);
        Ok(())
    })
    .unwrap();
    // The file's ARCHIVE_VERSION makes a studio-album track live.
    f.db.write(|c| Ok(c.execute("INSERT INTO file_tags (part_key, status, version, scanned_at) VALUES ('/p/1', 'read', 'live', 1)", [])?))
        .unwrap();
    assert!(f.db.read(|c| live::track_is_live(c, "501")).unwrap());
}

fn synced_from(source: LyricsSource) -> Lyrics {
    let mut l = crate::plex::details::parse_lyrics_text(SYNCED);
    l.source = source;
    l
}

#[tokio::test]
async fn borrowed_timing_on_a_live_version_shows_unsynced() {
    let f = fx();
    // Plex's licensed lyrics are matched by artist and title.
    f.plex.set("601", Ok(Some(synced_from(LyricsSource::Plex))));
    // A live version doesn't take them unasked ("Use anyway").
    let s = f.svc.shown("601").await.unwrap().unwrap();
    assert_eq!(s.notice, Some(Notice::VersionHeld));
    assert_eq!(s.pick.as_deref(), Some("server"));
    f.released_ok.store(true, Ordering::Relaxed);
    let s = f.svc.shown("601").await.unwrap().unwrap();
    assert!(s.borrowed_timing);
    assert!(!s.lyrics.synced && s.lyrics.timed.is_empty());
    assert_eq!(s.lyrics.lines, vec!["Hello", "World"]);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["borrowedTiming"], true);
    assert_eq!(v["synced"], false);
    // The same lyrics on the studio version keep their timing.
    f.plex.set("501", Ok(Some(synced_from(LyricsSource::Plex))));
    let s = f.svc.shown("501").await.unwrap().unwrap();
    assert!(!s.borrowed_timing && s.lyrics.synced);
    assert!(serde_json::to_value(&s).unwrap().get("borrowedTiming").is_none());
}

#[tokio::test]
async fn lrclib_timing_on_a_live_version_is_borrowed_too() {
    let f = fx();
    f.released_ok.store(true, Ordering::Relaxed);
    f.http.route("api/get", 200, &record("Test Artist", "Opening", 245.0, Some(SYNCED), None).to_string());
    let s = f.svc.shown("601").await.unwrap().unwrap();
    assert_eq!(s.lyrics.source, LyricsSource::Lrclib);
    assert!(s.borrowed_timing && !s.lyrics.synced);
}

#[tokio::test]
async fn lyrics_tied_to_the_file_keep_their_sync_on_live_versions() {
    let f = fx();
    // A lyrics file next to the song.
    f.plex.set("601", Ok(Some(synced_from(LyricsSource::File))));
    let s = f.svc.shown("601").await.unwrap().unwrap();
    assert!(!s.borrowed_timing && s.lyrics.synced);
    // Your own lyrics.
    let f = fx();
    f.svc.save_own("601", MINE, true).await.unwrap();
    let s = f.svc.shown("601").await.unwrap().unwrap();
    assert_eq!(s.lyrics.source, LyricsSource::User);
    assert!(!s.borrowed_timing && s.lyrics.synced);
}

#[tokio::test]
async fn the_setting_keeps_borrowed_timing_but_still_says_so() {
    let f = fx();
    f.live_timing.store(true, Ordering::Relaxed);
    f.released_ok.store(true, Ordering::Relaxed);
    f.plex.set("601", Ok(Some(synced_from(LyricsSource::Plex))));
    let s = f.svc.shown("601").await.unwrap().unwrap();
    assert!(s.borrowed_timing && s.lyrics.synced && s.lyrics.timed.len() == 2);
    // Plain lyrics have no timing to borrow.
    let f = fx();
    f.plex.set("601", Ok(Some(plex_lyrics())));
    let s = f.svc.shown("601").await.unwrap().unwrap();
    assert!(!s.borrowed_timing);
    assert_eq!(f.svc.shown("999").await.unwrap(), None);
}

#[tokio::test]
async fn a_transcription_beats_borrowed_timing_on_a_live_version() {
    let f = fx();
    f.plex.set("601", Ok(Some(synced_from(LyricsSource::Plex))));
    let now = crate::db::queries::now_s();
    f.db.write(|c| user::put_auto(c, "601", MINE, "small", now)).unwrap();
    let s = f.svc.shown("601").await.unwrap().unwrap();
    assert_eq!(s.lyrics.source, LyricsSource::Auto);
    assert!(s.lyrics.synced && !s.borrowed_timing);
    // Hidden transcriptions don't, and on the studio version Plex's still win.
    f.show_auto.store(false, Ordering::Relaxed);
    assert_eq!(f.svc.shown("601").await.unwrap().unwrap().notice, Some(Notice::VersionHeld));
    f.released_ok.store(true, Ordering::Relaxed);
    assert!(f.svc.shown("601").await.unwrap().unwrap().borrowed_timing);
    // With them shown, a transcription still beats borrowed timing.
    f.show_auto.store(true, Ordering::Relaxed);
    assert_eq!(f.svc.shown("601").await.unwrap().unwrap().lyrics.source, LyricsSource::Auto);
    f.show_auto.store(true, Ordering::Relaxed);
    f.plex.set("501", Ok(Some(synced_from(LyricsSource::Plex))));
    f.db.write(|c| user::put_auto(c, "501", MINE, "small", now)).unwrap();
    assert_eq!(f.svc.shown("501").await.unwrap().unwrap().lyrics.source, LyricsSource::Plex);
}

// ---------------------------------------------------------------- couldn't ask, credits, misses

#[tokio::test]
async fn a_source_down_with_nothing_found_says_unavailable_not_none() {
    let f = fx();
    // The server is down and LRCLIB is off: "couldn't ask".
    f.on.store(false, Ordering::Relaxed);
    f.plex.set("501", Err("offline"));
    assert_eq!(f.svc.shown("501").await.unwrap().unwrap().notice, Some(Notice::Unavailable));
    assert_eq!(f.svc.get("501").await.unwrap(), None);
    // LRCLIB down (the server has none): the same, and nothing is cached.
    let f = fx();
    f.http.fail("lrclib");
    assert_eq!(f.svc.shown("501").await.unwrap().unwrap().notice, Some(Notice::Unavailable));
    let rows: i64 = f.db.read(|c| Ok(c.query_row("SELECT count(*) FROM lyrics_cache", [], |r| r.get(0))?)).unwrap();
    assert_eq!(rows, 0);
    // Everyone answered and nobody has any: plain none.
    let f = fx();
    f.http.route("api/get", 404, "{}");
    f.http.route("api/search", 200, "[]");
    assert_eq!(f.svc.shown("501").await.unwrap(), None);
    // Down but something else was found: it shows.
    let f = fx();
    f.plex.set("501", Err("offline"));
    f.http.route("api/get", 200, &record("Test Artist", "Opening", 200.0, Some(SYNCED), None).to_string());
    let s = f.svc.shown("501").await.unwrap().unwrap();
    assert_eq!(s.notice, None);
    assert_eq!(s.lyrics.source, LyricsSource::Lrclib);
}

#[tokio::test]
async fn instrumentals_say_so() {
    let f = fx();
    let mut inst = record("Test Artist", "Opening", 200.0, None, None);
    inst["instrumental"] = true.into();
    f.http.route("api/get", 200, &inst.to_string());
    f.http.route("api/search", 200, "[]");
    assert_eq!(f.svc.shown("501").await.unwrap().unwrap().notice, Some(Notice::Instrumental));
    // Cached, it still says so.
    assert_eq!(f.svc.shown("501").await.unwrap().unwrap().notice, Some(Notice::Instrumental));
    assert_eq!(f.http.calls("api/get"), 1);
    // A title that says it, with nothing found.
    let f = fx();
    f.on.store(false, Ordering::Relaxed);
    f.db.write(|c| Ok(c.execute("UPDATE tracks SET title = 'Opening (Instrumental)' WHERE id = '501'", [])?)).unwrap();
    assert_eq!(f.svc.shown("501").await.unwrap().unwrap().notice, Some(Notice::Instrumental));
    assert!(candidates::title_says_instrumental("Song [Karaoke Version]"));
    assert!(!candidates::title_says_instrumental("Instrumentality"));
}

#[tokio::test]
async fn several_artists_retry_with_the_lead_artist() {
    let f = fx();
    f.db.write(|c| Ok(c.execute("UPDATE tracks SET track_artist = 'Test Artist & Guest' WHERE id = '501'", [])?)).unwrap();
    // LRCLIB knows the song under the lead artist only.
    f.http.route("api/get?artist_name=Test+Artist+%26+Guest", 404, "{}");
    f.http.route("api/search?artist_name=Test+Artist+%26+Guest", 200, "[]");
    f.http.route("api/get?artist_name=Test+Artist&", 200, &record("Test Artist", "Opening", 200.0, Some(SYNCED), None).to_string());
    let s = f.svc.shown("501").await.unwrap().unwrap();
    assert_eq!(s.lyrics.source, LyricsSource::Lrclib);
    assert!(f.http.urls().iter().any(|u| u.contains("Test+Artist+%26+Guest")), "asked as credited first");
    assert_eq!(lead_artist("Ana feat. Bo"), Some("Ana".into()));
    assert_eq!(lead_artist("Ana, Bo & Cy"), Some("Ana".into()));
    assert_eq!(lead_artist("Solo Artist"), None);
}

#[tokio::test]
async fn compilations_never_ask_for_various_artists() {
    let f = fx();
    f.db.write(|c| Ok(c.execute("UPDATE tracks SET artist_title = 'Various Artists', track_artist = NULL WHERE id = '501'", [])?)).unwrap();
    f.http.route("lrclib", 200, &record("Various Artists", "Opening", 200.0, Some(SYNCED), None).to_string());
    assert_eq!(f.svc.get("501").await.unwrap(), None);
    assert!(f.http.urls().is_empty(), "Various Artists isn't anyone to look up");
}

#[tokio::test]
async fn the_servers_lyrics_are_reused_briefly() {
    let f = fx();
    let svc = LyricsService::with_server_ttl(
        f.db.clone(),
        f.http.clone(),
        f.plex.clone(),
        Switches::default(),
        Duration::from_millis(1),
        Duration::from_secs(60),
    );
    f.plex.set("501", Ok(Some(plex_lyrics())));
    svc.shown("501").await.unwrap().unwrap();
    svc.shown("501").await.unwrap().unwrap();
    assert_eq!(*f.plex.calls.lock(), 1, "a nudge or a check landing doesn't ask the server again");
    // Look again forgets it.
    svc.look_again("501").await.unwrap();
    svc.shown("501").await.unwrap().unwrap();
    assert_eq!(*f.plex.calls.lock(), 2);
    // Failures aren't kept.
    f.plex.set("801", Err("offline"));
    svc.shown("801").await.unwrap();
    svc.shown("801").await.unwrap();
    assert_eq!(*f.plex.calls.lock(), 4);
}

#[tokio::test]
async fn look_again_forgets_a_remembered_miss() {
    let f = fx();
    f.http.route("api/get", 404, "{}");
    f.http.route("api/search", 200, "[]");
    assert_eq!(f.svc.get("501").await.unwrap(), None);
    assert_eq!(f.svc.get("501").await.unwrap(), None);
    assert_eq!(f.http.calls("lrclib"), 2, "the miss is remembered");
    // LRCLIB has it now; Look again asks.
    f.http.route("api/get", 200, &record("Test Artist", "Opening", 200.0, Some(SYNCED), None).to_string());
    f.svc.look_again("501").await.unwrap();
    assert!(f.svc.get("501").await.unwrap().is_some());
    // Found lyrics stay cached through another Look again.
    f.svc.look_again("501").await.unwrap();
    let before = f.http.calls("lrclib");
    f.svc.get("501").await.unwrap();
    assert_eq!(f.http.calls("lrclib"), before);
}
