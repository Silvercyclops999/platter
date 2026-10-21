//! Crates: folding names, grouping (label variants, producers once per
//! album, studios, era buckets, crates of one left out), the album page
//! shelves and the view filter.

use std::collections::HashMap;

use rusqlite::{Connection, params};

use super::commands::{index_for, list, page, shelves};
use super::*;
use crate::db::merge::ServerView;
use crate::db::schema::migrate;
use crate::tags::TagView;

fn db() -> Connection {
    let mut c = Connection::open_in_memory().unwrap();
    migrate(&mut c).unwrap();
    c
}

/// An album with `songs` songs `{id}-1`.., each on part `/p/{id}-n`.
fn album(c: &Connection, id: &str, artist: &str, year: Option<i32>, label: Option<&str>, genres: &[&str], songs: u32) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, year, studio, genres, track_count)
         VALUES (?1, ?1, ?1, ?2, ?2, ?3, ?4, ?5, ?6)",
        params![id, artist, year, label, serde_json::to_string(genres).unwrap(), songs],
    )
    .unwrap();
    for n in 1..=songs {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, duration_ms, track_number)
             VALUES (?1, ?1, ?2, ?2, ?3, ?3, ?4, 1000, ?5)",
            params![format!("{id}-{n}"), id, artist, format!("/p/{id}-{n}"), n],
        )
        .unwrap();
    }
}

fn tag_credit(c: &Connection, part: &str, name: &str, role: &str) {
    c.execute(
        "INSERT INTO tag_credits (part_key, person_key, name, role) VALUES (?1, ?2, ?3, ?4)",
        params![part, crate::artistinfo::fold_name(name), name, role],
    )
    .unwrap();
}

fn file_label(c: &Connection, part: &str, label: &str) {
    c.execute(
        "INSERT INTO file_tags (part_key, status, scanned_at, ver, sleeve) VALUES (?1, 'ok', 1, 4, ?2)",
        params![part, serde_json::json!({ "label": label }).to_string()],
    )
    .unwrap();
}

fn fact(c: &Connection, album: &str, kind: &str, name: &str) {
    let key = if kind == "label" { label_key(name) } else { studio_key(name) };
    c.execute("INSERT INTO album_facts (album_id, kind, fact_key, name) VALUES (?1, ?2, ?3, ?4)", params![album, kind, key, name]).unwrap();
}

fn all(c: &Connection) -> Index {
    index_for(c, TagView::from_bits(0), &ServerView::All).unwrap()
}

fn names(ix: &Index, kind: CrateKind) -> Vec<(String, Vec<String>)> {
    let mut v: Vec<(String, Vec<String>)> = ix.of_kind(kind).map(|e| (e.name.clone(), e.album_ids.clone())).collect();
    v.sort();
    v
}

#[test]
fn label_names_fold_their_variants() {
    assert_eq!(label_key("Low Tide Records"), "low tide");
    assert_eq!(label_key("LOW TIDE"), "low tide");
    assert_eq!(label_key("Low-Tide Recordings Ltd."), "low tide");
    assert_eq!(label_key("The Low Tide Music Group"), "low tide");
    assert_eq!(label_key("Records"), "records", "a name that is only an ending stays");
    assert_ne!(label_key("Low Tide"), label_key("High Tide"));
    assert_eq!(studio_key("Electric Lady Studios"), studio_key("electric lady"));
    assert_eq!(split_labels("North Star / Low Tide; [no label]"), ["North Star", "Low Tide"]);
    assert_eq!(decade(Some(1994)), Some(1990));
    assert_eq!(decade(Some(2000)), Some(2000));
    assert_eq!(decade(Some(0)), None);
    assert_eq!(era_name(1990), "1990s");
}

