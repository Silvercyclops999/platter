//! Writing tags: every format round trip, audio untouched, and every way a
//! write is refused or fails leaves the original as it was.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::Values;
use super::doc::{Ctx, Doc};
use super::fields::Field;
use super::write::{self, Edits, Hooks, NoHooks, Recent, Refusal, Stage};

pub(crate) const FIXTURES: &[&str] = &["song.flac", "song-v24.mp3", "song-v23.mp3", "song.m4a", "song.ogg"];

pub(crate) fn fixture(dir: &Path, name: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tagedit/fixtures").join(name);
    let dst = dir.join(name);
    std::fs::copy(src, &dst).unwrap();
    // An hour old, so the folder doesn't look busy.
    let old = SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options().write(true).open(&dst).unwrap().set_modified(old).unwrap();
    dst
}

fn ctx() -> Ctx {
    Ctx { custom_key: Some("PROVENANCE".into()), slash_numbers: false }
}

fn read(path: &Path) -> (Values, String) {
    let (_, v, s) = write::read_file(path, &ctx()).unwrap();
    (v, s)
}

fn edits(pairs: &[(Field, &str)]) -> Edits {
    pairs.iter().map(|(f, v)| (*f, v.to_string())).collect()
}

/// The compressed audio packets, as the player's decoder reads them.
fn packets(path: &Path) -> Vec<Vec<u8>> {
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    let f = std::fs::File::open(path).unwrap();
    let mss = MediaSourceStream::new(Box::new(f), Default::default());
    let mut h = Hint::new();
    if let Some(e) = path.extension() {
        h.with_extension(&e.to_string_lossy());
    }
    let mut format = symphonia::default::get_probe()
        .probe(&h, mss, FormatOptions::default(), MetadataOptions::default())
        .unwrap();
    let id = format.default_track(TrackType::Audio).unwrap().id;
    let mut out = Vec::new();
    while let Ok(Some(p)) = format.next_packet() {
        if p.track_id == id {
            out.push(p.data.to_vec());
        }
    }
    assert!(!out.is_empty(), "{} has audio", path.display());
    out
}

/// What Platter's own tag reader (the background scan) makes of a file.
fn scanned(path: &Path) -> crate::tags::FileTags {
    let bytes = std::fs::read(path).unwrap();
    let n = bytes.len() as u64;
    let mut s = crate::tags::parse::Sparse::new(bytes, Some(n));
    s.container = path.extension().map(|e| e.to_string_lossy().to_string());
    match crate::tags::parse::parse(&s, true) {
        crate::tags::parse::Step::Done(t) => t,
        other => panic!("{other:?}"),
    }
}

fn full_edit() -> Edits {
    edits(&[
        (Field::AlbumArtist, "Demo Artist"),
        (Field::Album, "Demo Album (Deluxe)"),
        (Field::Year, "2019"),
        (Field::Genre, "Hip Hop"),
        (Field::ReleaseType, "album;demo"),
        (Field::Edition, "Deluxe"),
        (Field::Group, "demo-album"),
        (Field::Era, "Demo Era"),
        (Field::Quality, "lossless"),
        (Field::Explicit, "explicit"),
        (Field::TrackNumber, "3"),
        (Field::TrackTotal, "12"),
        (Field::DiscNumber, "1"),
        (Field::DiscTotal, "2"),
        (Field::Title, "Démo Song (feat. Other Artist)"),
        (Field::Artist, "Demo Artist"),
        (Field::ArchiveVersion, "leak"),
        (Field::Status, "Leaked"),
        (Field::PicardStatus, "bootleg"),
        (Field::CustomStatus, "Vault copy"),
        (Field::VersionNote, "OG file"),
        (Field::ArchiveDate, "2018-05"),
        (Field::Source, "Group buy"),
        (Field::Flags, "fanedit"),
        (Field::Isrc, "USRC17607839"),
    ])
}

