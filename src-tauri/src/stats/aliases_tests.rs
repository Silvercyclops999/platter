//! One artist under two names: a shared MusicBrainz id or "Same artist as..."
//! makes their plays one artist's in top lists, counts, Wrapped and Play these.

use std::sync::Arc;

use rusqlite::{Connection, params};

use super::aliases::{self, Aliases};
use super::tests::{MIN, add_track, db, local_ms, play};
use super::*;
use crate::sync::SyncStore;

fn artist(c: &Connection, id: &str, title: &str, mbid: Option<&str>) {
    c.execute("INSERT INTO artists (id, title, sort_title, thumb, mbid) VALUES (?1, ?2, ?2, ?3, ?4)", params![id, title, format!("/a/{id}"), mbid])
        .unwrap();
}

/// An artist with one album and one song (`t<id>`).
fn act(c: &Connection, id: &str, title: &str, mbid: Option<&str>) {
    artist(c, id, title, mbid);
    let al = format!("al{id}");
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, thumb) VALUES (?1, ?2, ?2, ?3, ?4, ?5)",
        params![al, format!("{title} LP"), id, title, format!("/al/{al}")],
    )
    .unwrap();
    add_track(c, &format!("t{id}"), &format!("{title} song"), (&al, &format!("{title} LP")), (id, title), "[]");
}

fn plays(c: &Connection, track: &str, at: &str, n: u32) {
    let t = local_ms(c, at);
    for i in 0..n {
        play(c, track, t + i as i64 * 10 * MIN as i64, 4 * MIN);
    }
}

fn year(y: i64) -> Days {
    Days { lo: days_from_civil(y, 1, 1), hi: days_from_civil(y, 12, 31), all: false }
}

fn names(top: &[TopArtist]) -> Vec<(&str, i64)> {
    top.iter().map(|a| (a.name.as_str(), a.plays)).collect()
}

/// "Northwind" renamed itself "The Northwinds"; Plex holds both, matched to one id.
fn renamed(c: &mut Connection) {
    act(c, "1", "Northwind", Some("aaaa0000-0000-4000-8000-000000000001"));
    act(c, "2", "The Northwinds", Some("aaaa0000-0000-4000-8000-000000000001"));
    act(c, "3", "Copper Hollow", Some("bbbb0000-0000-4000-8000-000000000002"));
    plays(c, "t1", "2025-03-01 10:00:00", 3);
    plays(c, "t2", "2025-06-01 10:00:00", 5);
    plays(c, "t3", "2025-04-01 10:00:00", 6);
    refresh(c).unwrap();
}

#[test]
fn a_shared_musicbrainz_id_makes_one_artist_under_the_name_played_most() {
    let mut c = db();
    renamed(&mut c);
    let d = year(2025);
    let top = top_artists(&c, d, 10).unwrap();
    assert_eq!(names(&top), vec![("The Northwinds", 8), ("Copper Hollow", 6)]);
    assert_eq!(top[0].artist_id.as_deref(), Some("2"), "the row opens the name played most");
    assert_eq!(top[0].ms, 8 * 4 * MIN as i64);
    let o = overview(&c, d, 10).unwrap();
    assert_eq!(o.totals.artists, 2);
    assert_eq!(names(&o.top_artists), names(&top));
    // A limit counts merged rows.
    assert_eq!(names(&top_artists(&c, d, 1).unwrap()), vec![("The Northwinds", 8)]);
    // All time reads the whole-history table the same way.
    let all = Days { all: true, ..d };
    assert_eq!(names(&top_artists(&c, all, 10).unwrap()), vec![("The Northwinds", 8), ("Copper Hollow", 6)]);
    assert_eq!(distinct_artists(&c, all).unwrap(), 2);
}

#[test]
fn wrapped_and_recaps_count_the_artist_once() {
    let mut c = db();
    renamed(&mut c);
    let w = wrapped::wrapped(&c, 2025).unwrap();
    assert_eq!(names(&w.top_artists), vec![("The Northwinds", 8), ("Copper Hollow", 6)]);
    assert_eq!(w.totals.artists, 2);
    assert_eq!(w.new_artists, 2, "two names first played this year are one new artist");
    let r = month_recap(&c, "2025-06").unwrap();
    assert_eq!(names(&r.top_artists), vec![("The Northwinds", 5)]);
}

#[test]
fn an_artist_heard_before_under_the_old_name_is_not_new() {
    let mut c = db();
    renamed(&mut c);
    plays(&c, "t1", "2024-11-01 10:00:00", 1);
    refresh(&mut c).unwrap();
    let w = wrapped::wrapped(&c, 2025).unwrap();
    assert_eq!(w.new_artists, 1, "only Copper Hollow is new");
    let m = moments(&c, year(2025)).unwrap();
    assert_eq!(m.new_artists, Some(1));
    assert_eq!(m.discovery.map(|x| x.artist.name), Some("Copper Hollow".into()));
}

#[test]
fn play_these_takes_songs_under_both_names() {
    let mut c = db();
    renamed(&mut c);
    let ids = play_ids(&c, year(2025), TopKind::Artists, 10).unwrap();
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&"t1".to_string()) && ids.contains(&"t2".to_string()));
}

#[test]
fn different_ids_and_no_ids_stay_apart() {
    let mut c = db();
    act(&c, "1", "Lumen Drift", Some("cccc0000-0000-4000-8000-000000000003"));
    act(&c, "2", "Lumen Drift Trio", Some("dddd0000-0000-4000-8000-000000000004"));
    act(&c, "3", "Saltmarsh", None);
    act(&c, "4", "Saltmarsh Band", None);
    for t in ["t1", "t2", "t3", "t4"] {
        plays(&c, t, "2025-02-02 09:00:00", 2);
    }
    refresh(&mut c).unwrap();
    assert!(aliases::groups(&c).unwrap().is_empty());
    assert_eq!(top_artists(&c, year(2025), 10).unwrap().len(), 4);
}

