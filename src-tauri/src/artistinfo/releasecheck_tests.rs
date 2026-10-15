//! The MusicBrainz release check for albums Plex couldn't match, against
//! fixtures shaped like MusicBrainz's answers. All HTTP is faked.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::http::fake::FakeHttp;
use super::musicbrainz::{FoundRelease, pick_official_near};
use super::releasecheck::{self, FOUND_TTL, MISS_TTL, album_status, candidates, run_round};
use super::service::InfoEvents;
use super::*;
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::db::release::{ReleaseStatus, set_use_musicbrainz};
use crate::plex::keychain::MemoryStore;

const SEARCH: &str = include_str!("fixtures/mb_rg_search.json");
const RELEASES: &str = include_str!("fixtures/mb_rg_releases.json");
const RG: &str = "cccccccc-1111-4111-8111-000000000001";
const REL: &str = "eeeeeeee-1111-4111-8111-000000000001";
const DELUXE: &str = "eeeeeeee-1111-4111-8111-000000000002";
const ALBUM: &str = "al-9";

#[derive(Default)]
struct Events(Mutex<Vec<String>>);
impl InfoEvents for Events {
    fn info_changed(&self, _: &str) {}
    fn release_status_changed(&self, id: &str) {
        self.0.lock().push(id.to_owned());
    }
}

struct Rig {
    svc: ArtistInfoService,
    http: Arc<FakeHttp>,
    db: DbHandle,
    events: Arc<Events>,
    switches: Switches,
    _dir: tempfile::TempDir,
}

/// One album by Rue Ellis that Plex couldn't match, with `tracks` songs.
fn rig(title: &str, tracks: usize) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("r.db")).unwrap();
    let title = title.to_owned();
    db.write(move |c| {
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('ar1', 'Rue Ellis', 'Ellis')", [])?;
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, matched)
             VALUES (?1, ?2, ?2, 'ar1', 'Rue Ellis', ?3, 0)",
            rusqlite::params![ALBUM, title, tracks as i64],
        )?;
        for i in 0..tracks {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, disc_number, part_key)
                 VALUES (?1, ?2, ?3, ?4, 'ar1', 'Rue Ellis', ?5, 1, ?6)",
                rusqlite::params![format!("t{i}"), format!("Song {i}"), ALBUM, title, i as i64 + 1, format!("/p/{i}")],
            )?;
        }
        Ok(())
    })
    .unwrap();
    let http = Arc::new(FakeHttp::new());
    http.route("release-group/?query", 200, SEARCH);
    http.route(&format!("release-group/{RG}?inc=releases"), 200, RELEASES);
    let switches = Switches::default();
    let svc = ArtistInfoService::new(db.clone(), http.clone(), Arc::new(MemoryStore::new()), switches.clone());
    let events = Arc::new(Events::default());
    svc.set_events(events.clone());
    Rig { svc, http, db, events, switches, _dir: dir }
}

fn row(r: &Rig) -> Option<releasecheck::CheckRow> {
    r.db.read(|c| releasecheck::get(c, ALBUM)).unwrap()
}

fn status(r: &Rig) -> (ReleaseStatus, Vec<String>) {
    let s = r.db.read(|c| album_status(c, ALBUM)).unwrap().unwrap();
    (s.status, s.reasons)
}

const NO_GAP: Duration = Duration::ZERO;

#[tokio::test]
async fn an_exact_match_means_plex_missed_it() {
    let r = rig("Northern Lights", 12);
    assert_eq!(status(&r).0, ReleaseStatus::ProbablyUnreleased);
    assert_eq!(run_round(&r.svc, NO_GAP).await, 1);
    let got = row(&r).unwrap();
    assert!(got.found);
    assert_eq!((got.release_group_id.as_deref(), got.release_id.as_deref()), (Some(RG), Some(REL)));
    assert_eq!(got.expires_at - got.checked_at, FOUND_TTL, "kept 30 days");
    let (s, why) = status(&r);
    assert_eq!(s, ReleaseStatus::PlexMissed);
    assert!(why.iter().any(|w| w.contains("MusicBrainz has an official release")));
    assert_eq!(*r.events.0.lock(), [ALBUM.to_string()], "the album page hears about it");
    assert_eq!(r.http.calls("musicbrainz.org"), 2, "a search and one release group");
    // The other rule callers agree: a public link and scrobbles are allowed now.
    assert_eq!(r.db.read(|c| crate::platform::discord::commands::album_unreleased(c, ALBUM)).unwrap(), Some(false));
    // Nothing more to do until it expires.
    assert_eq!(run_round(&r.svc, NO_GAP).await, 0);
}

