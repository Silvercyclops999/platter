//! The tag editor end to end over a music folder of fixture files.

use std::collections::BTreeMap;

use super::editor::{Options, SaveFile, SaveRequest, spread};
use super::fields::{Field, Scheme};
use super::testkit::{Lib, age, values};
use super::write::Refusal;
use crate::tags::provenance::{self, Config};

const ALBUM: &str = "Demo Artist/Demo Album";

fn lib2() -> Lib {
    let lib = Lib::new();
    lib.add("t1", "al1", "Demo Album", &format!("{ALBUM}/01 Demo Song.flac"), "song.flac", 1, 1, "Demo Song");
    lib.add("t2", "al1", "Demo Album", &format!("{ALBUM}/02 Other Song.flac"), "song.flac", 1, 2, "Other Song");
    lib
}

fn edits(pairs: &[(Field, &str)]) -> BTreeMap<Field, String> {
    pairs.iter().map(|(f, v)| (*f, v.to_string())).collect()
}

#[test]
fn without_a_music_folder_details_are_view_only() {
    let lib = lib2();
    lib.editor.set_options(Options::default());
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    assert!(!d.writable && d.needs_music_folder);
    assert!(d.reason.as_deref().unwrap().contains("Set up the music folder"));
    assert_eq!(d.files.len(), 2);
    assert!(d.files.iter().all(|f| f.stamp.is_none()));
    assert_eq!(d.files[1].values.get(&Field::Title).map(String::as_str), Some("Other Song"), "from the library cache");
    let r = tokio_test_save(&lib, SaveRequest { label: "x".into(), files: vec![SaveFile { track_id: "t1".into(), stamp: "s".into(), edits: edits(&[(Field::Genre, "Pop")]) }] });
    assert!(r.unwrap_err().to_string().contains("Set up the music folder"));
}

fn tokio_test_save(lib: &Lib, req: SaveRequest) -> crate::error::AppResult<super::editor::SaveResult> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(lib.editor.save(req))
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

#[test]
fn an_album_saves_only_what_changed_then_plex_rescans_its_folder() {
    let lib = lib2();
    // Mixed album names: one file carries the deluxe name.
    lib.retag(&format!("{ALBUM}/02 Other Song.flac"), &[(Field::Album, "Demo Album (Deluxe)")]);
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    assert!(d.writable && d.reason.is_none(), "{:?}", d.reason);
    let vals: Vec<_> = d.files.iter().map(|f| &f.values).collect();
    assert_eq!(super::editor::shared(&vals, Field::Album), None, "(several)");
    assert_eq!(super::editor::shared(&vals, Field::AlbumArtist).as_deref(), Some("Demo Artist"));
    // Only the genre is edited: the mixed album names stay as they are.
    let files = spread(&d.files, &edits(&[(Field::Genre, "Hip Hop")]));
    assert_eq!(files.len(), 2);
    let r = rt().block_on(lib.editor.save(SaveRequest { label: "Edited tags on Demo Album".into(), files })).unwrap();
    assert_eq!(r.message, "Saved 2 files");
    assert!(r.undo_id.is_some());
    let a = values(&lib.local(&format!("{ALBUM}/01 Demo Song.flac")));
    let b = values(&lib.local(&format!("{ALBUM}/02 Other Song.flac")));
    assert_eq!((a.get(&Field::Genre), b.get(&Field::Genre)), (Some(&"Hip Hop".to_string()), Some(&"Hip Hop".to_string())));
    assert_eq!(a.get(&Field::Album).map(String::as_str), Some("Demo Album"));
    assert_eq!(b.get(&Field::Album).map(String::as_str), Some("Demo Album (Deluxe)"), "mixed values untouched");
    // One rescan, of that folder only, in the library's section.
    let calls = lib.plex.refreshed.lock().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].1.as_str(), calls[0].2.as_str()), ("7", "/srv/music/Demo Artist/Demo Album"));
    assert_eq!(r.rescanned, vec!["/srv/music/Demo Artist/Demo Album".to_string()]);
    // The tag cache has the new values.
    let cached = lib.db.read(|c| crate::tags::store::get(c, "/library/parts/t1/1/file")).unwrap().unwrap().1;
    assert_eq!(cached.album.as_deref(), Some("Demo Album"));
    assert_eq!(cached.version.as_deref(), Some("leak"));
}

