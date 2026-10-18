//! Archive tests: the evidence list and its precedence, marks (and their
//! sync), folder and length hints, song status, versions grouping and its
//! incremental refresh, search grouping, artist sections, eras and dates.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{Connection, params};

use super::eras::{self, parse_date};
use super::sections::{Placed, classify};
use super::versions::{self, Song};
use super::{marks, status};
use crate::db::release::{self, AlbumFacts, EvidenceKind, Mark, ReleaseStatus};
use crate::db::types::{SearchHit, SearchKind, SearchResults};
use crate::db::{DbHandle, schema};
use crate::model::{Album, ReleaseSection, SongStatus};
use crate::sync::SyncStore;

// ------------------------------------------------------------------ helpers

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
    schema::migrate(&mut c).unwrap();
    (dir, c)
}

/// An album. `matched`: Plex matched it. `format` and `subs` as Plex has them.
#[allow(clippy::too_many_arguments)]
fn album(c: &Connection, id: &str, artist: &str, title: &str, matched: bool, format: Option<&str>, subs: &[&str], year: Option<i32>) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, matched, format, subformats, year, thumb, added_at)
         VALUES (?1, ?2, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            title,
            format!("ar:{artist}"),
            artist,
            matched,
            format,
            serde_json::to_string(subs).unwrap(),
            year,
            format!("/thumb/{id}"),
            year.unwrap_or(2000) as i64
        ],
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn song(c: &Connection, id: &str, album_id: &str, album: &str, artist: &str, title: &str, ms: i64, file: Option<&str>, no: u32) {
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, file, track_number)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![id, title, album_id, album, format!("ar:{artist}"), artist, ms, format!("/p/{id}"), file, no],
    )
    .unwrap();
}

/// File tags for a song: version, era, date, group.
fn tags(c: &Connection, id: &str, version: Option<&str>, era: Option<&str>, date: Option<&str>, grp: Option<&str>) {
    c.execute(
        "INSERT OR REPLACE INTO file_tags (part_key, status, version, era, date, grp, scanned_at, ver) VALUES (?1, 'ok', ?2, ?3, ?4, ?5, 1, 9)",
        params![format!("/p/{id}"), version, era, date, grp],
    )
    .unwrap();
}

fn album_tag(c: &Connection, id: &str, version: Option<&str>, grp: Option<&str>, edition: Option<&str>) {
    c.execute(
        "INSERT OR REPLACE INTO album_tags (album_id, version, grp, edition) VALUES (?1, ?2, ?3, ?4)",
        params![id, version, grp, edition],
    )
    .unwrap();
}

fn refresh(c: &Connection) {
    crate::identity::index::rebuild(c).unwrap();
    super::refresh_all(c).unwrap();
}

fn status_of(c: &Connection, id: &str) -> SongStatus {
    versions::song_status(c, id).unwrap().unwrap().0
}

fn group_of(c: &Connection, id: &str) -> String {
    c.query_row("SELECT group_key FROM versions WHERE track_id = ?1", [id], |r| r.get(0)).unwrap()
}

fn unmatched() -> AlbumFacts {
    AlbumFacts { matched: Some(false), ..Default::default() }
}

fn kinds(f: &AlbumFacts) -> Vec<EvidenceKind> {
    release::assess(f).1.iter().map(|e| e.kind).collect()
}

// ------------------------------------------------------------------ evidence

#[test]
fn each_rule_alone() {
    // 1. A demo release type.
    let f = AlbumFacts { subformats: vec!["demo".into()], matched: Some(true), ..Default::default() };
    assert_eq!(release::assess(&f).0, ReleaseStatus::TaggedUnreleased);
    assert_eq!(kinds(&f), [EvidenceKind::Tag]);
    // 1. An ARCHIVE_VERSION, either way.
    let f = AlbumFacts { version: Some("leak".into()), matched: Some(true), ..Default::default() };
    assert_eq!(release::assess(&f).0, ReleaseStatus::TaggedUnreleased);
    let f = AlbumFacts { version: Some("released".into()), ..unmatched() };
    assert_eq!(release::assess(&f).0, ReleaseStatus::Released);
    // 2. A mark.
    let f = AlbumFacts { mark: Some(Mark::Unreleased), matched: Some(true), ..Default::default() };
    assert_eq!(release::assess(&f).0, ReleaseStatus::ConfirmedUnreleased);
    let f = AlbumFacts { mark: Some(Mark::Released), ..unmatched() };
    assert_eq!(release::assess(&f).0, ReleaseStatus::Released);
    assert_eq!(kinds(&f), [EvidenceKind::Mark]);
    // 3. Plex couldn't match it, MusicBrainz not asked.
    let (s, ev) = release::assess(&unmatched());
    assert_eq!(s, ReleaseStatus::ProbablyUnreleased);
    assert_eq!(ev.iter().map(|e| e.kind).collect::<Vec<_>>(), [EvidenceKind::Unmatched, EvidenceKind::Untyped, EvidenceKind::NotChecked]);
    assert!(ev.iter().all(|e| !e.decides), "a guess, not a decision");
    // 4. MusicBrainz either way.
    let f = AlbumFacts { mb_found: Some(false), ..unmatched() };
    assert!(kinds(&f).contains(&EvidenceKind::Musicbrainz));
    assert!(release::score(&release::assess(&f).1) > release::score(&release::assess(&unmatched()).1), "stronger");
    let f = AlbumFacts { mb_found: Some(true), ..unmatched() };
    assert_eq!(release::assess(&f).0, ReleaseStatus::PlexMissed);
    // Matched, nothing tagged: released with nothing to say.
    let (s, ev) = release::assess(&AlbumFacts { matched: Some(true), ..Default::default() });
    assert_eq!((s, ev.len()), (ReleaseStatus::Released, 0));
}

