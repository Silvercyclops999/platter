//! Sleeve notes: merging sources with precedence and labels, role groups,
//! reading MusicBrainz and Discogs releases, the credit page, the cache when
//! offline and the shared MusicBrainz pacing. All HTTP is faked.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::discogs::{self, parse_release as parse_discogs, pick_search, roles};
use super::gather::{self, PlexCredit, format_line, span};
use super::service::{DiscogsAccess, Due, PlexCredits, SleeveEvents, SleeveService, SleeveSwitches};
use super::*;
use crate::artistinfo::http::fake::FakeHttp;
use crate::artistinfo::mbdetails::{PARSE_VERSION, parse_release};
use crate::artistinfo::{ArtistInfoService, InfoSource, Switches};
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::AppResult;
use crate::plex::keychain::MemoryStore;
use crate::tags::FileTags;

fn c(name: &str, role: &str) -> RawCredit {
    RawCredit::new(name, role, None)
}

fn song(id: &str, title: &str, n: u32) -> LibSong {
    LibSong { id: id.into(), title: title.into(), disc: 1, number: n }
}

fn group<'a>(groups: &'a [RoleGroup], role: &str) -> &'a RoleGroup {
    groups.iter().find(|g| g.role == role).unwrap_or_else(|| panic!("a {role} group"))
}

// ---------------------------------------------------------------- merge

#[test]
fn facts_follow_precedence_and_agreeing_sources_add_up() {
    let tags = Part { label: Some("Low Tide".into()), released: Some("1997".into()), ..Default::default() };
    let plex = Part { label: Some("low  tide".into()), ..Default::default() };
    let mb = Part {
        label: Some("Low Tide Music".into()),
        released: Some("1997-05-21".into()),
        catalogue: Some("LT-001".into()),
        link: Some("https://musicbrainz.org/release/x".into()),
        ..Default::default()
    };
    let discogs = Part { catalogue: Some("lt 001".into()), country: Some("UK".into()), ..Default::default() };
    // Given out of order: precedence comes from the source, not the list.
    let parts = vec![(Source::Discogs, discogs), (Source::Musicbrainz, mb), (Source::Plex, plex), (Source::Tags, tags)];
    let n = merge("al", &parts, &[], &HashMap::new());
    let label = n.label.unwrap();
    assert_eq!(label.value, "Low Tide", "the files' own tags win");
    assert_eq!(label.sources, [Source::Tags, Source::Plex], "Plex agrees; MusicBrainz says something else");
    let released = n.released.unwrap();
    assert_eq!(released.value, "1997-05-21", "agreeing dates keep the precise one");
    assert_eq!(released.sources, [Source::Tags, Source::Musicbrainz]);
    assert_eq!(n.catalogue.unwrap().sources, [Source::Musicbrainz, Source::Discogs], "LT-001 and lt 001 agree");
    assert_eq!(n.country.unwrap(), Fact { value: "UK".into(), sources: vec![Source::Discogs] });
    assert_eq!(n.links, [Link { source: Source::Musicbrainz, url: "https://musicbrainz.org/release/x".into() }]);
    assert!(n.recorded.is_none() && n.notes.is_none());
}

