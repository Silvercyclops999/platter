//! The release group rules, with MusicBrainz faked.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::db::DbHandle;

pub const ARTIST: &str = "aaaaaaaa-1111-2222-3333-444444444444";
pub const OTHER: &str = "bbbbbbbb-1111-2222-3333-444444444444";
pub const RG_FIRST: &str = "10000000-0000-0000-0000-000000000001";
pub const RG_SECOND: &str = "10000000-0000-0000-0000-000000000002";
pub const RG_EP: &str = "10000000-0000-0000-0000-000000000003";
pub const RG_BOOT: &str = "10000000-0000-0000-0000-000000000004";
pub const RG_LIVE: &str = "10000000-0000-0000-0000-000000000005";
pub const RG_SKETCH: &str = "10000000-0000-0000-0000-000000000006";
pub const RG_TIDES: &str = "10000000-0000-0000-0000-000000000007";
pub const RG_VA: &str = "10000000-0000-0000-0000-000000000008";
pub const RG_SINGLE: &str = "10000000-0000-0000-0000-000000000009";
pub const RG_PROMO: &str = "10000000-0000-0000-0000-000000000010";
pub const RG_DUET: &str = "10000000-0000-0000-0000-000000000011";

fn ac(id: &str, name: &str) -> Value {
    json!([{"name": name, "artist": {"id": id, "name": name}}])
}

/// One release group as `/release-group/{id}?inc=artist-credits+releases` returns it.
pub fn rg(id: &str, title: &str, primary: &str, secondary: &[&str], date: &str, statuses: &[&str], credit: Value) -> Value {
    json!({
        "id": id, "title": title, "primary-type": primary, "secondary-types": secondary,
        "first-release-date": date, "artist-credit": credit,
        "releases": statuses.iter().enumerate().map(|(i, s)| json!({"id": format!("r{i}"), "status": s})).collect::<Vec<_>>()
    })
}

/// Northlight's discography on the faked MusicBrainz.
pub fn discography() -> Vec<Value> {
    let n = || ac(ARTIST, "Northlight");
    vec![
        rg(RG_FIRST, "First Light", "Album", &[], "2015-03-01", &["Official"], n()),
        rg(RG_SECOND, "Second Wind", "Album", &[], "2021-06-11", &["Official", "Promotion"], n()),
        rg(RG_EP, "Small Hours", "EP", &[], "2019", &["Official"], n()),
        rg(RG_BOOT, "Basement Tapes", "Album", &[], "2018", &["Bootleg"], n()),
        rg(RG_LIVE, "Live at the Harbor", "Album", &["Live"], "2022", &["Official"], n()),
        rg(RG_SKETCH, "Quiet Sketches", "Album", &[], "2023", &["Official"], n()),
        rg(RG_TIDES, "Tides", "Album", &[], "2017", &["Official"], n()),
        rg(RG_VA, "Summer Hits", "Album", &["Compilation"], "2016", &["Official"], ac(VARIOUS_ARTISTS, "Various Artists")),
        rg(RG_SINGLE, "Second Wind", "Single", &[], "2020-11-01", &["Official"], n()),
        rg(RG_PROMO, "Radio Cuts", "Album", &[], "2020", &["Promotion"], n()),
        rg(
            RG_DUET,
            "Together",
            "Album",
            &[],
            "2024",
            &["Official"],
            json!([{"name": "Northlight", "joinphrase": " & ", "artist": {"id": ARTIST, "name": "Northlight"}}, {"name": "Guest", "artist": {"id": OTHER, "name": "Guest"}}]),
        ),
    ]
}

/// The artist's releases browsed (`/release?artist=`), official only like
/// MusicBrainz filters them.
fn browse() -> Value {
    let releases: Vec<Value> = discography()
        .into_iter()
        .filter(|g| g["releases"].as_array().unwrap().iter().any(|r| r["status"] == "Official"))
        .map(|g| {
            json!({
                "id": format!("rel-{}", g["id"].as_str().unwrap()), "status": "Official", "date": g["first-release-date"],
                "artist-credit": g["artist-credit"],
                "release-group": {"id": g["id"], "title": g["title"], "primary-type": g["primary-type"],
                    "secondary-types": g["secondary-types"], "first-release-date": g["first-release-date"]}
            })
        })
        .collect();
    json!({"release-count": releases.len(), "releases": releases})
}