#[test]
fn same_artist_as_merges_by_hand_and_undoes() {
    let dir = tempfile::tempdir().unwrap();
    let h = crate::db::DbHandle::open(dir.path().join("a.db")).unwrap();
    h.write(|c| {
        act(c, "1", "Kestrel Lane", None);
        act(c, "2", "Kestrel", None);
        act(c, "3", "Marrow Bell", None);
        plays(c, "t1", "2025-05-05 08:00:00", 2);
        plays(c, "t2", "2025-05-06 08:00:00", 4);
        plays(c, "t3", "2025-05-07 08:00:00", 3);
        refresh(c).map(|_| ())
    })
    .unwrap();
    let store = SyncStore::new(h.clone(), "dev1");
    let a = Aliases::with_clock(store.clone(), Arc::new(|| 1_700_000_000));
    let top = |h: &crate::db::DbHandle| h.read(|c| top_artists(c, year(2025), 10)).unwrap();
    assert_eq!(names(&top(&h)), vec![("Kestrel", 4), ("Marrow Bell", 3), ("Kestrel Lane", 2)]);

    let list = a.set("Kestrel Lane", "Kestrel Lane", "kestrel", "Kestrel", true).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].key.as_str(), list[0].same_as.as_str(), list[0].name.as_str()), ("kestrel lane", "kestrel", "Kestrel Lane"));
    assert_eq!(names(&top(&h)), vec![("Kestrel", 6), ("Marrow Bell", 3)]);
    assert_eq!(h.read(|c| distinct_artists(c, year(2025))).unwrap(), 2);
    // Kept in the sync store, keyed by the artist's stats key.
    assert!(store.get(&crate::sync::store::ARTIST_ALIASES, "kestrel lane").unwrap().is_some());

    // A third name joins the same artist through either side.
    a.set("marrow bell", "Marrow Bell", "kestrel lane", "Kestrel Lane", true).unwrap();
    assert_eq!(names(&top(&h)), vec![("Kestrel", 9)]);

    assert!(a.set("kestrel", "", "Kestrel", "", true).is_err(), "not the same as itself");
    assert!(a.set(" ", "", "kestrel", "", true).is_err());

    a.set("marrow bell", "", "", "", false).unwrap();
    let list = a.set("kestrel lane", "", "", "", false).unwrap();
    assert!(list.is_empty());
    assert_eq!(names(&top(&h)), vec![("Kestrel", 4), ("Marrow Bell", 3), ("Kestrel Lane", 2)]);
}

#[test]
fn played_artists_for_the_picker() {
    let mut c = db();
    renamed(&mut c);
    let all = aliases::played_artists(&c, "", 10).unwrap();
    assert_eq!(all.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["Copper Hollow", "The Northwinds", "Northwind"]);
    let some = aliases::played_artists(&c, "NORTH", 10).unwrap();
    assert_eq!(some.len(), 2);
}

#[test]
fn fold_adds_up_and_keeps_the_most_played_key() {
    let g: aliases::Groups = [("a", "a"), ("b", "a"), ("c", "a")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let rows = vec![("x".to_string(), 9, 1), ("b".to_string(), 5, 5), ("a".to_string(), 3, 3), ("c".to_string(), 2, 2)];
    assert_eq!(aliases::fold(&rows, &g), vec![("b".to_string(), 10, 10), ("x".to_string(), 9, 1)]);
    assert_eq!(aliases::members(&["b".to_string()], &g).len(), 3);
}

#[test]
fn homes_top_artists_list_one_artist_under_the_name_played_most() {
    let c = db();
    artist(&c, "1", "Northwind", Some("aaaa0000-0000-4000-8000-000000000001"));
    artist(&c, "2", "The Northwinds", Some("aaaa0000-0000-4000-8000-000000000001"));
    artist(&c, "3", "Copper Hollow", None);
    artist(&c, "4", "Saltmarsh", None);
    for (id, n) in [("1", 30), ("2", 12), ("3", 35), ("4", 0)] {
        c.execute("UPDATE artists SET view_count = ?2 WHERE id = ?1", params![id, n]).unwrap();
    }
    let top = crate::db::queries::top_artists(&c, 10).unwrap();
    assert_eq!(top.iter().map(|a| (a.title.as_str(), a.view_count)).collect::<Vec<_>>(), vec![("Northwind", 42), ("Copper Hollow", 35)]);
}

#[test]
fn sync_keeps_each_artists_musicbrainz_id() {
    let mut c = db();
    let m: crate::plex::client::PlexMetadata = serde_json::from_value(serde_json::json!({
        "ratingKey": "7", "title": "Northwind", "type": "artist",
        "Guid": [{"id": "mbid://AAAA0000-0000-4000-8000-000000000001"}]
    }))
    .unwrap();
    let a = m.to_artist();
    assert_eq!(a.mbid.as_deref(), Some("AAAA0000-0000-4000-8000-000000000001"));
    let tx = c.transaction().unwrap();
    crate::db::store::upsert_artists(&tx, &[a], 1).unwrap();
    tx.commit().unwrap();
    let got: Option<String> = c.query_row("SELECT mbid FROM artists WHERE id = '7'", [], |r| r.get(0)).unwrap();
    assert_eq!(got.as_deref(), Some("aaaa0000-0000-4000-8000-000000000001"), "kept lower-case so ids compare");
}
