//! Release status from whichever tags a library uses: each scheme read
//! from each container, the values and their synonyms, a custom tag and
//! its mapping, the cache, album rating and what the release rule makes of
//! it.

use rusqlite::{Connection, params};

use super::provenance::{
    self, AlbumRelease, Config, FileRelease, Provenance, ValueMap, album_release, compare_count, map_value, resolve, set_config, synonym,
};
use super::store::{self, Status};
use super::tests::{atom, data_atom, flac, freeform, id3, mp4, ogg, run, streaminfo, txxx, vorbis};
use super::FileTags;
use crate::db::release::ReleaseStatus;
use crate::db::schema;

use FileRelease::{Alternate, Leaked, Released, Unreleased};

fn auto() -> Config {
    Config::default()
}

fn status_of(t: &FileTags) -> Option<FileRelease> {
    resolve(t, &provenance::config()).status
}

/// The same tags as FLAC, ID3 v2.3 and v2.4, MP4 and Ogg Vorbis files.
fn every_container(tags: &[(&str, &str)]) -> Vec<(String, FileTags)> {
    let entries: Vec<String> = tags.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let refs: Vec<&str> = entries.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    out.push(("flac".to_string(), run(&flac(&[streaminfo(), (4, vorbis(&refs))]), Some("flac")).0));
    for ver in [3u8, 4] {
        let frames: Vec<(&[u8], Vec<u8>)> = tags.iter().map(|(k, v)| (&b"TXXX"[..], txxx(3, k, v))).collect();
        out.push((format!("id3 v2.{ver}"), run(&[id3(ver, &frames), vec![0xff; 400]].concat(), Some("mp3")).0));
    }
    let items: Vec<Vec<u8>> = tags.iter().map(|(k, v)| freeform(k, v)).collect();
    out.push(("mp4".to_string(), run(&mp4(&items, false, 500, true), Some("mp4")).0));
    let vc = vorbis(&refs);
    out.push(("ogg".to_string(), run(&ogg(b"\x01vorbis....", [b"\x03vorbis".to_vec(), vc].concat()), Some("ogg")).0));
    out
}

// ------------------------------------------------------------------ reading

#[test]
fn plain_status_and_version_are_read_from_every_container() {
    for (kind, t) in every_container(&[("STATUS", "Leaked"), ("VERSION", "OG file")]) {
        assert_eq!(t.status_tag.as_deref(), Some("Leaked"), "{kind}");
        assert_eq!(t.version_note.as_deref(), Some("OG file"), "{kind}");
        assert_eq!(t.version, None, "VERSION is free text, never ARCHIVE_VERSION ({kind})");
        assert_eq!(resolve(&t, &auto()), Provenance { status: Some(Leaked), note: Some("OG file".into()) }, "{kind}");
    }
}

#[test]
fn picards_release_status_and_type_are_read_from_every_container() {
    // Vorbis comments use RELEASESTATUS; ID3 and MP4 name the item.
    let vorbis_keys = [("RELEASESTATUS", "bootleg"), ("RELEASETYPE", "album")];
    let named_keys = [("MusicBrainz Album Status", "bootleg"), ("MusicBrainz Album Type", "album")];
    for (kind, t) in every_container(&vorbis_keys).into_iter().chain(every_container(&named_keys)) {
        assert_eq!(t.mb_status.as_deref(), Some("bootleg"), "{kind}");
        assert_eq!(t.mb_type.as_deref(), Some("album"), "{kind}");
        assert_eq!(status_of(&t), Some(Unreleased), "{kind}");
    }
    for (kind, t) in every_container(&[("RELEASESTATUS", "Official")]) {
        assert_eq!(status_of(&t), Some(Released), "{kind}");
    }
    // A demo release type is an alternate version when nothing else says.
    for (kind, t) in every_container(&[("RELEASETYPE", "album; demo")]) {
        assert_eq!(status_of(&t), Some(Alternate), "{kind}");
    }
}

#[test]
fn archive_version_comes_before_every_other_scheme() {
    for (kind, t) in every_container(&[("ARCHIVE_VERSION", "leak"), ("STATUS", "Released"), ("RELEASESTATUS", "official")]) {
        assert_eq!(status_of(&t), Some(Leaked), "{kind}");
    }
    let t = |v: &str| FileTags { version: Some(v.into()), status_tag: Some("Unreleased".into()), ..Default::default() };
    assert_eq!(status_of(&t("released")), Some(Released));
    assert_eq!(status_of(&t("alternate")), Some(Alternate));
    for v in ["demo", "snippet", "session", "build", "unreleased"] {
        assert_eq!(status_of(&t(v)), Some(Unreleased), "{v}");
    }
    // Other versions are released ones, as the album rule has them.
    assert_eq!(status_of(&t("live")), Some(Released));
    assert_eq!(status_of(&t("remix")), Some(Released));
}

