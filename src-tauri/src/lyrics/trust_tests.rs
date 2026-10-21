//! Lyrics you can trust: the check against the recording, the stricter
//! rules without it, and the user's choices. The speech model is faked: the
//! fake recognizer "hears" a fixed timeline of words for whatever window it
//! is handed (the window's start is written into the fake audio samples).
//!
//! The lyrics in these fixtures are made up for the tests.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use super::candidates::{self, Candidate, Decision, Origin};
use super::check::{self, HeardWord, Point, Verdict};
use super::checker::{self, AudioCheck, CheckEvents, CheckJob, CheckReason, CheckState, CheckStatus, Checker, CheckerDeps, JobCand, SongPcm};
use super::choice::{self, Choice};
use super::*;
use crate::artistinfo::http::fake::FakeHttp;
use crate::lyrics::transcribe::audio::{AudioError, RATE};
use crate::lyrics::transcribe::engine::{Progress, RecognizeError, Recognizer, Stop};
use crate::lyrics::transcribe::lines::{Segment, Word};
use crate::lyrics::transcribe::model::{ModelKind, ModelSpec, ModelStore};
use crate::lyrics::transcribe::pace::{Pacer, fake::{self, FakeLoad}};

// ---------------------------------------------------------------- fixtures

/// "KING", the song on the album: (time, line).
const KING_A: &[(u32, &str)] = &[
    (12_000, "Woke up in a palace made of paper"),
    (16_000, "Every window painted gold by hand"),
    (20_000, "They keep calling me the king of nothing"),
    (24_000, "Sitting on a throne of borrowed sand"),
    (28_000, "King king wear it while it lasts"),
    (32_000, "Every crown is rented from the past"),
    (36_000, "King king nobody stays on top"),
    (40_000, "Hold the room until the music stops"),
    (90_000, "Second morning and the walls are thinner"),
    (94_000, "Paper peeling where the painters stood"),
    (98_000, "Every servant learned to call me sir"),
    (102_000, "Nobody told me that it never would"),
    (170_000, "King king wear it while it lasts"),
    (174_000, "Every crown is rented from the past"),
    (178_000, "King king nobody stays on top"),
    (182_000, "Hold the room until the music stops"),
    (186_000, "Hold the room until the lights go out"),
    (190_000, "Hand it over when they call you out"),
];

/// Another song by the same artist, also called "KING": different words,
/// a similar length.
const KING_B: &[(u32, &str)] = &[
    (10_500, "Midnight on the bridge with no one watching"),
    (14_500, "Counting every headlight passing by"),
    (18_500, "Mama said a king don't need permission"),
    (22_500, "Just a reason and a place to cry"),
    (26_500, "King of the corner king of the cold"),
    (30_500, "Selling every story that we told"),
    (34_500, "King of the corner never been sold"),
    (38_500, "Write my name in silver not in gold"),
    (88_000, "Engines humming underneath the river"),
    (92_000, "Every tunnel echoes what we say"),
    (172_000, "King of the corner king of the cold"),
    (176_000, "Selling every story that we told"),
    (180_000, "King of the corner never been sold"),
    (184_000, "Write my name in silver not in gold"),
];

fn lrc(lines: &[(u32, &str)]) -> String {
    lines
        .iter()
        .map(|(t, l)| format!("[{:02}:{:02}.{:02}]{l}", t / 60_000, (t / 1000) % 60, (t % 1000) / 10))
        .collect::<Vec<_>>()
        .join("\n")
}

fn plain(lines: &[(u32, &str)]) -> String {
    lines.iter().map(|(_, l)| *l).collect::<Vec<_>>().join("\n")
}

fn synced(lines: &[(u32, &str)]) -> Lyrics {
    let mut l = parse_lyrics_text(&lrc(lines));
    l.source = LyricsSource::Lrclib;
    l
}

/// What a recording sounds like: each line's words, 350 ms apart, from
/// `at(t)` (the written time moved however the recording differs).
fn timeline(lines: &[(u32, &str)], at: impl Fn(u32) -> u32) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for (t, l) in lines {
        let start = at(*t);
        for (i, w) in l.split_whitespace().enumerate() {
            out.push((start + i as u32 * 350, w.to_string()));
        }
    }
    out
}

/// Heard words as the check keeps them.
fn heard(tl: &[(u32, String)], from: u32, to: u32) -> Vec<HeardWord> {
    tl.iter()
        .filter(|(t, _)| *t >= from && *t < to)
        .filter_map(|(t, w)| check::fold_word(w).map(|text| HeardWord { text, ms: *t }))
        .collect()
}

// ---------------------------------------------------------------- scoring

#[test]
fn the_right_song_matches_and_the_other_king_does_not() {
    let w = (10_000, 40_000);
    let rec = timeline(KING_A, |t| t);
    let h = heard(&rec, w.0, w.1);
    let a = check::judge(&h, &synced(KING_A), w);
    assert_eq!(a.verdict, Verdict::Match, "{a:?}");
    assert!(a.score > 0.8);
    let b = check::judge(&h, &synced(KING_B), w);
    assert_eq!(b.verdict, Verdict::NoMatch, "{b:?}");
    assert!(b.score < 0.15, "shared words like king and gold barely count: {b:?}");
    // And the other way round.
    let rec_b = timeline(KING_B, |t| t);
    let hb = heard(&rec_b, w.0, w.1);
    assert_eq!(check::judge(&hb, &synced(KING_B), w).verdict, Verdict::Match);
    assert_eq!(check::judge(&hb, &synced(KING_A), w).verdict, Verdict::NoMatch);
}

#[test]
fn mishearings_ad_libs_and_repeats_still_match() {
    // Misheard words, sung contractions and ad-libs between lines.
    let mut rec: Vec<(u32, String)> = Vec::new();
    let sung = [
        "woke up in the palace made of paper yeah",
        "every window painted cold by hand uh",
        "they keep callin me the king of nothin",
        "sittin on the throne of borrowed sand let's go",
    ];
    for (i, l) in sung.iter().enumerate() {
        for (j, w) in l.split_whitespace().enumerate() {
            rec.push((12_000 + i as u32 * 4000 + j as u32 * 300, w.into()));
        }
    }
    let h = heard(&rec, 0, 60_000);
    assert_eq!(check::judge(&h, &synced(KING_A), (10_000, 40_000)).verdict, Verdict::Match);
    // The chorus sung three times, written once in plain lyrics.
    let chorus = "king king wear it while it lasts every crown is rented from the past";
    let mut rec = Vec::new();
    let mut t = 0;
    for _ in 0..3 {
        for w in chorus.split_whitespace().chain(["oh", "yeah", "come", "on"]) {
            rec.push((t, w.to_string()));
            t += 300;
        }
    }
    let written = Lyrics { lines: plain(&KING_A[..8]).lines().map(String::from).collect(), ..Default::default() };
    let j = check::judge(&heard(&rec, 0, 60_000), &written, (0, 30_000));
    assert_eq!(j.verdict, Verdict::Match, "{j:?}");
}

