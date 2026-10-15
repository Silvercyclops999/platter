use std::net::IpAddr;
use std::time::Duration;

use url::Url;

use super::fake::FakeRenderer;
use super::link::{DlnaItem, REFUSED, didl, format_time, parse_time};
use super::soap::Soap;
use super::*;
use crate::audio::queue::Repeat;
use crate::remote::lan::LanLink;
use crate::remote::types::RemoteCommand;

fn soap() -> Soap {
    Soap::new(reqwest::Client::new(), Duration::from_secs(2))
}

async fn device(fake: &FakeRenderer) -> DlnaDevice {
    describe(&reqwest::Client::new(), &soap(), &fake.location()).await.expect("described")
}

fn item(key: &str, original: Option<&str>) -> DlnaItem {
    DlnaItem {
        rating_key: key.into(),
        original: original.map(|m| (format!("http://server.test/library/parts/{key}/file?X-Plex-Token=t"), m.to_string())),
        transcode: Some(format!("http://server.test/music/:/transcode/universal/start.mp3?path=%2Flibrary%2Fmetadata%2F{key}&X-Plex-Token=t")),
        title: format!("Song {key}"),
        artist: "Rue & Ellis".into(),
        album: "Late".into(),
        image: None,
        duration_ms: 200_000,
    }
}

#[test]
fn search_answers_point_only_at_the_device_that_answered() {
    let me: IpAddr = "127.0.0.1".parse().unwrap();
    let ans = "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=1800\r\nLocation: http://127.0.0.1:49152/desc.xml\r\nST: urn:schemas-upnp-org:device:MediaRenderer:1\r\n\r\n";
    assert_eq!(answer_location(ans, me).unwrap().as_str(), "http://127.0.0.1:49152/desc.xml");
    let elsewhere = ans.replace("127.0.0.1:49152", "198.51.100.7:80");
    assert_eq!(answer_location(&elsewhere, me), None, "a device can't send Platter to another host");
    assert_eq!(answer_location(&ans.replace("http://", "https://"), me), None);
    assert_eq!(answer_location("NOTIFY * HTTP/1.1\r\nLOCATION: http://127.0.0.1/x\r\n\r\n", me), None);
    assert!(search_request().contains("ST: urn:schemas-upnp-org:device:MediaRenderer:1"));
}

#[test]
fn descriptions_and_formats_are_read() {
    let loc = Url::parse("http://127.0.0.1:1400/xml/device.xml").unwrap();
    let xml = r#"<root><URLBase>http://127.0.0.1:1400/</URLBase><device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
        <friendlyName>Kitchen &amp; Den</friendlyName><modelName>Smart TV</modelName><UDN>uuid:abc</UDN><serviceList>
        <service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType><controlURL>MediaRenderer/AVTransport/Control</controlURL></service>
        <service><serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType><controlURL>/rc</controlURL></service>
        </serviceList></device></root>"#;
    let d = parse_description(xml, &loc).unwrap();
    assert_eq!((d.id.as_str(), d.name.as_str()), ("dlna:abc", "Kitchen & Den"));
    assert_eq!(d.av_transport.control.as_str(), "http://127.0.0.1:1400/MediaRenderer/AVTransport/Control");
    assert!(d.connection.is_none() && d.rendering.is_some());
    assert_eq!(d.player().kind, crate::remote::types::PlayerKind::Tv);
    assert_eq!(d.player().via, crate::remote::types::PlayerVia::Dlna);
    // A media server (not a renderer) isn't listed.
    assert!(parse_description(&xml.replace("MediaRenderer:1", "MediaServer:1"), &loc).is_none());
    // Control URLs on another host are refused.
    assert!(parse_description(&xml.replace("MediaRenderer/AVTransport/Control", "http://198.51.100.7/c"), &loc).is_none());

    assert_eq!(sink_types("http-get:*:audio/mpeg:*,http-get:*:AUDIO/X-FLAC:DLNA.ORG_PN=X,rtsp:*:audio/l16:*,http-get:*:*:*"), ["audio/mpeg", "audio/x-flac"]);
    let mut d = d;
    d.sinks = sink_types("http-get:*:audio/x-flac:*");
    assert!(d.takes("audio/flac") && !d.takes("audio/mpeg"));
    d.sinks.clear();
    assert!(d.takes("audio/mpeg") && !d.takes("audio/flac"), "a renderer that didn't say gets MP3");
}

