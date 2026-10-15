//! File tag tests: parsers on tiny made-up headers, Range sizing, the scan
//! behind playback, the cache, and what the tags change in the library.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rusqlite::Connection;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::fetch::{FULL_READ_MAX, FetchError, Reader, read_tags};
use super::parse::{FIRST_READ, MORE_READ, Sparse, Step, parse};
use super::scan::{PartSource, TagEvents, TagScanStatus, TagScanner};
use super::store::{self, Member, Skipped, Status};
use super::{FileTags, TagView};
use crate::audio::StreamSource;
use crate::audio::cache::PrecacheHost;
use crate::audio::gate::{Priority, TransferGate};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::model::{Album, ReleaseSection, Track};

// ------------------------------------------------------------------ fixtures

fn vorbis(entries: &[&str]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(4u32.to_le_bytes());
    b.extend(b"test");
    b.extend((entries.len() as u32).to_le_bytes());
    for e in entries {
        b.extend((e.len() as u32).to_le_bytes());
        b.extend(e.as_bytes());
    }
    b
}

fn flac(blocks: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut b = b"fLaC".to_vec();
    for (i, (kind, body)) in blocks.iter().enumerate() {
        let last = if i + 1 == blocks.len() { 0x80 } else { 0 };
        b.push(last | kind);
        b.extend(&(body.len() as u32).to_be_bytes()[1..]);
        b.extend(body);
    }
    b.extend(vec![0xff; 256]); // audio frames
    b
}

fn streaminfo() -> (u8, Vec<u8>) {
    (0, vec![0; 34])
}

fn synchsafe(n: u32) -> [u8; 4] {
    [(n >> 21) as u8 & 0x7f, (n >> 14) as u8 & 0x7f, (n >> 7) as u8 & 0x7f, n as u8 & 0x7f]
}

fn txxx(enc: u8, desc: &str, value: &str) -> Vec<u8> {
    let mut d = vec![enc];
    match enc {
        1 => {
            for s in [desc, value] {
                d.extend([0xff, 0xfe]);
                for u in s.encode_utf16() {
                    d.extend(u.to_le_bytes());
                }
                if s == desc {
                    d.extend([0, 0]);
                }
            }
        }
        _ => {
            d.extend(desc.as_bytes());
            d.push(0);
            d.extend(value.as_bytes());
        }
    }
    d
}

fn id3(ver: u8, frames: &[(&[u8], Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (id, data) in frames {
        body.extend(*id);
        match ver {
            2 => body.extend(&(data.len() as u32).to_be_bytes()[1..]),
            3 => body.extend((data.len() as u32).to_be_bytes()),
            _ => body.extend(synchsafe(data.len() as u32)),
        }
        if ver > 2 {
            body.extend([0, 0]);
        }
        body.extend(data);
    }
    body.extend(vec![0; 64]); // padding
    let mut b = b"ID3".to_vec();
    b.extend([ver, 0, 0]);
    b.extend(synchsafe(body.len() as u32));
    b.extend(body);
    b
}

fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    b.extend(kind);
    b.extend(body);
    b
}

fn data_atom(kind: u32, value: &[u8]) -> Vec<u8> {
    let mut b = kind.to_be_bytes().to_vec();
    b.extend([0, 0, 0, 0]);
    b.extend(value);
    atom(b"data", &b)
}

fn freeform(name: &str, value: &str) -> Vec<u8> {
    let mut body = atom(b"mean", &[&[0u8, 0, 0, 0][..], b"com.apple.iTunes"].concat());
    body.extend(atom(b"name", &[&[0u8, 0, 0, 0][..], name.as_bytes()].concat()));
    body.extend(data_atom(1, value.as_bytes()));
    atom(b"----", &body)
}

fn mp4(items: &[Vec<u8>], moov_at_end: bool, mdat_len: usize, full_meta: bool) -> Vec<u8> {
    let ilst = atom(b"ilst", &items.concat());
    let hdlr = atom(b"hdlr", &[0u8; 25]);
    let meta_body = if full_meta { [&[0u8, 0, 0, 0][..], &hdlr, &ilst].concat() } else { [hdlr, ilst].concat() };
    let meta = atom(b"meta", &meta_body);
    let udta = atom(b"udta", &meta);
    let trak = atom(b"trak", &vec![0u8; 3000]);
    let moov = atom(b"moov", &[atom(b"mvhd", &[0u8; 100]), trak, udta].concat());
    let ftyp = atom(b"ftyp", b"M4A \0\0\0\0M4A mp42isom");
    let mdat = atom(b"mdat", &vec![0x55; mdat_len]);
    if moov_at_end { [ftyp, mdat, moov].concat() } else { [ftyp, moov, mdat].concat() }
}

fn ogg_page(seq: u32, packet: &[u8]) -> Vec<u8> {
    let mut lacing = vec![255u8; packet.len() / 255];
    lacing.push((packet.len() % 255) as u8);
    let mut b = b"OggS".to_vec();
    b.extend([0, 0]);
    b.extend([0u8; 8]);
    b.extend(1u32.to_le_bytes());
    b.extend(seq.to_le_bytes());
    b.extend([0u8; 4]);
    b.push(lacing.len() as u8);
    b.extend(lacing);
    b.extend(packet);
    b
}

fn ogg(first: &[u8], comment: Vec<u8>) -> Vec<u8> {
    [ogg_page(0, first), ogg_page(1, &comment), ogg_page(2, &[0u8; 300])].concat()
}

/// Runs the parser like the scan does: the first 64 KB, then what it asks for.
fn run(file: &[u8], container: Option<&str>) -> (FileTags, usize) {
    let first = file[..file.len().min(FIRST_READ)].to_vec();
    let mut s = Sparse::new(first, Some(file.len() as u64));
    s.container = container.map(str::to_string);
    let mut reads = 1;
    loop {
        match parse(&s, false) {
            Step::Done(t) => return (t, reads),
            Step::Need { offset, len } => {
                assert!(reads < 8, "too many reads");
                let end = (offset as usize + len.max(MORE_READ)).min(file.len());
                s.add(offset, file[offset as usize..end].to_vec());
                reads += 1;
            }
        }
    }
}

fn archive_entries() -> Vec<&'static str> {
    vec![
        "TITLE=Made Up Song",
        "ARCHIVE_VERSION=Leak",
        "ARCHIVE_ERA=Paper Era",
        "ARCHIVE_QUALITY=CDQ",
        "ARCHIVE_DATE=2019-05-04",
        "ARCHIVE_EDITION=Build v2",
        "ARCHIVE_GROUP=test-group",
        "ARCHIVE_SOURCE=made-up source",
        "archive_flags=fanedit",
        "ITUNESADVISORY=1",
    ]
}