#[test]
fn every_format_round_trips_every_field_and_the_scan_reads_them() {
    for name in FIXTURES {
        let d = tempfile::tempdir().unwrap();
        let p = fixture(d.path(), name);
        let (_, s) = read(&p);
        let e = full_edit();
        let w = write::write_file(&p, &s, &e, &ctx(), &NoHooks).unwrap().expect("something changed");
        let (after, stamp) = read(&p);
        for (f, v) in &e {
            assert_eq!(after.get(f), Some(v), "{name}: {f:?}");
        }
        assert_eq!(w.stamp, stamp, "{name}: the stamp after the write is the file's");
        // The library's own reader sees the same thing.
        let t = scanned(&p);
        assert_eq!(t.version.as_deref(), Some("leak"), "{name}");
        assert_eq!(t.era.as_deref(), Some("Demo Era"), "{name}");
        assert_eq!(t.edition.as_deref(), Some("Deluxe"), "{name}");
        assert_eq!(t.group.as_deref(), Some("demo-album"), "{name}");
        assert_eq!(t.isrc.as_deref(), Some("USRC17607839"), "{name}");
        assert_eq!(t.explicit, Some(true), "{name}");
        assert_eq!(t.album_artist.as_deref(), Some("Demo Artist"), "{name}");
        assert_eq!(t.album.as_deref(), Some("Demo Album (Deluxe)"), "{name}");
        assert_eq!(t.year, Some(2019), "{name}");
        assert_eq!(t.status_tag.as_deref(), Some("Leaked"), "{name}");
        assert_eq!(t.version_note.as_deref(), Some("OG file"), "{name}");
        assert_eq!(t.mb_status.as_deref(), Some("bootleg"), "{name}");
        assert_eq!(t.mb_type.as_deref(), Some("album;demo"), "{name}");
        assert_eq!(t.flags.as_deref(), Some("fanedit"), "{name}");
        // Removing a field takes it out.
        let rm = edits(&[(Field::Era, ""), (Field::Explicit, "")]);
        write::write_file(&p, &stamp, &rm, &ctx(), &NoHooks).unwrap().unwrap();
        let (after, _) = read(&p);
        assert!(!after.contains_key(&Field::Era) && !after.contains_key(&Field::Explicit), "{name}: {after:?}");
        assert_eq!(after.get(&Field::Edition).map(String::as_str), Some("Deluxe"), "{name}: the rest stays");
    }
}

#[test]
fn audio_is_never_touched() {
    for name in FIXTURES {
        let d = tempfile::tempdir().unwrap();
        let p = fixture(d.path(), name);
        let before = packets(&p);
        let (_, s) = read(&p);
        write::write_file(&p, &s, &full_edit(), &ctx(), &NoHooks).unwrap().unwrap();
        assert_eq!(packets(&p), before, "{name}: every audio packet is byte for byte the same");
    }
}

fn id3_major(p: &Path) -> u8 {
    let b = std::fs::read(p).unwrap();
    assert_eq!(&b[..3], b"ID3");
    b[3]
}

#[test]
fn id3_keeps_the_version_the_file_has() {
    let d = tempfile::tempdir().unwrap();
    let v23 = fixture(d.path(), "song-v23.mp3");
    let v24 = fixture(d.path(), "song-v24.mp3");
    assert_eq!((id3_major(&v23), id3_major(&v24)), (3, 4));
    for p in [&v23, &v24] {
        let (_, s) = read(p);
        write::write_file(p, &s, &full_edit(), &ctx(), &NoHooks).unwrap().unwrap();
    }
    assert_eq!(id3_major(&v23), 3, "v2.3 stays v2.3");
    assert_eq!(id3_major(&v24), 4);
    assert_eq!(Doc::read(&v23).unwrap().id3_version(), Some("2.3"));
    // Non-Latin text survives v2.3 (written as UTF-16).
    assert_eq!(read(&v23).0.get(&Field::Title).map(String::as_str), Some("Démo Song (feat. Other Artist)"));
    assert_eq!(scanned(&v23).year, Some(2019), "the year is still readable from TYER");
}

