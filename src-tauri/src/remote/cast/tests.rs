//! Cast links against the fake device on localhost.

use std::time::Duration;

use super::fake::FakeCast;
use super::link::{CastApp, CastLink, CastPlan, DirectItem, PlexLoad};
use super::proto::{NS_MEDIA, NS_PLEX, NS_RECEIVER};
use super::{CastDevice, PLEX_APP};
use crate::audio::queue::Repeat;
use crate::remote::lan::LanLink;
use crate::remote::types::{PlayerKind, RemoteCommand};

fn device(fake: &FakeCast) -> CastDevice {
    CastDevice {
        id: "cast:fake".into(),
        name: "Fake".into(),
        model: "Chromecast".into(),
        addr: fake.addr,
        kind: PlayerKind::Tv,
        status: None,
    }
}

fn plan(plex: bool) -> CastPlan {
    let item = |k: &str| DirectItem {
        rating_key: k.into(),
        url: format!("http://192.0.2.1:32400/library/parts/{k}/file.flac?X-Plex-Token=t"),
        content_type: "audio/flac".into(),
        title: format!("Song {k}"),
        artist: "Band".into(),
        album: "Record".into(),
        image: None,
        duration_ms: 200_000,
    };
    CastPlan {
        plex: plex.then(|| PlexLoad {
            rating_key: "1".into(),
            offset_ms: 30_000,
            machine_id: "srv".into(),
            protocol: "http".into(),
            address: "192.0.2.1".into(),
            port: 32400,
            token: "transient-token".into(),
            play_queue_id: "77".into(),
        }),
        items: vec![item("1"), item("2"), item("3")],
        start: 0,
        offset_ms: 30_000,
        repeat: Repeat::Off,
    }
}

async fn start(fake: &FakeCast, p: &CastPlan) -> crate::error::AppResult<CastLink> {
    CastLink::start(&device(fake), p, super::conn::tls_config().unwrap(), Duration::from_secs(2)).await
}

#[tokio::test]
async fn plex_receiver_plays_the_server_play_queue() {
    let fake = FakeCast::start().await;
    let link = start(&fake, &plan(true)).await.unwrap();
    assert_eq!(link.app, CastApp::Plex);
    let load = fake.state.lock().plex_load.clone().unwrap();
    assert_eq!(load["media"]["customData"]["containerKey"], "/playQueues/77?own=1&window=200");
    assert_eq!(load["media"]["customData"]["server"]["accessToken"], "transient-token");
    assert_eq!(load["currentTime"], 30.0);
    assert_eq!(fake.types(NS_RECEIVER), vec!["LAUNCH"]);
    assert_eq!(fake.log().iter().find(|(_, v)| v["type"] == "LAUNCH").unwrap().1["appId"], PLEX_APP);

    let t = link.poll().await.unwrap().unwrap();
    assert_eq!((t.state.as_str(), t.time_ms, t.rating_key.as_deref()), ("playing", 30_000, Some("1")));
    assert_eq!(t.volume, Some(40));

    link.command(&RemoteCommand::Pause).await.unwrap();
    link.command(&RemoteCommand::Next).await.unwrap();
    link.command(&RemoteCommand::Seek { position_ms: 5_000 }).await.unwrap();
    link.command(&RemoteCommand::Volume { volume: 0.7 }).await.unwrap();
    let t = link.poll().await.unwrap().unwrap();
    assert_eq!(fake.types(NS_PLEX), vec!["PAUSE", "NEXT", "SEEK"]);
    assert_eq!((t.state.as_str(), t.time_ms), ("paused", 5_000));
    assert_eq!(t.rating_key.as_deref(), Some("2"));
    assert_eq!(t.volume, Some(70));

    link.leave(true).await;
    assert_eq!(fake.types(NS_RECEIVER).last().map(String::as_str), Some("STOP"));
    assert!(fake.state.lock().app.is_none());
}