#[test]
fn partial_hearing_or_partial_lyrics_is_never_a_wrong_song() {
    // Every other word lost or misheard.
    let rec: Vec<(u32, String)> = timeline(&KING_A[..8], |t| t)
        .into_iter()
        .enumerate()
        .map(|(i, (t, w))| (t, if i % 2 == 1 { "mumble".to_string() } else { w }))
        .collect();
    let j = check::judge(&heard(&rec, 10_000, 40_000), &synced(KING_A), (10_000, 40_000));
    assert_ne!(j.verdict, Verdict::NoMatch, "{j:?}");
    // Lyrics with only the first verse written down.
    let rec = timeline(KING_A, |t| t);
    let j = check::judge(&heard(&rec, 10_000, 44_000), &synced(&KING_A[..4]), (10_000, 44_000));
    assert_ne!(j.verdict, Verdict::NoMatch, "{j:?}");
}

#[test]
fn an_instrumental_window_and_another_language_cant_tell() {
    // Three words over a guitar solo.
    let rec = vec![(15_000, "hey".to_string()), (20_000, "yeah".into()), (25_000, "go".into()), (26_000, "now".into())];
    let j = check::judge(&heard(&rec, 10_000, 40_000), &synced(KING_A), (10_000, 40_000));
    assert_eq!(j.verdict, Verdict::CantTell);
    // Spanish lyrics; an English model hears English-ish sounds.
    let es = Lyrics {
        lines: vec![
            "Camino solo por la calle vacía".into(),
            "La luna me sigue sin decir nada".into(),
            "Mi corazón canta una canción perdida".into(),
            "Y el viento lleva tu nombre al mar".into(),
        ],
        ..Default::default()
    };
    assert!(!check::looks_english(&es));
    assert!(check::looks_english(&synced(KING_A)));
    let rec: Vec<(u32, String)> = "come in old sole bore la cal of a sea ya la loon a me see gay seen this here"
        .split_whitespace()
        .enumerate()
        .map(|(i, w)| (10_000 + i as u32 * 400, w.to_string()))
        .collect();
    assert_eq!(check::judge(&heard(&rec, 0, 60_000), &es, (10_000, 40_000)).verdict, Verdict::CantTell);
}

#[test]
fn a_steady_offset_is_measured_to_a_tenth_of_a_second() {
    let rec = timeline(KING_A, |t| t + 1530);
    let w = (10_000, 40_000);
    let j = check::judge(&heard(&rec, w.0, w.1 + 2000), &synced(KING_A), w);
    assert_eq!(j.verdict, Verdict::MatchOffset);
    assert_eq!(j.offset_ms, 1500);
    // Lyrics running late (heard earlier): a negative offset.
    let rec = timeline(KING_A, |t| t - 800);
    let j = check::judge(&heard(&rec, w.0 - 1000, w.1), &synced(KING_A), w);
    assert_eq!(j.offset_ms, -800);
    // Within 0.3 s is a plain match.
    let rec = timeline(KING_A, |t| t + 200);
    assert_eq!(check::judge(&heard(&rec, w.0, w.1), &synced(KING_A), w).verdict, Verdict::Match);
}

#[test]
fn drift_stretches_timing_between_two_points() {
    let points = [Point { at_ms: 12_000, offset_ms: 1500 }, Point { at_ms: 172_000, offset_ms: 3100 }];
    assert_eq!(check::offset_at(&points, 12_000), 1500);
    assert_eq!(check::offset_at(&points, 92_000), 2300);
    assert_eq!(check::offset_at(&points, 172_000), 3100);
    let mut l = synced(KING_A);
    check::retime(&mut l, &points, 0);
    assert_eq!(l.timed[0].start_ms, 13_500);
    assert_eq!(l.timed[12].start_ms, 170_000 + 3100);
    // One point is a steady shift; the user's nudge goes on top. Lines moved
    // before the start are dropped, except the one still playing at 0.
    let orig = synced(KING_A);
    let mut l = orig.clone();
    check::retime(&mut l, &[Point { at_ms: 0, offset_ms: -20_000 }], 500);
    assert_eq!(l.timed.len(), orig.timed.len() - 1, "the first line is gone, not piled on 0");
    assert_eq!(l.timed[0].text, orig.timed[1].text);
    assert_eq!(l.timed[0].start_ms, 0);
    assert_eq!(l.timed[1].start_ms, 500);
    assert!(l.timed.windows(2).all(|w| w[0].start_ms < w[1].start_ms));
}

#[test]
fn windows_skip_the_intro_and_need_room_for_a_second() {
    assert_eq!(check::first_window(Some(12_000), 205_000), (10_000, 40_000));
    // No timing: a little way in.
    assert_eq!(check::first_window(None, 200_000), (30_000, 60_000));
    // Short songs: the window fits inside.
    assert_eq!(check::first_window(Some(50_000), 60_000), (30_000, 60_000));
    let w2 = check::second_window((10_000, 40_000), Some(190_000), 205_000).unwrap();
    assert_eq!(w2, (165_000, 195_000));
    assert_eq!(check::second_window((10_000, 40_000), Some(60_000), 90_000), None);
}

#[test]
fn texts_that_differ_a_lot_disagree() {
    assert!(check::disagree(&synced(KING_A), &synced(KING_B)));
    // The same words, timed or not, agree.
    let p = Lyrics { lines: plain(KING_A).lines().map(String::from).collect(), ..Default::default() };
    assert!(!check::disagree(&synced(KING_A), &p));
    assert_ne!(check::lyrics_hash(&synced(KING_A)), check::lyrics_hash(&p));
    assert_eq!(check::lyrics_hash(&synced(KING_A)), check::lyrics_hash(&synced(KING_A)));
}

// ---------------------------------------------------------------- decide (pure)

fn cand(key: &str, origin: Origin, l: Lyrics) -> Candidate {
    Candidate::new(key, origin, l)
}

#[test]
fn decide_takes_the_first_match_and_skips_no_matches() {
    let cands = vec![
        cand("server", Origin::Server, synced(KING_B)),
        cand("lrclib:a", Origin::LrclibSearch, synced(KING_A)),
        cand("auto", Origin::Auto, synced(KING_A)),
    ];
    let no = candidates::Stored { verdict: Verdict::NoMatch, score: 0.0, points: vec![] };
    let yes = candidates::Stored { verdict: Verdict::Match, score: 0.9, points: vec![] };
    let v = |c: &Candidate| match c.key.as_str() {
        "server" => Some(&no),
        "lrclib:a" => Some(&yes),
        _ => None,
    };
    assert_eq!(candidates::decide(&cands, v, true, false), Decision::Show { idx: 1, note: Some(candidates::Note::Checked) });
    // Nothing judged yet: even a title search hit shows while it's checked.
    let none = |_: &Candidate| None;
    assert_eq!(candidates::decide(&cands[1..2], none, true, false), Decision::Show { idx: 0, note: Some(candidates::Note::Checking) });
    // The server's are shown while they're checked.
    assert_eq!(candidates::decide(&cands, none, true, false), Decision::Show { idx: 0, note: Some(candidates::Note::Checking) });
    // Every one disagreed.
    let all_no = |_: &Candidate| Some(&no);
    assert_eq!(candidates::decide(&cands[..2], all_no, true, false), Decision::Notice(Notice::NoneMatched));
    // Without the check: the first, not checked.
    assert_eq!(
        candidates::decide(&cands, none, false, false),
        Decision::Show { idx: 0, note: Some(candidates::Note::NotChecked) }
    );
}

