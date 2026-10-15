//! Identify by sound: fingerprints of made-up audio, the AcoustID reply and
//! request (a fake service, never the real one), the stored results, the
//! fingerprint tier in the matcher (never for unreleased songs), the library
//! doctor finding and the "sounds like" hint, and a whole round over HTTP
//! with a faked lookup.

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::acoustid::{self, AcoustId, Hit, LookupError, SoundLookup};
use super::print::{self, PrintError};
use super::store::{self, Todo};
use super::{Outcome, Scope, SoundId};
use crate::audio::StreamSource;
use crate::audio::gate::TransferGate;
use crate::db::{DbHandle, schema};
use crate::error::{AppError, AppResult};
use crate::identity::{Context, MatchKind, SongKey, Tier, compare, index};
use crate::plex::keychain::{MemoryStore, TokenStore};
use crate::tags::fetch::Reader;
use crate::tags::scan::PartSource;

const REC: &str = "0f0e0d0c-0b0a-4909-8807-060504030201";
const OTHER_REC: &str = "11111111-2222-4333-8444-555555555555";

// ------------------------------------------------------------------ fixtures

/// A WAV of `secs` seconds: a little tune of tones, different per `seed`.
fn wav(secs: u32, seed: u32) -> Vec<u8> {
    let rate = 22_050;
    let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut out = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut out, spec).unwrap();
        let notes = [220.0, 277.18, 329.63, 440.0, 392.0, 349.23, 293.66, 246.94];
        for i in 0..rate * secs {
            let t = i as f32 / rate as f32;
            let step = ((t * 2.0) as u32 + seed) as usize;
            let f = notes[step % notes.len()] * (1.0 + (seed % 3) as f32 * 0.5);
            let g = notes[(step * 3 + 1) % notes.len()];
            let v = 0.4 * (2.0 * std::f32::consts::PI * f * t).sin() + 0.2 * (2.0 * std::f32::consts::PI * g * t).sin();
            let s = (v * i16::MAX as f32) as i16;
            w.write_sample(s).unwrap();
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }
    out.into_inner()
}

fn conn() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
    schema::migrate(&mut c).unwrap();
    (dir, c)
}

fn album(c: &Connection, id: &str, artist: &str, title: &str) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, thumb, matched, added_at)
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, 1, 1)",
        params![id, title, format!("ar:{artist}"), artist, format!("/library/metadata/{id}/thumb/1")],
    )
    .unwrap();
}

fn song(c: &Connection, id: &str, album_id: &str, title: &str, ms: i64) {
    let (album_title, artist): (String, String) =
        c.query_row("SELECT title, artist_title FROM albums WHERE id = ?1", [album_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, codec, container, bitrate)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pcm', 'wav', 705)",
        params![id, title, album_id, album_title, format!("ar:{artist}"), artist, ms, format!("/p/{id}.wav")],
    )
    .unwrap();
}

/// The file's tags were read, with this recording id (or none).
fn tagged(c: &Connection, id: &str, recording: Option<&str>) {
    c.execute(
        "INSERT OR REPLACE INTO file_tags (part_key, status, recording_mbid, scanned_at, ver) VALUES (?1, 'ok', ?2, 1, ?3)",
        params![format!("/p/{id}.wav"), recording, crate::tags::CACHE_VERSION],
    )
    .unwrap();
}

fn demo(c: &Connection, album_id: &str) {
    c.execute("UPDATE albums SET subformats = '[\"Demo\"]' WHERE id = ?1", [album_id]).unwrap();
}

fn hit(score: f32, id: &str, title: &str, artist: &str) -> Hit {
    Hit { score, recording_mbid: id.into(), title: title.into(), artist: artist.into(), album: Some("Night Drive".into()) }
}

fn todo(c: &Connection, id: &str) -> Todo {
    store::pending(c, Scope::All, 100).unwrap().into_iter().find(|t| t.track_id == id).expect("pending")
}

