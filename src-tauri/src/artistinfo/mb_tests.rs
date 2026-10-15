//! MusicBrainz details and ListenBrainz listener numbers: parsing against
//! fixtures shaped like the real answers, and the service's album credits
//! and artist members. All HTTP is faked.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::http::fake::FakeHttp;
use super::listenbrainz::{LbStats, parse_stats};
use super::mbdetails::{
    CreditRow, CreditsState, LibTrack, MbCredit, MbRelease, album_credits, match_tracks, parse_artist, parse_release, pick_release,
};
use super::merge::{PlexArtist, Sources, merge};
use super::service::{InfoEvents, MbidLookup, album_key};
use super::*;
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::AppResult;
use crate::plex::keychain::MemoryStore;

const RELEASE: &str = include_str!("fixtures/mb_release.json");
const ARTIST_JSON: &str = include_str!("fixtures/mb_artist.json");
const GROUP: &str = include_str!("fixtures/mb_release_group.json");

const BAND: &str = "11111111-2222-3333-4444-555555555555";
const RELEASE_ID: &str = "aaaaaaaa-1111-4111-8111-000000000001";
const ARTIST: &str = "plexid-9876";
const ALBUM: &str = "album-55";

fn v(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

fn credit<'a>(cs: &'a [MbCredit], name: &str) -> &'a MbCredit {
    cs.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{name} is credited"))
}

// ---------------------------------------------------------------- parsing

#[test]
fn release_dates_label_and_release_credits() {
    let r = parse_release(&v(RELEASE)).unwrap();
    assert_eq!(r.release_id, RELEASE_ID);
    assert_eq!(r.first_release_date.as_deref(), Some("1997-05-21"));
    assert_eq!(r.date.as_deref(), Some("2004-03-01"));
    assert_eq!(r.label.as_deref(), Some("Low Tide Music"));
    let names: Vec<&str> = r.credits.iter().map(|c| c.name.as_str()).collect();
    // Assistants and roles we don't show (artwork, links) are left out.
    assert_eq!(names, ["Ada Moss", "Bo Kettle"]);
    assert_eq!(credit(&r.credits, "Ada Moss").roles, ["producer"]);
    assert_eq!(credit(&r.credits, "Bo Kettle").roles, ["mastering"]);
}

#[test]
fn track_credits_come_from_recordings_works_and_the_credit_line() {
    let r = parse_release(&v(RELEASE)).unwrap();
    assert_eq!(r.tracks.len(), 2);
    let t1 = &r.tracks[0];
    assert_eq!((t1.disc, t1.position, t1.title.as_str()), (1, 1, "Opening"));
    assert_eq!(credit(&t1.credits, "Ada Moss").roles, ["producer"]);
    assert_eq!(credit(&t1.credits, "Cy Fader").roles, ["mixer"]);
    assert_eq!(credit(&t1.credits, "Cal Rowe").roles, ["composer"]);
    assert_eq!(credit(&t1.credits, "Dee Lin").roles, ["lyricist"]);
    // Everyone after "feat." is featured; the main artist is not.
    assert_eq!(credit(&t1.credits, "Singer").roles, ["featured"]);
    assert!(!t1.credits.iter().any(|c| c.name == "The Band"));

    // Disc two: the recording's credit line ("ft."), a writer who is also the composer.
    let t2 = &r.tracks[1];
    assert_eq!((t2.disc, t2.position), (2, 1));
    assert_eq!(credit(&t2.credits, "Cal Rowe").roles, ["writer", "composer"], "one person once, roles in order");
    assert_eq!(credit(&t2.credits, "Guest Horn").roles, ["featured"]);
    assert_eq!(credit(&t2.credits, "Guest Keys").roles, ["featured"]);
    assert_eq!(credit(&t2.credits, "Guest Keys").mbid, None);
}