#[test]
fn version_titles_are_recognised() {
    for t in ["Opening (Demo)", "Opening [Alternate Take]", "Opening - Acoustic", "Opening (Early Version)", "Opening (Remix"] {
        assert!(candidates::title_says_version(t), "{t}");
    }
    for t in ["Opening (Instrumental)", "Opening (Radio Edit)", "Opening - Sped Up", "Opening (Alt. Take)", "Opening (Reprise)"] {
        assert!(candidates::title_says_version(t), "{t}");
    }
    for t in ["Opening", "Demolition", "Remixed Feelings", "Opening (feat. Guest)"] {
        assert!(!candidates::title_says_version(t), "{t}");
    }
    // Whole words only.
    for t in ["Opening (Through the Fire)", "Opening (Demon)", "Opening (Obsession)", "Opening (Democracy)", "Opening (Salty)"] {
        assert!(!candidates::title_says_version(t), "{t}");
    }
}

#[test]
fn track_ids_from_every_server_kind_are_accepted() {
    for id in ["12345", "0a1b2c3d:12345", "0a1b2c3d:4b5e8c3f2a1d4e6f9a8b7c6d5e4f3a2b", "0a1b2c3d:4b5e8c3f-2a1d-4e6f-9a8b-7c6d5e4f3a2b", "0a1b2c3d:al-12_ab"] {
        assert!(crate::lyrics::user::valid_track_id(id), "{id}");
    }
    for id in ["", "0a1b2c3d:", "0a1b2c3d:../x", "0a1b2c3d:a/b", r"0a1b2c3d:a\b", "0a1b2c3d:a b", "0a1b2c3d:a\nb"] {
        assert!(!crate::lyrics::user::valid_track_id(id), "{id}");
    }
}

#[test]
fn stricter_lrclib_timing_rule() {
    // Within 2 s: timing kept.
    assert!(candidates::lrclib_sync_ok(205_000, "Crown Season", Some(206.5), Some("Anything")));
    // 2 to 5 s: only on the same album (edition notes don't count).
    assert!(!candidates::lrclib_sync_ok(205_000, "Crown Season", Some(209.0), Some("Night Bridge")));
    assert!(candidates::lrclib_sync_ok(205_000, "Crown Season", Some(209.0), Some("Crown Season (Deluxe Edition)")));
    // Further: never; no length: never.
    assert!(!candidates::lrclib_sync_ok(205_000, "Crown Season", Some(211.0), Some("Crown Season")));
    assert!(!candidates::lrclib_sync_ok(205_000, "Crown Season", None, Some("Crown Season")));
}

// ---------------------------------------------------------------- the service with a fake model

/// Fake audio: a tone whose level carries the time in ms, so the fake
/// recognizer knows which window it was handed.
fn song_pcm(duration_ms: u32) -> Vec<f32> {
    let n = duration_ms as usize * RATE as usize / 1000;
    (0..n).map(|i| 0.1 + (i * 1000 / RATE as usize) as f32 * 1e-7).collect()
}

fn window_start(pcm: &[f32]) -> u32 {
    ((pcm[0] - 0.1) / 1e-7).round() as u32
}

struct FakeRecognizer {
    /// Track id → what its recording sounds like (the fake audio carries the track).
    heard: Mutex<Vec<(u32, String)>>,
    calls: AtomicUsize,
    threads: AtomicUsize,
}

impl Recognizer for FakeRecognizer {
    fn unavailable(&self) -> Option<String> {
        None
    }
    fn transcribe(&self, _: &Path, _: ModelKind, pcm: &[f32], threads: usize, _: &crate::lyrics::transcribe::engine::Hints, _: Stop, _: Progress) -> Result<Vec<Segment>, RecognizeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.threads.store(threads, Ordering::SeqCst);
        let start = window_start(pcm);
        let end = start + (pcm.len() * 1000 / RATE as usize) as u32;
        let words: Vec<Word> = self
            .heard
            .lock()
            .iter()
            .filter(|(t, _)| *t >= start && *t < end)
            .map(|(t, w)| Word { start_ms: t - start, end_ms: t - start + 250, text: format!(" {w}") })
            .collect();
        Ok(if words.is_empty() { vec![] } else { vec![Segment { words, sure: None }] })
    }
}

/// The song's audio; never arrives while the flag is set.
struct FakePcm(Mutex<Result<Vec<f32>, AudioError>>, AtomicBool);

#[async_trait]
impl SongPcm for FakePcm {
    async fn pcm(&self, _: &str) -> Result<Vec<f32>, AudioError> {
        while self.1.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.0.lock().clone()
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<String>>, Mutex<Vec<CheckStatus>>);

impl CheckEvents for Events {
    fn checked(&self, track_id: &str) {
        self.0.lock().push(track_id.into());
    }
    fn status(&self, s: &CheckStatus) {
        self.1.lock().push(s.clone());
    }
}

#[derive(Default)]
struct FakePlex(Mutex<Vec<(String, Lyrics)>>);

#[async_trait]
impl ServerLyrics for FakePlex {
    async fn lyrics(&self, id: &str) -> AppResult<Option<Lyrics>> {
        Ok(self.0.lock().iter().find(|(t, _)| t == id).map(|(_, l)| l.clone()))
    }
}

struct Fx {
    svc: LyricsService,
    checker: Checker,
    rec: Arc<FakeRecognizer>,
    pcm: Arc<FakePcm>,
    events: Arc<Events>,
    http: Arc<FakeHttp>,
    plex: Arc<FakePlex>,
    check_on: Arc<AtomicBool>,
    fix_timing: Arc<AtomicBool>,
    released_ok: Arc<AtomicBool>,
    official_first: Arc<AtomicBool>,
    none_matched: Arc<Mutex<Vec<String>>>,
    load: Arc<FakeLoad>,
    db: DbHandle,
    models: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

const MODEL_SIZE: u64 = 64;

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            r#"
            INSERT INTO artists (id, title, sort_title) VALUES ('7', 'Test Artist', 'Test Artist');
            INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, track_count)
              VALUES ('70', 'Crown Season', 'Crown Season', '7', 'Test Artist', 'album', '[]', 10),
                     ('80', 'Basement Tapes', 'Basement Tapes', '7', 'Test Artist', 'album', '["demo"]', 4);
            INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key)
              VALUES ('701', 'KING', '70', 'Crown Season', '7', 'Test Artist', 205000, '/p/1'),
                     ('702', 'Opening', '70', 'Crown Season', '7', 'Test Artist', 200000, '/p/2'),
                     ('703', 'Opening (Alternate Version)', '70', 'Crown Season', '7', 'Test Artist', 200000, '/p/3'),
                     ('801', 'KING', '80', 'Basement Tapes', '7', 'Test Artist', 205000, '/p/4');
            "#,
        )?;
        Ok(())
    })
    .unwrap();
    let http = Arc::new(FakeHttp::new());
    let plex = Arc::new(FakePlex::default());
    let check_on = Arc::new(AtomicBool::new(true));
    let fix_timing = Arc::new(AtomicBool::new(true));
    let released_ok = Arc::new(AtomicBool::new(false));
    let official_first = Arc::new(AtomicBool::new(true));
    let switches = Switches {
        lrclib: Arc::new(AtomicBool::new(true)),
        fix_timing: fix_timing.clone(),
        released_ok: released_ok.clone(),
        official_first: official_first.clone(),
        ..Default::default()
    };
    let svc = LyricsService::with_gap(db.clone(), http.clone(), plex.clone(), switches, Duration::from_millis(1));
    let models = dir.path().join("models");
    let specs = vec![ModelSpec {
        kind: ModelKind::Small,
        file: "small.bin".into(),
        url: "https://models.invalid/small.bin".into(),
        size: MODEL_SIZE,
        sha256: String::new(),
    }];
    let rec = Arc::new(FakeRecognizer { heard: Mutex::new(Vec::new()), calls: AtomicUsize::new(0), threads: AtomicUsize::new(0) });
    let load = FakeLoad::new(5.0);
    let pcm = Arc::new(FakePcm(Mutex::new(Ok(song_pcm(205_000))), AtomicBool::new(false)));
    let events = Arc::new(Events::default());
    let none_matched = Arc::new(Mutex::new(Vec::new()));
    let checker = Checker::new(CheckerDeps {
        db: db.clone(),
        models: ModelStore::new(models.clone(), specs, Arc::new(|_: &Path| Some(u64::MAX))),
        recognizer: rec.clone(),
        audio: pcm.clone(),
        events: events.clone(),
        enabled: check_on.clone(),
        none_matched: Some({
            let n = none_matched.clone();
            Arc::new(move |id: &str| n.lock().push(id.into()))
        }),
        pacer: Pacer::with_cores(load.clone(), fake::timing(), 16),
    });
    svc.set_checker(Arc::new(checker.clone()));
    let f = Fx {
        svc,
        checker,
        rec,
        pcm,
        events,
        http,
        plex,
        check_on,
        fix_timing,
        released_ok,
        official_first,
        none_matched,
        load,
        db,
        models,
        _dir: dir,
    };
    f.install_model();
    f
}