/// Fingerprint, then an answer, the way a round saves them.
fn identified_as(c: &Connection, id: &str, hits: &[Hit]) {
    let t = todo(c, id);
    store::save_print(c, &t, "AQAA", 200, 1).unwrap();
    store::save_result(c, id, hits, 2).unwrap();
}

// ------------------------------------------------------------------ fingerprints

#[test]
fn a_fingerprint_is_made_from_the_audio_and_is_the_same_each_time() {
    let a = print::fingerprint(wav(20, 0), Some("wav")).unwrap();
    let again = print::fingerprint(wav(20, 0), Some("wav")).unwrap();
    let other = print::fingerprint(wav(20, 5), Some("wav")).unwrap();
    assert_eq!(a, again);
    assert_ne!(a, other, "different audio, different fingerprint");
    // Algorithm 1 (Chromaprint's default) in the first byte; URL-safe base64.
    assert!(a.starts_with("AQ"), "{a}");
    assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
}

#[test]
fn only_the_first_two_minutes_go_in() {
    let (_, secs) = print::raw(wav(130, 1), Some("wav")).unwrap();
    assert_eq!(secs, print::PRINT_SECONDS);
    let (_, secs) = print::raw(wav(30, 1), Some("wav")).unwrap();
    assert_eq!(secs, 30);
}

#[test]
fn a_file_cut_short_still_prints_and_too_little_or_junk_does_not() {
    let whole = wav(40, 2);
    let half = whole[..whole.len() / 2].to_vec();
    let (_, secs) = print::raw(half, Some("wav")).unwrap();
    assert!((18..=21).contains(&secs), "{secs}");
    assert_eq!(print::fingerprint(wav(5, 2), Some("wav")), Err(PrintError::TooShort));
    assert!(matches!(print::fingerprint(vec![7u8; 4096], None), Err(PrintError::Decode(_))));
}

#[test]
fn reads_are_sized_for_two_minutes() {
    // 320 kbps: 40 kB a second for 125 s, plus room for tags and art.
    assert_eq!(print::bytes_wanted(Some(320), 240_000, None), 40_000 * 125 + 1024 * 1024);
    // Never more than the file.
    assert_eq!(print::bytes_wanted(Some(320), 240_000, Some(3_000_000)), 3_000_000);
    // No bitrate: a share of the size.
    assert_eq!(print::bytes_wanted(None, 250_000, Some(50_000_000)), 25_000_000 + 1024 * 1024);
    // Nothing known: a cap.
    assert_eq!(print::bytes_wanted(None, 0, None), 40 * 1024 * 1024);
}

// ------------------------------------------------------------------ AcoustID

const REPLY: &str = r#"{"status":"ok","results":[
  {"id":"a","score":0.62,"recordings":[{"id":"22222222-2222-4333-8444-555555555555","title":"Weak","artists":[{"name":"Someone"}]}]},
  {"id":"b","score":0.97,"recordings":[
     {"id":"0F0E0D0C-0B0A-4909-8807-060504030201","title":"Night Drive","artists":[{"name":"Ana","joinphrase":" & "},{"name":"Bo"}],
      "releasegroups":[{"id":"x","title":"Night Drive (Single)","type":"Single"},{"id":"y","title":"Lights","type":"Album"}]},
     {"id":"33333333-2222-4333-8444-555555555555"}
  ]}
]}"#;

