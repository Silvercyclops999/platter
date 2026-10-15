//! Sharing a transcription to LRCLIB: when it's offered, what is sent, and
//! the publish steps against a fake LRCLIB.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::json;
use wiremock::matchers::{header_regex, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::publish::{self, Payload, PublishError, Publisher};
use super::share::{self, LyricsShare, Options, WhyNot};
use super::transcribe::store as tstore;
use super::user;
use crate::db::DbHandle;

const LRC: &str = "[00:01.00]<00:01.00>Hello <00:01.30>there\n[00:06.00]\n[00:08.00]<00:08.00>Second <00:08.40>line\n";
const ON: Options = Options { offer: true, unreleased: false };

fn db() -> (DbHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            r#"
            INSERT INTO artists (id, title, sort_title) VALUES ('7', 'Test Artist', 'Test Artist');
            INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, track_count, studio, matched)
              VALUES ('50', 'First Light', 'First Light', '7', 'Test Artist', 'album', '[]', 2, 'Night Records', 1),
                     ('80', 'Basement Tapes', 'Basement Tapes', '7', 'Test Artist', 'album', '["demo"]', 1, NULL, 1);
            INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, track_number)
              VALUES ('501', 'Opening', '50', 'First Light', '7', 'Test Artist', 200500, '/p/1', 1),
                     ('502', 'No Length', '50', 'First Light', '7', 'Test Artist', 0, '/p/2', 2),
                     ('801', 'Tape One', '80', 'Basement Tapes', '7', 'Test Artist', 190000, '/p/3', 1);
            "#,
        )?;
        for id in ["501", "502", "801"] {
            tstore::save(c, id, LRC, "medium", 100)?;
        }
        Ok(())
    })
    .unwrap();
    (db, dir)
}

fn info(db: &DbHandle, id: &str, o: Options) -> share::ShareInfo {
    db.read(|c| share::info(c, id, o, false)).unwrap()
}

fn reviewed(db: &DbHandle, id: &str) {
    db.write(|c| share::mark_reviewed(c, id, 1)).unwrap();
}

#[test]
fn enhanced_lrc_goes_out_as_line_lrc_and_plain_words() {
    let (synced, plain) = publish::lrc_for_lrclib(LRC);
    assert_eq!(synced, "[00:01.00]Hello there\n[00:06.00]\n[00:08.00]Second line");
    assert_eq!(plain, "Hello there\n\nSecond line");
}

#[test]
fn the_dialog_shows_exactly_what_is_sent() {
    let (db, _d) = db();
    reviewed(&db, "501");
    let i = info(&db, "501", ON);
    assert!(i.offer && i.why_not.is_none() && !i.unreleased);
    let p = i.payload.unwrap();
    assert_eq!(
        p,
        Payload {
            track_name: "Opening".into(),
            artist_name: "Test Artist".into(),
            album_name: "First Light".into(),
            duration: 201.0,
            plain_lyrics: "Hello there\n\nSecond line".into(),
            synced_lyrics: "[00:01.00]Hello there\n[00:06.00]\n[00:08.00]Second line".into(),
        }
    );
    // The JSON LRCLIB gets, field for field.
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["trackName"], "Opening");
    assert_eq!(v["syncedLyrics"].as_str().unwrap().lines().count(), 3);
}

#[test]
fn not_offered_before_the_lyrics_are_reviewed() {
    let (db, _d) = db();
    let i = info(&db, "501", ON);
    assert!(!i.offer);
    assert_eq!(i.why_not, Some(WhyNot::NotReviewed));
    assert!(i.payload.is_none(), "nothing to send before");
    reviewed(&db, "501");
    assert!(info(&db, "501", ON).offer);
}

#[test]
fn only_transcriptions_with_full_details_are_offered() {
    let (db, _d) = db();
    reviewed(&db, "502");
    assert_eq!(info(&db, "502", ON).why_not, Some(WhyNot::MissingInfo), "no length");
    // Edited in the editor: the user's own now, not a transcription.
    db.write(|c| user::put(c, "501", "[00:01.00]Mine", false, 2)).unwrap();
    reviewed(&db, "501");
    assert_eq!(info(&db, "501", ON).why_not, Some(WhyNot::NotTranscribed));
    assert_eq!(info(&db, "999", ON).why_not, Some(WhyNot::NotTranscribed));
}

#[test]
fn unreleased_songs_need_the_setting_and_say_so() {
    let (db, _d) = db();
    reviewed(&db, "801");
    let i = info(&db, "801", ON);
    assert!(!i.offer && i.unreleased);
    assert_eq!(i.why_not, Some(WhyNot::Unreleased));
    let i = info(&db, "801", Options { offer: true, unreleased: true });
    assert!(i.offer && i.unreleased, "offered, with the extra warning");
}

#[test]
fn the_setting_off_and_shared_songs_are_not_offered() {
    let (db, _d) = db();
    reviewed(&db, "501");
    assert_eq!(info(&db, "501", Options { offer: false, unreleased: true }).why_not, Some(WhyNot::Off));
    assert_eq!(db.read(|c| share::info(c, "501", ON, true)).unwrap().why_not, Some(WhyNot::Shared), "shared on another device");
    db.write(|c| share::mark_shared(c, "501", 5)).unwrap();
    assert_eq!(info(&db, "501", ON).why_not, Some(WhyNot::Shared));
}

// ---------------------------------------------------------------- LRCLIB

