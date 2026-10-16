//! Mode detection, one signal at a time.

use std::collections::{BTreeMap, HashMap};

use super::modes::*;
use crate::db::DbHandle;

const A_MBID: &str = "a0000000-0000-0000-0000-000000000001";
const B_MBID: &str = "b0000000-0000-0000-0000-000000000002";
const C_MBID: &str = "c0000000-0000-0000-0000-000000000003";
const RG_JOINT: &str = "70000000-0000-0000-0000-000000000007";

fn artist(id: &str, name: &str, mbid: Option<&str>, files: usize, albums: &[(&str, &str, Option<&str>)]) -> LibraryArtist {
    LibraryArtist {
        artist_id: id.into(),
        name: name.into(),
        mbid: mbid.map(str::to_string),
        files,
        albums: albums
            .iter()
            .map(|(t, f, rg)| LibraryAlbumDir { title: t.to_string(), folder: f.to_string(), release_group: rg.map(str::to_string) })
            .collect(),
        ..Default::default()
    }
}

fn clean() -> LibraryArtist {
    artist("ar1", "Northlight", Some(A_MBID), 24, &[("First Light", "/data/media/Music/Lidarr/Northlight/First Light (2019)", None)])
}

fn find<'a>(m: &'a [ArtistMode], name: &str) -> &'a ArtistMode {
    m.iter().find(|x| x.name == name).unwrap_or_else(|| panic!("{name} missing"))
}

#[test]
fn official_files_with_no_curated_tags_are_managed() {
    let m = assign(&ModeInputs { library: vec![clean()], ..Default::default() });
    let a = find(&m, "Northlight");
    assert_eq!(a.mode, Mode::Managed);
    assert_eq!(a.line, "Managed: all files official, no curated tags");
    assert_eq!(a.key, A_MBID);
}

#[test]
fn files_tagged_unreleased_or_leaked_protect_the_artist() {
    let mut a = artist("ar2", "Quiet Hours", Some(B_MBID), 500, &[("Sessions", "/data/media/Music/Archive/Quiet Hours/Sessions", None)]);
    a.curated = BTreeMap::from([("unreleased".to_string(), 400), ("leaked".to_string(), 3)]);
    let m = assign(&ModeInputs { library: vec![a, clean()], ..Default::default() });
    let q = find(&m, "Quiet Hours");
    assert_eq!(q.mode, Mode::Protected);
    assert_eq!(q.line, "Protected: 403 files tagged unreleased or leaked");
    assert_eq!(q.curated_files, 403);
    assert_eq!(m[0].name, "Quiet Hours", "protected artists list first");
}

#[test]
fn a_demo_typed_album_protects_the_artist() {
    let mut a = clean();
    a.typed_unreleased = vec!["Night Demos".into()];
    let m = assign(&ModeInputs { library: vec![a], ..Default::default() });
    assert_eq!(m[0].mode, Mode::Protected);
    assert_eq!(m[0].line, "Protected: 1 album typed demo, unreleased or leak");
}

#[test]
fn an_excluded_artist_in_the_library_is_protected() {
    let m = assign(&ModeInputs { library: vec![clean()], exclusions: vec![A_MBID.to_uppercase()], ..Default::default() });
    assert_eq!(m[0].mode, Mode::Protected);
    assert!(m[0].excluded);
    assert_eq!(m[0].line, "Protected: on Lidarr's import list exclusions");
}