#[test]
fn replies_are_read_best_first_and_untitled_recordings_left_out() {
    let hits = acoustid::parse(REPLY).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].recording_mbid, REC, "lower case");
    assert_eq!(hits[0].title, "Night Drive");
    assert_eq!(hits[0].artist, "Ana & Bo");
    assert_eq!(hits[0].album.as_deref(), Some("Lights"), "an album before a single");
    assert!((hits[0].score - 0.97).abs() < 1e-6);
    assert_eq!(hits[1].title, "Weak");
    assert!(acoustid::parse(r#"{"status":"ok","results":[]}"#).unwrap().is_empty());
}

#[test]
fn a_refused_key_and_other_errors_are_told_apart() {
    let bad_key = r#"{"status":"error","error":{"code":4,"message":"invalid API key"}}"#;
    assert_eq!(acoustid::parse(bad_key), Err(LookupError::KeyRefused));
    let bad_fp = r#"{"status":"error","error":{"code":3,"message":"invalid fingerprint"}}"#;
    assert_eq!(acoustid::parse(bad_fp), Err(LookupError::Rejected("invalid fingerprint".into())));
    assert!(matches!(acoustid::parse("<html>"), Err(LookupError::Rejected(_))));
}

#[tokio::test]
async fn only_the_fingerprint_and_length_are_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/lookup"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REPLY))
        .expect(1)
        .mount(&server)
        .await;
    let a = AcoustId::with_url(reqwest::Client::new(), &format!("{}/v2/lookup", server.uri()));
    let hits = a.lookup("appkey", "AQABz0", 245).await.unwrap();
    assert_eq!(hits[0].recording_mbid, REC);
    let reqs: Vec<Request> = server.received_requests().await.unwrap();
    let form: Vec<(String, String)> = url::form_urlencoded::parse(&reqs[0].body).into_owned().collect();
    let mut names: Vec<&str> = form.iter().map(|(k, _)| k.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["client", "duration", "fingerprint", "format", "meta"]);
    let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    assert_eq!(get("fingerprint"), Some("AQABz0"));
    assert_eq!(get("duration"), Some("245"));
    assert_eq!(get("client"), Some("appkey"));
}

#[tokio::test]
async fn a_busy_service_pauses_lookups() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(429)).expect(1).mount(&server).await;
    let a = AcoustId::with_url(reqwest::Client::new(), &server.uri());
    assert_eq!(a.lookup("k", "AQ", 100).await, Err(LookupError::Busy));
    assert!(a.limiter().paused());
    // While paused nothing is sent.
    assert_eq!(a.lookup("k", "AQ", 100).await, Err(LookupError::Busy));
}

#[test]
fn lookups_stay_under_three_a_second() {
    const { assert!(acoustid::PER_SECOND < 3.0) };
}

// ------------------------------------------------------------------ store

#[test]
fn songs_are_picked_by_scope_and_done_once_per_file() {
    let (_d, c) = conn();
    album(&c, "a1", "Ana", "Night Drive");
    song(&c, "t1", "a1", "Night Drive", 200_000);
    song(&c, "t2", "a1", "Tagged", 200_000);
    song(&c, "t3", "a1", "Skit", 4_000);
    tagged(&c, "t2", Some(REC));
    let ids = |s| store::pending(&c, s, 10).unwrap().into_iter().map(|t| t.track_id).collect::<Vec<_>>();
    assert_eq!(ids(Scope::Untagged), ["t1"], "tagged and too-short songs are left out");
    let mut all = ids(Scope::All);
    all.sort();
    assert_eq!(all, ["t1", "t2"]);
    identified_as(&c, "t1", &[hit(0.95, REC, "Night Drive", "Ana")]);
    assert!(ids(Scope::Untagged).is_empty());
    let n = store::counts(&c, Scope::Untagged).unwrap();
    assert_eq!((n.total, n.done, n.identified), (1, 1, 1));
    // A changed file is done again.
    c.execute("UPDATE tracks SET part_key = '/p/t1-new.wav' WHERE id = 't1'", []).unwrap();
    assert_eq!(ids(Scope::Untagged), ["t1"]);
}

#[test]
fn weak_matches_are_kept_as_no_match() {
    let (_d, c) = conn();
    album(&c, "a1", "Ana", "Night Drive");
    song(&c, "t1", "a1", "Night Drive", 200_000);
    let t = todo(&c, "t1");
    store::save_print(&c, &t, "AQAA", 200, 1).unwrap();
    assert_eq!(store::printed(&c, 10).unwrap(), [("t1".to_string(), "AQAA".to_string(), 200)]);
    assert!(!store::save_result(&c, "t1", &[hit(0.5, REC, "Night Drive", "Ana")], 2).unwrap());
    assert!(store::printed(&c, 10).unwrap().is_empty(), "the fingerprint goes once answered");
    assert!(store::sounds_like(&c, "t1").unwrap().is_none());
}

