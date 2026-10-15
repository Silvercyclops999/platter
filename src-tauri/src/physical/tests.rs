//! Physical collection tests: marks in the sync store, matching Discogs
//! items to library albums, the import plan, the stats, and the Discogs
//! client and service against a faked Discogs (never a real account).

use std::sync::Arc;
use std::time::Duration;

use rusqlite::params;

use super::commands::{Physical, TOKEN_ACCOUNT, pick_release};
use super::discogs::{BUSY, DiscogsClient, REFUSED, format_of};
use super::*;
use crate::artistinfo::http::HttpResponse;
use crate::artistinfo::http::fake::FakeHttp;
use crate::artistinfo::limit::Limiter;
use crate::db::DbHandle;
use crate::plex::keychain::{MemoryStore, TokenStore};
use crate::plex::state::EventSink;
use crate::plex::token::Token;
use crate::sync::store::{NAMESPACES, PHYSICAL};

const SECRET: &str = "fakeDiscogsToken1234";

struct Events(parking_lot::Mutex<Vec<String>>);
impl EventSink for Events {
    fn emit(&self, event: &str, _: serde_json::Value) {
        self.0.lock().push(event.to_string());
    }
}

fn store() -> (SyncStore, DbHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("p.db")).unwrap();
    (SyncStore::new(db.clone(), "dev1"), db, dir)
}

fn album(db: &DbHandle, id: &str, artist: &str, title: &str, plays: i64) {
    album_as(db, id, artist, title, plays, "album");
}

fn album_as(db: &DbHandle, id: &str, artist: &str, title: &str, plays: i64, format: &str) {
    let (id, artist, title, format) = (id.to_string(), artist.to_string(), title.to_string(), format.to_string());
    db.write(move |c| {
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, view_count, thumb, matched, format)
             VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
            params![id, title, format!("ar:{artist}"), artist, plays, format!("/library/metadata/{id}/thumb/1"), format],
        )?;
        Ok(())
    })
    .unwrap();
}

fn copy(format: Format) -> OwnedCopy {
    OwnedCopy { format, ..Default::default() }
}

fn lib(id: &str, artist: &str, title: &str, plays: i64) -> LibAlbum {
    LibAlbum { id: id.into(), title: title.into(), artist: artist.into(), thumb: None, plays, unreleased: false }
}

fn item(release: u64, instance: u64, artist: &str, title: &str, format: Format) -> DiscogsItem {
    DiscogsItem { release_id: release, instance_id: instance, artist: artist.into(), title: title.into(), format, ..Default::default() }
}

// ---------------------------------------------------------------- marks

#[test]
fn marks_live_in_the_sync_store_and_sync_with_the_library_group() {
    assert!(NAMESPACES.contains(&PHYSICAL.name()));
    assert!(crate::sync::engine::Groups::default().namespaces().contains(&PHYSICAL.name()));
    let none = crate::sync::engine::Groups { library: false, ..Default::default() };
    assert!(!none.namespaces().contains(&PHYSICAL.name()));

    let (s, _db, _d) = store();
    let first = set(
        &s,
        "a1",
        SetOwned {
            copies: vec![OwnedCopy { format: Format::Vinyl, pressing: "  2016 repress ".into(), colour: "Red".into(), condition: "VG+".into(), ..Default::default() }],
            title: "Lantern Songs".into(),
            artist: "The Band".into(),
            thumb: None,
        },
        100,
    )
    .unwrap()
    .unwrap();
    assert_eq!(first.copies[0].pressing, "2016 repress", "trimmed");
    assert_eq!(first.at, 100);
    // Editing keeps when it was first marked.
    let again = set(&s, "a1", SetOwned { copies: vec![copy(Format::Vinyl), copy(Format::Cd)], ..Default::default() }, 200).unwrap().unwrap();
    assert_eq!(again.at, 100);
    assert_eq!(again.formats(), vec![Format::Vinyl, Format::Cd]);
    assert_eq!(get(&s, "a1").unwrap().unwrap().copies.len(), 2);
    // No copies takes the mark off.
    assert!(set(&s, "a1", SetOwned::default(), 300).unwrap().is_none());
    assert!(get(&s, "a1").unwrap().is_none());
    assert!(set(&s, " ", SetOwned { copies: vec![copy(Format::Cd)], ..Default::default() }, 1).is_err());
    let many = SetOwned { copies: vec![copy(Format::Cd); MAX_COPIES + 1], ..Default::default() };
    assert!(set(&s, "a2", many, 1).is_err());
    let long = SetOwned { copies: vec![OwnedCopy { notes: "x".repeat(1000), ..copy(Format::Other) }], ..Default::default() };
    assert_eq!(set(&s, "a3", long, 1).unwrap().unwrap().copies[0].notes.len(), NOTE_MAX);
}