/// A faked MusicBrainz: answers by URL substring, counts requests.
#[derive(Default)]
pub struct FakeMb {
    pub routes: Mutex<Vec<(String, Option<Value>)>>,
    pub calls: Mutex<Vec<String>>,
    pub down: Mutex<bool>,
}

impl FakeMb {
    pub fn northlight() -> Arc<Self> {
        let f = Self::default();
        for g in discography() {
            let id = g["id"].as_str().unwrap().to_string();
            f.route(&format!("release-group/{id}?"), Some(g));
        }
        f.route("/release?artist=", Some(browse()));
        f.route(
            "/artist/?query=",
            Some(json!({"artists": [{"id": ARTIST, "name": "Northlight", "score": 100}]})),
        );
        // Searching for the album "Second Wind": the album and the single.
        f.route(
            "/release-group?query=releasegroup%3A%22second+wind%22",
            Some(json!({"release-groups": [
                {"id": RG_SINGLE, "title": "Second Wind", "primary-type": "Single", "first-release-date": "2020-11-01"},
                {"id": RG_SECOND, "title": "Second Wind", "primary-type": "Album", "first-release-date": "2021-06-11"}
            ]})),
        );
        f.route(
            "/release-group?query=releasegroup%3A%22basement+tapes%22",
            Some(json!({"release-groups": [{"id": RG_BOOT, "title": "Basement Tapes", "primary-type": "Album"}]})),
        );
        f.route(
            "/release-group?query=releasegroup%3A%22quiet+sketches%22",
            Some(json!({"release-groups": [{"id": RG_SKETCH, "title": "Quiet Sketches", "primary-type": "Album"}]})),
        );
        // The song "Breathe In": on the album, the single, a compilation and a live album.
        let rel = |id: &str, primary: &str, sec: &[&str], date: &str| {
            json!({"id": format!("rel-{id}"), "status": "Official", "date": date,
                   "release-group": {"id": id, "primary-type": primary, "secondary-types": sec}})
        };
        f.route(
            "/recording?query=recording%3A%22breathe+in%22",
            Some(json!({"recordings": [{
                "id": "rec-1", "score": 100, "title": "Breathe In", "artist-credit": ac(ARTIST, "Northlight"),
                "releases": [
                    rel(RG_VA, "Album", &["Compilation"], "2016"),
                    rel(RG_SINGLE, "Single", &[], "2020-11-01"),
                    rel(RG_SECOND, "Album", &[], "2021-06-11"),
                    rel(RG_LIVE, "Album", &["Live"], "2022")
                ]
            }]})),
        );
        // A song only on a compilation.
        f.route(
            "/recording?query=recording%3A%22summer+song%22",
            Some(json!({"recordings": [{
                "id": "rec-2", "score": 100, "title": "Summer Song", "artist-credit": ac(ARTIST, "Northlight"),
                "releases": [rel(RG_VA, "Album", &["Compilation"], "2016")]
            }]})),
        );
        Arc::new(f)
    }

    pub fn route(&self, url: &str, v: Option<Value>) {
        self.routes.lock().push((url.to_string(), v));
    }

    pub fn count(&self, part: &str) -> usize {
        self.calls.lock().iter().filter(|u| u.contains(part)).count()
    }
}

#[async_trait]
impl MbGet for FakeMb {
    async fn get(&self, url: &str) -> AppResult<Option<Value>> {
        assert!(url.starts_with(WS), "only MusicBrainz's web service: {url}");
        self.calls.lock().push(url.to_string());
        if *self.down.lock() {
            return Err(AppError::Network(MB_DOWN.into()));
        }
        Ok(self.routes.lock().iter().rev().find(|(u, _)| url.contains(u.as_str())).and_then(|(_, v)| v.clone()))
    }
}

pub fn want() -> Want {
    Want { name: "Northlight".into(), mbid: Some(ARTIST.into()), artist_id: Some("ar1".into()) }
}

