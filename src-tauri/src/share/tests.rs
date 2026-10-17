use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use parking_lot::Mutex;

use super::link::{self, AppLink, ShareKind};
use super::matching::*;
use super::*;
use crate::artistinfo::http::fake::FakeHttp;
use crate::error::AppError;

// ---------------------------------------------------------------- matching

fn song(artist: &str, title: &str, album: &str, ms: i64) -> SongQuery {
    SongQuery { artist: artist.into(), album_artist: artist.into(), title: title.into(), album: album.into(), duration_ms: ms }
}

fn hit(id: u64, artist: &str, title: &str, album: &str, ms: i64) -> SongHit {
    SongHit { id, artist: artist.into(), title: title.into(), album: album.into(), duration_ms: Some(ms) }
}

#[test]
fn folds_case_accents_punctuation_and_edition_notes() {
    assert_eq!(title_key("Bohemian Rhapsody - Remastered 2011"), "bohemian rhapsody");
    assert_eq!(title_key("Café del Mar (Deluxe Edition)"), "cafe del mar");
    assert_eq!(title_key("Song (feat. Someone Else)"), "song");
    assert_eq!(title_key("Song [ft. X]"), "song");
    assert_eq!(title_key("Song feat. X"), "song");
    assert_eq!(title_key("Rock & Roll"), "rock and roll");
    assert_eq!(title_key("Song - Single"), "song");
    // Different recordings keep their notes.
    assert_eq!(title_key("Song (Live)"), "song live");
    assert_eq!(title_key("Song - Acoustic"), "song acoustic");
    // The exact key keeps edition notes.
    assert_ne!(exact_key("Album (Deluxe Edition)"), exact_key("Album"));
    assert_eq!(exact_key("Album (feat. X)"), exact_key("album"));
}

#[test]
fn artist_credits_match_on_the_same_names() {
    assert!(artists_match("Beyoncé", "BEYONCE"));
    assert!(artists_match("Simon & Garfunkel", "Simon and Garfunkel"));
    assert!(artists_match("Queen & David Bowie", "Queen, David Bowie"));
    // A featured artist missing on one side is fine when the lead is the same.
    assert!(artists_match("Artist feat. Guest", "Artist"));
    assert!(!artists_match("Guest", "Artist feat. Guest"), "the lead artist must be the same");
    assert!(!artists_match("The Band", "Band of Horses"));
}

#[test]
fn accepts_a_close_song_match() {
    let q = song("Queen", "Bohemian Rhapsody", "A Night at the Opera", 355_000);
    let hits = [
        hit(1, "Queen", "Bohemian Rhapsody", "Greatest Hits", 355_100),
        hit(2, "Queen", "Bohemian Rhapsody (Remastered 2011)", "A Night At The Opera (Deluxe Edition)", 356_500),
        hit(3, "Queen", "Bohemian Rhapsody", "A Night At The Opera", 357_900),
    ];
    // Same album wins over another album; the same edition over a deluxe one.
    assert_eq!(best_song(&q, &hits), Some(3));
    assert_eq!(best_song(&q, &hits[..2]), Some(2));
}

#[test]
fn rejects_near_misses() {
    let q = song("Queen", "Bohemian Rhapsody", "A Night at the Opera", 355_000);
    let cases = [
        hit(1, "Queen", "Bohemian Rhapsody", "A Night at the Opera", 358_500), // 3.5 s off
        hit(2, "Queen", "Bohemian Rhapsody (Live)", "A Night at the Opera", 355_000), // live take
        hit(3, "Panic! At the Disco", "Bohemian Rhapsody", "A Night at the Opera", 355_000), // cover
        hit(4, "Queen", "Bohemian Rhapsody", "Greatest Hits", 355_000), // other album
        hit(5, "Queen", "Bohemian Rhap", "A Night at the Opera", 355_000), // other title
    ];
    for h in &cases {
        assert!(!song_matches(&q, h), "{h:?} should not match");
    }
    assert_eq!(best_song(&q, &cases), None);
    // Without a length in the library there's nothing to check: no match.
    let no_len = song("Queen", "Bohemian Rhapsody", "A Night at the Opera", 0);
    assert!(!song_matches(&no_len, &hit(1, "Queen", "Bohemian Rhapsody", "A Night at the Opera", 1)));
    // Nor without one in the store.
    let mut h = hit(1, "Queen", "Bohemian Rhapsody", "A Night at the Opera", 355_000);
    h.duration_ms = None;
    assert!(!song_matches(&q, &h));
}