#[test]
fn formats_read_in_one_order_and_serialize_plainly() {
    let o = Owned { copies: vec![copy(Format::Other), copy(Format::Cd), copy(Format::Cd), copy(Format::Vinyl)], ..Default::default() };
    assert_eq!(o.formats(), vec![Format::Vinyl, Format::Cd, Format::Other]);
    assert_eq!(serde_json::to_value(Format::Cd).unwrap(), "cd");
    let v = serde_json::to_value(&o).unwrap();
    assert!(!crate::sync::store::looks_secret(&v), "nothing in a mark looks like a secret");
}

// ---------------------------------------------------------------- matching

#[test]
fn discogs_artist_names_lose_their_numbers_and_stars() {
    assert_eq!(clean_discogs_artist("Nirvana (2)"), "Nirvana");
    assert_eq!(clean_discogs_artist("Prince*"), "Prince");
    assert_eq!(clean_discogs_artist("Sunn O)))"), "Sunn O)))");
    assert_eq!(clean_discogs_artist("Band (UK)"), "Band (UK)");
    assert_eq!(clean_discogs_artist("Blink-182"), "Blink-182");
}

#[test]
fn matches_by_folded_names_and_by_release_already_linked() {
    let library = vec![
        lib("1", "The Beatles", "Abbey Road", 50),
        lib("2", "Björk", "Homogenic (Deluxe Edition)", 3),
        lib("3", "Various Artists", "Greatest Hits", 1),
        lib("4", "Various Artists", "Night Drive Tapes", 1),
        LibAlbum { unreleased: true, ..lib("5", "The Band", "Lantern Songs", 9) },
        lib("6", "Someone", "Renamed By Hand", 1),
    ];
    let marks = vec![(
        "6".to_string(),
        Owned { copies: vec![OwnedCopy { discogs: Some(DiscogsRef { release_id: 77, instance_id: 1 }), ..copy(Format::Vinyl) }], ..Default::default() },
    )];
    let m = Matcher::new(&library, &marks);
    assert_eq!(m.album(1, "The Beatles", "Abbey Road").as_deref(), Some("1"));
    assert_eq!(m.album(2, "Beatles*", "Abbey Road (Remastered)").as_deref(), Some("1"));
    assert_eq!(m.album(3, "Bjork (2)", "Homogenic").as_deref(), Some("2"));
    assert_eq!(m.album(4, "Various", "Night Drive Tapes").as_deref(), Some("4"));
    assert_eq!(m.album(5, "Various", "Greatest Hits"), None, "a generic compilation title says nothing");
    assert_eq!(m.album(6, "The Band", "Lantern Songs"), None, "unreleased albums never match");
    assert_eq!(m.album(77, "Anyone", "Anything").as_deref(), Some("6"), "a release linked before stays linked");
    let duo = DiscogsItem { first_artist: "The Beatles".into(), ..item(8, 8, "The Beatles & Friends", "Abbey Road", Format::Cd) };
    assert_eq!(m.item(&duo).as_deref(), Some("1"), "a shared credit matches on its first artist");
}

// ---------------------------------------------------------------- importing

