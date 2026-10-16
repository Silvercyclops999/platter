//! One-click fixes: preview, apply, undo, with a fake catalogue.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;

use super::fields::Field;
use super::fixes::{FixKind, LOOKUP_DOWN, lead_key, title_key, to_request};
use super::lookup::{CatTrack, Candidate, Catalog, LookupError, deezer_album_tracks, deezer_isrc, deezer_releases, mb_releases, mb_tracks};
use super::testkit::{Lib, values};

const A: &str = "Demo Artist/Demo Album";

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

/// Three songs, one disc, no disc tags (the fixtures carry none).
fn lib3() -> Lib {
    let lib = Lib::new();
    lib.add("t1", "al1", "Demo Album", &format!("{A}/01 - Demo Song.flac"), "song.flac", 1, 1, "Demo Song");
    lib.add("t2", "al1", "Demo Album", &format!("{A}/02 - Other Song.flac"), "song.flac", 1, 2, "Other Song");
    lib.add("t3", "al1", "Demo Album", &format!("{A}/03 - Third Song.flac"), "song.flac", 1, 3, "Third Song");
    lib.retag(&format!("{A}/02 - Other Song.flac"), &[(Field::Title, "Other Song")]);
    lib.retag(&format!("{A}/03 - Third Song.flac"), &[(Field::Title, "Third Song")]);
    lib
}

#[test]
fn disc_tags_fill_one_of_one_and_undo() {
    let lib = lib3();
    let p = lib.editor.fix_preview(FixKind::DiscTags, "al1").unwrap();
    assert_eq!(p.changes.len(), 6, "disc number and total on three songs");
    assert!(p.changes.iter().all(|c| c.after == "1" && c.before.is_none()));
    assert!(!p.pick_each);
    let r = rt().block_on(lib.editor.save(to_request(&p.label, &p.changes))).unwrap();
    assert_eq!(r.saved, 3);
    let v = values(&lib.local(&format!("{A}/02 - Other Song.flac")));
    assert_eq!((v.get(&Field::DiscNumber).map(String::as_str), v.get(&Field::DiscTotal).map(String::as_str)), (Some("1"), Some("1")));
    assert_eq!(lib.editor.history()[0].label, "Filled disc tags on Demo Album");
    // Done: nothing more to fill.
    assert!(lib.editor.fix_preview(FixKind::DiscTags, "al1").unwrap().changes.is_empty());
    rt().block_on(lib.editor.undo(&r.undo_id.unwrap())).unwrap();
    assert!(!values(&lib.local(&format!("{A}/02 - Other Song.flac"))).contains_key(&Field::DiscNumber));
}

#[test]
fn disc_tags_leave_multi_disc_folders_alone() {
    let lib = Lib::new();
    lib.add("t1", "al1", "Demo Album", &format!("{A}/1-01 - Demo Song.flac"), "song.flac", 1, 1, "Demo Song");
    lib.add("t2", "al1", "Demo Album", &format!("{A}/2-01 - Other Song.flac"), "song.flac", 2, 1, "Other Song");
    let p = lib.editor.fix_preview(FixKind::DiscTags, "al1").unwrap();
    assert!(p.changes.is_empty());
    assert_eq!(p.kept.len(), 2);
    assert!(p.kept[0].reason.contains("more than one disc"));
}

#[test]
fn release_status_is_suggested_per_file_from_the_same_disc() {
    let lib = lib3();
    // The fixtures say ARCHIVE_VERSION=leak; one song has none, and one
    // song on disc 2 has nothing beside it to go by.
    lib.retag(&format!("{A}/03 - Third Song.flac"), &[(Field::ArchiveVersion, "")]);
    lib.add("t4", "al1", "Demo Album", &format!("{A}/04 - Bonus.flac"), "song.flac", 2, 1, "Bonus");
    lib.retag(&format!("{A}/04 - Bonus.flac"), &[(Field::ArchiveVersion, ""), (Field::Title, "Bonus"), (Field::DiscNumber, "2")]);
    let p = lib.editor.fix_preview(FixKind::ReleaseStatus, "al1").unwrap();
    assert!(p.pick_each, "nothing is ticked for the user");
    assert_eq!(p.changes.len(), 1);
    let c = &p.changes[0];
    assert_eq!((c.track_id.as_str(), c.field, c.after.as_str()), ("t3", Field::ArchiveVersion, "leak"));
    assert_eq!(c.note.as_deref(), Some("Like the other 2 songs on disc 1"));
    assert_eq!(p.kept.len(), 1);
    assert!(p.kept[0].reason.contains("disc 2"), "disc 2 isn't assumed to be anything");
    let r = rt().block_on(lib.editor.save(to_request(&p.label, &p.changes))).unwrap();
    assert_eq!(r.saved, 1);
    assert_eq!(values(&lib.local(&format!("{A}/03 - Third Song.flac"))).get(&Field::ArchiveVersion).map(String::as_str), Some("leak"));
    rt().block_on(lib.editor.undo(&r.undo_id.unwrap())).unwrap();
    assert!(!values(&lib.local(&format!("{A}/03 - Third Song.flac"))).contains_key(&Field::ArchiveVersion));
}