#[tokio::test]
async fn an_edition_note_in_the_title_still_matches() {
    let r = rig("Northern Lights (Deluxe)", 16);
    run_round(&r.svc, NO_GAP).await;
    let got = row(&r).unwrap();
    assert!(got.found);
    assert_eq!(got.release_id.as_deref(), Some(DELUXE), "the release with the same track count");
    let search = r.http.urls()[0].to_lowercase();
    assert!(!search.contains("deluxe"), "edition notes aren't sent: {search}");
}

#[tokio::test]
async fn nothing_on_musicbrainz_makes_the_guess_stronger() {
    let r = rig("Northern Lights", 12);
    r.http.route("release-group/?query", 200, &json!({"release-groups": []}).to_string());
    run_round(&r.svc, NO_GAP).await;
    let got = row(&r).unwrap();
    assert!(!got.found);
    assert_eq!(got.expires_at - got.checked_at, MISS_TTL, "asked again after a week");
    let (s, why) = status(&r);
    assert_eq!(s, ReleaseStatus::ProbablyUnreleased);
    assert!(why.iter().any(|w| w.contains("MusicBrainz has no release")));
    assert!(r.events.0.lock().is_empty(), "still unreleased: nothing changed");
}

#[tokio::test]
async fn another_artists_album_is_not_a_match() {
    let r = rig("Northern Lights", 12);
    let mut v: Value = serde_json::from_str(SEARCH).unwrap();
    v["release-groups"][0]["artist-credit"][0]["name"] = json!("Rue Ellis Band");
    r.http.route("release-group/?query", 200, &v.to_string());
    run_round(&r.svc, NO_GAP).await;
    assert!(!row(&r).unwrap().found);
    assert_eq!(r.http.calls(&format!("release-group/{RG}")), 0);
}

#[tokio::test]
async fn a_track_count_too_far_off_is_not_a_match() {
    let r = rig("Northern Lights", 5);
    run_round(&r.svc, NO_GAP).await;
    assert!(!row(&r).unwrap().found);
    assert_eq!(status(&r).0, ReleaseStatus::ProbablyUnreleased);
}

#[test]
fn official_releases_within_two_tracks_count() {
    let v: Value = serde_json::from_str(RELEASES).unwrap();
    assert_eq!(pick_official_near(&v, 12).as_deref(), Some(REL), "the promo is earlier but not official");
    assert_eq!(pick_official_near(&v, 13).as_deref(), Some(REL));
    assert_eq!(pick_official_near(&v, 10).as_deref(), Some(REL));
    assert_eq!(pick_official_near(&v, 17).as_deref(), Some(DELUXE));
    assert_eq!(pick_official_near(&v, 14).as_deref(), Some(REL), "a tie goes to the earliest");
    assert_eq!(pick_official_near(&v, 9), None);
    assert_eq!(pick_official_near(&json!({"releases": []}), 12), None);
}

#[tokio::test]
async fn the_setting_off_asks_nothing_and_ignores_stored_answers() {
    let r = rig("Northern Lights", 12);
    set_use_musicbrainz(false);
    assert_eq!(run_round(&r.svc, NO_GAP).await, 0);
    assert_eq!(r.http.calls("musicbrainz.org"), 0);
    set_use_musicbrainz(true);
    run_round(&r.svc, NO_GAP).await;
    assert_eq!(status(&r).0, ReleaseStatus::PlexMissed);
    set_use_musicbrainz(false);
    assert_eq!(status(&r).0, ReleaseStatus::ProbablyUnreleased, "the answer stops counting");
    assert_eq!(r.db.read(|c| releasecheck::found_ids(c, ALBUM)).unwrap(), None, "and Discord stops using its ids");
    set_use_musicbrainz(true);

    // MusicBrainz itself off (or Go offline): nothing asked either.
    let r = rig("Northern Lights", 12);
    r.switches.set(InfoSource::Musicbrainz, false);
    assert_eq!(run_round(&r.svc, NO_GAP).await, 0);
    assert_eq!(r.http.calls("musicbrainz.org"), 0);
}