#[test]
fn an_import_marks_matches_keeps_edits_and_lists_the_rest() {
    let library = vec![lib("1", "Rue", "Cold Coast", 10), lib("2", "Rue", "Harbour", 4), lib("3", "Lake", "Shore", 2)];
    let marks = vec![
        // Bought on CD (added by hand) and on Discogs as a record, edited since.
        (
            "1".to_string(),
            Owned {
                copies: vec![
                    copy(Format::Cd),
                    OwnedCopy { format: Format::Vinyl, notes: "signed".into(), discogs: Some(DiscogsRef { release_id: 10, instance_id: 100 }), ..Default::default() },
                ],
                title: "Cold Coast".into(),
                artist: "Rue".into(),
                thumb: None,
                at: 5,
            },
        ),
        // Sold since: only on Discogs before.
        (
            "3".to_string(),
            Owned { copies: vec![OwnedCopy { discogs: Some(DiscogsRef { release_id: 30, instance_id: 300 }), ..copy(Format::Cassette) }], ..Default::default() },
        ),
        // Not in the library right now (another server offline): left alone.
        (
            "gone".to_string(),
            Owned { copies: vec![OwnedCopy { discogs: Some(DiscogsRef { release_id: 40, instance_id: 400 }), ..copy(Format::Vinyl) }], ..Default::default() },
        ),
    ];
    let items = vec![
        item(10, 100, "Rue", "Cold Coast", Format::Vinyl),
        item(11, 101, "Rue", "Cold Coast", Format::Cassette),
        item(20, 200, "Rue", "Harbour", Format::Cd),
        item(90, 900, "Zed", "Unknown Record", Format::Vinyl),
        item(91, 901, "Abe (3)", "Another", Format::Other),
    ];
    let plan = plan_import(&library, &marks, &items, "listener", 1000);
    let get = |id: &str| plan.ops.iter().find(|(k, _)| k == id).map(|(_, o)| o.clone());
    let one = get("1").unwrap().unwrap();
    assert_eq!(one.copies.len(), 3);
    assert_eq!(one.copies[0], copy(Format::Cd), "copies added by hand stay");
    assert_eq!(one.copies[1].notes, "signed", "the user's edits to a Discogs copy stay");
    assert_eq!(one.copies[2].format, Format::Cassette);
    assert_eq!(one.at, 5);
    let two = get("2").unwrap().unwrap();
    assert_eq!((two.title.as_str(), two.artist.as_str(), two.at), ("Harbour", "Rue", 1000));
    assert_eq!(get("3"), Some(None), "a record no longer in the collection goes");
    assert_eq!(get("gone"), None);
    let r = &plan.report;
    assert_eq!((r.total, r.matched, r.added), (5, 3, 1));
    assert_eq!(r.unmatched.iter().map(|u| u.title.as_str()).collect::<Vec<_>>(), vec!["Another", "Unknown Record"]);
    assert_eq!(r.unmatched[0].artist, "Abe");
    let with = r.clone().with_links();
    assert!(with.unmatched[1].links.bandcamp.contains("Zed"));

    // The same import again changes nothing.
    let (s, _db, _d) = store();
    for (k, o) in &marks {
        s.put(&PHYSICAL, k, o).unwrap();
    }
    apply_plan(&s, &plan).unwrap();
    let marks2 = list(&s).unwrap();
    assert_eq!(marks2.len(), 3, "1, 2 and the one not in the library");
    let again = plan_import(&library, &marks2, &items, "listener", 2000);
    assert!(again.ops.is_empty());
    assert_eq!(again.report.added, 0);
}

// ---------------------------------------------------------------- stats

#[test]
fn stats_count_formats_most_played_and_whats_missing() {
    let library = vec![
        lib("1", "A", "One", 90),
        lib("2", "B", "Two", 70),
        LibAlbum { unreleased: true, ..lib("3", "C", "Demo", 60) },
        lib("4", "D", "Four", 40),
        lib("5", "E", "Five", 0),
    ];
    let marks = vec![
        ("2".to_string(), Owned { copies: vec![copy(Format::Vinyl), copy(Format::Vinyl), copy(Format::Cd)], ..Default::default() }),
        ("5".to_string(), Owned { copies: vec![copy(Format::Cassette)], ..Default::default() }),
        ("elsewhere".to_string(), Owned { copies: vec![copy(Format::Cd)], ..Default::default() }),
    ];
    let s = stats(&library, &marks, false);
    assert_eq!((s.albums, s.copies), (2, 4), "only albums in the library count");
    assert_eq!(
        s.by_format,
        vec![
            FormatCount { format: Format::Vinyl, albums: 1, copies: 2 },
            FormatCount { format: Format::Cd, albums: 1, copies: 1 },
            FormatCount { format: Format::Cassette, albums: 1, copies: 1 },
        ]
    );
    assert_eq!(s.most_played.iter().map(|a| a.album_id.as_str()).collect::<Vec<_>>(), vec!["2"], "never-played albums aren't on it");
    assert!(s.missing.is_none(), "off by default");
    let s = stats(&library, &marks, true);
    let missing: Vec<&str> = s.missing.as_ref().unwrap().iter().map(|a| a.album_id.as_str()).collect();
    assert_eq!(missing, vec!["1", "4"], "played, not owned, not unreleased");
}