struct FakeCatalog {
    down: bool,
    tracks: Vec<CatTrack>,
    asked: Mutex<Vec<String>>,
}

#[async_trait]
impl Catalog for FakeCatalog {
    async fn releases(&self, artist: &str, album: &str) -> Result<Vec<Candidate>, LookupError> {
        self.asked.lock().push(format!("{artist} / {album}"));
        if self.down {
            return Err(LookupError::Down("unreachable".into()));
        }
        Ok(vec![release()])
    }
    async fn tracks(&self, _: &Candidate) -> Result<Vec<CatTrack>, LookupError> {
        if self.down {
            return Err(LookupError::Down("unreachable".into()));
        }
        Ok(self.tracks.clone())
    }
}

fn release() -> Candidate {
    Candidate {
        source: "musicbrainz".into(),
        id: "11111111-2222-3333-4444-555555555555".into(),
        title: "Demo Album".into(),
        artist: "Demo Artist".into(),
        date: Some("2019".into()),
        country: Some("XW".into()),
        tracks: 3,
    }
}

fn cat(title: &str, artist: &str, ms: i64, isrc: Option<&str>) -> CatTrack {
    CatTrack { title: title.into(), artist: artist.into(), duration_ms: Some(ms), isrc: isrc.map(str::to_string) }
}

#[test]
fn isrcs_come_from_one_confirmed_release_and_differing_lengths_are_kept() {
    let lib = lib3();
    for f in ["01 - Demo Song", "02 - Other Song", "03 - Third Song"] {
        lib.retag(&format!("{A}/{f}.flac"), &[(Field::ArchiveVersion, "released")]);
    }
    lib.editor.set_catalog(Arc::new(FakeCatalog {
        down: false,
        asked: Mutex::new(Vec::new()),
        tracks: vec![
            // Plex says each song is 1.000 s long.
            cat("Demo Song (feat. Other Artist)", "Demo Artist feat. Other Artist", 2200, Some("USRC17607839")),
            cat("Other Song", "Demo Artist", 9000, Some("GBAYE0601498")),
        ],
    }));
    let r = rt().block_on(lib.editor.isrc_releases("al1")).unwrap();
    assert_eq!((r.wanted, r.candidates.len()), (3, 1));
    let p = rt().block_on(lib.editor.isrc_preview("al1", &r.candidates[0])).unwrap();
    assert_eq!(p.changes.len(), 1, "{p:?}");
    assert_eq!((p.changes[0].track_id.as_str(), p.changes[0].after.as_str()), ("t1", "USRC17607839"));
    let reasons: Vec<&str> = p.kept.iter().map(|k| k.reason.as_str()).collect();
    assert!(reasons.iter().any(|r| r.contains("differs from the release's (0:09)")), "{reasons:?}");
    assert!(reasons.iter().any(|r| r.contains("No song on \"Demo Album\" matches")), "{reasons:?}");
    let s = rt().block_on(lib.editor.save(to_request(&p.label, &p.changes))).unwrap();
    assert_eq!(s.saved, 1);
    assert_eq!(values(&lib.local(&format!("{A}/01 - Demo Song.flac"))).get(&Field::Isrc).map(String::as_str), Some("USRC17607839"));
    assert_eq!(values(&lib.local(&format!("{A}/01 - Demo Song.flac"))).get(&Field::Title).map(String::as_str), Some("Demo Song"), "credits never change");
    rt().block_on(lib.editor.undo(&s.undo_id.unwrap())).unwrap();
    assert!(!values(&lib.local(&format!("{A}/01 - Demo Song.flac"))).contains_key(&Field::Isrc));
}

#[test]
fn isrc_lookup_skips_songs_that_arent_released_and_says_when_the_service_is_down() {
    let lib = lib3();
    lib.editor.set_catalog(Arc::new(FakeCatalog { down: true, asked: Mutex::new(Vec::new()), tracks: Vec::new() }));
    // The fixtures are leaks: nothing to look up, and nothing asked.
    let r = rt().block_on(lib.editor.isrc_releases("al1")).unwrap();
    assert_eq!(r.wanted, 0);
    assert!(r.candidates.is_empty());
    lib.retag(&format!("{A}/01 - Demo Song.flac"), &[(Field::ArchiveVersion, "released")]);
    let e = rt().block_on(lib.editor.isrc_releases("al1")).unwrap_err();
    assert!(e.to_string().ends_with(LOOKUP_DOWN), "{e}");
    let e = rt().block_on(lib.editor.isrc_preview("al1", &release())).unwrap_err();
    assert!(e.to_string().ends_with(LOOKUP_DOWN), "{e}");
}