#[tokio::test]
async fn answers_expire_and_are_checked_again() {
    let r = rig("Northern Lights", 12);
    let now = now_s();
    r.db.write(|c| {
        let found = FoundRelease { release_group_id: RG.into(), release_id: REL.into() };
        releasecheck::put(c, ALBUM, Some(&found), now)
    })
    .unwrap();
    let due = |at: i64| r.db.read(move |c| candidates(c, at)).unwrap().len();
    assert_eq!(due(now), 0);
    assert_eq!(due(now + FOUND_TTL - 1), 0);
    assert_eq!(due(now + FOUND_TTL), 1, "a found release is checked again after a month");
    r.db.write(|c| releasecheck::put(c, ALBUM, None, now)).unwrap();
    assert_eq!(due(now + MISS_TTL - 1), 0);
    assert_eq!(due(now + MISS_TTL), 1, "a miss after a week");
    // An expired miss that MusicBrainz now has flips to found.
    r.db.write(move |c| {
        c.execute("UPDATE album_mb_check SET expires_at = ?1", [now - 1])?;
        Ok(())
    })
    .unwrap();
    run_round(&r.svc, NO_GAP).await;
    assert!(row(&r).unwrap().found);
    assert_eq!(*r.events.0.lock(), [ALBUM.to_string()]);
}

#[tokio::test]
async fn tagged_albums_are_never_looked_up() {
    for tag in ["demo", "version", "file version", "typed", "matched"] {
        let r = rig("Northern Lights", 12);
        r.db.write(move |c| {
            match tag {
                "demo" => c.execute("UPDATE albums SET subformats = '[\"Demo\"]'", [])?,
                "version" => c.execute("INSERT INTO album_tags (album_id, version) VALUES (?1, 'leak')", [ALBUM])?,
                "file version" => {
                    c.execute("INSERT INTO file_tags (part_key, status, version, scanned_at) VALUES ('/p/3', 'ok', 'snippet', 1)", [])?
                }
                "typed" => c.execute("UPDATE albums SET format = 'Album'", [])?,
                _ => c.execute("UPDATE albums SET matched = 1", [])?,
            };
            Ok(())
        })
        .unwrap();
        assert_eq!(run_round(&r.svc, NO_GAP).await, 0, "{tag}");
        assert_eq!(r.http.calls("musicbrainz.org"), 0, "{tag}: its name never leaves the computer");
    }
}

#[tokio::test]
async fn a_failed_lookup_stores_nothing_and_waits_for_the_next_round() {
    let r = rig("Northern Lights", 12);
    r.http.fail("release-group/?query");
    assert_eq!(run_round(&r.svc, NO_GAP).await, 0);
    assert_eq!(row(&r), None);
    assert_eq!(status(&r).0, ReleaseStatus::ProbablyUnreleased);
}

#[tokio::test]
async fn tags_beat_a_found_release() {
    let r = rig("Northern Lights", 12);
    run_round(&r.svc, NO_GAP).await;
    assert_eq!(status(&r).0, ReleaseStatus::PlexMissed);
    r.db.write(|c| {
        c.execute("INSERT INTO album_tags (album_id, version) VALUES (?1, 'leak')", [ALBUM])?;
        Ok(())
    })
    .unwrap();
    assert_eq!(status(&r).0, ReleaseStatus::TaggedUnreleased);
}

#[tokio::test]
async fn a_found_release_clears_an_uploaded_discord_cover() {
    let r = rig("Northern Lights", 12);
    r.db.write(|c| crate::platform::discord::art::put(c, ALBUM, Some("https://i.imgur.com/x.jpg"), "imgur", 1, i64::MAX))
        .unwrap();
    run_round(&r.svc, NO_GAP).await;
    assert_eq!(r.db.read(|c| crate::platform::discord::art::get(c, ALBUM)).unwrap(), None);
}
