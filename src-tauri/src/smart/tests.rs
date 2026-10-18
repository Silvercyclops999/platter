//! Smart playlists: rules to Plex queries and back, local evaluation,
//! limits, sorts and schedules, saving to a faked Plex (and falling back
//! when it refuses), and prompted playlists against a faked AI provider.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use super::eval::{self, Row};
use super::plexuri::{self, TagIds};
use super::prompt::{self, Anthropic};
use super::rules::*;
use super::*;
use crate::plex::keychain::{MemoryStore, TokenStore};
use crate::plex::token::Token;
use crate::web::fake::FakeWeb;

const NOW: i64 = 1_800_000_000;
const DAY: i64 = 24 * 3600;

fn rule(field: Field, op: Op, value: &str) -> Rule {
    Rule { field, op, value: value.into(), plex_field: None }
}

fn tags() -> TagIds {
    let mut t = HashMap::new();
    t.insert("album.genre".to_string(), vec![("11".to_string(), "Rock".to_string()), ("12".to_string(), "Jazz".to_string())]);
    t.insert("album.mood".to_string(), vec![("31".to_string(), "Night".to_string())]);
    t.insert("track.genre".to_string(), vec![("51".to_string(), "Rock".to_string())]);
    t
}

// ---- Plex queries -------------------------------------------------------------

#[test]
fn rules_become_a_plex_query_and_read_back_the_same() {
    let rules = SmartRules {
        match_all: true,
        rules: vec![
            rule(Field::Genre, Op::Is, "rock"),
            rule(Field::Artist, Op::Contains, "The Band"),
            rule(Field::Year, Op::Gt, "1990"),
            rule(Field::LastPlayed, Op::NotInLast, "6m"),
            rule(Field::Rating, Op::Gt, "3.5"),
            rule(Field::Liked, Op::Is, "true"),
            rule(Field::Decade, Op::IsNot, "1980"),
        ],
        limit: Some(Limit { unit: LimitUnit::Songs, value: 50 }),
        sort: SortBy::Random,
        update: Update::Live,
        extra: vec![],
    };
    let q = plexuri::build_query(&rules, 10.0, &tags()).unwrap();
    assert!(q.starts_with("type=10&album.genre=11&artist.title=The%20Band&album.year%3E%3E=1990"), "{q}");
    assert!(q.contains("track.lastViewedAt%3C%3C=-6mon"), "{q}");
    assert!(q.contains("track.userRating%3E%3E=7&track.userRating%3E%3E=9&album.decade%21=1980"), "{q}");
    assert!(q.ends_with("sort=random&limit=50"), "{q}");
    let uri = plexuri::smart_uri("m1", "7", &q);
    assert!(uri.starts_with("server://m1/com.plexapp.plugins.library/library/sections/7/all?type=10"));

    // As Plex reports it: library://x/directory/<the path, encoded again>.
    let content = format!("library://x/directory/{}", percent_encoding::utf8_percent_encode(&format!("/library/sections/7/all?{q}"), percent_encoding::NON_ALPHANUMERIC));
    let parsed = plexuri::parse_content(&content).unwrap();
    assert_eq!(parsed.section_id.as_deref(), Some("7"));
    let back = plexuri::rules_from(&parsed, 10.0, &tags()).unwrap();
    let mut want = rules.clone();
    want.rules[0].value = "Rock".into(); // Plex's spelling of the tag
    assert_eq!(back, want);
}

#[test]
fn match_any_wraps_the_conditions() {
    let rules = SmartRules {
        match_all: false,
        rules: vec![rule(Field::Mood, Op::Is, "Night"), rule(Field::PlayCount, Op::Lt, "2")],
        ..Default::default()
    };
    let q = plexuri::build_query(&rules, 10.0, &tags()).unwrap();
    assert_eq!(q, "type=10&push=1&album.mood=31&or=1&track.viewCount%3C%3C=2&pop=1");
    let parsed = plexuri::parse_content(&format!("/library/sections/7/all?{q}")).unwrap();
    let back = plexuri::rules_from(&parsed, 10.0, &tags()).unwrap();
    assert!(!back.match_all);
    assert_eq!(back.rules, rules.rules);
}