#[test]
fn the_library_reads_plays_and_unreleased_from_the_cache() {
    let (_s, db, _d) = store();
    album(&db, "1", "A", "One", 3);
    album_as(&db, "2", "B", "Two", 9, "demo");
    let l = db.read(library).unwrap();
    assert_eq!(l.iter().map(|a| (a.id.as_str(), a.plays, a.unreleased)).collect::<Vec<_>>(), vec![("2", 9, true), ("1", 3, false)]);
}

// ---------------------------------------------------------------- Discogs

const IDENTITY: &str = r#"{"id": 1, "username": "shelf user", "resource_url": "x"}"#;

fn page(n: usize, pages: usize, releases: &str) -> String {
    format!(r#"{{"pagination": {{"page": {n}, "pages": {pages}, "per_page": 100, "items": 3}}, "releases": [{releases}]}}"#)
}

const RECORD: &str = r#"{"id": 10, "instance_id": 100, "date_added": "2020-01-01T00:00:00-08:00",
  "basic_information": {"id": 10, "master_id": 5, "title": "Cold Coast", "year": 2016,
    "thumb": "https://i.discogs.com/abc/thumb.jpg", "cover_image": "https://i.discogs.com/abc/big.jpg",
    "formats": [{"name": "Vinyl", "qty": "1", "text": "Red Translucent", "descriptions": ["LP", "Album", "Reissue"]}],
    "artists": [{"name": "Rue (2)", "anv": "", "join": "&", "id": 1}, {"name": "Lake*", "anv": "", "join": "", "id": 2}]},
  "notes": [{"field_id": 1, "value": "Near Mint (NM or M-)"}, {"field_id": 2, "value": "Very Good Plus (VG+)"}, {"field_id": 3, "value": "Signed"}]}"#;

const TAPE: &str = r#"{"id": 20, "instance_id": 200, "basic_information": {"title": "Harbour", "year": 0, "thumb": "https://evil.example/x.jpg",
  "formats": [{"name": "Box Set", "qty": "1"}, {"name": "Cassette", "qty": "1", "descriptions": ["Album"]}],
  "artists": [{"name": "Rue (2)", "join": ""}]}}"#;

const ODD: &str = r#"{"id": 30, "instance_id": 300, "basic_information": {"title": "Files", "formats": [{"name": "File"}], "artists": [{"name": "Zed"}]}}"#;

fn client(http: Arc<FakeHttp>) -> DiscogsClient {
    DiscogsClient { http, limiter: Arc::new(Limiter::per_second(1.0)) }
}

fn token() -> Token {
    Token::new(SECRET).unwrap()
}

#[test]
fn discogs_formats_map_to_ours() {
    assert_eq!(format_of("Vinyl"), Format::Vinyl);
    assert_eq!(format_of("CDr"), Format::Cd);
    assert_eq!(format_of("cassette"), Format::Cassette);
    assert_eq!(format_of("Box Set"), Format::Other);
}

