//! Library doctor tests: each check finds what it should and stays quiet
//! otherwise (unreleased music, editions, builds), the incremental check,
//! the stored list with covers and ignores, cover sizing over HTTP (and a
//! server that fails), and the settings.

use std::collections::HashMap;
use std::time::Duration;

use rusqlite::{Connection, params};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::checks::{self, ArtistData};
use super::{Checks, Finding, Fix, Ignored, Kind, Options, commands, covers, store};
use crate::audio::StreamSource;
use crate::audio::gate::TransferGate;
use crate::db::{DbHandle, schema};
use crate::error::{AppError, AppResult};
use crate::tags::fetch::Reader;
use crate::tags::scan::PartSource;

// ------------------------------------------------------------------ helpers

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
    schema::migrate(&mut c).unwrap();
    (dir, c)
}

/// An album Plex matched, by `artist`.
fn album(c: &Connection, id: &str, artist: &str, title: &str, year: Option<i32>) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, year, thumb, matched, added_at)
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, 1, 1)",
        params![id, title, format!("ar:{artist}"), artist, year, format!("/library/metadata/{id}/thumb/1")],
    )
    .unwrap();
}

/// A song. `codec`/`kbps` for its quality.
#[allow(clippy::too_many_arguments)]
fn song(c: &Connection, id: &str, album_id: &str, artist: &str, title: &str, no: i64, ms: i64, codec: &str, kbps: u32) {
    let album_title: String = c.query_row("SELECT title FROM albums WHERE id = ?1", [album_id], |r| r.get(0)).unwrap();
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, duration_ms, part_key, codec, bitrate)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![id, title, album_id, album_title, format!("ar:{artist}"), artist, no, ms, format!("/p/{id}"), codec, kbps],
    )
    .unwrap();
}

fn credit(c: &Connection, id: &str, who: &str) {
    c.execute("UPDATE tracks SET track_artist = ?2 WHERE id = ?1", params![id, who]).unwrap();
}

/// The file's album tags, read by this build.
fn file(c: &Connection, id: &str, album: Option<&str>, album_artist: Option<&str>, year: Option<i32>) {
    c.execute(
        "INSERT OR REPLACE INTO file_tags (part_key, status, album, album_artist, year, scanned_at, ver) VALUES (?1, 'ok', ?2, ?3, ?4, 1, ?5)",
        params![format!("/p/{id}"), album, album_artist, year, crate::tags::CACHE_VERSION],
    )
    .unwrap();
}

fn check(c: &Connection, artist: &str) -> Vec<Finding> {
    let d = checks::load(c, &format!("ar:{artist}")).unwrap();
    checks::run_checks(&format!("ar:{artist}"), &d)
}

fn of(found: &[Finding], k: Kind) -> Vec<&Finding> {
    found.iter().filter(|f| f.kind == k).collect()
}

// ------------------------------------------------------------------ duplicates

#[test]
fn the_same_recording_twice_on_an_album_is_a_duplicate() {
    let (_d, c) = db();
    album(&c, "a1", "Ana", "Night Drive", Some(2016));
    song(&c, "t1", "a1", "Ana", "Low Light", 1, 200_000, "flac", 900);
    song(&c, "t2", "a1", "Ana", "Low Light", 9, 200_800, "mp3", 320);
    song(&c, "t3", "a1", "Ana", "Other Song", 2, 180_000, "flac", 900);
    // Two "Intro"s in different places are two songs.
    song(&c, "t4", "a1", "Ana", "Intro", 3, 60_000, "flac", 900);
    song(&c, "t5", "a1", "Ana", "Intro", 8, 60_500, "flac", 900);
    let found = check(&c, "Ana");
    let dupes = of(&found, Kind::Duplicate);
    assert_eq!(dupes.len(), 1, "{found:#?}");
    let f = dupes[0];
    assert_eq!(f.id, "duplicate:a1");
    assert_eq!(f.fix, Fix::OpenAlbum);
    assert_eq!(f.tracks, ["t1", "t2"]);
    assert!(f.title.contains("Low Light") && f.title.contains("twice"), "{}", f.title);
    assert!(f.evidence[0].contains("FLAC") && f.evidence[0].contains("MP3 320 kbps"), "{:?}", f.evidence);
}

