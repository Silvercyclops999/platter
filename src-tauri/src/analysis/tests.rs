//! The background analysis over made-up songs: a fake file reader, the
//! real decoder and analysis, and the stored results.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::params;

use super::features::{RATE, synth};
use super::store::{self, Todo};
use super::{Analyzer, Outcome};
use crate::audio::automix::MixInfoSource;
use crate::db::DbHandle;
use crate::lyrics::transcribe::audio::AudioError;
use crate::lyrics::transcribe::{SongAudio, SongReader};

fn wav(samples: &[f32]) -> Vec<u8> {
    let spec = hound::WavSpec { channels: 1, sample_rate: RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut out = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut out, spec).unwrap();
        for s in samples {
            w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
    }
    out.into_inner()
}

#[derive(Default)]
struct Files {
    files: Mutex<HashMap<String, Result<Vec<u8>, AudioError>>>,
    reads: Mutex<Vec<String>>,
}

#[async_trait]
impl SongReader for Files {
    async fn read(&self, track_id: &str, _cancel: &AtomicBool) -> Result<SongAudio, AudioError> {
        self.reads.lock().push(track_id.to_string());
        match self.files.lock().get(track_id).cloned() {
            Some(Ok(bytes)) => Ok(SongAudio { bytes, hint: Some("wav".into()) }),
            Some(Err(e)) => Err(e),
            None => Err(AudioError::Unreadable("no such file".into())),
        }
    }
}

fn db() -> (tempfile::TempDir, DbHandle) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("lib.db")).unwrap();
    (dir, db)
}

fn song(db: &DbHandle, id: &str, secs: u64) {
    let id = id.to_string();
    db.write(move |c| {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key)
             VALUES (?1, ?1, 'al', 'Album', 'ar', 'Artist', ?2, ?3)",
            params![id, (secs * 1000) as i64, format!("/p/{id}.wav")],
        )?;
        Ok(())
    })
    .unwrap();
}

const CHORD: [f64; 3] = [261.63, 329.63, 392.0];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_round_analyses_songs_once_and_keeps_the_results() {
    let (_d, db) = db();
    let files = Arc::new(Files::default());
    for (id, bpm) in [("a", 120.0), ("b", 96.0)] {
        song(&db, id, 30);
        files.files.lock().insert(id.into(), Ok(wav(&synth::clicks(bpm, 30.0, 0.3, &CHORD))));
    }
    song(&db, "broken", 30);
    files.files.lock().insert("broken".into(), Ok(b"not audio at all".to_vec()));
    song(&db, "long", 3 * 60 * 60);
    let an = Analyzer::new(db.clone(), files.clone(), None);
    let done = Arc::new(Mutex::new(Vec::new()));
    {
        let d = done.clone();
        an.set_on_done(Arc::new(move |ids| d.lock().extend(ids)));
    }
    let r = an.run_round(10, Duration::ZERO, &|| true).await.unwrap();
    assert_eq!(r, None);
    let mut got = done.lock().clone();
    got.sort();
    assert_eq!(got, vec!["a".to_string(), "b".to_string()]);
    let infos = an.infos(&["a".into(), "b".into(), "broken".into()]).await;
    assert!((infos["a"].bpm.unwrap() - 120.0).abs() < 1.5, "{:?}", infos["a"].bpm);
    assert!((infos["b"].bpm.unwrap() - 96.0).abs() < 1.5, "{:?}", infos["b"].bpm);
    assert!(infos["a"].intro.is_some() && infos["a"].outro.is_some());
    assert!(!infos.contains_key("broken"));
    // The long song wasn't even read; nothing is read a second time.
    assert!(!files.reads.lock().contains(&"long".to_string()));
    let before = files.reads.lock().len();
    an.run_round(10, Duration::ZERO, &|| true).await.unwrap();
    assert_eq!(files.reads.lock().len(), before);
    let c = db.read(store::counts).unwrap();
    assert_eq!((c.total, c.done), (4, 4));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_file_is_done_again_and_wanted_songs_go_first() {
    let (_d, db) = db();
    let files = Arc::new(Files::default());
    for id in ["x", "y", "z"] {
        song(&db, id, 20);
        files.files.lock().insert(id.into(), Ok(wav(&synth::clicks(110.0, 20.0, 0.1, &CHORD))));
    }
    let an = Analyzer::new(db.clone(), files.clone(), None);
    an.want(&["x".to_string()]);
    an.run_round(1, Duration::ZERO, &|| true).await.unwrap();
    assert_eq!(files.reads.lock().as_slice(), ["x".to_string()], "the wanted song first");
    an.run_round(10, Duration::ZERO, &|| true).await.unwrap();
    assert_eq!(files.reads.lock().len(), 3);
    db.write(|c| {
        c.execute("UPDATE tracks SET part_key = '/p/x2.wav' WHERE id = 'x'", [])?;
        Ok(())
    })
    .unwrap();
    assert!(an.infos(&["x".into()]).await.is_empty(), "an old file's analysis isn't used");
    let pending = db.read(|c| store::pending(c, 10)).unwrap();
    assert_eq!(pending, vec![Todo { track_id: "x".into(), part_key: "/p/x2.wav".into(), duration_ms: 20_000 }]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_server_stops_the_round_for_later() {
    let (_d, db) = db();
    let files = Arc::new(Files::default());
    song(&db, "a", 20);
    song(&db, "b", 20);
    files.files.lock().insert("a".into(), Err(AudioError::Unreachable("offline".into())));
    files.files.lock().insert("b".into(), Err(AudioError::Unreachable("offline".into())));
    let an = Analyzer::new(db.clone(), files.clone(), None);
    let r = an.run_round(10, Duration::ZERO, &|| true).await.unwrap();
    assert_eq!(r, Some(Outcome::Later));
    assert_eq!(files.reads.lock().len(), 1, "stopped at the first");
    assert_eq!(db.read(|c| store::pending(c, 10)).unwrap().len(), 2, "both still to do");
    // Turned off mid-round: nothing more is read.
    let r = an.run_round(10, Duration::ZERO, &|| false).await.unwrap();
    assert_eq!(r, None);
    assert_eq!(files.reads.lock().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_busy_check_holds_analysis_back() {
    use crate::lyrics::transcribe::pace::{Pacer, fake};
    let (_d, db) = db();
    let files = Arc::new(Files::default());
    song(&db, "a", 20);
    files.files.lock().insert("a".into(), Ok(wav(&synth::clicks(100.0, 20.0, 0.1, &CHORD))));
    // A fullscreen game in front: busy at once.
    let load = fake::FakeLoad::new(5.0);
    load.in_front.store(true, std::sync::atomic::Ordering::SeqCst);
    let pacer = Pacer::with_cores(load.clone(), fake::timing(), 8);
    let an = Analyzer::new(db.clone(), files.clone(), Some(pacer));
    let task = {
        let an = an.clone();
        tokio::spawn(async move { an.run_round(10, Duration::ZERO, &|| true).await })
    };
    // Busy: readings come in, nothing is read.
    while load.reads() < 6 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(files.reads.lock().is_empty(), "waits while the computer is busy");
    load.in_front.store(false, std::sync::atomic::Ordering::SeqCst);
    task.await.unwrap().unwrap();
    assert_eq!(files.reads.lock().as_slice(), ["a".to_string()]);
}