#[test]
fn a_song_credited_to_its_album_artist_matches() {
    let q = SongQuery {
        artist: "Queen & David Bowie".into(),
        album_artist: "Queen".into(),
        title: "Under Pressure".into(),
        album: "Hot Space".into(),
        duration_ms: 248_000,
    };
    assert!(song_matches(&q, &hit(1, "Queen", "Under Pressure", "Hot Space", 247_000)));
    assert!(song_matches(&q, &hit(1, "Queen & David Bowie", "Under Pressure", "Hot Space", 247_000)));
}

#[test]
fn albums_match_on_artist_and_title_and_prefer_the_same_edition() {
    let q = AlbumQuery { artist: "Queen".into(), title: "A Night at the Opera".into(), track_count: 12 };
    let hits = [
        AlbumHit { id: 1, artist: "Queen".into(), title: "A Night At The Opera (Deluxe Edition)".into(), track_count: Some(24) },
        AlbumHit { id: 2, artist: "Queen".into(), title: "A Night At The Opera".into(), track_count: Some(12) },
        AlbumHit { id: 3, artist: "Other".into(), title: "A Night At The Opera".into(), track_count: Some(12) },
    ];
    assert_eq!(best_album(&q, &hits), Some(2));
    assert_eq!(best_album(&q, &hits[..1]), Some(1));
    assert_eq!(best_album(&q, &hits[2..]), None);
}

#[test]
fn search_terms_carry_only_artist_and_title() {
    assert_eq!(search_term("Artist feat. Guest", "Song (feat. Guest) - Remastered"), "artist song");
}

// ---------------------------------------------------------------- store replies

const SONG_REPLY: &str = r#"{"resultCount":3,"results":[
 {"wrapperType":"track","kind":"song","artistId":7,"collectionId":50,"trackId":501,"artistName":"Test Artist","collectionName":"First Light","trackName":"Opening","trackTimeMillis":201000,"trackCount":10},
 {"wrapperType":"track","kind":"music-video","trackId":502,"artistName":"Test Artist","trackName":"Opening","collectionName":"First Light"},
 {"wrapperType":"track","kind":"song","collectionId":60,"trackId":601,"artistName":"Test Artist","collectionName":"Hits","trackName":"Opening","trackTimeMillis":201000}
]}"#;

const ALBUM_REPLY: &str = r#"{"resultCount":2,"results":[
 {"wrapperType":"collection","collectionType":"Album","collectionId":50,"artistName":"Test Artist","collectionName":"First Light","trackCount":10},
 {"wrapperType":"collection","collectionType":"Album","collectionId":51,"artistName":"Test Artist","collectionName":"First Light (Deluxe Edition)","trackCount":14}
]}"#;

#[test]
fn reads_store_replies_and_skips_other_kinds() {
    let songs = itunes::parse_songs(SONG_REPLY.as_bytes()).unwrap();
    assert_eq!(songs.iter().map(|s| s.id).collect::<Vec<_>>(), vec![501, 601]);
    assert_eq!(songs[0].duration_ms, Some(201_000));
    let albums = itunes::parse_albums(ALBUM_REPLY.as_bytes()).unwrap();
    assert_eq!(albums.len(), 2);
    assert!(itunes::parse_songs(b"not json").is_err());
    assert_eq!(itunes::song_page(501), "https://song.link/i/501");
    assert_eq!(itunes::album_page(50), "https://album.link/i/50");
}