#[test]
fn unreleased_songs_are_never_called_duplicates() {
    let (_d, c) = db();
    album(&c, "a1", "Ana", "Vault", None);
    c.execute("UPDATE albums SET subformats = '[\"Demo\"]' WHERE id = 'a1'", []).unwrap();
    song(&c, "t1", "a1", "Ana", "Low Light", 1, 200_000, "mp3", 128);
    song(&c, "t2", "a1", "Ana", "Low Light", 2, 200_100, "mp3", 128);
    assert!(of(&check(&c, "Ana"), Kind::Duplicate).is_empty());
}

// ------------------------------------------------------------------ album tags

#[test]
fn album_tags_that_differ_between_files_are_found() {
    let (_d, c) = db();
    album(&c, "a1", "Ana", "Night Drive", Some(2016));
    for (i, id) in ["t1", "t2", "t3"].iter().enumerate() {
        song(&c, id, "a1", "Ana", &format!("Song {i}"), i as i64 + 1, 200_000, "flac", 900);
    }
    file(&c, "t1", Some("Night Drive"), Some("Ana"), Some(2016));
    file(&c, "t2", Some("Night Drive"), Some("Ana"), Some(2016));
    file(&c, "t3", Some("Night Drive (Deluxe)"), Some("Ana"), Some(2017));
    let found = check(&c, "Ana");
    let f = of(&found, Kind::AlbumTags);
    assert_eq!(f.len(), 1, "{found:#?}");
    let f = f[0];
    assert_eq!(f.fix, Fix::Tags);
    assert!(f.title.contains("album name and year"), "{}", f.title);
    assert!(f.evidence[0].contains("\u{201c}Night Drive\u{201d} on 2 songs"), "{:?}", f.evidence);
    assert!(f.evidence.iter().any(|e| e.contains("2017")));
    assert_eq!(f.table.len(), 3);
    assert_eq!(f.table[2].album.as_deref(), Some("Night Drive (Deluxe)"));
}

#[test]
fn matching_tags_case_and_spacing_aside_and_old_reads_say_nothing() {
    let (_d, c) = db();
    album(&c, "a1", "Ana", "Night Drive", Some(2016));
    song(&c, "t1", "a1", "Ana", "One", 1, 200_000, "flac", 900);
    song(&c, "t2", "a1", "Ana", "Two", 2, 200_000, "flac", 900);
    file(&c, "t1", Some("Night  Drive"), Some("Ana"), Some(2016));
    file(&c, "t2", Some("night drive"), Some("ana"), Some(2016));
    assert!(of(&check(&c, "Ana"), Kind::AlbumTags).is_empty());
    // A file read by an older build has no album tags yet: it isn't counted.
    c.execute("UPDATE file_tags SET album = 'Else', ver = 1 WHERE part_key = '/p/t2'", []).unwrap();
    assert!(of(&check(&c, "Ana"), Kind::AlbumTags).is_empty());
}

// ------------------------------------------------------------------ album artist

#[test]
fn a_missing_album_artist_on_a_multi_artist_album_is_found() {
    let (_d, c) = db();
    album(&c, "a1", "Ana", "Friends", Some(2020));
    song(&c, "t1", "a1", "Ana", "One", 1, 200_000, "flac", 900);
    song(&c, "t2", "a1", "Ana", "Two", 2, 200_000, "flac", 900);
    credit(&c, "t2", "Bo");
    file(&c, "t1", Some("Friends"), None, None);
    file(&c, "t2", Some("Friends"), None, None);
    let found = check(&c, "Ana");
    let f = of(&found, Kind::AlbumArtist);
    assert_eq!(f.len(), 1, "{found:#?}");
    assert_eq!(f[0].title, "No album artist on any song");
    assert!(f[0].evidence.iter().any(|e| e.contains("credit Ana and Bo")), "{:?}", f[0].evidence);
    // With the album artist set on every file, nothing to say.
    file(&c, "t1", Some("Friends"), Some("Ana"), None);
    file(&c, "t2", Some("Friends"), Some("Ana"), None);
    assert!(of(&check(&c, "Ana"), Kind::AlbumArtist).is_empty());
}

