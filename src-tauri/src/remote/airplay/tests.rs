//! AirPlay against the fake receiver on localhost.

use std::net::UdpSocket;
use std::time::Duration;

use super::alac;
use super::fake::{FakeOptions, FakeRaop};
use super::raop::{RaopConfig, RaopStream};
use super::rtsp::{NEEDS_PAIRING, NEEDS_PASSWORD, WRONG_PASSWORD};
use super::{AirPlayDevice, AirPlayTarget, UNSUPPORTED_ENCRYPTION, UNSUPPORTED_PIN, from_mdns};
use crate::audio::netsink::NetStream;
use crate::error::AppError;
use crate::remote::lan::{RAOP_SERVICE, answer};

fn cfg(fake: &FakeRaop, password: Option<&str>) -> RaopConfig {
    RaopConfig {
        addr: fake.addr,
        password: password.map(str::to_string),
        timeout: Duration::from_secs(2),
        client_id: "0123456789ABCDEF".into(),
        volume: 0.5,
    }
}

fn wait_for(f: impl Fn() -> bool) {
    for _ in 0..100 {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out");
}

#[test]
fn a_session_sets_up_streams_flushes_and_tears_down() {
    let fake = FakeRaop::start(FakeOptions::default());
    let (mut stream, shared) = RaopStream::open(&cfg(&fake, None)).unwrap();
    assert_eq!(fake.methods(), vec!["OPTIONS", "ANNOUNCE", "SETUP", "RECORD", "SET_PARAMETER"]);
    let announce = fake.last("ANNOUNCE").unwrap();
    assert!(announce.body.contains("a=rtpmap:96 AppleLossless"));
    assert!(announce.body.contains("a=fmtp:96 352 0 16 40 10 14 2 255 0 0 44100"));
    assert!(fake.last("RECORD").unwrap().header("rtp-info").unwrap().starts_with("seq="));
    assert_eq!(fake.last("SET_PARAMETER").unwrap().body, "volume: -15.000000\r\n");
    // 2 s of our own delay plus the receiver's quarter second.
    assert_eq!(shared.delay(), Duration::from_millis(2250));

    let pcm: Vec<i16> = (0..alac::FRAMES * 2).map(|i| i as i16).collect();
    for _ in 0..3 {
        stream.send(&pcm).unwrap();
    }
    wait_for(|| fake.rec.lock().audio.len() == 3);
    {
        let rec = fake.rec.lock();
        assert_eq!(rec.audio[0][1], 0xe0, "the first packet is marked");
        assert_eq!(rec.audio[1][1], 0x60);
        let seq = |p: &Vec<u8>| u16::from_be_bytes([p[2], p[3]]);
        assert_eq!(seq(&rec.audio[1]), seq(&rec.audio[0]).wrapping_add(1));
        let ts = |p: &Vec<u8>| u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
        assert_eq!(ts(&rec.audio[1]), ts(&rec.audio[0]).wrapping_add(352));
        assert_eq!(alac::decode_stereo(&rec.audio[0][12..]).unwrap(), pcm);
    }
    wait_for(|| !fake.rec.lock().syncs.is_empty());
    assert_eq!(fake.rec.lock().syncs[0][..2], [0x90, 0xd4]);

    // The receiver asks for the time and gets an answer.
    let port = fake.rec.lock().client_timing.unwrap();
    let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
    probe.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut req = [0u8; 32];
    req[0] = 0x80;
    req[1] = 0xd2;
    req[24..32].copy_from_slice(&[9; 8]);
    probe.send_to(&req, ("127.0.0.1", port)).unwrap();
    let mut buf = [0u8; 64];
    let n = probe.recv(&mut buf).unwrap();
    assert_eq!((n, buf[1]), (32, 0xd3));
    assert_eq!(buf[8..16], [9; 8], "our answer carries their send time");

    shared.flush().unwrap();
    assert!(fake.last("FLUSH").unwrap().header("rtp-info").unwrap().contains("rtptime="));
    stream.send(&pcm).unwrap();
    wait_for(|| fake.rec.lock().audio.len() == 4);
    assert_eq!(fake.rec.lock().audio[3][1], 0xe0, "a flush starts a new run");

    shared.set_volume(0.0).unwrap();
    assert_eq!(fake.last("SET_PARAMETER").unwrap().body, "volume: -144.000000\r\n");
    stream.close();
    assert_eq!(fake.methods().last().map(String::as_str), Some("TEARDOWN"));
    assert!(stream.send(&pcm).is_err(), "a closed session takes no more audio");
}

#[test]
fn passwords_are_asked_for_checked_and_used() {
    let fake = FakeRaop::start(FakeOptions { password: Some("hunter2".into()), ..Default::default() });
    let err = |pw: Option<&str>| match RaopStream::open(&cfg(&fake, pw)) {
        Err(AppError::Auth(m)) => m,
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("opened"),
    };
    assert_eq!(err(None), NEEDS_PASSWORD);
    assert_eq!(err(Some("wrong")), WRONG_PASSWORD);
    let (mut s, _) = RaopStream::open(&cfg(&fake, Some("hunter2"))).unwrap();
    assert!(fake.last("RECORD").unwrap().header("authorization").unwrap().starts_with("Digest username=\"iTunes\""));
    s.close();
}

#[test]
fn receivers_that_need_pairing_say_so() {
    let fake = FakeRaop::start(FakeOptions { pairing: true, ..Default::default() });
    match RaopStream::open(&cfg(&fake, None)) {
        Err(AppError::Auth(m)) => assert_eq!(m, NEEDS_PAIRING),
        _ => panic!("expected the pairing error"),
    }
}

#[test]
fn a_gone_receiver_fails_to_open() {
    let fake = FakeRaop::start(FakeOptions::default());
    let addr = fake.addr;
    drop(fake);
    std::thread::sleep(Duration::from_millis(50));
    let mut c = RaopConfig {
        addr,
        password: None,
        timeout: Duration::from_millis(500),
        client_id: "0".into(),
        volume: 1.0,
    };
    c.timeout = Duration::from_millis(300);
    assert!(RaopStream::open(&c).is_err());
}

#[test]
fn mdns_answers_become_receivers() {
    let ok = from_mdns(&answer(RAOP_SERVICE, "AABBCCDDEEFF@Kitchen", "192.0.2.40", 7000, &[("et", "0,3,5"), ("cn", "0,1,2,3"), ("am", "AudioAccessory5,1")])).unwrap();
    assert_eq!((ok.id.as_str(), ok.name.as_str(), ok.password, ok.unsupported.as_deref()), ("airplay:AABBCCDDEEFF", "Kitchen", false, None));
    assert_eq!(ok.output_id(), "net:airplay:AABBCCDDEEFF");
    let rsa = from_mdns(&answer(RAOP_SERVICE, "001122334455@Old", "192.0.2.41", 5000, &[("et", "1"), ("pw", "false")])).unwrap();
    assert_eq!(rsa.unsupported.as_deref(), Some(UNSUPPORTED_ENCRYPTION));
    assert_eq!(rsa.player().unsupported.as_deref(), Some(UNSUPPORTED_ENCRYPTION));
    let pin = from_mdns(&answer(RAOP_SERVICE, "001122334466@Pin", "192.0.2.42", 7000, &[("et", "0"), ("sf", "0x8")])).unwrap();
    assert_eq!(pin.unsupported.as_deref(), Some(UNSUPPORTED_PIN));
    let tv = from_mdns(&answer(RAOP_SERVICE, "001122334477@Den", "192.0.2.43", 7000, &[("am", "AppleTV6,2"), ("pw", "true")])).unwrap();
    assert_eq!(tv.player().kind, crate::remote::types::PlayerKind::Tv);
    assert!(tv.password);
}

#[test]
fn the_target_probes_and_opens_through_the_network_output() {
    use crate::audio::netsink::NetTarget;
    let fake = FakeRaop::start(FakeOptions::default());
    let dev = AirPlayDevice {
        id: "airplay:AA".into(),
        name: "Kitchen".into(),
        model: None,
        addr: fake.addr,
        password: false,
        unsupported: None,
        computer: false,
    };
    let t = AirPlayTarget::new(dev, None, "0123456789ABCDEF".into(), Duration::from_secs(2), 0.8);
    t.probe().unwrap();
    assert_eq!(fake.methods(), vec!["OPTIONS"]);
    assert_eq!(t.device().id, "net:airplay:AA");
    assert!(t.session().is_none());
    let mut s = t.connect().unwrap();
    assert!(t.session().is_some());
    assert_eq!(fake.last("SET_PARAMETER").unwrap().body, "volume: -6.000000\r\n");
    t.set_volume(1.0).unwrap();
    assert_eq!(fake.last("SET_PARAMETER").unwrap().body, "volume: 0.000000\r\n");
    s.close();
    assert!(t.session().is_none());
}