#[test]
fn schemes_are_tried_in_order_until_one_knows() {
    let t = FileTags { status_tag: Some("Whatever".into()), mb_status: Some("official".into()), ..Default::default() };
    assert_eq!(status_of(&t), Some(Released), "an unknown STATUS falls through to Picard's");
    let t = FileTags { status_tag: Some("Leak".into()), mb_status: Some("official".into()), ..Default::default() };
    assert_eq!(status_of(&t), Some(Leaked), "STATUS comes first");
    let t = FileTags { mb_status: Some("Pseudo-Release".into()), mb_type: Some("album".into()), ..Default::default() };
    assert_eq!(status_of(&t), None, "a pseudo-release says nothing about provenance");
    assert_eq!(status_of(&FileTags::default()), None);
}

#[test]
fn the_note_is_version_else_archive_source() {
    let t = FileTags { version_note: Some("Official release (web rip)".into()), source: Some("forum".into()), ..Default::default() };
    assert_eq!(resolve(&t, &auto()).note.as_deref(), Some("Official release (web rip)"));
    let t = FileTags { source: Some("forum".into()), ..Default::default() };
    assert_eq!(resolve(&t, &auto()).note.as_deref(), Some("forum"));
    // The note never decides the status.
    let t = FileTags { version_note: Some("Leak".into()), ..Default::default() };
    assert_eq!(resolve(&t, &auto()), Provenance { status: None, note: Some("Leak".into()) });
}

// ------------------------------------------------------------------ values

#[test]
fn values_are_matched_in_any_case_with_synonyms() {
    for v in ["Released", "RELEASED", " official ", "Official Release", "retail", "Promotion", "promo", "Official release (web rip)"] {
        assert_eq!(synonym(v), Some(Released), "{v}");
    }
    for v in ["Unreleased", "unofficial", "Bootleg", "not-released", "Scrapped", "vault"] {
        assert_eq!(synonym(v), Some(Unreleased), "{v}");
    }
    for v in ["Leak", "leaked", "LEAKS", "Leaked."] {
        assert_eq!(synonym(v), Some(Leaked), "{v}");
    }
    for v in ["alt", "Alt version", "alternate_version", "Alternative", "demo", "OG", "OG File", "og-file", "Reference", "Snippet", "Session"] {
        assert_eq!(synonym(v), Some(Alternate), "{v}");
    }
}

#[test]
fn unknown_values_stay_unknown() {
    for v in ["", "  ", "yes", "maybe", "Tidal rip", "Remaster", "2019", "Live", "Remix", "single", "released?nope"] {
        assert_eq!(map_value(v, &auto()), None, "{v:?} is never guessed");
    }
}

#[test]
fn custom_values_come_before_the_built_in_ones() {
    let cfg = Config::new(
        "",
        &[
            ValueMap { value: "Vault copy".into(), status: Unreleased },
            ValueMap { value: "promo".into(), status: Leaked },
        ],
    );
    assert_eq!(map_value("vault COPY", &cfg), Some(Unreleased));
    assert_eq!(map_value("vault_copy (2nd)", &cfg), Some(Unreleased), "brackets left out");
    assert_eq!(map_value("Promo", &cfg), Some(Leaked), "the user's meaning wins");
    assert_eq!(map_value("Released", &cfg), Some(Released), "built-in values still work");
    let t = FileTags { status_tag: Some("Vault copy".into()), ..Default::default() };
    assert_eq!(resolve(&t, &cfg).status, Some(Unreleased));
}

// ------------------------------------------------------------------ custom tag

#[test]
fn a_custom_tag_is_read_from_every_container() {
    set_config(Config::new("provenance", &[ValueMap { value: "From the vault".into(), status: Unreleased }]));
    assert_eq!(provenance::custom_key().as_deref(), Some("PROVENANCE"));
    for (kind, t) in every_container(&[("Provenance", "From the vault"), ("STATUS", "Released")]) {
        assert_eq!(t.custom_key.as_deref(), Some("PROVENANCE"), "{kind}");
        assert_eq!(t.custom_status.as_deref(), Some("From the vault"), "{kind}");
        assert_eq!(status_of(&t), Some(Unreleased), "the custom tag replaces STATUS ({kind})");
    }
    // STATUS alone doesn't count while a custom tag is set.
    for (kind, t) in every_container(&[("STATUS", "Released")]) {
        assert_eq!(status_of(&t), None, "{kind}");
    }
    // ARCHIVE_VERSION still comes first.
    for (kind, t) in every_container(&[("ARCHIVE_VERSION", "released"), ("PROVENANCE", "From the vault")]) {
        assert_eq!(status_of(&t), Some(Released), "{kind}");
    }
    set_config(Config::default());
    assert_eq!(FileTags::for_read().custom_key, None);
}

