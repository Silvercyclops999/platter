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
struct Events(Mutex<Vec<String>>, Mutex<Vec<String>>);
impl InfoEvents for Events {
    fn info_changed(&self, id: &str) {
        self.0.lock().push(id.to_owned());
    }

    fn popular_done(&self, id: &str) {
        self.1.lock().push(id.to_owned());
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
    // The MusicBrainz details and ListenBrainz stats have their own tests
    // (`rig_mb`); here they stay off so the request counts are the older ones.
    let switches = Switches::default();
    switches.set(InfoSource::Musicbrainz, false);
    switches.set(InfoSource::ListenbrainzStats, false);
    let svc = ArtistInfoService::new(db.clone(), http.clone(), keys.clone(), switches);
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
    assert_eq!(info.members[0], Member { name: "Singer".into(), artist_id: Some("77".into()), years: None });
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
    let url = r.http.urls().into_iter().rfind(|u| u.contains("audioscrobbler") && u.contains("artist.getinfo")).unwrap();
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

// ---------------------------------------------------------------- popular

fn lb_reply(songs: &[(&str, u64)]) -> String {
    let items: Vec<_> = songs
        .iter()
        .map(|(t, n)| json!({"recording_name": t, "total_listen_count": n, "artists": [{"artist_credit_name": "The Band"}]}))
        .collect();
    serde_json::Value::Array(items).to_string()
}

fn popular_row(db: &DbHandle, id: &str) -> Option<cache::Row> {
    let id = id.to_owned();
    db.read(move |c| cache::get(c, &id, crate::db::popular::GLOBAL_ROW)).unwrap()
}

#[tokio::test]
async fn popular_list_comes_from_listenbrainz_and_is_cached_for_a_week() {
    let r = rig(None, Some(MBID));
    r.svc.switches().set(InfoSource::Wikipedia, false);
    r.http.route("top-recordings-for-artist", 200, &lb_reply(&[("Hit", 900), ("Hit - Remastered", 100), ("B Side", 50)]));
    assert!(r.svc.refresh(ARTIST).await.unwrap());
    assert_eq!(*r.events.0.lock(), [ARTIST], "the page hears about the new list");
    let url = r.http.urls().into_iter().find(|u| u.contains("listenbrainz")).unwrap();
    assert!(url.ends_with(&format!("/1/popularity/top-recordings-for-artist/{MBID}")));
    let row = popular_row(&r.db, ARTIST).unwrap();
    let top: crate::db::popular::GlobalTop = row.parse().unwrap();
    assert_eq!(top.songs.iter().map(|s| (s.title.as_str(), s.listens)).collect::<Vec<_>>(), [("Hit", 1000), ("B Side", 50)]);
    assert_eq!(row.expires_at - row.fetched_at, service::POPULAR_TTL_S);
    // Within the week: not asked again.
    let calls = r.http.calls("listenbrainz");
    let (_, stale) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert!(!stale);
    r.svc.refresh(ARTIST).await.unwrap();
    assert_eq!(r.http.calls("listenbrainz"), calls);
    // A week on it's stale again.
    let (_, stale) = r.svc.cached(ARTIST, now_s() + service::POPULAR_TTL_S).await.unwrap();
    assert!(stale);
}

#[tokio::test]
async fn listenbrainz_offline_or_a_bad_answer_stores_nothing_and_retries_later() {
    for setup in ["offline", "malformed", "busy"] {
        let r = rig(None, Some(MBID));
        r.svc.switches().set(InfoSource::Wikipedia, false);
        match setup {
            "offline" => r.http.fail("listenbrainz"),
            "malformed" => r.http.route("listenbrainz", 200, r#"{"payload": "not a list"}"#),
            _ => r.http.route("listenbrainz", 503, ""),
        }
        assert!(!r.svc.refresh(ARTIST).await.unwrap(), "{setup}");
        assert!(popular_row(&r.db, ARTIST).is_none(), "{setup}: nothing cached");
        let (_, stale) = r.svc.cached(ARTIST, now_s()).await.unwrap();
        assert!(stale, "{setup}: the next page open tries again");
        assert!(r.events.0.lock().is_empty());
    }
}

#[tokio::test]
async fn without_a_token_or_a_list_last_fm_top_tracks_fill_in() {
    let r = rig(None, Some(MBID));
    r.svc.switches().set(InfoSource::Wikipedia, false);
    // ListenBrainz wants a token for this artist; none saved, no Last.fm: a miss.
    r.http.route("listenbrainz", 401, r#"{"code":401,"error":"token"}"#);
    assert!(!r.svc.refresh(ARTIST).await.unwrap());
    assert_eq!(popular_row(&r.db, ARTIST).unwrap().data, None);
    assert!(!r.svc.is_rejected(InfoSource::Listenbrainz));

    // With Last.fm keyed, its top tracks stand in.
    r.http.route("artist.getinfo", 200, &lastfm_reply());
    r.http.route(
        "artist.gettoptracks",
        200,
        &json!({"toptracks": {"track": [{"name": "Hit", "playcount": "70"}, {"name": "Other", "playcount": "9"}]}}).to_string(),
    );
    r.svc.switches().set(InfoSource::Lastfm, true);
    r.svc.set_key(InfoSource::Lastfm, Some("k".into())).await.unwrap();
    assert!(popular_row(&r.db, ARTIST).is_none(), "a new key asks again for artists with no list");
    assert!(r.svc.refresh(ARTIST).await.unwrap());
    let top: crate::db::popular::GlobalTop = popular_row(&r.db, ARTIST).unwrap().parse().unwrap();
    assert_eq!(top.from, "lastfm");
    assert_eq!(top.songs[0].title, "Hit");
    let url = r.http.urls().into_iter().find(|u| u.contains("gettoptracks")).unwrap();
    assert!(url.contains(MBID) && url.contains("limit=100"));
}

#[tokio::test]
async fn a_listenbrainz_token_is_checked_sent_and_refused_tokens_switch_it_off() {
    let r = rig(None, Some(MBID));
    r.svc.switches().set(InfoSource::Wikipedia, false);
    r.http.route("validate-token", 200, r#"{"code":200,"valid":true}"#);
    let st = r.svc.set_key(InfoSource::Listenbrainz, Some("tok".into())).await.unwrap();
    assert!(st.saved && !st.rejected);
    assert!(r.svc.keys_status().listenbrainz.saved);
    r.http.route("top-recordings-for-artist", 200, &lb_reply(&[("Hit", 1)]));
    r.svc.refresh(ARTIST).await.unwrap();
    let log = r.http.log.lock().clone();
    let (url, headers) = log.iter().find(|(u, _)| u.contains("top-recordings")).unwrap();
    assert!(!url.contains("tok"));
    assert_eq!(headers, &vec![("Authorization", "Token tok".to_string())]);

    // A refused token: off for the session, the row shows it.
    r.http.route("validate-token", 200, r#"{"code":200,"valid":false}"#);
    let st = r.svc.set_key(InfoSource::Listenbrainz, Some("bad".into())).await.unwrap();
    assert!(st.saved && st.rejected);
    assert!(!r.svc.usable(InfoSource::Listenbrainz));
    // No token at all is fine: ListenBrainz still works for cached answers.
    let st = r.svc.set_key(InfoSource::Listenbrainz, None).await.unwrap();
    assert!(!st.saved && !st.rejected);
    assert!(r.svc.usable(InfoSource::Listenbrainz));
    assert!(r.svc.set_key(InfoSource::Wikipedia, Some("x".into())).await.is_err());
}

#[tokio::test]
async fn my_plays_or_both_sources_off_fetch_no_popular_list() {
    let r = rig(None, Some(MBID));
    r.svc.switches().set(InfoSource::Wikipedia, false);
    r.db.set_popular_source(crate::db::PopularSource::MyPlays);
    r.svc.refresh(ARTIST).await.unwrap();
    assert_eq!(r.http.calls("listenbrainz"), 0);
    r.db.set_popular_source(crate::db::PopularSource::Global);
    r.svc.switches().set(InfoSource::Listenbrainz, false);
    r.svc.refresh(ARTIST).await.unwrap();
    assert_eq!(r.http.calls("listenbrainz"), 0);
}

/// Counts Plex lookups; credited artists must never cause one.
#[derive(Default)]
struct CountingMbid(std::sync::atomic::AtomicUsize);
#[async_trait]
impl MbidLookup for CountingMbid {
    async fn artist_mbid(&self, _: &str) -> AppResult<Option<String>> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(None)
    }
}

/// The band's track crediting a guest who has no library artist.
fn add_credited_track(db: &DbHandle) {
    db.write(|c| {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_artist, part_key)
             VALUES ('t1', 'Night Drive', 'al1', 'LP', ?1, 'The Band', 'The Band feat. Rue Ellis', '/p')",
            [ARTIST],
        )?;
        Ok(())
    })
    .unwrap();
}

const GUEST: &str = "99999999-2222-3333-4444-555555555555";

#[tokio::test]
async fn credited_artists_get_info_by_musicbrainz_search_never_plex() {
    let r = rig(None, Some(MBID));
    let plex = Arc::new(CountingMbid::default());
    r.svc.set_mbid_lookup(plex.clone());
    r.svc.switches().set(InfoSource::Wikipedia, false);
    add_credited_track(&r.db);
    r.http.route(
        "musicbrainz.org/ws/2/artist",
        200,
        &json!({"artists": [{"id": GUEST, "name": "Rue Ellis", "score": 100}]}).to_string(),
    );
    r.http.route("top-recordings-for-artist", 200, &lb_reply(&[("Night Drive", 5)]));
    let id = "credit:rue ellis";
    let (info, stale) = r.svc.cached(id, now_s()).await.unwrap();
    assert!(stale && info.bio.is_none());
    assert!(r.svc.refresh(id).await.unwrap());
    assert_eq!(plex.0.load(std::sync::atomic::Ordering::Relaxed), 0, "no Plex call for a credit id");
    let mb = r.http.urls().into_iter().find(|u| u.contains("musicbrainz")).unwrap();
    assert!(mb.contains("Rue+Ellis") || mb.contains("Rue%20Ellis"), "{mb}");
    assert!(r.http.urls().iter().any(|u| u.ends_with(GUEST)), "popular list by the found id");
    assert!(popular_row(&r.db, id).unwrap().data.is_some());
    assert_eq!(*r.events.0.lock(), [id]);

    // Not an exact match: no id, so no ListenBrainz request.
    let r = rig(None, Some(MBID));
    r.svc.switches().set(InfoSource::Wikipedia, false);
    add_credited_track(&r.db);
    r.http.route(
        "musicbrainz.org/ws/2/artist",
        200,
        &json!({"artists": [{"id": GUEST, "name": "Rue Ellis Trio", "score": 100}]}).to_string(),
    );
    r.svc.refresh(id).await.unwrap();
    assert_eq!(r.http.calls("listenbrainz"), 0);

    // A credit nobody has is not found.
    assert!(matches!(r.svc.info("credit:nobody").await, Err(AppError::NotFound(_))));
}

#[tokio::test]
async fn bio_names_in_the_library_link_and_wikidata_links_come_through() {
    let r = rig(None, Some(MBID));
    let now = now_s();
    let wiki = WikiData {
        qid: "Q1".into(),
        bio: Some("The Band toured with Singer for years. The Band then split.".into()),
        article_url: Some("https://en.wikipedia.org/wiki/The_Band".into()),
        links: vec![InfoLink { kind: "website".into(), label: "Official website".into(), url: "https://band.example/".into() }],
        ..Default::default()
    };
    let fanart = FanartData {
        mbid: MBID.into(),
        thumbs: vec!["https://assets.fanart.tv/1.jpg".into(), "https://assets.fanart.tv/2.jpg".into()],
        ..Default::default()
    };
    r.db.write(move |c| {
        cache::put(c, ARTIST, "wikipedia", Some(&wiki), now, cache::TTL_S)?;
        cache::put(c, ARTIST, "fanart", Some(&fanart), now, cache::TTL_S)?;
        Ok(())
    })
    .unwrap();
    r.keys.set(InfoSource::Fanart.key_account(), &crate::plex::Token::new("fk").unwrap()).unwrap();
    r.svc.switches().set(InfoSource::Fanart, true);
    let (info, _) = r.svc.cached(ARTIST, now).await.unwrap();
    // "The Band" is the artist itself: never linked.
    assert_eq!(info.bio_links, [BioLink { text: "Singer".into(), artist_id: "77".into() }]);
    assert_eq!(info.links.iter().map(|l| l.kind.as_str()).collect::<Vec<_>>(), ["website", "wikipedia"]);
    assert_eq!(info.gallery.len(), 3, "Plex's photo, then Fanart.tv's two");
    assert!(commands::listed_for(&info, "https://band.example/"));
}

// ---------------------------------------------------------------- prefetch

async fn until_idle(svc: &ArtistInfoService) {
    for _ in 0..500 {
        if svc.prefetch_idle() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("prefetch never finished");
}

#[tokio::test]
async fn a_finished_popular_fetch_tells_the_page_found_or_not() {
    let r = rig(None, Some(MBID));
    r.svc.switches().set(InfoSource::Wikipedia, false);
    r.http.fail("listenbrainz");
    assert!(!r.svc.refresh(ARTIST).await.unwrap());
    assert_eq!(*r.events.1.lock(), [ARTIST], "the page stops waiting on a failed fetch");
    assert!(r.events.0.lock().is_empty());

    let r = rig(None, Some(MBID));
    r.http.route("top-recordings-for-artist", 200, &lb_reply(&[("Hit", 9)]));
    wiki_routes(&r.http);
    assert!(r.svc.refresh(ARTIST).await.unwrap());
    // The list first, then the rest: the page reloads twice at most.
    assert_eq!(*r.events.1.lock(), [ARTIST]);
    assert_eq!(*r.events.0.lock(), [ARTIST, ARTIST]);
    let urls = r.http.urls();
    let lb = urls.iter().position(|u| u.contains("listenbrainz")).unwrap();
    let wiki = urls.iter().position(|u| u.contains("wiki")).unwrap();
    assert!(lb < wiki, "Popular is asked for before the bio");
}

#[tokio::test]
async fn prefetch_fills_missing_lists_politely_and_only_lists() {
    let r = rig(None, Some(MBID));
    wiki_routes(&r.http);
    r.http.route("top-recordings-for-artist", 200, &lb_reply(&[("Hit", 9)]));
    // '77' already has a fresh list; a third artist is new.
    let fresh = crate::db::popular::GlobalTop::from_pairs("listenbrainz", [("Old".to_string(), 1)]);
    r.db.write(move |c| {
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('78', 'Drummer', 'Drummer')", [])?;
        cache::put(c, "77", crate::db::popular::GLOBAL_ROW, Some(&fresh), now_s(), 3600)?;
        Ok(())
    })
    .unwrap();
    let gap = std::time::Duration::from_millis(150);
    r.svc.set_prefetch_gap(gap);
    let started = std::time::Instant::now();
    r.svc.queue_popular([ARTIST.to_string(), "77".to_string(), "78".to_string(), ARTIST.to_string()]);
    // Queued again while running: no second worker, no repeat.
    r.svc.queue_popular([ARTIST.to_string()]);
    until_idle(&r.svc).await;
    assert_eq!(r.http.calls("listenbrainz"), 2, "the fresh one is skipped, the repeat merged");
    assert!(started.elapsed() >= gap * 2, "a pause after each fetch");
    assert!(popular_row(&r.db, ARTIST).unwrap().data.is_some());
    assert!(popular_row(&r.db, "78").unwrap().data.is_some());
    // Only the Popular list: the bio waits for the page.
    assert!(!r.http.urls().iter().any(|u| u.contains("wiki")));
    let (_, stale) = r.svc.cached(ARTIST, now_s()).await.unwrap();
    assert!(stale, "the other sources still refresh on page open");
    assert_eq!(*r.events.1.lock(), [ARTIST, "78"]);

    // Again: everything fresh, nothing asked.
    r.svc.queue_popular([ARTIST.to_string(), "78".to_string()]);
    until_idle(&r.svc).await;
    assert_eq!(r.http.calls("listenbrainz"), 2);

    // Popular by my plays: the prefetch asks nothing.
    r.db.write(|c| {
        c.execute("DELETE FROM artist_info", [])?;
        Ok(())
    })
    .unwrap();
    r.db.set_popular_source(crate::db::PopularSource::MyPlays);
    r.svc.queue_popular([ARTIST.to_string()]);
    until_idle(&r.svc).await;
    assert_eq!(r.http.calls("listenbrainz"), 2);
}

#[tokio::test]
async fn prefetch_targets_home_and_most_played_artists() {
    let r = rig(None, Some(MBID));
    r.db.write(|c| {
        c.execute("UPDATE artists SET view_count = 5 WHERE id = '77'", [])?;
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('79', 'Never Played', 'Never Played')", [])?;
        Ok(())
    })
    .unwrap();
    let ids = r.db.read(|c| super::service::prefetch_targets(c, now_s())).unwrap();
    assert_eq!(ids, ["77"]);
}