#[test]
fn mark_an_album_or_an_era_explicit() {
    let lib = lib3();
    lib.retag(&format!("{A}/01 - Demo Song.flac"), &[(Field::Explicit, "explicit")]);
    let p = lib.editor.explicit_preview(&["al1".to_string()], true).unwrap();
    assert_eq!(p.changes.len(), 2, "one already is");
    assert_eq!(p.label, "Marked Demo Album as explicit");
    let r = rt().block_on(lib.editor.save(to_request(&p.label, &p.changes))).unwrap();
    assert_eq!(r.saved, 2);
    let p = lib.editor.explicit_preview(&["al1".to_string()], false).unwrap();
    assert_eq!(p.changes.len(), 3);
    // Eras come from the tag cache, which saving updated.
    rt().block_on(lib.editor.save(to_request("x", &lib.editor.explicit_preview(&["al1".into()], true).unwrap().changes))).unwrap();
    let d = lib.editor.open(Some("al1"), &[]).unwrap();
    let files = super::editor::spread(&d.files, &[(Field::Era, "Demo Era".to_string())].into_iter().collect());
    rt().block_on(lib.editor.save(super::editor::SaveRequest { label: "x".into(), files })).unwrap();
    assert_eq!(lib.editor.era_albums("ar1", "Demo Era").unwrap(), ["al1"]);
    assert!(lib.editor.era_albums("ar1", "Other Era").unwrap().is_empty());
}

#[test]
fn titles_and_artists_compare_without_features_or_punctuation() {
    assert_eq!(title_key("Demo Song (feat. Other Artist)"), "demo song");
    assert_eq!(title_key("Demo Song feat. Other Artist"), "demo song");
    assert_eq!(title_key("Demo-Song!"), "demo song");
    assert_eq!(lead_key("Demo Artist, Other Artist"), "demo artist");
    assert_eq!(lead_key("Demo Artist & Other Artist"), "demo artist");
}

#[test]
fn catalogue_answers_are_read() {
    let mb = json!({"releases": [
        {"id": "11111111-2222-3333-4444-555555555555", "status": "Official", "title": "Demo Album", "date": "2019-05-31", "country": "XW",
         "track-count": 12, "artist-credit": [{"name": "Demo Artist", "joinphrase": " & "}, {"name": "Other Artist"}]},
        {"id": "21111111-2222-3333-4444-555555555555", "status": "Bootleg", "title": "Demo Album"},
        {"id": "not-an-id", "status": "Official", "title": "Demo Album"}
    ]});
    let r = mb_releases(&mb);
    assert_eq!(r.len(), 1, "official releases with real ids only");
    assert_eq!(r[0].artist, "Demo Artist & Other Artist");
    assert_eq!(r[0].tracks, 12);
    let rel = json!({"media": [{"tracks": [
        {"title": "Demo Song", "length": 200000, "artist-credit": [{"name": "Demo Artist"}], "recording": {"isrcs": ["USRC17607839"]}},
        {"title": "Two Masters", "length": 1000, "recording": {"isrcs": ["USRC17607839", "GBAYE0601498"], "artist-credit": [{"name": "Demo Artist"}]}}
    ]}]});
    let t = mb_tracks(&rel);
    assert_eq!(t[0], cat("Demo Song", "Demo Artist", 200000, Some("USRC17607839")));
    assert_eq!(t[1].isrc, None, "two ISRCs for one recording are ambiguous");
    assert_eq!(t[1].artist, "Demo Artist");
    let dz = json!({"data": [{"id": 302127, "title": "Demo Album", "artist": {"name": "Demo Artist"}, "nb_tracks": 14, "release_date": "2019-05-31"}]});
    assert_eq!(deezer_releases(&dz)[0].id, "302127");
    let album = json!({"tracks": {"data": [{"id": 3135556, "title": "Demo Song", "duration": 200, "artist": {"name": "Demo Artist"}}]}});
    assert_eq!(deezer_album_tracks(&album), vec![(3135556, "Demo Song".to_string(), "Demo Artist".to_string(), Some(200))]);
    assert_eq!(deezer_isrc(&json!({"isrc": "GBDUW0000059"})).as_deref(), Some("GBDUW0000059"));
    assert_eq!(deezer_isrc(&json!({"isrc": ""})), None);
}