/// The shapes Plex Web's own smart playlists come back in (read-only probe).
#[test]
fn existing_plex_smart_playlists_map_to_the_editor() {
    let liked = "library://x/directory/%2Flibrary%2Fsections%2F7%2Fall%3Ftype%3D10%26track.userRating%253E%253E%3D4%26group%3Dguid%26having%3Dmin(album.originallyAvailableAt)%26sort%3Dtrack.lastRatedAt";
    let r = plexuri::rules_from(&plexuri::parse_content(liked).unwrap(), 10.0, &TagIds::new()).unwrap();
    assert_eq!(r.rules, vec![rule(Field::Rating, Op::Gt, "2")]);
    assert_eq!(r.sort, SortBy::None);
    // One copy per song and Plex's own sort are kept as they are.
    assert!(r.extra.contains(&("group".into(), "guid".into())));
    assert!(r.extra.contains(&("sort".into(), "track.lastRatedAt".into())));
    let q = plexuri::build_query(&r, 10.0, &TagIds::new()).unwrap();
    assert!(q.contains("group=guid") && q.contains("having=min%28album.originallyAvailableAt%29") && q.contains("sort=track.lastRatedAt"), "{q}");

    let recent = "library://x/directory/%2Flibrary%2Fsections%2F5%2Fall%3Ftype%3D10%26album.addedAt%253E%253E%3D-1mon%26sort%3Dalbum.addedAt%3Adesc";
    let r = plexuri::rules_from(&plexuri::parse_content(recent).unwrap(), 10.0, &TagIds::new()).unwrap();
    assert_eq!(r.rules, vec![rule(Field::AddedAt, Op::InLast, "1m")]);

    let played = "library://x/directory/%2Flibrary%2Fsections%2F5%2Fall%3Ftype%3D10%26track.lastViewedAt%253E%253E%3D-2w%26group%3Dguid%26having%3Dmin(album.originallyAvailableAt)%26sort%3Dtrack.lastViewedAt%3Adesc";
    let r = plexuri::rules_from(&plexuri::parse_content(played).unwrap(), 10.0, &TagIds::new()).unwrap();
    assert_eq!(r.rules, vec![rule(Field::LastPlayed, Op::InLast, "2w")]);
    assert_eq!(r.sort, SortBy::RecentlyPlayed);

    let everything = "library://x/directory/%2Flibrary%2Fsections%2F7%2Fall%3Ftype%3D10";
    let r = plexuri::rules_from(&plexuri::parse_content(everything).unwrap(), 10.0, &TagIds::new()).unwrap();
    assert!(r.rules.is_empty() && r.match_all);
}

#[test]
fn rules_the_editor_cant_show_say_so() {
    let title = "library://x/directory/%2Flibrary%2Fsections%2F7%2Fall%3Ftype%3D10%26track.title%3Dlove";
    let e = plexuri::rules_from(&plexuri::parse_content(title).unwrap(), 10.0, &TagIds::new()).unwrap_err();
    assert!(e.to_string().contains("Edit it in Plex"), "{e}");
    let albums = "/library/sections/7/all?type=9";
    assert!(plexuri::parse_content(albums).is_err());
    // A track-level genre keeps its Plex field when saved back.
    let tg = "/library/sections/7/all?type=10&track.genre=51";
    let r = plexuri::rules_from(&plexuri::parse_content(tg).unwrap(), 10.0, &tags()).unwrap();
    assert_eq!(r.rules[0].plex_field.as_deref(), Some("track.genre"));
    assert_eq!(plexuri::build_query(&r, 10.0, &tags()).unwrap(), "type=10&track.genre=51");
}

#[test]
fn rules_plex_cant_hold_are_named() {
    let r = SmartRules {
        rules: vec![rule(Field::Explicit, Op::Is, "false"), rule(Field::PlayedHere, Op::InLast, "7d")],
        limit: Some(Limit { unit: LimitUnit::Hours, value: 2 }),
        update: Update::Daily,
        ..Default::default()
    };
    assert_eq!(r.plex_gaps().len(), 4);
    assert!(plexuri::build_query(&r, 10.0, &tags()).is_err());
    let unknown_genre = SmartRules { rules: vec![rule(Field::Genre, Op::Is, "Polka")], ..Default::default() };
    assert!(plexuri::build_query(&unknown_genre, 10.0, &tags()).unwrap_err().to_string().contains("Polka"));
}

