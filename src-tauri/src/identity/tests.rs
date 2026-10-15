//! Song identity tests: the folding fixtures, tier order, the length slack,
//! the unreleased rule, a two-server library, and index speed.

use std::collections::HashMap;
use std::time::Instant;

use rusqlite::{Connection, params};

use super::fold::{self, fold_artist, fold_title};
use super::index::{self, Candidate};
use super::{ALBUM_SLACK_MS, Context, MatchKind, SONG_SLACK_MS, SongKey, Tier, compare};
use crate::db::schema;

fn key(artist: &str, title: &str, album: &str, ms: u64) -> SongKey {
    SongKey::from_names(artist, title, album, ms)
}

// ------------------------------------------------------------------ folding

#[test]
fn folding_basics() {
    assert_eq!(fold::fold_text("BEYONCÉ"), "beyonce");
    assert_eq!(fold::fold_text("ＡＢＣ"), "abc", "full-width forms fold");
    assert_eq!(fold_artist("The Beatles"), "beatles");
    assert_eq!(fold_artist("The The"), "the");
    assert_eq!(fold_artist("Simon & Garfunkel"), fold_artist("Simon and Garfunkel"));
    assert_eq!(fold_artist("A feat. B"), "a");
    assert_eq!(fold_artist("A ft. B"), "a");
    assert_eq!(fold_artist("A (feat. B)"), "a");
    assert_eq!(fold_artist("R.E.M."), fold_artist("REM"));
    assert_eq!(fold_artist("Various Artists"), "", "says nothing about who made it");
    assert_eq!(fold_title("Don't Stop").title, fold_title("Dont Stop").title);
    let t = fold_title("Song (Demo) [v2]");
    assert_eq!((t.title.as_str(), t.version.as_str()), ("song", "demo; v2"), "kept, not thrown away");
    assert_eq!(fold_title("Song (Remastered 2011)").version, "remastered 2011");
    assert!(fold::version_core("remastered 2011").is_empty(), "a remaster is the same recording");
    assert_eq!(fold::version_core("remastered 2011; demo"), vec!["demo".to_string()]);
    assert_eq!(fold::fold_album("The Album (Deluxe Edition)"), "the album");
    // A title that is only a version note keeps it as the name.
    assert_eq!(fold_title("(Demo)").title, "demo");
    // Scripts where a mark changes the letter keep it.
    assert_ne!(fold::fold_text("が"), fold::fold_text("か"));
}

/// 40 titles that must fold to the plain song, with the kind of match each
/// makes against it.
const MUST_MATCH: [(&str, MatchKind); 40] = {
    use MatchKind::{OtherVersion as O, SameRecording as S};
    [
        ("Song (Remastered 2011)", S),
        ("Song - Remastered 2011", S),
        ("Song - 2011 Remaster", S),
        ("Song (2009 Remastered Version)", S),
        ("Song [Remastered]", S),
        ("Song (feat. Someone)", S),
        ("Song (ft. Someone)", S),
        ("Song feat. Someone", S),
        ("Song [feat. Someone & Other]", S),
        ("Song (featuring Someone)", S),
        ("Song (with Someone)", S),
        ("Song (prod. Someone)", S),
        ("Song (Explicit)", S),
        ("Song [Clean]", S),
        ("Song (Album Version)", S),
        ("Song - Single Version", S),
        ("Song (Mono)", S),
        ("Song - Stereo", S),
        ("Song (Bonus Track)", S),
        ("Song (Deluxe Edition)", S),
        ("Song (Original Mix)", S),
        ("Song - Remastered", S),
        ("SONG", S),
        ("Sóng", S),
        ("Song!", S),
        ("Song (feat. X) [Remastered 2011]", S),
        ("Song \u{2013} Remastered 2011", S),
        ("Song (25th Anniversary Edition)", S),
        ("Song (Demo)", O),
        ("Song - Snippet", O),
        ("Song [v2]", O),
        ("Song (Live)", O),
        ("Song (Live at Wembley 1986)", O),
        ("Song - Live", O),
        ("Song (Acoustic)", O),
        ("Song (Instrumental)", O),
        ("Song (Radio Edit)", O),
        ("Song (Extended Mix)", O),
        ("Song (Someone Remix)", O),
        ("Song (Remastered Demo)", O),
    ]
};

