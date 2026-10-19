//! Which way Lidarr is reached: the home address, the away address, a
//! computer at home, then the Bridge, with each one up or down.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use bytes::Bytes;

use super::Lidarr;
use super::api::{LidarrGet, LidarrRoute, Route};
use super::request_tests::rig;
use super::write::{LidarrWrite, WriteCall};
use crate::artistinfo::http::HttpResponse;
use crate::error::{AppError, AppResult};

/// A route that answers like Lidarr, or isn't reachable.
struct Fake(Route, bool);

#[async_trait]
impl LidarrGet for Fake {
    async fn get(&self, _: &str, _: &[(&str, &str)]) -> AppResult<HttpResponse> {
        if !self.1 {
            return Err(AppError::Network("unreachable".into()));
        }
        Ok(HttpResponse { status: 200, body: Bytes::from_static(br#"{"appName": "Lidarr", "version": "2.8.2"}"#), ..Default::default() })
    }
    fn route(&self) -> Route {
        self.0
    }
}

#[async_trait]
impl LidarrWrite for Fake {
    async fn write(&self, _: &WriteCall) -> AppResult<HttpResponse> {
        Err(AppError::Network("unreachable".into()))
    }
    fn route(&self) -> Route {
        self.0
    }
}

/// A computer and a Bridge whose being up can be switched.
fn sides(l: &Lidarr) -> (Arc<AtomicBool>, Arc<AtomicBool>) {
    let (pc, br) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let p = pc.clone();
    l.set_computer_route(Some(Arc::new(move || p.load(Ordering::SeqCst).then(|| Arc::new(Fake(Route::Computer, true)) as Arc<dyn LidarrRoute>))));
    let b = br.clone();
    l.set_bridge_route(Some(Arc::new(move || b.load(Ordering::SeqCst).then(|| Arc::new(Fake(Route::Bridge, true)) as Arc<dyn LidarrRoute>))));
    (pc, br)
}

fn order(l: &Lidarr) -> Result<Vec<Route>, String> {
    l.routes().map(|v| v.iter().map(|r| LidarrGet::route(r.as_ref())).collect()).map_err(|e| e.to_string())
}

#[test]
fn routes_go_home_away_computer_then_bridge_with_each_side_up_or_down() {
    let r = rig();
    let mut s = crate::settings::load(&r.db);
    s.lidarr_away_url = "https://lidarr.example.com".into();
    crate::settings::save(&r.db, &s).unwrap();
    // The key entered with both addresses set.
    r.lidarr.set_key(Some("abc123")).unwrap();
    let (pc, br) = sides(&r.lidarr);
    use Route::*;
    for (c, b, want) in [
        (true, true, vec![Home, Away, Computer, Bridge]),
        (false, true, vec![Home, Away, Bridge]),
        (true, false, vec![Home, Away, Computer]),
        (false, false, vec![Home, Away]),
    ] {
        pc.store(c, Ordering::SeqCst);
        br.store(b, Ordering::SeqCst);
        assert_eq!(order(&r.lidarr), Ok(want), "computer {c}, bridge {b}");
        assert_eq!(r.lidarr.status().home_side, c || b);
    }

    // Nothing set up here (a phone): only the home sides, which hold the key.
    r.lidarr.set_key(None).unwrap();
    let mut s = crate::settings::load(&r.db);
    s.lidarr_url = String::new();
    s.lidarr_away_url = String::new();
    crate::settings::save(&r.db, &s).unwrap();
    for (c, b, want) in [
        (true, true, Ok(vec![Computer, Bridge])),
        (true, false, Ok(vec![Computer])),
        (false, true, Ok(vec![Bridge])),
        (false, false, Err("invalid: Set Lidarr's address first.".to_string())),
    ] {
        pc.store(c, Ordering::SeqCst);
        br.store(b, Ordering::SeqCst);
        assert_eq!(order(&r.lidarr), want, "computer {c}, bridge {b}");
        let st = r.lidarr.status();
        assert_eq!(st.usable(), c || b, "{st:?}");
        assert!(!st.ready, "not before a check passes through it");
    }
    // An address here but no key: the home sides still answer.
    let mut s = crate::settings::load(&r.db);
    s.lidarr_url = "http://10.20.30.40:8686".into();
    crate::settings::save(&r.db, &s).unwrap();
    pc.store(true, Ordering::SeqCst);
    br.store(false, Ordering::SeqCst);
    assert_eq!(order(&r.lidarr), Ok(vec![Computer]));
    pc.store(false, Ordering::SeqCst);
    assert_eq!(order(&r.lidarr), Err("invalid: Add Lidarr's API key first.".to_string()));
}

#[tokio::test]
async fn the_first_route_that_answers_is_used() {
    use Route::*;
    let all = |up: [bool; 4]| -> Vec<Arc<dyn LidarrRoute>> {
        [Home, Away, Computer, Bridge].into_iter().zip(up).map(|(r, u)| Arc::new(Fake(r, u)) as Arc<dyn LidarrRoute>).collect()
    };
    for (up, want) in [
        ([true, true, true, true], Some(Home)),
        ([false, true, true, true], Some(Away)),
        ([false, false, true, true], Some(Computer)),
        ([false, false, false, true], Some(Bridge)),
        ([false, false, false, false], None),
    ] {
        let got = Lidarr::connect(all(up)).await.ok().map(|(c, _, _)| c.route());
        assert_eq!(got, want, "{up:?}");
    }
}

#[test]
fn the_last_check_through_a_home_side_counts_with_no_address_here() {
    let r = rig();
    r.lidarr.set_key(None).unwrap();
    let mut s = crate::settings::load(&r.db);
    s.lidarr_url = String::new();
    crate::settings::save(&r.db, &s).unwrap();
    let (pc, _) = sides(&r.lidarr);
    pc.store(true, Ordering::SeqCst);
    assert_eq!(r.lidarr.check_key(), "homeside");
    // A check that passed through the computer makes Request show.
    let last = super::LastCheck { passed: true, managed_ok: Some(true), checked_at: 1, url: "homeside".into(), roots: vec![] };
    r.db.write(|c| crate::db::store::kv_set(c, "lidarr.lastCheck", &serde_json::to_string(&last).unwrap())).unwrap();
    assert!(r.lidarr.status().ready);
    // The computer off: no longer ready.
    pc.store(false, Ordering::SeqCst);
    assert!(!r.lidarr.status().ready);
}