#[test]
fn tags_beat_marks_and_marks_beat_guesses() {
    // A released mark on a demo: the tag wins and the popover says so.
    let f = AlbumFacts { subformats: vec!["demo".into()], mark: Some(Mark::Released), ..Default::default() };
    let (s, ev) = release::assess(&f);
    assert_eq!(s, ReleaseStatus::TaggedUnreleased);
    assert!(ev.iter().any(|e| e.kind == EvidenceKind::MarkOverridden && e.text.contains("overridden by the file's tags")));
    // An unreleased mark on a file tagged released.
    let f = AlbumFacts { version: Some("released".into()), mark: Some(Mark::Unreleased), ..unmatched() };
    let (s, ev) = release::assess(&f);
    assert_eq!(s, ReleaseStatus::Released);
    assert!(ev.iter().any(|e| e.kind == EvidenceKind::MarkOverridden));
    // A mark that agrees with the tag isn't "overridden".
    let f = AlbumFacts { version: Some("leak".into()), mark: Some(Mark::Unreleased), ..Default::default() };
    assert!(!kinds(&f).contains(&EvidenceKind::MarkOverridden));
    // Marks beat every guess: unmatched, MusicBrainz found, a release type.
    let f = AlbumFacts { mark: Some(Mark::Released), mb_found: Some(false), ..unmatched() };
    assert_eq!(release::assess(&f).0, ReleaseStatus::Released);
    let f = AlbumFacts { mark: Some(Mark::Unreleased), mb_found: Some(true), ..unmatched() };
    assert_eq!(release::assess(&f).0, ReleaseStatus::ConfirmedUnreleased);
    let f = AlbumFacts { mark: Some(Mark::Unreleased), format: Some("album".into()), matched: Some(true), ..Default::default() };
    assert_eq!(release::assess(&f).0, ReleaseStatus::ConfirmedUnreleased);
    // The setting off doesn't touch marks.
    release::set_count_unmatched(false);
    let f = AlbumFacts { mark: Some(Mark::Unreleased), ..unmatched() };
    assert_eq!(release::assess(&f).0, ReleaseStatus::ConfirmedUnreleased);
    assert_eq!(release::assess(&unmatched()).0, ReleaseStatus::Released);
    release::set_count_unmatched(true);
}

#[test]
fn rules_combined_list_every_piece() {
    let f = AlbumFacts { mb_found: Some(false), folders: vec!["Leaks".into()], short_songs: 3, songs: 4, ..unmatched() };
    let (s, ev) = release::assess(&f);
    assert_eq!(s, ReleaseStatus::ProbablyUnreleased);
    let k: Vec<EvidenceKind> = ev.iter().map(|e| e.kind).collect();
    assert_eq!(
        k,
        [EvidenceKind::Unmatched, EvidenceKind::Untyped, EvidenceKind::Musicbrainz, EvidenceKind::Folder, EvidenceKind::Duration]
    );
    assert_eq!(ev[3].text, "Folder name: Leaks");
    assert_eq!(release::score(&ev), 30 + 10 + 40 + 10 + 5);
}

#[test]
fn hints_never_decide_alone() {
    let hints = |f: AlbumFacts| AlbumFacts { folders: vec!["Leaks".into(), "Demos".into()], short_songs: 5, songs: 5, ..f };
    // Plex matched it: still released, whatever the folders say.
    let f = hints(AlbumFacts { matched: Some(true), ..Default::default() });
    let (s, ev) = release::assess(&f);
    assert_eq!(s, ReleaseStatus::Released);
    assert!(ev.iter().all(|e| !e.decides), "listed, never deciding");
    // A release type: released.
    assert_eq!(release::assess(&hints(AlbumFacts { format: Some("ep".into()), ..unmatched() })).0, ReleaseStatus::Released);
    // MusicBrainz found it: Plex missed it, hints or not.
    assert_eq!(release::assess(&hints(AlbumFacts { mb_found: Some(true), ..unmatched() })).0, ReleaseStatus::PlexMissed);
    // The unmatched guess off: hints don't bring it back.
    release::set_count_unmatched(false);
    assert_eq!(release::assess(&hints(unmatched())).0, ReleaseStatus::Released);
    release::set_count_unmatched(true);
    // And they never touch the yes/no every feature uses.
    assert!(!release::album_unreleased("x", None, &["live"], Some(true), None, None, None));
}

#[test]
fn folder_keywords_match_whole_words_in_folders() {
    let k = release::default_folder_keywords();
    assert_eq!(release::folder_hits("/music/Artist/Leaks/2019/01 Song.flac", &k), ["Leaks"]);
    assert_eq!(release::folder_hits("/music/Artist/leak pile/01.flac", &k), ["Leaks"], "singular");
    assert_eq!(release::folder_hits("D:\\Music\\Artist\\UNRELEASED SESSIONS\\a.mp3", &k), ["Unreleased", "Sessions"]);
    assert!(release::folder_hits("/music/Artist/Album/Leaks.flac", &k).is_empty(), "the file name doesn't count");
    assert!(release::folder_hits("/music/Leaksville/Album/a.flac", &k).is_empty(), "whole words only");
    assert!(release::folder_hits("/music/Démos/a.flac", &k).len() == 1, "accents fold");
    assert!(release::folder_hits("/music/Leaks/a.flac", &[] as &[&str]).is_empty());
}