#[test]
fn rescan_off_or_plex_down_still_saves() {
    let lib = lib2();
    lib.editor.set_options(Options { mapping: Some(lib.mapping()), rescan: false });
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    let r = rt().block_on(lib.editor.save(SaveRequest { label: "x".into(), files: spread(&d.files, &edits(&[(Field::Era, "Demo Era")])) })).unwrap();
    assert_eq!(r.saved, 2);
    assert!(lib.plex.refreshed.lock().is_empty());
    lib.editor.set_options(Options { mapping: Some(lib.mapping()), rescan: true });
    *lib.plex.down.lock() = true;
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    let r = rt().block_on(lib.editor.save(SaveRequest { label: "x".into(), files: spread(&d.files, &edits(&[(Field::Era, "Other Era")])) })).unwrap();
    assert_eq!(r.saved, 2);
    assert!(r.rescan_error.as_deref().unwrap().contains("next scan"));
}

#[test]
fn one_changed_file_means_nothing_is_written() {
    let lib = lib2();
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    lib.retag(&format!("{ALBUM}/02 Other Song.flac"), &[(Field::Title, "Retagged Elsewhere")]);
    let before = values(&lib.local(&format!("{ALBUM}/01 Demo Song.flac")));
    let r = rt().block_on(lib.editor.save(SaveRequest { label: "x".into(), files: spread(&d.files, &edits(&[(Field::Genre, "Pop")])) })).unwrap();
    assert_eq!(r.saved, 0);
    assert_eq!(r.problems.len(), 1);
    assert_eq!(r.problems[0].why, Refusal::Changed);
    assert!(r.message.starts_with("Nothing was saved."), "{}", r.message);
    assert_eq!(values(&lib.local(&format!("{ALBUM}/01 Demo Song.flac"))), before, "the unchanged file wasn't written either");
    assert!(lib.editor.history().is_empty());
}

#[test]
fn a_folder_another_program_is_changing_is_left_alone() {
    let lib = lib2();
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    std::fs::write(lib.local(&format!("{ALBUM}/cover.jpg")), b"jpeg").unwrap();
    let r = rt().block_on(lib.editor.save(SaveRequest { label: "x".into(), files: spread(&d.files, &edits(&[(Field::Genre, "Pop")])) })).unwrap();
    assert_eq!(r.saved, 0);
    assert_eq!(r.message, "Another program is changing this folder. Try again shortly.");
    age(&lib.local(&format!("{ALBUM}/cover.jpg")));
    let r = rt().block_on(lib.editor.save(SaveRequest { label: "x".into(), files: spread(&d.files, &edits(&[(Field::Genre, "Pop")])) })).unwrap();
    assert_eq!(r.saved, 2, "a minute later it saves");
}

#[test]
fn a_bad_value_is_refused_before_anything_is_written() {
    let lib = lib2();
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    let mut files = spread(&d.files, &edits(&[(Field::Genre, "Pop")]));
    files[1].edits.insert(Field::Isrc, "not an isrc".into());
    let e = rt().block_on(lib.editor.save(SaveRequest { label: "x".into(), files })).unwrap_err();
    assert!(e.to_string().contains("isn't an ISRC"), "{e}");
    assert!(!values(&lib.local(&format!("{ALBUM}/01 Demo Song.flac"))).contains_key(&Field::Genre));
}

#[test]
fn undo_puts_back_what_the_edit_replaced_on_files_unchanged_since() {
    let lib = lib2();
    let p1 = lib.local(&format!("{ALBUM}/01 Demo Song.flac"));
    let p2 = lib.local(&format!("{ALBUM}/02 Other Song.flac"));
    let (b1, b2) = (values(&p1), values(&p2));
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    let r = rt()
        .block_on(lib.editor.save(SaveRequest { label: "Edited tags on Demo Album".into(), files: spread(&d.files, &edits(&[(Field::Genre, "Pop"), (Field::Album, "Renamed")])) }))
        .unwrap();
    let id = r.undo_id.unwrap();
    assert_eq!(lib.editor.history()[0].label, "Edited tags on Demo Album");
    // Another program changes the second file afterwards.
    lib.retag(&format!("{ALBUM}/02 Other Song.flac"), &[(Field::Title, "Changed After")]);
    let u = rt().block_on(lib.editor.undo(&id)).unwrap();
    assert_eq!(u.saved, 1);
    assert_eq!(u.problems.len(), 1);
    assert!(u.problems[0].message.contains("kept as it is"), "{}", u.problems[0].message);
    assert_eq!(values(&p1), b1, "the first file is back as it was");
    let v2 = values(&p2);
    assert_eq!(v2.get(&Field::Album).map(String::as_str), Some("Renamed"), "the changed file kept the edit");
    assert_ne!(v2, b2);
    assert!(lib.editor.history().is_empty(), "an edit is undone once");
    assert!(rt().block_on(lib.editor.undo(&id)).is_err());
}