// ------------------------------------------------------------------ matcher

fn key(title: &str, artist: &str) -> SongKey {
    SongKey::from_names(artist, title, "", 200_000)
}

#[test]
fn the_fingerprint_tier_sits_between_the_ids_and_the_names() {
    assert!(Tier::PlexTrackMbid < Tier::Fingerprint && Tier::Fingerprint < Tier::Text);
    // A mislabelled file ("Track 01") that sounds like a tagged song.
    let file = key("Track 01", "Unknown Artist").with_sound(Some(REC));
    let tagged = SongKey { recording_mbid: Some(REC.into()), ..key("Night Drive", "Ana") };
    let m = compare(&file, &tagged, Context::Single).unwrap();
    assert_eq!((m.tier, m.kind), (Tier::Fingerprint, MatchKind::SameRecording));
    // Two files that sound like the same recording.
    let other = key("Untitled", "").with_sound(Some(REC));
    assert_eq!(compare(&file, &other, Context::Single).unwrap().tier, Tier::Fingerprint);
    // Ids win over it.
    let both = SongKey { isrc: Some("USAAA0000001".into()), ..file.clone() };
    let both2 = SongKey { isrc: Some("USAAA0000001".into()), ..tagged.clone() };
    assert_eq!(compare(&both, &both2, Context::Single).unwrap().tier, Tier::Isrc);
}

#[test]
fn a_fingerprint_never_overrides_recording_ids_in_the_tags() {
    let a = SongKey { recording_mbid: Some(OTHER_REC.into()), ..key("Night Drive", "Ana") }.with_sound(Some(REC));
    let b = SongKey { recording_mbid: Some(REC.into()), ..key("Night Drive", "Ana") };
    let m = compare(&a, &b, Context::Single).unwrap();
    assert_ne!(m.tier, Tier::Fingerprint);
    assert_eq!(m.kind, MatchKind::OtherVersion, "two different tagged recordings");
}

#[test]
fn an_unreleased_song_never_matches_the_release_it_sounds_like() {
    let leak = SongKey { unreleased: true, ..key("Untitled 3", "Ana") }.with_sound(Some(REC));
    let release = SongKey { recording_mbid: Some(REC.into()), ..key("Night Drive", "Ana") };
    assert_eq!(compare(&leak, &release, Context::Single), None);
    assert_eq!(compare(&release, &leak, Context::Single), None);
}

#[test]
fn the_index_finds_songs_by_sound_but_never_unreleased_ones() {
    let (_d, c) = conn();
    album(&c, "a1", "Ana", "Night Drive");
    album(&c, "a2", "Ana", "Unknown Album");
    album(&c, "a3", "Ana", "Sessions");
    demo(&c, "a3");
    song(&c, "t1", "a1", "Night Drive", 200_000);
    tagged(&c, "t1", Some(REC));
    song(&c, "t2", "a2", "Track 01", 200_000);
    song(&c, "t3", "a3", "Untitled 3", 205_000);
    identified_as(&c, "t2", &[hit(0.95, REC, "Night Drive", "Ana")]);
    identified_as(&c, "t3", &[hit(0.9, REC, "Night Drive", "Ana")]);
    index::rebuild(&c).unwrap();
    let sound: Option<String> = c.query_row("SELECT sound_mbid FROM song_index WHERE track_id = 't2'", [], |r| r.get(0)).unwrap();
    assert_eq!(sound.as_deref(), Some(REC));

    // Looking for the release finds the mislabelled copy by its sound, not the demo.
    let want = SongKey { recording_mbid: Some(REC.into()), ..key("Night Drive", "Ana") };
    let found = index::lookup(&c, &want, Context::Single, &Default::default()).unwrap();
    let ids: Vec<(&str, Tier)> = found.iter().map(|x| (x.track_id.as_str(), x.matched.tier)).collect();
    assert!(ids.contains(&("t1", Tier::RecordingMbid)));
    assert!(ids.contains(&("t2", Tier::Fingerprint)));
    assert!(!ids.iter().any(|(id, _)| *id == "t3"), "the unreleased song is never a match");

    // And the demo's own sound finds nothing.
    let leak = SongKey { unreleased: true, ..key("Untitled 3", "Ana") }.with_sound(Some(REC));
    assert!(index::lookup(&c, &leak, Context::Single, &Default::default()).unwrap().is_empty());
}