fn expected_archive() -> FileTags {
    FileTags {
        explicit: Some(true),
        version: Some("leak".into()),
        era: Some("Paper Era".into()),
        quality: Some("cdq".into()),
        date: Some("2019-05-04".into()),
        edition: Some("Build v2".into()),
        group: Some("test-group".into()),
        source: Some("made-up source".into()),
        flags: Some("fanedit".into()),
        ..Default::default()
    }
}

// ------------------------------------------------------------------ recording ids

const ISRC: &str = "USRC17607839";
const REC: &str = "4f3c2b1a-0d9e-4c8b-a7f6-5e4d3c2b1a09";

fn ufid_frame(owner: &str, id: &str) -> Vec<u8> {
    [owner.as_bytes(), &[0], id.as_bytes()].concat()
}

#[test]
fn flac_keeps_isrc_and_the_recording_id() {
    let f = flac(&[
        streaminfo(),
        (4, vorbis(&["TITLE=Made Up", "isrc=US-RC1-76-07839", &format!("MUSICBRAINZ_TRACKID={}", REC.to_uppercase())])),
    ]);
    let (t, _) = run(&f, Some("flac"));
    assert_eq!(t.isrc.as_deref(), Some(ISRC), "dashes dropped, upper case");
    assert_eq!(t.recording_mbid.as_deref(), Some(REC), "lower case");
}

#[test]
fn mp3_keeps_tsrc_and_the_musicbrainz_ufid() {
    for ver in [3u8, 4] {
        let f = [
            id3(
                ver,
                &[
                    (b"UFID", ufid_frame("http://example.com", "not-this")),
                    (b"TSRC", [&[0u8][..], ISRC.as_bytes()].concat()),
                    (b"UFID", ufid_frame("http://musicbrainz.org", REC)),
                    // The release track id is not the recording id.
                    (b"TXXX", txxx(0, "MusicBrainz Release Track Id", "11111111-2222-3333-4444-555555555555")),
                ],
            ),
            vec![0xff; 400],
        ]
        .concat();
        let (t, _) = run(&f, Some("mp3"));
        assert_eq!(t.isrc.as_deref(), Some(ISRC), "v2.{ver}");
        assert_eq!(t.recording_mbid.as_deref(), Some(REC), "v2.{ver}");
    }
    // ID3v2.2 names them TRC and UFI; UTF-16 text decodes too.
    let mut trc = vec![1u8, 0xff, 0xfe];
    for u in ISRC.encode_utf16() {
        trc.extend(u.to_le_bytes());
    }
    let f = id3(2, &[(b"TRC", trc), (b"UFI", ufid_frame("http://musicbrainz.org", REC))]);
    let t = run(&f, Some("mp3")).0;
    assert_eq!(t.isrc.as_deref(), Some(ISRC));
    assert_eq!(t.recording_mbid.as_deref(), Some(REC));
}

#[test]
fn m4a_keeps_the_freeform_isrc_and_recording_id() {
    let f = mp4(&[freeform("ISRC", ISRC), freeform("MusicBrainz Track Id", REC)], true, 200_000, true);
    let (t, reads) = run(&f, Some("mp4"));
    assert_eq!(t.isrc.as_deref(), Some(ISRC));
    assert_eq!(t.recording_mbid.as_deref(), Some(REC));
    assert_eq!(reads, 2);
}

#[test]
fn ids_that_are_not_ids_are_left_out() {
    let f = flac(&[streaminfo(), (4, vorbis(&["ISRC=not an isrc", "MUSICBRAINZ_TRACKID=12345", "ISRC=GBAYE0601498; USRC17607839"]))]);
    let t = run(&f, Some("flac")).0;
    assert_eq!(t.isrc.as_deref(), Some("GBAYE0601498"), "the first good value of the first good tag");
    assert_eq!(t.recording_mbid, None);
    assert_eq!(super::clean_isrc("usrc17607839").as_deref(), Some(ISRC));
    assert_eq!(super::clean_isrc("USRC1760783"), None);
    assert_eq!(super::clean_mbid(" 4F3C2B1A-0D9E-4C8B-A7F6-5E4D3C2B1A09\0").as_deref(), Some(REC));
}

