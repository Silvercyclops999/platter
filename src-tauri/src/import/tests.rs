use std::collections::HashMap;
use std::io::Write;

use rusqlite::{Connection, params};

use super::files::{InputFile, decode_text, read_input, read_zip_bytes};
use super::matching::{Status, decide};
use super::sources::streaming_export::StreamingExport;
use super::sources::table::{Table, detect_columns, detect_delimiter, parse_csv, parse_duration, parse_m3u, split_csv};
use super::sources::{Parser, parse_time};
use super::store::{self, Decision};
use super::{Parsed, SongIn};
use crate::db::schema;
use crate::identity::index;

const TRANSFER: &str = include_str!("fixtures/transfer.csv");
const SEMICOLON: &str = include_str!("fixtures/semicolon.csv");
const M3U: &str = include_str!("fixtures/list.m3u8");
const LIBRARY: &str = include_str!("fixtures/YourLibrary.json");
const PLAYLISTS: &str = include_str!("fixtures/Playlist1.json");
const SHORT_HISTORY: &str = include_str!("fixtures/StreamingHistory_music_0.json");
const HISTORY: &str = include_str!("fixtures/Streaming_History_Audio_2024.json");

fn with_bom(s: &str) -> Vec<u8> {
    let mut v = vec![0xEF, 0xBB, 0xBF];
    v.extend_from_slice(s.as_bytes());
    v
}

fn names(songs: &[SongIn]) -> Vec<String> {
    songs.iter().map(|s| format!("{} - {}", s.artist, s.title)).collect()
}

// ------------------------------------------------------------------ times

#[test]
fn history_time_stamps() {
    assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_time("2024-03-01T18:04:37Z"), Some(1_709_316_277_000));
    assert_eq!(parse_time("2024-03-01 18:04"), Some(1_709_316_240_000));
    assert_eq!(parse_time("2024-03-06T12:30:15.250Z"), Some(1_709_728_215_250));
    assert_eq!(parse_time("2024-03-01T20:04:37+02:00"), Some(1_709_316_277_000));
    assert_eq!(parse_time("2000-02-29T00:00:00Z"), Some(951_782_400_000));
    assert_eq!(parse_time("yesterday"), None);
    assert_eq!(parse_time("2024-13-01 10:00"), None);
}

// ------------------------------------------------------------------ CSV and M3U

#[test]
fn transfer_csv_with_bom_quotes_and_a_liked_list() {
    let p = Table.parse(&[InputFile::new("My export.csv", with_bom(TRANSFER))]).unwrap();
    assert_eq!(p.playlists.len(), 1);
    let road = &p.playlists[0];
    assert_eq!(road.name, "Road Trip");
    assert_eq!(names(&road.songs), ["The Paper Kites - Northern Lights", "Low Tide - Quiet, Please", "Low Tide - Golden Hour (Live)"]);
    assert_eq!(road.songs[1].isrc.as_deref(), Some("QZABC2100001"));
    assert_eq!(road.songs[0].album.as_deref(), Some("Lights Out"));
    // A list named like liked songs is the liked songs.
    assert_eq!(names(p.liked.as_ref().unwrap()), ["Echo Field - Ghost Town", "Echo Field - Say \"Hello\""]);
    assert_eq!(p.skipped.incomplete_rows, 1, "the row with no artist");
    assert!(p.plays.is_empty() && p.albums.is_none());
}

#[test]
fn semicolons_line_breaks_in_quotes_and_lengths() {
    assert_eq!(detect_delimiter(SEMICOLON), ';');
    let p = parse_csv(SEMICOLON, "file").unwrap();
    let songs = &p.playlists[0].songs;
    assert_eq!(p.playlists[0].name, "Evening");
    assert_eq!(songs[1].title, "Line one\nline two");
    assert_eq!(songs.iter().map(|s| s.duration_ms).collect::<Vec<_>>(), [Some(221_000), Some(225_000), Some(201_000)]);
}

#[test]
fn tabs_several_artists_and_a_milliseconds_column() {
    let text = "Track Name\tArtist Name(s)\tAlbum Name\tDuration (ms)\tISRC\n\
                Northern Lights\tThe Paper Kites, Guest Singer\tLights Out\t221000\tqz-abc-21-00001\n";
    assert_eq!(detect_delimiter(text), '\t');
    let p = parse_csv(text, "Exported list").unwrap();
    let s = &p.playlists[0].songs[0];
    assert_eq!(p.playlists[0].name, "Exported list", "no playlist column: named after the file");
    assert_eq!(s.artist, "The Paper Kites, Guest Singer", "kept whole: a comma can be part of one name");
    assert_eq!(s.duration_ms, Some(221_000));
    assert_eq!(s.isrc.as_deref(), Some("QZABC2100001"));
}