// ------------------------------------------------------------------ doctor and hint

#[test]
fn a_mislabelled_file_is_a_doctor_finding_with_suggested_tags() {
    let (_d, c) = conn();
    album(&c, "a2", "Unknown Artist", "Unknown Album");
    song(&c, "t2", "a2", "Track 01", 200_000);
    identified_as(&c, "t2", &[hit(0.93, REC, "Night Drive", "Ana")]);
    index::rebuild(&c).unwrap();
    let f = crate::doctor::sounds::findings(&c).unwrap();
    assert_eq!(f.len(), 1);
    let f = &f[0];
    assert_eq!(f.kind, crate::doctor::Kind::SoundsLike);
    assert_eq!(f.fix, crate::doctor::Fix::SuggestedTags);
    assert!(f.title.contains("sounds like Night Drive by Ana"), "{}", f.title);
    let s = f.suggest.as_ref().unwrap();
    assert_eq!((s.title.as_str(), s.new_title.as_str(), s.new_artist.as_str()), ("Track 01", "Night Drive", "Ana"));
    assert_eq!(s.new_album.as_deref(), Some("Night Drive"));
    assert_eq!(s.score, 93);
    assert!(f.evidence.iter().any(|e| e.contains("Only the fingerprint")));
    // It shows on the page while the check is on, and not when it's off.
    let on = crate::doctor::Options::default();
    assert!(crate::doctor::store::all(&c, &on).unwrap().iter().any(|x| x.kind == crate::doctor::Kind::SoundsLike));
    let off = crate::doctor::Options {
        checks: crate::doctor::Checks { sounds_like: false, ..Default::default() },
        ..on
    };
    assert!(!crate::doctor::store::all(&c, &off).unwrap().iter().any(|x| x.kind == crate::doctor::Kind::SoundsLike));
}

#[test]
fn a_file_whose_names_already_agree_is_no_finding() {
    let (_d, c) = conn();
    album(&c, "a1", "Ana", "Night Drive");
    song(&c, "t1", "a1", "Night Drive", 200_000);
    identified_as(&c, "t1", &[hit(0.95, REC, "Night Drive", "Ana")]);
    index::rebuild(&c).unwrap();
    assert!(crate::doctor::sounds::findings(&c).unwrap().is_empty());
}

#[test]
fn an_unreleased_song_gets_a_hint_and_never_a_rename() {
    let (_d, c) = conn();
    album(&c, "a1", "Ana", "Night Drive");
    album(&c, "a3", "Ana", "Sessions");
    demo(&c, "a3");
    song(&c, "t1", "a1", "Night Drive", 200_000);
    tagged(&c, "t1", Some(REC));
    song(&c, "t3", "a3", "Untitled 3", 205_000);
    identified_as(&c, "t3", &[hit(0.9, REC, "Night Drive", "Ana")]);
    index::rebuild(&c).unwrap();
    assert!(crate::doctor::sounds::findings(&c).unwrap().is_empty(), "no finding suggests renaming it");
    let h = store::sounds_like(&c, "t3").unwrap().unwrap();
    assert!(h.unreleased);
    assert_eq!((h.title.as_str(), h.artist.as_str(), h.score), ("Night Drive", "Ana", 90));
    let copy = h.in_library.unwrap();
    assert_eq!((copy.track_id.as_str(), copy.album_id.as_str()), ("t1", "a1"));
    // Nothing unidentified has a hint.
    assert!(store::sounds_like(&c, "t1").unwrap().is_none());
}

// ------------------------------------------------------------------ a round

#[derive(Default)]
struct FakeLookup {
    calls: Mutex<Vec<(String, String, u32)>>,
    answer: Mutex<Option<Result<Vec<Hit>, LookupError>>>,
}