#[tokio::test(start_paused = true)]
async fn reads_the_collection_page_by_page_with_the_token_only_in_a_header() {
    let http = Arc::new(FakeHttp::new());
    http.route("/oauth/identity", 200, IDENTITY);
    http.route("&page=1", 200, &page(1, 2, &format!("{RECORD},{TAPE}")));
    http.route("&page=2", 200, &page(2, 2, ODD));
    let c = client(http.clone());
    assert_eq!(c.identity(&token()).await.unwrap(), "shelf user");
    let t0 = tokio::time::Instant::now();
    let items = c.collection(&token(), "shelf user").await.unwrap();
    assert!(tokio::time::Instant::now() - t0 >= Duration::from_secs(1), "paced to one a second");
    assert_eq!(items.len(), 3);
    let r = &items[0];
    assert_eq!((r.release_id, r.instance_id, r.year), (10, 100, Some(2016)));
    assert_eq!(r.artist, "Rue & Lake");
    assert_eq!(r.format, Format::Vinyl);
    assert_eq!(r.pressing, "LP, Album, Reissue");
    assert_eq!(r.colour, "Red Translucent");
    assert_eq!(r.condition, "Near Mint (NM or M-), sleeve Very Good Plus (VG+)");
    assert_eq!(r.notes, "Signed");
    assert_eq!(r.thumb.as_deref(), Some("/ext/https://i.discogs.com/abc/thumb.jpg"));
    let t = &items[1];
    assert_eq!((t.format, t.year, t.thumb.as_deref()), (Format::Cassette, None, None), "the box's cassette; covers only from Discogs");
    assert_eq!(items[2].format, Format::Other);

    let urls = http.urls();
    assert!(urls.iter().any(|u| u.contains("/users/shelf%20user/collection/folders/0/releases?per_page=100&page=2")));
    for (url, headers) in http.log.lock().iter() {
        assert!(!url.contains(SECRET), "never in a URL");
        assert!(headers.iter().any(|(k, v)| *k == "Authorization" && v == &format!("Discogs token={SECRET}")));
    }
}

