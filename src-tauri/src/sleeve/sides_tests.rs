//! Record sides: reading them from vinyl editions (one record, two records,
//! a second record that starts again at A), CD-only albums with none, and
//! splits by hand kept in the sync store and carried to another device.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::LibSong;
use super::discogs::parse_release as parse_discogs;
use super::sides::*;
use crate::artistinfo::cache;
use crate::artistinfo::mbdetails::parse_release;
use crate::artistinfo::service::album_key;
use crate::db::DbHandle;
use crate::sync::SyncStore;

fn songs(n: u32) -> Vec<LibSong> {
    (1..=n).map(|i| LibSong { id: format!("t{i}"), title: format!("Song {i}"), disc: 1, number: i }).collect()
}

fn labels(s: &[Side]) -> Vec<(&str, Vec<&str>)> {
    s.iter().map(|x| (x.label.as_str(), x.track_ids.iter().map(String::as_str).collect())).collect()
}

/// A MusicBrainz release: one medium per (format, numbers).
fn mb(media: &[(&str, &[&str])]) -> Value {
    let mut n = 0;
    let media: Vec<Value> = media
        .iter()
        .enumerate()
        .map(|(mi, (format, numbers))| {
            let tracks: Vec<Value> = numbers
                .iter()
                .enumerate()
                .map(|(ti, num)| {
                    n += 1;
                    json!({"position": ti + 1, "number": num, "title": format!("Song {n}"), "recording": {"title": format!("Song {n}"), "relations": []}})
                })
                .collect();
            json!({"position": mi + 1, "format": format, "tracks": tracks})
        })
        .collect();
    json!({"id": "aaaaaaaa-1111-4111-8111-000000000009", "title": "Lantern Songs", "media": media})
}

fn discogs(positions: &[&str]) -> Value {
    let tracks: Vec<Value> =
        positions.iter().enumerate().map(|(i, p)| json!({"position": p, "title": format!("Song {}", i + 1), "type_": "track"})).collect();
    json!({"id": 99, "title": "Lantern Songs", "formats": [{"name": "Vinyl", "qty": "2"}], "tracklist": tracks})
}

#[test]
fn side_letters_come_from_printed_positions() {
    for (p, want) in [("A1", Some("A")), ("b12", Some("B")), ("C", Some("C")), ("AA", Some("AA")), ("A2a", Some("A")), ("D3.1", Some("D"))] {
        assert_eq!(side_letter(p).as_deref(), want, "{p}");
    }
    for p in ["1", "12", "1-4", "CD1-2", "CD1", "DVD", "Video 1", "", "AB1", "A-1"] {
        assert_eq!(side_letter(p), None, "{p} names no side");
    }
}

#[test]
fn one_record_from_musicbrainz_has_two_sides() {
    let r = parse_release(&mb(&[("12\" Vinyl", &["A1", "A2", "A3", "B1", "B2"])])).unwrap();
    assert_eq!(r.tracks[3].number.as_deref(), Some("B1"), "the printed number is kept");
    let s = resolve("al", &songs(5), Some(&r), None, None);
    assert_eq!(s.source, Some(SidesSource::Musicbrainz));
    assert!(s.detected);
    assert_eq!(labels(&s.sides), [("A", vec!["t1", "t2", "t3"]), ("B", vec!["t4", "t5"])]);
}

#[test]
fn a_double_album_has_four_sides_even_when_the_second_record_starts_at_a() {
    // Printed A to D.
    let r = parse_release(&mb(&[("Vinyl", &["A1", "A2", "B1", "B2"]), ("Vinyl", &["C1", "C2", "D1"])])).unwrap();
    let lib: Vec<LibSong> = [(1, 1), (1, 2), (1, 3), (1, 4), (2, 1), (2, 2), (2, 3)]
        .iter()
        .enumerate()
        .map(|(i, (d, n))| LibSong { id: format!("t{}", i + 1), title: format!("Song {}", i + 1), disc: *d, number: *n })
        .collect();
    let s = resolve("al", &lib, Some(&r), None, None);
    assert_eq!(labels(&s.sides), [("A", vec!["t1", "t2"]), ("B", vec!["t3", "t4"]), ("C", vec!["t5", "t6"]), ("D", vec!["t7"])]);
    // Printed A, B, A, B: still A to D.
    let r = parse_release(&mb(&[("Vinyl", &["A1", "A2", "B1", "B2"]), ("Vinyl", &["A1", "A2", "B1"])])).unwrap();
    let s = resolve("al", &lib, Some(&r), None, None);
    assert_eq!(s.sides.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), ["A", "B", "C", "D"]);
    assert_eq!(s.sides[2].track_ids, ["t5", "t6"]);
}

