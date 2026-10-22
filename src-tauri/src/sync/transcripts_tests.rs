//! Transcriptions between two devices through a sync file in a temp folder.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tempfile::TempDir;

use super::engine::{self, Groups};
use super::store::SyncStore;
use super::transcripts::{self, Transcript};
use crate::db::DbHandle;
use crate::lyrics::transcribe::store as tstore;
use crate::lyrics::user;
use crate::plex::details::LyricsSource;

const TAG: &str = "0123456789abcdef";
const LRC: &str = "[00:01.00]<00:01.00>Hello <00:01.30>there\n[00:04.00]<00:04.00>Second <00:04.30>line";
const LRC_MEDIUM: &str = "[00:01.00]<00:01.00>Hello <00:01.30>there\n[00:04.00]<00:04.00>Second <00:04.30>time";

struct Dev {
    db: DbHandle,
    store: SyncStore,
    clock: Arc<AtomicU64>,
    _dir: TempDir,
}

impl Dev {
    fn new(id: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("lib.db")).unwrap();
        db.write(|c| {
            c.execute_batch(
                r#"
                INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, track_number)
                  VALUES ('501', 'Opening', '50', 'First Light', '7', 'Test Artist', 200500, '/p/1', 1),
                         ('502', 'Closing', '50', 'First Light', '7', 'Test Artist', 200500, '/p/2', 2);
                "#,
            )?;
            Ok(())
        })
        .unwrap();
        let clock = Arc::new(AtomicU64::new(1_000_000));
        let c = clock.clone();
        let store = SyncStore::with_clock(db.clone(), id, Arc::new(move || c.load(Ordering::SeqCst)));
        Dev { db, store, clock, _dir: dir }
    }

    fn at(&self, ms: u64) {
        self.clock.store(ms, Ordering::SeqCst);
    }

    fn pass(&self, folder: &Path) {
        engine::pass(&self.store, folder, TAG, Groups::default(), self.store.device(), self.store.now()).unwrap();
    }

    /// Transcribed here, as the transcriber saves it.
    fn transcribe(&self, id: &str, body: &str, model: &str, at: i64) {
        assert!(self.db.write(|c| tstore::save(c, id, body, model, at)).unwrap());
        transcripts::saved(&self.store, &self.db, id, body, model, at).unwrap();
    }

    fn apply(&self) -> Vec<(String, bool)> {
        transcripts::apply(&self.store, &self.db).unwrap()
    }
}

/// A writes, then B reads (and writes), then A reads.
fn exchange(a: &Dev, b: &Dev, folder: &Path) {
    a.pass(folder);
    b.pass(folder);
    a.pass(folder);
}

#[test]
fn a_transcription_travels_with_its_model_and_shows_as_transcribed() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (Dev::new("dev-a"), Dev::new("dev-b"));
    a.transcribe("501", LRC, "small", 100);
    exchange(&a, &b, folder.path());
    assert_eq!(b.apply(), vec![("501".to_string(), true)]);
    let own = b.db.read(|c| user::get(c, "501")).unwrap().unwrap();
    assert_eq!(own.auto.as_deref(), Some("small"), "still marked transcribed, with the model");
    assert_eq!(own.text, LRC);
    assert_eq!(own.lyrics().unwrap().source, LyricsSource::Auto);
    // The song counts as done there: the batch won't run the model on it.
    assert!(b.db.read(|c| tstore::done(c, "501")).unwrap());
    assert!(!b.db.read(tstore::unreleased_todo).unwrap().contains(&"501".to_string()));
    assert!(b.apply().is_empty(), "nothing new the second time");
}

#[test]
fn lyrics_the_user_wrote_beat_a_transcription() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (Dev::new("dev-a"), Dev::new("dev-b"));
    b.db.write(|c| user::put(c, "501", "[00:01.00]My own words", false, 50)).unwrap();
    a.transcribe("501", LRC, "medium", 100);
    exchange(&a, &b, folder.path());
    assert!(b.apply().is_empty());
    let own = b.db.read(|c| user::get(c, "501")).unwrap().unwrap();
    assert_eq!((own.text.as_str(), own.auto), ("[00:01.00]My own words", None));
    // And taking it before transcribing never replaces them either.
    assert_eq!(transcripts::take(&b.store, &b.db, "501").unwrap(), Some(true));
    assert!(b.db.read(|c| user::get(c, "501")).unwrap().unwrap().auto.is_none());
}