#[test]
fn members_with_years_and_relationships() {
    let a = parse_artist(&v(ARTIST_JSON)).unwrap();
    assert_eq!(a.mbid, BAND);
    assert_eq!(a.life_span.as_deref(), Some("1985–present"));
    let m: Vec<(&str, Option<&str>, bool)> = a.members.iter().map(|m| (m.name.as_str(), m.years.as_deref(), m.current)).collect();
    // One row per person (two instruments merge, widest years); current members first.
    assert_eq!(
        m,
        [("Singer", Some("1985–present"), true), ("Keys Player", None, true), ("Drummer", Some("1985–1999"), false)]
    );
    let rel: Vec<(&str, &str)> = a.related.iter().map(|r| (r.relation.as_str(), r.name.as_str())).collect();
    assert_eq!(rel, [("Subgroup", "The Band Trio"), ("Collaborators", "Band and Friends"), ("Formerly", "The Early Band")]);
}

#[test]
fn a_person_lists_the_bands_they_are_in() {
    let p = json!({
        "id": "eeeeeeee-1111-4111-8111-000000000001",
        "life-span": {"begin": "1966-02-03", "ended": false},
        "relations": [{
            "type": "member of band", "target-type": "artist", "direction": "forward",
            "artist": {"id": BAND, "name": "The Band"}
        }]
    });
    let a = parse_artist(&p).unwrap();
    assert!(a.members.is_empty());
    assert_eq!((a.related[0].relation.as_str(), a.related[0].name.as_str()), ("Member of", "The Band"));
    assert_eq!(a.related[0].mbid.as_deref(), Some(BAND));
}

#[test]
fn not_an_entity_is_nothing() {
    assert_eq!(parse_release(&json!({"error": "Not Found"})), None);
    assert_eq!(parse_artist(&json!({"id": "nope"})), None);
}

#[test]
fn the_release_read_is_official_with_the_albums_track_count() {
    let g = v(GROUP);
    assert_eq!(pick_release(&g, 2).as_deref(), Some(RELEASE_ID), "official and the same count beats an earlier bootleg");
    // No release with that count: the earliest official one.
    assert_eq!(pick_release(&g, 7).as_deref(), Some("aaaaaaaa-1111-4111-8111-000000000008"));
    assert_eq!(pick_release(&json!({"releases": []}), 2), None);
}

#[test]
fn songs_match_by_disc_and_position_else_title() {
    let r = parse_release(&v(RELEASE)).unwrap();
    let lib = vec![
        LibTrack { id: "t1".into(), title: "Opening".into(), disc: 1, number: 1 },
        LibTrack { id: "t2".into(), title: "Second Light (Remastered)".into(), disc: 2, number: 1 },
    ];
    assert_eq!(match_tracks(&r.tracks, &lib), [Some("t1".into()), Some("t2".into())]);
    // A different count: titles only.
    let lib = vec![
        LibTrack { id: "x".into(), title: "second light".into(), disc: 1, number: 1 },
        LibTrack { id: "y".into(), title: "Bonus".into(), disc: 1, number: 2 },
        LibTrack { id: "z".into(), title: "Other".into(), disc: 1, number: 3 },
    ];
    assert_eq!(match_tracks(&r.tracks, &lib), [None, Some("x".into())]);
}

#[test]
fn album_credits_gather_everyone_and_keep_featured_with_their_songs() {
    let r = parse_release(&v(RELEASE)).unwrap();
    let lib = vec![
        LibTrack { id: "t1".into(), title: "Opening".into(), disc: 1, number: 1 },
        LibTrack { id: "t2".into(), title: "Second Light".into(), disc: 2, number: 1 },
    ];
    let library = HashMap::from([("singer".to_string(), "77".to_string()), ("cal rowe".to_string(), "88".to_string())]);
    let c = album_credits(ALBUM, &r, &lib, &library);
    assert_eq!(c.state, CreditsState::Found);
    assert_eq!(c.released.as_deref(), Some("1997-05-21"), "the original date");
    assert_eq!(c.edition_date.as_deref(), Some("2004-03-01"));
    assert_eq!(c.url.as_deref(), Some(format!("https://musicbrainz.org/release/{RELEASE_ID}").as_str()));
    let names: Vec<&str> = c.credits.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names[0], "Ada Moss", "producers first");
    assert!(names.contains(&"Cal Rowe") && names.contains(&"Dee Lin") && names.contains(&"Cy Fader"));
    assert!(!names.contains(&"Singer") && !names.contains(&"Guest Horn"), "featured artists stay on their songs");
    let cal = c.credits.iter().find(|x| x.name == "Cal Rowe").unwrap();
    assert_eq!(cal, &CreditRow { name: "Cal Rowe".into(), artist_id: Some("88".into()), roles: vec!["writer".into(), "composer".into()] });
    let t1 = c.tracks.iter().find(|t| t.track_id == "t1").unwrap();
    let singer = t1.credits.iter().find(|x| x.name == "Singer").unwrap();
    assert_eq!((singer.artist_id.as_deref(), singer.roles.as_slice()), (Some("77"), ["featured".to_string()].as_slice()));
}