#[test]
fn bars_and_utf16_files() {
    let text = "artist|title\nLow Tide|Shoreline Song\n";
    assert_eq!(detect_delimiter(text), '|');
    let mut bytes = vec![0xFF, 0xFE];
    for u in "Artist,Song\nÉcho Fiéld,Café\n".encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let p = Table.parse(&[InputFile::new("a.csv", bytes)]).unwrap();
    assert_eq!(names(&p.playlists[0].songs), ["Écho Fiéld - Café"]);
    // Latin-1 from an old spreadsheet.
    assert_eq!(decode_text(&[0x43, 0x61, 0x66, 0xE9]), "Café");
}

#[test]
fn missing_columns() {
    // No title or no artist column: a clear error.
    assert!(parse_csv("Song;Notes\nA;B\n", "x").is_err());
    assert!(parse_csv("Title,Comment\nA,B\n", "x").is_err());
    assert!(Table.parse(&[InputFile::new("x.csv", "Title,Comment\nA,B\n")]).is_err());
    // Only artist and title is enough.
    let p = parse_csv("title,artist\nSong,Someone\n", "x").unwrap();
    assert_eq!(p.playlists[0].songs[0], SongIn { artist: "Someone".into(), title: "Song".into(), ..Default::default() });
    // An empty file.
    assert!(parse_csv("", "x").is_err());
}

#[test]
fn column_names_exports_use() {
    let h = |s: &str| s.split(',').map(String::from).collect::<Vec<_>>();
    let c = detect_columns(&h("Playlist name,Track name,Artist name,Album,ISRC"));
    assert_eq!((c.playlist, c.title, c.artist, c.album, c.isrc), (Some(0), Some(1), Some(2), Some(3), Some(4)));
    let c = detect_columns(&h("Album Artist,Song Title (Service),Album Name,Length"));
    assert_eq!((c.artist, c.title, c.album, c.duration), (Some(0), Some(1), Some(2), Some(3)));
    assert_eq!(split_csv("a,\"b,c\",d\r\n\r\ne,f,g", ','), vec![vec!["a", "b,c", "d"], vec!["e", "f", "g"]]);
    assert_eq!(parse_duration("1:02:03", false), Some(3_723_000));
    assert_eq!(parse_duration("212.5", false), Some(212_500));
    assert_eq!(parse_duration("", false), None);
    assert_eq!(parse_duration("-3", false), None);
}

#[test]
fn m3u_lists() {
    let p = parse_m3u(M3U, "file");
    assert_eq!(p.playlists[0].name, "Night Drive");
    assert_eq!(
        names(&p.playlists[0].songs),
        ["The Paper Kites - Northern Lights", "Low Tide - Quiet, Please", "Echo Field - Ghost Town"]
    );
    assert_eq!(p.playlists[0].songs[0].duration_ms, Some(221_000));
    assert_eq!(p.playlists[0].songs[1].duration_ms, None, "-1 is unknown");
    assert_eq!(p.skipped.incomplete_rows, 1);
}

// ------------------------------------------------------------------ account data and history