#[test]
fn folder_and_length_hints_come_from_the_songs() {
    let (_d, c) = db();
    album(&c, "al", "Artist", "Tape", false, None, &[], None);
    song(&c, "1", "al", "Tape", "Artist", "One", 30_000, Some("/m/Artist/Leaks/Tape/01.flac"), 1);
    song(&c, "2", "al", "Tape", "Artist", "Two", 40_000, Some("/m/Artist/Leaks/Tape/02.flac"), 2);
    song(&c, "3", "al", "Tape", "Artist", "Three", 200_000, None, 3);
    let s = status::album_status(&c, "al").unwrap().unwrap();
    assert_eq!(s.status, ReleaseStatus::ProbablyUnreleased);
    assert!(s.evidence.iter().any(|e| e.text == "Folder name: Leaks"));
    assert!(s.evidence.iter().any(|e| e.kind == EvidenceKind::Duration));
    assert_eq!(s.reasons.len(), s.evidence.len());
    // Stored per album.
    status::refresh(&c, None).unwrap();
    let (st, ev, score): (String, String, i32) =
        c.query_row("SELECT status, evidence, score FROM album_status WHERE album_id = 'al'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!(st, "probablyUnreleased");
    assert!(ev.contains("Folder name: Leaks"));
    assert_eq!(score, s.score);
    // The keywords follow the setting.
    release::set_folder_keywords(vec!["Vault".into()]);
    let s = status::album_status(&c, "al").unwrap().unwrap();
    assert!(!s.evidence.iter().any(|e| e.kind == EvidenceKind::Folder));
    release::set_folder_keywords(release::default_folder_keywords());
}

// ------------------------------------------------------------------ marks

fn store(dir: &tempfile::TempDir, name: &str, clock: Arc<AtomicU64>) -> SyncStore {
    let db = DbHandle::open(dir.path().join(format!("{name}.db"))).unwrap();
    SyncStore::with_clock(db, name, Arc::new(move || clock.load(Ordering::SeqCst)))
}

#[test]
fn a_mark_reaches_every_consumer_of_the_rule() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(&dir, "a", Arc::new(AtomicU64::new(1_000)));
    let none: [&str; 0] = [];
    assert!(release::album_unreleased("al", None, &none, Some(false), None, None, None));
    marks::set(&s, "album", "al", Some(Mark::Released)).unwrap();
    assert!(!release::album_unreleased("al", None, &none, Some(false), None, None, None), "mark beats the guess");
    assert!(release::album_unreleased("al", None, &["demo"], Some(false), None, None, None), "tag beats the mark");
    // A song's own mark beats its album's.
    marks::set(&s, "track", "t1", Some(Mark::Unreleased)).unwrap();
    assert!(release::track_unreleased("t1", "al", None, &none, Some(false), None, None, None));
    assert!(!release::track_unreleased("t2", "al", None, &none, Some(false), None, None, None));
    // Cleared: back to the guess.
    marks::set(&s, "album", "al", None).unwrap();
    assert!(release::album_unreleased("al", None, &none, Some(false), None, None, None));
    assert!(marks::set(&s, "artist", "x", Some(Mark::Released)).is_err());
    release::set_marks(HashMap::new());
}

#[test]
fn marks_merge_through_the_sync_file_last_change_wins() {
    let dir = tempfile::tempdir().unwrap();
    let (ca, cb) = (Arc::new(AtomicU64::new(1_000)), Arc::new(AtomicU64::new(1_000)));
    let a = store(&dir, "a", ca.clone());
    let b = store(&dir, "b", cb.clone());
    marks::set(&a, "album", "al", Some(Mark::Unreleased)).unwrap();
    cb.store(2_000, Ordering::SeqCst);
    marks::set(&b, "album", "al", Some(Mark::Released)).unwrap();
    let ns = [crate::sync::store::RELEASE_MARKS.name()];
    a.merge(&b.entries(&ns).unwrap(), &ns).unwrap();
    b.merge(&a.entries(&ns).unwrap(), &ns).unwrap();
    marks::load(&a).unwrap();
    assert_eq!(release::mark_of("album:al"), Some(Mark::Released), "the later change");
    marks::load(&b).unwrap();
    assert_eq!(release::mark_of("album:al"), Some(Mark::Released), "both agree");
    // Marks travel with the library group.
    assert!(crate::sync::engine::Groups::default().namespaces().contains(&"releaseMarks"));
    assert!(crate::sync::engine::Groups::default().namespaces().contains(&"albumSections"));
    release::set_marks(HashMap::new());
}

#[test]
fn custom_sections_are_named_and_kept_per_artist() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(&dir, "a", Arc::new(AtomicU64::new(1_000)));
    marks::set_section(&s, "al1", "ar", Some("  Vault Picks ")).unwrap();
    marks::set_section(&s, "al2", "ar", Some("vault picks")).unwrap();
    marks::set_section(&s, "al3", "other", Some("Elsewhere")).unwrap();
    assert_eq!(marks::section_names(&s, "ar").unwrap(), ["Vault Picks"], "one spelling");
    assert_eq!(marks::custom_map(&s, Some("ar")).unwrap().len(), 2);
    assert!(marks::set_section(&s, "al1", "ar", Some("   ")).is_err());
    assert!(marks::set_section(&s, "al1", "ar", Some(&"x".repeat(41))).is_err());
    marks::set_section(&s, "al1", "ar", None).unwrap();
    assert_eq!(marks::custom_map(&s, Some("ar")).unwrap().len(), 1);
}

// ------------------------------------------------------------------ versions

fn s(id: &str, artist: &str, album: &str, title: &str) -> Song {
    Song {
        id: id.into(),
        artist_id: format!("ar:{artist}"),
        album_id: format!("al:{album}"),
        title: title.into(),
        album_title: album.into(),
        album_artist: artist.into(),
        ..Default::default()
    }
}