fn lib(title: &str, unreleased: bool) -> LibraryAlbum {
    LibraryAlbum { id: format!("lib-{title}"), title: title.into(), also: vec![], unreleased, mbid: None }
}

/// What the library has by Northlight: a released album, an edition and
/// an unreleased build.
pub fn library() -> Vec<LibraryAlbum> {
    vec![lib("First Light", false), lib("Tides (Deluxe Edition)", false), lib("Quiet Sketches [Unreleased Build v2]", true)]
}

fn group(id: &str) -> Group {
    parse_group(discography().iter().find(|g| g["id"] == id).unwrap()).unwrap()
}

#[test]
fn bootleg_and_promo_only_groups_are_refused() {
    assert_eq!(vet(&group(RG_BOOT), &want(), &[], true), Err(Refusal::NotOfficial));
    assert_eq!(vet(&group(RG_PROMO), &want(), &[], true), Err(Refusal::NotOfficial));
    assert_eq!(vet(&group(RG_SECOND), &want(), &[], false), Ok(()), "an official release next to a promo is fine");
    assert!(Refusal::NotOfficial.message().contains("bootleg"));
}

#[test]
fn credits_must_be_the_artist_alone() {
    assert_eq!(vet(&group(RG_VA), &want(), &[], true), Err(Refusal::VariousArtists));
    assert_eq!(vet(&group(RG_DUET), &want(), &[], true), Err(Refusal::OtherArtist));
    let someone_else = Want { name: "Guest".into(), mbid: Some(OTHER.into()), artist_id: None };
    assert_eq!(vet(&group(RG_SECOND), &someone_else, &[], true), Err(Refusal::OtherArtist));
    // Without a MusicBrainz id the folded name decides.
    let by_name = Want { name: "the  NORTHLIGHT".into(), mbid: None, artist_id: None };
    assert_eq!(vet(&group(RG_SECOND), &by_name, &[], true), Ok(()));
}

#[test]
fn secondary_types_only_when_asked() {
    assert_eq!(vet(&group(RG_LIVE), &want(), &[], false), Err(Refusal::Secondary { types: vec!["live".into()] }));
    assert_eq!(vet(&group(RG_LIVE), &want(), &[], true), Ok(()));
}

#[test]
fn title_clash_with_an_unreleased_build_is_refused() {
    let r = vet(&group(RG_SKETCH), &want(), &library(), true);
    assert_eq!(r, Err(Refusal::TitleClash { title: "Quiet Sketches [Unreleased Build v2]".into(), unreleased: true }));
    assert!(r.unwrap_err().message().contains("unreleased album already in your library"));
}

#[test]
fn title_clash_with_an_edition_is_refused() {
    assert_eq!(vet(&group(RG_TIDES), &want(), &library(), true), Err(Refusal::TitleClash { title: "Tides (Deluxe Edition)".into(), unreleased: false }));
    // An ARCHIVE_* group name counts as a title too.
    let tagged = vec![LibraryAlbum { id: "u4".into(), title: "Untitled 4".into(), also: vec!["Second Wind".into()], unreleased: true, mbid: None }];
    assert!(matches!(vet(&group(RG_SECOND), &want(), &tagged, true), Err(Refusal::TitleClash { unreleased: true, .. })));
}

#[test]
fn titles_clash_by_their_base_whatever_the_edition_or_version_suffix() {
    let have = |t: &str| vec![lib(t, false)];
    for owned in [
        "First Light (Deluxe)",
        "First Light - v2",
        "First Light [Leak]",
        "FIRST LIGHT!",
        "First  Light (Remastered 2011)",
        "First Light (10th Anniversary Edition)",
        "First Light \u{2013} Demo",
        "First Light (Alt Mix)",
        "First Light (Expanded Version)",
    ] {
        assert!(clash("First Light", &have(owned)).is_some(), "First Light vs {owned}");
        assert!(matches!(vet(&group(RG_FIRST), &want(), &have(owned), true), Err(Refusal::TitleClash { .. })), "{owned}");
    }
    // The other way round: a requested edition against the plain album.
    assert!(clash("First Light (Deluxe)", &have("First Light")).is_some());
    assert!(clash("First Light [Leak]", &have("First Light - v2")).is_some());
    // Different titles don't clash.
    assert!(clash("First Lights", &have("First Light")).is_none());
    assert!(clash("Second Wind", &have("First Light (Deluxe)")).is_none());
}