#[test]
fn times_and_metadata() {
    assert_eq!(parse_time("0:03:21"), Some(201_000));
    assert_eq!(parse_time("1:00:00.500"), Some(3_600_500));
    assert_eq!(parse_time("NOT_IMPLEMENTED"), None);
    assert_eq!(format_time(201_000), "0:03:21");
    let d = didl(&item("7", None), "http://x/a?b=1&c=2", "audio/mpeg");
    assert!(d.contains("<dc:title>Song 7</dc:title>") && d.contains("Rue &amp; Ellis") && d.contains("b=1&amp;c=2"));
    assert!(d.contains("protocolInfo=\"http-get:*:audio/mpeg:*\"") && d.contains("duration=\"0:03:20.000\""));
}

#[tokio::test]
async fn a_renderer_is_described_with_what_it_plays() {
    let fake = FakeRenderer::start().await;
    let d = device(&fake).await;
    assert_eq!((d.id.as_str(), d.name.as_str(), d.model.as_deref()), ("dlna:1234-abcd", "Den Receiver", Some("AV-100")));
    assert_eq!(d.sinks, ["audio/mpeg", "audio/x-flac"]);
    let found = Ssdp::fixed(reqwest::Client::new(), vec![fake.location(), fake.location()]).renderers().await;
    assert_eq!(found.len(), 1, "one entry per device");
}

#[tokio::test]
async fn plays_the_original_seeks_to_the_position_and_takes_controls() {
    let fake = FakeRenderer::start().await;
    let d = device(&fake).await;
    let link = DlnaLink::start(d, vec![item("1", Some("audio/flac")), item("2", None)], 0, 61_000, Repeat::Off, soap()).await.unwrap();
    {
        let s = fake.state.lock();
        assert!(s.uri.contains("/library/parts/1/"), "the original file");
        assert!(s.meta.contains(":audio/flac:"));
        assert_eq!(s.seeks, ["0:01:01"]);
    }
    fake.at(62_000);
    let t = link.poll().await.unwrap().unwrap();
    assert_eq!((t.state.as_str(), t.time_ms, t.rating_key.as_deref(), t.volume), ("playing", 62_000, Some("1"), Some(30)));
    link.command(&RemoteCommand::Pause).await.unwrap();
    assert_eq!(link.poll().await.unwrap().unwrap().state, "paused");
    link.command(&RemoteCommand::Volume { volume: 0.55 }).await.unwrap();
    assert_eq!(fake.state.lock().volume, 55);
    link.command(&RemoteCommand::Seek { position_ms: 90_000 }).await.unwrap();
    link.command(&RemoteCommand::Next).await.unwrap();
    assert!(fake.state.lock().uri.contains("transcode"), "song 2 has only the transcode");
    assert_eq!(link.index(), 1);
    assert!(link.command(&RemoteCommand::Next).await.is_err(), "the end of the queue");
    assert!(link.command(&RemoteCommand::Shuffle { on: true }).await.is_err());
    link.leave(true).await;
    assert_eq!(fake.actions().last().map(String::as_str), Some("Stop"));
}

#[tokio::test]
async fn the_next_song_follows_when_one_ends_and_the_queue_end_stops() {
    let fake = FakeRenderer::start().await;
    let d = device(&fake).await;
    let link = DlnaLink::start(d, vec![item("1", None), item("2", None)], 0, 0, Repeat::Off, soap()).await.unwrap();
    fake.at(150_000);
    assert_eq!(link.poll().await.unwrap().unwrap().state, "playing");
    fake.at(199_000);
    link.poll().await.unwrap();
    fake.finish_song();
    let t = link.poll().await.unwrap().unwrap();
    assert_eq!((t.state.as_str(), t.rating_key.as_deref()), ("buffering", Some("2")));
    assert!(fake.state.lock().meta.contains("Song 2"));
    // Repeat one plays song 2 again when it ends.
    link.command(&RemoteCommand::Repeat { mode: Repeat::One }).await.unwrap();
    fake.at(199_000);
    link.poll().await.unwrap();
    fake.finish_song();
    assert_eq!(link.poll().await.unwrap().unwrap().rating_key.as_deref(), Some("2"));
    // Off at the last song: it stops, and the service takes playback back.
    link.command(&RemoteCommand::Repeat { mode: Repeat::Off }).await.unwrap();
    fake.at(199_000);
    link.poll().await.unwrap();
    fake.finish_song();
    assert_eq!(link.poll().await.unwrap().unwrap().state, "stopped");
}