fn export_files() -> Vec<InputFile> {
    vec![
        InputFile::new("Account Data/YourLibrary.json", LIBRARY),
        InputFile::new("Account Data/Playlist1.json", PLAYLISTS),
        InputFile::new("Account Data/StreamingHistory_music_0.json", SHORT_HISTORY),
        InputFile::new("Account Data/Identity.json", r#"{"displayName":"someone","imageUrl":"x"}"#),
        InputFile::new("Extended/Streaming_History_Audio_2024.json", HISTORY),
        InputFile::new("Extended/ReadMeFirst.pdf", "not json"),
    ]
}

#[test]
fn account_data_library_and_playlists() {
    let p = StreamingExport.parse(&export_files()).unwrap();
    assert_eq!(
        names(p.liked.as_ref().unwrap()),
        ["The Paper Kites - Northern Lights", "Echo Field - Ghost Town", "Nobody Here - Not In The Library"]
    );
    assert_eq!(p.albums.as_ref().unwrap().len(), 2);
    assert_eq!(p.playlists.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Road Trip", "Rainy Day"]);
    assert_eq!(p.playlists[0].songs.len(), 2, "the episode is skipped");
    assert_eq!(p.playlists[0].songs[0].service_id.as_deref(), Some("service:track:aaa1"));
    assert_eq!(p.skipped.unknown_files, 1, "the account details file is left alone");
}

#[test]
fn history_keeps_only_songs_and_drops_private_podcasts_and_empty_records() {
    let p = StreamingExport.parse(&[InputFile::new("h.json", HISTORY)]).unwrap();
    let got: Vec<(i64, u64, String)> = p.plays.iter().map(|x| (x.ended_at, x.ms_played, x.song.title.clone())).collect();
    assert_eq!(
        got,
        [(1_709_316_277_000, 221_000, "Northern Lights".to_string()), (1_709_728_215_250, 204_000, "Ghost Town".to_string())]
    );
    assert_eq!(p.skipped.private_plays, 1);
    assert_eq!(p.skipped.podcasts, 1);
    assert_eq!(p.skipped.no_metadata, 1);
    assert_eq!(p.skipped.short_plays, 1);
    assert!(p.liked.is_none() && p.playlists.is_empty());
    // The short history format too.
    let p = StreamingExport.parse(&[InputFile::new("s.json", SHORT_HISTORY)]).unwrap();
    assert_eq!(p.plays.len(), 2);
    assert_eq!(p.skipped.short_plays, 1);
}

#[test]
fn files_that_are_not_an_export_are_refused() {
    assert!(StreamingExport.parse(&[InputFile::new("x.json", r#"{"displayName":"a"}"#)]).is_err());
    assert!(StreamingExport.parse(&[InputFile::new("x.json", "[1, 2, 3]")]).is_err());
    assert!(StreamingExport.parse(&[InputFile::new("x.csv", "a,b")]).is_err());
}

fn zip_of(files: &[InputFile]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for f in files {
            z.start_file(f.name.as_str(), opts).unwrap();
            z.write_all(&f.bytes).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn zips_folders_and_single_files() {
    let bytes = zip_of(&export_files());
    let read = read_zip_bytes(&bytes).unwrap();
    assert_eq!(read.len(), 5, "the pdf is not read");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("export.zip"), &bytes).unwrap();
    std::fs::create_dir(dir.path().join("loose")).unwrap();
    std::fs::write(dir.path().join("loose").join("list.csv"), "title,artist\nA,B\n").unwrap();
    std::fs::write(dir.path().join("cover.jpg"), [0u8; 4]).unwrap();
    let all = read_input(dir.path()).unwrap();
    assert_eq!(all.len(), 6);
    assert!(all.iter().any(|f| f.name == "export.zip/Account Data/YourLibrary.json"));
    assert!(all.iter().any(|f| f.name == "loose/list.csv"));
    let one = read_input(&dir.path().join("export.zip")).unwrap();
    assert_eq!(one.len(), 5);
}

// ------------------------------------------------------------------ the library

const A: &str = "aaaaaaaa";

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
    schema::migrate(&mut c).unwrap();
    (dir, c)
}

fn album(c: &Connection, id: &str, artist: &str, title: &str, subformats: &str) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, matched, subformats)
         VALUES (?1, ?2, ?2, ?3, ?4, 1, ?5, 1, ?6)",
        params![format!("{A}:{id}"), title, format!("{A}:ar:{artist}"), artist, A, subformats],
    )
    .unwrap();
}

fn song(c: &Connection, id: &str, album_id: &str, album: &str, artist: &str, title: &str, ms: i64) {
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, src)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![format!("{A}:{id}"), title, format!("{A}:{album_id}"), album, format!("{A}:ar:{artist}"), artist, ms, format!("/p/{id}"), A],
    )
    .unwrap();
}

fn tid(n: &str) -> String {
    format!("{A}:{n}")
}

/// A small library: some songs the fixtures name, one with an ISRC under a
/// different title, a studio cut of a song imported live, a song on two
/// albums, and an unreleased demo.
fn library(c: &Connection) {
    album(c, "al1", "The Paper Kites", "Lights Out", "[]");
    song(c, "1", "al1", "Lights Out", "The Paper Kites", "Northern Lights", 221_500);
    album(c, "al2", "Low Tide", "Shoreline", "[]");
    song(c, "2", "al2", "Shoreline", "Low Tide", "Quiet Please (2020 Remaster)", 240_000);
    c.execute("INSERT INTO file_tags (part_key, status, isrc, scanned_at, ver) VALUES ('/p/2', 'ok', 'QZABC2100001', 1, 1)", []).unwrap();
    song(c, "3", "al2", "Shoreline", "Low Tide", "Golden Hour", 200_000);
    album(c, "al3", "Echo Field", "Static", "[]");
    song(c, "4", "al3", "Static", "Echo Field", "Ghost Town", 204_000);
    album(c, "al4", "Echo Field", "Best Of", "[]");
    song(c, "5", "al4", "Best Of", "Echo Field", "Ghost Town", 204_000);
    album(c, "demo", "Nobody Here", "Demos", "[\"demo\"]");
    song(c, "6", "demo", "Demos", "Nobody Here", "Also Missing", 100_000);
    index::rebuild(c).unwrap();
}

fn import(c: &mut Connection, source: &str, parsed: Parsed, now: i64) -> store::RunStats {
    let prepared = store::prepare(c, source, parsed, HashMap::new()).unwrap();
    store::write(c, source, Some("export.zip"), prepared, source == "streaming", now).unwrap()
}

fn song_row(c: &Connection, source: &str, title: &str) -> (String, Option<String>, Option<String>, i64) {
    c.query_row(
        "SELECT status, track_id, tier, confidence FROM import_songs WHERE source = ?1 AND title = ?2",
        [source, title],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .unwrap()
}

#[test]
fn matching_tiers_and_the_review_queue() {
    let (_d, mut c) = db();
    library(&c);
    let p = Table.parse(&[InputFile::new("x.csv", TRANSFER)]).unwrap();
    import(&mut c, "transfer", p, 1_000);
    // ISRC, though the library's title differs.
    assert_eq!(song_row(&c, "transfer", "Quiet, Please"), ("matched".into(), Some(tid("2")), Some("isrc".into()), 99));
    // Names, no length, same album.
    assert_eq!(song_row(&c, "transfer", "Northern Lights"), ("matched".into(), Some(tid("1")), Some("names".into()), 70));
    // A live cut of a studio song: another version, so unsure.
    assert_eq!(song_row(&c, "transfer", "Golden Hour (Live)").0, "unsure");
    // On the album it was imported from, though it's on two.
    assert_eq!(song_row(&c, "transfer", "Ghost Town").1, Some(tid("4")));
    assert_eq!(song_row(&c, "transfer", "Say \"Hello\"").0, "missing");

    // Names and length within the slack.
    let p = parse_csv(SEMICOLON, "x").unwrap();
    import(&mut c, "lists", p, 1_000);
    assert_eq!(song_row(&c, "lists", "Northern Lights"), ("matched".into(), Some(tid("1")), Some("names".into()), 90));

    // Names only, on another album, and two library songs it could be.
    let p = parse_csv("title,artist,album\nGhost Town,Echo Field,Something Else\n", "x").unwrap();
    import(&mut c, "lists", p, 1_000);
    let queue = store::review_queue(&c).unwrap();
    let titles: Vec<&str> = queue.iter().map(|q| q.title.as_str()).collect();
    assert_eq!(titles, ["Ghost Town", "Golden Hour (Live)"]);
    assert_eq!(queue[0].candidates.len(), 2);
    assert_eq!(queue[1].candidates[0].id, tid("3"));

    // Decisions: accept, pick another, none.
    store::review(&mut c, queue[1].song_id, &Decision::Accept, 2_000).unwrap();
    assert_eq!(song_row(&c, "transfer", "Golden Hour (Live)"), ("matched".into(), Some(tid("3")), Some("reviewed".into()), 100));
    store::review(&mut c, queue[0].song_id, &Decision::Pick { track_id: tid("5") }, 2_000).unwrap();
    assert_eq!(song_row(&c, "lists", "Ghost Town").1, Some(tid("5")));
    assert!(store::review(&mut c, queue[0].song_id, &Decision::Pick { track_id: "nope".into() }, 2_000).is_err());
    assert!(store::review_queue(&c).unwrap().is_empty());

    // Decisions survive importing again.
    let p = Table.parse(&[InputFile::new("x.csv", TRANSFER)]).unwrap();
    import(&mut c, "transfer", p, 3_000);
    assert_eq!(song_row(&c, "transfer", "Golden Hour (Live)").1, Some(tid("3")));
    store::review(&mut c, queue[1].song_id, &Decision::None, 4_000).unwrap();
    assert_eq!(song_row(&c, "transfer", "Golden Hour (Live)").0, "missing");
}

#[test]
fn decide_rules() {
    use crate::identity::index::Candidate;
    use crate::identity::{Match, MatchKind, Tier};
    let cand = |id: &str, tier, confidence, kind, same_album| Candidate {
        track_id: id.into(),
        matched: Match { tier, confidence, kind },
        same_album,
    };
    assert_eq!(decide(&[], true).status, Status::Missing);
    let one = [cand("x", Tier::Text, 70, MatchKind::SameRecording, false)];
    assert_eq!(decide(&one, true).status, Status::Matched, "the only song it could be");
    assert_eq!(decide(&one, true).confidence, 65);
    let two = [one[0].clone(), cand("y", Tier::Text, 70, MatchKind::SameRecording, false)];
    assert_eq!(decide(&two, true).status, Status::Unsure);
    assert_eq!(decide(&two, false).status, Status::Matched, "no album to go on");
    let other = [cand("x", Tier::TextOtherVersion, 60, MatchKind::OtherVersion, true)];
    assert_eq!(decide(&other, true).status, Status::Unsure);
    assert_eq!(decide(&other, true).candidates, ["x"]);
}

#[test]
fn unreleased_songs_are_never_matched() {
    let (_d, mut c) = db();
    library(&c);
    let p = StreamingExport.parse(&[InputFile::new("p.json", PLAYLISTS)]).unwrap();
    import(&mut c, "streaming", p, 1);
    assert_eq!(song_row(&c, "streaming", "Also Missing").0, "missing", "only a demo has it");
}

#[test]
fn account_import_report_missing_list_and_dedupe() {
    let (_d, mut c) = db();
    library(&c);
    let p = StreamingExport.parse(&export_files()).unwrap();
    let stats = import(&mut c, "streaming", p, 10);
    // Northern Lights at 18:04 is in both histories: one play. Plus Ghost
    // Town twice (two days).
    assert_eq!(stats.plays_added, 3);
    let r = &store::reports(&c).unwrap()[0];
    assert_eq!(r.source, "streaming");
    // Songs: Northern Lights, Ghost Town, Not In The Library, Also Missing.
    assert_eq!((r.songs.total, r.songs.matched, r.songs.missing), (4, 2, 2));
    assert_eq!((r.liked.total, r.liked.matched), (3, 2));
    assert_eq!(r.playlists, 2);
    assert_eq!((r.albums.total, r.albums.matched), (2, 1));
    assert_eq!((r.plays.total, r.plays.matched), (3, 3));
    assert_eq!(r.skipped.private_plays, 1);
    assert_eq!(r.skipped.podcasts, 2, "one in history, one in a playlist");
    let keys: Vec<Option<String>> = c
        .prepare("SELECT rating_key FROM imported_plays ORDER BY played_at")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(keys, [Some(tid("1")), Some(tid("4")), Some(tid("4"))]);

    // Missing: wanted by both playlists and the likes, then by one.
    let m = store::missing(&c, 100).unwrap();
    assert_eq!(m.iter().map(|x| (x.title.as_str(), x.wants)).collect::<Vec<_>>(), [("Not In The Library", 3), ("Also Missing", 1)]);
    assert_eq!(m[0].wanted_by, ["Liked songs", "Rainy Day", "Road Trip"]);
    assert_eq!(m[0].links.bandcamp, "https://bandcamp.com/search?q=Nobody%20Here%20Not%20In%20The%20Library&item_type=t");
    assert!(store::link_allowed(&m[0].links.qobuz) && store::link_allowed(&m[0].links.itunes));

    // The same download again adds nothing.
    let before: i64 = c.query_row("SELECT count(*) FROM import_songs", [], |r| r.get(0)).unwrap();
    let p = StreamingExport.parse(&export_files()).unwrap();
    let again = import(&mut c, "streaming", p, 20);
    assert_eq!(again.plays_added, 0);
    let after: i64 = c.query_row("SELECT count(*) FROM import_songs", [], |r| r.get(0)).unwrap();
    assert_eq!(before, after);
    assert_eq!(store::lists(&c).unwrap().len(), 3);
    let plays: i64 = c.query_row("SELECT count(*) FROM imported_plays", [], |r| r.get(0)).unwrap();
    assert_eq!(plays, 3);
}

#[test]
fn a_newer_export_updates_instead_of_adding() {
    let (_d, mut c) = db();
    library(&c);
    let p = StreamingExport.parse(&export_files()).unwrap();
    import(&mut c, "streaming", p, 10);
    // The newer download: Rainy Day is gone, Road Trip changed, one less
    // liked song, and a history-only zip later changes none of that.
    let newer_lists = r#"{"playlists":[{"name":"Road Trip","items":[
        {"track":{"trackName":"Ghost Town","artistName":"Echo Field","albumName":"Static"}}]}]}"#;
    let newer_library = r#"{"tracks":[{"artist":"Echo Field","album":"Static","track":"Ghost Town"}],"albums":[]}"#;
    let p = StreamingExport.parse(&[InputFile::new("Playlist1.json", newer_lists), InputFile::new("YourLibrary.json", newer_library)]).unwrap();
    import(&mut c, "streaming", p, 20);
    let lists = store::lists(&c).unwrap();
    assert_eq!(lists.iter().map(|l| (l.name.as_str(), l.total)).collect::<Vec<_>>(), [("Liked songs", 1), ("Road Trip", 1)]);
    let songs: i64 = c.query_row("SELECT count(*) FROM import_songs", [], |r| r.get(0)).unwrap();
    assert_eq!(songs, 1, "songs nothing wants any more are dropped");
    assert!(store::missing(&c, 100).unwrap().is_empty());
    let p = StreamingExport.parse(&[InputFile::new("h.json", HISTORY)]).unwrap();
    import(&mut c, "streaming", p, 30);
    assert_eq!(store::lists(&c).unwrap().len(), 2);
    // A list file adds and replaces by name, keeping the others.
    let p = parse_csv("title,artist,playlist\nA,B,One\n", "x").unwrap();
    import(&mut c, "lists", p, 40);
    let p = parse_csv("title,artist,playlist\nC,D,Two\n", "x").unwrap();
    import(&mut c, "lists", p, 50);
    let p = parse_csv("title,artist,playlist\nE,F,One\nG,H,One\n", "x").unwrap();
    import(&mut c, "lists", p, 60);
    let lists: Vec<(String, u64)> = store::lists(&c).unwrap().into_iter().filter(|l| l.source == "lists").map(|l| (l.name, l.total)).collect();
    assert_eq!(lists, [("One".to_string(), 2), ("Two".to_string(), 1)]);
}