#[test]
fn a_value_read_for_another_tag_name_doesnt_count() {
    let t = FileTags { custom_key: Some("OLDTAG".into()), custom_status: Some("Leak".into()), ..Default::default() };
    assert_eq!(resolve(&t, &Config::new("NEWTAG", &[])).status, None);
    assert_eq!(resolve(&t, &Config::new("oldtag", &[])).status, Some(Leaked));
}

#[test]
fn auto_and_blank_mean_auto() {
    assert_eq!(Config::new("", &[]).tag, None);
    assert_eq!(Config::new("  Auto ", &[]).tag, None);
    assert_eq!(Config::new(" origin ", &[]).tag.as_deref(), Some("ORIGIN"));
    assert!(set_config(Config::new("origin", &[])));
    assert!(!set_config(Config::new("ORIGIN", &[])), "the same tag in another case is no change");
    assert!(set_config(Config::default()));
}

#[test]
fn id3_v22_and_utf16_frames_carry_the_status_too() {
    let f = id3(2, &[(b"TXX", txxx(0, "STATUS", "Unreleased"))]);
    assert_eq!(status_of(&run(&f, Some("mp3")).0), Some(Unreleased));
    let f = [id3(3, &[(b"TXXX", txxx(1, "STATUS", "Leak"))]), vec![0xff; 400]].concat();
    assert_eq!(status_of(&run(&f, Some("mp3")).0), Some(Leaked));
    // MP4 items other than freeform ones are left alone.
    let f = mp4(&[atom(b"\xa9cmt", &data_atom(1, b"Leak"))], false, 100, true);
    assert_eq!(run(&f, Some("mp4")).0.status_tag, None);
}

// ------------------------------------------------------------------ albums

#[test]
fn disc_one_decides_an_album() {
    // An official album with a disc of unreleased extras stays official.
    let official = [(1, Some(Released)), (1, Some(Released)), (1, Some(Released)), (2, Some(Unreleased)), (2, Some(Leaked))];
    assert_eq!(album_release(&official), AlbumRelease { status: Some(Released), mixed: true });
    // An unreleased build with a few released songs on disc 1 is mixed, not official.
    let build = [(1, Some(Released)), (1, Some(Unreleased)), (1, Some(Unreleased)), (1, Some(Leaked))];
    assert_eq!(album_release(&build), AlbumRelease { status: Some(Unreleased), mixed: true });
    // Half isn't most.
    assert_eq!(album_release(&[(1, Some(Released)), (1, Some(Leaked))]).status, Some(Leaked));
    // All released, all one kind.
    assert_eq!(album_release(&[(1, Some(Released)), (0, Some(Released))]), AlbumRelease { status: Some(Released), mixed: false });
    assert_eq!(album_release(&[(1, Some(Alternate)), (1, Some(Alternate))]), AlbumRelease { status: Some(Alternate), mixed: false });
    // Unknown files don't count; nothing known is no status.
    assert_eq!(album_release(&[(1, Some(Released)), (1, None), (1, None)]).status, Some(Released));
    assert_eq!(album_release(&[(1, None), (2, None)]), AlbumRelease::default());
    assert_eq!(album_release(&[]), AlbumRelease::default());
    // Nothing known on disc 1: every disc counts.
    assert_eq!(album_release(&[(1, None), (2, Some(Leaked)), (3, Some(Released))]).status, Some(Leaked));
}

#[test]
fn the_musicbrainz_count_uses_released_songs_then_disc_one() {
    assert_eq!(compare_count(10, 12, 15), 10);
    assert_eq!(compare_count(0, 12, 15), 12);
    assert_eq!(compare_count(0, 0, 15), 15);
}

// ------------------------------------------------------------------ the cache and the rule

fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
    schema::migrate(&mut c).unwrap();
    c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('ar', 'Made Up Band', 'Made Up Band')", []).unwrap();
    (dir, c)
}