#[test]
fn search_url_sends_only_the_words() {
    let u = itunes::search_url("test artist opening", itunes::Entity::Song);
    assert!(u.starts_with("https://itunes.apple.com/search?"));
    let parsed = url::Url::parse(&u).unwrap();
    let pairs: Vec<(String, String)> = parsed.query_pairs().map(|(k, v)| (k.into(), v.into())).collect();
    assert_eq!(
        pairs,
        vec![
            ("term".into(), "test artist opening".into()),
            ("media".into(), "music".into()),
            ("entity".into(), "song".into()),
            ("limit".into(), "25".into()),
        ]
    );
}

// ---------------------------------------------------------------- links

#[test]
fn builds_links_with_time_and_server_tag() {
    assert_eq!(link::build(ShareKind::Album, "50", None, None), "platter://album/50");
    assert_eq!(link::build(ShareKind::Album, "50", Some(83), None), "platter://album/50", "only songs keep a time");
    let tag = link::server_tag("machine-a");
    assert_eq!(tag.len(), 8);
    assert_eq!(link::build(ShareKind::Track, "501", Some(83), Some("machine-a")), format!("platter://song/501?t=83&s={tag}"));
    assert_eq!(link::build(ShareKind::Playlist, "p9", None, Some("")), "platter://playlist/p9");
    assert_ne!(link::server_tag("machine-a"), link::server_tag("machine-b"));
}

#[test]
fn parses_links_and_rejects_bad_ones() {
    let l = |kind, id: &str, t, server: Option<&str>| AppLink { kind, id: id.into(), t, server: server.map(Into::into) };
    assert_eq!(link::parse("platter://song/501?t=83&s=0a1b2c3d"), Some(l(ShareKind::Track, "501", Some(83), Some("0a1b2c3d"))));
    assert_eq!(link::parse("platter://track/501"), Some(l(ShareKind::Track, "501", None, None)));
    assert_eq!(link::parse("PLATTER://Album/50/"), Some(l(ShareKind::Album, "50", None, None)));
    assert_eq!(link::parse("platter:///artist/7"), Some(l(ShareKind::Artist, "7", None, None)));
    assert_eq!(link::parse("platter:playlist/p9"), Some(l(ShareKind::Playlist, "p9", None, None)));
    // Times: seconds, m:ss, h:mm:ss; ignored on anything but songs.
    assert_eq!(link::parse("platter://song/1?t=1:23").unwrap().t, Some(83));
    assert_eq!(link::parse("platter://song/1?t=1:00:05").unwrap().t, Some(3605));
    assert_eq!(link::parse("platter://song/1?t=90s").unwrap().t, Some(90));
    assert_eq!(link::parse("platter://album/1?t=90").unwrap().t, None);
    // A malformed tag is ignored rather than trusted.
    assert_eq!(link::parse("platter://album/1?s=zz").unwrap().server, None);
    for bad in [
        "https://song.link/i/1",
        "platter://song",
        "platter://song/1/extra",
        "platter://video/1",
        "platter://song/1?t=-5",
        "platter://song/1?t=abc",
        "platter://song/1?t=99999999",
        "platter://song/..%2Fx",
        "platter://song/a%20b",
        "not a link",
    ] {
        assert_eq!(link::parse(bad), None, "{bad}");
    }
    // Round trip.
    let built = link::build(ShareKind::Track, "501", Some(83), Some("m"));
    assert_eq!(link::parse(&built), Some(l(ShareKind::Track, "501", Some(83), Some(&link::server_tag("m")))));
}

// ---------------------------------------------------------------- service

struct FixedServer(Option<String>);

#[async_trait::async_trait]
impl ServerId for FixedServer {
    async fn machine_id(&self) -> Option<String> {
        self.0.clone()
    }
}

#[derive(Default)]
struct Events(Mutex<u32>);

impl ShareEvents for Events {
    fn link_waiting(&self) {
        *self.0.lock() += 1;
    }
}