#[test]
fn imported_playlist_pages_show_missing_rows() {
    let (_d, mut c) = db();
    library(&c);
    let p = StreamingExport.parse(&export_files()).unwrap();
    import(&mut c, "streaming", p, 10);
    let lists = store::lists(&c).unwrap();
    assert_eq!(lists[0].id, "liked:streaming");
    let road = lists.iter().find(|l| l.name == "Road Trip").unwrap();
    let page = store::list_page(&c, &road.id).unwrap();
    assert_eq!(page.rows.iter().map(|r| r.status).collect::<Vec<_>>(), [Status::Matched, Status::Missing]);
    assert_eq!(page.rows[0].track.as_ref().unwrap().id, tid("1"));
    assert!(page.rows[1].track.is_none());
    let liked = store::list_page(&c, "liked:streaming").unwrap();
    assert_eq!(liked.rows.len(), 3);
    assert!(store::list_page(&c, "pl:999").is_err());
    // A matched song that left the library shows as missing.
    c.execute("DELETE FROM tracks WHERE id = ?1", [tid("1")]).unwrap();
    let page = store::list_page(&c, &road.id).unwrap();
    assert_eq!(page.rows[0].status, Status::Missing);
}

#[test]
fn a_song_comes_out_of_one_list_and_is_forgotten_when_nothing_wants_it() {
    let (_d, mut c) = db();
    library(&c);
    let p = StreamingExport.parse(&export_files()).unwrap();
    import(&mut c, "streaming", p, 10);
    let road = store::lists(&c).unwrap().into_iter().find(|l| l.name == "Road Trip").unwrap();
    let gone = store::list_page(&c, &road.id).unwrap().rows[1].song_id;
    let others: Vec<String> = store::lists(&c)
        .unwrap()
        .into_iter()
        .filter(|l| l.id != road.id && store::list_page(&c, &l.id).unwrap().rows.iter().any(|r| r.song_id == gone))
        .map(|l| l.id)
        .collect();
    store::remove_from_list(&mut c, &road.id, gone).unwrap();
    let page = store::list_page(&c, &road.id).unwrap();
    assert!(page.rows.iter().all(|r| r.song_id != gone));
    assert_eq!(page.list.total, 1);
    // Already out: not found, and nothing else changes.
    assert!(matches!(store::remove_from_list(&mut c, &road.id, gone), Err(crate::error::AppError::NotFound(_))));
    assert!(matches!(store::remove_from_list(&mut c, "pl:x", gone), Err(crate::error::AppError::NotFound(_))));
    let n = |c: &Connection| c.query_row("SELECT count(*) FROM import_songs WHERE id = ?1", [gone], |r| r.get::<_, i64>(0)).unwrap();
    for id in &others {
        assert_eq!(n(&c), 1, "another list still wants it");
        store::remove_from_list(&mut c, id, gone).unwrap();
    }
    assert_eq!(n(&c), 0, "a song nothing wants is forgotten");
    // Other lists keep their songs.
    assert!(!store::list_page(&c, "liked:streaming").unwrap().rows.is_empty());
}