#[test]
fn grouping_names_by_the_spelling_used_most_and_leaves_crates_of_one_out() {
    let m = |key: &str, name: &str, album: &str| Member::new(CrateKind::Label, key.into(), name, album);
    let order: HashMap<String, Order> = ["a", "b", "c", "d"]
        .iter()
        .enumerate()
        .map(|(i, id)| (id.to_string(), Order { year: 2000 - i as i32, artist: format!("artist {id}"), title: id.to_string() }))
        .collect();
    let members = [
        m("low tide", "Low Tide Records", "a"),
        m("low tide", "Low Tide", "b"),
        m("low tide", "Low Tide", "c"),
        m("low tide", "Low Tide", "c"),
        m("solo", "Solo", "d"),
        // Not in the library (or hidden): never counted.
        m("solo", "Solo", "gone"),
    ];
    let crates = group(&members, &order);
    assert_eq!(crates.len(), 1, "a crate of one is no crate");
    let lt = &crates[0];
    assert_eq!(lt.name, "Low Tide");
    assert_eq!(lt.album_ids, ["c", "b", "a"], "each album once, by year");
    assert_eq!(lt.years, Some((1998, 2000)));
    assert_eq!(lt.artists, 3);
}

#[test]
fn labels_come_from_notes_then_tags_then_plex_with_variants_in_one_crate() {
    let c = db();
    // Plex's label only.
    album(&c, "a1", "Ada", Some(1994), Some("Low Tide Records"), &[], 2);
    // The files say otherwise, and win over Plex.
    album(&c, "a2", "Bo", Some(1996), Some("Somebody Else"), &[], 3);
    file_label(&c, "/p/a2-1", "LOW TIDE");
    file_label(&c, "/p/a2-2", "Low Tide");
    file_label(&c, "/p/a2-3", "Low Tide");
    // Sleeve notes win over both, and can name two labels.
    album(&c, "a3", "Cy", Some(2003), Some("Somebody Else"), &[], 1);
    fact(&c, "a3", "label", "Low Tide Recordings");
    fact(&c, "a3", "label", "North Star");
    album(&c, "a4", "Di", Some(2005), Some("North Star Music"), &[], 1);
    // One album alone on its label.
    album(&c, "a5", "Ed", None, Some("Tiny"), &[], 1);
    let ix = all(&c);
    let labels = names(&ix, CrateKind::Label);
    assert_eq!(
        labels,
        [("Low Tide".to_string(), vec!["a1".to_string(), "a2".into(), "a3".into()]), ("North Star".to_string(), vec!["a3".to_string(), "a4".into()])],
        "Somebody Else and Tiny hold one album each (a2's files and a3's notes overrule Plex)"
    );
}

#[test]
fn producers_across_songs_count_once_per_album_from_tags_and_notes() {
    let c = db();
    album(&c, "a1", "Ada", Some(1990), None, &[], 3);
    album(&c, "a2", "Bo", Some(1992), None, &[], 2);
    album(&c, "a3", "Cy", Some(1999), None, &[], 2);
    for n in 1..=3 {
        tag_credit(&c, &format!("/p/a1-{n}"), "Max Volume", "producer");
    }
    tag_credit(&c, "/p/a2-1", "MAX VOLUME", "producer");
    tag_credit(&c, "/p/a2-1", "Wren", "engineer");
    tag_credit(&c, "/p/a3-1", "Wren", "engineer");
    // Sleeve notes credit a producer on the album itself.
    c.execute(
        "INSERT INTO sleeve_credits (album_id, track_id, person_key, name, role, sources) VALUES ('a3', '', 'max volume', 'Max Volume', 'producer', '[\"musicbrainz\"]')",
        [],
    )
    .unwrap();
    let ix = all(&c);
    let p = ix.find(CrateKind::Producer, "max volume").expect("a producer crate");
    assert_eq!(p.album_ids, ["a1", "a2", "a3"], "three albums, not six songs");
    assert_eq!(p.name, "Max Volume");
    assert!(ix.find(CrateKind::Producer, "wren").is_none(), "engineers aren't producer crates");
    let l = list(&c, &ix, CrateKind::Producer).unwrap();
    assert_eq!(l.crates[0].count, 3);
}