#[tokio::test]
async fn stopped_midway_or_taken_over_is_handed_back() {
    let fake = FakeRenderer::start().await;
    let d = device(&fake).await;
    let link = DlnaLink::start(d, vec![item("1", None), item("2", None)], 0, 0, Repeat::Off, soap()).await.unwrap();
    fake.at(30_000);
    link.poll().await.unwrap();
    fake.state.lock().transport = "STOPPED".into();
    assert_eq!(link.poll().await.unwrap().unwrap().state, "stopped", "stopped with the remote: not the next song");
    // Another app put its own stream on.
    {
        let mut s = fake.state.lock();
        s.uri = "http://198.51.100.9/other.mp3".into();
        s.transport = "PLAYING".into();
    }
    assert_eq!(link.poll().await.unwrap(), None);
}

#[tokio::test]
async fn a_refused_format_falls_back_to_the_transcode() {
    let fake = FakeRenderer::start().await;
    fake.state.lock().refuse = vec!["audio/flac".into()];
    let d = device(&fake).await;
    DlnaLink::start(d, vec![item("1", Some("audio/flac"))], 0, 0, Repeat::Off, soap()).await.unwrap();
    let s = fake.state.lock();
    assert!(s.uri.contains("transcode"));
    assert_eq!(s.actions.iter().filter(|a| *a == "SetAVTransportURI").count(), 2);
}

#[tokio::test]
async fn a_renderer_that_refuses_everything_says_so() {
    let fake = FakeRenderer::start().await;
    fake.state.lock().refuse = vec!["audio/flac".into(), "audio/mpeg".into()];
    let d = device(&fake).await;
    let e = DlnaLink::start(d, vec![item("1", Some("audio/flac"))], 0, 0, Repeat::Off, soap()).await.unwrap_err();
    assert_eq!(e.to_string(), format!("invalid: {REFUSED}"));
    assert!(!fake.state.lock().actions.contains(&"Play".to_string()));
}

#[tokio::test]
async fn a_renderer_that_goes_away_fails_its_polls() {
    let fake = FakeRenderer::start().await;
    let d = device(&fake).await;
    let link = DlnaLink::start(d, vec![item("1", None)], 0, 0, Repeat::Off, soap()).await.unwrap();
    fake.vanish();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(matches!(link.poll().await, Err(crate::error::AppError::Network(_))));
}

#[test]
fn the_plan_offers_the_original_only_in_formats_the_renderer_takes() {
    use crate::remote::service::ServerCtx;
    let server = ServerCtx { base: Url::parse("http://127.0.0.1:32400").unwrap(), token: Token::new("server-token").unwrap(), machine_id: "m1".into() };
    let token = Token::new("transient").unwrap();
    let mut flac = crate::audio::testutil::track("10", "Rue", "/library/parts/10/file.flac");
    flac.codec = Some("flac".into());
    flac.container = Some("flac".into());
    let mut alac = crate::audio::testutil::track("11", "Rue", "/library/parts/11/file.m4a");
    alac.codec = Some("alac".into());
    let mut dev = parse_description(
        "<root><device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType><UDN>uuid:x</UDN><serviceList><service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType><controlURL>/c</controlURL></service></serviceList></device></root>",
        &Url::parse("http://127.0.0.1:1/d.xml").unwrap(),
    )
    .unwrap();
    dev.sinks = vec!["audio/x-flac".into(), "audio/mpeg".into()];
    let items = plan(&server, &token, "me", &dev, &[flac.clone(), alac]);
    let (url, mime) = items[0].original.clone().unwrap();
    assert!(url.contains("/library/parts/10/file.flac") && url.contains("X-Plex-Token=transient"));
    assert_eq!(mime, "audio/flac");
    assert!(items[1].original.is_none(), "Apple Lossless goes through the transcoder");
    assert!(items[1].transcode.as_deref().unwrap().contains("start.mp3"));
    dev.sinks = vec!["audio/mpeg".into()];
    assert!(plan(&server, &token, "me", &dev, &[flac])[0].original.is_none(), "no FLAC on this renderer");
}