#[test]
fn an_undated_release_has_no_edition_date() {
    let mut r = MbRelease { release_id: RELEASE_ID.into(), date: Some("1997".into()), ..Default::default() };
    let c = album_credits(ALBUM, &r, &[], &HashMap::new());
    assert_eq!((c.released.as_deref(), c.edition_date.as_deref()), (Some("1997"), None));
    r.date = None;
    assert_eq!(album_credits(ALBUM, &r, &[], &HashMap::new()).released, None);
}

#[test]
fn listenbrainz_stats_keep_only_the_totals() {
    let body = json!({"payload": {
        "artist_mbid": BAND, "total_listen_count": 51234, "total_user_count": 812,
        "listeners": [{"user_name": "someone", "listen_count": 900}]
    }});
    let s = parse_stats(body.to_string().as_bytes(), BAND).unwrap().unwrap();
    assert_eq!(s, LbStats { mbid: BAND.into(), listeners: Some(812), listens: Some(51234) });
    assert_eq!(parse_stats(br#"{"payload": {}}"#, BAND).unwrap(), None);
    assert!(parse_stats(b"<html>", BAND).is_err());
}

// ---------------------------------------------------------------- merge

fn plex() -> PlexArtist {
    PlexArtist { id: ARTIST.into(), title: "The Band".into(), summary: None, thumb: None, art: None }
}

#[test]
fn musicbrainz_members_win_and_listenbrainz_fills_the_listener_line() {
    let mb = parse_artist(&v(ARTIST_JSON)).unwrap();
    let lb = LbStats { mbid: BAND.into(), listeners: Some(812), listens: Some(51234) };
    let s = Sources { musicbrainz: Some(mb), lb_stats: Some(lb), ..Default::default() };
    let lib = HashMap::from([("singer".to_string(), "77".to_string())]);
    let info = merge(&plex(), &s, &lib);
    assert_eq!(info.members[0], Member { name: "Singer".into(), artist_id: Some("77".into()), years: Some("1985–present".into()) });
    assert_eq!(info.years_active.as_deref(), Some("1985–present"));
    assert_eq!(info.related.len(), 3);
    assert_eq!((info.listeners, info.scrobbles, info.listeners_from.as_deref()), (Some(812), Some(51234), Some("listenbrainz")));
    let sources: Vec<&str> = info.credits.iter().map(|c| c.source.as_str()).collect();
    assert!(sources.contains(&"musicbrainz") && sources.contains(&"listenbrainz"));
}

#[test]
fn last_fm_numbers_come_first() {
    let lastfm = LastfmData { listeners: Some(48_000), playcount: Some(900_000), ..Default::default() };
    let lb = LbStats { mbid: BAND.into(), listeners: Some(812), listens: Some(51234) };
    let s = Sources { lastfm: Some(lastfm), lb_stats: Some(lb), ..Default::default() };
    let info = merge(&plex(), &s, &HashMap::new());
    assert_eq!((info.listeners, info.listeners_from.as_deref()), (Some(48_000), Some("lastfm")));
    assert!(!info.credits.iter().any(|c| c.source == "listenbrainz"));
}

// ---------------------------------------------------------------- service

struct Lookup {
    album: Option<&'static str>,
}

#[async_trait]
impl MbidLookup for Lookup {
    async fn artist_mbid(&self, _: &str) -> AppResult<Option<String>> {
        Ok(Some(BAND.to_owned()))
    }
    async fn album_mbid(&self, _: &str) -> AppResult<Option<String>> {
        Ok(self.album.map(str::to_owned))
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<String>>);
impl InfoEvents for Events {
    fn info_changed(&self, _: &str) {}
    fn album_credits_changed(&self, id: &str) {
        self.0.lock().push(id.to_owned());
    }
}

struct Rig {
    svc: ArtistInfoService,
    http: Arc<FakeHttp>,
    events: Arc<Events>,
    switches: Switches,
    _dir: tempfile::TempDir,
}

/// One band with a two-disc album of two songs, and Singer in the library.
fn rig(album_mbid: Option<&'static str>, format: &str) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("i.db")).unwrap();
    let format = format.to_owned();
    db.write(move |c| {
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES (?1, 'The Band', 'Band')", [ARTIST])?;
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('77', 'Singer', 'Singer')", [])?;
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format) VALUES (?1, 'Lantern Songs', 'Lantern Songs', ?2, 'The Band', ?3)",
            rusqlite::params![ALBUM, ARTIST, format],
        )?;
        for (id, title, disc) in [("t1", "Opening", 1), ("t2", "Second Light", 2)] {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, disc_number, part_key)
                 VALUES (?1, ?2, ?3, 'Lantern Songs', ?4, 'The Band', 1, ?5, '/p')",
                rusqlite::params![id, title, ALBUM, ARTIST, disc],
            )?;
        }
        Ok(())
    })
    .unwrap();
    let http = Arc::new(FakeHttp::new());
    let switches = Switches::default();
    // Only the sources under test.
    switches.set(InfoSource::Wikipedia, false);
    switches.set(InfoSource::Listenbrainz, false);
    let svc = ArtistInfoService::new(db, http.clone(), Arc::new(MemoryStore::new()), switches.clone());
    svc.set_mbid_lookup(Arc::new(Lookup { album: album_mbid }));
    let events = Arc::new(Events::default());
    svc.set_events(events.clone());
    Rig { svc, http, events, switches, _dir: dir }
}