#[test]
fn discogs_positions_give_sides_when_musicbrainz_has_none() {
    let cd = parse_release(&mb(&[("CD", &["1", "2", "3", "4", "5", "6"])])).unwrap();
    let d = parse_discogs(&discogs(&["A1", "A2", "B1", "C1", "C2", "D1"])).unwrap();
    let s = resolve("al", &songs(6), Some(&cd), Some(&d), None);
    assert_eq!(s.source, Some(SidesSource::Discogs));
    assert_eq!(labels(&s.sides), [("A", vec!["t1", "t2"]), ("B", vec!["t3"]), ("C", vec!["t4", "t5"]), ("D", vec!["t6"])]);
}

#[test]
fn a_cd_only_album_has_no_sides() {
    let cd = parse_release(&mb(&[("CD", &["1", "2", "3"])])).unwrap();
    let d = parse_discogs(&discogs(&["1", "2", "3"])).unwrap();
    let s = resolve("al", &songs(3), Some(&cd), Some(&d), None);
    assert!(s.sides.is_empty());
    assert_eq!(s.source, None);
    assert!(!s.detected);
    assert!(resolve("al", &songs(3), None, None, None).sides.is_empty(), "nothing cached");
}

#[test]
fn songs_the_edition_lacks_join_the_side_before_and_jumbled_sides_are_refused() {
    // A CD bonus song after the vinyl's last: it goes with side B.
    let d = parse_discogs(&discogs(&["A1", "A2", "B1", "B2"])).unwrap();
    let s = from_discogs(&d, &songs(5)).unwrap();
    assert_eq!(labels(&s), [("A", vec!["t1", "t2"]), ("B", vec!["t3", "t4", "t5"])]);
    // A side that comes back later isn't a record's order.
    let mut keys = HashMap::new();
    for (id, l) in [("t1", "A"), ("t2", "B"), ("t3", "A")] {
        keys.insert(id.to_owned(), (0, l.to_owned()));
    }
    assert_eq!(group(&songs(3), &keys), None);
    // One side is no split.
    let one = parse_discogs(&discogs(&["A1", "A2", "A3"])).unwrap();
    assert_eq!(from_discogs(&one, &songs(3)), None);
}

#[test]
fn a_split_by_hand_wins_and_an_empty_one_means_no_sides() {
    let r = parse_release(&mb(&[("Vinyl", &["A1", "A2", "B1", "B2"])])).unwrap();
    let m = ManualSides { starts: vec!["t2".into(), "t4".into()] };
    let s = resolve("al", &songs(4), Some(&r), None, Some(&m));
    assert_eq!(s.source, Some(SidesSource::Manual));
    assert!(s.detected, "the edition's sides are still known");
    assert_eq!(labels(&s.sides), [("A", vec!["t1"]), ("B", vec!["t2", "t3"]), ("C", vec!["t4"])]);
    let none = resolve("al", &songs(4), Some(&r), None, Some(&ManualSides::default()));
    assert!(none.sides.is_empty());
    assert_eq!(none.source, Some(SidesSource::Manual));
}

#[test]
fn starts_by_hand_are_checked_and_put_in_order() {
    let lib = songs(4);
    assert_eq!(clean_starts(&["t4".into(), "t2".into()], &lib).unwrap().starts, ["t2", "t4"]);
    assert!(clean_starts(&["t1".into()], &lib).is_err(), "side A always starts on the first song");
    assert!(clean_starts(&["nope".into()], &lib).is_err());
    assert!(clean_starts(&[], &lib).unwrap().starts.is_empty());
}