#[test]
fn values_are_checked() {
    assert!(rule(Field::Rating, Op::Gt, "6").check().is_err());
    assert!(rule(Field::Decade, Op::Is, "1995").check().is_err());
    assert!(rule(Field::AddedAt, Op::InLast, "soon").check().is_err());
    assert!(rule(Field::Genre, Op::Gt, "Rock").check().is_err());
    assert!(rule(Field::ReleaseType, Op::Is, "live").check().is_ok());
    assert!(rule(Field::Duration, Op::Lt, "4").check().is_ok());
}

// ---- local evaluation -----------------------------------------------------------

fn row(id: &str, artist: &str, year: i32, plays: u32, minutes: u64) -> Row {
    Row {
        id: id.into(),
        title: format!("Song {id}"),
        artist_id: format!("a-{artist}"),
        artist: artist.into(),
        album: format!("{artist} LP"),
        track: id.parse().unwrap_or(0),
        year: Some(year),
        added_at: NOW - 400 * DAY,
        plays,
        duration_ms: minutes * 60_000,
        section: "albums".into(),
        genres: vec!["Rock".into()],
        ..Default::default()
    }
}

fn rows() -> Vec<Row> {
    let mut v = vec![row("1", "Alpha", 1975, 9, 3), row("2", "Alpha", 1975, 0, 4), row("3", "Beta", 1992, 4, 5), row("4", "Gamma", 2004, 1, 50)];
    v[1].rating = Some(10.0);
    v[2].genres = vec!["Jazz".into()];
    v[2].moods = vec!["Night".into()];
    v[2].played_here = Some(NOW - 2 * DAY);
    v[2].added_at = NOW - 3 * DAY;
    v[3].explicit = true;
    v[3].section = "live".into();
    v[3].last_played = Some(NOW - DAY);
    v[3].version = Some("Remaster".into());
    v
}

fn ids(v: Vec<&Row>) -> Vec<&str> {
    v.into_iter().map(|r| r.id.as_str()).collect()
}

#[test]
fn each_field_evaluates_locally() {
    let rows = rows();
    let run = |r: Rule| ids(eval::evaluate(&rows, &SmartRules { rules: vec![r], ..Default::default() }, NOW, 10.0, 0));
    assert_eq!(run(rule(Field::Genre, Op::Is, "jazz")), ["3"]);
    assert_eq!(run(rule(Field::Mood, Op::IsNot, "Night")), ["1", "2", "4"]);
    assert_eq!(run(rule(Field::Artist, Op::Contains, "alp")), ["1", "2"]);
    assert_eq!(run(rule(Field::Year, Op::Lt, "1990")), ["1", "2"]);
    assert_eq!(run(rule(Field::Decade, Op::Is, "1990")), ["3"]);
    assert_eq!(run(rule(Field::AddedAt, Op::InLast, "1w")), ["3"]);
    assert_eq!(run(rule(Field::PlayCount, Op::Is, "0")), ["2"]);
    assert_eq!(run(rule(Field::LastPlayed, Op::InLast, "2d")), ["4"]);
    assert_eq!(run(rule(Field::PlayedHere, Op::NotInLast, "1w")), ["1", "2", "4"]);
    assert_eq!(run(rule(Field::Rating, Op::Gt, "4")), ["2"]);
    assert_eq!(run(rule(Field::Liked, Op::Is, "true")), ["2"]);
    assert_eq!(run(rule(Field::Duration, Op::Gt, "4.5")), ["3", "4"]);
    assert_eq!(run(rule(Field::ReleaseType, Op::Is, "live")), ["4"]);
    assert_eq!(run(rule(Field::Explicit, Op::Is, "false")), ["1", "2", "3"]);
    assert_eq!(run(rule(Field::Version, Op::Contains, "master")), ["4"]);
}