#[tokio::test(start_paused = true)]
async fn discogs_failures_read_plainly_and_never_carry_the_token() {
    let http = Arc::new(FakeHttp::new());
    let c = client(http.clone());
    http.route("/oauth/identity", 401, r#"{"message": "You must authenticate to access this resource."}"#);
    let e = c.identity(&token()).await.unwrap_err();
    assert_eq!(e.to_string(), format!("invalid: {REFUSED}"));
    http.route_resp("/oauth/identity", HttpResponse { status: 429, retry_after: Some(30), ..Default::default() });
    let e = c.identity(&token()).await.unwrap_err();
    assert_eq!(e.to_string(), format!("server: {BUSY}"));
    tokio::time::advance(Duration::from_secs(31)).await;
    http.fail("/oauth/identity");
    let e = c.identity(&token()).await.unwrap_err();
    assert!(e.to_string().contains("Couldn't reach Discogs"));
    http.route("/oauth/identity", 200, "not json");
    assert!(c.identity(&token()).await.is_err());
    http.route("/oauth/identity", 500, "");
    tokio::time::advance(Duration::from_secs(600)).await;
    let e = c.identity(&token()).await.unwrap_err();
    assert!(!e.to_string().contains(SECRET));
}

// ---------------------------------------------------------------- the service

fn service(http: Arc<FakeHttp>) -> (Physical, Arc<MemoryStore>, Arc<Events>, DbHandle, SyncStore, tempfile::TempDir) {
    let (s, db, dir) = store();
    let keys = Arc::new(MemoryStore::new());
    let events = Arc::new(Events(Default::default()));
    let p = Physical::new(db.clone(), s.clone(), keys.clone(), client(http), events.clone(), Arc::new(|| 1_700_000_000));
    (p, keys, events, db, s, dir)
}

#[tokio::test(start_paused = true)]
async fn the_token_is_checked_before_its_saved_and_imports_read_the_collection() {
    let http = Arc::new(FakeHttp::new());
    let (p, keys, events, db, s, _d) = service(http.clone());
    album(&db, "1", "Rue", "Cold Coast", 12);
    album(&db, "2", "Rue", "Harbour", 3);
    assert!(!p.status().await.unwrap().saved);
    assert!(p.import().await.unwrap_err().to_string().contains("Add your Discogs token"));

    // Refused: nothing saved.
    http.route("/oauth/identity", 401, "{}");
    assert!(p.set_token(Some(SECRET.into())).await.is_err());
    assert!(keys.get(TOKEN_ACCOUNT).unwrap().is_none());

    http.route("/oauth/identity", 200, IDENTITY);
    let st = p.set_token(Some(format!("  {SECRET} "))).await.unwrap();
    assert!(st.saved);
    assert_eq!(st.username.as_deref(), Some("shelf user"));
    assert_eq!(keys.get(TOKEN_ACCOUNT).unwrap().unwrap().expose(), SECRET, "in the keychain, trimmed");

    http.route("&page=1", 200, &page(1, 1, &format!("{RECORD},{TAPE},{ODD}")));
    let r = p.import().await.unwrap();
    assert_eq!((r.total, r.matched, r.added, r.unmatched.len()), (3, 2, 2, 1));
    assert_eq!(r.unmatched[0].title, "Files");
    assert!(r.unmatched[0].links.qobuz.contains("Zed"));
    let owned = p.list().await.unwrap();
    assert_eq!(owned.iter().map(|o| (o.album_id.as_str(), o.formats.clone())).collect::<Vec<_>>(), vec![("1", vec![Format::Vinyl]), ("2", vec![Format::Cassette])]);
    assert!(s.get(&PHYSICAL, "1").unwrap().is_some(), "in the sync store");
    assert!(events.0.lock().iter().any(|e| e == super::commands::EV_CHANGED));

    let st = p.status().await.unwrap();
    assert!(!st.importing);
    assert_eq!(st.report.as_ref().unwrap().matched, 2, "the report is kept");
    let json = serde_json::to_string(&st).unwrap();
    assert!(!json.contains(SECRET), "the token never reaches the UI");

    // Stats see it; "missing" follows the setting.
    let stats = p.stats().await.unwrap();
    assert_eq!(stats.albums, 2);
    assert!(stats.missing.is_none());

    // Removing the token forgets the report but keeps the marks.
    let st = p.set_token(None).await.unwrap();
    assert!(!st.saved && st.report.is_none() && st.username.is_none());
    assert!(keys.get(TOKEN_ACCOUNT).unwrap().is_none());
    assert_eq!(p.list().await.unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn an_import_that_fails_changes_nothing() {
    let http = Arc::new(FakeHttp::new());
    let (p, keys, _e, db, _s, _d) = service(http.clone());
    album(&db, "1", "Rue", "Cold Coast", 1);
    keys.set(TOKEN_ACCOUNT, &token()).unwrap();
    http.route("/oauth/identity", 200, IDENTITY);
    http.fail("/collection/");
    assert!(p.import().await.is_err());
    assert!(p.list().await.unwrap().is_empty());
    assert!(!p.status().await.unwrap().importing, "not stuck importing");
    // An empty library (still loading) refuses rather than unmarking.
    let (p2, keys2, _e2, _db2, _s2, _d2) = service(http.clone());
    keys2.set(TOKEN_ACCOUNT, &token()).unwrap();
    http.route("/collection/", 200, &page(1, 1, RECORD));
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(p2.import().await.unwrap_err().to_string().contains("still loading"));
}

#[tokio::test]
async fn marks_set_through_the_service_tell_the_ui() {
    let http = Arc::new(FakeHttp::new());
    let (p, _k, events, _db, _s, _d) = service(http);
    let o = p
        .set("a9".into(), SetOwned { copies: vec![copy(Format::Cd)], title: "T".into(), artist: "A".into(), thumb: None })
        .await
        .unwrap()
        .unwrap();
    assert_eq!((o.album_id.as_str(), o.formats.clone(), o.at), ("a9", vec![Format::Cd], 1_700_000_000));
    assert_eq!(events.0.lock().len(), 1);
    assert!(p.set("a9".into(), SetOwned::default()).await.unwrap().is_none());
    assert!(p.list().await.unwrap().is_empty());
}

#[test]
fn a_shelf_item_finds_its_musicbrainz_album_by_folded_names() {
    let cat = |t: &str, a: &str| crate::discover::CatAlbum {
        release_group_id: format!("rg-{t}-{a}"),
        title: t.into(),
        artist: a.into(),
        artist_mbid: None,
        kind: "album".into(),
        secondary: vec![],
        year: None,
        thumb: None,
        library_album_id: None,
        request: None,
        refusal: None,
        in_missing: false,
    };
    let found = vec![cat("Cold Coast Live", "Rue"), cat("Cold Coast", "Other"), cat("Cold Coast (Deluxe)", "Rue")];
    assert_eq!(pick_release(found.clone(), "Rue (2)", "Cold Coast").unwrap().release_group_id, "rg-Cold Coast (Deluxe)-Rue");
    assert!(pick_release(found.clone(), "Nobody", "Cold Coast").is_none(), "same title, other artist: not it");
    assert!(pick_release(found, "Rue", "").is_none());
}