#[async_trait]
impl SoundLookup for FakeLookup {
    async fn lookup(&self, key: &str, fingerprint: &str, duration_s: u32) -> Result<Vec<Hit>, LookupError> {
        self.calls.lock().push((key.into(), fingerprint.into(), duration_s));
        self.answer.lock().clone().unwrap_or_else(|| Ok(vec![hit(0.96, REC, "Night Drive", "Ana")]))
    }
}

struct Parts(String);
#[async_trait]
impl PartSource for Parts {
    async fn part(&self, _track_id: &str, part_key: &str) -> AppResult<StreamSource> {
        if self.0.is_empty() {
            return Err(AppError::Network("offline".into()));
        }
        Ok(StreamSource { url: format!("{}{part_key}", self.0), ..Default::default() })
    }
}

async fn setup(base: String, built_in: Option<&str>) -> (tempfile::TempDir, DbHandle, SoundId, Arc<FakeLookup>, Arc<MemoryStore>) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    db.write(|c| {
        album(c, "a2", "Unknown Artist", "Unknown Album");
        song(c, "t2", "a2", "Track 01", 20_000);
        song(c, "t4", "a2", "Tagged", 20_000);
        tagged(c, "t4", Some(OTHER_REC));
        Ok(())
    })
    .unwrap();
    let lookup = Arc::new(FakeLookup::default());
    let keys = Arc::new(MemoryStore::new());
    let reader = Reader::new(crate::audio::http::http_client(), TransferGate::new(), None);
    let s = SoundId::new(db.clone(), Arc::new(Parts(base)), reader, lookup.clone(), keys.clone(), built_in);
    (dir, db, s, lookup, keys)
}

async fn serve_wav() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/p/t2.wav"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wav(20, 3)))
        .mount(&server)
        .await;
    server
}

fn yes() -> bool {
    true
}