#[test]
fn one_artist_without_an_album_artist_is_fine_unless_the_album_is_split_across_artists() {
    let (_d, c) = db();
    album(&c, "a1", "Ana", "Solo", None);
    song(&c, "t1", "a1", "Ana", "One", 1, 200_000, "flac", 900);
    file(&c, "t1", Some("Solo"), None, None);
    assert!(of(&check(&c, "Ana"), Kind::AlbumArtist).is_empty());
    // The same album name under someone else: Plex broke it up.
    album(&c, "b1", "Bo", "Solo", None);
    let found = check(&c, "Ana");
    let f = of(&found, Kind::AlbumArtist);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].others[0].id, "b1");
    assert!(f[0].evidence.iter().any(|e| e.contains("also under Bo")));
}

// ------------------------------------------------------------------ split albums

#[test]
fn an_album_in_two_halves_is_split() {
    let (_d, c) = db();
    album(&c, "a1", "Ana", "Night Drive", Some(2016));
    album(&c, "a2", "Ana", "Night Drive", Some(2016));
    for n in 1..=4 {
        song(&c, &format!("x{n}"), "a1", "Ana", &format!("Song {n}"), n, 200_000, "flac", 900);
    }
    for n in 5..=6 {
        song(&c, &format!("y{n}"), "a2", "Ana", &format!("Song {n}"), n, 200_000, "flac", 900);
    }
    let found = check(&c, "Ana");
    let f = of(&found, Kind::SplitAlbum);
    assert_eq!(f.len(), 1, "{found:#?}");
    let f = f[0];
    assert_eq!(f.id, "splitAlbum:a1+a2");
    assert_eq!(f.album_id, "a1", "the bigger part keeps its place");
    assert_eq!(f.others[0].id, "a2");
    assert_eq!(f.fix, Fix::MergeAlbums);
    assert!(f.evidence.iter().any(|e| e.contains("songs 1 to 4")) && f.evidence.iter().any(|e| e.contains("songs 5 to 6")));
}

#[test]
fn editions_copies_other_years_and_unreleased_builds_are_not_split() {
    let setup = |c: &Connection| {
        album(c, "a1", "Ana", "Night Drive", Some(2016));
        album(c, "a2", "Ana", "Night Drive", Some(2016));
        song(c, "x1", "a1", "Ana", "Song 1", 1, 200_000, "flac", 900);
        song(c, "y2", "a2", "Ana", "Song 2", 2, 200_000, "flac", 900);
    };
    // An edition tag keeps them apart.
    let (_d, c) = db();
    setup(&c);
    c.execute("INSERT INTO album_tags (album_id, edition) VALUES ('a2', 'Build v2')", []).unwrap();
    assert!(of(&check(&c, "Ana"), Kind::SplitAlbum).is_empty());
    // Another year: a reissue.
    let (_d, c) = db();
    setup(&c);
    c.execute("UPDATE albums SET year = 2021 WHERE id = 'a2'", []).unwrap();
    assert!(of(&check(&c, "Ana"), Kind::SplitAlbum).is_empty());
    // The same songs in both: two copies, not halves.
    let (_d, c) = db();
    setup(&c);
    song(&c, "y1", "a2", "Ana", "Song 1", 1, 200_000, "flac", 900);
    assert!(of(&check(&c, "Ana"), Kind::SplitAlbum).is_empty());
    // An unreleased build next to the released album.
    let (_d, c) = db();
    setup(&c);
    c.execute("UPDATE albums SET subformats = '[\"Demo\"]' WHERE id = 'a2'", []).unwrap();
    assert!(of(&check(&c, "Ana"), Kind::SplitAlbum).is_empty());
    // A deluxe edition title is another album.
    let (_d, c) = db();
    setup(&c);
    c.execute("UPDATE albums SET title = 'Night Drive (Deluxe Edition)' WHERE id = 'a2'", []).unwrap();
    assert!(of(&check(&c, "Ana"), Kind::SplitAlbum).is_empty());
}