#[test]
fn the_undo_log_keeps_the_last_twenty_edits_across_restarts() {
    use super::undo::{UndoEntry, UndoLog};
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("tag-edits.json");
    let mut log = UndoLog::open(Some(path.clone()));
    for i in 0..25 {
        log.push(UndoEntry { id: format!("e{i}"), at: i, label: format!("Edit {i}"), custom_key: None, files: Vec::new() });
    }
    let again = UndoLog::open(Some(path));
    assert_eq!(again.entries.len(), 20);
    assert_eq!(again.entries[0].id, "e24", "newest first");
    assert_eq!(again.entries[19].id, "e5");
}

#[test]
fn the_release_status_follows_the_scheme_the_album_uses() {
    // ARCHIVE_VERSION (the FLAC fixture carries one).
    let lib = lib2();
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    assert_eq!((d.scheme, d.status_field), (Scheme::Archive, Field::ArchiveVersion));
    // A plain STATUS tag (the Ogg fixture).
    let lib = Lib::new();
    lib.add("o1", "al2", "Demo Album", "Demo Artist/Ogg/01.ogg", "song.ogg", 1, 1, "Demo Song");
    let d = lib.editor.open(Some("al2"), &[]).unwrap();
    assert_eq!(d.scheme, Scheme::Plain);
    assert_eq!(d.status_choices[0], "Released");
    // Picard's release status.
    let lib = Lib::new();
    lib.add("m1", "al3", "Demo Album", "Demo Artist/Mp3/01.mp3", "song-v24.mp3", 1, 1, "Demo Song");
    lib.retag("Demo Artist/Mp3/01.mp3", &[(Field::PicardStatus, "official")]);
    let d = lib.editor.open(Some("al3"), &[]).unwrap();
    assert_eq!((d.scheme, d.status_field), (Scheme::Picard, Field::PicardStatus));
    // Nothing yet: the Settings choice, Platter's own tag on Auto ...
    let lib = Lib::new();
    lib.add("m2", "al4", "Demo Album", "Demo Artist/M4a/01.m4a", "song.m4a", 1, 1, "Demo Song");
    assert_eq!(lib.editor.open(Some("al4"), &[]).unwrap().scheme, Scheme::Archive);
    // ... or the custom tag when one is set.
    provenance::set_config(Config::new("PROVENANCE", &[]));
    let d = lib.editor.open(Some("al4"), &[]).unwrap();
    assert_eq!((d.scheme, d.custom_key.as_deref()), (Scheme::Custom, Some("PROVENANCE")));
    let files = spread(&d.files, &edits(&[(Field::CustomStatus, "Vault copy")]));
    rt().block_on(lib.editor.save(SaveRequest { label: "x".into(), files })).unwrap();
    assert_eq!(values(&lib.local("Demo Artist/M4a/01.m4a")).get(&Field::CustomStatus).map(String::as_str), Some("Vault copy"));
    provenance::set_config(Config::default());
}

#[test]
fn a_song_outside_the_music_folder_or_gone_says_why() {
    let lib = lib2();
    std::fs::remove_file(lib.local(&format!("{ALBUM}/02 Other Song.flac"))).unwrap();
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    assert!(d.writable);
    assert!(d.files[1].error.as_deref().unwrap().contains("isn't in the music folder"));
    assert_eq!(d.reason.as_deref(), Some("1 file couldn't be read and won't be changed."));
    // The music folder itself gone (the share disconnected).
    let lib = lib2();
    lib.editor.set_options(Options { mapping: Some(super::paths::Mapping { server: "/srv/music".into(), local: "/nowhere/at/all".into() }), rescan: true });
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    assert!(!d.writable);
    assert!(d.reason.as_deref().unwrap().contains("Check that the share is connected"), "{:?}", d.reason);
}

#[test]
fn the_ui_requests_read_as_sent() {
    use super::editor::CheckScope;
    let s: CheckScope = serde_json::from_value(serde_json::json!({ "kind": "library" })).unwrap();
    assert_eq!(s, CheckScope::Library);
    let s: CheckScope = serde_json::from_value(serde_json::json!({ "kind": "album", "id": "al1" })).unwrap();
    assert_eq!(s, CheckScope::Album("al1".into()));
    let r: SaveRequest = serde_json::from_value(serde_json::json!({
        "label": "Edited tags", "files": [{ "trackId": "t1", "stamp": "s", "edits": { "genre": "Pop", "isrc": "" } }]
    }))
    .unwrap();
    assert_eq!(r.files[0].edits, edits(&[(Field::Genre, "Pop"), (Field::Isrc, "")]));
    let k: super::fixes::FixKind = serde_json::from_value(serde_json::json!("discTags")).unwrap();
    assert_eq!(k, super::fixes::FixKind::DiscTags);
}
