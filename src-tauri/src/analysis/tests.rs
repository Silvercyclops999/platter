//! The background analysis over made-up songs: a fake file reader, the
//! real decoder and analysis, and the stored results.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::params;

use super::features::{RATE, synth};
use super::store::{self, Todo};
use super::{Analyzer, Outcome, Worker};
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

type Hook = Box<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
struct Files {
    files: Mutex<HashMap<String, Result<Vec<u8>, AudioError>>>,
    reads: Mutex<Vec<String>>,
    /// Songs whose read never finishes until it's cancelled.
    hang: Mutex<Vec<String>>,
    /// Called as each read starts.
    on_read: Mutex<Option<Hook>>,
}

#[async_trait]
impl SongReader for Files {
    async fn read(&self, track_id: &str, cancel: &AtomicBool) -> Result<SongAudio, AudioError> {
        self.reads.lock().push(track_id.to_string());
        if let Some(h) = self.on_read.lock().as_ref() {
            h(track_id);
        }
        if self.hang.lock().iter().any(|h| h == track_id) {
            while !cancel.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            return Err(AudioError::Cancelled);
        }
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
    let infos = an.infos(&["a".into(), "b".into(), "broken".into()]).await.unwrap();
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
    assert!(an.infos(&["x".into()]).await.unwrap().is_empty(), "an old file's analysis isn't used");
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

fn status_of(db: &DbHandle, id: &str) -> Option<String> {
    let id = id.to_string();
    db.read(move |c| Ok(c.query_row("SELECT status FROM track_analysis WHERE track_id = ?1", [&id], |r| r.get(0)).ok())).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panic_in_the_worker_loses_that_job_only() {
    let w = Worker::start();
    let r = w.run(|| -> u32 { panic!("bad file") }).await.unwrap();
    assert_eq!(r, Err("bad file".to_string()));
    assert_eq!(w.run(|| 2 + 2).await.unwrap(), Ok(4), "the thread carries on");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_song_that_crashes_the_analysis_is_saved_as_failed() {
    let (_d, db) = db();
    let files = Arc::new(Files::default());
    for id in ["a", "b"] {
        song(&db, id, 20);
        files.files.lock().insert(id.into(), Ok(wav(&synth::clicks(120.0, 20.0, 0.1, &CHORD))));
    }
    let an = Analyzer::new(db.clone(), files.clone(), None);
    *an.0.panic_on.lock() = Some("b".into());
    an.run_round(10, Duration::ZERO, &|| true).await.unwrap();
    assert_eq!(status_of(&db, "b").as_deref(), Some(store::FAILED));
    assert_eq!(status_of(&db, "a").as_deref(), Some(store::OK), "the next song still analysed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turning_automix_off_stops_a_song_part_way() {
    let (_d, db) = db();
    let files = Arc::new(Files::default());
    song(&db, "a", 20);
    files.hang.lock().push("a".into());
    let an = Analyzer::new(db.clone(), files.clone(), None);
    let on = Arc::new(AtomicBool::new(true));
    let task = {
        let (an, on) = (an.clone(), on.clone());
        tokio::spawn(async move { an.run_round(10, Duration::ZERO, &move || on.load(Ordering::SeqCst)).await })
    };
    while files.reads.lock().is_empty() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    on.store(false, Ordering::SeqCst);
    assert_eq!(task.await.unwrap().unwrap(), Some(Outcome::Stopped));
    assert_eq!(status_of(&db, "a"), None, "nothing kept");
    assert_eq!(db.read(|c| store::pending(c, 10)).unwrap().len(), 1, "done another time");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_busy_computer_stops_a_song_part_way() {
    use crate::lyrics::transcribe::pace::{Pacer, fake};
    let (_d, db) = db();
    let files = Arc::new(Files::default());
    song(&db, "a", 20);
    files.hang.lock().push("a".into());
    let load = fake::FakeLoad::new(5.0);
    let pacer = Pacer::with_cores(load.clone(), fake::timing(), 8);
    let an = Analyzer::new(db.clone(), files.clone(), Some(pacer));
    let task = {
        let an = an.clone();
        tokio::spawn(async move { an.run_round(10, Duration::ZERO, &|| true).await })
    };
    while files.reads.lock().is_empty() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    load.in_front.store(true, Ordering::SeqCst);
    assert_eq!(task.await.unwrap().unwrap(), Some(Outcome::Stopped));
    assert_eq!(status_of(&db, "a"), None);
}

#[test]
fn decoding_and_analysis_give_up_once_told_to_stop() {
    use super::features::{self, Decoded};
    let stop = AtomicBool::new(true);
    let bytes = wav(&synth::clicks(120.0, 20.0, 0.1, &CHORD));
    assert_eq!(features::decode_mono(bytes.clone(), Some("wav"), 60.0, &stop).unwrap(), Decoded::Stopped);
    let go = AtomicBool::new(false);
    let Decoded::Audio(mono) = features::decode_mono(bytes.clone(), Some("wav"), 60.0, &go).unwrap() else { panic!("decoded") };
    assert_eq!(features::analyse_until(&mono, &stop), Err(features::Stopped));
    // Much longer than the library says: not decoded to the end.
    assert_eq!(features::decode_mono(bytes, Some("wav"), 10.0, &go).unwrap(), Decoded::TooLong);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_too_big_to_read_counts_as_too_long() {
    let (_d, db) = db();
    let files = Arc::new(Files::default());
    song(&db, "a", 20);
    files.files.lock().insert("a".into(), Ok(wav(&synth::clicks(120.0, 20.0, 0.1, &CHORD))));
    let an = Analyzer::with_max_bytes(db.clone(), files.clone(), None, 1000);
    an.run_round(10, Duration::ZERO, &|| true).await.unwrap();
    assert_eq!(status_of(&db, "a").as_deref(), Some(store::LONG));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unreadable_songs_are_tried_a_day_later_and_failed_ones_with_a_new_version() {
    let (_d, db) = db();
    let files = Arc::new(Files::default());
    song(&db, "gone", 20);
    files.files.lock().insert("gone".into(), Err(AudioError::Unreadable("404".into())));
    song(&db, "broken", 20);
    files.files.lock().insert("broken".into(), Ok(b"not audio at all".to_vec()));
    let an = Analyzer::new(db.clone(), files.clone(), None);
    an.run_round(10, Duration::ZERO, &|| true).await.unwrap();
    assert_eq!(status_of(&db, "gone").as_deref(), Some(store::UNREAD));
    assert_eq!(status_of(&db, "broken").as_deref(), Some(store::FAILED));
    assert!(db.read(|c| store::pending(c, 10)).unwrap().is_empty(), "neither straight away");
    // A day on, the unreadable one is tried again; the broken one waits.
    db.write(|c| {
        c.execute("UPDATE track_analysis SET analysed_at = analysed_at - ?1", [store::RETRY_UNREAD_S + 1])?;
        Ok(())
    })
    .unwrap();
    let ids: Vec<String> = db.read(|c| store::pending(c, 10)).unwrap().into_iter().map(|t| t.track_id).collect();
    assert_eq!(ids, vec!["gone".to_string()]);
    // An older analysis version: the broken one gets one more try.
    db.write(|c| {
        c.execute("UPDATE track_analysis SET version = version - 1 WHERE track_id = 'broken'", [])?;
        Ok(())
    })
    .unwrap();
    let mut ids: Vec<String> = db.read(|c| store::pending(c, 10)).unwrap().into_iter().map(|t| t.track_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["broken".to_string(), "gone".to_string()]);
    an.run_round(10, Duration::ZERO, &|| true).await.unwrap();
    let ids: Vec<String> = db.read(|c| store::pending(c, 10)).unwrap().into_iter().map(|t| t.track_id).collect();
    assert!(ids.is_empty(), "once per version: {ids:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn songs_saved_before_a_database_error_are_still_announced() {
    let (_d, db) = db();
    let files = Arc::new(Files::default());
    // Newest first: "a" is done before "b".
    for id in ["b", "a"] {
        song(&db, id, 20);
        files.files.lock().insert(id.into(), Ok(wav(&synth::clicks(120.0, 20.0, 0.1, &CHORD))));
    }
    {
        let db2 = db.clone();
        *files.on_read.lock() = Some(Box::new(move |id| {
            if id == "b" {
                db2.write(|c| {
                    c.execute_batch("DROP TABLE track_analysis")?;
                    Ok(())
                })
                .unwrap();
            }
        }));
    }
    let an = Analyzer::new(db.clone(), files.clone(), None);
    let done = Arc::new(Mutex::new(Vec::new()));
    {
        let d = done.clone();
        an.set_on_done(Arc::new(move |ids| d.lock().extend(ids)));
    }
    assert!(an.run_round(10, Duration::ZERO, &|| true).await.is_err());
    assert_eq!(done.lock().as_slice(), ["a".to_string()]);
}

#[test]
fn pruning_leaves_analyses_alone_while_the_library_is_empty() {
    let (_d, db) = db();
    song(&db, "a", 20);
    let t = Todo { track_id: "a".into(), part_key: "/p/a.wav".into(), duration_ms: 20_000 };
    db.write(move |c| store::save_status(c, &t, store::LONG, 1)).unwrap();
    db.write(|c| {
        c.execute("DELETE FROM tracks", [])?;
        Ok(())
    })
    .unwrap();
    assert_eq!(db.write(|c| store::prune(c)).unwrap(), 0);
    assert_eq!(status_of(&db, "a").as_deref(), Some(store::LONG));
    song(&db, "b", 20);
    assert_eq!(db.write(|c| store::prune(c)).unwrap(), 1, "with a library, songs no longer in it go");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rounds_prune_at_most_once_a_day() {
    let (_d, db) = db();
    song(&db, "a", 20);
    song(&db, "keep", 20);
    let an = Analyzer::new(db.clone(), Arc::new(Files::default()), None);
    an.run_round(10, Duration::ZERO, &|| true).await.unwrap();
    db.write(|c| {
        c.execute("DELETE FROM tracks WHERE id = 'a'", [])?;
        Ok(())
    })
    .unwrap();
    an.run_round(10, Duration::ZERO, &|| true).await.unwrap();
    assert!(status_of(&db, "a").is_some(), "not pruned again the same day");
}