#[test]
fn forty_suffixes_fold_to_the_song() {
    let base = key("Artist", "Song", "Album", 200_000);
    for (title, kind) in MUST_MATCH {
        let k = key("Artist", title, "Other Album", 200_000);
        assert_eq!(k.title_fold, "song", "{title}");
        let m = compare(&base, &k, Context::Single).unwrap_or_else(|| panic!("{title} should match"));
        assert_eq!(m.kind, kind, "{title}: {m:?}");
        assert_eq!(compare(&k, &base, Context::Single), Some(m), "{title}: either way round");
    }
}

/// 20 pairs that must never be called the same song, not even another
/// version of it. (artist, title, album) each.
type Song3 = (&'static str, &'static str, &'static str);
const MUST_NOT_MERGE: [(Song3, Song3); 20] = [
    (("A", "Song Pt. 1", "X"), ("A", "Song Pt. 2", "X")),
    (("A", "Song (Part 1)", "X"), ("A", "Song (Part 2)", "X")),
    (("A", "Song Pt. 1", "X"), ("A", "Song", "X")),
    (("A", "Song Part II", "X"), ("A", "Song Part 2", "X")),
    (("A", "Song", "X"), ("A", "Song (Interlude)", "X")),
    (("A", "Song", "X"), ("A", "Song (Reprise)", "X")),
    (("A", "Intro", "First"), ("A", "Intro", "Second")),
    (("A", "Outro", "First"), ("A", "Outro", "Second")),
    (("A", "Interlude", "First"), ("A", "Interlude", "Second")),
    (("A", "Skit", "First"), ("A", "Skit", "Second")),
    (("A", "Song", "X"), ("B", "Song", "X")),
    (("A", "Song", "X"), ("A", "Song 2", "X")),
    (("A", "Song", "X"), ("A", "Songs", "X")),
    (("A", "Song", "X"), ("A", "The Song", "X")),
    (("A", "Song (Pt. 1)", "X"), ("A", "Song (Pt. 2)", "X")),
    (("A", "Love", "X"), ("A", "Love Me", "X")),
    (("A", "Untitled", "First"), ("A", "Untitled", "Second")),
    (("A", "がっこう", "X"), ("A", "かっこう", "X")),
    (("A", "Song", "X"), ("A", "Song (Intro)", "X")),
    (("A", "Song", "X"), ("A & B", "Song", "X")),
];

#[test]
fn twenty_pairs_never_merge() {
    for ((a1, t1, al1), (a2, t2, al2)) in MUST_NOT_MERGE {
        let x = key(a1, t1, al1, 200_000);
        let y = key(a2, t2, al2, 200_000);
        assert_eq!(compare(&x, &y, Context::Single), None, "{a1} {t1} / {a2} {t2}");
        assert_eq!(compare(&y, &x, Context::Single), None, "{a2} {t2} / {a1} {t1}");
    }
    // Generic titles do match on the same album.
    assert!(compare(&key("A", "Intro", "First", 60_000), &key("A", "Intro", "First (Deluxe)", 60_000), Context::Single).is_some());
}

#[test]
fn part_two_spellings_follow_the_switch() {
    const { assert!(!fold::ROMAN_PART_EQUALS_NUMBER, "different until decided") };
    assert_eq!(fold_title("Song Pt. 2").title, fold_title("Song Part 2").title);
    assert_eq!(fold_title("Song, Pt. II").title, "song part ii");
}

// ------------------------------------------------------------------ the matcher

#[test]
fn tiers_come_in_order() {
    let mut a = key("Artist", "Song", "Album", 200_000);
    a.isrc = Some("USRC17607839".into());
    a.recording_mbid = Some("rec-1".into());
    a.plex_track_mbid = Some("trk-1".into());
    // Ids win even over names that don't match at all.
    let mut b = key("Someone Else", "Other Title", "", 0);
    b.isrc = a.isrc.clone();
    assert_eq!(compare(&a, &b, Context::Single).unwrap().tier, Tier::Isrc);
    b.isrc = None;
    b.recording_mbid = a.recording_mbid.clone();
    assert_eq!(compare(&a, &b, Context::Single).unwrap().tier, Tier::RecordingMbid);
    b.recording_mbid = None;
    b.plex_track_mbid = a.plex_track_mbid.clone();
    assert_eq!(compare(&a, &b, Context::Single).unwrap().tier, Tier::PlexTrackMbid);
    b.plex_track_mbid = None;
    assert_eq!(compare(&a, &b, Context::Single), None);

    let c = key("Artist", "Song", "Album", 201_000);
    let m = compare(&a, &c, Context::Single).unwrap();
    assert_eq!((m.tier, m.kind), (Tier::Text, MatchKind::SameRecording));
    let d = key("Artist", "Song (Demo)", "Album", 201_000);
    let m2 = compare(&a, &d, Context::Single).unwrap();
    assert_eq!((m2.tier, m2.kind), (Tier::TextOtherVersion, MatchKind::OtherVersion));
    assert!(m.confidence > m2.confidence);
    assert!(Tier::Isrc < Tier::RecordingMbid && Tier::PlexTrackMbid < Tier::Text && Tier::Text < Tier::TextOtherVersion);
}

#[test]
fn length_slack_and_missing_lengths() {
    assert_eq!((SONG_SLACK_MS, ALBUM_SLACK_MS), (3_000, 2_000));
    let a = key("Artist", "Song", "", 200_000);
    let b = key("Artist", "Song", "", 202_500);
    assert_eq!(compare(&a, &b, Context::Single).unwrap().kind, MatchKind::SameRecording);
    assert_eq!(compare(&a, &b, Context::MatchedAlbum).unwrap().kind, MatchKind::OtherVersion);
    let edge = key("Artist", "Song", "", 200_000 + SONG_SLACK_MS);
    assert_eq!(compare(&a, &edge, Context::Single).unwrap().kind, MatchKind::SameRecording);
    // A live cut that runs long is another version at most.
    let long = key("Artist", "Song", "", 260_000);
    assert_eq!(compare(&a, &long, Context::Single).unwrap().kind, MatchKind::OtherVersion);
    // No length: names only, less sure.
    let none = key("Artist", "Song", "", 0);
    let m = compare(&a, &none, Context::Single).unwrap();
    assert_eq!(m.kind, MatchKind::SameRecording);
    assert!(m.confidence < compare(&a, &b, Context::Single).unwrap().confidence);
}

#[test]
fn two_recording_ids_are_never_the_same_recording() {
    let mut a = key("Artist", "Song", "", 200_000);
    let mut b = a.clone();
    a.recording_mbid = Some("rec-1".into());
    b.recording_mbid = Some("rec-2".into());
    assert_eq!(compare(&a, &b, Context::Single).unwrap().kind, MatchKind::OtherVersion);
}

#[test]
fn the_track_artist_is_a_second_name() {
    let mut a = key("Remixer", "Song", "Remixes", 200_000);
    a.album_artist_fold = Some(a.artist_fold.clone());
    a.artist_fold = fold_artist("Original Artist");
    let b = key("Original Artist", "Song", "Album", 200_000);
    assert!(compare(&a, &b, Context::Single).is_some());
}

#[test]
fn unreleased_songs_never_match() {
    let a = key("Artist", "Song", "", 200_000);
    let mut b = a.clone();
    b.unreleased = true;
    b.isrc = Some("USRC17607839".into());
    let mut a2 = a.clone();
    a2.isrc = b.isrc.clone();
    assert_eq!(compare(&a2, &b, Context::Single), None);
    assert_eq!(compare(&b, &a2, Context::Single), None);
}

// ------------------------------------------------------------------ the index

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
    schema::migrate(&mut c).unwrap();
    (dir, c)
}