// ------------------------------------------------------------------ low quality

#[test]
fn a_lossy_copy_next_to_a_lossless_one_is_low_quality() {
    let (_d, c) = db();
    album(&c, "a1", "Ana", "Night Drive", Some(2016));
    album(&c, "s1", "Ana", "Low Light", Some(2016));
    song(&c, "t1", "a1", "Ana", "Low Light", 1, 200_000, "flac", 900);
    song(&c, "t2", "a1", "Ana", "Other", 2, 150_000, "flac", 900);
    song(&c, "u1", "s1", "Ana", "Low Light", 1, 200_500, "mp3", 128);
    c.execute("UPDATE tracks SET bit_depth = 16, sample_rate = 44100 WHERE id = 't1'", []).unwrap();
    let found = check(&c, "Ana");
    let f = of(&found, Kind::LowQuality);
    assert_eq!(f.len(), 1, "{found:#?}");
    let f = f[0];
    assert_eq!(f.id, "lowQuality:s1>a1");
    assert_eq!(f.album_id, "s1");
    assert_eq!(f.others[0].id, "a1");
    assert_eq!(f.tracks, ["u1"]);
    assert!(f.evidence[0].contains("MP3 128 kbps here, FLAC 16-bit/44.1 kHz on Night Drive"), "{:?}", f.evidence);
}

#[test]
fn similar_copies_and_unreleased_ones_are_left_alone() {
    let (_d, c) = db();
    album(&c, "a1", "Ana", "Night Drive", Some(2016));
    album(&c, "s1", "Ana", "Low Light", Some(2016));
    song(&c, "t1", "a1", "Ana", "Low Light", 1, 200_000, "mp3", 320);
    song(&c, "u1", "s1", "Ana", "Low Light", 1, 200_500, "mp3", 256);
    assert!(of(&check(&c, "Ana"), Kind::LowQuality).is_empty(), "256 next to 320 is fine");
    c.execute("UPDATE tracks SET bitrate = 128 WHERE id = 'u1'", []).unwrap();
    assert_eq!(of(&check(&c, "Ana"), Kind::LowQuality).len(), 1);
    // An unreleased copy never matches a released one.
    c.execute("UPDATE albums SET subformats = '[\"Demo\"]' WHERE id = 's1'", []).unwrap();
    assert!(of(&check(&c, "Ana"), Kind::LowQuality).is_empty());
}

// ------------------------------------------------------------------ unmatched

fn mb(c: &Connection, album_id: &str, found: bool) {
    c.execute(
        "INSERT OR REPLACE INTO album_mb_check (album_id, found, release_group_id, checked_at, expires_at) VALUES (?1, ?2, 'rg', 1, 9999999999)",
        params![album_id, found],
    )
    .unwrap();
}

#[test]
fn unmatched_albums_musicbrainz_knows_are_found_and_unreleased_ones_never() {
    let (_d, c) = db();
    album(&c, "a1", "Ana", "Night Drive", Some(2016));
    album(&c, "a2", "Ana", "Vault Tapes", None);
    album(&c, "a3", "Ana", "Demo Tape", None);
    c.execute("UPDATE albums SET matched = 0", []).unwrap();
    c.execute("UPDATE albums SET subformats = '[\"Demo\"]' WHERE id = 'a3'", []).unwrap();
    song(&c, "t1", "a1", "Ana", "One", 1, 200_000, "flac", 900);
    song(&c, "t2", "a2", "Ana", "Two", 1, 200_000, "flac", 900);
    song(&c, "t3", "a3", "Ana", "Three", 1, 200_000, "flac", 900);
    mb(&c, "a1", true);
    mb(&c, "a2", false);
    mb(&c, "a3", true);
    let found = check(&c, "Ana");
    let f = of(&found, Kind::Unmatched);
    assert_eq!(f.len(), 1, "{found:#?}");
    assert_eq!(f[0].album_id, "a1");
    assert_eq!(f[0].fix, Fix::Match);
    assert!(!f[0].evidence.is_empty(), "the release evidence is the why");
    // A mark wins: confirmed unreleased is never one to match.
    let mut marks = HashMap::new();
    marks.insert(crate::db::release::mark_key("album", "a1"), crate::db::release::Mark::Unreleased);
    crate::db::release::set_marks(marks);
    let found = check(&c, "Ana");
    crate::db::release::set_marks(HashMap::new());
    assert!(of(&found, Kind::Unmatched).is_empty());
}