#[tokio::test]
async fn album_credits_are_looked_up_once_by_plex_release_id_and_cached_30_days() {
    let r = rig(Some(RELEASE_ID), "album");
    r.http.route(&format!("ws/2/release/{RELEASE_ID}"), 200, RELEASE);
    let now = now_s();
    let (first, due) = r.svc.album_cached(ALBUM, now).await.unwrap();
    assert_eq!((first.state, due), (CreditsState::Pending, true));

    assert!(r.svc.refresh_album(ALBUM).await.unwrap());
    assert_eq!(*r.events.0.lock(), [ALBUM.to_string()], "the page hears when it's done");
    assert_eq!(r.http.calls("musicbrainz.org"), 1, "a known release id is one request");
    assert!(r.http.urls()[0].contains("inc=release-groups+artist-credits+labels+recordings"));

    let (c, due) = r.svc.album_cached(ALBUM, now).await.unwrap();
    assert!(!due);
    assert_eq!(c.state, CreditsState::Found);
    assert_eq!(c.released.as_deref(), Some("1997-05-21"));
    let t1 = c.tracks.iter().find(|t| t.track_id == "t1").expect("disc 1 song matched");
    assert_eq!(t1.credits.iter().find(|x| x.name == "Singer").unwrap().artist_id.as_deref(), Some("77"));
    assert!(c.tracks.iter().any(|t| t.track_id == "t2"), "disc 2 song matched");

    // Still fresh after 29 days, due again after 30.
    assert!(!r.svc.album_cached(ALBUM, now + 29 * 86_400).await.unwrap().1);
    assert!(r.svc.album_cached(ALBUM, now + 31 * 86_400).await.unwrap().1);
}