impl Fx {
    fn install_model(&self) {
        std::fs::create_dir_all(&self.models).unwrap();
        std::fs::write(self.models.join("small.bin"), vec![0u8; MODEL_SIZE as usize]).unwrap();
    }

    fn remove_model(&self) {
        std::fs::remove_file(self.models.join("small.bin")).unwrap();
    }

    fn hears(&self, tl: Vec<(u32, String)>) {
        *self.rec.heard.lock() = tl;
    }

    /// LRCLIB: no exact match, these search hits.
    fn lrclib_search(&self, hits: serde_json::Value) {
        self.http.route("api/get", 404, "{}");
        self.http.route("api/search", 200, &hits.to_string());
    }

    fn rows(&self, sql: &str) -> i64 {
        self.db.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
    }
}

fn hit(album: &str, duration: f64, synced: &str) -> serde_json::Value {
    serde_json::json!({
        "id": 1, "trackName": "KING", "artistName": "Test Artist", "albumName": album,
        "duration": duration, "instrumental": false, "syncedLyrics": synced, "plainLyrics": null
    })
}

fn exact(album: &str, duration: f64, synced: &str) -> serde_json::Value {
    let mut h = hit(album, duration, synced);
    h["trackName"] = "Opening".into();
    h
}

#[tokio::test]
async fn same_title_collision_never_shows_the_wrong_song() {
    let f = fx();
    // LRCLIB has both songs called KING; the other one is closer in length.
    f.lrclib_search(serde_json::json!([hit("Crown Season", 207.0, &lrc(KING_A)), hit("Night Bridge", 204.5, &lrc(KING_B))]));
    f.hears(timeline(KING_A, |t| t + 1500));
    // Before the check: the closest title hit shows at once, marked as being checked.
    let s = f.svc.shown("701").await.unwrap().unwrap();
    assert_eq!((s.notice, s.check), (None, Some(candidates::Note::Checking)));
    assert!(!s.lyrics.lines.is_empty() || !s.lyrics.timed.is_empty());
    assert_eq!(f.checker.queued(), vec!["701"]);
    f.checker.drain().await.unwrap();
    assert_eq!(*f.events.0.lock(), vec!["701"]);
    // After: the song that was heard, moved by the measured 1.5 s.
    let s = f.svc.shown("701").await.unwrap().unwrap();
    assert_eq!(s.check, Some(candidates::Note::Checked));
    assert_eq!(s.lyrics.timed[0].text, KING_A[0].1);
    assert_eq!(s.lyrics.timed[0].start_ms, 13_500);
    assert_eq!(s.auto_offset_ms, 1500);
    assert!(f.checker.queued().is_empty(), "nothing left to check");
    // The picker shows both with their results.
    let list = f.svc.candidates("701").await.unwrap();
    let checks: Vec<_> = list.items.iter().map(|c| (c.album.clone().unwrap(), c.check)).collect();
    assert_eq!(
        checks,
        vec![("Night Bridge".to_string(), Some(Verdict::NoMatch)), ("Crown Season".to_string(), Some(Verdict::MatchOffset))]
    );
    assert!(list.items[1].shown && !list.items[0].shown);
    assert!(list.can_check);
}