// ------------------------------------------------------------------ incremental

#[test]
fn unchanged_artists_are_skipped_and_a_change_checks_again() {
    let (_d, mut c) = db();
    album(&c, "a1", "Ana", "Night Drive", Some(2016));
    song(&c, "t1", "a1", "Ana", "One", 1, 200_000, "flac", 900);
    let (sig, found) = checks::check_if_changed(&c, "ar:Ana").unwrap().expect("first check");
    store::replace_artist(&mut c, "ar:Ana", &sig, &found).unwrap();
    assert!(store::checked(&c, "ar:Ana").unwrap().is_some());
    assert!(checks::check_if_changed(&c, "ar:Ana").unwrap().is_none(), "nothing changed");
    song(&c, "t2", "a1", "Ana", "One", 2, 200_000, "mp3", 320);
    let (sig2, found) = checks::check_if_changed(&c, "ar:Ana").unwrap().expect("a new song");
    assert_ne!(sig, sig2);
    assert_eq!(of(&found, Kind::Duplicate).len(), 1);
    store::replace_artist(&mut c, "ar:Ana", &sig2, &found).unwrap();
    // Check now forgets what each check saw.
    store::forget(&mut c).unwrap();
    assert!(checks::check_if_changed(&c, "ar:Ana").unwrap().is_some());
}

#[test]
fn signatures_are_stable_and_follow_the_data() {
    let mut d = ArtistData::default();
    let a = checks::signature(&d);
    assert_eq!(a, checks::signature(&d));
    d.elsewhere.insert("x".into(), Vec::new());
    assert_ne!(a, checks::signature(&d));
}

// ------------------------------------------------------------------ store and covers

fn opts() -> Options {
    Options::default()
}