struct Fx {
    share: ShareService,
    http: Arc<FakeHttp>,
    enabled: Arc<AtomicBool>,
    db: DbHandle,
    _dir: tempfile::TempDir,
}

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("s.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            r#"
            INSERT INTO artists (id, title, sort_title) VALUES ('7', 'Test Artist', 'Test Artist');
            INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, track_count)
              VALUES ('50', 'First Light', 'First Light', '7', 'Test Artist', 'album', '[]', 10),
                     ('80', 'Basement Tapes', 'Basement Tapes', '7', 'Test Artist', 'album', '["demo"]', 4);
            INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key)
              VALUES ('501', 'Opening', '50', 'First Light', '7', 'Test Artist', 200500, '/p/1'),
                     ('801', 'Opening (Demo)', '80', 'Basement Tapes', '7', 'Test Artist', 190000, '/p/2');
            INSERT INTO playlists (id, title) VALUES ('p9', 'Road Trip');
            "#,
        )?;
        Ok(())
    })
    .unwrap();
    let http = Arc::new(FakeHttp::new());
    let enabled = Arc::new(AtomicBool::new(true));
    let share = ShareService::with_gap(
        db.clone(),
        http.clone(),
        Arc::new(FixedServer(Some("machine-a".into()))),
        enabled.clone(),
        Duration::from_millis(1),
    );
    Fx { share, http, enabled, db, _dir: dir }
}

#[tokio::test]
async fn finds_a_song_link_and_caches_it() {
    let f = fx();
    f.http.route("itunes.apple.com/search", 200, SONG_REPLY);
    let url = f.share.public_link(ShareKind::Track, "501").await.unwrap();
    assert_eq!(url, "https://song.link/i/501");
    // Only the artist and title were sent.
    let sent = f.http.urls();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("term=test+artist+opening"), "{}", sent[0]);
    assert!(!sent[0].contains("501") && !sent[0].contains("machine"));
    // A second ask is answered from the cache.
    assert_eq!(f.share.public_link(ShareKind::Track, "501").await.unwrap(), url);
    assert_eq!(f.http.calls("itunes"), 1);
}

#[tokio::test]
async fn finds_an_album_link() {
    let f = fx();
    f.http.route("entity=album", 200, ALBUM_REPLY);
    assert_eq!(f.share.public_link(ShareKind::Album, "50").await.unwrap(), "https://album.link/i/50");
}