// ------------------------------------------------------------------ parsers

#[test]
fn flac_vorbis_comments_are_read_in_one_go() {
    let f = flac(&[streaminfo(), (4, vorbis(&archive_entries()))]);
    let (t, reads) = run(&f, Some("flac"));
    assert_eq!(t, expected_archive());
    assert_eq!(reads, 1);
}

#[test]
fn flac_comments_after_a_big_picture_take_a_second_read() {
    let f = flac(&[streaminfo(), (6, vec![7; 200_000]), (4, vorbis(&["EXPLICIT=1", "ARCHIVE_VERSION=demo"]))]);
    let (t, reads) = run(&f, Some("flac"));
    assert_eq!(t.explicit, Some(true));
    assert_eq!(t.version.as_deref(), Some("demo"));
    assert_eq!(reads, 2, "one jump past the picture");
}

#[test]
fn flac_without_comments_has_no_tags() {
    let (t, _) = run(&flac(&[streaminfo()]), Some("flac"));
    assert_eq!(t, FileTags::default());
}

#[test]
fn id3v23_and_v24_user_text_frames() {
    for ver in [3u8, 4] {
        let f = [
            id3(
                ver,
                &[
                    (b"TIT2", vec![0, b'x']),
                    (b"TXXX", txxx(0, "ARCHIVE_VERSION", "snippet")),
                    (b"TXXX", txxx(1, "ARCHIVE_EDITION", "Deluxe")),
                    (b"TXXX", txxx(3, "ITUNESADVISORY", "2")),
                ],
            ),
            vec![0xff; 400],
        ]
        .concat();
        let (t, reads) = run(&f, Some("mp3"));
        assert_eq!(t.version.as_deref(), Some("snippet"), "v2.{ver}");
        assert_eq!(t.edition.as_deref(), Some("Deluxe"), "v2.{ver}");
        assert_eq!(t.explicit, Some(false), "clean, v2.{ver}");
        assert_eq!(reads, 1);
    }
}

#[test]
fn id3v22_txx_frames() {
    let f = id3(2, &[(b"TXX", txxx(0, "ARCHIVE_GROUP", "g1"))]);
    assert_eq!(run(&f, Some("mp3")).0.group.as_deref(), Some("g1"));
}

#[test]
fn id3_frames_after_a_big_cover_take_a_second_read() {
    let f = id3(3, &[(b"APIC", vec![1; 150_000]), (b"TXXX", txxx(0, "ITUNESADVISORY", "1"))]);
    let (t, reads) = run(&f, Some("mp3"));
    assert_eq!(t.explicit, Some(true));
    assert_eq!(reads, 2);
}

#[test]
fn flac_with_an_id3_tag_in_front_reads_both() {
    let f = [id3(3, &[(b"TXXX", txxx(0, "ARCHIVE_ERA", "Front"))]), flac(&[streaminfo(), (4, vorbis(&["EXPLICIT=yes"]))])].concat();
    let (t, _) = run(&f, Some("flac"));
    assert_eq!(t.era.as_deref(), Some("Front"));
    assert_eq!(t.explicit, Some(true));
}

#[test]
fn mp4_items_with_moov_up_front() {
    let f = mp4(&[atom(b"rtng", &data_atom(21, &[1])), freeform("ARCHIVE_VERSION", "Session")], false, 1000, true);
    let (t, reads) = run(&f, Some("mp4"));
    assert_eq!(t.explicit, Some(true));
    assert_eq!(t.version.as_deref(), Some("session"));
    assert_eq!(reads, 1);
}

#[test]
fn mp4_moov_at_the_end_takes_a_second_read() {
    let f = mp4(&[atom(b"rtng", &data_atom(21, &[2])), freeform("ARCHIVE_QUALITY", "LQ")], true, 300_000, true);
    let (t, reads) = run(&f, Some("mp4"));
    assert_eq!(t.explicit, Some(false));
    assert_eq!(t.quality.as_deref(), Some("lq"));
    assert_eq!(reads, 2, "one jump past the audio");
}

#[test]
fn mp4_plain_meta_atom() {
    let f = mp4(&[freeform("ARCHIVE_FLAGS", "aiedit; fanedit")], false, 10, false);
    let t = run(&f, Some("mp4")).0;
    assert_eq!(t.flags.as_deref(), Some("aiedit,fanedit"));
    assert!(t.has_flag("fanedit"));
}

#[test]
fn ogg_vorbis_opus_and_flac_comments() {
    let vc = vorbis(&["ITUNESADVISORY=1", "ARCHIVE_VERSION=build"]);
    let cases = [
        (b"\x01vorbis....".to_vec(), [b"\x03vorbis".to_vec(), vc.clone()].concat()),
        (b"OpusHead....".to_vec(), [b"OpusTags".to_vec(), vc.clone()].concat()),
        (b"\x7fFLAC....".to_vec(), [vec![0x84, 0, 0, vc.len() as u8], vc.clone()].concat()),
    ];
    for (first, comment) in cases {
        let (t, _) = run(&ogg(&first, comment), Some("ogg"));
        assert_eq!(t.explicit, Some(true));
        assert_eq!(t.version.as_deref(), Some("build"));
    }
}