#[tokio::test]
async fn a_round_fingerprints_looks_up_and_caches() {
    let server = serve_wav().await;
    let (_d, db, s, lookup, _) = setup(server.uri(), Some("builtin")).await;
    let r = s.run_round(Scope::Untagged, 10, Duration::ZERO, &yes).await.unwrap();
    assert_eq!(r, None);
    let calls = lookup.calls.lock().clone();
    assert_eq!(calls.len(), 1, "the tagged song is left alone");
    assert_eq!(calls[0].0, "builtin");
    assert!(calls[0].1.starts_with("AQ"));
    assert_eq!(calls[0].2, 20, "the whole file's length");
    let hint = db.read(|c| store::sounds_like(c, "t2")).unwrap().unwrap();
    assert_eq!(hint.recording_mbid, REC);
    // The matcher sees it.
    let sound: Option<String> =
        db.read(|c| Ok(c.query_row("SELECT sound_mbid FROM song_index WHERE track_id = 't2'", [], |r| r.get(0))?)).unwrap();
    assert_eq!(sound.as_deref(), Some(REC));
    // Done once: the next round reads and sends nothing.
    s.run_round(Scope::Untagged, 10, Duration::ZERO, &yes).await.unwrap();
    assert_eq!(lookup.calls.lock().len(), 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn without_a_key_nothing_is_read_or_sent() {
    let server = serve_wav().await;
    let (_d, _db, s, lookup, _) = setup(server.uri(), None).await;
    let st = s.status().await.unwrap();
    assert!(!st.configured && !st.app_key && !st.user_key);
    s.run_round(Scope::All, 10, Duration::ZERO, &yes).await.unwrap();
    assert!(lookup.calls.lock().is_empty());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_users_own_key_is_kept_in_the_keychain_and_used_first() {
    let server = serve_wav().await;
    let (_d, _db, s, lookup, keys) = setup(server.uri(), Some("builtin")).await;
    s.set_key(Some(" mine-123 ".into())).unwrap();
    assert_eq!(keys.get(super::KEY_ACCOUNT).unwrap().unwrap().expose(), "mine-123");
    let st = s.status().await.unwrap();
    assert!(st.user_key && st.app_key && st.configured);
    s.run_round(Scope::Untagged, 10, Duration::ZERO, &yes).await.unwrap();
    assert_eq!(lookup.calls.lock()[0].0, "mine-123");
    assert!(s.set_key(Some("not a key!".into())).is_err());
    s.set_key(None).unwrap();
    assert!(keys.get(super::KEY_ACCOUNT).unwrap().is_none());
    assert_eq!(s.key().as_deref(), Some("builtin"));
}

#[tokio::test]
async fn a_refused_key_keeps_the_fingerprint_for_later() {
    let server = serve_wav().await;
    let (_d, db, s, lookup, _) = setup(server.uri(), Some("old")).await;
    *lookup.answer.lock() = Some(Err(LookupError::KeyRefused));
    let r = s.run_round(Scope::Untagged, 10, Duration::ZERO, &yes).await.unwrap();
    assert_eq!(r, Some(Outcome::Waiting(LookupError::KeyRefused)));
    assert!(s.status().await.unwrap().key_refused);
    assert_eq!(db.read(|c| store::printed(c, 10)).unwrap().len(), 1);
    // Refused: no more lookups until a new key.
    s.run_round(Scope::Untagged, 10, Duration::ZERO, &yes).await.unwrap();
    assert_eq!(lookup.calls.lock().len(), 1);
    // A new key looks it up without reading the file again.
    *lookup.answer.lock() = None;
    s.set_key(Some("new".into())).unwrap();
    s.run_round(Scope::Untagged, 10, Duration::ZERO, &yes).await.unwrap();
    assert_eq!(lookup.calls.lock().len(), 2);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert!(db.read(|c| store::sounds_like(c, "t2")).unwrap().is_some());
}

#[tokio::test]
async fn trouble_reaching_the_file_stops_the_round_and_a_bad_file_is_not_tried_again() {
    // Offline: nothing saved, try later.
    let (_d, db, s, lookup, _) = setup(String::new(), Some("k")).await;
    assert_eq!(s.run_round(Scope::Untagged, 10, Duration::ZERO, &yes).await.unwrap(), Some(Outcome::Later));
    assert!(db.read(|c| store::pending(c, Scope::Untagged, 10)).unwrap().len() == 1);

    // A file that isn't audio: marked, never sent.
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_bytes(vec![9u8; 5000])).mount(&server).await;
    let (_d2, db2, s2, lookup2, _) = setup(server.uri(), Some("k")).await;
    assert_eq!(s2.run_round(Scope::Untagged, 10, Duration::ZERO, &yes).await.unwrap(), None);
    assert!(lookup2.calls.lock().is_empty());
    assert!(db2.read(|c| store::pending(c, Scope::Untagged, 10)).unwrap().is_empty());
    let n = db2.read(|c| store::counts(c, Scope::Untagged)).unwrap();
    assert_eq!((n.done, n.identified), (1, 0));
    assert!(lookup.calls.lock().is_empty());
}

#[tokio::test]
async fn turning_it_off_stops_a_round() {
    let server = serve_wav().await;
    let (_d, _db, s, lookup, _) = setup(server.uri(), Some("k")).await;
    s.run_round(Scope::All, 10, Duration::ZERO, &|| false).await.unwrap();
    assert!(lookup.calls.lock().is_empty());
}

/// The real service, with a key from the environment. Run by hand:
/// `PLATTER_ACOUSTID_TEST_KEY=... cargo test soundid -- --ignored`.
#[tokio::test]
#[ignore]
async fn live_lookup_answers() {
    let Ok(key) = std::env::var("PLATTER_ACOUSTID_TEST_KEY") else { return };
    let fp = print::fingerprint(wav(30, 4), Some("wav")).unwrap();
    let a = AcoustId::new(reqwest::Client::new());
    // Made-up audio matches nothing, but the key and request are accepted.
    let hits = a.lookup(&key, &fp, 30).await.unwrap();
    assert!(hits.iter().all(|h| h.score <= 1.0));
}