#[test]
fn versions_group_by_folded_title_group_tag_and_ids() {
    let songs = vec![
        s("1", "Artist", "Album", "Song"),
        s("2", "Artist", "Tape", "Song (Demo)"),
        s("3", "ARTIST", "Leaks", "song - snippet [v2]"),
        s("4", "Artist", "Album", "Song Part II"),
        s("5", "Artist", "Tape", "Song Part 2"),
        s("6", "Artist", "Album", "Intro"),
        s("7", "Artist", "Tape", "Intro"),
        s("8", "Other", "Theirs", "Song"),
        s("9", "Artist", "Album", "Song (Interlude)"),
        Song { group: Some("Build".into()), ..s("10", "Artist", "Build v1", "Intro") },
        Song { group: Some("build".into()), ..s("11", "Artist", "Build v2", "Intro") },
        Song { recording_mbid: Some("rec-1".into()), ..s("12", "Artist", "Album", "Totally Different") },
        Song { recording_mbid: Some("REC-1".into()), ..s("13", "Artist", "Live", "Another Name") },
        Song { track_artist: Some("Artist feat. Guest".into()), ..s("14", "Remixer", "Remixes", "Song (Remixer Remix)") },
    ];
    let g = versions::group(&songs);
    let same = |a: usize, b: usize| g[a - 1] == g[b - 1];
    assert!(same(1, 2) && same(1, 3), "suffixes fold away");
    assert!(!same(4, 5), "Part II and Part 2 stay apart");
    assert!(!same(1, 4) && !same(1, 9));
    assert!(!same(6, 7), "a generic title only joins on the same album");
    assert!(!same(1, 8), "another artist");
    assert!(same(10, 11), "a shared ARCHIVE_GROUP joins generic titles");
    assert!(same(12, 13), "the same recording id");
    assert!(same(1, 14), "the song's own artist is a second name");
    // Deterministic names.
    assert_eq!(g, versions::group(&songs));
}

#[test]
fn song_status_alternates_and_unreleased() {
    let (_d, c) = db();
    album(&c, "rel", "Artist", "Album", true, Some("album"), &[], Some(2019));
    song(&c, "r1", "rel", "Album", "Artist", "Song", 200_000, None, 1);
    song(&c, "r2", "rel", "Album", "Artist", "Intro", 60_000, None, 2);
    // Plex couldn't match it.
    album(&c, "tape", "Artist", "Tape", false, None, &[], None);
    song(&c, "t1", "tape", "Tape", "Artist", "Song (Early Version)", 190_000, None, 1);
    song(&c, "t2", "tape", "Tape", "Artist", "Brand New", 150_000, None, 2);
    song(&c, "t3", "tape", "Tape", "Artist", "Intro", 50_000, None, 3);
    // Tagged unreleased.
    album(&c, "demo", "Artist", "Demos", true, Some("album"), &["demo"], None);
    song(&c, "d1", "demo", "Demos", "Artist", "Song", 180_000, None, 1);
    // One song on the tape tagged as released.
    song(&c, "t4", "tape", "Tape", "Artist", "Single Cut", 180_000, None, 4);
    tags(&c, "t4", Some("released"), None, None, None);
    refresh(&c);
    assert_eq!(status_of(&c, "r1"), SongStatus::Released);
    assert_eq!(status_of(&c, "t1"), SongStatus::ProbablyAlternate);
    assert_eq!(versions::song_status(&c, "t1").unwrap().unwrap().1.as_deref(), Some("r1"));
    assert_eq!(status_of(&c, "t2"), SongStatus::ProbablyUnreleased);
    assert_eq!(status_of(&c, "t3"), SongStatus::ProbablyUnreleased, "Intro is on the stoplist");
    assert_eq!(status_of(&c, "d1"), SongStatus::Unreleased);
    assert_eq!(status_of(&c, "t4"), SongStatus::Released, "status is per song");
    // An alternate is still unreleased everywhere.
    let none: [&str; 0] = [];
    assert!(release::track_unreleased("t1", "tape", None, &none, Some(false), None, None, None));
    // Versions of the released song, either way round.
    let mut ids = versions::other_ids(&c, "r1", false).unwrap();
    ids.sort();
    assert_eq!(ids, ["d1", "t1"]);
    assert_eq!(versions::other_ids(&c, "t2", false).unwrap(), Vec::<String>::new());
}

#[test]
fn fan_edits_and_server_copies_are_not_other_versions() {
    let (_d, c) = db();
    album(&c, "a", "Artist", "Album", true, Some("album"), &[], None);
    song(&c, "1", "a", "Album", "Artist", "Song", 200_000, None, 1);
    album(&c, "b", "Artist", "Tape", false, None, &[], None);
    song(&c, "2", "b", "Tape", "Artist", "Song (Fan Edit)", 200_000, None, 1);
    song(&c, "3", "b", "Tape", "Artist", "Song - Snippet", 30_000, None, 2);
    c.execute("INSERT INTO file_tags (part_key, status, flags, scanned_at, ver) VALUES ('/p/2', 'ok', 'fanedit', 1, 9)", []).unwrap();
    // The same album on another server.
    album(&c, "a2", "Artist", "Album", true, Some("album"), &[], None);
    song(&c, "1b", "a2", "Album", "Artist", "Song", 200_000, None, 1);
    c.execute("INSERT INTO dupes (id, kind, canon) VALUES ('1b', 'track', '1')", []).unwrap();
    refresh(&c);
    assert_eq!(versions::other_ids(&c, "1", false).unwrap(), ["3"]);
    let mut shown = versions::other_ids(&c, "1", true).unwrap();
    shown.sort();
    assert_eq!(shown, ["2", "3"], "with fan edits shown");
    let mut t = crate::db::queries::tracks_by_ids(&c, &["1".into()]).unwrap();
    versions::annotate(&c, &mut t, false).unwrap();
    assert_eq!((t[0].other_versions, t[0].song_status), (Some(1), Some(SongStatus::Released)));
    // The panel: released first, the current one flagged.
    let panel = versions::versions_of(&c, "3", false).unwrap();
    assert_eq!(panel.iter().map(|v| v.track.id.as_str()).collect::<Vec<_>>(), ["1", "3"]);
    assert!(panel[1].current && !panel[0].current);
}