#[test]
fn explicit_values_and_first_value_wins() {
    let mut t = FileTags::default();
    t.set("EXPLICIT", "0");
    t.set("ITUNESADVISORY", "1");
    assert_eq!(t.explicit, Some(false), "first one wins");
    let mut t = FileTags::default();
    t.set("ITUNESADVISORY", "0");
    assert_eq!(t.explicit, None, "0 means not tagged");
    t.set("archive_version", " Released ");
    t.set("ARCHIVE_VERSION", "leak");
    assert_eq!(t.version.as_deref(), Some("released"));
    t.set("ARCHIVE_FLAGS", "fanedit");
    t.set("ARCHIVE_FLAGS", "aiedit,fanedit");
    assert_eq!(t.flags.as_deref(), Some("fanedit,aiedit"));
    t.set("COMMENT", "ignored");
    assert!(FileTags::wants("archive_group") && !FileTags::wants("TITLE"));
}

#[test]
fn unknown_and_truncated_files_give_what_they_can() {
    let (t, _) = run(&[0u8; 100], None);
    assert_eq!(t, FileTags::default());
    // A comment block cut short by the end of the file.
    let mut f = flac(&[streaminfo(), (4, vorbis(&["EXPLICIT=1", "ARCHIVE_ERA=cut"]))]);
    f.truncate(f.len() - 270);
    let s = Sparse::new(f.clone(), None);
    assert!(matches!(parse(&s, true), Step::Done(_)));
}

// ------------------------------------------------------------------ Range reads

/// Serves files by path with Range support (or without, when `no_range`).
struct Files {
    files: HashMap<String, Vec<u8>>,
    no_range: bool,
    delay: Duration,
}

impl Respond for Files {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let Some(f) = self.files.get(req.url.path()) else { return ResponseTemplate::new(404) };
        let range = req.headers.get("range").and_then(|v| v.to_str().ok()).and_then(|v| {
            let (a, b) = v.strip_prefix("bytes=")?.split_once('-')?;
            Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
        });
        match range {
            Some((a, b)) if !self.no_range => {
                if a >= f.len() {
                    return ResponseTemplate::new(416);
                }
                let end = b.min(f.len() - 1);
                ResponseTemplate::new(206)
                    .insert_header("content-range", format!("bytes {a}-{end}/{}", f.len()).as_str())
                    .set_body_bytes(f[a..=end].to_vec())
                    .set_delay(self.delay)
            }
            _ => ResponseTemplate::new(200).set_body_bytes(f.clone()).set_delay(self.delay),
        }
    }
}

async fn serve(files: &[(&str, Vec<u8>)], no_range: bool, delay: Duration) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(Files { files: files.iter().map(|(p, b)| (p.to_string(), b.clone())).collect(), no_range, delay })
        .mount(&server)
        .await;
    server
}

fn src(server: &MockServer, path: &str) -> StreamSource {
    StreamSource { url: format!("{}{path}", server.uri()), headers: vec![("X-Plex-Token".into(), "t".into())], ..Default::default() }
}

fn reader(gate: Arc<TransferGate>, host: Option<Arc<dyn PrecacheHost>>) -> Reader {
    Reader::new(crate::audio::http::http_client(), gate, host)
}

async fn ranges(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.headers.get("range").and_then(|v| v.to_str().ok()).unwrap_or("").to_string())
        .collect()
}

#[tokio::test]
async fn first_read_is_64_kb_and_moov_at_the_end_is_one_more() {
    let file = mp4(&[atom(b"rtng", &data_atom(21, &[1]))], true, 500_000, true);
    let server = serve(&[("/library/parts/1/100/file.m4a", file.clone())], false, Duration::ZERO).await;
    let r = reader(TransferGate::new(), None);
    let t = read_tags(&r, &src(&server, "/library/parts/1/100/file.m4a"), Some("mp4")).await.unwrap();
    assert_eq!(t.explicit, Some(true));
    let got = ranges(&server).await;
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0], format!("bytes=0-{}", FIRST_READ - 1));
    // The second read starts at the atom after the audio (the moov).
    let mdat_end = atom(b"ftyp", b"M4A \0\0\0\0M4A mp42isom").len() + 500_008;
    assert!(got[1].starts_with(&format!("bytes={mdat_end}-")), "{got:?}");
    let token: Vec<_> = server.received_requests().await.unwrap().iter().map(|r| r.url.query().unwrap_or("").to_string()).collect();
    assert!(token.iter().all(|q| q.is_empty()), "the token stays in a header");
}

#[tokio::test]
async fn a_server_ignoring_range_is_read_whole_only_when_small() {
    let small = flac(&[streaminfo(), (4, vorbis(&["EXPLICIT=1"]))]);
    let big = [flac(&[streaminfo()]), vec![0; FULL_READ_MAX as usize + 10]].concat();
    let server = serve(&[("/s.flac", small), ("/b.flac", big)], true, Duration::ZERO).await;
    let r = reader(TransferGate::new(), None);
    let t = read_tags(&r, &src(&server, "/s.flac"), Some("flac")).await.unwrap();
    assert_eq!(t.explicit, Some(true));
    assert_eq!(read_tags(&r, &src(&server, "/b.flac"), Some("flac")).await, Err(FetchError::RangeRefused));
}