#[test]
fn the_users_choice_wins_either_way() {
    let mut curated = clean();
    curated.curated = BTreeMap::from([("unreleased".to_string(), 2)]);
    let prefs = HashMap::from([(A_MBID.to_string(), ArtistPrefs { mode: ModeSetting::Managed, ..Default::default() })]);
    let m = assign(&ModeInputs { library: vec![curated], prefs, ..Default::default() });
    assert_eq!(m[0].mode, Mode::Managed);
    assert_eq!(m[0].auto, Mode::Protected);
    assert!(m[0].line.starts_with("Managed: you chose Managed (Auto would protect it: 2 files tagged unreleased"), "{}", m[0].line);

    let prefs = HashMap::from([(A_MBID.to_string(), ArtistPrefs { mode: ModeSetting::Protected, ..Default::default() })]);
    let m = assign(&ModeInputs { library: vec![clean()], prefs, ..Default::default() });
    assert_eq!(m[0].mode, Mode::Protected);
    assert_eq!(m[0].line, "Protected: you chose Protected");
}

#[test]
fn albums_inside_a_protected_folder_protect_their_artists_and_release_groups() {
    let mut owner = artist(
        "ar2",
        "Quiet Hours",
        Some(B_MBID),
        30,
        &[("Sessions", "/data/media/Music/Archive/Quiet Hours/Sessions", None), ("Joint Tape", "/data/media/Music/Archive/Quiet Hours/Joint Tape", Some(RG_JOINT))],
    );
    owner.curated = BTreeMap::from([("unreleased".to_string(), 30)]);
    // A collaboration Plex files under its own artist, but the folder is the protected artist's.
    let collab = artist("ar3", "Quiet Hours & Harbor Sound", None, 8, &[("Joint Tape", "/data/media/Music/Archive/Quiet Hours/Joint Tape", Some(RG_JOINT))]);
    // The same release group elsewhere in the library.
    let elsewhere = artist("ar4", "Harbor Sound", Some(C_MBID), 10, &[("Joint Tape", "/data/media/Music/Lidarr/Harbor Sound/Joint Tape", Some(RG_JOINT))]);
    let m = assign(&ModeInputs { library: vec![owner, collab, elsewhere, clean()], ..Default::default() });
    let c = find(&m, "Quiet Hours & Harbor Sound");
    assert_eq!(c.mode, Mode::Protected);
    assert_eq!(c.line, "Protected: has Joint Tape inside Quiet Hours's protected folder");
    let h = find(&m, "Harbor Sound");
    assert_eq!(h.mode, Mode::Protected);
    assert_eq!(h.line, "Protected: shares Joint Tape with protected Quiet Hours");
    assert_eq!(find(&m, "Northlight").mode, Mode::Managed);
}

#[test]
fn a_lidarr_artist_credited_on_a_protected_release_group_is_protected() {
    let mut owner = artist("ar2", "Quiet Hours", Some(B_MBID), 3, &[("Joint Tape", "/data/media/Music/Archive/Quiet Hours/Joint Tape", Some(RG_JOINT))]);
    owner.curated = BTreeMap::from([("leaked".to_string(), 3)]);
    let m = assign(&ModeInputs {
        library: vec![owner],
        lidarr_artists: vec![LidarrArtistRef { id: 9, name: "Glasshouse".into(), mbid: C_MBID.into(), path: "/data/staging/music/Glasshouse".into() }],
        lidarr_albums: vec![LidarrAlbumRef { artist_id: 9, release_group: RG_JOINT.to_uppercase(), title: "Joint Tape".into() }],
        ..Default::default()
    });
    let g = find(&m, "Glasshouse");
    assert_eq!(g.mode, Mode::Protected);
    assert_eq!(g.line, "Protected: credited on Joint Tape in Quiet Hours's protected folder");
    assert_eq!(g.artist_id, None);
}