#[tokio::test]
async fn without_a_plex_id_the_album_is_found_by_search_then_its_best_release() {
    let r = rig(None, "album");
    let search = json!({"release-groups": [{
        "id": "bbbbbbbb-1111-4111-8111-000000000001", "title": "Lantern Songs", "score": 100,
        "artist-credit": [{"name": "The Band", "artist": {"id": BAND, "name": "The Band"}}]
    }]});
    r.http.route("release-group/?query", 200, &search.to_string());
    r.http.route("release-group/bbbbbbbb-1111-4111-8111-000000000001", 200, GROUP);
    r.http.route(&format!("ws/2/release/{RELEASE_ID}"), 200, RELEASE);
    assert!(r.svc.refresh_album(ALBUM).await.unwrap());
    let (c, _) = r.svc.album_cached(ALBUM, now_s()).await.unwrap();
    assert_eq!(c.state, CreditsState::Found, "urls asked: {:?}", r.http.urls());
    assert_eq!(r.http.calls("musicbrainz.org"), 3);
}

#[tokio::test]
async fn no_sure_match_is_none_and_unreleased_albums_are_never_looked_up() {
    let r = rig(None, "album");
    r.http.route("release-group/?query", 200, &json!({"release-groups": []}).to_string());
    r.svc.refresh_album(ALBUM).await.unwrap();
    let (c, due) = r.svc.album_cached(ALBUM, now_s()).await.unwrap();
    assert_eq!((c.state, due), (CreditsState::None, false));

    let r = rig(Some(RELEASE_ID), "demo");
    let (c, due) = r.svc.album_cached(ALBUM, now_s()).await.unwrap();
    assert_eq!((c.state, due), (CreditsState::None, false));
    assert!(!r.svc.refresh_album(ALBUM).await.unwrap());
    assert_eq!(r.http.calls("musicbrainz.org"), 0, "an unreleased album's name never leaves the computer");
}

#[tokio::test]
async fn the_musicbrainz_setting_off_asks_nothing() {
    let r = rig(Some(RELEASE_ID), "album");
    r.switches.set(InfoSource::Musicbrainz, false);
    let (c, due) = r.svc.album_cached(ALBUM, now_s()).await.unwrap();
    assert_eq!((c.state, due), (CreditsState::Off, false));
    assert!(!r.svc.refresh_album(ALBUM).await.unwrap());
    r.svc.refresh(ARTIST).await.unwrap();
    assert_eq!(r.http.calls("musicbrainz.org/ws/2/artist/"), 0);
}

#[tokio::test]
async fn a_failed_lookup_waits_before_asking_again() {
    let r = rig(Some(RELEASE_ID), "album");
    r.http.fail("musicbrainz.org");
    r.svc.refresh_album(ALBUM).await.unwrap();
    let now = now_s();
    let (c, due) = r.svc.album_cached(ALBUM, now).await.unwrap();
    assert_eq!((c.state, due), (CreditsState::None, false));
    assert!(r.svc.album_cached(ALBUM, now + 11 * 60).await.unwrap().1, "tried again later");
}

#[tokio::test]
async fn artist_members_relationships_and_listenbrainz_numbers() {
    let r = rig(None, "album");
    r.http.route(&format!("ws/2/artist/{BAND}?inc=artist-rels"), 200, ARTIST_JSON);
    r.http.route(
        &format!("stats/artist/{BAND}/listeners"),
        200,
        &json!({"payload": {"total_user_count": 812, "total_listen_count": 51234}}).to_string(),
    );
    assert!(r.svc.refresh(ARTIST).await.unwrap());
    let (info, _) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert_eq!(info.members.len(), 3);
    assert_eq!(info.members[0].artist_id.as_deref(), Some("77"), "a member in the library links");
    assert_eq!(info.members[0].years.as_deref(), Some("1985–present"));
    assert_eq!(info.related.len(), 3);
    assert_eq!((info.listeners, info.listeners_from.as_deref()), (Some(812), Some("listenbrainz")));

    // Turning ListenBrainz stats off hides the line.
    r.switches.set(InfoSource::ListenbrainzStats, false);
    let (info, _) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert_eq!(info.listeners, None);
}

#[test]
fn album_rows_stay_apart_from_artist_rows() {
    assert_eq!(album_key("12"), "album:12");
}