#[test]
fn versions_refresh_per_artist() {
    let (_d, c) = db();
    album(&c, "a", "Alpha", "Album", true, Some("album"), &[], None);
    song(&c, "a1", "a", "Album", "Alpha", "Song", 200_000, None, 1);
    album(&c, "b", "Beta", "Record", true, Some("album"), &[], None);
    song(&c, "b1", "b", "Record", "Beta", "Tune", 200_000, None, 1);
    refresh(&c);
    // A new demo for Alpha arrives; only Alpha's rows are redone.
    album(&c, "a-demo", "Alpha", "Demos", true, Some("album"), &["demo"], None);
    song(&c, "a2", "a-demo", "Demos", "Alpha", "Song (Demo)", 180_000, None, 1);
    c.execute("UPDATE versions SET group_key = 'stale' WHERE track_id = 'b1'", []).unwrap();
    crate::identity::index::refresh_artists(&c, &["ar:Alpha".into()]).unwrap();
    versions::refresh_artists(&c, &["ar:Alpha".into()]).unwrap();
    assert_eq!(group_of(&c, "a1"), group_of(&c, "a2"));
    assert_eq!(status_of(&c, "a2"), SongStatus::Unreleased);
    assert_eq!(group_of(&c, "b1"), "stale", "other artists are left alone");
}

#[test]
fn search_shows_the_released_version_first() {
    let (_d, c) = db();
    album(&c, "rel", "Artist", "Album", true, Some("album"), &[], None);
    song(&c, "r1", "rel", "Album", "Artist", "Song", 200_000, None, 1);
    album(&c, "tape", "Artist", "Tape", false, None, &[], Some(2030));
    song(&c, "t1", "tape", "Tape", "Artist", "Song (Demo)", 190_000, None, 1);
    song(&c, "t2", "tape", "Tape", "Artist", "Song - Snippet", 30_000, None, 2);
    song(&c, "t3", "tape", "Tape", "Artist", "Elsewhere", 100_000, None, 3);
    refresh(&c);
    let get = |id: &str| crate::db::queries::tracks_by_ids(&c, &[id.into()]).unwrap().remove(0);
    // The newest version ranks first; the released one must still lead.
    let mut out = SearchResults {
        songs: vec![get("t1"), get("t3"), get("t2")],
        top: Some(SearchHit { kind: SearchKind::Track, track: Some(get("t2")), artist: None, album: None, playlist: None }),
        ..Default::default()
    };
    versions::group_search(&c, &mut out, false).unwrap();
    assert_eq!(out.songs.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["r1", "t3"]);
    assert_eq!(out.songs[0].other_versions, Some(2));
    let mut under: Vec<&str> = out.versions["r1"].iter().map(|t| t.id.as_str()).collect();
    under.sort();
    assert_eq!(under, ["t1", "t2"]);
    assert_eq!(out.top.unwrap().track.unwrap().id, "r1", "the top result too");
    // Nothing released: the hit stays as it is.
    let mut out = SearchResults { songs: vec![get("t3")], ..Default::default() };
    versions::group_search(&c, &mut out, false).unwrap();
    assert_eq!(out.songs[0].id, "t3");
    assert!(out.versions.is_empty());
}

#[test]
fn a_marked_released_version_leads_search_only_once_marked() {
    let (_d, c) = db();
    album(&c, "tape", "Artist", "Tape", false, None, &[], None);
    song(&c, "t1", "tape", "Tape", "Artist", "Song", 190_000, None, 1);
    album(&c, "leak", "Artist", "Leak", false, None, &[], None);
    song(&c, "l1", "leak", "Leak", "Artist", "Song (Leak)", 190_000, None, 1);
    refresh(&c);
    assert_eq!(status_of(&c, "t1"), SongStatus::ProbablyUnreleased);
    let mut m = HashMap::new();
    m.insert("album:tape".to_string(), Mark::Released);
    release::set_marks(m);
    refresh(&c);
    assert_eq!(status_of(&c, "t1"), SongStatus::Released);
    assert_eq!(status_of(&c, "l1"), SongStatus::ProbablyAlternate);
    release::set_marks(HashMap::new());
}

// ------------------------------------------------------------------ sections

fn al(id: &str, section: ReleaseSection) -> Album {
    Album {
        id: id.into(),
        title: id.into(),
        sort_title: id.into(),
        artist_id: "ar".into(),
        artist_title: "Artist".into(),
        year: None,
        thumb: None,
        format: None,
        subformats: Vec::new(),
        section,
        genres: Vec::new(),
        track_count: 1,
        added_at: 0,
        view_count: 0,
        last_viewed_at: None,
        tint: None,
        released: None,
        studio: None,
        matched: None,
        styles: Vec::new(),
        moods: Vec::new(),
        mbid: None,
    }
}

fn placed(id: &str, section: ReleaseSection, status: ReleaseStatus, version: Option<&str>) -> Placed {
    Placed { album: al(id, section), status, version: version.map(str::to_string), custom: None }
}

fn layout(out: &[crate::db::types::ArtistSection]) -> Vec<(String, Vec<String>)> {
    out.iter()
        .map(|s| (s.custom.clone().unwrap_or_else(|| release::section_key(s.section).to_string()), s.albums.iter().map(|a| a.id.clone()).collect()))
        .collect()
}