#[tokio::test]
async fn no_confident_match_is_not_found_and_remembered() {
    let f = fx();
    f.http.route("itunes", 200, r#"{"resultCount":1,"results":[{"wrapperType":"track","kind":"song","trackId":9,"artistName":"Someone Else","trackName":"Opening","collectionName":"First Light","trackTimeMillis":200500}]}"#);
    let e = f.share.public_link(ShareKind::Track, "501").await.unwrap_err();
    assert!(matches!(&e, AppError::NotFound(m) if m == "Couldn't find this song online"), "{e:?}");
    let _ = f.share.public_link(ShareKind::Track, "501").await;
    assert_eq!(f.http.calls("itunes"), 1, "a miss is cached too");
    // Expired rows are asked again.
    f.db.write(|c| Ok(c.execute("UPDATE share_links SET expires_at = 0", [])?)).unwrap();
    f.http.route("itunes", 200, SONG_REPLY);
    assert_eq!(f.share.public_link(ShareKind::Track, "501").await.unwrap(), "https://song.link/i/501");
}

#[tokio::test]
async fn unreleased_music_never_gets_a_public_link() {
    let f = fx();
    f.http.route("itunes", 200, SONG_REPLY);
    for (kind, id) in [(ShareKind::Track, "801"), (ShareKind::Album, "80")] {
        let e = f.share.public_link(kind, id).await.unwrap_err();
        assert!(matches!(&e, AppError::Invalid(m) if m == MSG_UNRELEASED), "{e:?}");
        assert_eq!(f.share.info(kind, id, None).await.unwrap().public, PublicLink::Unreleased);
    }
    assert_eq!(f.http.calls("itunes"), 0, "nothing about unreleased music leaves the computer");
}

#[tokio::test]
async fn the_setting_turns_public_links_off() {
    let f = fx();
    f.enabled.store(false, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(f.share.info(ShareKind::Track, "501", None).await.unwrap().public, PublicLink::Off);
    assert!(matches!(f.share.public_link(ShareKind::Track, "501").await, Err(AppError::Invalid(_))));
    assert_eq!(f.http.calls("itunes"), 0);
    // Unreleased stays unreleased either way.
    assert_eq!(f.share.info(ShareKind::Album, "80", None).await.unwrap().public, PublicLink::Unreleased);
}

#[tokio::test]
async fn store_trouble_is_an_error_not_a_cached_miss() {
    let f = fx();
    f.http.fail("itunes");
    assert!(matches!(f.share.public_link(ShareKind::Track, "501").await, Err(AppError::Network(_))));
    f.http.route("itunes", 403, "");
    assert!(matches!(f.share.public_link(ShareKind::Track, "501").await, Err(AppError::Server(_))));
    f.http.route("itunes", 200, SONG_REPLY);
    assert!(f.share.public_link(ShareKind::Track, "501").await.is_ok(), "nothing was cached by the failures");
}

#[tokio::test]
async fn card_qr_codes_hold_only_the_platter_link() {
    let f = fx();
    let tag = link::server_tag("machine-a");
    let q = f.share.card_qr(ShareKind::Track, "501", Some(61)).await.unwrap();
    assert_eq!(q.text, format!("platter://song/501?t=61&s={tag}"));
    // No server details: not its id, not an address, not a web link.
    assert!(!q.text.contains("machine") && !q.text.contains("http") && !q.text.contains("plex"));
    assert_eq!(q.rows.len(), q.size);
    assert!(q.rows.iter().all(|r| r.len() == q.size && r.chars().all(|c| c == '0' || c == '1')));
    // The same QR as the link would make on its own.
    assert_eq!(crate::sync::keycopy::qr_text(&q.text).unwrap().rows, q.rows);
    assert!(matches!(f.share.card_qr(ShareKind::Album, "nope", None).await, Err(AppError::NotFound(_))));
}

#[tokio::test]
async fn info_gives_app_links_for_every_kind() {
    let f = fx();
    let tag = link::server_tag("machine-a");
    let i = f.share.info(ShareKind::Track, "501", Some(83)).await.unwrap();
    assert_eq!(i.app_link, format!("platter://song/501?t=83&s={tag}"));
    assert_eq!(i.public, PublicLink::Available);
    assert_eq!(f.share.info(ShareKind::Artist, "7", None).await.unwrap().public, PublicLink::None);
    let p = f.share.info(ShareKind::Playlist, "p9", None).await.unwrap();
    assert_eq!((p.app_link, p.public), (format!("platter://playlist/p9?s={tag}"), PublicLink::None));
    assert!(matches!(f.share.info(ShareKind::Album, "nope", None).await, Err(AppError::NotFound(_))));
}

#[tokio::test]
async fn opening_links_resolves_against_the_library() {
    let f = fx();
    let tag = link::server_tag("machine-a");
    let o = f.share.resolve(&format!("platter://song/501?t=83&s={tag}")).await;
    assert_eq!(
        o,
        DeepLinkOpen {
            status: DeepLinkStatus::Open,
            kind: Some(ShareKind::Track),
            id: Some("501".into()),
            album_id: Some("50".into()),
            at_seconds: Some(83),
            join: None,
            circle: None,
            share_ask: None,
            relay: None,
        }
    );
    // A listening session invite is handed over to join.
    let invite = crate::social::listen::invite::Token { session: [1; 16], secret: [2; 16], expires: u32::MAX, relay: None }.link();
    let j = f.share.resolve(&invite).await;
    assert_eq!((j.status, j.kind, j.join.as_deref()), (DeepLinkStatus::Open, None, Some(invite.as_str())));
    // So are shared playlist and Blend invites.
    let t = crate::social::listen::invite::Token { session: [1; 16], secret: [2; 16], expires: u32::MAX, relay: None };
    for word in ["collab", "blend"] {
        let l = t.link_as(word);
        let o = f.share.resolve(&l).await;
        assert_eq!((o.status, o.join.as_deref(), o.circle.as_deref()), (DeepLinkStatus::Open, None, Some(l.as_str())));
    }
    // A share link is handed over to ask for that person's music.
    let ask = crate::social::listen::invite::Token { session: [1; 16], secret: [2; 16], expires: u32::MAX, relay: None }.link_as("share");
    let a = f.share.resolve(&ask).await;
    assert_eq!((a.status, a.join.as_deref(), a.share_ask.as_deref()), (DeepLinkStatus::Open, None, Some(ask.as_str())));
    let a = f.share.resolve("platter://artist/7").await;
    assert_eq!((a.status, a.album_id, a.at_seconds), (DeepLinkStatus::Open, None, None));
    assert_eq!(f.share.resolve("platter://album/999").await.status, DeepLinkStatus::NotInLibrary);
    let other = link::server_tag("machine-b");
    assert_eq!(
        f.share.resolve(&format!("platter://album/50?s={other}")).await.status,
        DeepLinkStatus::NotInLibrary,
        "the same number on another server is a different album"
    );
    assert_eq!(f.share.resolve("platter://nothing").await.status, DeepLinkStatus::Invalid);
}

#[tokio::test]
async fn an_opened_link_waits_for_the_ui_once() {
    let f = fx();
    let ev = Arc::new(Events::default());
    f.share.set_events(ev.clone());
    f.share.open("platter://playlist/p9").await;
    assert_eq!(*ev.0.lock(), 1);
    let t = f.share.take().unwrap();
    assert_eq!((t.status, t.kind, t.id.as_deref()), (DeepLinkStatus::Open, Some(ShareKind::Playlist), Some("p9")));
    assert_eq!(f.share.take(), None);
    // The newest link wins when two arrive before the UI looks.
    f.share.open("platter://album/50").await;
    f.share.open("platter://album/999").await;
    assert_eq!(f.share.take().unwrap().status, DeepLinkStatus::NotInLibrary);
}

#[tokio::test]
async fn links_name_the_server_that_has_the_item() {
    let f = fx();
    let here = link::server_tag("machine-a");
    let other = link::server_tag("machine-b");
    f.db.write(|c| {
        c.execute_batch(&format!(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, format, subformats, track_count)
               VALUES ('{here}:60', 'Here', 'Here', '7', 'Test Artist', 'album', '[]', 1),
                      ('{other}:60', 'There', 'There', '7', 'Test Artist', 'album', '[]', 1);"
        ))?;
        Ok(())
    })
    .unwrap();
    // The link carries the server's own id and the tag of the server with it.
    let i = f.share.info(ShareKind::Album, &format!("{other}:60"), None).await.unwrap();
    assert_eq!(i.app_link, format!("platter://album/60?s={other}"));
    let i = f.share.info(ShareKind::Album, &format!("{here}:60"), None).await.unwrap();
    assert_eq!(i.app_link, format!("platter://album/60?s={here}"));
    // Opening it finds that server's album, not the same number elsewhere.
    let o = f.share.resolve(&format!("platter://album/60?s={other}")).await;
    assert_eq!((o.status, o.id.clone()), (DeepLinkStatus::Open, Some(format!("{other}:60"))));
    let o = f.share.resolve(&format!("platter://album/60?s={here}")).await;
    assert_eq!(o.id, Some(format!("{here}:60")));
    // A link from before tags means the connected server.
    let o = f.share.resolve("platter://album/60").await;
    assert_eq!(o.id, Some(format!("{here}:60")));
    // A server whose music isn't here.
    let gone = link::server_tag("machine-c");
    assert_eq!(f.share.resolve(&format!("platter://album/60?s={gone}")).await.status, DeepLinkStatus::NotInLibrary);
}