/// An easy target: about one nonce in 16 answers.
const EASY: &str = "0FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";

/// Publishes only with a token that answers the challenge.
struct CheckToken;

impl Respond for CheckToken {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let token = req.headers.get("X-Publish-Token").and_then(|v| v.to_str().ok()).unwrap_or_default();
        let target: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&EASY[i * 2..i * 2 + 2], 16).unwrap()).collect();
        match token.split_once(':') {
            Some(("abc123", nonce)) if publish::answers("abc123", &target, nonce) => {
                let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
                assert_eq!(body["trackName"], "Opening");
                assert_eq!(body["albumName"], "First Light");
                assert_eq!(body["duration"], 201.0);
                ResponseTemplate::new(201)
            }
            _ => ResponseTemplate::new(400).set_body_json(json!({
                "message": "The provided publish token is incorrect",
                "name": "IncorrectPublishTokenError",
                "statusCode": 400
            })),
        }
    }
}

async fn lrclib(publish_with: impl Respond + 'static) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/request-challenge"))
        .and(header_regex("user-agent", r"^Platter/\d"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "prefix": "abc123", "target": EASY })))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/api/publish")).respond_with(publish_with).mount(&server).await;
    server
}

fn sample() -> Payload {
    Payload {
        track_name: "Opening".into(),
        artist_name: "Test Artist".into(),
        album_name: "First Light".into(),
        duration: 201.0,
        plain_lyrics: "Hello there".into(),
        synced_lyrics: "[00:01.00]Hello there".into(),
    }
}

fn no_cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn the_challenge_is_solved_below_the_target() {
    let never = AtomicBool::new(false);
    let nonce = publish::solve("abc123", EASY, &never).unwrap();
    let target: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&EASY[i * 2..i * 2 + 2], 16).unwrap()).collect();
    assert!(publish::answers("abc123", &target, &nonce));
    assert!(publish::solve("abc123", "nothex", &never).is_none());
}

#[tokio::test]
async fn publishes_with_a_solved_token() {
    let server = lrclib(CheckToken).await;
    let p = Publisher::with_base(&format!("{}/api", server.uri()));
    p.publish(&sample(), no_cancel()).await.unwrap();
    let got = server.received_requests().await.unwrap();
    assert_eq!(got.len(), 2, "a challenge, then the publish");
}

#[tokio::test]
async fn a_rejection_says_why() {
    let server = lrclib(
        ResponseTemplate::new(400).set_body_json(json!({ "message": "Lyrics are too short", "name": "ValidationError", "statusCode": 400 })),
    )
    .await;
    let p = Publisher::with_base(&format!("{}/api", server.uri()));
    let e = p.publish(&sample(), no_cancel()).await.unwrap_err();
    assert_eq!(e, PublishError::Rejected("Lyrics are too short".into()));
    assert_eq!(e.message(), "LRCLIB didn't take these lyrics: Lyrics are too short");
}

#[tokio::test]
async fn busy_and_offline_are_told_apart() {
    let server = lrclib(ResponseTemplate::new(503)).await;
    let p = Publisher::with_base(&format!("{}/api", server.uri()));
    assert_eq!(p.publish(&sample(), no_cancel()).await.unwrap_err(), PublishError::Busy);
    // Nothing listening there.
    let gone = Publisher::with_base("http://127.0.0.1:9/api");
    let e = gone.publish(&sample(), no_cancel()).await.unwrap_err();
    assert_eq!(e, PublishError::Offline);
    assert!(e.message().contains("Couldn't reach LRCLIB"));
}

#[tokio::test]
async fn solving_can_be_cancelled() {
    let server = MockServer::start().await;
    // A target nothing reaches.
    Mock::given(method("POST"))
        .and(path("/api/request-challenge"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "prefix": "abc123", "target": "0".repeat(64) })))
        .mount(&server)
        .await;
    let p = Publisher::with_base(&format!("{}/api", server.uri()));
    let cancel = no_cancel();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        c.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let e = tokio::time::timeout(Duration::from_secs(10), p.publish(&sample(), cancel)).await.unwrap().unwrap_err();
    assert_eq!(e, PublishError::Cancelled);
    assert_eq!(server.received_requests().await.unwrap().len(), 1, "nothing published");
}

#[tokio::test]
async fn sharing_remembers_success_and_changes_nothing_on_failure() {
    let (db, _d) = db();
    reviewed(&db, "501");
    // Refused before review: LRCLIB isn't asked at all.
    let server = lrclib(ResponseTemplate::new(400).set_body_json(json!({ "message": "No" }))).await;
    let s = LyricsShare::new(Publisher::with_base(&format!("{}/api", server.uri())));
    let e = s.share(&db, "801", ON, false).await.unwrap_err();
    assert!(e.to_string().contains("unreleased"), "{e}");
    assert!(server.received_requests().await.unwrap().is_empty());
    // A rejection: still offered.
    assert!(s.share(&db, "501", ON, false).await.is_err());
    assert!(info(&db, "501", ON).offer);

    let server = lrclib(CheckToken).await;
    let s = LyricsShare::new(Publisher::with_base(&format!("{}/api", server.uri())));
    s.share(&db, "501", ON, false).await.unwrap();
    assert_eq!(info(&db, "501", ON).why_not, Some(WhyNot::Shared), "no second offer");
    assert!(s.share(&db, "501", ON, false).await.is_err());
}