#[tokio::test]
async fn missing_files_and_server_trouble_are_told_apart() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::path("/gone")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    Mock::given(wiremock::matchers::path("/busy")).respond_with(ResponseTemplate::new(503)).mount(&server).await;
    let r = reader(TransferGate::new(), None);
    assert!(matches!(read_tags(&r, &src(&server, "/gone"), None).await, Err(FetchError::File(_))));
    assert!(matches!(read_tags(&r, &src(&server, "/busy"), None).await, Err(FetchError::Transient(_))));
}

struct Host(AtomicBool);
impl PrecacheHost for Host {
    fn precache_allowed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn is_streaming(&self, _: &Track) -> bool {
        false
    }
}

#[tokio::test]
async fn reads_wait_while_playback_is_downloading() {
    let server = serve(&[("/a.flac", flac(&[streaminfo()]))], false, Duration::ZERO).await;
    let host = Arc::new(Host(AtomicBool::new(false)));
    let r = reader(TransferGate::new(), Some(host.clone()));
    let s = src(&server, "/a.flac");
    let task = tokio::spawn(async move { read_tags(&r, &s, Some("flac")).await });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(ranges(&server).await.is_empty(), "nothing sent while playback downloads");
    host.0.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap();
    assert_eq!(ranges(&server).await.len(), 1);
}