#[tokio::test]
async fn a_check_says_where_it_is_and_a_stuck_one_says_it_couldnt_check_yet() {
    let f = fx();
    f.lrclib_search(serde_json::json!([hit("Crown Season", 205.0, &lrc(KING_A))]));
    f.hears(timeline(KING_A, |t| t));
    f.checker.spawn();
    f.svc.shown("701").await.unwrap();
    for _ in 0..300 {
        if f.events.0.lock().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let states: Vec<CheckState> = f.events.1.lock().iter().filter(|s| s.track_id == "701").map(|s| s.state).collect();
    assert_eq!(states, vec![CheckState::Queued, CheckState::Reading, CheckState::Listening, CheckState::Done]);
    assert_eq!(f.checker.status("701").unwrap().state, CheckState::Done);
    // The song's audio never comes: after the stuck time it couldn't check yet.
    let f = fx();
    f.lrclib_search(serde_json::json!([hit("Crown Season", 205.0, &lrc(KING_A))]));
    f.hears(timeline(KING_A, |t| t));
    f.pcm.1.store(true, Ordering::SeqCst);
    f.checker.spawn();
    f.svc.shown("701").await.unwrap();
    for _ in 0..300 {
        if f.checker.status("701").is_some_and(|s| s.state == CheckState::Failed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let s = f.checker.status("701").unwrap();
    assert_eq!((s.state, s.reason), (CheckState::Failed, Some(CheckReason::Stuck)));
    assert!(f.events.1.lock().iter().any(|s| s.state == CheckState::Reading), "it said what it was doing first");
    // The audio arrives after all: the check finishes and says so.
    f.pcm.1.store(false, Ordering::SeqCst);
    for _ in 0..300 {
        if f.checker.status("701").is_some_and(|s| s.state == CheckState::Done) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(f.checker.status("701").unwrap().state, CheckState::Done);
}

#[tokio::test]
async fn a_check_waiting_on_a_busy_computer_says_so() {
    let f = fx();
    f.lrclib_search(serde_json::json!([hit("Crown Season", 205.0, &lrc(KING_A))]));
    f.hears(timeline(KING_A, |t| t));
    f.load.set(90.0);
    f.checker.spawn();
    f.svc.shown("701").await.unwrap();
    // A hang guard, not a time budget: a busy CI machine can take a while.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if f.checker.status("701").is_some_and(|s| s.state == CheckState::Waiting && s.reason.is_some()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(f.checker.status("701").unwrap().reason, Some(CheckReason::Busy));
}

#[tokio::test]
async fn the_check_waits_while_the_computer_is_busy_and_uses_two_threads() {
    let f = fx();
    f.lrclib_search(serde_json::json!([hit("Crown Season", 205.0, &lrc(KING_A))]));
    f.hears(timeline(KING_A, |t| t));
    f.load.set(90.0);
    f.checker.spawn();
    assert_eq!(f.svc.shown("701").await.unwrap().unwrap().check, Some(candidates::Note::Checking));
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(f.load.reads() > 0, "the processor is read while a song waits");
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), 0, "nothing runs while the computer is busy");
    assert!(f.events.0.lock().is_empty());
    f.load.set(5.0);
    for _ in 0..300 {
        if !f.events.0.lock().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(*f.events.0.lock(), vec!["701"]);
    assert_eq!(f.rec.threads.load(Ordering::SeqCst), 2, "background work gets two threads");
}

#[tokio::test]
async fn same_title_collision_the_other_way_round() {
    let f = fx();
    // Only the other KING is on LRCLIB, and the recording is this one.
    f.lrclib_search(serde_json::json!([hit("Night Bridge", 205.0, &lrc(KING_B))]));
    f.hears(timeline(KING_A, |t| t));
    assert_eq!(f.svc.shown("701").await.unwrap().unwrap().check, Some(candidates::Note::Checking));
    f.checker.drain().await.unwrap();
    let s = f.svc.shown("701").await.unwrap().unwrap();
    assert_eq!(s.notice, Some(Notice::NoneMatched));
    assert!(s.lyrics.timed.is_empty() && s.lyrics.lines.is_empty());
    assert_eq!(f.svc.get("701").await.unwrap(), None);
    // Every candidate disagreed: a full transcription is asked for.
    assert_eq!(*f.none_matched.lock(), vec!["701"]);
}

#[tokio::test]
async fn the_check_stops_at_the_first_match_and_is_cached_by_text() {
    let f = fx();
    f.lrclib_search(serde_json::json!([hit("Night Bridge", 205.0, &lrc(KING_B)), hit("Crown Season", 205.5, &lrc(KING_A))]));
    // The server's lyrics come first and are right.
    let mut server = synced(KING_A);
    server.source = LyricsSource::Plex;
    f.plex.0.lock().push(("701".into(), server));
    f.hears(timeline(KING_A, |t| t));
    let s = f.svc.shown("701").await.unwrap().unwrap();
    // The server's are shown while they're checked.
    assert_eq!(s.check, Some(candidates::Note::Checking));
    assert_eq!(s.pick.as_deref(), Some("server"));
    f.checker.drain().await.unwrap();
    // Judged: the server's (a match, checked again near the end for drift).
    // The LRCLIB hits after it weren't.
    assert_eq!(f.rows("SELECT count(*) FROM lyrics_check"), 1);
    assert_eq!(f.rows("SELECT count(*) FROM lyrics_heard"), 2);
    let s = f.svc.shown("701").await.unwrap().unwrap();
    assert_eq!((s.check, s.pick.as_deref()), (Some(candidates::Note::Checked), Some("server")));
    assert!(f.checker.queued().is_empty());
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), 2);
    // The server's text changes: a new check, but the heard words are reused.
    let mut lines = KING_A.to_vec();
    lines[3] = (24_000, "Sitting on a throne of stolen sand");
    let mut changed = synced(&lines);
    changed.source = LyricsSource::Plex;
    f.plex.0.lock()[0].1 = changed;
    assert_eq!(f.svc.shown("701").await.unwrap().unwrap().check, Some(candidates::Note::Checking));
    f.checker.drain().await.unwrap();
    assert_eq!(f.rows("SELECT count(*) FROM lyrics_check"), 2);
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), 2, "the windows weren't transcribed again");
}

#[tokio::test]
async fn drift_is_measured_at_a_second_window_and_stretched() {
    let f = fx();
    f.lrclib_search(serde_json::json!([hit("Crown Season", 205.0, &lrc(KING_A))]));
    // The recording runs slower: 1.5 s late at the start, 3.1 s by the end.
    f.hears(timeline(KING_A, |t| t + 1500 + (t.saturating_sub(12_000)) / 100));
    f.svc.shown("701").await.unwrap();
    f.checker.drain().await.unwrap();
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), 2, "two windows");
    let s = f.svc.shown("701").await.unwrap().unwrap();
    assert!(s.retimed);
    let first = s.lyrics.timed[0].start_ms as i64;
    let last = s.lyrics.timed.last().unwrap().start_ms as i64;
    assert!((first - 13_500).abs() <= 200, "{first}");
    let want_last = 190_000 + 1500 + (190_000 - 12_000) / 100;
    assert!((last - want_last as i64).abs() <= 300, "{last} vs {want_last}");
    // "Fix lyrics timing automatically" off: checked, shown as written.
    f.fix_timing.store(false, Ordering::Relaxed);
    let s = f.svc.shown("701").await.unwrap().unwrap();
    assert_eq!(s.check, Some(candidates::Note::Checked));
    assert_eq!(s.lyrics.timed[0].start_ms, 12_000);
    assert!(!s.retimed && s.auto_offset_ms == 0);
}

#[tokio::test]
async fn without_the_model_stricter_rules_apply_and_say_not_checked() {
    let f = fx();
    f.remove_model();
    assert!(!f.checker.available());
    // 4 s longer, another album: words only.
    f.http.route("api/get", 200, &exact("Somewhere Else", 204.0, &lrc(KING_A)).to_string());
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!(s.check, Some(candidates::Note::NotChecked));
    assert!(!s.lyrics.synced && s.lyrics.lines.len() == KING_A.len());
    assert!(f.checker.queued().is_empty(), "nothing is checked without the model");
    // Same album: kept in time.
    let f = fx();
    f.remove_model();
    f.http.route("api/get", 200, &exact("Crown Season (Deluxe)", 204.0, &lrc(KING_A)).to_string());
    assert!(f.svc.shown("702").await.unwrap().unwrap().lyrics.synced);
    // The setting off is the same as no model.
    let f = fx();
    f.check_on.store(false, Ordering::Relaxed);
    f.http.route("api/get", 200, &exact("Somewhere Else", 201.5, &lrc(KING_A)).to_string());
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert!(s.lyrics.synced && s.check == Some(candidates::Note::NotChecked));
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn candidates_that_disagree_without_the_model_say_so() {
    let f = fx();
    f.remove_model();
    f.lrclib_search(serde_json::json!([hit("Night Bridge", 205.0, &lrc(KING_B)), hit("Crown Season", 206.0, &lrc(KING_A))]));
    let s = f.svc.shown("701").await.unwrap().unwrap();
    assert_eq!(s.check, Some(candidates::Note::NotChecked));
    assert!(s.disagree);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["disagree"], true);
    assert_eq!(v["check"], "notChecked");
    // One hit only: nothing to disagree with.
    let f = fx();
    f.remove_model();
    f.lrclib_search(serde_json::json!([hit("Crown Season", 206.0, &lrc(KING_A))]));
    assert!(!f.svc.shown("701").await.unwrap().unwrap().disagree);
}