#[test]
fn credits_merge_by_person_with_each_roles_sources() {
    let songs = [song("t1", "Opening", 1), song("t2", "Second", 2)];
    let mut tags = Part::default();
    tags.tracks.insert("t1".into(), TrackPart { credits: vec![c("Ada Moss", "producer"), c("Guest", "featured")], recorded: Some("1996-11".into()) });
    let mut mb = Part { credits: vec![c("ada moss", "producer"), c("Bo Kettle", "mastering")], ..Default::default() };
    mb.tracks.insert(
        "t1".into(),
        TrackPart { credits: vec![RawCredit::new("Cy Hart", "performer", Some("Guitar")), c("Ada Moss", "writer")], recorded: Some("1996-11-02".into()) },
    );
    let mut dg = Part::default();
    dg.tracks.insert("t2".into(), TrackPart { credits: vec![c("Ada Moss", "producer"), RawCredit::new("Cy Hart", "performer", Some("bass"))], recorded: None });
    let library = HashMap::from([("ada moss".to_string(), "77".to_string())]);
    let n = merge("al", &[(Source::Tags, tags), (Source::Musicbrainz, mb), (Source::Discogs, dg)], &songs, &library);

    let producers = group(&n.groups, "producer");
    assert_eq!(producers.people.len(), 1, "one person once");
    let ada = &producers.people[0];
    assert_eq!((ada.name.as_str(), ada.key.as_str(), ada.artist_id.as_deref()), ("Ada Moss", "ada moss", Some("77")));
    assert_eq!(ada.sources, [Source::Tags, Source::Musicbrainz, Source::Discogs]);
    // Featured artists stay with their songs.
    assert!(n.groups.iter().all(|g| g.role != "featured"));
    let players = group(&n.groups, "performer");
    assert_eq!(players.people[0].details, ["guitar", "bass"]);
    assert_eq!(players.people[0].sources, [Source::Musicbrainz, Source::Discogs]);

    let t1 = &n.tracks[0];
    assert_eq!(group(&t1.groups, "featured").people[0].name, "Guest");
    assert_eq!(group(&t1.groups, "producer").people[0].sources, [Source::Tags]);
    assert_eq!(group(&t1.groups, "writer").people[0].sources, [Source::Musicbrainz]);
    let rec = t1.recorded.as_ref().unwrap();
    assert_eq!((rec.value.as_str(), rec.sources.as_slice()), ("1996-11-02", [Source::Tags, Source::Musicbrainz].as_slice()));
    assert_eq!(group(&n.tracks[1].groups, "producer").people[0].sources, [Source::Discogs]);
}

#[test]
fn roles_group_in_sleeve_order_with_unknown_roles_last() {
    let p = |name: &str, roles: &[(&str, Option<&str>)]| Person {
        name: name.into(),
        key: fold_name(name),
        artist_id: None,
        roles: roles.iter().map(|(r, d)| RoleCredit { role: (*r).into(), detail: d.map(str::to_owned), sources: vec![Source::Tags] }).collect(),
    };
    let people = vec![
        p("Eng", &[("engineer", None)]),
        p("Prod", &[("producer", None), ("mixer", None)]),
        p("Odd", &[("whistler", None)]),
        p("Player", &[("performer", Some("drums")), ("performer", Some("percussion"))]),
    ];
    let g = group_by_role(&people);
    let order: Vec<&str> = g.iter().map(|x| x.role.as_str()).collect();
    assert_eq!(order, ["producer", "performer", "engineer", "mixer", "whistler"]);
    assert_eq!(g[1].people[0].details, ["drums", "percussion"], "one row per person, every instrument");
    assert_eq!(group(&g, "mixer").people[0].name, "Prod");
}

#[test]
fn spans_and_formats_read_well() {
    assert_eq!(span(["1996-11", "1997-02", "1996-12"].into_iter()).as_deref(), Some("1996-11/1997-02"));
    assert_eq!(span(["1996-11/1996-12", "1997-01"].into_iter()).as_deref(), Some("1996-11/1997-01"));
    assert_eq!(span(["2001", "2001"].into_iter()).as_deref(), Some("2001"));
    assert_eq!(span(std::iter::empty()), None);
    let f = |xs: &[&str]| format_line(&xs.iter().map(|x| x.to_string()).collect::<Vec<_>>());
    assert_eq!(f(&["CD", "CD"]).as_deref(), Some("2×CD"));
    assert_eq!(f(&["CD", "DVD-Video"]).as_deref(), Some("CD + DVD-Video"));
    assert_eq!(f(&[]), None);
}

// ---------------------------------------------------------------- MusicBrainz

const REL: &str = "aaaaaaaa-1111-4111-8111-000000000001";

