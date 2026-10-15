//! Against a real Jellyfin server, only one made for testing: never a
//! server with anyone's own music on it. Skipped unless asked for.
//!
//! To run it, start a throwaway Jellyfin (10.9 or later) with a music
//! library of freely licensed songs (Creative Commons), finish its setup
//! wizard with a test user, then:
//!
//! ```text
//! docker run -d --name jf-test -p 8096:8096 -v /path/to/cc-music:/music:ro jellyfin/jellyfin
//! JELLYFIN_TEST_URL=http://localhost:8096 JELLYFIN_TEST_USER=test JELLYFIN_TEST_PASSWORD=test \
//!   cargo test jellyfin::live_tests -- --ignored --nocapture
//! ```
//!
//! Remove the container afterwards (`docker rm -f jf-test`).

use std::sync::Arc;

use crate::db::source::ItemKind;

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

#[tokio::test]
#[ignore = "needs a throwaway Jellyfin server (JELLYFIN_TEST_URL)"]
async fn signs_in_reads_the_library_and_streams() {
    let (Some(url), Some(user), Some(password)) = (env("JELLYFIN_TEST_URL"), env("JELLYFIN_TEST_USER"), env("JELLYFIN_TEST_PASSWORD")) else {
        eprintln!("JELLYFIN_TEST_URL, JELLYFIN_TEST_USER and JELLYFIN_TEST_PASSWORD aren't set; skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let backends = crate::backend::Backends::empty();
    backends.keep_records_in(dir.path().to_path_buf());
    let jf = super::Jellyfin::new(super::manager::Deps {
        backends: backends.clone(),
        db: None,
        dir: dir.path().to_path_buf(),
        store: Arc::new(crate::plex::keychain::MemoryStore::new()),
        http: super::client::http(),
        device_id: "platter-live-test".into(),
        prefs: Arc::default(),
        offline: Arc::default(),
        live: false,
    });
    let check = jf.check(&url).await.unwrap();
    eprintln!("server {} version {} quick connect {}", check.name, check.version, check.quick_connect);
    let saved = jf.password(&url, &user, password).await.unwrap();
    let server = backends.server(&saved.tag).unwrap();
    let lib = server.library();
    let sections = lib.music_sections().await.unwrap();
    assert!(!sections.is_empty(), "the test server needs a music library");
    let section = &sections[0].id;
    let n = lib.count(section, ItemKind::Track).await.unwrap();
    assert!(n > 0, "the music library is empty");
    let tracks = lib.tracks(section, 0, 5).await.unwrap();
    let t = &tracks[0];
    assert_eq!(crate::sid::tag_of(&t.id), Some(saved.tag.as_str()));
    let src = server.streams().stream(t).await.unwrap();
    let resp = reqwest::Client::new()
        .get(&src.url)
        .header("Range", "bytes=0-1023")
        .header(&src.headers[0].0, &src.headers[0].1)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let liked = lib.liked_tracks(section).await.unwrap().unwrap_or_default();
    eprintln!("{n} songs, {} liked", liked.len());
    jf.remove(&saved.tag).await.unwrap();
}