#[test]
fn studios_eras_years_and_genres() {
    let c = db();
    album(&c, "a1", "Ada", Some(1971), None, &["Rock"], 1);
    album(&c, "a2", "Bo", Some(1979), None, &["rock", "Jazz"], 1);
    album(&c, "a3", "Cy", Some(1980), None, &["Jazz"], 1);
    album(&c, "a4", "Di", Some(1979), None, &[], 1);
    album(&c, "a5", "Ed", None, None, &[], 1);
    fact(&c, "a1", "studio", "Electric Lady Studios");
    fact(&c, "a3", "studio", "Electric Lady");
    fact(&c, "a2", "studio", "Sun Studio");
    let ix = all(&c);
    assert_eq!(names(&ix, CrateKind::Studio), [("Electric Lady".to_string(), vec!["a1".to_string(), "a3".into()])]);
    assert_eq!(
        names(&ix, CrateKind::Era),
        [("1970s".to_string(), vec!["a1".to_string(), "a2".into(), "a4".into()])],
        "the 1980s hold one album; no year, no era"
    );
    assert_eq!(names(&ix, CrateKind::Year), [("1979".to_string(), vec!["a2".to_string(), "a4".into()])]);
    let genres = names(&ix, CrateKind::Genre);
    assert_eq!(genres.len(), 2);
    assert_eq!(genres[1], ("Rock".to_string(), vec!["a1".to_string(), "a2".into()]), "Rock and rock are one genre");
    // The era's page lists its years that are crates.
    let era = page(&c, &ix, CrateKind::Era, "1970").unwrap().unwrap();
    assert_eq!(era.years, [1979]);
    assert_eq!(era.track_ids, ["a1-1", "a2-1", "a4-1"], "Play all goes album by album, by year");
    assert!(era.albums.iter().all(|a| !a.on_map), "nothing analysed yet");
    assert_eq!((era.from_year, era.to_year), (Some(1971), Some(1979)));
    assert!(page(&c, &ix, CrateKind::Era, "1980").unwrap().is_none(), "no crate, no page");
}

#[test]
fn genres_that_differ_by_spaces_hyphens_or_case_are_one_crate() {
    let c = db();
    album(&c, "a1", "Ada", Some(1990), None, &["Hip-Hop"], 1);
    album(&c, "a2", "Bo", Some(1991), None, &["Hip-Hop"], 1);
    album(&c, "a3", "Cy", Some(1992), None, &["Hip Hop"], 1);
    album(&c, "a4", "Di", Some(1993), None, &["hiphop"], 1);
    album(&c, "a5", "Ed", Some(1994), None, &["Hip-Hop/Rap", "Rap"], 1);
    album(&c, "a6", "Fi", Some(1995), None, &["Rap"], 1);
    let ix = all(&c);
    let genres = names(&ix, CrateKind::Genre);
    let hip = genres.iter().find(|(n, _)| n == "Hip-Hop").expect("one Hip-Hop crate, named as most albums spell it");
    assert_eq!(hip.1, ["a1", "a2", "a3", "a4"]);
    assert!(!genres.iter().any(|(n, _)| n == "Hip Hop" || n == "hiphop"), "{genres:?}");
    assert_eq!(super::genre_key("Hip-Hop"), super::genre_key("hip hop"));
    assert_ne!(super::genre_key("Hip-Hop"), super::genre_key("Hip-Hop/Rap"));
}