#[test]
fn private_fields_are_never_stored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let mut c = schema::open_writer(&path).unwrap();
    schema::migrate(&mut c).unwrap();
    library(&c);
    let p = StreamingExport.parse(&export_files()).unwrap();
    import(&mut c, "streaming", p, 10);
    // Every text in every table, then the database file itself.
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%fts%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let secrets = ["192.0.2.10", "198.51.100.7", "203.0.113.5", "fixture-agent", "desktop-os-fixture", "phone-os-fixture", "ZZ", "Secret Listening"];
    for t in &tables {
        let mut st = c.prepare(&format!("SELECT * FROM \"{t}\"")).unwrap();
        let n = st.column_count();
        let mut rows = st.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            for i in 0..n {
                if let Ok(Some(s)) = r.get::<_, Option<String>>(i) {
                    for x in secrets {
                        assert!(!s.contains(x), "{x} found in {t}: {s}");
                    }
                }
            }
        }
    }
    c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    drop(c);
    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    for x in ["192.0.2.10", "198.51.100.7", "fixture-agent", "phone-os-fixture", "Secret Listening"] {
        assert!(!text.contains(x), "{x} is in the database file");
    }
}

#[test]
fn forgetting_a_source_removes_its_rows_and_plays() {
    let (_d, mut c) = db();
    library(&c);
    let p = StreamingExport.parse(&export_files()).unwrap();
    import(&mut c, "streaming", p, 10);
    let p = parse_csv("title,artist\nA,B\n", "x").unwrap();
    import(&mut c, "lists", p, 10);
    store::forget(&mut c, "streaming").unwrap();
    let n = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(n("SELECT count(*) FROM imported_plays"), 0);
    assert_eq!(n("SELECT count(*) FROM import_songs WHERE source = 'streaming'"), 0);
    assert_eq!(n("SELECT count(*) FROM import_songs WHERE source = 'lists'"), 1);
    assert_eq!(store::reports(&c).unwrap().len(), 1);
}