/// An album Plex couldn't match, so only tags can make it released.
fn album(c: &Connection, id: &str) {
    c.execute(
        "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, matched)
         VALUES (?1, ?1, ?1, 'ar', 'Made Up Band', 0, 0)",
        [id],
    )
    .unwrap();
}

/// A song on `disc` whose file carries `tags`.
fn song(c: &Connection, album: &str, id: &str, disc: u32, tags: &[(&str, &str)]) {
    c.execute(
        "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, disc_number, part_key)
         VALUES (?1, ?1, ?2, ?2, 'ar', 'Made Up Band', 1, ?3, ?4)",
        params![id, album, disc, format!("/p/{id}")],
    )
    .unwrap();
    let mut t = FileTags::for_read();
    for (k, v) in tags {
        t.set(k, v);
    }
    store::save(c, &format!("/p/{id}"), Status::Ok, &t, 1).unwrap();
}

fn album_status(c: &Connection, id: &str) -> crate::archive::status::AlbumReleaseStatus {
    crate::archive::status::album_status(c, id).unwrap().unwrap()
}

#[test]
fn an_official_album_with_an_unreleased_disc_two_stays_official() {
    let (_d, c) = db();
    album(&c, "al");
    for i in 0..4 {
        song(&c, "al", &format!("a{i}"), 1, &[("STATUS", "Released"), ("VERSION", "Official release (web rip)")]);
    }
    song(&c, "al", "b0", 2, &[("STATUS", "Unreleased")]);
    song(&c, "al", "b1", 2, &[("STATUS", "Leaked"), ("VERSION", "Leak")]);
    store::refresh_albums(&c).unwrap();
    assert_eq!(store::album_release(&c, "al").unwrap(), AlbumRelease { status: Some(Released), mixed: true });
    let s = album_status(&c, "al");
    assert_eq!(s.status, ReleaseStatus::Released, "not probably unreleased: the tags say official");
    assert!(s.mixed);
    assert!(s.evidence.iter().any(|e| e.decides && e.text.contains("first disc")));
    assert_eq!(crate::platform::discord::commands::album_unreleased(&c, "al").unwrap(), Some(false));
    // The songs on disc 2 keep their own status.
    assert_eq!(store::track_release(&c, "b1").unwrap().status, Some(Leaked));
    assert_eq!(store::track_release(&c, "b1").unwrap().provenance.as_deref(), Some("Leak"));
    let idx = store::index(&c).unwrap();
    assert_eq!(idx.albums["al"].release, Some(Released));
    assert!(idx.albums["al"].mixed);
    assert_eq!(idx.tracks["b0"].release, Some(Unreleased));
    assert_eq!(idx.tracks["a0"].provenance.as_deref(), Some("Official release (web rip)"));
    // The song index (sharing, stats, lyrics) follows each file.
    crate::identity::index::rebuild(&c).unwrap();
    let unreleased = |id: &str| -> bool {
        c.query_row("SELECT unreleased FROM song_index WHERE track_id = ?1", [id], |r| r.get(0)).unwrap()
    };
    assert!(!unreleased("a0"));
    assert!(unreleased("b0") && unreleased("b1"));
    // Song statuses too.
    crate::archive::versions::rebuild(&c).unwrap();
    let mut tracks = crate::db::queries::tracks_by_ids(&c, &["a0".into(), "b1".into()]).unwrap();
    crate::archive::versions::annotate(&c, &mut tracks, false).unwrap();
    let by = |id: &str| tracks.iter().find(|t| t.id == id).unwrap().clone();
    assert_eq!(by("a0").song_status, Some(crate::model::SongStatus::Released));
    assert_eq!(by("b1").song_status, Some(crate::model::SongStatus::Unreleased));
    assert_eq!(by("b1").file_release.unwrap().status, Some(Leaked));
}

#[test]
fn an_unreleased_build_with_some_released_songs_is_mixed_not_official() {
    let (_d, c) = db();
    album(&c, "bu");
    song(&c, "bu", "1", 1, &[("STATUS", "Released")]);
    song(&c, "bu", "2", 1, &[("STATUS", "Unreleased")]);
    song(&c, "bu", "3", 1, &[("STATUS", "Unreleased")]);
    store::refresh_albums(&c).unwrap();
    let s = album_status(&c, "bu");
    assert_eq!(s.status, ReleaseStatus::TaggedUnreleased);
    assert!(s.mixed);
    assert_eq!(crate::platform::discord::commands::album_unreleased(&c, "bu").unwrap(), Some(true));
}

