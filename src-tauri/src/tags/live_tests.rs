//! Live, read-only check of tag reads against the developer's real server.
//! `#[ignore]`; run with `cargo test live_tags -- --ignored --nocapture`
//! while nothing is playing (one transfer at a time).
//!
//! Prints counts only: formats, reads per file and which tags exist. Never
//! names, values, addresses or ids. The token comes from the `.env` named by
//! PLATTER_ENV (default: the repo root's `.env`) and is never printed.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::audio::StreamSource;
use crate::audio::gate::TransferGate;
use crate::plex::auth::PlexTv;
use crate::plex::client::PlexClient;
use crate::plex::discovery::{self, RaceConfig};
use crate::plex::identity::ClientIdentity;
use crate::plex::token::Token;

use super::fetch::Reader;
use super::parse::{FIRST_READ, MORE_READ, Sparse, Step, parse};

/// Files read per container.
const PER_FORMAT: usize = 3;

fn dev_token() -> Token {
    let path = std::env::var("PLATTER_ENV").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../.env").into());
    for item in dotenvy::from_path_iter(path).expect(".env not found") {
        let (k, v) = item.expect(".env parse");
        if k == "PLEX_TOKEN" {
            return Token::new(v).expect("empty PLEX_TOKEN");
        }
    }
    panic!("PLEX_TOKEN missing from .env");
}

async fn connect() -> PlexClient {
    let mut id = ClientIdentity::with_id("platter-live-tests-0001");
    id.device_name = "Platter live tests".into();
    let id = Arc::new(id);
    let tv = PlexTv::new(id.clone()).unwrap();
    let token = dev_token();
    let servers = discovery::servers_from_resources(tv.resources(&token).await.unwrap());
    let server = servers.iter().find(|s| s.owned).or(servers.first()).expect("no server");
    let http = reqwest::Client::builder().default_headers(id.headers().unwrap()).build().unwrap();
    let p = discovery::race(&http, &server.connections, Some(&server.machine_id), RaceConfig::default())
        .await
        .expect("no connection answered");
    PlexClient::new(&p.uri, server.access_token.clone().unwrap_or(token), id, Some(p.kind)).unwrap()
}

#[tokio::test]
#[ignore]
async fn live_tags_range_reads() {
    let c = connect().await;
    let sec = c.music_sections().await.unwrap().remove(0);
    let total = c.tracks(&sec.id, 0, 1).await.unwrap().total;
    // Spread the sample over the library: a page from the start, middle and end.
    let mut picked: BTreeMap<String, Vec<(String, Option<String>)>> = BTreeMap::new();
    for start in [0, total / 3, total * 2 / 3, total.saturating_sub(200)] {
        for t in c.tracks(&sec.id, start, 200).await.unwrap().items {
            let fmt = t.container.clone().unwrap_or_else(|| "unknown".into());
            let list = picked.entry(fmt).or_default();
            if list.len() < PER_FORMAT && !list.iter().any(|x| x.0 == t.part_key) {
                list.push((t.part_key.clone(), t.container.clone()));
            }
        }
    }
    let reader = Reader::new(crate::audio::http::http_client(), TransferGate::new(), None);
    for (fmt, parts) in &picked {
        let mut reads_hist: BTreeMap<usize, usize> = BTreeMap::new();
        let mut present: BTreeMap<&str, usize> = BTreeMap::new();
        let mut range_ok = 0;
        for (key, container) in parts {
            let req = c.stream_request(key).unwrap();
            let src = StreamSource {
                url: req.url.to_string(),
                headers: req
                    .headers
                    .iter()
                    .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
                    .collect(),
                ..Default::default()
            };
            let first = match reader.range(&src, 0, FIRST_READ).await {
                Ok(g) => g,
                Err(e) => {
                    println!("{fmt}: read failed ({:?})", std::mem::discriminant(&e));
                    continue;
                }
            };
            if !first.whole {
                range_ok += 1;
            }
            let mut s = Sparse::new(first.bytes, first.total);
            s.container = container.clone();
            let mut reads = 1;
            let tags = loop {
                match parse(&s, reads > 6) {
                    Step::Done(t) => break t,
                    Step::Need { offset, len } => {
                        let got = reader.range(&src, offset, len.max(MORE_READ)).await.unwrap();
                        s.add(offset, got.bytes);
                        reads += 1;
                    }
                }
            };
            *reads_hist.entry(reads).or_default() += 1;
            for (name, has) in [
                ("explicit", tags.explicit.is_some()),
                ("version", tags.version.is_some()),
                ("era", tags.era.is_some()),
                ("quality", tags.quality.is_some()),
                ("date", tags.date.is_some()),
                ("edition", tags.edition.is_some()),
                ("group", tags.group.is_some()),
                ("source", tags.source.is_some()),
                ("flags", tags.flags.is_some()),
            ] {
                if has {
                    *present.entry(name).or_default() += 1;
                }
            }
        }
        println!(
            "{fmt}: files {}, range honoured {range_ok}, reads per file {reads_hist:?}, tags present {present:?}",
            parts.len()
        );
    }
}