#[test]
fn sections_by_release_type() {
    use ReleaseSection as R;
    use ReleaseStatus as S;
    // One fixture per RELEASETYPE, all released.
    let types = [
        (None, &[][..], R::Albums),
        (Some("album"), &[][..], R::Albums),
        (Some("ep"), &[][..], R::SinglesEps),
        (Some("single"), &[][..], R::SinglesEps),
        (Some("album"), &["live"][..], R::Live),
        (Some("album"), &["compilation"][..], R::Compilations),
        (Some("album"), &["soundtrack"][..], R::Soundtracks),
        (Some("album"), &["remix"][..], R::Remixes),
        (Some("album"), &["mixtape/street"][..], R::Mixtapes),
    ];
    for (format, subs, want) in types {
        assert_eq!(release::release_section(format, subs), want, "{format:?} {subs:?}");
    }
    let out = classify(vec![
        placed("lp", R::Albums, S::Released, None),
        placed("live", R::Live, S::Released, None),
        placed("ep", R::SinglesEps, S::Released, None),
        placed("tape", R::Mixtapes, S::Released, None),
    ]);
    // Page order: Albums, Singles & EPs, Mixtapes, Unreleased, Live...
    assert_eq!(layout(&out), [("albums".into(), vec!["lp".to_string()]), ("singlesEps".into(), vec!["ep".into()]), ("mixtapes".into(), vec!["tape".into()]), ("live".into(), vec!["live".into()])]);
}

#[test]
fn unreleased_and_demo_sections() {
    use ReleaseSection as R;
    use ReleaseStatus as S;
    // album;demo, a probable leak and a confirmed one: one Unreleased section.
    let out = classify(vec![
        placed("lp", R::Albums, S::Released, None),
        placed("demo-type", R::Demos, S::TaggedUnreleased, None),
        placed("guess", R::Albums, S::ProbablyUnreleased, None),
        placed("marked", R::Live, S::ConfirmedUnreleased, None),
    ]);
    assert_eq!(layout(&out), [("albums".into(), vec!["lp".to_string()]), ("unreleased".into(), vec!["demo-type".into(), "guess".into(), "marked".into()])]);
    assert!(out[1].albums.iter().all(|a| a.section == R::Unreleased));
    // Only demo versions: still one section.
    let out = classify(vec![placed("d1", R::Albums, S::TaggedUnreleased, Some("demo")), placed("d2", R::Albums, S::TaggedUnreleased, Some("Demo"))]);
    assert_eq!(layout(&out), [("unreleased".into(), vec!["d1".to_string(), "d2".into()])]);
    // Demos and other unreleased material: Demos gets its own.
    let out = classify(vec![placed("d1", R::Albums, S::TaggedUnreleased, Some("demo")), placed("leak", R::Albums, S::TaggedUnreleased, Some("leak"))]);
    assert_eq!(layout(&out), [("unreleased".into(), vec!["leak".to_string()]), ("demos".into(), vec!["d1".into()])]);
    // A demo type that a mark makes released goes back to Albums.
    let out = classify(vec![placed("x", R::Demos, S::Released, None)]);
    assert_eq!(layout(&out), [("albums".into(), vec!["x".to_string()])]);
    // Plex missed it: released.
    let out = classify(vec![placed("pm", R::Albums, S::PlexMissed, None)]);
    assert_eq!(layout(&out)[0].0, "albums");
}

#[test]
fn custom_sections_take_their_albums() {
    use ReleaseSection as R;
    use ReleaseStatus as S;
    let mut a = placed("a", R::Albums, S::Released, None);
    a.custom = Some("Zeta".into());
    let mut b = placed("b", R::Albums, S::ProbablyUnreleased, None);
    b.custom = Some("alpha".into());
    let out = classify(vec![a, b, placed("c", R::Albums, S::Released, None)]);
    assert_eq!(layout(&out), [("albums".into(), vec!["c".to_string()]), ("alpha".into(), vec!["b".into()]), ("Zeta".into(), vec!["a".into()])]);
}

#[test]
fn the_artist_page_uses_stored_statuses() {
    let (_d, c) = db();
    album(&c, "lp", "Artist", "Album", true, Some("album"), &[], Some(2019));
    song(&c, "1", "lp", "Album", "Artist", "One", 200_000, None, 1);
    album(&c, "tape", "Artist", "Tape", false, None, &[], None);
    song(&c, "2", "tape", "Tape", "Artist", "Two", 200_000, None, 1);
    album(&c, "demo", "Artist", "Demos", true, Some("album"), &["demo"], None);
    song(&c, "3", "demo", "Demos", "Artist", "Three", 200_000, None, 1);
    album_tag(&c, "demo", Some("demo"), None, None);
    c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('ar:Artist', 'Artist', 'Artist')", []).unwrap();
    refresh(&c);
    let mut page = crate::db::queries::artist_page(&c, "ar:Artist").unwrap();
    let mut custom = HashMap::new();
    custom.insert("lp".to_string(), "Favourites".to_string());
    super::sections::apply(&c, &mut page, &custom).unwrap();
    assert_eq!(layout(&page.sections), [("unreleased".into(), vec!["tape".to_string()]), ("demos".into(), vec!["demo".into()]), ("Favourites".into(), vec!["lp".into()])]);
}

// ------------------------------------------------------------------ eras

#[test]
fn archive_dates_in_the_pinned_formats() {
    let d = |s: &str| parse_date(s).map(|d| d.text());
    assert_eq!(d("2019").as_deref(), Some("2019"));
    assert_eq!(d("2019-05").as_deref(), Some("2019-05"));
    assert_eq!(d(" 2019-05-31 ").as_deref(), Some("2019-05-31"));
    assert_eq!(d("2020-02-29").as_deref(), Some("2020-02-29"), "leap year");
    for bad in ["2019-02-29", "2019-13", "2019-00", "2019-04-31", "19", "2019/05", "Summer 2019", "2019-5", "2019-05-1", "", "2019-05-10-1"] {
        assert_eq!(parse_date(bad), None, "{bad}");
    }
    assert!(parse_date("2019").unwrap() < parse_date("2019-01").unwrap());
    assert!(parse_date("2019-12-31").unwrap().key() < parse_date("2020").unwrap().key());
}