#[test]
fn archive_tags_keep_their_answers() {
    let (_d, c) = db();
    album(&c, "ar1");
    // ARCHIVE_VERSION on the files: the album rule reads it as before, whatever STATUS says.
    song(&c, "ar1", "1", 1, &[("ARCHIVE_VERSION", "leak"), ("STATUS", "Released")]);
    song(&c, "ar1", "2", 1, &[("ARCHIVE_VERSION", "leak")]);
    album(&c, "ar2");
    song(&c, "ar2", "3", 1, &[("ARCHIVE_VERSION", "released")]);
    album(&c, "ar3");
    song(&c, "ar3", "4", 1, &[]);
    store::refresh_albums(&c).unwrap();
    assert_eq!(album_status(&c, "ar1").status, ReleaseStatus::TaggedUnreleased);
    assert_eq!(album_status(&c, "ar1").reasons[0], "Its files are tagged as version \"leak\".");
    assert_eq!(album_status(&c, "ar2").status, ReleaseStatus::Released);
    assert_eq!(album_status(&c, "ar3").status, ReleaseStatus::ProbablyUnreleased, "untagged, unmatched: as before");
    assert_eq!(store::album_version(&c, "ar1").unwrap().as_deref(), Some("leak"));
    assert_eq!(store::track_release(&c, "1").unwrap().status, Some(Leaked));
}

#[test]
fn the_cache_keeps_the_raw_tags_and_works_statuses_out_again() {
    let (_d, c) = db();
    album(&c, "al");
    song(&c, "al", "1", 1, &[("STATUS", "From the vault"), ("VERSION", "Reference"), ("RELEASETYPE", "album")]);
    let (_, back) = store::get(&c, "/p/1").unwrap().unwrap();
    assert_eq!(back.status_tag.as_deref(), Some("From the vault"));
    assert_eq!(back.version_note.as_deref(), Some("Reference"));
    assert_eq!(back.mb_type.as_deref(), Some("album"));
    assert_eq!(store::track_release(&c, "1").unwrap().status, None, "unknown");
    // A custom value maps it, without reading the file again.
    set_config(Config::new("", &[ValueMap { value: "from the vault".into(), status: Unreleased }]));
    assert!(store::restatus(&c).unwrap());
    assert_eq!(store::track_release(&c, "1").unwrap().status, Some(Unreleased));
    assert_eq!(store::album_release(&c, "al").unwrap().status, Some(Unreleased));
    assert!(!store::restatus(&c).unwrap(), "nothing changed the second time");
    // A new custom tag name means reading the file again.
    assert!(store::pending(&c, 10).unwrap().is_empty());
    set_config(Config::new("ORIGIN", &[]));
    assert_eq!(store::pending(&c, 10).unwrap().len(), 1);
    assert_eq!(store::counts(&c).unwrap().done, 0);
    let mut t = FileTags::for_read();
    t.set("origin", "Leak");
    store::save(&c, "/p/1", Status::Ok, &t, 2).unwrap();
    assert!(store::pending(&c, 10).unwrap().is_empty());
    assert_eq!(store::track_release(&c, "1").unwrap().status, Some(Leaked));
    // A failed read is stored for the tag in effect, so it isn't read again and again.
    song(&c, "al", "2", 1, &[]);
    store::save(&c, "/p/2", Status::Failed, &FileTags::for_read(), 2).unwrap();
    assert!(store::pending(&c, 10).unwrap().is_empty());
    set_config(Config::default());
}

#[test]
fn the_musicbrainz_check_counts_released_songs_and_skips_tagged_unreleased_albums() {
    let (_d, c) = db();
    // Official by its tags, with a disc of extras: looked up (for its cover) by its released songs.
    album(&c, "official");
    for i in 0..3 {
        song(&c, "official", &format!("o{i}"), 1, &[("STATUS", "Released")]);
    }
    song(&c, "official", "o9", 2, &[("STATUS", "Unreleased")]);
    // Untagged with a bonus disc: disc 1 is compared.
    album(&c, "untagged");
    for i in 0..5 {
        song(&c, "untagged", &format!("u{i}"), if i < 2 { 1 } else { 2 }, &[]);
    }
    // Unreleased by its tags: never looked up.
    album(&c, "leaks");
    song(&c, "leaks", "l1", 1, &[("STATUS", "Leaked")]);
    store::refresh_albums(&c).unwrap();
    let due = crate::artistinfo::releasecheck::candidates(&c, 0).unwrap();
    let count = |id: &str| due.iter().find(|x| x.album_id == id).map(|x| x.track_count);
    assert_eq!(count("official"), Some(3));
    assert_eq!(count("untagged"), Some(2));
    assert_eq!(count("leaks"), None);
}