#[test]
fn an_upgrade_skips_the_title_clash_but_keeps_every_other_rule() {
    let mine = LibraryAlbum { id: "a1".into(), title: "First Light".into(), also: vec!["first-light".into()], unreleased: false, mbid: None };
    // Same-titled builds and leaks don't block it: the user confirmed which release it is.
    let lib2 = vec![mine.clone(), lib("First Light [Leak]", true), lib("First Light (Deluxe)", false)];
    assert_eq!(vet_upgrade(&group(RG_FIRST), &want(), &lib2, "a1"), Ok(()));
    // The album itself unreleased: refused.
    let unreleased = LibraryAlbum { unreleased: true, ..mine.clone() };
    assert_eq!(vet_upgrade(&group(RG_FIRST), &want(), &[unreleased], "a1"), Err(Refusal::UnreleasedAlbum));
    // Bootlegs, live albums and other artists' releases: never.
    assert_eq!(vet_upgrade(&group(RG_BOOT), &want(), std::slice::from_ref(&mine), "a1"), Err(Refusal::NotOfficial));
    assert!(matches!(vet_upgrade(&group(RG_LIVE), &want(), std::slice::from_ref(&mine), "a1"), Err(Refusal::Secondary { .. })));
    assert_eq!(vet_upgrade(&group(RG_DUET), &want(), std::slice::from_ref(&mine), "a1"), Err(Refusal::OtherArtist));
    assert_eq!(vet_upgrade(&group(RG_FIRST), &want(), &[], "a1"), Err(Refusal::NotFound), "not an album in the library");
}

#[test]
fn a_tracklist_comes_from_the_first_official_release_never_a_withdrawn_one() {
    let v = json!({"releases": [
        {"status": "Withdrawn", "date": "2001", "media": [{"position": 1, "tracks": [{"position": 1, "title": "Gone"}]}]},
        {"status": "Official", "date": "2016", "media": [{"position": 1, "tracks": [{"position": 1, "title": "Late"}]}]},
        {"status": "Official", "date": "2015-03-01", "media": [
            {"position": 1, "tracks": [{"position": 1, "title": "Dawn", "length": 200000}]},
            {"position": 2, "tracks": [{"position": 1, "title": "Dusk"}]}]}
    ]});
    let t = parse_tracklist(&v);
    assert_eq!(t.iter().map(|x| (x.disc, x.position, x.title.as_str())).collect::<Vec<_>>(), vec![(1, 1, "Dawn"), (2, 1, "Dusk")]);
    assert_eq!(t[0].length_ms, Some(200_000));
    assert!(parse_tracklist(&json!({"releases": [{"status": "Cancelled", "media": []}]})).is_empty());
}

#[test]
fn requests_never_take_live_or_compilation_groups() {
    assert!(matches!(vet(&group(RG_LIVE), &want(), &[], false), Err(Refusal::Secondary { .. })));
}

#[test]
fn album_before_ep_before_single_then_earliest() {
    let picked = pick(vec![group(RG_SINGLE), group(RG_EP), group(RG_SECOND)], &want(), &[], false).unwrap();
    assert_eq!(picked.id, RG_SECOND, "the album, though the single came first");
    let picked = pick(vec![group(RG_SINGLE), group(RG_EP)], &want(), &[], false).unwrap();
    assert_eq!(picked.id, RG_EP);
    let mut early = group(RG_SECOND);
    early.id = "10000000-0000-0000-0000-0000000000aa".into();
    early.first_release = "2001".into();
    assert_eq!(pick(vec![group(RG_SECOND), early.clone()], &want(), &[], false).unwrap().id, early.id);
    // Nothing passes: the refusal says why, a clash first.
    assert_eq!(pick(vec![group(RG_BOOT)], &want(), &[], false), Err(Refusal::NotOfficial));
    assert!(matches!(pick(vec![group(RG_BOOT), group(RG_SKETCH)], &want(), &library(), false), Err(Refusal::TitleClash { .. })));
    assert_eq!(pick(vec![], &want(), &[], false), Err(Refusal::NotFound));
}