/// An album Plex matched (so it counts as released unless tagged).
fn album(c: &Connection, id: &str, artist: &str, title: &str, subformats: &str) {
    let src = id.split(':').next().unwrap();
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, matched, subformats)
         VALUES (?1, ?2, ?2, ?3, ?4, 1, ?5, 1, ?6)",
        params![id, title, format!("{src}:ar:{artist}"), artist, src, subformats],
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn song(c: &Connection, id: &str, album_id: &str, album: &str, artist: &str, own: Option<&str>, title: &str, ms: i64, mbid: Option<&str>) {
    let src = id.split(':').next().unwrap();
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_artist, duration_ms,
           part_key, src, mbid)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![id, title, album_id, album, format!("{src}:ar:{artist}"), artist, own, ms, format!("/p/{id}"), src, mbid],
    )
    .unwrap();
}

fn key_of(c: &Connection, id: &str) -> SongKey {
    let t = crate::db::queries::tracks_by_ids(c, &[id.to_string()]).unwrap().remove(0);
    let tags = c
        .query_row("SELECT isrc, recording_mbid FROM file_tags WHERE part_key = ?1", [&t.part_key], |r| {
            Ok(crate::tags::FileTags { isrc: r.get(0)?, recording_mbid: r.get(1)?, ..Default::default() })
        })
        .ok();
    let unreleased: bool = c
        .query_row("SELECT max(unreleased) FROM song_index WHERE track_id = ?1", [id], |r| r.get::<_, i64>(0))
        .map(|v| v != 0)
        .unwrap_or(false);
    SongKey::from_track(&t, tags.as_ref(), unreleased)
}