fn mb_release() -> Value {
    json!({
        "id": REL, "title": "Lantern Songs", "date": "2004-03-01", "country": "GB", "barcode": "5012345678900",
        "status": "Official", "disambiguation": "2004 remaster", "packaging": "Jewel Case",
        "annotation": "Recorded live to tape. See [https://example.org|the band's site] for '''more'''.",
        "release-group": {"id": "bbbbbbbb-1111-4111-8111-000000000001", "first-release-date": "1997-05-21"},
        "label-info": [{"catalog-number": "LT-001", "label": {"name": "Low Tide Music"}}],
        "relations": [
            {"target-type": "artist", "type": "producer", "attributes": [], "artist": {"id": "cccccccc-1111-4111-8111-000000000001", "name": "Ada Moss"}},
            {"target-type": "place", "type": "recorded at", "place": {"name": "Harbour Studio"}},
            {"target-type": "place", "type": "mastered at", "place": {"name": "Cut Room"}}
        ],
        "media": [{"position": 1, "format": "CD", "tracks": [
            {"position": 1, "title": "Opening", "recording": {"title": "Opening", "relations": [
                {"target-type": "artist", "type": "instrument", "attributes": ["guest", "electric guitar"], "begin": "1996-11-02", "end": "1996-11-04",
                 "artist": {"name": "Cy Hart"}},
                {"target-type": "artist", "type": "vocal", "attributes": [], "begin": "1996-11-03", "artist": {"name": "Rue Ellis"}},
                {"target-type": "artist", "type": "conductor", "attributes": [], "artist": {"name": "Max Baton"}},
                {"target-type": "place", "type": "mixed at", "place": {"name": "Mix House"}}
            ]}},
            {"position": 2, "title": "Second", "recording": {"title": "Second", "relations": []}}
        ]}]
    })
}

#[test]
fn a_release_keeps_what_the_notes_show() {
    let r = parse_release(&mb_release()).unwrap();
    assert_eq!(r.parsed, PARSE_VERSION);
    assert_eq!((r.catalogue.as_deref(), r.country.as_deref(), r.barcode.as_deref()), (Some("LT-001"), Some("GB"), Some("5012345678900")));
    assert_eq!(r.disambiguation.as_deref(), Some("2004 remaster"));
    assert_eq!(r.formats, ["CD"]);
    assert_eq!(r.annotation.as_deref(), Some("Recorded live to tape. See the band's site for more."));
    let places: Vec<(&str, &str)> = r.places.iter().map(|p| (p.role.as_str(), p.name.as_str())).collect();
    assert_eq!(places, [("recorded", "Harbour Studio"), ("mastered", "Cut Room")]);
    let t1 = &r.tracks[0];
    assert_eq!(t1.recorded.as_deref(), Some("1996-11-02/1996-11-04"));
    let cy = t1.credits.iter().find(|c| c.name == "Cy Hart").unwrap();
    assert_eq!((cy.roles.as_slice(), cy.instruments.as_slice()), (["performer".to_string()].as_slice(), ["electric guitar".to_string()].as_slice()));
    let rue = t1.credits.iter().find(|c| c.name == "Rue Ellis").unwrap();
    assert_eq!(rue.instruments, ["vocals"]);
    assert_eq!(t1.credits.iter().find(|c| c.name == "Max Baton").unwrap().roles, ["conductor"]);
    assert_eq!(t1.places[0].name, "Mix House");

    let songs = [song("t1", "Opening", 1), song("t2", "Second", 2)];
    let p = gather::mb_part(&r, &songs);
    assert_eq!(p.released.as_deref(), Some("1997-05-21"));
    assert_eq!(p.edition_date.as_deref(), Some("2004-03-01"));
    assert_eq!(p.recorded.as_deref(), Some("1996-11-02/1996-11-04"));
    assert!(p.studios.contains(&("mixed".into(), "Mix House".into())), "song studios count for the album");
    assert_eq!(p.tracks["t1"].credits.iter().find(|c| c.name == "Cy Hart").unwrap().detail.as_deref(), Some("electric guitar"));
}

// ---------------------------------------------------------------- Discogs