#[tokio::test]
async fn other_versions_wait_for_use_anyway_unless_the_audio_matches() {
    let f = fx();
    f.remove_model();
    f.lrclib_search(serde_json::json!([hit("Crown Season", 205.0, &lrc(KING_A))]));
    // An unreleased take of KING: the album's lyrics are held back.
    let s = f.svc.shown("801").await.unwrap().unwrap();
    assert_eq!(s.notice, Some(Notice::VersionHeld));
    let key = s.pick.clone().unwrap();
    assert!(key.starts_with("lrclib:"));
    // "Use anyway" picks them for this song.
    f.svc.choose("801", Some(key.clone())).await.unwrap();
    let s = f.svc.shown("801").await.unwrap().unwrap();
    assert!(s.chosen && s.lyrics.synced && s.notice.is_none());
    // With the model, the recording decides.
    let f = fx();
    f.lrclib_search(serde_json::json!([hit("Crown Season", 205.0, &lrc(KING_A))]));
    f.hears(timeline(KING_A, |t| t));
    // Shown at once while the recording decides.
    assert_eq!(f.svc.shown("801").await.unwrap().unwrap().check, Some(candidates::Note::Checking));
    f.checker.drain().await.unwrap();
    let s = f.svc.shown("801").await.unwrap().unwrap();
    assert_eq!(s.check, Some(candidates::Note::Checked));
    assert!(s.lyrics.synced);
    // A title marker counts too.
    let f = fx();
    f.remove_model();
    f.http.route("api/get", 404, "{}");
    f.http.route(
        "api/search",
        200,
        &serde_json::json!([{
            "trackName": "Opening (Alternate Version)", "artistName": "Test Artist", "albumName": "Single",
            "duration": 200.0, "syncedLyrics": lrc(KING_A)
        }])
        .to_string(),
    );
    assert_eq!(f.svc.shown("703").await.unwrap().unwrap().notice, Some(Notice::VersionHeld));
    f.released_ok.store(true, Ordering::Relaxed);
    assert!(f.svc.shown("703").await.unwrap().unwrap().notice.is_none());
}

#[tokio::test]
async fn a_quiet_or_unreadable_song_cant_tell_and_shows_not_checked() {
    let f = fx();
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_A)).to_string());
    *f.pcm.0.lock() = Ok(vec![0.0; 200 * RATE as usize]);
    f.svc.shown("702").await.unwrap();
    f.checker.drain().await.unwrap();
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), 0, "silence isn't transcribed");
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!(s.check, Some(candidates::Note::NotChecked));
    assert!(s.lyrics.synced);
    // The server can't be reached: not checked, asked again later.
    let f = fx();
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_A)).to_string());
    *f.pcm.0.lock() = Err(AudioError::Unreachable("offline".into()));
    f.svc.shown("702").await.unwrap();
    f.checker.drain().await.unwrap();
    assert_eq!(f.svc.shown("702").await.unwrap().unwrap().check, Some(candidates::Note::NotChecked));
    let expires = f.rows("SELECT expires_at - checked_at FROM lyrics_check");
    assert_eq!(expires, 3600);
    assert!(f.checker.queued().is_empty());
}

#[tokio::test]
async fn nothing_leaves_the_computer_for_the_check() {
    let f = fx();
    f.lrclib_search(serde_json::json!([hit("Crown Season", 205.0, &lrc(KING_A))]));
    f.hears(timeline(KING_A, |t| t));
    f.svc.shown("701").await.unwrap();
    let before = f.http.urls();
    f.checker.drain().await.unwrap();
    f.svc.shown("701").await.unwrap();
    // The check made no requests (LRCLIB answers came from the cache), and
    // only words and the length were ever sent.
    assert_eq!(f.http.urls(), before);
    assert!(before.iter().all(|u| u.starts_with("https://lrclib.net/api/")));
}

#[tokio::test]
async fn the_users_own_lyrics_and_transcriptions_arent_checked() {
    let f = fx();
    f.svc.save_own("702", &lrc(KING_A), true).await.unwrap();
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!(s.lyrics.source, LyricsSource::User);
    assert_eq!(s.check, None);
    assert!(f.checker.queued().is_empty());
}

// ---------------------------------------------------------------- choices

#[tokio::test]
async fn choices_pick_none_and_offsets() {
    let f = fx();
    f.remove_model();
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_A)).to_string());
    // The user's offset moves the lines (positive: later).
    f.svc.set_offset("702", 520).await.unwrap();
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!(s.offset_ms, 500, "rounded to 0.1 s");
    assert_eq!(s.lyrics.timed[0].start_ms, 12_500);
    // "No lyrics for this song".
    f.svc.choose("702", Some(choice::NONE.into())).await.unwrap();
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!(s.notice, Some(Notice::ChosenNone));
    assert_eq!(f.svc.get("702").await.unwrap(), None);
    // The picker still lists what was found, and a new pick resets the offset.
    let list = f.svc.candidates("702").await.unwrap();
    assert_eq!(list.pick.as_deref(), Some("none"));
    let key = list.items[0].key.clone();
    f.svc.choose("702", Some(key.clone())).await.unwrap();
    assert_eq!(f.svc.choice("702").await.unwrap(), Choice { pick: Some(key), offset_ms: 0 });
    // Back to automatic: the row goes.
    f.svc.choose("702", None).await.unwrap();
    assert_eq!(f.rows("SELECT count(*) FROM lyrics_choice"), 0);
    // A pick that isn't there any more falls back to automatic.
    f.svc.choose("702", Some("lrclib:00ff".into())).await.unwrap();
    assert!(!f.svc.shown("702").await.unwrap().unwrap().chosen);
    // Refused: unknown picks, far offsets, bad ids.
    assert_eq!(f.svc.choose("702", Some("http://x".into())).await.unwrap_err().kind(), "invalid");
    assert_eq!(f.svc.set_offset("702", 90_000).await.unwrap_err().kind(), "invalid");
    assert_eq!(f.svc.choose("../x", None).await.unwrap_err().kind(), "invalid");
    // Counted and cleared.
    f.svc.set_offset("701", -300).await.unwrap();
    assert_eq!(f.svc.choice_count().await.unwrap(), 2);
    assert_eq!(f.svc.clear_choices().await.unwrap(), vec!["701", "702"]);
    assert_eq!(f.svc.choice_count().await.unwrap(), 0);
}

#[tokio::test]
async fn choices_travel_in_the_sync_file() {
    use crate::sync::store::{LYRICS_CHOICES, SyncStore};
    let f = fx();
    let store = SyncStore::new(f.db.clone(), "pc");
    let c = Choice { pick: Some("server".into()), offset_ms: -700 };
    crate::sync::lyrics::chose(&store, "702", &c).unwrap();
    assert_eq!(store.list(&LYRICS_CHOICES).unwrap(), vec![("702".to_string(), c.clone())]);
    // Another device's choice arrives in the store: the table follows.
    assert_eq!(crate::sync::lyrics::apply_choices(&store, &f.db).unwrap(), 1);
    assert_eq!(f.svc.choice("702").await.unwrap(), c);
    assert_eq!(crate::sync::lyrics::apply_choices(&store, &f.db).unwrap(), 0, "nothing new");
    // Removed there: removed here.
    crate::sync::lyrics::chose(&store, "702", &Choice::default()).unwrap();
    assert_eq!(crate::sync::lyrics::apply_choices(&store, &f.db).unwrap(), 1);
    assert_eq!(f.svc.choice("702").await.unwrap(), Choice::default());
    // Bad entries are skipped.
    store.put(&LYRICS_CHOICES, "703", &Choice { pick: Some("evil".into()), offset_ms: 0 }).unwrap();
    assert_eq!(crate::sync::lyrics::apply_choices(&store, &f.db).unwrap(), 0);
}