#[test]
fn only_store_search_links_open() {
    assert!(store::link_allowed("https://bandcamp.com/search?q=a"));
    assert!(!store::link_allowed("http://bandcamp.com/search?q=a"));
    assert!(!store::link_allowed("https://bandcamp.com.evil.example/"));
    assert!(!store::link_allowed("https://example.com/"));
    assert!(!store::link_allowed("file:///etc/passwd"));
}

#[test]
fn the_sources_setting_keeps_known_sources_once() {
    let mut s = crate::settings::Settings::default();
    assert!(s.import_sources.is_empty(), "all off until picked");
    let patch = crate::settings::SettingsPatch {
        import_sources: Some(vec!["lists".into(), "nope".into(), "streaming".into(), "lists".into()]),
        ..Default::default()
    };
    s.apply(&patch).unwrap();
    assert_eq!(s.import_sources, ["streaming", "lists"]);
    assert!(crate::settings::SYNCED_SETTINGS.contains(&"importSources"));
}

#[test]
fn file_names_with_artists_that_start_with_digits() {
    let text = "#EXTM3U\n\
                /music/2Pac - First Song.mp3\n\
                /music/10cc - Second Song.flac\n\
                /music/50 Cent - Third Song.mp3\n\
                /music/311 - Fourth Song.mp3\n\
                /music/01 - 21 Savage - Fifth Song.mp3\n\
                /music/07_112 - Sixth Song.mp3\n\
                /music/03. Seventh Song.mp3\n\
                /music/01 - Eighth Song.mp3\n";
    let p = parse_m3u(text, "digits");
    assert_eq!(
        names(&p.playlists[0].songs),
        [
            "2Pac - First Song",
            "10cc - Second Song",
            "50 Cent - Third Song",
            "311 - Fourth Song",
            "21 Savage - Fifth Song",
            "112 - Sixth Song",
        ]
    );
    assert_eq!(p.skipped.incomplete_rows, 2, "a track number and a title name no artist");
}