#[tokio::test]
async fn reads_queue_behind_other_transfers_and_yield_to_playback() {
    let gate = TransferGate::new();
    let server = serve(&[("/a.flac", flac(&[streaminfo()]))], false, Duration::from_millis(600)).await;
    // Pre-cache holds the gate: the tag read waits.
    let pre = gate.acquire(&Priority::Precache.cell()).await;
    let r = reader(gate.clone(), None);
    let s = src(&server, "/a.flac");
    let task = tokio::spawn(async move { read_tags(&r, &s, Some("flac")).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(ranges(&server).await.is_empty(), "one transfer at a time");
    assert!(!pre.is_preempted(), "a background read never preempts");
    drop(pre);
    // Now it's sending (slow server); playback wants the gate and gets it.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(gate.holder_priority(), Some(Priority::Background as u8));
    let cur = tokio::time::timeout(Duration::from_secs(2), gate.acquire(&Priority::Current.cell())).await.unwrap();
    assert_eq!(gate.holder_priority(), Some(Priority::Current as u8));
    drop(cur);
    tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap();
    assert!(ranges(&server).await.len() >= 2, "the cut read was sent again");
}

// ------------------------------------------------------------------ cache and scan

fn album(id: &str, artist: &str, year: Option<i32>, added: i64, subformats: &[&str]) -> Album {
    Album { mbid: None,
        id: id.into(),
        title: format!("Album {id}"),
        sort_title: format!("Album {id}"),
        artist_id: artist.into(),
        artist_title: format!("Artist {artist}"),
        year,
        thumb: None,
        format: Some("album".into()),
        subformats: subformats.iter().map(|s| s.to_string()).collect(),
        section: if subformats.contains(&"demo") { ReleaseSection::Demos } else { ReleaseSection::Albums },
        genres: vec![],
        track_count: 2,
        added_at: added,
        view_count: 0,
        last_viewed_at: None,
        tint: None,
        released: None,
        studio: None,
        matched: None,
        styles: vec![],
        moods: vec![],
    }
}

fn track(id: &str, album_id: &str, n: u32, container: &str) -> Track {
    let mut t = crate::audio::testutil::track(id, "a", &format!("/library/parts/{id}/100/file.{container}"));
    t.album_id = album_id.into();
    t.artist_id = "ar".into();
    t.track_number = n;
    t.container = Some(container.into());
    t
}

fn db_with(albums: &[Album], tracks: &[Track]) -> (DbHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("t.db")).unwrap();
    db.write(|c| {
        c.execute("INSERT INTO artists (id, title, sort_title) VALUES ('ar', 'Artist', 'Artist')", [])?;
        let tx = c.transaction()?;
        crate::db::store::upsert_albums(&tx, albums, 1)?;
        crate::db::store::upsert_tracks(&tx, tracks, 1)?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    (db, dir)
}

fn tags(f: impl FnOnce(&mut FileTags)) -> FileTags {
    let mut t = FileTags::default();
    f(&mut t);
    t
}

fn save(c: &Connection, part: &str, t: &FileTags) {
    store::save(c, part, Status::Ok, t, 1).unwrap();
}

/// Serves every part from one base URL, noting which track asked.
struct Parts(String, parking_lot::Mutex<Vec<String>>);
impl Parts {
    fn new(base: String) -> Self {
        Self(base, parking_lot::Mutex::default())
    }
}
#[async_trait::async_trait]
impl PartSource for Parts {
    async fn part(&self, track_id: &str, part_key: &str) -> AppResult<StreamSource> {
        if self.0.is_empty() {
            return Err(AppError::Network("offline".into()));
        }
        self.1.lock().push(track_id.to_string());
        Ok(StreamSource { url: format!("{}{part_key}", self.0), ..Default::default() })
    }
}

#[derive(Default)]
struct Events {
    changed: parking_lot::Mutex<usize>,
    last: parking_lot::Mutex<Option<TagScanStatus>>,
}
impl TagEvents for Events {
    fn progress(&self, s: &TagScanStatus) {
        *self.last.lock() = Some(*s);
    }
    fn changed(&self) {
        *self.changed.lock() += 1;
    }
}

async fn until(mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition never held");
}

#[tokio::test]
async fn the_scan_reads_each_file_once_and_again_when_it_changes() {
    let a = album("1", "ar", Some(2020), 10, &[]);
    let t1 = track("11", "1", 1, "flac");
    let t2 = track("12", "1", 2, "mp3");
    let t3 = track("13", "1", 3, "flac");
    // t3 is missing on the server: marked failed, not retried until it changes.
    let files = [
        (t1.part_key.clone(), flac(&[streaminfo(), (4, vorbis(&["ITUNESADVISORY=1", "ARCHIVE_VERSION=leak"]))])),
        (t2.part_key.clone(), id3(4, &[(b"TXXX", txxx(3, "ARCHIVE_QUALITY", "lossless"))])),
    ];
    let refs: Vec<(&str, Vec<u8>)> = files.iter().map(|(p, b)| (p.as_str(), b.clone())).collect();
    let server = serve(&refs, false, Duration::ZERO).await;
    let (db, _d) = db_with(&[a], &[t1.clone(), t2.clone(), t3.clone()]);
    let events = Arc::new(Events::default());
    let scan = TagScanner::new(
        db.clone(),
        Arc::new(Parts::new(server.uri())),
        reader(TransferGate::new(), None),
        Some(events.clone()),
    );
    scan.spawn();
    until(|| scan.status().done == 3 && !scan.status().running).await;
    let st = scan.status();
    assert_eq!((st.done, st.total, st.failed), (3, 3, 1));
    let (status, t) = db.read(|c| store::get(c, &t1.part_key)).unwrap().unwrap();
    assert_eq!((status.as_str(), t.explicit, t.version.as_deref()), ("ok", Some(true), Some("leak")));
    assert_eq!(db.read(|c| store::get(c, &t3.part_key)).unwrap().unwrap().0, "failed");
    assert!(*events.changed.lock() >= 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 3, "one read per file");

    // The file changes: Plex gives it a new part key, so it's read again
    // and the old row goes.
    let new_key = "/library/parts/11/200/file.flac".to_string();
    db.write(|c| {
        c.execute("UPDATE tracks SET part_key = ?1 WHERE id = '11'", [&new_key])?;
        Ok(())
    })
    .unwrap();
    scan.poke();
    until(|| db.read(|c| store::get(c, &new_key)).unwrap().is_some()).await;
    assert_eq!(db.read(|c| store::get(c, &new_key)).unwrap().unwrap().0, "failed", "404 here");
    // The scan pruned the row for the key no track has any more.
    assert!(db.read(|c| store::get(c, &t1.part_key)).unwrap().is_none());
    assert_eq!(db.write(|c| store::prune(c)).unwrap(), 0);
}

#[tokio::test]
async fn each_file_is_asked_of_the_server_its_track_is_on() {
    // Part keys don't carry a server; the track id says which one has the file.
    let id = format!("{}:21", crate::sid::tag("other-server"));
    let mut t = track("21", "1", 1, "flac");
    t.id = id.clone();
    let server = serve(&[(t.part_key.as_str(), flac(&[streaminfo()]))], false, Duration::ZERO).await;
    let (db, _d) = db_with(&[album("1", "ar", None, 1, &[])], &[t]);
    let parts = Arc::new(Parts::new(server.uri()));
    let scan = TagScanner::new(db.clone(), parts.clone(), reader(TransferGate::new(), None), None);
    scan.spawn();
    until(|| scan.status().done == 1 && !scan.status().running).await;
    assert_eq!(*parts.1.lock(), [id]);
}

#[tokio::test]
async fn files_read_by_an_older_build_are_read_again_for_their_ids() {
    let t1 = track("11", "1", 1, "flac");
    let t2 = track("12", "1", 2, "flac");
    let mut a = album("1", "ar", Some(2020), 10, &[]);
    a.matched = Some(true);
    let (db, _d) = db_with(&[a], &[t1.clone(), t2.clone()]);
    // t1 was read before ids were kept: an old row (version 0) with a badge.
    db.write(|c| {
        c.execute(
            "INSERT INTO file_tags (part_key, status, version, scanned_at) VALUES (?1, 'ok', 'leak', 1)",
            [&t1.part_key],
        )?;
        Ok(())
    })
    .unwrap();
    let pending = db.read(|c| store::pending(c, 10)).unwrap();
    let ids: Vec<&str> = pending.iter().map(|p| p.track_id.as_str()).collect();
    assert_eq!(ids, ["12", "11"], "never-read files first, then old reads");
    assert_eq!(db.read(store::counts).unwrap().done, 0, "an old read doesn't count as done");
    // The old row keeps working until the new read replaces it.
    assert_eq!(db.read(|c| store::track_version(c, "11")).unwrap().as_deref(), Some("leak"));

    let files = [
        (t1.part_key.clone(), flac(&[streaminfo(), (4, vorbis(&["ARCHIVE_VERSION=leak", &format!("ISRC={ISRC}")]))])),
        (t2.part_key.clone(), flac(&[streaminfo(), (4, vorbis(&[&format!("MUSICBRAINZ_TRACKID={REC}")]))])),
    ];
    let refs: Vec<(&str, Vec<u8>)> = files.iter().map(|(p, b)| (p.as_str(), b.clone())).collect();
    let server = serve(&refs, false, Duration::ZERO).await;
    let scan = TagScanner::new(db.clone(), Arc::new(Parts::new(server.uri())), reader(TransferGate::new(), None), None);
    scan.spawn();
    until(|| scan.status().done == 2 && !scan.status().running).await;
    let (_, t) = db.read(|c| store::get(c, &t1.part_key)).unwrap().unwrap();
    assert_eq!((t.isrc.as_deref(), t.version.as_deref()), (Some(ISRC), Some("leak")));
    assert!(db.read(|c| store::pending(c, 10)).unwrap().is_empty(), "read once at the new version");
    // The ids reach the song index for those songs' artist.
    until(|| {
        db.read(|c| {
            Ok(c.query_row("SELECT count(*) FROM song_index WHERE isrc = ?1 OR recording_mbid = ?2", [ISRC, REC], |r| {
                r.get::<_, i64>(0)
            })?)
        })
        .unwrap()
            == 2
    })
    .await;
}

#[tokio::test]
async fn the_scan_pauses_offline_and_when_turned_off() {
    let (db, _d) = db_with(&[album("1", "ar", None, 1, &[])], &[track("11", "1", 1, "flac")]);
    let events = Arc::new(Events::default());
    let scan = TagScanner::new(db.clone(), Arc::new(Parts::new(String::new())), reader(TransferGate::new(), None), Some(events.clone()));
    scan.spawn();
    until(|| scan.status().paused).await;
    assert_eq!(scan.status().done, 0, "nothing marked while offline");
    assert!(db.read(|c| store::get(c, "/library/parts/11/100/file.flac")).unwrap().is_none());
    scan.set_enabled(false);
    until(|| !scan.status().enabled && !scan.status().running).await;
    assert!(!events.last.lock().unwrap().enabled);
}

// ------------------------------------------------------------------ editions, filters, badges

fn member(id: &str, edition: Option<&str>, version: Option<&str>, year: Option<i32>, added: i64) -> Member {
    Member {
        album_id: id.into(),
        edition: edition.map(str::to_string),
        version: version.map(str::to_string),
        unreleased_type: false,
        year,
        added_at: added,
    }
}

#[test]
fn the_face_is_standard_then_released_then_earliest() {
    let m = [
        member("a", Some("Deluxe"), Some("released"), Some(2020), 5),
        member("b", Some("Standard"), Some("released"), Some(2021), 9),
    ];
    assert_eq!(store::face(&m).unwrap().album_id, "b");
    let m = [member("a", Some("Build v1"), Some("leak"), Some(2018), 5), member("b", None, Some("released"), Some(2021), 9)];
    assert_eq!(store::face(&m).unwrap().album_id, "b");
    let m = [member("a", None, Some("leak"), Some(2019), 5), member("b", None, Some("build"), Some(2018), 9), member("c", None, Some("leak"), None, 1)];
    assert_eq!(store::face(&m).unwrap().album_id, "b", "earliest year; no year last");
    let mut demo = member("d", None, None, Some(2010), 1);
    demo.unreleased_type = true;
    assert_eq!(store::face(&[demo, member("e", None, None, Some(2015), 2)]).unwrap().album_id, "e");
}

/// Two editions of one album (1 Standard, 2 Deluxe), an album of fan edits
/// (3), and a plain album (4).
fn grouped_library() -> (DbHandle, tempfile::TempDir) {
    let albums = [
        album("1", "ar", Some(2020), 10, &[]),
        album("2", "ar", Some(2021), 20, &[]),
        album("3", "ar", Some(2022), 30, &[]),
        album("4", "ar", Some(2019), 40, &[]),
    ];
    let tracks = [
        track("11", "1", 1, "flac"),
        track("12", "1", 2, "flac"),
        track("21", "2", 1, "flac"),
        track("22", "2", 2, "flac"),
        track("31", "3", 1, "mp3"),
        track("41", "4", 1, "mp3"),
        track("42", "4", 2, "mp3"),
    ];
    let (db, d) = db_with(&albums, &tracks);
    db.write(|c| {
        let p = |id: &str, ext: &str| format!("/library/parts/{id}/100/file.{ext}");
        for id in ["11", "12"] {
            save(c, &p(id, "flac"), &tags(|t| {
                t.set("ARCHIVE_GROUP", "g");
                t.set("ARCHIVE_EDITION", "Standard");
                t.set("ARCHIVE_VERSION", "released");
            }));
        }
        save(c, &p("21", "flac"), &tags(|t| {
            t.set("ARCHIVE_GROUP", "g");
            t.set("ARCHIVE_EDITION", "Deluxe");
            t.set("ITUNESADVISORY", "1");
        }));
        save(c, &p("22", "flac"), &tags(|t| t.set("ARCHIVE_GROUP", "g")));
        save(c, &p("31", "mp3"), &tags(|t| t.set("ARCHIVE_FLAGS", "aiedit")));
        save(c, &p("41", "mp3"), &tags(|t| t.set("EXPLICIT", "1")));
        save(c, &p("42", "mp3"), &FileTags::default());
        store::refresh_albums(c)
    })
    .unwrap();
    (db, d)
}

#[test]
fn editions_collapse_into_one_card_and_edits_hide() {
    use crate::db::queries::albums_view;
    use crate::db::types::AlbumSort;
    let (db, _d) = grouped_library();
    let ids = |view: TagView| -> Vec<String> {
        db.read(|c| albums_view(c, AlbumSort::Name, 0, 100, None, view)).unwrap().items.into_iter().map(|a| a.id).collect()
    };
    let page = db.read(|c| albums_view(c, AlbumSort::Name, 0, 100, None, TagView::default())).unwrap();
    assert_eq!(ids(TagView::default()), vec!["1", "4"], "Deluxe behind Standard, the fan-edit album hidden");
    assert_eq!(page.total, 2);
    let all = TagView { group_editions: false, show_edits: true, filter_explicit: false };
    assert_eq!(ids(all), vec!["1", "2", "3", "4"]);

    // Artist page sections follow the same rule.
    let mut ap = db.read(|c| crate::db::queries::artist_page(c, "ar")).unwrap();
    db.read(|c| super::view::artist_page(c, &mut ap, TagView::default())).unwrap();
    let shown: Vec<_> = ap.sections.iter().flat_map(|s| s.albums.iter().map(|a| a.id.clone())).collect();
    assert_eq!(shown, vec!["1", "4"]);

    // The picker lists both editions, face first, with their labels.
    let ix = db.read(store::index).unwrap();
    let g = &ix.groups["g"];
    assert_eq!(g.iter().map(|e| (e.album_id.as_str(), e.label.as_deref())).collect::<Vec<_>>(), vec![("1", Some("Standard")), ("2", Some("Deluxe"))]);
    assert!(ix.albums["2"].explicit && !ix.albums["1"].explicit);
    assert!(ix.tracks["21"].explicit && ix.tracks["31"].edit);
    assert_eq!(ix.tracks["11"].version.as_deref(), Some("released"));
    assert!(!ix.tracks.contains_key("42"), "nothing to show, not listed");

    // The album page of the Deluxe edition: no Standard under "More by".
    let mut p = db.read(|c| crate::db::queries::album_page(c, "2")).unwrap();
    db.read(|c| super::view::album_page(c, &mut p, TagView::default())).unwrap();
    assert!(p.more_by_artist.iter().all(|a| a.id != "1" && a.id != "3"));
}

#[test]
fn explicit_filter_and_edits_leave_play_contexts() {
    let (db, _d) = grouped_library();
    let mut tracks = db.read(|c| crate::db::queries::tracks_by_ids(c, &["21".into(), "22".into(), "31".into(), "41".into(), "42".into()])).unwrap();
    let filtered = TagView { filter_explicit: true, ..TagView::default() };
    let skipped = db.read(|c| store::drop_hidden(c, &mut tracks, filtered)).unwrap();
    assert_eq!(skipped, Skipped { explicit: 2, edits: 1 });
    assert_eq!(tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["22", "42"]);
    assert_eq!(
        super::commands::skipped_message(skipped).unwrap(),
        "Skipped 2 explicit songs and 1 fan or AI edit."
    );
    assert_eq!(super::commands::skipped_message(Skipped::default()), None);
    let mut all = db.read(|c| crate::db::queries::tracks_by_ids(c, &["21".into(), "31".into()])).unwrap();
    let allowed = TagView { show_edits: true, ..TagView::default() };
    assert_eq!(db.read(|c| store::drop_hidden(c, &mut all, allowed)).unwrap(), Skipped::default());
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn album_contexts_skip_explicit_songs_with_one_notice() {
    use crate::audio::source::{ContextKind, PlayContext, TrackLookup};
    let (db, _d) = grouped_library();
    db.set_tag_view(TagView { filter_explicit: true, ..TagView::default() });
    let lookup = crate::bridge::CacheLookup(db.clone(), None, Default::default(), None);
    let ctx = PlayContext { kind: ContextKind::Album, id: Some("2".into()), track_ids: None };
    let (r, skipped) = lookup.context_with(&ctx, db.tag_view()).await.unwrap();
    assert_eq!(r.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["22"]);
    assert_eq!(skipped.explicit, 1);
    // An album of only explicit songs can't play, and says why.
    let only = PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(vec!["41".into()]) };
    let err = lookup.context(&only).await.unwrap_err();
    assert!(err.to_string().contains("explicit"), "{err}");
    // The artist context leaves explicit songs and fan edits out too.
    let artist = PlayContext { kind: ContextKind::Artist, id: Some("ar".into()), track_ids: None };
    let (r, skipped) = lookup.context_with(&artist, db.tag_view()).await.unwrap();
    assert!(r.tracks.iter().all(|t| !["21", "31", "41"].contains(&t.id.as_str())));
    assert_eq!(skipped, Skipped { explicit: 2, edits: 1 });
}

#[test]
fn archive_versions_make_shares_and_discord_unreleased() {
    let (db, _d) = grouped_library();
    db.write(|c| {
        save(c, "/library/parts/41/100/file.mp3", &tags(|t| t.set("ARCHIVE_VERSION", "leak")));
        store::refresh_albums(c)
    })
    .unwrap();
    let v = db.read(|c| store::track_version(c, "41")).unwrap();
    assert!(crate::db::release::is_unreleased(Some("album"), &[] as &[&str], Some(true), None, v.as_deref(), None));
    assert_eq!(db.read(|c| store::album_version(c, "1")).unwrap().as_deref(), Some("released"));
    assert_eq!(db.read(|c| store::track_version(c, "42")).unwrap(), None);
}