#[test]
fn lidarr_artists_match_by_musicbrainz_id_or_folder_never_by_name() {
    let m = assign(&ModeInputs {
        library: vec![clean()],
        lidarr_artists: vec![
            // Same name, another artist.
            LidarrArtistRef { id: 1, name: "Northlight".into(), mbid: C_MBID.into(), path: "/data/media/Music/Lidarr/Northlight (UK)".into() },
            LidarrArtistRef { id: 2, name: "Somebody".into(), mbid: B_MBID.into(), path: "/x/Somebody".into() },
        ],
        ..Default::default()
    });
    let mine = m.iter().find(|x| x.artist_id.as_deref() == Some("ar1")).unwrap();
    assert_eq!(mine.lidarr_id, None, "a name alone never matches");
    let other = m.iter().find(|x| x.lidarr_id == Some(1)).unwrap();
    assert_eq!(other.line, "Managed: in Lidarr only, nothing of it in your library yet");

    let m = assign(&ModeInputs {
        library: vec![artist("ar1", "Northlight", None, 2, &[("First Light", "/data/media/Music/Lidarr/Northlight/First Light", None)])],
        lidarr_artists: vec![LidarrArtistRef { id: 5, name: "Other Name".into(), mbid: A_MBID.into(), path: "/data/media/Music/Lidarr/Northlight/".into() }],
        ..Default::default()
    });
    assert_eq!(m.len(), 1, "the same folder is the same artist");
    assert_eq!(m[0].lidarr_id, Some(5));
}

#[test]
fn the_artist_folder_is_the_parent_of_most_album_folders() {
    let a = artist("ar1", "N", None, 0, &[("A", "/m/N/A", None), ("B", "/m/N/B", None), ("C", "/m/Loose/C", None)]);
    assert_eq!(a.folder().as_deref(), Some("/m/N"));
    assert_eq!(parent_raw(r"M:\Music\N\A\"), Some(r"M:\Music\N".to_string()));
}

#[test]
fn library_facts_come_from_tags_types_and_files() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("m.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched, mbid) VALUES
               ('a1', 'First Light', 'First Light', 'ar1', 'Northlight', 'Album', '[]', 1, '70000000-0000-0000-0000-000000000001'),
               ('a2', 'Night Tapes', 'Night Tapes', 'ar1', 'Northlight', NULL, '[\"demo\"]', 0, NULL),
               ('a3', 'Sessions', 'Sessions', 'ar2', 'Quiet Hours', 'Album', '[]', 1, NULL);
             INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, file) VALUES
               ('t1', 'Dawn', 'a1', 'First Light', 'ar1', 'Northlight', '/p/t1', '/music/Northlight/First Light/01 Dawn.flac'),
               ('t2', 'Dusk', 'a2', 'Night Tapes', 'ar1', 'Northlight', '/p/t2', '/music/Northlight/Night Tapes/01 Dusk.flac'),
               ('t3', 'Low', 'a3', 'Sessions', 'ar2', 'Quiet Hours', '/p/t3', '/music/Quiet Hours/Sessions/01 Low.flac'),
               ('t4', 'High', 'a3', 'Sessions', 'ar2', 'Quiet Hours', '/p/t4', '/music/Quiet Hours/Sessions/02 High.flac');
             INSERT INTO file_tags (part_key, status, version, release, scanned_at) VALUES
               ('/p/t1', 'ok', NULL, 'released', 1),
               ('/p/t3', 'ok', NULL, 'unreleased', 1),
               ('/p/t4', 'ok', 'leak', 'released', 1);",
        )?;
        Ok(())
    })
    .unwrap();
    let got = db.read(library_artists).unwrap();
    let n = got.iter().find(|a| a.artist_id == "ar1").unwrap();
    assert_eq!((n.files, n.curated_files()), (2, 0));
    assert_eq!(n.typed_unreleased, vec!["Night Tapes"]);
    assert_eq!(n.folder().as_deref(), Some("/music/Northlight"));
    assert!(n.albums.iter().any(|a| a.release_group.as_deref() == Some("70000000-0000-0000-0000-000000000001")));
    let q = got.iter().find(|a| a.artist_id == "ar2").unwrap();
    assert_eq!(q.curated, BTreeMap::from([("leak".to_string(), 1), ("unreleased".to_string(), 1)]), "ARCHIVE_VERSION first, then the provenance status");
}