// ---------------------------------------------------------------- the worker

#[tokio::test]
async fn requests_merge_and_the_newest_song_goes_first() {
    let f = fx();
    let job = |id: &str| CheckJob {
        track_id: id.into(),
        duration_ms: 205_000,
        cands: vec![JobCand { key: "server".into(), hash: "h".into(), lyrics: synced(KING_A) }],
        fix_timing: true,
    };
    for id in ["1", "2", "1", "3", "4"] {
        f.checker.request(job(id));
    }
    assert_eq!(f.checker.queued(), vec!["4", "3", "1"], "older requests drop off");
    let stored = checker::to_stored(
        &check::Judged { verdict: Verdict::Match, score: 0.9, offset_ms: 400, at_ms: Some(20_000) },
        None,
    );
    assert_eq!(stored.verdict, Verdict::MatchOffset);
    assert_eq!(stored.points, vec![Point { at_ms: 20_000, offset_ms: 400 }]);
}

// ---------------------------------------------------------------- sure no match, transcriptions fill in

fn judged(verdict: Verdict, score: f32) -> check::Judged {
    check::Judged { verdict, score, offset_ms: 0, at_ms: None }
}

#[test]
fn a_no_match_needs_two_sure_windows_that_agree() {
    let no = judged(Verdict::NoMatch, 0.05);
    // One window alone can't tell.
    assert_eq!(check::agree(&[(no.clone(), 40)]).verdict, Verdict::CantTell);
    // Two that agree, each with plenty heard: no match.
    assert_eq!(check::agree(&[(no.clone(), 40), (no.clone(), 30)]).verdict, Verdict::NoMatch);
    // A window that matches, or scores between the lines, says otherwise.
    assert_eq!(check::agree(&[(no.clone(), 40), (no.clone(), 30), (judged(Verdict::Match, 0.8), 30)]).verdict, Verdict::CantTell);
    assert_eq!(check::agree(&[(no.clone(), 40), (no.clone(), 30), (judged(Verdict::CantTell, 0.3), 30)]).verdict, Verdict::CantTell);
    // Few words heard, or a score near the line, isn't sure.
    assert_eq!(check::agree(&[(no.clone(), 40), (no.clone(), check::SURE_HEARD - 1)]).verdict, Verdict::CantTell);
    assert_eq!(check::agree(&[(no.clone(), 40), (judged(Verdict::NoMatch, 0.2), 30)]).verdict, Verdict::CantTell);
    // The other windows come from other parts of the song.
    let w1 = check::first_window(Some(10_000), 200_000);
    let more = check::confirm_windows(w1, 200_000);
    assert_eq!(more, vec![(90_000, 120_000), (140_000, 170_000), (50_000, 80_000)]);
    assert!(check::confirm_windows(check::first_window(Some(10_000), 40_000), 40_000).is_empty(), "too short for another");
}

#[test]
fn a_transcription_only_fills_in() {
    let cands = vec![cand("lrclib:a", Origin::LrclibExact, synced(KING_A)), cand("auto", Origin::Auto, synced(KING_B))];
    let none = |_: &Candidate| None;
    let cant = candidates::Stored { verdict: Verdict::CantTell, score: 0.1, points: vec![] };
    let no = candidates::Stored { verdict: Verdict::NoMatch, score: 0.0, points: vec![] };
    // Found lyrics come first: being checked, not checked, or can't tell.
    assert_eq!(candidates::decide(&cands, none, true, false), Decision::Show { idx: 0, note: Some(candidates::Note::Checking) });
    assert_eq!(candidates::decide(&cands, none, false, false), Decision::Show { idx: 0, note: Some(candidates::Note::NotChecked) });
    let v = |c: &Candidate| (c.key != "auto").then_some(&cant);
    assert_eq!(candidates::decide(&cands, v, true, false), Decision::Show { idx: 0, note: Some(candidates::Note::NotChecked) });
    // Surely wrong, or nothing found: the transcription.
    let v = |c: &Candidate| (c.key != "auto").then_some(&no);
    assert_eq!(candidates::decide(&cands, v, true, false), Decision::Show { idx: 1, note: None });
    assert_eq!(candidates::decide(&cands[1..], none, true, false), Decision::Show { idx: 0, note: None });
    // A transcription is never sent to the check, wherever it sits.
    assert!(!cands[1].checkable() && !cands[1].trusted());
    assert_eq!(candidates::winner(&cands, none), None);
}

/// A recording sung all the way through: KING_A's lines, one every 4 s.
fn sung_throughout() -> Vec<(u32, String)> {
    let lines: Vec<(u32, &str)> = (0..49).map(|i| (500 + i as u32 * 4000, KING_A[i % KING_A.len()].1)).collect();
    timeline(&lines, |t| t)
}

#[tokio::test]
async fn one_window_no_match_keeps_found_lyrics_and_doesnt_transcribe() {
    let f = fx();
    // The exact lookup has lyrics; the recording's first part sounds
    // different and the rest has no words the model can make out.
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_B)).to_string());
    f.hears(timeline(&KING_A[..8], |t| t));
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!((s.check, s.lyrics.timed[0].text.as_str()), (Some(candidates::Note::Checking), KING_B[0].1), "shown while checking");
    f.checker.drain().await.unwrap();
    assert!(f.rec.calls.load(Ordering::SeqCst) > 1, "other windows were heard");
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!(s.check, Some(candidates::Note::NotChecked));
    assert_eq!(s.lyrics.timed[0].text, KING_B[0].1);
    assert!(f.none_matched.lock().is_empty(), "no transcription");
    let list = f.svc.candidates("702").await.unwrap();
    assert_eq!(list.items[0].check, Some(Verdict::CantTell));
}

#[tokio::test]
async fn two_agreeing_windows_hide_found_lyrics_and_allow_a_transcription() {
    let f = fx();
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_B)).to_string());
    f.hears(timeline(KING_A, |t| t));
    f.svc.shown("702").await.unwrap();
    f.checker.drain().await.unwrap();
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), 2, "the first window and the middle agreed");
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!(s.notice, Some(Notice::NoneMatched));
    assert_eq!(*f.none_matched.lock(), vec!["702"]);
    // The transcription that follows fills the song.
    f.db.write(|c| crate::lyrics::transcribe::store::save(c, "702", &lrc(KING_A), "small", 1).map(|_| ())).unwrap();
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!((s.pick.as_deref(), s.lyrics.source), (Some("auto"), LyricsSource::Auto));
}

#[tokio::test]
async fn quiet_windows_are_skipped_when_making_sure() {
    let f = fx();
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_B)).to_string());
    f.hears(sung_throughout());
    // The middle of the song is silent.
    let mut pcm = song_pcm(205_000);
    pcm[85 * RATE as usize..125 * RATE as usize].fill(0.0);
    *f.pcm.0.lock() = Ok(pcm);
    f.svc.shown("702").await.unwrap();
    f.checker.drain().await.unwrap();
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), 2, "the silent window wasn't transcribed");
    assert_eq!(f.rows("SELECT count(*) FROM lyrics_heard"), 3);
    assert_eq!(f.svc.shown("702").await.unwrap().unwrap().notice, Some(Notice::NoneMatched));
    assert_eq!(*f.none_matched.lock(), vec!["702"]);
}