#[test]
fn missing_skips_what_the_library_has_or_clashes_with() {
    let mut all = BTreeMap::new();
    groups_from_releases(&browse(), &mut all);
    let titles: Vec<String> = missing(all.into_values().collect(), &want(), &library()).into_iter().map(|g| g.title).collect();
    // First Light (have), Tides (edition), Quiet Sketches (unreleased build),
    // bootleg, promo, live, VA, duet and singles are all left out.
    assert_eq!(titles, vec!["Small Hours", "Second Wind"]);
}

#[test]
fn release_groups_sort_into_discography_sections() {
    let g = |primary: &str, sec: &[&str]| Group {
        id: "x".into(),
        title: "x".into(),
        primary: Some(primary.into()),
        secondary: sec.iter().map(|s| s.to_string()).collect(),
        first_release: String::new(),
        statuses: vec![],
        credit: vec![],
    };
    assert_eq!(release_section(&g("album", &[])), Some("albums"));
    assert_eq!(release_section(&g("ep", &[])), Some("singlesEps"));
    assert_eq!(release_section(&g("single", &[])), Some("singlesEps"));
    assert_eq!(release_section(&g("album", &["compilation"])), Some("compilations"));
    assert_eq!(release_section(&g("album", &["live", "compilation"])), Some("compilations"), "a live best-of is a compilation");
    assert_eq!(release_section(&g("album", &["live"])), Some("live"));
    assert_eq!(release_section(&g("ep", &["remix"])), Some("remixes"));
    assert_eq!(release_section(&g("album", &["soundtrack"])), Some("soundtracks"));
    assert_eq!(release_section(&g("album", &["mixtape/street"])), Some("mixtapes"));
    assert_eq!(release_section(&g("album", &["interview"])), None, "spoken word has no section");
    assert_eq!(release_section(&g("broadcast", &[])), None);
    let kinds = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let every = kinds(&["studio", "ep", "live", "compilation"]);
    assert_eq!(wanted_section(&g("single", &[]), &every), Some("singlesEps"), "singles come with EPs");
    assert_eq!(wanted_section(&g("album", &["live"]), &every), Some("live"));
    assert_eq!(wanted_section(&g("album", &["soundtrack"]), &every), None, "not a Get kind");
    assert_eq!(wanted_section(&g("ep", &[]), &kinds(&["studio"])), None);
    assert_eq!(wanted_section(&g("album", &["compilation"]), &kinds(&["compilation"])), Some("compilations"));
}

#[test]
fn missing_in_keeps_singles_and_secondaries_by_section() {
    let mut all = BTreeMap::new();
    groups_from_releases(&browse(), &mut all);
    let groups: Vec<Group> = all.into_values().collect();
    let every: Vec<String> = ["studio", "ep", "live", "compilation"].iter().map(|s| s.to_string()).collect();
    let got: Vec<(String, &str, bool)> =
        missing_in(groups.clone(), &want(), &library(), &every).into_iter().map(|m| (m.group.title, m.section, m.refusal.is_some())).collect();
    assert_eq!(
        got,
        vec![
            ("Small Hours".to_string(), "singlesEps", false),
            ("Second Wind".to_string(), "singlesEps", false),
            ("Second Wind".to_string(), "albums", false),
            ("Live at the Harbor".to_string(), "live", true),
        ],
        "the single shows now; the live album shows with why it can't be asked for; bootleg, VA, duet and clashes stay out"
    );
    let albums_only = missing_in(groups, &want(), &library(), &["studio".to_string()]);
    assert_eq!(albums_only.iter().map(|m| m.group.title.as_str()).collect::<Vec<_>>(), ["Second Wind"]);
}