#[test]
fn the_bigger_model_wins_then_the_newest() {
    let t = |model: &str, at| Transcript { text: "x".into(), model: model.into(), part: "/p/1".into(), at, timing: 2, heard: 0 };
    let (small_new, small_old, medium_old) = (t("small", 300), t("small", 100), t("medium", 50));
    assert_eq!(transcripts::best([&small_new, &medium_old]), Some(&medium_old));
    assert_eq!(transcripts::best([&small_old, &small_new]), Some(&small_new));

    // Two devices transcribe the same song with different models at once.
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (Dev::new("dev-a"), Dev::new("dev-b"));
    a.transcribe("501", LRC, "small", 300);
    b.at(900_000);
    b.transcribe("501", LRC_MEDIUM, "medium", 50);
    exchange(&a, &b, folder.path());
    assert_eq!(a.apply(), vec![("501".to_string(), true)]);
    assert!(b.apply().is_empty(), "medium was already there");
    for d in [&a, &b] {
        let own = d.db.read(|c| user::get(c, "501")).unwrap().unwrap();
        assert_eq!((own.auto.as_deref(), own.text.as_str()), (Some("medium"), LRC_MEDIUM));
    }
}

#[test]
fn turbo_is_the_biggest_model_and_the_voice_beats_the_mix() {
    let t = |model: &str, at, heard| Transcript { text: "x".into(), model: model.into(), part: "/p/1".into(), at, timing: 2, heard };
    // Turbo over the others, however old.
    assert_eq!(transcripts::best([&t("small", 300, 0), &t("turbo", 10, 0)]).unwrap().model, "turbo");
    assert_eq!(transcripts::best([&t("medium", 300, 0), &t("turbo", 10, 0)]).unwrap().model, "turbo");
    // The same model: heard from the voice over the mix, then the newest.
    let (voice, mix_newer) = (t("turbo", 100, tstore::HEARD_VOICE), t("turbo", 900, tstore::HEARD_MIX));
    assert_eq!(transcripts::best([&voice, &mix_newer]), Some(&voice));
    // A result from an older build counts as the old way.
    assert_eq!(t("turbo", 1, 0).heard(), tstore::HEARD_OLD);
}

#[test]
fn a_song_done_again_with_turbo_replaces_a_small_transcription_on_the_other_device() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (Dev::new("dev-a"), Dev::new("dev-b"));
    // b (a phone) has a small model's lyrics, from the mix.
    b.transcribe("501", LRC, "small", 300);
    b.db.write(|c| tstore::set_heard(c, "501", tstore::HEARD_MIX)).unwrap();
    // a (a computer with the voice) does it again with Turbo.
    a.db.write(|c| tstore::set_heard(c, "501", tstore::HEARD_VOICE)).unwrap();
    a.transcribe("501", LRC_MEDIUM, "turbo", 100);
    exchange(&a, &b, folder.path());
    assert_eq!(b.apply(), vec![("501".to_string(), true)]);
    let own = b.db.read(|c| user::get(c, "501")).unwrap().unwrap();
    assert_eq!((own.auto.as_deref(), own.text.as_str()), (Some("turbo"), LRC_MEDIUM));
    // Noted as heard from the voice there, so a computer there doesn't redo it.
    assert_eq!(b.db.read(|c| tstore::heard(c, "501")).unwrap(), Some(tstore::HEARD_VOICE));
    // And forgetting it reaches the other device too.
    transcripts::forgot(&a.store, "501").unwrap();
    a.db.write(|c| tstore::forget(c, "501")).unwrap();
    exchange(&a, &b, folder.path());
    b.apply();
    assert!(b.db.read(|c| user::get(c, "501")).unwrap().is_none(), "Turbo's result was forgotten too");
}

#[test]
fn a_transcription_from_another_device_is_noted_as_heard_there() {
    let b = Dev::new("dev-b");
    let put = |t: Transcript| b.store.put(&super::store::TRANSCRIPTS, &transcripts::key("501", &t.model), &t).unwrap();
    let t = |at, heard| Transcript { text: LRC.into(), model: "turbo".into(), part: "/p/1".into(), at, timing: 2, heard };
    // From an older build (the Bridge before an update): the old way, so a
    // computer that hears the voice does it again.
    put(t(100, 0));
    assert_eq!(b.apply(), vec![("501".to_string(), true)]);
    assert_eq!(b.db.read(|c| tstore::heard(c, "501")).unwrap(), Some(tstore::HEARD_OLD));
    let due = b.db.read(|c| tstore::redo_due(c, tstore::HEARD_VOICE, 10)).unwrap();
    assert_eq!(due, vec!["501"]);
    // The same words said again, now heard from the voice: only the note changes.
    put(t(100, tstore::HEARD_VOICE));
    assert!(b.apply().is_empty());
    assert_eq!(b.db.read(|c| tstore::heard(c, "501")).unwrap(), Some(tstore::HEARD_VOICE));
    assert!(b.db.read(|c| tstore::redo_due(c, tstore::HEARD_VOICE, 10)).unwrap().is_empty());
}