fn discogs_release() -> Value {
    json!({
        "id": 4242, "released": "1997-05-00", "country": "UK",
        "labels": [{"name": "Low Tide Music (2)", "catno": "LT-001"}],
        "identifiers": [{"type": "Barcode", "value": "5 012345 678900"}, {"type": "Matrix / Runout", "value": "x"}],
        "formats": [{"name": "Vinyl", "qty": "2", "descriptions": ["LP", "Album", "Reissue"]}],
        "notes": "Pressed at [l=Platen Works]. Thanks to [a=Rue Ellis (3)].",
        "companies": [{"name": "Harbour Studio", "entity_type_name": "Recorded At"}, {"name": "Platen Works", "entity_type_name": "Pressed By"}],
        "extraartists": [
            {"name": "Ada Moss", "role": "Producer, Mixed By"},
            {"name": "Lee Shoot", "role": "Photography By"},
            {"name": "Big Boss", "role": "Executive-Producer"}
        ],
        "tracklist": [
            {"position": "", "type_": "heading", "title": "Side A"},
            {"position": "A1", "type_": "track", "title": "Opening", "extraartists": [{"name": "Cy Hart", "role": "Guitar [Lead], Backing Vocals"}]},
            {"position": "A2", "type_": "track", "title": "Second", "extraartists": [{"name": "Dee Lin", "role": "Lyrics By"}]}
        ]
    })
}

#[test]
fn discogs_roles_map_onto_the_sleeve_roles() {
    assert_eq!(roles("Producer, Mixed By"), [("producer", None), ("mixer", None)]);
    assert_eq!(roles("Written-By"), [("writer", None)]);
    assert_eq!(roles("Mastered By, Lacquer Cut By"), [("mastering", None)]);
    assert_eq!(roles("Executive-Producer"), []);
    assert_eq!(roles("Guitar [Lead, Rhythm]"), [("performer", Some("guitar".to_string()))]);
    assert_eq!(roles("Programmed By"), [("performer", Some("programming".to_string()))]);
    assert_eq!(roles("Design, Photography By"), []);
}

#[test]
fn a_discogs_release_becomes_a_source_part() {
    let r = parse_discogs(&discogs_release()).unwrap();
    assert_eq!(r.released.as_deref(), Some("1997-05"));
    assert_eq!(r.label.as_deref(), Some("Low Tide Music"), "Discogs' number suffix goes");
    assert_eq!(r.barcode.as_deref(), Some("5012345678900"));
    assert_eq!(r.format.as_deref(), Some("2×Vinyl"));
    assert_eq!(r.edition.as_deref(), Some("Reissue"));
    assert_eq!(r.notes.as_deref(), Some("Pressed at Platen Works. Thanks to Rue Ellis."));
    assert_eq!(r.studios, [("recorded".to_string(), "Harbour Studio".to_string())]);
    assert_eq!(r.tracks.len(), 2, "headings aren't tracks");
    let songs = [song("t1", "Opening", 1), song("t2", "Second", 2)];
    let p = discogs::to_part(&r, &songs);
    assert_eq!(p.link.as_deref(), Some("https://www.discogs.com/release/4242"));
    let names: Vec<(&str, &str)> = p.credits.iter().map(|c| (c.name.as_str(), c.role.as_str())).collect();
    assert_eq!(names, [("Ada Moss", "producer"), ("Ada Moss", "mixer")]);
    let t1: Vec<(&str, Option<&str>)> = p.tracks["t1"].credits.iter().map(|c| (c.role.as_str(), c.detail.as_deref())).collect();
    assert_eq!(t1, [("performer", Some("guitar")), ("performer", Some("backing vocals"))]);
    assert_eq!(p.tracks["t2"].credits[0].role, "lyricist");
}

#[test]
fn a_search_result_must_be_the_album() {
    let v = json!({"results": [
        {"id": 1, "title": "Other Band - Lantern Songs"},
        {"id": 2, "title": "The Band (2) - Lantern Songs (Deluxe Edition)"},
        {"id": 3, "title": "The Band - Lantern Songs"}
    ]});
    assert_eq!(pick_search(&v, "The Band", "Lantern Songs"), Some(2));
    assert_eq!(pick_search(&v, "Nobody", "Lantern Songs"), None);
    assert_eq!(pick_search(&json!({}), "The Band", "Lantern Songs"), None);
}

// ---------------------------------------------------------------- service

const ALBUM: &str = "album-55";