#[tokio::test]
async fn without_plex_receiver_the_default_one_gets_the_queue() {
    let fake = FakeCast::start().await;
    fake.state.lock().no_plex_app = true;
    let link = start(&fake, &plan(true)).await.unwrap();
    assert_eq!(link.app, CastApp::Default);
    assert_eq!(fake.types(NS_MEDIA), vec!["QUEUE_LOAD"]);
    let t = link.poll().await.unwrap().unwrap();
    assert_eq!(t.rating_key.as_deref(), Some("1"));
    link.command(&RemoteCommand::Next).await.unwrap();
    link.command(&RemoteCommand::Repeat { mode: Repeat::All }).await.unwrap();
    let t = link.poll().await.unwrap().unwrap();
    assert_eq!(t.rating_key.as_deref(), Some("2"));
    assert_eq!(t.repeat, 2);
    let updates: Vec<_> = fake.log().into_iter().filter(|(_, v)| v["type"] == "QUEUE_UPDATE").collect();
    assert_eq!(updates[0].1["jump"], 1);
    assert!(updates[0].1["mediaSessionId"].is_i64());
}

#[tokio::test]
async fn a_refused_load_is_an_error() {
    let fake = FakeCast::start().await;
    fake.state.lock().refuse_load = true;
    assert!(start(&fake, &plan(true)).await.is_err());
    // Nothing to fall back to.
    let fake = FakeCast::start().await;
    fake.state.lock().no_plex_app = true;
    let mut p = plan(true);
    p.items.clear();
    assert!(start(&fake, &p).await.is_err());
}

#[tokio::test]
async fn stopped_elsewhere_and_gone_are_told_apart() {
    let fake = FakeCast::start().await;
    let link = start(&fake, &plan(false)).await.unwrap();
    assert!(link.poll().await.unwrap().is_some());
    fake.stop_elsewhere();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(link.poll().await.unwrap().is_none(), "another app or sender took over");

    let fake = FakeCast::start().await;
    let link = start(&fake, &plan(false)).await.unwrap();
    fake.vanish();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(link.poll().await.is_err(), "the device went away");
}

#[tokio::test]
async fn nobody_listening_fails_fast() {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    let dev = CastDevice {
        id: "cast:x".into(),
        name: "X".into(),
        model: "Chromecast".into(),
        addr,
        kind: PlayerKind::Tv,
        status: None,
    };
    let r = CastLink::start(&dev, &plan(true), super::conn::tls_config().unwrap(), Duration::from_secs(1)).await;
    assert!(r.is_err());
}

#[test]
fn mdns_answers_become_devices() {
    use crate::remote::lan::{CAST_SERVICE, answer};
    let tv = super::from_mdns(&answer(CAST_SERVICE, "x", "192.0.2.5", 8009, &[("id", "u1"), ("fn", "Den"), ("md", "Chromecast"), ("ca", "201221")])).unwrap();
    assert_eq!((tv.kind, tv.addr.port()), (PlayerKind::Tv, 8009));
    let spk = super::from_mdns(&answer(CAST_SERVICE, "x", "192.0.2.6", 8009, &[("id", "u2"), ("fn", "Kitchen"), ("md", "Google Home"), ("ca", "199172"), ("rs", "Radio")])).unwrap();
    assert_eq!(spk.kind, PlayerKind::Speaker);
    assert_eq!(spk.player().playing.as_deref(), Some("Radio"));
    let g = super::from_mdns(&answer(CAST_SERVICE, "x", "192.0.2.6", 32123, &[("id", "g"), ("fn", "Everywhere"), ("md", "Google Cast Group")])).unwrap();
    assert_eq!((g.kind, g.player().product.as_str()), (PlayerKind::Group, "Speaker group"));
    // No model capabilities: words in the model decide.
    let a = super::from_mdns(&answer(CAST_SERVICE, "x", "192.0.2.7", 8009, &[("id", "u3"), ("md", "Nest Audio")])).unwrap();
    assert_eq!((a.kind, a.name.as_str()), (PlayerKind::Speaker, "Nest Audio"));
    assert!(super::from_mdns(&answer(CAST_SERVICE, "x", "192.0.2.7", 8009, &[("fn", "no id")])).is_none());
}