fn era_library(c: &Connection) {
    c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('ar:Artist', 'Artist', 'Artist')", []).unwrap();
    album(c, "glass", "Artist", "Glass Season", true, Some("album"), &[], Some(2019));
    c.execute("UPDATE albums SET view_count = 50 WHERE id = 'glass'", []).unwrap();
    song(c, "g1", "glass", "Glass Season", "Artist", "Undertow", 200_000, None, 1);
    song(c, "g2", "glass", "Glass Season", "Artist", "Kite", 200_000, None, 2);
    tags(c, "g1", None, Some("Glass Season"), None, None);
    tags(c, "g2", None, Some("glass season"), None, None);
    album(c, "build1", "Artist", "Glass Build v1", false, None, &[], None);
    song(c, "b1", "build1", "Glass Build v1", "Artist", "Undertow (Demo)", 190_000, None, 1);
    tags(c, "b1", Some("demo"), Some("Glass Season"), Some("2018-11-02"), None);
    album(c, "build2", "Artist", "Glass Build v2", false, None, &[], None);
    song(c, "b2", "build2", "Glass Build v2", "Artist", "Kite (v2)", 190_000, None, 1);
    tags(c, "b2", Some("session"), Some("Glass Season"), Some("2019-02"), None);
    c.execute("INSERT OR REPLACE INTO file_tags (part_key, status, version, era, date, source, scanned_at, ver) VALUES ('/p/b2', 'ok', 'session', 'Glass Season', '2019-02', 'Studio bounce', 1, 9)", []).unwrap();
    album_tag(c, "build2", Some("session"), None, Some("Build v2"));
    album(c, "ferry", "Artist", "Night Ferry", true, Some("ep"), &[], Some(2021));
    song(c, "f1", "ferry", "Night Ferry", "Artist", "Harbor", 200_000, None, 1);
    tags(c, "f1", None, Some("Night Ferry"), Some("Summer 2021"), None);
    album(c, "loose", "Artist", "Loose", false, None, &[], None);
    song(c, "x1", "loose", "Loose", "Artist", "Nowhere", 200_000, None, 1);
    tags(c, "x1", Some("leak"), Some("Lost Tapes"), None, None);
    refresh(c);
}