#[tokio::test]
async fn a_transcription_doesnt_outrank_found_lyrics_but_one_asked_for_does() {
    let f = fx();
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_A)).to_string());
    f.db.write(|c| crate::lyrics::transcribe::store::save(c, "702", &lrc(KING_B), "medium", 1).map(|_| ())).unwrap();
    // Being checked, then can't tell: the found lyrics either way.
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!((s.check, s.lyrics.source), (Some(candidates::Note::Checking), LyricsSource::Lrclib));
    f.checker.drain().await.unwrap();
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!((s.check, s.lyrics.source), (Some(candidates::Note::NotChecked), LyricsSource::Lrclib));
    // Without the check too.
    f.remove_model();
    assert_eq!(f.svc.shown("702").await.unwrap().unwrap().lyrics.source, LyricsSource::Lrclib);
    // "Transcribe this song" picks the transcription: it wins.
    let list = f.svc.candidates("702").await.unwrap();
    assert!(list.items.iter().any(|c| c.key == "auto" && !c.shown));
    f.svc.choose("702", Some("auto".into())).await.unwrap();
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert!(s.chosen);
    assert_eq!((s.pick.as_deref(), s.lyrics.source), (Some("auto"), LyricsSource::Auto));
}

#[tokio::test]
async fn after_the_upgrade_found_lyrics_show_again_over_a_transcription() {
    let f = fx();
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_A)).to_string());
    f.remove_model();
    let key = f.svc.candidates("702").await.unwrap().items[0].key.clone();
    f.install_model();
    // An older build: one window said no match, and a transcription followed.
    let hash = key.trim_start_matches("lrclib:").to_owned();
    f.db.write(|c| {
        checker::save(c, "702", &key, &hash, &candidates::Stored { verdict: Verdict::NoMatch, score: 0.1, points: vec![] }, 1, None)?;
        crate::lyrics::transcribe::store::save(c, "702", &lrc(KING_B), "medium", 1).map(|_| ())
    })
    .unwrap();
    assert_eq!(f.svc.shown("702").await.unwrap().unwrap().pick.as_deref(), Some("auto"));
    // The upgrade.
    f.db.write(|c| {
        c.pragma_update(None, "user_version", 35)?;
        crate::db::schema::migrate(c)
    })
    .unwrap();
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!((s.pick.as_deref(), s.check), (Some(key.as_str()), Some(candidates::Note::Checking)));
    // Checked again, and the transcription is still there to pick.
    f.hears(timeline(KING_A, |t| t));
    f.checker.drain().await.unwrap();
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!((s.pick.as_deref(), s.check), (Some(key.as_str()), Some(candidates::Note::Checked)));
    assert!(f.svc.candidates("702").await.unwrap().items.iter().any(|c| c.key == "auto"));
    assert_eq!(f.rows("SELECT count(*) FROM user_lyrics WHERE auto IS NOT NULL"), 1);
}

// ---------------------------------------------------------------- official lyrics first

use crate::lyrics::transcribe::Official;

#[tokio::test]
async fn official_synced_lyrics_are_checked_before_transcribing() {
    let f = fx();
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_A)).to_string());
    f.hears(timeline(KING_A, |t| t + 800));
    // Not judged yet: checked now, and they match.
    assert_eq!(f.svc.official_synced("702").await, Official::Matched);
    assert!(f.rec.calls.load(Ordering::SeqCst) >= 1);
    // Asked again: the stored verdict answers, nothing is transcribed.
    let calls = f.rec.calls.load(Ordering::SeqCst);
    assert_eq!(f.svc.official_synced("702").await, Official::Matched);
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), calls);
    // Shown, moved by the measured offset.
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!((s.check, s.lyrics.timed[0].start_ms), (Some(candidates::Note::Checked), 12_800));
}

#[tokio::test]
async fn official_lyrics_that_dont_match_leave_the_song_to_transcribe() {
    let f = fx();
    // The other song called KING, synced, while the recording is KING_A throughout.
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_B)).to_string());
    f.hears(sung_throughout());
    assert_eq!(f.svc.official_synced("702").await, Official::NoneMatched);
    // Plain lyrics only: nothing synced to show in time.
    let f = fx();
    f.http.route("api/get", 200, &serde_json::json!({"id": 1, "trackName": "Opening", "artistName": "Test Artist",
        "albumName": "Crown Season", "duration": 200.0, "instrumental": false, "syncedLyrics": null,
        "plainLyrics": plain(KING_A)}).to_string());
    f.hears(timeline(KING_A, |t| t));
    assert_eq!(f.svc.official_synced("702").await, Official::NoneMatched);
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), 0, "nothing to check");
}

#[tokio::test]
async fn official_first_can_be_turned_off_and_a_chosen_transcription_is_kept() {
    let f = fx();
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_A)).to_string());
    f.hears(timeline(KING_A, |t| t));
    f.official_first.store(false, Ordering::SeqCst);
    assert_eq!(f.svc.official_synced("702").await, Official::Unknown);
    f.official_first.store(true, Ordering::SeqCst);
    // "Transcribe this song" picks the transcription: it stays.
    f.svc.choose("702", Some("auto".into())).await.unwrap();
    assert_eq!(f.svc.official_synced("702").await, Official::Unknown);
    assert_eq!(f.rec.calls.load(Ordering::SeqCst), 0);
    // Without the check it can't look.
    f.svc.choose("702", None).await.unwrap();
    f.check_on.store(false, Ordering::SeqCst);
    assert_eq!(f.svc.official_synced("702").await, Official::Unknown);
}

#[tokio::test]
async fn a_transcriptions_steady_offset_is_measured_and_corrected() {
    let f = fx();
    // A transcription a second early against what the check hears.
    f.db.write(|c| crate::lyrics::transcribe::store::save(c, "702", &lrc(KING_A), "medium", 1).map(|_| ())).unwrap();
    f.hears(timeline(KING_A, |t| t + 1000));
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!((s.pick.as_deref(), s.lyrics.timed[0].start_ms), (Some("auto"), 12_000));
    assert_eq!(f.checker.queued(), vec!["702"], "its timing is measured");
    f.checker.drain().await.unwrap();
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert_eq!((s.pick.as_deref(), s.lyrics.timed[0].start_ms, s.auto_offset_ms), (Some("auto"), 13_000, 1000));
    assert_eq!(s.check, None, "a transcription isn't labelled checked");
    assert!(f.none_matched.lock().is_empty());
    // With "Fix lyrics timing automatically" off it keeps its own timing.
    f.fix_timing.store(false, Ordering::SeqCst);
    assert_eq!(f.svc.shown("702").await.unwrap().unwrap().lyrics.timed[0].start_ms, 12_000);
}

#[tokio::test]
async fn a_chosen_transcription_is_measured_too() {
    let f = fx();
    f.http.route("api/get", 200, &exact("Crown Season", 200.0, &lrc(KING_A)).to_string());
    f.db.write(|c| crate::lyrics::transcribe::store::save(c, "702", &lrc(KING_A), "medium", 1).map(|_| ())).unwrap();
    f.svc.choose("702", Some("auto".into())).await.unwrap();
    f.hears(timeline(KING_A, |t| t + 1000));
    f.svc.shown("702").await.unwrap();
    f.checker.drain().await.unwrap();
    let s = f.svc.shown("702").await.unwrap().unwrap();
    assert!(s.chosen);
    assert_eq!((s.pick.as_deref(), s.lyrics.timed[0].start_ms), (Some("auto"), 13_000));
}