#[test]
fn artist_names_with_commas_match_whole_and_several_artists_match_the_first() {
    let (_d, mut c) = db();
    album(&c, "t1", "Tyler, The Creator", "First Record", "[]");
    song(&c, "t1", "t1", "First Record", "Tyler, The Creator", "Opening Song", 200_000);
    album(&c, "e1", "Earth, Wind & Fire", "Second Record", "[]");
    song(&c, "e1", "e1", "Second Record", "Earth, Wind & Fire", "Closing Song", 210_000);
    album(&c, "k1", "The Paper Kites", "Lights Out", "[]");
    song(&c, "k1", "k1", "Lights Out", "The Paper Kites", "Northern Lights", 221_000);
    album(&c, "w1", "Tyler", "Other Record", "[]");
    song(&c, "w1", "w1", "Other Record", "Tyler", "Opening Song", 200_000);
    index::rebuild(&c).unwrap();
    let text = "Track Name,Artist Name(s),Album Name\n\
                Opening Song,\"Tyler, The Creator\",First Record\n\
                Closing Song,\"Earth, Wind & Fire\",Second Record\n\
                Northern Lights,\"The Paper Kites, Guest Singer\",Lights Out\n";
    let p = parse_csv(text, "x").unwrap();
    assert_eq!(p.playlists[0].songs[0].artist, "Tyler, The Creator");
    assert_eq!(p.playlists[0].songs[1].artist, "Earth, Wind & Fire");
    import(&mut c, "lists", p, 1_000);
    assert_eq!(song_row(&c, "lists", "Opening Song").1, Some(tid("t1")), "the whole name, not \"Tyler\"");
    assert_eq!(song_row(&c, "lists", "Closing Song").1, Some(tid("e1")));
    assert_eq!(song_row(&c, "lists", "Northern Lights").1, Some(tid("k1")), "the first of several artists");
}