#[test]
fn match_all_any_limits_and_sorts() {
    let rows = rows();
    let any = SmartRules { match_all: false, rules: vec![rule(Field::Genre, Op::Is, "Jazz"), rule(Field::Liked, Op::Is, "true")], ..Default::default() };
    assert_eq!(ids(eval::evaluate(&rows, &any, NOW, 10.0, 0)), ["2", "3"]);
    let all = SmartRules { rules: vec![rule(Field::Genre, Op::Is, "Rock"), rule(Field::Artist, Op::Is, "alpha")], ..Default::default() };
    assert_eq!(ids(eval::evaluate(&rows, &all, NOW, 10.0, 0)), ["1", "2"]);

    let sorted = |sort, limit| ids(eval::evaluate(&rows, &SmartRules { sort, limit, ..Default::default() }, NOW, 10.0, 0));
    assert_eq!(sorted(SortBy::MostPlayed, None), ["1", "3", "4", "2"]);
    assert_eq!(sorted(SortBy::LeastPlayed, Some(Limit { unit: LimitUnit::Songs, value: 2 })), ["2", "4"]);
    assert_eq!(sorted(SortBy::RecentlyAdded, Some(Limit { unit: LimitUnit::Songs, value: 1 })), ["3"]);
    assert_eq!(sorted(SortBy::Rating, Some(Limit { unit: LimitUnit::Songs, value: 1 })), ["2"]);
    // An hour fits songs of 3, 4 and 5 minutes, then not the 50.
    assert_eq!(sorted(SortBy::None, Some(Limit { unit: LimitUnit::Hours, value: 1 })), ["1", "2", "3"]);
    let short = SmartRules { limit: Some(Limit { unit: LimitUnit::Hours, value: 1 }), sort: SortBy::MostPlayed, rules: vec![rule(Field::Duration, Op::Lt, "60")], ..Default::default() };
    // Most played first: 3, 5 and 50 minutes fill the hour; the 4 doesn't fit.
    assert_eq!(ids(eval::evaluate(&rows, &short, NOW, 10.0, 0)), ["1", "3", "4"]);
    // Random order holds for one seed and changes with another.
    let r1 = sorted(SortBy::Random, None);
    assert_eq!(r1, sorted(SortBy::Random, None));
    let other: Vec<Vec<String>> = (1..20u64)
        .map(|s| eval::evaluate(&rows, &SmartRules { sort: SortBy::Random, ..Default::default() }, NOW, 10.0, s).into_iter().map(|r| r.id.clone()).collect())
        .collect();
    assert!(other.iter().any(|o| o.iter().map(String::as_str).collect::<Vec<_>>() != r1));
}

// ---- the service ------------------------------------------------------------------

#[derive(Default)]
pub(super) struct FakePlex {
    online: Mutex<bool>,
    refuse: Mutex<bool>,
    pub(super) created: Mutex<Vec<(String, String)>>,
    pub(super) updated: Mutex<Vec<(String, String)>>,
    content: Mutex<String>,
}

#[async_trait]
impl SmartRemote for FakePlex {
    async fn online(&self) -> bool {
        *self.online.lock()
    }
    async fn target(&self) -> AppResult<(String, String)> {
        Ok(("m1".into(), "7".into()))
    }
    async fn tags(&self, _s: &str, f: &str) -> AppResult<Vec<(String, String)>> {
        Ok(tags().get(f).cloned().unwrap_or_default())
    }
    async fn create(&self, title: &str, uri: &str) -> AppResult<String> {
        if *self.refuse.lock() {
            return Err(AppError::Invalid("Unexpected response 400.".into()));
        }
        self.created.lock().push((title.into(), uri.into()));
        Ok("900".into())
    }
    async fn update(&self, id: &str, uri: &str) -> AppResult<()> {
        self.updated.lock().push((id.into(), uri.into()));
        Ok(())
    }
    async fn rename(&self, _id: &str, _t: &str) -> AppResult<()> {
        Ok(())
    }
    async fn content(&self, _id: &str) -> AppResult<String> {
        Ok(self.content.lock().clone())
    }
    async fn fetch(&self, id: &str) -> AppResult<(Playlist, Vec<PlaylistEntry>)> {
        Ok((Playlist { id: id.into(), title: "Smart".into(), smart: true, track_count: 0, duration_ms: 0, thumb: None, summary: None, updated_at: None }, vec![]))
    }
}

