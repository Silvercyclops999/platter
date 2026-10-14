//! Service tests: matching, caching, refresh, keys and switches. All HTTP is
//! faked; nothing here reaches a real service.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;

use super::cache;
use super::http::fake::FakeHttp;
use super::service::{InfoEvents, MbidLookup};
use super::*;
use crate::db::DbHandle;
use crate::db::queries::now_s;
use crate::error::{AppError, AppResult};
use crate::plex::keychain::{MemoryStore, TokenStore};

const ARTIST: &str = "plexid-9876";
const MBID: &str = "11111111-2222-3333-4444-555555555555";

struct Mbid(Option<&'static str>);
#[async_trait]
impl MbidLookup for Mbid {
    async fn artist_mbid(&self, _: &str) -> AppResult<Option<String>> {
        Ok(self.0.map(str::to_owned))
    }
}

struct PlexDown;
#[async_trait]
impl MbidLookup for PlexDown {
    async fn artist_mbid(&self, _: &str) -> AppResult<Option<String>> {
        Err(AppError::Network("offline".into()))
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<String>>);
impl InfoEvents for Events {
    fn info_changed(&self, id: &str) {
        self.0.lock().push(id.to_owned());
    }
}

struct Rig {
    svc: ArtistInfoService,
    http: Arc<FakeHttp>,
    keys: Arc<MemoryStore>,
    events: Arc<Events>,
    db: DbHandle,
    _dir: tempfile::TempDir,
}

fn rig(summary: Option<&str>, mbid: Option<&'static str>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("i.db")).unwrap();
    let summary = summary.map(str::to_owned);
    db.write(move |c| {
        c.execute(
            "INSERT INTO artists (id, title, sort_title, thumb, summary) VALUES (?1, 'The Band', 'Band', '/library/metadata/1/thumb/1', ?2)",
            rusqlite::params![ARTIST, summary],
        )?;
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('77', 'Singer', 'Singer')", [])?;
        Ok(())
    })
    .unwrap();
    let http = Arc::new(FakeHttp::new());
    let keys = Arc::new(MemoryStore::new());
    let svc = ArtistInfoService::new(db.clone(), http.clone(), keys.clone(), Switches::default());
    svc.set_mbid_lookup(Arc::new(Mbid(mbid)));
    let events = Arc::new(Events::default());
    svc.set_events(events.clone());
    Rig { svc, http, keys, events, db, _dir: dir }
}

fn band_entity(mbid: &str) -> serde_json::Value {
    let item = |q: &str| json!({"rank": "normal", "mainsnak": {"datavalue": {"value": {"id": q}}}});
    json!({
        "claims": {
            "P31": [item("Q215380")],
            "P434": [{"rank": "preferred", "mainsnak": {"datavalue": {"value": mbid}}}],
            "P740": [item("Q100")],
            "P2031": [{"rank": "normal", "mainsnak": {"datavalue": {"value": {"time": "+1985-00-00T00:00:00Z"}}}}],
            "P527": [item("Q201"), item("Q202")]
        },
        "sitelinks": {"enwiki": {"title": "The Band (group)"}}
    })
}

const LONG_BIO: &str = "The Band are a group from a small town. They have made records for a long time and toured widely. \
    Their sound mixes guitars with tape loops, and their live shows are long. Critics have written about them often, \
    and their early singles still get played. They signed to a label, left it, and now release music themselves. \
    Members have side projects as well.";

/// Wikidata and Wikipedia answers for the band.
fn wiki_routes(http: &FakeHttp) {
    http.route("haswbstatement", 200, &json!({"query": {"search": [{"title": "Q1"}]}}).to_string());
    http.route("props=claims", 200, &json!({"entities": {"Q1": band_entity(MBID)}}).to_string());
    http.route(
        "props=labels",
        200,
        &json!({"entities": {
            "Q100": {"labels": {"en": {"value": "Abingdon"}}},
            "Q201": {"labels": {"mul": {"value": "Singer"}}},
            "Q202": {"labels": {"en": {"value": "Drummer"}}}
        }})
        .to_string(),
    );
    http.route("en.wikipedia.org", 200, &json!({"query": {"pages": [{"extract": LONG_BIO}]}}).to_string());
}

#[tokio::test]
async fn mbid_path_fills_bio_facts_members_and_credits() {
    let r = rig(Some("Short Plex bio."), Some(MBID));
    wiki_routes(&r.http);
    assert!(r.svc.refresh(ARTIST).await.unwrap());
    let (info, stale) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert!(!stale);
    assert_eq!(info.bio.as_deref(), Some(LONG_BIO));
    assert_eq!(info.bio_source, Some(BioSource::Wikipedia));
    assert_eq!(info.origin.as_deref(), Some("Abingdon"));
    assert_eq!(info.years_active.as_deref(), Some("1985–present"));
    assert_eq!(info.members[0], Member { name: "Singer".into(), artist_id: Some("77".into()) });
    assert_eq!(info.members[1].artist_id, None);
    let wp = &info.credits[0];
    assert_eq!((wp.text.as_str(), wp.licence.as_deref()), ("From Wikipedia", Some("CC BY-SA 4.0")));
    assert_eq!(wp.url.as_deref(), Some("https://en.wikipedia.org/wiki/The_Band_%28group%29"));
    // Found by MusicBrainz id: no name search.
    assert_eq!(r.http.calls("wbsearchentities"), 0);
    assert_eq!(*r.events.0.lock(), [ARTIST]);
    // Only the name or MBID leaves the computer, never Plex ids.
    assert!(r.http.urls().iter().all(|u| !u.contains(ARTIST)));
}

#[tokio::test]
async fn name_search_accepts_a_single_music_artist() {
    let r = rig(None, None);
    wiki_routes(&r.http);
    r.http.route(
        "wbsearchentities",
        200,
        &json!({"search": [
            {"id": "Q1", "label": "The Band"},
            {"id": "Q9", "label": "The Band discography"}
        ]})
        .to_string(),
    );
    r.svc.refresh(ARTIST).await.unwrap();
    let (info, _) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert_eq!(info.bio_source, Some(BioSource::Wikipedia));
    assert_eq!(r.http.calls("haswbstatement"), 0, "no MBID from Plex");
    // Wikidata's MBID is kept for the keyed sources.
    let row = r.db.read(|c| cache::get(c, ARTIST, "wikipedia")).unwrap().unwrap();
    assert_eq!(row.parse::<WikiData>().unwrap().mbid.as_deref(), Some(MBID));
}

#[tokio::test]
async fn ambiguous_or_non_music_name_matches_show_nothing() {
    let r = rig(Some("Plex bio."), None);
    r.http.route(
        "wbsearchentities",
        200,
        &json!({"search": [{"id": "Q1", "label": "The Band"}, {"id": "Q2", "label": "the band"}]}).to_string(),
    );
    // Both are bands: refuse to guess.
    r.http.route(
        "props=claims",
        200,
        &json!({"entities": {"Q1": band_entity(MBID), "Q2": band_entity("99999999-2222-3333-4444-555555555555")}}).to_string(),
    );
    r.svc.refresh(ARTIST).await.unwrap();
    let (info, _) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert_eq!(info.bio_source, Some(BioSource::Plex));
    assert!(info.credits.is_empty() && info.origin.is_none());
    assert_eq!(r.http.calls("en.wikipedia.org"), 0);
    // The miss is cached, so the next open doesn't ask again.
    let row = r.db.read(|c| cache::get(c, ARTIST, "wikipedia")).unwrap().unwrap();
    assert_eq!(row.data, None);

    // A single hit that isn't a musician (a film, say) is refused too.
    let r = rig(None, None);
    r.http.route("wbsearchentities", 200, &json!({"search": [{"id": "Q5", "label": "The Band"}]}).to_string());
    r.http.route(
        "props=claims",
        200,
        &json!({"entities": {"Q5": {"claims": {"P31": [{"mainsnak": {"datavalue": {"value": {"id": "Q11424"}}}}]}}}}).to_string(),
    );
    r.svc.refresh(ARTIST).await.unwrap();
    assert_eq!(r.svc.cached(ARTIST, now_s()).await.unwrap().0.bio, None);
}

#[tokio::test]
async fn fresh_cache_is_reused_and_stale_entries_show_then_refresh() {
    let r = rig(None, Some(MBID));
    wiki_routes(&r.http);
    r.svc.refresh(ARTIST).await.unwrap();
    let calls = r.http.urls().len();
    // Fresh: nothing to fetch.
    assert!(!r.svc.refresh(ARTIST).await.unwrap());
    assert_eq!(r.http.urls().len(), calls);

    // 31 days on: the old data still shows, marked stale.
    let later = now_s() + cache::TTL_S + 24 * 3600;
    let (info, stale) = r.svc.cached(ARTIST, later).await.unwrap();
    assert!(stale);
    assert_eq!(info.bio.as_deref(), Some(LONG_BIO));

    // A background refresh replaces it and tells the page.
    r.db.write(|c| {
        c.execute("UPDATE artist_info SET expires_at = 0", [])?;
        Ok(())
    })
    .unwrap();
    r.http.route("en.wikipedia.org", 200, &json!({"query": {"pages": [{"extract": "New lead section, longer than nothing."}]}}).to_string());
    r.events.0.lock().clear();
    let _ = r.svc.info(ARTIST).await.unwrap();
    for _ in 0..100 {
        if !r.events.0.lock().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(*r.events.0.lock(), [ARTIST]);
    let (info, stale) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert!(!stale);
    assert_eq!(info.bio.as_deref(), Some("New lead section, longer than nothing."));
}

#[tokio::test]
async fn a_down_source_falls_back_silently_and_is_tried_again() {
    let r = rig(Some("Plex bio."), Some(MBID));
    r.http.fail("wikidata.org");
    assert!(!r.svc.refresh(ARTIST).await.unwrap());
    let (info, stale) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert_eq!((info.bio.as_deref(), info.bio_source), (Some("Plex bio."), Some(BioSource::Plex)));
    assert!(stale, "nothing cached, so the next page open tries again");
    // 503: backs off, still silent.
    r.http.route("wikidata.org", 503, "");
    assert!(!r.svc.refresh(ARTIST).await.unwrap());
    assert!(r.events.0.lock().is_empty());
}

#[tokio::test]
async fn plex_offline_uses_the_cached_mbid() {
    let r = rig(None, Some(MBID));
    wiki_routes(&r.http);
    r.svc.refresh(ARTIST).await.unwrap();
    r.db.write(|c| {
        c.execute("UPDATE artist_info SET expires_at = 0", [])?;
        Ok(())
    })
    .unwrap();
    r.svc.set_mbid_lookup(Arc::new(PlexDown));
    let before = r.http.calls("haswbstatement");
    r.svc.refresh(ARTIST).await.unwrap();
    assert_eq!(r.http.calls("haswbstatement"), before + 1, "still looked up by the cached MBID");
}

fn lastfm_reply() -> String {
    json!({"artist": {"url": "https://www.last.fm/music/The+Band", "stats": {"listeners": "4321", "playcount": "98765"},
        "tags": {"tag": [{"name": "indie"}]}, "bio": {"content": "Short."}}})
    .to_string()
}

#[tokio::test]
async fn keyed_sources_need_a_key_and_a_switch() {
    let r = rig(None, Some(MBID));
    wiki_routes(&r.http);
    r.http.route("audioscrobbler", 200, &lastfm_reply());
    // Switch on, no key: not used.
    r.svc.switches().set(InfoSource::Lastfm, true);
    r.svc.refresh(ARTIST).await.unwrap();
    assert_eq!(r.http.calls("audioscrobbler"), 0);

    let st = r.svc.set_key(InfoSource::Lastfm, Some(" good-key ".into())).await.unwrap();
    assert!(st.saved && !st.rejected);
    // Stored in the keychain, trimmed.
    assert_eq!(r.keys.get("lastfm-api-key").unwrap().unwrap().expose(), "good-key");
    r.svc.refresh(ARTIST).await.unwrap();
    let (info, _) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert_eq!((info.listeners, info.scrobbles), (Some(4321), Some(98765)));
    assert_eq!(info.tags, ["indie"]);
    assert!(info.credits.iter().any(|c| c.text == "Data from Last.fm" && c.url.as_deref() == Some("https://www.last.fm/music/The+Band")));
    // Looked up by MBID, with the key, and nothing from Plex.
    let url = r.http.urls().into_iter().rfind(|u| u.contains("audioscrobbler")).unwrap();
    assert!(url.contains(MBID) && url.contains("api_key=good-key"));
}

#[tokio::test]
async fn a_rejected_key_turns_the_source_off_and_hides_its_data() {
    let r = rig(None, Some(MBID));
    r.http.route("audioscrobbler", 200, &lastfm_reply());
    r.svc.switches().set(InfoSource::Lastfm, true);
    r.svc.switches().set(InfoSource::Wikipedia, false);
    r.svc.set_key(InfoSource::Lastfm, Some("k".into())).await.unwrap();
    r.svc.refresh(ARTIST).await.unwrap();
    assert_eq!(r.svc.cached(ARTIST, now_s()).await.unwrap().0.listeners, Some(4321));

    // The key gets revoked: Last.fm answers error 10.
    r.db.write(|c| {
        c.execute("UPDATE artist_info SET expires_at = 0", [])?;
        Ok(())
    })
    .unwrap();
    r.http.route("audioscrobbler", 403, r#"{"error":10,"message":"Invalid API key"}"#);
    r.svc.refresh(ARTIST).await.unwrap();
    assert!(r.svc.keys_status().lastfm.rejected);
    assert!(!r.svc.usable(InfoSource::Lastfm));
    let (info, _) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert_eq!(info.listeners, None, "cached data from a refused key is hidden");
    assert!(info.credits.is_empty());

    // Entering a key checks it at once: still refused.
    let st = r.svc.set_key(InfoSource::Lastfm, Some("still-bad".into())).await.unwrap();
    assert!(st.saved && st.rejected);
    // A good key clears the error.
    r.http.route("audioscrobbler", 200, &lastfm_reply());
    let st = r.svc.set_key(InfoSource::Lastfm, Some("good".into())).await.unwrap();
    assert!(st.saved && !st.rejected);
    // Removing the key.
    let st = r.svc.set_key(InfoSource::Lastfm, None).await.unwrap();
    assert!(!st.saved);
    assert_eq!(r.keys.get("lastfm-api-key").unwrap(), None);
}

#[tokio::test]
async fn fanart_and_audiodb_keys_go_in_headers_and_401_rejects() {
    let r = rig(None, Some(MBID));
    r.http.route("webservice.fanart.tv", 401, r#"{"status":"error","error message":"Invalid API key"}"#);
    let st = r.svc.set_key(InfoSource::Fanart, Some("fk".into())).await.unwrap();
    assert!(st.rejected);
    r.http.route("theaudiodb.com/api", 200, r#"{"lookup":[]}"#);
    let st = r.svc.set_key(InfoSource::Audiodb, Some("ak".into())).await.unwrap();
    assert!(!st.rejected);
    let log = r.http.log.lock();
    for (url, headers) in log.iter() {
        assert!(!url.contains("fk") && !url.contains("/ak/"), "key in a URL: {url}");
        if url.contains("fanart") {
            assert_eq!(headers, &[("api-key", "fk".to_string())]);
        }
        if url.contains("theaudiodb") {
            assert_eq!(headers, &[("X-API-KEY", "ak".to_string())]);
        }
    }
}

#[tokio::test]
async fn each_source_off_hides_its_data_and_credits() {
    let r = rig(None, Some(MBID));
    let now = now_s();
    let wiki = WikiData {
        qid: "Q1".into(),
        bio: Some(LONG_BIO.into()),
        article_url: Some("https://en.wikipedia.org/wiki/The_Band".into()),
        origin: Some("Town".into()),
        ..Default::default()
    };
    let lastfm = LastfmData { listeners: Some(10), url: Some("https://www.last.fm/music/x".into()), ..Default::default() };
    let fanart = FanartData { mbid: MBID.into(), background: Some("https://assets.fanart.tv/bg.jpg".into()), ..Default::default() };
    let db = AudioDbData { bio: Some("x".repeat(500)), url: Some("https://www.theaudiodb.com/artist/1".into()), ..Default::default() };
    r.db.write(move |c| {
        cache::put(c, ARTIST, "wikipedia", Some(&wiki), now, cache::TTL_S)?;
        cache::put(c, ARTIST, "lastfm", Some(&lastfm), now, cache::TTL_S)?;
        cache::put(c, ARTIST, "fanart", Some(&fanart), now, cache::TTL_S)?;
        cache::put(c, ARTIST, "audiodb", Some(&db), now, cache::TTL_S)?;
        Ok(())
    })
    .unwrap();
    for (s, k) in [(InfoSource::Lastfm, "lk"), (InfoSource::Fanart, "fk"), (InfoSource::Audiodb, "ak")] {
        r.keys.set(s.key_account(), &crate::plex::Token::new(k).unwrap()).unwrap();
        r.svc.switches().set(s, true);
    }
    let credits = |info: &ArtistInfo| info.credits.iter().map(|c| c.source.clone()).collect::<Vec<_>>();

    let (all, _) = r.svc.cached(ARTIST, now).await.unwrap();
    assert_eq!(credits(&all), ["wikipedia", "wikidata", "lastfm", "fanart"]);
    assert!(all.banner.is_some());

    r.svc.switches().set(InfoSource::Wikipedia, false);
    let (info, _) = r.svc.cached(ARTIST, now).await.unwrap();
    assert_eq!(info.origin, None);
    assert_eq!(info.bio_source, Some(BioSource::Audiodb), "Last.fm has no bio here, so TheAudioDB's");
    assert_eq!(credits(&info), ["lastfm", "fanart", "audiodb"]);

    r.svc.switches().set(InfoSource::Lastfm, false);
    r.svc.switches().set(InfoSource::Fanart, false);
    let (info, _) = r.svc.cached(ARTIST, now).await.unwrap();
    assert_eq!((info.listeners, info.banner.as_deref()), (None, None));
    assert_eq!(credits(&info), ["audiodb"]);

    r.svc.switches().set(InfoSource::Audiodb, false);
    let (info, _) = r.svc.cached(ARTIST, now).await.unwrap();
    assert!(info.credits.is_empty() && info.bio.is_none());
}

#[tokio::test]
async fn clearing_the_cache_forgets_every_answer() {
    let r = rig(None, Some(MBID));
    wiki_routes(&r.http);
    r.svc.refresh(ARTIST).await.unwrap();
    assert_eq!(r.svc.clear_cache().await.unwrap(), 1);
    let (_, stale) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert!(stale);
}

#[tokio::test]
async fn unknown_artist_is_not_found() {
    let r = rig(None, None);
    assert!(matches!(r.svc.info("nope").await, Err(AppError::NotFound(_))));
}