#[test]
fn mp4_writes_freeform_itunes_items() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path(), "song.m4a");
    let (_, s) = read(&p);
    write::write_file(&p, &s, &edits(&[(Field::Isrc, "USRC17607839"), (Field::ArchiveVersion, "demo"), (Field::Status, "Unreleased")]), &ctx(), &NoHooks)
        .unwrap()
        .unwrap();
    let b = std::fs::read(&p).unwrap();
    let has = |n: &[u8]| b.windows(n.len()).any(|w| w == n);
    assert!(has(b"----") && has(b"com.apple.iTunes"), "freeform atoms");
    for n in [&b"ISRC"[..], b"ARCHIVE_VERSION", b"STATUS"] {
        assert!(has(n), "{}", String::from_utf8_lossy(n));
    }
    let t = scanned(&p);
    assert_eq!((t.isrc.as_deref(), t.version.as_deref(), t.status_tag.as_deref()), (Some("USRC17607839"), Some("demo"), Some("Unreleased")));
}

/// Every tag name in the file, as the check sees them.
fn keys(p: &Path) -> Vec<String> {
    let mut k = Doc::read(p).unwrap().keys();
    k.sort();
    k
}

#[test]
fn only_the_edited_fields_change_and_nothing_new_is_added() {
    for name in FIXTURES {
        let d = tempfile::tempdir().unwrap();
        let p = fixture(d.path(), name);
        let (before, s) = read(&p);
        let keys_before = keys(&p);
        write::write_file(&p, &s, &edits(&[(Field::Title, "Other Song")]), &ctx(), &NoHooks).unwrap().unwrap();
        let (after, _) = read(&p);
        let mut want = before.clone();
        want.insert(Field::Title, "Other Song".into());
        assert_eq!(after, want, "{name}");
        assert_eq!(keys(&p), keys_before, "{name}: the same tags, no extras");
    }
}

#[test]
fn a_value_already_there_writes_nothing() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path(), "song.flac");
    let bytes = std::fs::read(&p).unwrap();
    let (_, s) = read(&p);
    let r = write::write_file(&p, &s, &edits(&[(Field::Title, "Demo Song")]), &ctx(), &NoHooks).unwrap();
    assert!(r.is_none());
    assert_eq!(std::fs::read(&p).unwrap(), bytes);
}

#[test]
fn a_file_changed_since_the_editor_opened_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path(), "song.flac");
    let (_, s) = read(&p);
    // Another tagger changes it.
    let mut other = Doc::read(&p).unwrap();
    other.set(Field::Title, "Changed Elsewhere", &ctx()).unwrap();
    other.save(&p).unwrap();
    let bytes = std::fs::read(&p).unwrap();
    let r = write::write_file(&p, &s, &edits(&[(Field::Album, "Mine")]), &ctx(), &NoHooks).unwrap_err();
    assert_eq!(r.why, Refusal::Changed);
    assert!(r.message.contains("changed since the editor opened"), "{}", r.message);
    assert_eq!(std::fs::read(&p).unwrap(), bytes, "left as the other program wrote it");
}

struct FailAt(Stage, fn() -> io::Error);
impl Hooks for FailAt {
    fn at(&self, stage: Stage, _temp: &Path) -> io::Result<()> {
        if stage == self.0 { Err((self.1)()) } else { Ok(()) }
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    v.sort();
    v
}

#[test]
fn a_failure_at_any_step_keeps_the_original_and_removes_the_copy() {
    type Case = (Stage, fn() -> io::Error, Refusal);
    let cases: [Case; 5] = [
        (Stage::Copy, || io::Error::from(io::ErrorKind::PermissionDenied), Refusal::ReadOnly),
        (Stage::Copy, || io::Error::from(io::ErrorKind::StorageFull), Refusal::Failed),
        (Stage::Tags, || io::Error::other("disk went away"), Refusal::Failed),
        (Stage::Replace, || io::Error::from_raw_os_error(32), Refusal::Locked),
        (Stage::Replace, || io::Error::from(io::ErrorKind::NotFound), Refusal::Gone),
    ];
    for (stage, err, why) in cases {
        let d = tempfile::tempdir().unwrap();
        let p = fixture(d.path(), "song.flac");
        let bytes = std::fs::read(&p).unwrap();
        let listing = names(d.path());
        let (_, s) = read(&p);
        let r = write::write_file(&p, &s, &full_edit(), &ctx(), &FailAt(stage, err)).unwrap_err();
        assert_eq!(r.why, why, "{stage:?}: {}", r.message);
        assert_eq!(std::fs::read(&p).unwrap(), bytes, "{stage:?}: original untouched");
        assert_eq!(names(d.path()), listing, "{stage:?}: no temp file left");
    }
}

#[test]
fn a_file_gone_from_the_music_folder_says_so() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path(), "song.flac");
    let (_, s) = read(&p);
    std::fs::remove_file(&p).unwrap();
    let r = write::write_file(&p, &s, &full_edit(), &ctx(), &NoHooks).unwrap_err();
    assert_eq!(r.why, Refusal::Gone);
    assert!(r.message.contains("isn't in the music folder"), "{}", r.message);
}