#[test]
fn how_a_song_was_heard_travels_with_it() {
    let b = Dev::new("dev-b");
    b.db.write(|c| tstore::set_heard(c, "501", tstore::HEARD_VOICE)).unwrap();
    b.transcribe("501", LRC, "turbo", 100);
    let sent = b.store.get(&super::store::TRANSCRIPTS, &transcripts::key("501", "turbo")).unwrap().unwrap();
    assert_eq!(sent.heard, tstore::HEARD_VOICE);
    // Older builds read it fine: the field is extra to them, and absent reads as 0.
    let old: Transcript = serde_json::from_str(r#"{"text":"x","model":"small","part":"/p/1","at":1}"#).unwrap();
    assert_eq!(old.heard, 0);
}

#[test]
fn found_nothing_travels_so_the_song_is_not_redone() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (Dev::new("dev-a"), Dev::new("dev-b"));
    a.transcribe("502", "", "small", 100);
    exchange(&a, &b, folder.path());
    assert_eq!(b.apply(), vec![("502".to_string(), false)]);
    assert!(b.db.read(|c| tstore::found_nothing(c, "502")).unwrap());
    assert!(b.db.read(|c| tstore::done(c, "502")).unwrap());
    assert!(b.db.read(|c| user::get(c, "502")).unwrap().is_none());
}

#[test]
fn a_replaced_file_is_transcribed_again() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (Dev::new("dev-a"), Dev::new("dev-b"));
    b.db.write(|c| Ok(c.execute("UPDATE tracks SET part_key = '/p/1-new' WHERE id = '501'", [])?)).unwrap();
    a.transcribe("501", LRC, "medium", 100);
    exchange(&a, &b, folder.path());
    assert!(b.apply().is_empty(), "made from another version of the file");
    assert_eq!(transcripts::take(&b.store, &b.db, "501").unwrap(), None);
    assert!(!b.db.read(|c| tstore::done(c, "501")).unwrap());
}

#[test]
fn taking_a_result_saves_it_here() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (Dev::new("dev-a"), Dev::new("dev-b"));
    a.transcribe("501", LRC, "small", 100);
    exchange(&a, &b, folder.path());
    // Before the listener applied it, the batch reaches the song.
    assert_eq!(transcripts::take(&b.store, &b.db, "501").unwrap(), Some(true));
    assert_eq!(b.db.read(|c| user::get(c, "501")).unwrap().unwrap().auto.as_deref(), Some("small"));
    assert_eq!(transcripts::take(&b.store, &b.db, "502").unwrap(), None);
}

#[test]
fn forgetting_on_one_device_forgets_on_the_others() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (Dev::new("dev-a"), Dev::new("dev-b"));
    a.transcribe("501", LRC, "small", 100);
    exchange(&a, &b, folder.path());
    b.apply();
    a.at(2_000_000);
    a.db.write(|c| tstore::forget(c, "501")).unwrap();
    transcripts::forgot(&a.store, "501").unwrap();
    exchange(&a, &b, folder.path());
    assert_eq!(b.apply(), vec![("501".to_string(), false)]);
    assert!(b.db.read(|c| user::get(c, "501")).unwrap().is_none());
}

#[test]
fn transcriptions_made_before_sync_are_seeded() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (Dev::new("dev-a"), Dev::new("dev-b"));
    a.db.write(|c| tstore::save(c, "501", LRC, "medium", 100)).unwrap();
    a.db.write(|c| tstore::save(c, "502", "", "small", 100)).unwrap();
    transcripts::seed(&a.store, &a.db).unwrap();
    exchange(&a, &b, folder.path());
    let mut got = b.apply();
    got.sort();
    assert_eq!(got, vec![("501".to_string(), true), ("502".to_string(), false)]);
}

#[test]
fn shared_songs_are_remembered_everywhere() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (Dev::new("dev-a"), Dev::new("dev-b"));
    transcripts::shared(&a.store, "501", 100).unwrap();
    exchange(&a, &b, folder.path());
    assert!(transcripts::was_shared(&b.store, "501").unwrap());
    assert!(!transcripts::was_shared(&b.store, "502").unwrap());
}

#[test]
fn odd_results_from_another_device_are_left_out_and_a_future_time_is_clamped() {
    let b = Dev::new("dev-b");
    let put = |id: &str, t: Transcript| b.store.put(&super::store::TRANSCRIPTS, &transcripts::key(id, &t.model), &t).unwrap();
    let t = |text: String, model: &str, at: i64| Transcript { text, model: model.into(), part: "/p/1".into(), at, timing: 2, heard: 0 };
    // A model Platter doesn't have, and lyrics far longer than the editor takes.
    put("501", t(LRC.into(), "huge-v9", 100));
    put("501", t("x".repeat(user::MAX_BYTES + 1), "medium", 100));
    assert!(b.apply().is_empty());
    assert!(b.db.read(|c| user::get(c, "501")).unwrap().is_none());
    // Stamped years ahead: kept, but as made now, so a later result can still win.
    put("501", t(LRC.into(), "small", i64::MAX / 4));
    assert_eq!(b.apply(), vec![("501".to_string(), true)]);
    let own = b.db.read(|c| user::get(c, "501")).unwrap().unwrap();
    assert!(own.updated_at <= crate::db::queries::now_s());
}