#[test]
fn album_shelves_list_the_other_albums_other_artists_first_and_skip_empty_crates() {
    let c = db();
    album(&c, "a1", "Ada", Some(1990), Some("Low Tide"), &[], 1);
    album(&c, "a2", "Ada", Some(1991), Some("Low Tide"), &[], 1);
    album(&c, "a3", "Bo", Some(1999), Some("Low Tide"), &[], 1);
    album(&c, "a4", "Cy", Some(1999), Some("Tiny"), &[], 1);
    tag_credit(&c, "/p/a1-1", "Max Volume", "producer");
    tag_credit(&c, "/p/a4-1", "Max Volume", "producer");
    fact(&c, "a1", "studio", "Electric Lady");
    let ix = all(&c);
    let s = shelves(&c, &ix, "a1").unwrap();
    let got: Vec<(CrateKind, &str, Vec<&str>)> = s.iter().map(|x| (x.kind, x.name.as_str(), x.albums.iter().map(|a| a.id.as_str()).collect())).collect();
    assert_eq!(got, [(CrateKind::Label, "Low Tide", vec!["a3", "a2"]), (CrateKind::Producer, "Max Volume", vec!["a4"])], "no studio crate: a1 is alone there");
    assert_eq!(s[0].count, 3);
    assert!(shelves(&c, &ix, "a4").unwrap().iter().all(|x| x.kind == CrateKind::Producer), "Tiny holds only a4");
}

#[test]
fn sleeve_notes_keep_the_label_and_studios_for_crates() {
    use crate::sleeve::{Fact, SleeveNotes, Source, Studio};
    let c = db();
    let mut notes = crate::sleeve::merge("a1", &[], &[], &HashMap::new());
    notes.label = Some(Fact { value: "Low Tide Records / North Star".into(), sources: vec![Source::Musicbrainz] });
    notes.studios = vec![Studio { name: "Electric Lady Studios".into(), role: "recorded".into(), sources: vec![Source::Musicbrainz] }];
    crate::sleeve::store::index_album(&c, &notes).unwrap();
    let rows: Vec<(String, String, String)> = c
        .prepare("SELECT kind, fact_key, name FROM album_facts WHERE album_id = 'a1' ORDER BY kind, fact_key")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        [
            ("label".into(), "low tide".into(), "Low Tide Records".into()),
            ("label".into(), "north star".into(), "North Star".into()),
            ("studio".into(), "electric lady".into(), "Electric Lady Studios".into()),
        ]
    );
    // Put together again with less: the old facts go.
    let bare: SleeveNotes = crate::sleeve::merge("a1", &[], &[], &HashMap::new());
    crate::sleeve::store::index_album(&c, &bare).unwrap();
    let n: i64 = c.query_row("SELECT count(*) FROM album_facts", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn hidden_albums_and_other_servers_copies_leave_crates() {
    let c = db();
    album(&c, "aaaaaaaa:1", "Ada", Some(1990), Some("Low Tide"), &[], 1);
    album(&c, "aaaaaaaa:2", "Bo", Some(1991), Some("Low Tide"), &[], 1);
    album(&c, "bbbbbbbb:3", "Cy", Some(1992), Some("Low Tide"), &[], 1);
    let both = index_for(&c, TagView::from_bits(0), &ServerView::All).unwrap();
    assert_eq!(both.find(CrateKind::Label, "low tide").unwrap().album_ids.len(), 3);
    let one = index_for(&c, TagView::from_bits(0), &ServerView::Server("bbbbbbbb".into())).unwrap();
    assert!(one.find(CrateKind::Label, "low tide").is_none(), "one album on that server: no crate");
}

#[test]
fn the_stamp_moves_when_crates_would() {
    let c = db();
    let before = stamp(&c).unwrap();
    album(&c, "a1", "Ada", Some(1990), Some("Low Tide"), &[], 1);
    let added = stamp(&c).unwrap();
    assert_ne!(before, added);
    c.execute("UPDATE albums SET studio = 'North Star' WHERE id = 'a1'", []).unwrap();
    assert_ne!(added, stamp(&c).unwrap(), "a new label from Plex");
    let relabelled = stamp(&c).unwrap();
    fact(&c, "a1", "studio", "Sun");
    assert_ne!(relabelled, stamp(&c).unwrap());
}