const A: &str = "aaaaaaaa";
const B: &str = "bbbbbbbb";

fn on(tag: &'static str) -> impl Fn(&str) -> bool {
    move |id: &str| id.starts_with(tag)
}

/// Server A has 50 songs; server B has 30 of them (60%), spelled and cut a
/// little differently, plus 20 of its own that only look alike.
#[test]
fn two_servers_overlapping_sixty_percent() {
    let (_d, c) = db();
    let artists = ["Alpha", "The Bravo Band", "Charlie & Delta", "Echo", "Foxtrot"];
    for (ai, artist) in artists.iter().enumerate() {
        album(&c, &format!("{A}:al{ai}"), artist, &format!("Record {ai}"), "[]");
        album(&c, &format!("{B}:al{ai}"), &artist.to_uppercase(), &format!("Record {ai} (Deluxe Edition)"), "[]");
    }
    let mut expected: HashMap<String, String> = HashMap::new();
    for i in 0..50 {
        let ai = i % 5;
        let artist = artists[ai];
        let title = format!("Tune Number {i}");
        let ms = 180_000 + i as i64 * 1_000;
        let a_id = format!("{A}:{i}");
        let mbid = (i % 7 == 0).then(|| format!("mbid-{i}"));
        song(&c, &a_id, &format!("{A}:al{ai}"), &format!("Record {ai}"), artist, None, &title, ms, mbid.as_deref());
        if i < 30 {
            // The copy on B, spelled another way.
            let b_artist = match i % 3 {
                0 => artist.to_uppercase(),
                1 => artist.replace("The ", "").replace('&', "and"),
                _ => artist.to_string(),
            };
            let b_title = match i % 4 {
                0 => format!("{title} (Remastered 2011)"),
                1 => format!("{} (feat. Guest)", title.to_lowercase()),
                2 => format!("{title} - Album Version"),
                _ => title.clone(),
            };
            let b_ms = ms + [0, 1_500, -2_900, 800][i % 4];
            let b_id = format!("{B}:{i}");
            song(&c, &b_id, &format!("{B}:al{ai}"), &format!("Record {ai} (Deluxe Edition)"), &b_artist.to_uppercase(), None, &b_title, b_ms, mbid.as_deref());
            expected.insert(b_id, a_id);
        }
    }
    // B's own: other titles, a part 2, another artist's song of the same
    // name, a live cut, and a song too far off in length.
    let own: [(&str, &str, i64); 20] = [
        ("Alpha", "Tune Number 0 Pt. 2", 180_000),
        ("Golf", "Tune Number 1", 181_000),
        ("Alpha", "Tune Number 5 (Live)", 185_000),
        ("Echo", "Tune Number 3", 260_000),
        ("Alpha", "Brand New", 200_000),
        ("Alpha", "Brand Newer", 200_000),
        ("Echo", "Tune Number 9 (Interlude)", 189_000),
        ("Foxtrot", "Tune Number 3", 183_000),
        ("Hotel", "Tune Number 35", 215_000),
        ("Alpha", "Intro", 60_000),
        ("India", "Other 1", 100_000),
        ("India", "Other 2", 100_000),
        ("India", "Other 3", 100_000),
        ("India", "Other 4", 100_000),
        ("India", "Other 5", 100_000),
        ("Juliet", "Other 6", 100_000),
        ("Juliet", "Other 7", 100_000),
        ("Juliet", "Other 8", 100_000),
        ("Juliet", "Other 9", 100_000),
        ("Juliet", "Other 10", 100_000),
    ];
    album(&c, &format!("{B}:own"), "Mixed", "B Sides", "[]");
    for (k, (artist, title, ms)) in own.iter().enumerate() {
        song(&c, &format!("{B}:own{k}"), &format!("{B}:own"), "B Sides", "Mixed", Some(artist), title, *ms, None);
    }
    // A's own "Intro" on another album: generic, so no match for B's.
    song(&c, &format!("{A}:intro"), &format!("{A}:al0"), "Record 0", "Alpha", None, "Intro", 60_000, None);

    let rows = index::rebuild(&c).unwrap();
    assert!(rows >= 101, "{rows}");
    let none = HashMap::new();
    let mut same = 0;
    let mut other = 0;
    let b_ids: Vec<String> = {
        let mut st = c.prepare("SELECT id FROM tracks WHERE src = ?1").unwrap();
        st.query_map([B], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(b_ids.len(), 50);
    for id in &b_ids {
        let k = key_of(&c, id);
        let hit: Option<Candidate> = index::best(&c, &k, Context::Single, &none, on(A)).unwrap();
        match (expected.get(id), &hit) {
            (Some(want), Some(h)) => {
                assert_eq!(&h.track_id, want, "{id}");
                assert_eq!(h.matched.kind, MatchKind::SameRecording, "{id}: {h:?}");
                same += 1;
            }
            (Some(want), None) => panic!("{id} should match {want}"),
            (None, Some(h)) => {
                assert_eq!(h.matched.kind, MatchKind::OtherVersion, "{id} is B's own: {h:?}");
                other += 1;
            }
            (None, None) => {}
        }
    }
    assert_eq!(same, 30, "the 60% overlap");
    assert_eq!(other, 2, "the live cut and the long one are other versions");
}

#[test]
fn unreleased_songs_are_never_matched_through_the_index() {
    let (_d, c) = db();
    album(&c, &format!("{A}:demo"), "Artist", "Demos", "[\"demo\"]");
    song(&c, &format!("{A}:1"), &format!("{A}:demo"), "Demos", "Artist", None, "Secret", 200_000, Some("m1"));
    album(&c, &format!("{A}:al"), "Artist", "Album", "[]");
    song(&c, &format!("{A}:2"), &format!("{A}:al"), "Album", "Artist", None, "Leaked", 200_000, None);
    // A released album, but this file is tagged as a leak.
    c.execute(
        "INSERT INTO file_tags (part_key, status, version, isrc, scanned_at, ver) VALUES (?1, 'ok', 'leak', 'USRC17607839', 1, 1)",
        [format!("/p/{A}:2")],
    )
    .unwrap();
    album(&c, &format!("{B}:al"), "Artist", "Album", "[]");
    song(&c, &format!("{B}:1"), &format!("{B}:al"), "Album", "Artist", None, "Secret", 200_000, Some("m1"));
    song(&c, &format!("{B}:2"), &format!("{B}:al"), "Album", "Artist", None, "Leaked", 200_000, None);
    c.execute(
        "INSERT INTO file_tags (part_key, status, isrc, scanned_at, ver) VALUES (?1, 'ok', 'USRC17607839', 1, 1)",
        [format!("/p/{B}:2")],
    )
    .unwrap();
    index::rebuild(&c).unwrap();
    let none = HashMap::new();
    for id in ["1", "2"] {
        // The released copy on B finds nothing on A, even by id.
        let k = key_of(&c, &format!("{B}:{id}"));
        assert!(!k.unreleased);
        assert_eq!(index::best(&c, &k, Context::Single, &none, on(A)).unwrap(), None, "{id}");
        // And A's unreleased copy finds nothing on B.
        let k = key_of(&c, &format!("{A}:{id}"));
        assert!(k.unreleased, "{id}");
        assert!(index::lookup(&c, &k, Context::Single, &none).unwrap().is_empty(), "{id}");
    }
}

#[test]
fn ties_go_to_the_same_album_then_the_better_copy() {
    let (_d, c) = db();
    for (tag, title) in [(A, "Album"), (B, "Hits")] {
        album(&c, &format!("{tag}:al"), "Artist", title, "[]");
        song(&c, &format!("{tag}:1"), &format!("{tag}:al"), title, "Artist", None, "Song", 200_000, None);
    }
    index::rebuild(&c).unwrap();
    let none = HashMap::new();
    let k = key("Artist", "Song", "Hits", 200_000);
    let hits = index::lookup(&c, &k, Context::Single, &none).unwrap();
    assert_eq!(hits.iter().map(|h| h.track_id.as_str()).collect::<Vec<_>>(), [format!("{B}:1"), format!("{A}:1")]);
    // No album to go by: the server reached the best way.
    let k = key("Artist", "Song", "", 200_000);
    let ranks = HashMap::from([(A.to_string(), crate::db::merge::Rank::Relay), (B.to_string(), crate::db::merge::Rank::Local)]);
    assert_eq!(index::lookup(&c, &k, Context::Single, &ranks).unwrap()[0].track_id, format!("{B}:1"));
    let ranks = HashMap::from([(A.to_string(), crate::db::merge::Rank::Local), (B.to_string(), crate::db::merge::Rank::Remote)]);
    assert_eq!(index::lookup(&c, &k, Context::Single, &ranks).unwrap()[0].track_id, format!("{A}:1"));
}

#[test]
fn one_artist_is_redone_when_its_tags_change() {
    let (_d, c) = db();
    album(&c, &format!("{A}:al"), "Artist", "Album", "[]");
    song(&c, &format!("{A}:1"), &format!("{A}:al"), "Album", "Artist", None, "Song", 200_000, None);
    album(&c, &format!("{A}:al2"), "Other", "Album 2", "[]");
    song(&c, &format!("{A}:2"), &format!("{A}:al2"), "Album 2", "Other", None, "Tune", 200_000, None);
    index::rebuild(&c).unwrap();
    c.execute(
        "INSERT INTO file_tags (part_key, status, isrc, scanned_at, ver) VALUES (?1, 'ok', 'USRC17607839', 1, 1)",
        [format!("/p/{A}:1")],
    )
    .unwrap();
    let isrc = |c: &Connection| -> Option<String> {
        c.query_row("SELECT isrc FROM song_index WHERE track_id = ?1", [format!("{A}:1")], |r| r.get(0)).unwrap()
    };
    assert_eq!(isrc(&c), None);
    // Tracks of other artists are left alone (a stale row would survive).
    c.execute("UPDATE song_index SET title_fold = 'stale' WHERE track_id = ?1", [format!("{A}:2")]).unwrap();
    let n = index::refresh_for_tracks(&c, &[format!("{A}:1")]).unwrap();
    assert_eq!(n, 1);
    assert_eq!(isrc(&c).as_deref(), Some("USRC17607839"));
    let other: String = c.query_row("SELECT title_fold FROM song_index WHERE track_id = ?1", [format!("{A}:2")], |r| r.get(0)).unwrap();
    assert_eq!(other, "stale");
}

// ------------------------------------------------------------------ speed

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const WORDS: [&str; 24] = [
    "night", "river", "golden", "echo", "paper", "summer", "glass", "heart", "city", "silver", "dream", "fire",
    "ocean", "shadow", "light", "storm", "velvet", "wild", "blue", "stone", "morning", "neon", "hollow", "honey",
];

/// 100k songs on two servers; lookups must stay well under 10 ms.
#[test]
fn lookups_stay_fast_at_100k_songs() {
    let (_d, mut c) = db();
    let mut rng = Rng(7);
    let mut sample: Vec<(String, String, u64)> = Vec::new();
    let tx = c.transaction().unwrap();
    {
        let mut al = tx
            .prepare("INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, matched) VALUES (?1, ?2, ?2, ?3, ?4, 10, ?5, 1)")
            .unwrap();
        let mut tr = tx
            .prepare(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, src)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .unwrap();
        for a in 0..1_000 {
            let tag = if a % 2 == 0 { A } else { B };
            let artist = format!("{} {} {a}", WORDS[rng.below(24)], WORDS[rng.below(24)]);
            for b in 0..10 {
                let album_id = format!("{tag}:al{a}-{b}");
                let album_title = format!("{} {}", WORDS[rng.below(24)], WORDS[rng.below(24)]);
                al.execute(params![album_id, album_title, format!("{tag}:ar{a}"), artist, tag]).unwrap();
                for t in 0..10 {
                    let id = format!("{tag}:{a}-{b}-{t}");
                    let title = format!("{} {} {}", WORDS[rng.below(24)], WORDS[rng.below(24)], WORDS[rng.below(24)]);
                    let ms = 120_000 + rng.below(200_000) as i64;
                    tr.execute(params![id, title, album_id, album_title, format!("{tag}:ar{a}"), artist, ms, format!("/p/{id}"), tag])
                        .unwrap();
                    if rng.below(200) == 0 {
                        sample.push((artist.clone(), title, ms as u64));
                    }
                }
            }
        }
    }
    let started = Instant::now();
    let rows = index::rebuild(&tx).unwrap();
    let rebuild_s = started.elapsed().as_secs_f64();
    tx.commit().unwrap();
    assert_eq!(rows, 100_000);
    println!("song index rebuild, 100k songs: {rebuild_s:.2} s");

    let none = HashMap::new();
    let mut times: Vec<f64> = Vec::new();
    for (i, (artist, title, ms)) in sample.iter().cycle().take(500).enumerate() {
        let title = if i % 2 == 0 { format!("{title} (Remastered 2011)") } else { title.clone() };
        let k = key(artist, &title, "", *ms);
        let s = Instant::now();
        let hits = index::lookup(&c, &k, Context::Single, &none).unwrap();
        times.push(s.elapsed().as_secs_f64() * 1000.0);
        assert!(!hits.is_empty());
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = times[(times.len() as f64 * 0.95) as usize];
    println!("song index lookup at 100k: p50 {:.3} ms, p95 {p95:.3} ms", times[times.len() / 2]);
    assert!(p95 < 10.0, "p95 {p95} ms");
}