#[tokio::test]
async fn picks_from_an_album_or_a_song() {
    let mb = FakeMb::northlight();
    let p = Picker { mb: mb.clone() };
    let got = p.pick_album(&want(), "Second Wind (Deluxe)", &library()).await.unwrap();
    assert_eq!(got.group.unwrap().id, RG_SECOND);
    let got = p.pick_album(&want(), "Basement Tapes", &library()).await.unwrap();
    assert_eq!(got.refusal, Some(Refusal::NotOfficial));
    let got = p.pick_album(&want(), "Quiet Sketches", &library()).await.unwrap();
    assert!(matches!(got.refusal, Some(Refusal::TitleClash { unreleased: true, .. })), "{got:?}");

    // A song: the album, not the earlier single, the compilation or the live album.
    let got = p.pick_song(&want(), "Breathe In", None, &library()).await.unwrap();
    assert_eq!(got.group.unwrap().id, RG_SECOND);
    // Only on a compilation: nothing to request.
    let got = p.pick_song(&want(), "Summer Song", None, &[]).await.unwrap();
    assert!(got.group.is_none() && got.reason.is_some(), "{got:?}");
    assert!(mb.calls.lock().iter().all(|u| u.contains("fmt=json")));
}

#[tokio::test]
async fn an_artists_official_groups_come_from_browsing_releases() {
    let mb = FakeMb::northlight();
    let p = Picker { mb: mb.clone() };
    let groups = p.artist_groups(ARTIST).await.unwrap();
    assert!(groups.iter().all(|g| g.statuses.iter().all(|s| s == "official")));
    assert!(!groups.iter().any(|g| g.id == RG_BOOT));
    let url = &mb.calls.lock()[0];
    assert!(url.contains("status=official") && url.contains("type=album%7Cep%7Csingle"), "{url}");
}

#[tokio::test]
async fn musicbrainz_answers_are_cached_and_paced() {
    use crate::artistinfo::http::fake::FakeHttp;
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("m.db")).unwrap();
    let http = Arc::new(FakeHttp::new());
    http.route("musicbrainz.org/ws/2/release-group/", 200, &group_json());
    let now = Arc::new(Mutex::new(1_000_i64));
    let n = now.clone();
    let mb = MbCached { http: http.clone(), limiter: Arc::new(Limiter::per_second(100.0)), db, clock: Arc::new(move || *n.lock()), ttl: CACHE_TTL };
    let url = group_url(RG_SECOND);
    assert!(mb.get(&url).await.unwrap().is_some());
    assert!(mb.get(&url).await.unwrap().is_some());
    assert_eq!(http.calls("musicbrainz.org"), 1, "the second answer came from the cache");
    *now.lock() += CACHE_TTL + 1;
    http.route("musicbrainz.org/ws/2/release-group/", 503, "");
    assert!(mb.get(&url).await.unwrap().is_some(), "busy: the old answer stands in");
    let other = group_url(RG_FIRST).replace("inc=", "x=");
    assert!(mb.get(&other).await.unwrap_err().to_string().contains(MB_DOWN));
}

fn group_json() -> String {
    discography().into_iter().find(|g| g["id"] == RG_SECOND).unwrap().to_string()
}

#[test]
fn library_albums_by_id_or_folded_name() {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("l.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('a1', 'First Light', 'First Light', 'ar1', 'Northlight', 'Album', '[]', 1),
               ('a2', 'Quiet Sketches', 'Quiet Sketches', 'ar9', 'The Northlight', NULL, '[]', 1),
               ('a3', 'Elsewhere', 'Elsewhere', 'ar2', 'Harbor Sound', 'Album', '[]', 1),
               ('a4', 'Night Demos', 'Night Demos', 'ar1', 'Northlight', NULL, '[]', 0);
             INSERT INTO album_tags (album_id, version, grp) VALUES ('a2', 'build', 'Sketchbook');",
        )?;
        Ok(())
    })
    .unwrap();
    let got = db.read(|c| library_albums(c, Some("ar1"), "Northlight")).unwrap();
    let mut titles: Vec<(&str, bool)> = got.iter().map(|a| (a.title.as_str(), a.unreleased)).collect();
    titles.sort();
    assert_eq!(titles, vec![("First Light", false), ("Night Demos", true), ("Quiet Sketches", true)]);
    assert_eq!(got.iter().find(|a| a.title == "Quiet Sketches").unwrap().also, vec!["Sketchbook"]);
    assert_eq!(got.iter().find(|a| a.title == "First Light").unwrap().id, "a1");
}