#[test]
fn the_two_history_formats_count_a_play_once_in_either_order() {
    // The short history logs the minute, the extended one the second: the
    // same play ends at 18:04:59.900 in one and "18:04" in the other.
    let short = r#"[{ "endTime": "2024-03-01 18:04", "artistName": "Low Tide", "trackName": "Golden Hour", "msPlayed": 200000 }]"#;
    let extended = r#"[
        { "ts": "2024-03-01T18:04:59.900Z", "master_metadata_track_name": "Golden Hour", "master_metadata_album_artist_name": "Low Tide", "ms_played": 200000 },
        { "ts": "2024-03-01T18:09:10Z", "master_metadata_track_name": "Golden Hour", "master_metadata_album_artist_name": "Low Tide", "ms_played": 45000 },
        { "ts": "2024-03-01T18:09:58Z", "master_metadata_track_name": "Golden Hour", "master_metadata_album_artist_name": "Low Tide", "ms_played": 45000 }
    ]"#;
    let count = |c: &Connection| -> i64 { c.query_row("SELECT count(*) FROM imported_plays", [], |r| r.get(0)).unwrap() };
    for first_short in [true, false] {
        let (_d, mut c) = db();
        library(&c);
        let (a, b) = if first_short { (short, extended) } else { (extended, short) };
        import(&mut c, "streaming", StreamingExport.parse(&[InputFile::new("a.json", a)]).unwrap(), 10);
        import(&mut c, "streaming", StreamingExport.parse(&[InputFile::new("b.json", b)]).unwrap(), 20);
        // The full play once, and the two short plays a minute apart stay two.
        assert_eq!(count(&c), 3, "short first: {first_short}");
        // Both in one download.
        let (_d, mut c) = db();
        library(&c);
        let both = StreamingExport.parse(&[InputFile::new("a.json", a), InputFile::new("b.json", b)]).unwrap();
        assert_eq!(import(&mut c, "streaming", both, 10).plays_added, 3);
    }
}

#[test]
fn a_stray_bad_byte_keeps_a_utf8_file_utf8() {
    let mut bytes = "Écho Fiéld,Café\n".as_bytes().to_vec();
    bytes.push(0xFF);
    bytes.extend_from_slice("Low Tide,Shoreline\n".as_bytes());
    let text = decode_text(&bytes);
    assert!(text.starts_with("Écho Fiéld,Café\n"), "{text}");
    assert!(text.ends_with("Low Tide,Shoreline\n"));
    // A file with no UTF-8 in it at all is still Latin-1.
    assert_eq!(decode_text(&[0x43, 0x61, 0x66, 0xE9, 0x20, 0xFF]), "Café ÿ");
}

#[test]
fn a_long_history_is_written_in_chunks() {
    use crate::db::listens::{IMPORT_CHUNK, ImportSource, ImportedPlay, insert_imported};
    let (_d, mut c) = db();
    let n = IMPORT_CHUNK * 2 + 17;
    let plays: Vec<ImportedPlay> = (0..n)
        .map(|i| ImportedPlay {
            source: ImportSource::File,
            source_id: format!("x:{i}"),
            played_at: 1_700_000_000_000 + i as i64 * 1_000,
            ms_played: Some(200_000),
            title: format!("Song {}", i % 50),
            artist: Some("Low Tide".into()),
            album: None,
            rating_key: None,
        })
        .collect();
    assert_eq!(insert_imported(&mut c, &plays, 1).unwrap(), n);
    assert_eq!(insert_imported(&mut c, &plays, 2).unwrap(), 0, "again adds nothing");
}