#[derive(Default)]
struct FakeDiscogs {
    token: bool,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl DiscogsAccess for FakeDiscogs {
    async fn has_token(&self) -> bool {
        self.token
    }
    async fn owned_release(&self, _: &str) -> Option<u64> {
        None
    }
    async fn search(&self, artist: &str, title: &str) -> AppResult<Value> {
        self.calls.lock().push(format!("search {artist} {title}"));
        Ok(json!({"results": [{"id": 4242, "title": format!("{artist} - {title}")}]}))
    }
    async fn release(&self, id: u64) -> AppResult<Option<Value>> {
        self.calls.lock().push(format!("release {id}"));
        Ok(Some(discogs_release()))
    }
}

#[derive(Default)]
struct FakePlex {
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl PlexCredits for FakePlex {
    async fn track_credits(&self, track_id: &str) -> AppResult<Vec<PlexCredit>> {
        self.calls.lock().push(track_id.to_owned());
        Ok(if track_id == "t2" { vec![PlexCredit { name: "Dee Lin".into(), roles: vec!["lyricist".into()] }] } else { vec![] })
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<String>>);
impl SleeveEvents for Events {
    fn changed(&self, id: &str) {
        self.0.lock().push(id.to_owned());
    }
}

struct Rig {
    svc: SleeveService,
    info: ArtistInfoService,
    http: Arc<FakeHttp>,
    discogs: Arc<FakeDiscogs>,
    plex: Arc<FakePlex>,
    events: Arc<Events>,
    db: DbHandle,
    _dir: tempfile::TempDir,
}

/// The Band's two-song album (released), a Producer tag on the first song's
/// file, and Ada Moss in the library.
fn rig(format: &str, discogs_token: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("s.db")).unwrap();
    let format = format.to_owned();
    db.write(move |c| {
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('a1', 'The Band', 'Band')", [])?;
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('77', 'Ada Moss', 'Moss')", [])?;
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, year, studio)
             VALUES (?1, 'Lantern Songs', 'Lantern Songs', 'a1', 'The Band', ?2, 1997, 'Low Tide Music')",
            rusqlite::params![ALBUM, format],
        )?;
        for (id, title, n) in [("t1", "Opening", 1), ("t2", "Second", 2)] {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, disc_number, part_key)
                 VALUES (?1, ?2, ?3, 'Lantern Songs', 'a1', 'The Band', ?4, 1, ?5)",
                rusqlite::params![id, title, ALBUM, n, format!("/p/{id}")],
            )?;
        }
        let mut t = FileTags::default();
        t.set("PRODUCER", "Ada Moss");
        t.set("LABEL", "Low Tide Music");
        crate::tags::store::save(c, "/p/t1", crate::tags::store::Status::Ok, &t, 1)?;
        Ok(())
    })
    .unwrap();
    let http = Arc::new(FakeHttp::new());
    let switches = Switches::default();
    for s in [InfoSource::Wikipedia, InfoSource::Listenbrainz, InfoSource::ListenbrainzStats] {
        switches.set(s, false);
    }
    let info = ArtistInfoService::new(db.clone(), http.clone(), Arc::new(MemoryStore::new()), switches);
    let svc = SleeveService::new(db.clone(), info.clone(), SleeveSwitches::default());
    let discogs = Arc::new(FakeDiscogs { token: discogs_token, ..Default::default() });
    let plex = Arc::new(FakePlex::default());
    let events = Arc::new(Events::default());
    svc.set_discogs(discogs.clone());
    svc.set_plex(plex.clone());
    svc.set_events(events.clone());
    Rig { svc, info, http, discogs, plex, events, db, _dir: dir }
}

fn route_mb(r: &Rig) {
    let search = json!({"release-groups": [{
        "id": "bbbbbbbb-1111-4111-8111-000000000001", "title": "Lantern Songs", "score": 100,
        "artist-credit": [{"name": "The Band", "artist": {"id": "11111111-2222-3333-4444-555555555555", "name": "The Band"}}]
    }]});
    r.http.route("release-group/?query", 200, &search.to_string());
    let group = json!({"releases": [{"id": REL, "status": "Official", "date": "1997-05-21", "media": [{"track-count": 2}]}]});
    r.http.route("release-group/bbbbbbbb-1111-4111-8111-000000000001", 200, &group.to_string());
    r.http.route(&format!("ws/2/release/{REL}"), 200, &mb_release().to_string());
}

