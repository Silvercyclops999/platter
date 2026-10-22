//! Bringing music over all the way to Lidarr: an account data download is
//! read, its missing albums are picked on MusicBrainz and queued, and the
//! Get albums queue asks the faked Lidarr of the request tests. Nothing real
//! is reached. Shows that only the allowed writes reach Lidarr, that each
//! one is in the Changes log, that practice mode writes nothing, and that
//! the library is left as it was.

use std::io::Write;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::params;
use serde_json::json;

use super::mbpick::tests::RG_SECOND;
use super::request_tests::{Rig, assert_only_allowed_writes, rig};
use super::waves::{Waves, WavesDeps};
use super::waves_tests::Told;
use crate::error::AppResult;
use crate::import::bring::{AlbumState, AppGetter, Bring, BringDeps, Server};
use crate::web::Method;

#[derive(Default)]
struct Recorded {
    liked: Mutex<Vec<String>>,
    made: Mutex<Vec<String>>,
}

#[async_trait]
impl Server for Recorded {
    async fn like(&self, ids: Vec<String>) -> AppResult<()> {
        self.liked.lock().extend(ids);
        Ok(())
    }
    async fn create(&self, name: &str, _ids: Vec<String>) -> AppResult<String> {
        self.made.lock().push(name.to_string());
        Ok(format!("pl{}", self.made.lock().len()))
    }
    async fn add(&self, _id: &str, _ids: Vec<String>) -> AppResult<()> {
        Ok(())
    }
}

/// A data download: one liked song the library has (First Light), and
/// songs from three albums it doesn't: Second Wind (on MusicBrainz), the
/// Basement Tapes bootleg and Nowhere Songs (not on MusicBrainz).
fn download() -> Vec<u8> {
    let liked = json!({"tracks": [
        {"artist": "Northlight", "album": "First Light", "track": "Dawn", "uri": "svc:track:1"},
        {"artist": "Northlight", "album": "Second Wind", "track": "Breathe In", "uri": "svc:track:2"},
        {"artist": "Northlight", "album": "Second Wind", "track": "Breathe Out", "uri": "svc:track:3"},
        {"artist": "Northlight", "album": "Basement Tapes", "track": "Cellar", "uri": "svc:track:4"},
        {"artist": "Northlight", "album": "Nowhere Songs", "track": "Gone", "uri": "svc:track:5"}
    ], "albums": []});
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        z.start_file("Account Data/YourLibrary.json", opts).unwrap();
        z.write_all(&serde_json::to_vec(&liked).unwrap()).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

struct Setup {
    r: Rig,
    w: Waves,
    server: Arc<Recorded>,
    bring: Bring,
}

/// Lidarr set up and checked, Bring in my listening left off (the default):
/// Get albums goes on regardless.
async fn setup(practice: bool) -> Setup {
    let r = rig();
    let mut s = crate::settings::load(&r.db);
    s.lidarr_practice_until = if practice { 1_900_000_000 } else { 0 };
    crate::settings::save(&r.db, &s).unwrap();
    r.db.write(|c| {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key, src)
             VALUES ('t1', 'Dawn', 'a1', 'First Light', 'ar1', 'Northlight', 200000, '/p/t1', ?1)",
            params!["aaaaaaaa"],
        )?;
        crate::identity::index::rebuild(c)?;
        Ok(())
    })
    .unwrap();
    r.web.st.lock().free_space = Some(400_000_000_000);
    r.lidarr.check().await.unwrap();
    r.db.write(|c| crate::db::store::kv_set(c, "lidarr.typedConfirmed", "1")).unwrap();
    let day = Arc::new(Mutex::new(100));
    let w = Waves::new(WavesDeps {
        lidarr: r.lidarr.clone(),
        notifier: Arc::new(Told::default()),
        day_of: Arc::new(move |_| *day.lock()),
        hidden: Arc::new(Vec::new),
        utc_offset: None,
    });
    let server = Arc::new(Recorded::default());
    let bring = Bring::new(BringDeps {
        db: r.db.clone(),
        server: server.clone(),
        getter: Arc::new(AppGetter { lidarr: r.lidarr.clone(), waves: w.clone() }),
        clock_ms: Arc::new(|| 1_800_000_000_000),
    });
    Setup { r, w, server, bring }
}

fn library(r: &Rig) -> String {
    r.db.read(|c| {
        Ok(c.query_row(
            "SELECT (SELECT group_concat(id || title || album_id || part_key, '|') FROM tracks) || '#' ||
                    (SELECT group_concat(id || title || artist_title, '|') FROM albums)",
            [],
            |r| r.get(0),
        )?)
    })
    .unwrap()
}