fn device(dir: &std::path::Path, name: &str) -> (DbHandle, SyncStore, Sides) {
    let db = DbHandle::open(dir.join(format!("{name}.db"))).unwrap();
    db.write(|c| {
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('a1', 'The Band', 'Band')", [])?;
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, year) VALUES ('al', 'Lantern Songs', 'Lantern Songs', 'a1', 'The Band', 1997)",
            [],
        )?;
        for n in 1..=4 {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, disc_number, part_key)
                 VALUES (?1, ?2, 'al', 'Lantern Songs', 'a1', 'The Band', ?3, 1, ?4)",
                rusqlite::params![format!("t{n}"), format!("Song {n}"), n, format!("/p/t{n}")],
            )?;
        }
        Ok(())
    })
    .unwrap();
    let store = SyncStore::new(db.clone(), name);
    (db.clone(), store.clone(), Sides::new(db, store))
}

#[tokio::test]
async fn a_split_by_hand_is_stored_and_reaches_another_device() {
    let dir = tempfile::tempdir().unwrap();
    let (db, pc, sides) = device(dir.path(), "pc");
    let (_db2, laptop, laptop_sides) = device(dir.path(), "laptop");
    assert!(crate::sync::engine::Groups::default().namespaces().contains(&"albumSides"), "synced with the library group");

    // A cached vinyl edition gives the sides at first.
    let r = parse_release(&mb(&[("Vinyl", &["A1", "A2", "B1", "B2"])])).unwrap();
    db.write(move |c| cache::put(c, &album_key("al"), "musicbrainz", Some(&r), 1, 3600)).unwrap();
    let s = sides.get("al").await.unwrap();
    assert_eq!(s.source, Some(SidesSource::Musicbrainz));
    assert_eq!(s.sides.len(), 2);

    let s = sides.set("al", Some(vec!["t4".into(), "t3".into()])).await.unwrap();
    assert_eq!(labels(&s.sides), [("A", vec!["t1", "t2"]), ("B", vec!["t3"]), ("C", vec!["t4"])]);
    assert_eq!(pc.get(&crate::sync::store::ALBUM_SIDES, "al").unwrap().unwrap().starts, ["t3", "t4"]);
    assert!(sides.set("al", Some(vec!["t1".into()])).await.is_err(), "refused, and the split stays");
    assert_eq!(sides.get("al").await.unwrap().sides.len(), 3);

    // Through the sync file: the laptop sees the same split.
    let entries = pc.entries(&["albumSides"]).unwrap();
    laptop.merge(&entries, &["albumSides"]).unwrap();
    let there = laptop_sides.get("al").await.unwrap();
    assert_eq!(there.source, Some(SidesSource::Manual));
    assert_eq!(labels(&there.sides), [("A", vec!["t1", "t2"]), ("B", vec!["t3"]), ("C", vec!["t4"])]);

    // Back to the edition's sides.
    let back = sides.set("al", None).await.unwrap();
    assert_eq!(back.source, Some(SidesSource::Musicbrainz));
    assert!(pc.get(&crate::sync::store::ALBUM_SIDES, "al").unwrap().is_none());
    assert!(sides.get("nope").await.is_err(), "an album the library doesn't have");
}

#[tokio::test]
async fn playing_one_side_plays_only_its_songs_in_album_order() {
    use crate::audio::source::{ContextKind, PlayContext, TrackLookup};
    let dir = tempfile::tempdir().unwrap();
    let (db, _store, _sides) = device(dir.path(), "pc");
    let lookup = crate::bridge::CacheLookup(db, None, Default::default(), None);
    let side = PlayContext { kind: ContextKind::Album, id: Some("al".into()), track_ids: Some(vec!["t4".into(), "t3".into()]) };
    let r = lookup.context(&side).await.unwrap();
    assert_eq!(r.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t3", "t4"]);
    assert_eq!(r.title.as_deref(), Some("Lantern Songs"), "still the album");
    let whole = PlayContext { kind: ContextKind::Album, id: Some("al".into()), track_ids: None };
    assert_eq!(lookup.context(&whole).await.unwrap().tracks.len(), 4);
}