#[test]
fn library_albums_follow_the_users_marks_and_song_versions() {
    use crate::db::release::{Mark, mark_key, set_marks};
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("k.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, matched) VALUES
               ('a1', 'First Light', 'First Light', 'ar1', 'Northlight', 'Album', '[]', 1),
               ('a2', 'Night Demos', 'Night Demos', 'ar1', 'Northlight', NULL, '[]', 0),
               ('a3', 'Tides', 'Tides', 'ar1', 'Northlight', 'Album', '[]', 1),
               ('a4', 'Harbor', 'Harbor', 'ar1', 'Northlight', 'Album', '[]', 1);
             INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key) VALUES
               ('t1', 'Low', 'a3', 'Tides', 'ar1', 'Northlight', '/p/t1'),
               ('t2', 'Pier', 'a4', 'Harbor', 'ar1', 'Northlight', '/p/t2'),
               ('t3', 'Dock', 'a4', 'Harbor', 'ar1', 'Northlight', '/p/t3');
             INSERT INTO file_tags (part_key, status, version, scanned_at) VALUES ('/p/t1', 'ok', 'leak', 1);",
        )?;
        Ok(())
    })
    .unwrap();
    let state = |marks: Vec<(String, Mark)>| {
        set_marks(marks.into_iter().collect());
        let got = db.read(|c| library_albums(c, Some("ar1"), "Northlight")).unwrap();
        let mut v: Vec<(String, bool)> = got.into_iter().map(|a| (a.title, a.unreleased)).collect();
        v.sort();
        v
    };
    let plain = state(vec![]);
    assert_eq!(
        plain,
        vec![("First Light".into(), false), ("Harbor".into(), false), ("Night Demos".into(), true), ("Tides".into(), true)],
        "a leaked song's version makes its album count as unreleased"
    );
    let marked = state(vec![
        (mark_key("album", "a1"), Mark::Unreleased),
        (mark_key("album", "a2"), Mark::Released),
        (mark_key("track", "t3"), Mark::Unreleased),
    ]);
    assert_eq!(
        marked,
        vec![("First Light".into(), true), ("Harbor".into(), true), ("Night Demos".into(), false), ("Tides".into(), true)],
        "marks on albums and on songs count"
    );
    set_marks(Default::default());
}

#[test]
fn any_bracketed_or_dashed_note_still_clashes() {
    for t in ["Quiet Sketches (Build 2)", "Quiet Sketches [v2 leak]", "Quiet Sketches - Session Tapes", "QUIET SKETCHES {alt}", "Quiet Sketches \u{2013} Early"] {
        assert!(clash("Quiet Sketches", &[lib(t, true)]).is_some(), "{t}");
    }
    assert!(clash("Quiet", &[lib("Quiet Sketches", true)]).is_none());
    assert!(clash("Quiet Sketches II", &[lib("Quiet Sketches", true)]).is_none());
}

#[test]
fn an_album_you_have_isnt_offered_under_a_longer_name_or_another_title() {
    let have = vec![lib("Cruel Summer", false)];
    assert!(clash("Kanye West Presents Good Music: Cruel Summer", &have).is_some());
    assert!(clash("Cruel Summer", &[lib("GOOD Music: Cruel Summer", false)]).is_some());
    assert!(clash("Cruel Summer: The Remixes", &have).is_none(), "only the part after the colon counts");
    assert!(clash("Summer", &have).is_none());
    // Plex matched the album to this release group: that's the album, whatever it's called.
    let mut g = group(RG_SECOND);
    g.title = "Some Other Name".into();
    let mut matched = lib("Second Wind (Deluxe)", false);
    matched.mbid = Some(RG_SECOND.to_ascii_lowercase());
    assert!(matches!(vet(&g, &want(), &[matched], true), Err(Refusal::TitleClash { unreleased: false, .. })));
}