struct NoEvents;
impl EventSink for NoEvents {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

pub(super) struct Rig {
    pub(super) smart: Smart,
    pub(super) plex: Arc<FakePlex>,
    pub(super) web: Arc<FakeWeb>,
    keys: Arc<MemoryStore>,
    db: DbHandle,
    now: Arc<Mutex<i64>>,
    _dir: tempfile::TempDir,
}

fn add_track(db: &DbHandle, id: &str, album: &str, genre: &str, plays: u32, section: &str) {
    let (id, album, genre, section) = (id.to_string(), album.to_string(), genre.to_string(), section.to_string());
    db.write(move |c| {
        c.execute(
            "INSERT OR IGNORE INTO albums (id, title, sort_title, artist_id, artist_title, year, genres, section, added_at, format, subformats, matched)
             VALUES (?1, ?1, ?1, 'r', 'Band', 1999, ?2, ?3, 1, NULL, ?4, 1)",
            rusqlite::params![album, format!("[\"{genre}\"]"), section, if section == "demos" { "[\"Demo\"]" } else { "[]" }],
        )?;
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, duration_ms, part_key, view_count)
             VALUES (?1, ?2, ?3, ?3, 'r', 'Band', ?4, 180000, ?5, ?6)",
            rusqlite::params![id, format!("Song {id}"), album, id.parse::<i64>().unwrap_or(0), format!("/library/parts/{id}/1/f.flac"), plays],
        )?;
        Ok(())
    })
    .unwrap();
}

/// The rig, with FLAC files.
pub(super) fn rig_for_describe() -> Rig {
    let r = rig();
    r.db.write(|c| {
        c.execute("UPDATE tracks SET codec = 'flac'", [])?;
        Ok(())
    })
    .unwrap();
    r
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("s.db")).unwrap();
    add_track(&db, "1", "al1", "Rock", 5, "albums");
    add_track(&db, "2", "al1", "Rock", 0, "albums");
    add_track(&db, "3", "al2", "Jazz", 2, "albums");
    add_track(&db, "4", "al3", "Rock", 7, "demos");
    let plex = Arc::new(FakePlex::default());
    *plex.online.lock() = true;
    let web = Arc::new(FakeWeb::new());
    let keys = Arc::new(MemoryStore::new());
    let now = Arc::new(Mutex::new(NOW));
    let clock = now.clone();
    let smart = Smart::new(SmartDeps {
        db: db.clone(),
        store: Arc::new(SyncSmart { store: crate::sync::SyncStore::new(db.clone(), "dev1"), db: db.clone() }),
        remote: plex.clone(),
        ai: Arc::new(Anthropic::new(web.clone(), keys.clone())),
        events: Arc::new(NoEvents),
        clock: Arc::new(move || *clock.lock()),
    });
    Rig { smart, plex, web, keys, db, now, _dir: dir }
}

fn req(rules: SmartRules) -> SaveRequest {
    SaveRequest { id: None, title: "Mine".into(), summary: None, rules, picks: vec![], prompt: None, platter: false }
}

#[tokio::test]
async fn the_preview_counts_what_the_rules_find() {
    let r = rig();
    let rock = SmartRules { rules: vec![rule(Field::Genre, Op::Is, "Rock")], ..Default::default() };
    let p = r.smart.preview(rock.clone(), vec![]).await.unwrap();
    assert_eq!((p.count, p.duration_ms), (3, 3 * 180_000));
    assert_eq!(p.tracks.len(), 3);
    assert!(p.plex_gaps.is_empty());
    let played = SmartRules { rules: vec![rule(Field::Genre, Op::Is, "Rock"), rule(Field::PlayCount, Op::Gt, "0")], ..rock };
    assert_eq!(r.smart.preview(played, vec![]).await.unwrap().count, 2);
    let bad = SmartRules { rules: vec![rule(Field::Year, Op::Gt, "x")], ..Default::default() };
    assert!(r.smart.preview(bad, vec![]).await.is_err());
}

#[tokio::test]
async fn rules_plex_can_hold_become_a_plex_smart_playlist() {
    let r = rig();
    let rules = SmartRules { rules: vec![rule(Field::Genre, Op::Is, "Rock")], sort: SortBy::MostPlayed, ..Default::default() };
    let saved = r.smart.save(req(rules)).await.unwrap();
    assert_eq!(saved, Saved { id: "900".into(), platter: false, note: None });
    let (title, uri) = r.plex.created.lock()[0].clone();
    assert_eq!(title, "Mine");
    assert_eq!(uri, "server://m1/com.plexapp.plugins.library/library/sections/7/all?type=10&album.genre=11&sort=viewCount%3Adesc");
    // It's in the library cache as a smart playlist.
    let smart: bool = r.db.read(|c| Ok(c.query_row("SELECT smart FROM playlists WHERE id = '900'", [], |x| x.get(0))?)).unwrap();
    assert!(smart);
    assert!(r.smart.list().unwrap().is_empty(), "nothing kept on this computer");
}