#[test]
fn eras_group_date_and_sort() {
    let (_d, c) = db();
    era_library(&c);
    let eras = eras::artist_eras(&c, "ar:Artist", false).unwrap();
    assert_eq!(eras.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Glass Season", "Night Ferry", "Lost Tapes"]);
    let glass = &eras[0];
    assert_eq!((glass.start.as_deref(), glass.end.as_deref()), (Some("2018-11-02"), Some("2019-02")), "from ARCHIVE_DATE");
    assert_eq!((glass.track_count, glass.album_count), (4, 3), "released and unreleased, one spelling");
    assert_eq!(glass.cover_album_id.as_deref(), Some("glass"), "the most played album");
    // An unreadable date: the album years.
    assert_eq!((eras[1].start.as_deref(), eras[1].end.as_deref()), (Some("2021"), Some("2021")));
    // No dates or years: last.
    assert_eq!(eras[2].start, None);
}

#[test]
fn the_era_page_lists_builds_newest_first() {
    let (_d, c) = db();
    era_library(&c);
    let page = eras::era_page(&c, "ar:Artist", "GLASS SEASON", false).unwrap();
    assert_eq!(page.era.name, "Glass Season");
    let order: Vec<&str> = page.groups.iter().map(|g| g.album.id.as_str()).collect();
    assert_eq!(order, ["build2", "glass", "build1"], "2019-02, then 2019, then 2018-11");
    assert_eq!(page.groups[0].edition.as_deref(), Some("Build v2"));
    assert_eq!(page.groups[0].date.as_deref(), Some("2019-02"));
    assert!(eras::era_page(&c, "ar:Artist", "Nope", false).is_err());
}

#[test]
fn the_album_banner_is_for_unreleased_builds() {
    let (_d, c) = db();
    era_library(&c);
    let b = eras::album_archive(&c, "build2").unwrap().unwrap();
    assert_eq!(b.era.as_deref(), Some("Glass Season"));
    assert_eq!(b.date.as_deref(), Some("2019-02"));
    assert_eq!(b.edition.as_deref(), Some("Build v2"));
    assert_eq!(b.source.as_deref(), Some("Studio bounce"));
    assert_eq!(eras::album_archive(&c, "glass").unwrap(), None, "released albums have none");
}

#[test]
fn era_filter_covers_every_section() {
    let (_d, c) = db();
    era_library(&c);
    let eras = eras::artist_eras(&c, "ar:Artist", false).unwrap();
    let glass = &eras[0];
    // The filter keeps released and unreleased albums of the era alike.
    for id in ["glass", "build1", "build2"] {
        assert!(glass.album_ids.contains(&id.to_string()), "{id}");
    }
    assert!(!glass.album_ids.contains(&"ferry".to_string()));
    let mut page = crate::db::queries::artist_page(&c, "ar:Artist").unwrap();
    super::sections::apply(&c, &mut page, &HashMap::new()).unwrap();
    let kept: Vec<(String, Vec<String>)> = layout(&page.sections)
        .into_iter()
        .map(|(k, ids)| {
            let mut ids: Vec<String> = ids.into_iter().filter(|i| glass.album_ids.contains(i)).collect();
            ids.sort();
            (k, ids)
        })
        .filter(|(_, ids)| !ids.is_empty())
        .collect();
    assert_eq!(kept, [("albums".into(), vec!["glass".to_string()]), ("unreleased".into(), vec!["build1".into(), "build2".into()])]);
}

// ------------------------------------------------------------------ lineage

#[test]
fn lineage_runs_oldest_first_with_stages_and_undated_last() {
    use super::lineage::{Stage, lineage_of};
    let (_d, c) = db();
    album(&c, "rel", "Artist", "Glass", true, Some("album"), &[], Some(2019));
    song(&c, "r", "rel", "Glass", "Artist", "Glasshouse", 214_000, None, 1);
    album(&c, "b1", "Artist", "Build v1", false, None, &[], None);
    song(&c, "d", "b1", "Build v1", "Artist", "Glasshouse (Demo)", 198_000, None, 1);
    tags(&c, "d", Some("demo"), Some("Glass"), Some("2018-09-14"), None);
    album(&c, "b2", "Artist", "Build v2", false, None, &[], None);
    song(&c, "s", "b2", "Build v2", "Artist", "Glasshouse (Session)", 220_000, None, 1);
    tags(&c, "s", Some("session"), Some("Glass"), Some("2019-02-11"), None);
    album(&c, "lv", "Artist", "Live at Home", true, Some("album"), &["live"], Some(2022));
    song(&c, "l", "lv", "Live at Home", "Artist", "Glasshouse", 252_000, None, 1);
    album(&c, "rm", "Artist", "Glass (Remastered)", true, Some("album"), &[], Some(2024));
    song(&c, "m", "rm", "Glass (Remastered)", "Artist", "Glasshouse (Remastered 2024)", 214_000, None, 1);
    album(&c, "tk", "Artist", "Takes", false, None, &[], None);
    song(&c, "p", "tk", "Takes", "Artist", "Glasshouse (Piano Demo)", 167_000, None, 1);
    tags(&c, "p", Some("demo"), None, None, None);
    album(&c, "lk", "Artist", "Paper", false, None, &[], None);
    song(&c, "k", "lk", "Paper", "Artist", "Glasshouse", 55_000, None, 1);
    tags(&c, "k", Some("leak"), None, Some("2023-08-19"), None);
    refresh(&c);

    let l = lineage_of(&c, "s", false).unwrap();
    assert_eq!(l.title, "Glasshouse");
    let order: Vec<(&str, Stage)> = l.nodes.iter().map(|n| (n.item.track.id.as_str(), n.stage)).collect();
    assert_eq!(
        order,
        [
            ("d", Stage::Demo),
            ("r", Stage::Release),
            ("s", Stage::Session),
            ("l", Stage::Live),
            ("k", Stage::Leak),
            ("m", Stage::Remaster),
            ("p", Stage::Demo),
        ]
    );
    assert!(l.nodes.iter().find(|n| n.item.track.id == "s").unwrap().item.current);
    assert_eq!(l.nodes.last().unwrap().when, None, "undated goes last");
    // Only released versions can be shared.
    let share: Vec<&str> = l.nodes.iter().filter(|n| n.shareable).map(|n| n.item.track.id.as_str()).collect();
    assert_eq!(share, ["r", "l", "m"]);
    // Any version opens the same lineage.
    assert_eq!(lineage_of(&c, "p", false).unwrap().nodes.len(), 7);
}

#[test]
fn lineage_of_an_unknown_song_is_not_found() {
    let (_d, c) = db();
    assert!(super::lineage::lineage_of(&c, "nope", false).is_err());
}

#[test]
fn same_name_songs_with_different_featured_artists_are_different_songs() {
    use super::versions::{Song, group};
    let s = |id: &str, title: &str, album: &str| Song {
        id: id.into(),
        artist_id: "ar1".into(),
        album_id: format!("al-{album}"),
        title: title.into(),
        album_title: album.into(),
        album_artist: "Northlight".into(),
        ..Default::default()
    };
    let songs = vec![
        s("1", "Home (feat. Ana Reed)", "First Light"),
        s("2", "Home (feat. Cole Varga)", "Second Wind"),
        s("3", "Home (feat. Ana Reed) [Demo]", "Sessions"),
        s("4", "Home ft. Cole Varga", "Live"),
        s("5", "Lanterns (feat. Ana Reed)", "First Light"),
        s("6", "Lanterns (Demo)", "Sessions"),
    ];
    let g = group(&songs);
    assert_ne!(g[0], g[1], "different featured artists: different songs");
    assert_eq!(g[0], g[2], "same featured artist: a version");
    assert_eq!(g[1], g[3], "a shared featured artist joins");
    assert_eq!(g[4], g[5], "no featured artist joins its song");
    // A recording id links them whatever the credits say.
    let mut linked = songs.clone();
    linked[0].recording_mbid = Some("rec-1".into());
    linked[1].recording_mbid = Some("rec-1".into());
    let g = group(&linked);
    assert_eq!(g[0], g[1]);
}

#[test]
fn live_is_a_version_note_not_a_word_of_the_name() {
    use super::lineage::{Stage, lineage_of};
    let (_d, c) = db();
    album(&c, "rel", "Artist", "Live Through Glass", true, Some("album"), &[], Some(2019));
    song(&c, "r", "rel", "Live Through Glass", "Artist", "Live Forever", 214_000, None, 1);
    album(&c, "bt", "Artist", "Bootleg Tapes", true, Some("album"), &[], Some(2021));
    song(&c, "l", "bt", "Bootleg Tapes", "Artist", "Live Forever (Live at Home)", 252_000, None, 1);
    album(&c, "bs", "Artist", "B-Sides", true, Some("album"), &[], Some(2022));
    song(&c, "x", "bs", "B-Sides", "Artist", "Live Forever - Live", 250_000, None, 1);
    refresh(&c);
    let l = lineage_of(&c, "r", false).unwrap();
    let stage = |id: &str| l.nodes.iter().find(|n| n.item.track.id == id).map(|n| n.stage);
    assert_eq!(stage("r"), Some(Stage::Release), "\"Live\" in the name and the album title isn't a live version");
    assert_eq!(stage("l"), Some(Stage::Live));
    assert_eq!(stage("x"), Some(Stage::Live));
}