#[test]
fn vorbis_files_keep_their_own_number_and_explicit_tags() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path(), "song.flac");
    let mut doc = Doc::read(&p).unwrap();
    let raw = Ctx::default();
    // "3/12" in TRACKNUMBER, TOTALDISCS beside DISCNUMBER, and EXPLICIT=0.
    doc.set(Field::TrackNumber, "3", &raw).unwrap();
    doc.save(&p).unwrap();
    let mut doc = Doc::read(&p).unwrap();
    doc.set(Field::TrackNumber, "", &raw).unwrap();
    doc.save(&p).unwrap();
    let vc_set = |k: &str, v: &str| {
        let mut doc = Doc::read(&p).unwrap();
        let _ = &mut doc;
        // Write raw keys through a custom-tag context.
        doc.set(Field::CustomStatus, v, &Ctx { custom_key: Some(k.into()), slash_numbers: false }).unwrap();
        doc.save(&p).unwrap();
    };
    vc_set("TRACKNUMBER", "3/12");
    vc_set("DISCNUMBER", "1");
    vc_set("TOTALDISCS", "2");
    vc_set("EXPLICIT", "0");
    let (v, s) = read(&p);
    assert_eq!(v.get(&Field::TrackNumber).map(String::as_str), Some("3"));
    assert_eq!(v.get(&Field::TrackTotal).map(String::as_str), Some("12"));
    assert_eq!(v.get(&Field::DiscTotal).map(String::as_str), Some("2"));
    assert_eq!(v.get(&Field::Explicit).map(String::as_str), Some("clean"));
    write::write_file(&p, &s, &edits(&[(Field::TrackTotal, "13"), (Field::DiscTotal, "3"), (Field::Explicit, "explicit")]), &ctx(), &NoHooks)
        .unwrap()
        .unwrap();
    let k = keys(&p);
    assert!(k.contains(&"TOTALDISCS".to_string()) && !k.contains(&"DISCTOTAL".to_string()), "{k:?}");
    assert!(!k.contains(&"TRACKTOTAL".to_string()) && !k.contains(&"ITUNESADVISORY".to_string()), "{k:?}");
    let t = scanned(&p);
    assert_eq!(t.explicit, Some(true), "EXPLICIT=1 now");
    let (v, _) = read(&p);
    assert_eq!(v.get(&Field::TrackTotal).map(String::as_str), Some("13"));
}

#[test]
fn a_folder_another_program_just_changed_is_busy() {
    let d = tempfile::tempdir().unwrap();
    let p = fixture(d.path(), "song.flac");
    let now = SystemTime::now();
    let recent = Recent::default();
    assert!(!write::folder_busy(d.path(), now, &recent), "files an hour old");
    // Another program writes cover.jpg.
    std::fs::write(d.path().join("cover.jpg"), b"x").unwrap();
    assert!(write::folder_busy(d.path(), SystemTime::now(), &recent));
    std::fs::remove_file(d.path().join("cover.jpg")).unwrap();
    // Platter's own write doesn't count.
    std::fs::File::options().write(true).open(&p).unwrap().set_modified(SystemTime::now()).unwrap();
    assert!(write::folder_busy(d.path(), SystemTime::now(), &recent));
    recent.note(&p);
    assert!(!write::folder_busy(d.path(), SystemTime::now(), &recent));
    // A server clock far ahead isn't "busy".
    let far = SystemTime::now() - Duration::from_secs(600);
    assert!(!write::folder_busy(d.path(), far, &Recent::default()));
}