fn writes(r: &Rig) -> Vec<(Method, String)> {
    r.web.log.lock().iter().filter(|(m, _, _)| *m != Method::Get).map(|(m, p, _)| (*m, p.clone())).collect()
}

#[tokio::test]
async fn a_download_goes_all_the_way_to_lidarr_with_only_allowed_logged_writes() {
    let t = setup(false).await;
    let before = library(&t.r);
    let file = t.r._dir.path().join("my_data.zip");
    std::fs::write(&file, download()).unwrap();

    let st = t.bring.read(file).await.unwrap();
    assert_eq!(st.summary.sentence, "1 of 5 liked songs are already in your library. 4 songs from 3 albums aren't.");
    assert_eq!(*t.server.liked.lock(), ["t1"]);
    assert!(writes(&t.r).is_empty(), "reading a file asks Lidarr for nothing");

    t.bring.get(None).await.unwrap();
    t.bring.pick_pending().await.unwrap();
    let st = t.bring.status().await.unwrap();
    let state = |name: &str| st.albums.iter().find(|a| a.album == name).unwrap().state;
    assert_eq!(state("Second Wind"), AlbumState::Asked);
    assert_eq!(state("Nowhere Songs"), AlbumState::NotFound, "not on MusicBrainz: stays on the list");
    assert!(writes(&t.r).is_empty(), "picking only reads");

    // The Get albums queue asks Lidarr.
    let out = t.w.pass().await.unwrap();
    assert!(out.requested.contains(&"Second Wind".to_string()), "{out:?}");
    let recs = t.r.lidarr.requests();
    assert!(recs.iter().any(|r| r.release_group_id == RG_SECOND));
    assert_only_allowed_writes(&t.r.web);
    // Nothing Lidarr already had was changed: the only artist added is new,
    // nothing is deleted, and nothing but the asked album is monitored.
    for (m, p) in writes(&t.r) {
        assert!(
            matches!((m, p.as_str()), (Method::Post, "/api/v1/artist") | (Method::Post, "/api/v1/command") | (Method::Put, "/api/v1/album/monitor")),
            "{m:?} {p}"
        );
    }
    let monitored: Vec<String> =
        t.r.web.st.lock().albums.iter().filter(|a| a["monitored"] == true).filter_map(|a| a["foreignAlbumId"].as_str().map(String::from)).collect();
    assert_eq!(monitored, [RG_SECOND]);
    // Every write is in the Changes log.
    let log: Vec<String> = t.r.lidarr.changes().into_iter().map(|c| c.what).collect();
    assert!(log.iter().any(|w| w == "Queued 1 album (Bring music over)"), "{log:?}");
    assert!(log.iter().any(|w| w.starts_with("Requested Second Wind by Northlight")), "{log:?}");
    // The library is as it was.
    assert_eq!(library(&t.r), before);

    // Asking again sends nothing twice.
    let n = writes(&t.r).len();
    t.bring.get(None).await.unwrap();
    t.bring.pick_pending().await.unwrap();
    t.w.pass().await.unwrap();
    assert_eq!(writes(&t.r).len(), n);
}

#[tokio::test]
async fn practice_mode_writes_nothing_to_lidarr_and_logs_what_it_would_do() {
    let t = setup(true).await;
    let file = t.r._dir.path().join("my_data.zip");
    std::fs::write(&file, download()).unwrap();
    t.bring.read(file).await.unwrap();
    t.bring.get(None).await.unwrap();
    t.bring.pick_pending().await.unwrap();
    t.w.pass().await.unwrap();
    assert!(writes(&t.r).is_empty(), "{:?}", writes(&t.r));
    let practice: Vec<String> = t.r.lidarr.changes().into_iter().filter(|c| c.practice).map(|c| c.what).collect();
    assert!(practice.iter().any(|w| w.contains("Second Wind")), "{practice:?}");
}

#[tokio::test]
async fn lidarr_out_of_reach_asks_for_nothing_and_carries_on_later() {
    let t = setup(false).await;
    let file = t.r._dir.path().join("my_data.zip");
    std::fs::write(&file, download()).unwrap();
    t.bring.read(file).await.unwrap();
    t.bring.get(None).await.unwrap();
    t.bring.pick_pending().await.unwrap();
    t.r.web.st.lock().down = true;
    let out = t.w.pass().await.unwrap();
    assert!(out.requested.is_empty());
    assert!(out.paused.is_some(), "the queue pauses and says why");
    assert!(writes(&t.r).is_empty());
    t.r.web.st.lock().down = false;
    let out = t.w.pass().await.unwrap();
    assert!(out.requested.contains(&"Second Wind".to_string()), "{out:?}");
    assert_only_allowed_writes(&t.r.web);
}