#[tokio::test]
async fn plex_refusing_the_rules_saves_a_platter_one_and_says_why() {
    let r = rig();
    *r.plex.refuse.lock() = true;
    let rules = SmartRules { rules: vec![rule(Field::Genre, Op::Is, "Rock")], ..Default::default() };
    let saved = r.smart.save(req(rules)).await.unwrap();
    assert!(saved.platter && saved.id.starts_with(ID_PREFIX));
    assert!(saved.note.unwrap().contains("Plex didn't accept"));
    assert_eq!(r.smart.list().unwrap().len(), 1);
}

#[tokio::test]
async fn rules_plex_cant_express_are_kept_here_and_offline_plex_ones_wait() {
    let r = rig();
    let rules = SmartRules { rules: vec![rule(Field::ReleaseType, Op::IsNot, "demos")], ..Default::default() };
    let saved = r.smart.save(req(rules)).await.unwrap();
    assert!(saved.platter);
    assert!(saved.note.unwrap().contains("Release type"));
    assert!(r.plex.created.lock().is_empty());
    let page = r.smart.page(&saved.id).await.unwrap();
    assert_eq!(page.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["1", "2", "3"]);
    // Offline, a rule set Plex could hold can't be made in Plex.
    *r.plex.online.lock() = false;
    let e = r.smart.save(req(SmartRules::default())).await.unwrap_err();
    assert_eq!(e.kind(), "network");
    // ...but can be kept here on request.
    assert!(r.smart.save(SaveRequest { platter: true, ..req(SmartRules::default()) }).await.unwrap().platter);
}

#[tokio::test]
async fn daily_playlists_hold_their_songs_until_the_next_update() {
    let r = rig();
    let rules = SmartRules {
        rules: vec![rule(Field::PlayCount, Op::Gt, "0")],
        sort: SortBy::Random,
        update: Update::Daily,
        ..Default::default()
    };
    let saved = r.smart.save(req(rules)).await.unwrap();
    let first = r.smart.page(&saved.id).await.unwrap();
    assert_eq!(first.tracks.len(), 3);
    assert_eq!(first.playlist.next_update_at, Some(NOW - NOW.rem_euclid(DAY) + DAY));
    // A new song played: not in until the day turns.
    r.db.write(|c| {
        c.execute("UPDATE tracks SET view_count = 1 WHERE id = '2'", [])?;
        Ok(())
    })
    .unwrap();
    *r.now.lock() += 3600;
    assert_eq!(r.smart.page(&saved.id).await.unwrap().tracks.len(), 3);
    *r.now.lock() += DAY;
    assert_eq!(r.smart.page(&saved.id).await.unwrap().tracks.len(), 4);
}

#[tokio::test]
async fn editing_a_plex_smart_playlist_reads_and_writes_its_rules() {
    let r = rig();
    *r.plex.content.lock() = "library://x/directory/%2Flibrary%2Fsections%2F7%2Fall%3Ftype%3D10%26album.genre%3D12%26limit%3D25".into();
    let rules = r.smart.plex_rules("901").await.unwrap();
    assert_eq!(rules.rules, vec![rule(Field::Genre, Op::Is, "Jazz")]);
    assert_eq!(rules.limit, Some(Limit { unit: LimitUnit::Songs, value: 25 }));
    let mut changed = rules.clone();
    changed.rules[0].op = Op::IsNot;
    let saved = r.smart.save(SaveRequest { id: Some("901".into()), ..req(changed) }).await.unwrap();
    assert!(!saved.platter);
    assert_eq!(r.plex.updated.lock()[0].1, "server://m1/com.plexapp.plugins.library/library/sections/7/all?type=10&album.genre%21=12&limit=25");
    // Rules it can't hold aren't forced on it.
    let local = SmartRules { rules: vec![rule(Field::Explicit, Op::Is, "false")], ..Default::default() };
    assert!(r.smart.save(SaveRequest { id: Some("901".into()), ..req(local) }).await.is_err());
}

#[tokio::test]
async fn deleting_a_platter_playlist() {
    let r = rig();
    let saved = r.smart.save(SaveRequest { platter: true, ..req(SmartRules::default()) }).await.unwrap();
    r.smart.delete(&saved.id).await.unwrap();
    assert!(r.smart.list().unwrap().is_empty());
    assert!(r.smart.delete("900").await.is_err(), "Plex ones go from their own menu");
}