/// Waits for a condition the background refresh makes true.
async fn until(what: &str, f: impl Fn() -> bool) {
    for _ in 0..500 {
        if f() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn the_tab_answers_from_the_cache_then_looks_up_once_and_says_so() {
    let r = rig("album", true);
    route_mb(&r);
    let first = r.svc.notes(ALBUM).await.unwrap();
    assert!(first.looking, "lookups started");
    assert_eq!(first.lookups, Lookups { online: true, musicbrainz: true, discogs: true, unreleased: false });
    // Tags and Plex are there at once.
    let label = first.label.clone().unwrap();
    assert_eq!((label.value.as_str(), label.sources.as_slice()), ("Low Tide Music", [Source::Tags, Source::Plex].as_slice()));
    assert_eq!(group(&first.tracks[0].groups, "producer").people[0].artist_id.as_deref(), Some("77"));

    until("sleeve://changed", || !r.events.0.lock().is_empty()).await;
    let n = r.svc.notes(ALBUM).await.unwrap();
    assert!(!n.looking, "nothing left to look up");
    assert_eq!(r.events.0.lock().as_slice(), [ALBUM.to_string()]);
    assert_eq!(n.catalogue.as_ref().unwrap().sources, [Source::Musicbrainz, Source::Discogs]);
    assert_eq!(n.label.as_ref().unwrap().sources, [Source::Tags, Source::Plex, Source::Musicbrainz, Source::Discogs]);
    assert_eq!(n.links.len(), 2);
    let ada = &group(&n.groups, "producer").people[0];
    assert_eq!(ada.sources, [Source::Tags, Source::Musicbrainz, Source::Discogs]);
    // Plex's song credits came from the server, one request per song.
    assert_eq!(group(&n.tracks[1].groups, "lyricist").people[0].sources, [Source::Plex, Source::Discogs]);
    assert_eq!(r.plex.calls.lock().len(), 2);
    assert_eq!(r.discogs.calls.lock().as_slice(), ["search The Band Lantern Songs", "release 4242"]);
    let mb_calls = r.http.calls("musicbrainz.org");
    assert_eq!(mb_calls, 3, "search, release group, release");

    // Offline: the cache still answers and nothing is asked.
    r.svc.switches().online.store(false, Ordering::Relaxed);
    let off = r.svc.notes(ALBUM).await.unwrap();
    assert!(!off.looking && !off.lookups.online);
    assert_eq!(off.catalogue.as_ref().unwrap().value, "LT-001", "cached facts show offline");
    assert_eq!(r.http.calls("musicbrainz.org"), mb_calls);
    assert_eq!(r.discogs.calls.lock().len(), 2);
}

#[tokio::test]
async fn with_lookups_off_only_tags_and_plex_are_read() {
    let r = rig("album", true);
    route_mb(&r);
    r.svc.switches().online.store(false, Ordering::Relaxed);
    let n = r.svc.notes(ALBUM).await.unwrap();
    assert!(n.looking, "Plex's song credits are the user's own server");
    until("plex done", || !r.events.0.lock().is_empty()).await;
    assert_eq!(r.http.calls("musicbrainz.org"), 0);
    assert!(r.discogs.calls.lock().is_empty());
    let n = r.svc.notes(ALBUM).await.unwrap();
    assert!(!n.looking);
    assert!(n.links.is_empty());
}

#[tokio::test]
async fn unreleased_albums_are_never_looked_up_online() {
    let r = rig("demo", true);
    route_mb(&r);
    let n = r.svc.notes(ALBUM).await.unwrap();
    assert!(n.lookups.unreleased);
    until("plex done", || !r.events.0.lock().is_empty()).await;
    assert_eq!(r.http.calls("musicbrainz.org"), 0);
    assert!(r.discogs.calls.lock().is_empty(), "an unreleased album's name never goes to Discogs");
}

#[tokio::test]
async fn musicbrainz_goes_through_the_shared_limiter() {
    let r = rig("album", false);
    route_mb(&r);
    // The artist info's MusicBrainz pacing is paused (a 503 elsewhere): the
    // sleeve notes don't ask either.
    r.info.musicbrainz_limiter().throttled(None);
    r.svc.refresh_with(ALBUM, Due { musicbrainz: true, ..Default::default() }).await;
    assert_eq!(r.http.calls("musicbrainz.org"), 0, "paused means no request");
    let (n, due) = r.svc.cached(ALBUM, now_s()).await.unwrap();
    assert!(n.links.is_empty());
    assert!(!due.musicbrainz, "a failed lookup waits before the next try");
}

#[tokio::test(start_paused = true)]
async fn musicbrainz_requests_keep_one_a_second() {
    let r = rig("album", false);
    route_mb(&r);
    let t0 = tokio::time::Instant::now();
    r.svc.refresh_with(ALBUM, Due { musicbrainz: true, ..Default::default() }).await;
    assert_eq!(r.http.calls("musicbrainz.org"), 3);
    // Three requests at one a second: the last starts two seconds after the first.
    assert!(tokio::time::Instant::now() - t0 >= std::time::Duration::from_secs(2));
}

#[tokio::test]
async fn song_credits_are_the_albums_notes_for_one_song() {
    let r = rig("album", false);
    r.svc.switches().online.store(false, Ordering::Relaxed);
    let s = r.svc.song("t1").await.unwrap();
    assert_eq!(s.album_id, ALBUM);
    let song = s.song.unwrap();
    assert_eq!(song.title, "Opening");
    assert_eq!(group(&song.groups, "producer").people[0].name, "Ada Moss");
    assert!(r.svc.song("nope").await.is_err());
}

#[tokio::test]
async fn an_album_with_nothing_known_is_empty() {
    let r = rig("album", false);
    r.svc.switches().online.store(false, Ordering::Relaxed);
    r.db.write(|c| {
        c.execute("DELETE FROM file_tags", [])?;
        c.execute("DELETE FROM tag_credits", [])?;
        c.execute("UPDATE albums SET studio = NULL", [])?;
        Ok(())
    })
    .unwrap();
    let (n, _) = r.svc.cached(ALBUM, now_s()).await.unwrap();
    assert!(n.is_empty());
}

// ---------------------------------------------------------------- credit page

#[tokio::test]
async fn the_credit_page_lists_everything_by_role() {
    let r = rig("album", true);
    route_mb(&r);
    r.svc.refresh_with(ALBUM, Due { musicbrainz: true, discogs: true, plex: true }).await;
    let _ = r.svc.cached(ALBUM, now_s()).await.unwrap();

    let page = r.svc.credit_page("ADA  MOSS").await.unwrap().expect("Ada Moss is credited");
    assert_eq!((page.name.as_str(), page.key.as_str(), page.artist_id.as_deref()), ("Ada Moss", "ada moss", Some("77")));
    let roles: Vec<&str> = page.groups.iter().map(|g| g.role.as_str()).collect();
    assert_eq!(roles, ["producer", "mixer"], "producer first, as on the sleeve");
    let prod = &page.groups[0].albums[0];
    assert_eq!(prod.album.id, ALBUM);
    assert!(prod.whole, "credited on the album itself");
    assert_eq!(prod.sources, [Source::Tags, Source::Musicbrainz, Source::Discogs]);

    let cy = r.svc.credit_page("Cy Hart").await.unwrap().unwrap();
    let played = &cy.groups[0];
    assert_eq!(played.role, "performer");
    let a = &played.albums[0];
    assert!(!a.whole);
    assert_eq!(a.songs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Opening"]);
    assert!(a.details.contains(&"electric guitar".to_string()) && a.details.contains(&"guitar".to_string()));

    assert!(r.svc.credit_page("Nobody At All").await.unwrap().is_none());
    assert!(r.svc.credit_page("  ").await.unwrap().is_none());
}

#[tokio::test]
async fn tag_credits_cover_albums_whose_notes_were_never_opened() {
    let r = rig("album", false);
    // Only the file tag scan has run.
    let page = r.svc.credit_page("Ada Moss").await.unwrap().unwrap();
    let prod = &page.groups[0];
    assert_eq!(prod.role, "producer");
    assert_eq!(prod.albums[0].songs[0].title, "Opening");
    assert_eq!(prod.albums[0].sources, [Source::Tags]);
    // A file read again without the tag drops the credit.
    r.db.write(|c| crate::tags::store::save(c, "/p/t1", crate::tags::store::Status::Ok, &FileTags::default(), 2)).unwrap();
    assert!(r.svc.credit_page("Ada Moss").await.unwrap().is_none());
}