#[test]
fn the_list_has_stored_findings_and_covers_filtered_by_the_checks() {
    let (_d, mut c) = db();
    album(&c, "a1", "Ana", "Night Drive", Some(2016));
    album(&c, "a2", "Ana", "No Art", None);
    album(&c, "a3", "Ana", "Tiny Art", None);
    album(&c, "a4", "Ana", "Big Art", None);
    album(&c, "a5", "Ana", "Unknown Size", None);
    c.execute("UPDATE albums SET thumb = NULL WHERE id = 'a2'", []).unwrap();
    covers::save(&c, "a3", "/library/metadata/a3/thumb/1", Some((300, 300)), 1).unwrap();
    covers::save(&c, "a4", "/library/metadata/a4/thumb/1", Some((1200, 1200)), 1).unwrap();
    covers::save(&c, "a5", "/library/metadata/a5/thumb/1", None, 1).unwrap();
    song(&c, "t1", "a1", "Ana", "One", 1, 200_000, "flac", 900);
    song(&c, "t2", "a1", "Ana", "One", 2, 200_000, "flac", 900);
    let (sig, found) = checks::check_if_changed(&c, "ar:Ana").unwrap().unwrap();
    store::replace_artist(&mut c, "ar:Ana", &sig, &found).unwrap();

    let all = store::all(&c, &opts()).unwrap();
    let kinds: Vec<(Kind, &str)> = all.iter().map(|f| (f.kind, f.album_id.as_str())).collect();
    assert_eq!(kinds, [(Kind::Duplicate, "a1"), (Kind::Cover, "a2"), (Kind::Cover, "a3")], "page order: by kind, then album");
    let tiny = &all[2];
    assert_eq!(tiny.title, "Small cover (300 \u{d7} 300)");
    assert_eq!(tiny.fix, Fix::Cover);
    assert_eq!(all[1].title, "No cover");

    // The size setting decides what's small, without a new check.
    let big = Options { cover_px: 1400, ..opts() };
    assert_eq!(store::all(&c, &big).unwrap().iter().filter(|f| f.kind == Kind::Cover).count(), 3);
    // A check that's off drops out.
    let no_covers = Options { checks: Checks { covers: false, ..Checks::default() }, ..opts() };
    assert!(store::all(&c, &no_covers).unwrap().iter().all(|f| f.kind != Kind::Cover));
    let no_dupes = Options { checks: Checks { duplicates: false, ..Checks::default() }, ..opts() };
    assert!(store::all(&c, &no_dupes).unwrap().iter().all(|f| f.kind != Kind::Duplicate));

    // A cover that changed is sized again; one that's gone is forgotten.
    assert!(covers::pending(&c, 10).unwrap().iter().all(|(a, _)| a != "a3"));
    c.execute("UPDATE albums SET thumb = '/library/metadata/a3/thumb/2' WHERE id = 'a3'", []).unwrap();
    assert!(covers::pending(&c, 10).unwrap().iter().any(|(a, _)| a == "a3"));
    assert!(store::all(&c, &opts()).unwrap().iter().all(|f| f.album_id != "a3"), "an old size doesn't count for a new cover");
    c.execute("DELETE FROM albums WHERE id = 'a4'", []).unwrap();
    c.execute("DELETE FROM albums WHERE id = 'a1'", []).unwrap();
    c.execute("DELETE FROM albums WHERE artist_id = 'ar:Ana'", []).unwrap();
    store::prune(&mut c).unwrap();
    assert!(store::stored(&c).unwrap().is_empty());
    assert_eq!(c.query_row("SELECT count(*) FROM doctor_covers", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn ignored_findings_move_to_their_own_list() {
    let f = |id: &str| Finding {
        id: id.into(),
        kind: Kind::Cover,
        artist_id: "ar".into(),
        artist: "A".into(),
        album_id: "a".into(),
        album: "B".into(),
        thumb: None,
        title: "t".into(),
        text: "x".into(),
        evidence: Vec::new(),
        fix: Fix::Cover,
        tracks: Vec::new(),
        others: Vec::new(),
        table: Vec::new(),
        suggest: None,
    };
    let mut ig = HashMap::new();
    ig.insert("cover:2".to_string(), Ignored { kind: Kind::Cover, title: "t".into(), at: 1 });
    let (open, ignored) = commands::split(vec![f("cover:1"), f("cover:2")], &ig);
    assert_eq!(open.len(), 1);
    assert_eq!(ignored[0].id, "cover:2");
}

#[test]
fn ignores_live_in_the_sync_store_in_the_library_group() {
    use crate::sync::store::{DOCTOR_IGNORED, NAMESPACES};
    assert!(NAMESPACES.contains(&DOCTOR_IGNORED.name()));
    let g = crate::sync::engine::Groups { library: true, ..Default::default() };
    assert!(g.namespaces().contains(&DOCTOR_IGNORED.name()));
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("s.db")).unwrap();
    let s = crate::sync::SyncStore::new(db, "dev");
    s.put(&DOCTOR_IGNORED, "cover:a1", &Ignored { kind: Kind::Cover, title: "No cover".into(), at: 5 }).unwrap();
    let back: Vec<(String, Ignored)> = s.list(&DOCTOR_IGNORED).unwrap();
    assert_eq!(back[0].0, "cover:a1");
    assert_eq!(back[0].1.kind, Kind::Cover);
}

// ------------------------------------------------------------------ cover sizes

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::new(w, h);
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, 90]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg).unwrap();
    out
}

#[test]
fn sizes_come_from_the_first_bytes() {
    assert_eq!(covers::dimensions(&png(700, 500)), Some((700, 500)));
    let j = jpeg(1600, 1600);
    assert!(j.len() > 64 * 1024, "big enough to cut");
    assert_eq!(covers::dimensions(&j[..64 * 1024]), Some((1600, 1600)), "the header is enough");
    assert_eq!(covers::dimensions(b"not an image"), None);
}