// ---- prompted playlists -------------------------------------------------------------

fn ai_reply(json: &str) -> String {
    serde_json::json!({ "content": [{ "type": "text", "text": json }], "stop_reason": "end_turn" }).to_string()
}

#[tokio::test]
async fn a_prompt_needs_a_provider_and_key() {
    let r = rig();
    assert!(r.smart.prompt("night drive").await.is_err());
    assert!(r.web.log.lock().is_empty());
}

#[tokio::test]
async fn a_prompt_becomes_rules_and_ranked_picks_without_unreleased_music() {
    let r = rig();
    let mut s = crate::settings::load(&r.db);
    s.ai_provider = prompt::AiProvider::Anthropic;
    crate::settings::save(&r.db, &s).unwrap();
    r.keys.set(prompt::ANTHROPIC_KEY_ACCOUNT, &Token::new("ak-test").unwrap()).unwrap();
    r.web.route(
        "api.anthropic.com",
        Some("turn a description"),
        200,
        &ai_reply(r#"{"title":"Rock Night","matchAll":true,"rules":[{"field":"genre","op":"is","value":"Rock"},{"field":"bogus","op":"is","value":"x"}],"sort":"mostPlayed","limitSongs":20}"#),
    );
    r.web.route("api.anthropic.com", Some("numbered list"), 200, &ai_reply(r#"{"picks":[2,1,2,99]}"#));
    let p = r.smart.prompt("loud rock").await.unwrap();
    assert_eq!(p.title, "Rock Night");
    assert_eq!(p.rules.rules, vec![rule(Field::Genre, Op::Is, "Rock")], "unknown fields are left out");
    assert_eq!(p.rules.limit, Some(Limit { unit: LimitUnit::Songs, value: 20 }));
    // Most played first among released songs: 1 then 2. The picks reverse that.
    assert_eq!(p.picks, ["2", "1"]);
    let log = r.web.log.lock().clone();
    assert_eq!(log.len(), 2);
    let second = crate::web::fake::body_text(&log[1]);
    assert!(second.contains("Song 1") && !second.contains("Song 4"), "the demo is never sent: {second}");
    assert!(log[0].headers.iter().any(|(k, v)| *k == "x-api-key" && v == "ak-test"));
    assert!(!crate::web::fake::body_text(&log[0]).contains("ak-test"));
    let first: serde_json::Value = serde_json::from_str(&crate::web::fake::body_text(&log[0])).unwrap();
    assert_eq!(first["model"], prompt::DEFAULT_MODEL);
    assert_eq!(first["output_config"]["format"]["type"], "json_schema");

    // Saved with its picks, it's a Platter playlist with the picks first.
    let saved = r.smart.save(SaveRequest { picks: p.picks.clone(), prompt: Some("loud rock".into()), ..req(p.rules) }).await.unwrap();
    assert!(saved.platter);
    let page = r.smart.page(&saved.id).await.unwrap();
    assert_eq!(page.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>()[..2], ["2", "1"]);
    assert!(page.playlist.prompted);
}

#[test]
fn songs_on_albums_plex_couldnt_match_count_as_unreleased() {
    let r = rig();
    add_track(&r.db, "5", "al5", "Rock", 1, "albums");
    add_track(&r.db, "6", "al6", "Rock", 1, "albums");
    r.db.write(|c| {
        // al5: Plex found no match. al6: an old cache row, no guid known, no label.
        c.execute("UPDATE albums SET matched = 0 WHERE id = 'al5'", [])?;
        c.execute("UPDATE albums SET matched = NULL WHERE id = 'al6'", [])?;
        Ok(())
    })
    .unwrap();
    let unreleased = || {
        let rows = r.db.read(eval::read_rows).unwrap();
        let mut ids: Vec<String> = rows.into_iter().filter(|x| x.unreleased).map(|x| x.id).collect();
        ids.sort();
        ids
    };
    assert_eq!(unreleased(), ["4", "5", "6"]);
    crate::db::release::set_count_unmatched(false);
    assert_eq!(unreleased(), ["4"], "off: only the demo");
    crate::db::release::set_count_unmatched(true);
}

#[tokio::test]
async fn a_declined_prompt_says_so() {
    let r = rig();
    let mut s = crate::settings::load(&r.db);
    s.ai_provider = prompt::AiProvider::Anthropic;
    crate::settings::save(&r.db, &s).unwrap();
    r.keys.set(prompt::ANTHROPIC_KEY_ACCOUNT, &Token::new("ak").unwrap()).unwrap();
    r.web.route("api.anthropic.com", None, 200, r#"{"content":[],"stop_reason":"refusal"}"#);
    assert!(r.smart.prompt("x").await.unwrap_err().to_string().contains("declined"));
    r.web.route("api.anthropic.com", None, 401, "{}");
    assert_eq!(r.smart.prompt("x").await.unwrap_err().kind(), "auth");
}

#[test]
fn a_song_without_a_year_only_matches_is_not() {
    let mut rows = rows();
    rows[0].year = None;
    let run = |r: Rule| ids(eval::evaluate(&rows, &SmartRules { rules: vec![r], ..Default::default() }, NOW, 10.0, 0));
    assert_eq!(run(rule(Field::Year, Op::Lt, "1990")), ["2"], "no year isn't year 0");
    assert_eq!(run(rule(Field::Year, Op::Gt, "1990")), ["3", "4"]);
    assert_eq!(run(rule(Field::Year, Op::Is, "0")), Vec::<&str>::new());
    assert_eq!(run(rule(Field::Year, Op::IsNot, "1975")), ["1", "3", "4"]);
}

#[tokio::test]
async fn daily_playlists_turn_over_at_local_midnight() {
    let r = rig();
    // Ten hours east of UTC.
    r.smart.set_tz(600);
    let offset = 600 * 60;
    let rules = SmartRules { rules: vec![rule(Field::PlayCount, Op::Gt, "0")], update: Update::Daily, ..Default::default() };
    let saved = r.smart.save(req(rules)).await.unwrap();
    let first = r.smart.page(&saved.id).await.unwrap();
    let local = NOW + offset;
    let midnight = local - local.rem_euclid(DAY) + DAY - offset;
    assert_eq!(first.playlist.next_update_at, Some(midnight));
    r.db.write(|c| {
        c.execute("UPDATE tracks SET view_count = 1 WHERE id = '2'", [])?;
        Ok(())
    })
    .unwrap();
    *r.now.lock() = midnight - 1;
    assert_eq!(r.smart.page(&saved.id).await.unwrap().tracks.len(), 3, "still the same local day");
    *r.now.lock() = midnight;
    assert_eq!(r.smart.page(&saved.id).await.unwrap().tracks.len(), 4, "a new local day");
}

/// Counts writes to the store.
struct Counting(SyncSmart, Arc<Mutex<usize>>);
impl SmartStore for Counting {
    fn list(&self) -> AppResult<Vec<SmartPlaylist>> {
        self.0.list()
    }
    fn save(&self, all: &[SmartPlaylist]) -> AppResult<()> {
        *self.1.lock() += 1;
        self.0.save(all)
    }
}

#[tokio::test]
async fn opening_a_live_playlist_writes_only_when_its_songs_change() {
    let r = rig();
    let saves = Arc::new(Mutex::new(0usize));
    let now = r.now.clone();
    let smart = Smart::new(SmartDeps {
        db: r.db.clone(),
        store: Arc::new(Counting(SyncSmart { store: crate::sync::SyncStore::new(r.db.clone(), "dev1"), db: r.db.clone() }, saves.clone())),
        remote: r.plex.clone(),
        ai: Arc::new(Anthropic::new(r.web.clone(), r.keys.clone())),
        events: Arc::new(NoEvents),
        clock: Arc::new(move || *now.lock()),
    });
    let rules = SmartRules { rules: vec![rule(Field::PlayCount, Op::Gt, "0")], ..Default::default() };
    let saved = smart.save(SaveRequest { platter: true, ..req(rules) }).await.unwrap();
    smart.page(&saved.id).await.unwrap();
    let after_first = *saves.lock();
    for _ in 0..3 {
        *r.now.lock() += 60;
        smart.page(&saved.id).await.unwrap();
    }
    assert_eq!(*saves.lock(), after_first, "the same songs: nothing written");
    r.db.write(|c| {
        c.execute("UPDATE tracks SET view_count = 1 WHERE id = '2'", [])?;
        Ok(())
    })
    .unwrap();
    assert_eq!(smart.page(&saved.id).await.unwrap().tracks.len(), 4);
    assert_eq!(*saves.lock(), after_first + 1);
}