struct Parts(String);

#[async_trait::async_trait]
impl PartSource for Parts {
    async fn part(&self, _id: &str, key: &str) -> AppResult<StreamSource> {
        if self.0.is_empty() {
            return Err(AppError::Network("offline".into()));
        }
        Ok(StreamSource { url: format!("{}{}", self.0, crate::sid::split(key).1), ..Default::default() })
    }
}

fn reader() -> Reader {
    Reader::new(crate::audio::http::http_client(), TransferGate::new(), None)
}

#[tokio::test]
async fn covers_are_sized_over_http_and_failures_wait() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("c.db")).unwrap();
    db.write(|c| {
        album(c, "a1", "Ana", "Small", None);
        album(c, "a2", "Ana", "Big", None);
        album(c, "a3", "Ana", "Broken", None);
        Ok(())
    })
    .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/library/metadata/a1/thumb/1"))
        .respond_with(ResponseTemplate::new(206).set_body_bytes(png(400, 400)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/library/metadata/a2/thumb/1"))
        .respond_with(ResponseTemplate::new(206).set_body_bytes(png(1000, 1000)))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/library/metadata/a3/thumb/1")).respond_with(ResponseTemplate::new(404)).mount(&server).await;

    // Offline: nothing is kept, so it's tried again later.
    let n = covers::probe_round(&db, &Parts(String::new()), &reader(), 10, Duration::ZERO).await.unwrap();
    assert_eq!(n, 0);
    assert_eq!(db.read(|c| covers::pending(c, 10)).unwrap().len(), 3);

    let n = covers::probe_round(&db, &Parts(server.uri()), &reader(), 10, Duration::ZERO).await.unwrap();
    assert_eq!(n, 3);
    assert!(db.read(|c| covers::pending(c, 10)).unwrap().is_empty(), "a missing image isn't asked for again");
    let small: Vec<String> = db.read(|c| store::cover_findings(c, 600)).unwrap().into_iter().map(|f| f.album_id).collect();
    assert_eq!(small, ["a1"]);
}

// ------------------------------------------------------------------ settings

#[test]
fn settings_default_validate_sync_and_reach_the_doctor() {
    use crate::settings::{SYNCED_SETTINGS, Settings, SettingsPatch};
    let mut s = Settings::default();
    assert!(s.library_doctor, "on by default");
    assert_eq!(s.doctor_checks, Checks::default());
    assert!(Kind::ALL.iter().all(|k| s.doctor_checks.on(*k)), "every check on");
    assert_eq!(s.doctor_cover_px, 600);
    for k in ["libraryDoctor", "doctorChecks", "doctorCoverPx"] {
        assert!(SYNCED_SETTINGS.contains(&k), "{k} is synced");
    }
    s.apply(&SettingsPatch {
        library_doctor: Some(false),
        doctor_checks: Some(Checks { low_quality: false, ..Checks::default() }),
        doctor_cover_px: Some(1000),
        ..Default::default()
    })
    .unwrap();
    let o = s.doctor_options();
    assert!(!o.on && !o.checks.low_quality && o.checks.covers);
    assert_eq!(o.cover_px, 1000);
    assert!(s.apply(&SettingsPatch { doctor_cover_px: Some(601), ..Default::default() }).is_err());
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["libraryDoctor"], false);
    assert_eq!(json["doctorChecks"]["lowQuality"], false);
    assert_eq!(json["doctorCoverPx"], 1000);
    // Settings saved before the doctor existed take its defaults.
    let old: Settings = serde_json::from_value(serde_json::json!({ "doctorChecks": { "covers": false } })).unwrap();
    assert!(old.library_doctor && !old.doctor_checks.covers && old.doctor_checks.duplicates);
}

#[test]
fn kinds_round_trip_their_keys() {
    for k in Kind::ALL {
        assert_eq!(Kind::from_key(k.key()), Some(k));
        assert_eq!(serde_json::to_value(k).unwrap(), k.key());
    }
    assert_eq!(Kind::from_key("nope"), None);
}
