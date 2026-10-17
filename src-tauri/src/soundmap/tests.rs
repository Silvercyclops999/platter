//! The projection on made-up clusters, updates as songs are analysed, and
//! reading the map back in chunks and areas.

use rusqlite::params;

use super::project::{self, DIMS};
use super::store::{self, Area};
use super::update;
use crate::db::DbHandle;

/// A small steady random source, so the tests don't depend on luck.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 10_000) as f32 / 10_000.0
    }
}

/// `n` vectors around each of three centres that differ in tempo, energy,
/// brightness and onsets, with the pitch classes all over the place.
fn clusters(n: usize) -> Vec<(usize, Vec<f32>)> {
    let centres: [[f32; 4]; 3] = [[0.2, 0.2, 0.2, 0.2], [0.8, 0.8, 0.3, 0.7], [0.4, 0.6, 0.9, 0.3]];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut out = Vec::new();
    for (k, c) in centres.iter().enumerate() {
        for _ in 0..n {
            let mut v = vec![0.0f32; DIMS];
            v[0] = c[0] + (rng.next() - 0.5) * 0.08;
            v[1] = c[1] + (rng.next() - 0.5) * 0.08;
            v[4] = c[2] + (rng.next() - 0.5) * 0.08;
            v[3] = c[3] + (rng.next() - 0.5) * 0.08;
            for x in v.iter_mut().skip(8) {
                *x = rng.next();
            }
            v[2] = 0.5 + (rng.next() - 0.5) * 0.1;
            out.push((k, v));
        }
    }
    out
}

#[test]
fn near_songs_stay_near_on_the_map() {
    let data = clusters(60);
    let vectors: Vec<&[f32]> = data.iter().map(|(_, v)| v.as_slice()).collect();
    let b = project::fit(&vectors, None).unwrap();
    let pts: Vec<(f32, f32)> = data.iter().map(|(_, v)| project::place(&b, v)).collect();
    // Each point's nearest neighbour on the map is from its own cluster.
    let mut same = 0;
    for i in 0..pts.len() {
        let mut best = (f32::MAX, 0);
        for j in 0..pts.len() {
            if i == j {
                continue;
            }
            let d = (pts[i].0 - pts[j].0).powi(2) + (pts[i].1 - pts[j].1).powi(2);
            if d < best.0 {
                best = (d, j);
            }
        }
        if data[best.1].0 == data[i].0 {
            same += 1;
        }
    }
    assert!(same as f32 >= pts.len() as f32 * 0.97, "{same} of {}", pts.len());
    // And the clusters sit apart: centres further apart than their spread.
    let centre = |k: usize| {
        let p: Vec<_> = data.iter().zip(&pts).filter(|((c, _), _)| *c == k).map(|(_, p)| *p).collect();
        let n = p.len() as f32;
        let (x, y) = p.iter().fold((0.0, 0.0), |a, q| (a.0 + q.0, a.1 + q.1));
        let (cx, cy) = (x / n, y / n);
        let spread = p.iter().map(|q| ((q.0 - cx).powi(2) + (q.1 - cy).powi(2)).sqrt()).fold(0.0f32, f32::max);
        ((cx, cy), spread)
    };
    let cs: Vec<_> = (0..3).map(centre).collect();
    for i in 0..3 {
        for j in i + 1..3 {
            let d = ((cs[i].0.0 - cs[j].0.0).powi(2) + (cs[i].0.1 - cs[j].0.1).powi(2)).sqrt();
            assert!(d > cs[i].1.max(cs[j].1), "clusters {i} and {j} overlap: {d}");
        }
    }
    // Most of the map is within -1 to 1.
    let inside = pts.iter().filter(|p| p.0.abs() <= 1.0 && p.1.abs() <= 1.0).count();
    assert!(inside as f32 >= pts.len() as f32 * 0.9);
}

#[test]
fn the_same_songs_give_the_same_map() {
    let data = clusters(20);
    let vectors: Vec<&[f32]> = data.iter().map(|(_, v)| v.as_slice()).collect();
    assert_eq!(project::fit(&vectors, None), project::fit(&vectors, None));
}

#[test]
fn odd_input_doesnt_break_the_projection() {
    assert!(project::fit(&[], None).is_none());
    let one = vec![0.5f32; DIMS];
    let b = project::fit(&[one.as_slice()], None).unwrap();
    let (x, y) = project::place(&b, &one);
    assert!(x.is_finite() && y.is_finite());
    assert!(!project::usable(&[0.1; 3]));
    let mut nan = vec![0.5f32; DIMS];
    nan[3] = f32::NAN;
    assert!(!project::usable(&nan));
}

#[test]
fn the_projection_is_redone_when_the_library_grows_by_half() {
    let b = |count| project::Basis { mean: vec![], scale: vec![], axes: [vec![], vec![]], spread: 1.0, count };
    assert!(project::needs_refit(None, project::LAYOUT_VERSION, 1));
    assert!(!project::needs_refit(None, project::LAYOUT_VERSION, 0));
    assert!(!project::needs_refit(Some(&b(10)), project::LAYOUT_VERSION, 14));
    assert!(project::needs_refit(Some(&b(10)), project::LAYOUT_VERSION, 15));
    assert!(project::needs_refit(Some(&b(1)), project::LAYOUT_VERSION, 2));
    assert!(project::needs_refit(Some(&b(100)), project::LAYOUT_VERSION - 1, 100), "an older projection");
}

// ------------------------------------------------------------ the database

fn db() -> (tempfile::TempDir, DbHandle) {
    let dir = tempfile::tempdir().unwrap();
    let db = DbHandle::open(dir.path().join("lib.db")).unwrap();
    db.write(|c| {
        c.execute_batch(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, year, format, subformats, genres)
                 VALUES ('al', 'Album', 'Album', 'ar', 'Artist', 2016, 'album', '[]', '[\"Soul\"]');
             INSERT INTO albums (id, title, sort_title, artist_id, artist_title, year, format, subformats, genres)
                 VALUES ('demos', 'Demos', 'Demos', 'ar2', 'Other Band', 2019, 'demo', '[]', '[]');",
        )?;
        Ok(())
    })
    .unwrap();
    (dir, db)
}

/// A song on `album` with an analysis made from `v`.
fn analysed(db: &DbHandle, id: &str, album: &str, v: &[f32]) {
    let (id, album, f) = (id.to_string(), album.to_string(), serde_json::to_string(v).unwrap());
    db.write(move |c| {
        let (artist_id, artist) = if album == "al" { ("ar", "Artist") } else { ("ar2", "Other Band") };
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key)
             VALUES (?1, 'Song ' || ?1, ?2, ?2, ?3, ?4, 200000, ?5)",
            params![id, album, artist_id, artist, format!("/p/{id}")],
        )?;
        c.execute(
            "INSERT INTO track_analysis (track_id, part_key, status, version, features, analysed_at) VALUES (?1, ?2, 'ok', 1, ?3, 1)",
            params![id, format!("/p/{id}"), f],
        )?;
        Ok(())
    })
    .unwrap();
}

fn all_points(db: &DbHandle) -> Vec<(String, f32, f32)> {
    let c = db.read(|c| store::points(c, None, 10_000, None)).unwrap();
    c.ids.into_iter().zip(c.x).zip(c.y).map(|((id, x), y)| (id, x, y)).collect()
}

#[test]
fn songs_analysed_later_are_placed_without_moving_the_rest() {
    let (_d, db) = db();
    let data = clusters(4);
    for (i, (_, v)) in data.iter().enumerate() {
        analysed(&db, &format!("t{i:02}"), "al", v);
    }
    let u = update(&db).unwrap();
    assert!(u.refit);
    assert_eq!(u.placed, 12);
    let before = all_points(&db);
    assert_eq!(before.len(), 12);
    // Nothing new: nothing to do.
    assert!(!update(&db).unwrap().changed());

    // A few more (under half again): placed with the same projection.
    let more = clusters(1);
    for (i, (_, v)) in more.iter().enumerate() {
        analysed(&db, &format!("u{i}"), "al", v);
    }
    let u = update(&db).unwrap();
    assert!(!u.refit);
    assert_eq!(u.placed, 3);
    let after = all_points(&db);
    assert_eq!(after.len(), 15);
    for p in &before {
        assert!(after.contains(p), "{} moved", p.0);
    }

    // A changed file drops its point until it's analysed again.
    db.write(|c| {
        c.execute("UPDATE tracks SET part_key = '/p/new' WHERE id = 't00'", [])?;
        Ok(())
    })
    .unwrap();
    let u = update(&db).unwrap();
    assert_eq!(u.removed, 1);
    assert!(!all_points(&db).iter().any(|p| p.0 == "t00"));

    // Grown by half: the whole map is worked out again.
    let lots = clusters(3);
    for (i, (_, v)) in lots.iter().enumerate() {
        analysed(&db, &format!("v{i}"), "al", v);
    }
    let u = update(&db).unwrap();
    assert!(u.refit);
    assert_eq!(all_points(&db).len(), 23);
    let counts = db.read(store::counts).unwrap();
    assert_eq!((counts.placed, counts.analysed, counts.total), (23, 23, 24));
}

#[test]
fn failed_or_odd_analyses_stay_off_the_map() {
    let (_d, db) = db();
    analysed(&db, "ok", "al", &[0.5; DIMS]);
    analysed(&db, "short", "al", &[0.5; 4]);
    db.write(|c| {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key)
             VALUES ('bad', 'Bad', 'al', 'Album', 'ar', 'Artist', 200000, '/p/bad')",
            [],
        )?;
        c.execute("INSERT INTO track_analysis (track_id, part_key, status, version, analysed_at) VALUES ('bad', '/p/bad', 'failed', 1, 1)", [])?;
        Ok(())
    })
    .unwrap();
    update(&db).unwrap();
    let ids: Vec<String> = all_points(&db).into_iter().map(|p| p.0).collect();
    assert_eq!(ids, vec!["ok".to_string()]);
}

#[test]
fn the_map_comes_back_in_chunks_and_areas() {
    let (_d, db) = db();
    let data = clusters(10);
    for (i, (_, v)) in data.iter().enumerate() {
        analysed(&db, &format!("t{i:02}"), if i.is_multiple_of(10) { "demos" } else { "al" }, v);
    }
    update(&db).unwrap();
    let all = all_points(&db);
    assert_eq!(all.len(), 30);

    // Chunks of 7 cover every song once.
    let mut seen = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let c = db.read(|c| store::points(c, after.as_deref(), 7, None)).unwrap();
        assert!(c.ids.len() <= 7);
        assert_eq!((c.x.len(), c.y.len(), c.genre.len(), c.year.len(), c.unreleased.len()), (c.ids.len(), c.ids.len(), c.ids.len(), c.ids.len(), c.ids.len()));
        seen.extend(c.ids.clone());
        match c.next {
            Some(n) => after = Some(n),
            None => break,
        }
    }
    assert_eq!(seen.len(), 30);
    seen.dedup();
    assert_eq!(seen.len(), 30);

    // An area holds only the points inside it.
    let area = Area { min_x: -0.2, min_y: -2.0, max_x: 2.0, max_y: 2.0 };
    let c = db.read(move |c| store::points(c, None, 10_000, Some(area))).unwrap();
    let want: Vec<&String> = all.iter().filter(|p| p.1 >= -0.2 && p.1 <= 2.0 && p.2 >= -2.0 && p.2 <= 2.0).map(|p| &p.0).collect();
    assert!(!want.is_empty() && want.len() < 30);
    assert_eq!(c.ids.iter().collect::<Vec<_>>(), want);

    // Genre, year and the demo album's songs marked unreleased.
    let c = db.read(|c| store::points(c, None, 10_000, None)).unwrap();
    let i = c.ids.iter().position(|id| id == "t00").unwrap();
    assert!(c.unreleased[i]);
    assert_eq!((c.year[i], c.genre[i]), (2019, -1));
    let j = c.ids.iter().position(|id| id == "t01").unwrap();
    assert!(!c.unreleased[j]);
    assert_eq!(c.year[j], 2016);
    assert_eq!(c.genres[c.genre[j] as usize], "Soul");
}

#[test]
fn songs_and_artists_are_found_on_the_map() {
    let (_d, db) = db();
    for (i, (_, v)) in clusters(2).iter().enumerate() {
        analysed(&db, &format!("t{i}"), if i < 2 { "demos" } else { "al" }, v);
    }
    update(&db).unwrap();
    let found = db.read(|c| store::find(c, "other")).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].kind, found[0].id.as_str(), found[0].sub.as_str()), ("artist", "ar2", "2 songs"));
    let found = db.read(|c| store::find(c, "song t3")).unwrap();
    assert_eq!(found.iter().map(|f| (f.kind, f.id.as_str())).collect::<Vec<_>>(), vec![("song", "t3")]);
    assert!(db.read(|c| store::find(c, "  ")).unwrap().is_empty());
    assert!(db.read(|c| store::find(c, "%")).unwrap().is_empty(), "wildcards are plain text");
    let s = db.read(|c| store::song(c, "t3")).unwrap().unwrap();
    assert_eq!((s.title.as_str(), s.artist.as_str()), ("Song t3", "Artist"));
}

// ------------------------------------------------------------ review fixes

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
}

#[test]
fn working_the_projection_out_again_keeps_the_maps_way_round() {
    let data = clusters(30);
    let vectors: Vec<&[f32]> = data.iter().map(|(_, v)| v.as_slice()).collect();
    let first = project::fit(&vectors, None).unwrap();
    let neg = |v: &[f64]| v.iter().map(|x| -x).collect::<Vec<_>>();
    // The map in use is mirrored on its first axis: a refit follows it.
    let mut mirrored = first.clone();
    mirrored.axes[0] = neg(&first.axes[0]);
    let b = project::fit(&vectors, Some(&mirrored)).unwrap();
    assert!(close(&b.axes[0], &mirrored.axes[0]) && close(&b.axes[1], &first.axes[1]));
    // Turned a quarter: the axes swap to match as well as flip.
    let mut turned = first.clone();
    turned.axes = [neg(&first.axes[1]), first.axes[0].clone()];
    let b = project::fit(&vectors, Some(&turned)).unwrap();
    assert!(close(&b.axes[0], &turned.axes[0]) && close(&b.axes[1], &turned.axes[1]));
    // Already the same way round: nothing changes.
    assert_eq!(project::fit(&vectors, Some(&first)).unwrap(), first);
}

#[test]
fn a_stored_map_that_was_mirrored_stays_mirrored_after_a_refit() {
    let (_d, db) = db();
    for (i, (_, v)) in clusters(4).iter().enumerate() {
        analysed(&db, &format!("t{i:02}"), "al", v);
    }
    update(&db).unwrap();
    let before = all_points(&db);
    // Store the projection mirrored on both axes, from an older version so it's redone.
    db.write(|c| {
        let (mut b, _) = store::basis(c)?.unwrap();
        for a in b.axes.iter_mut() {
            a.iter_mut().for_each(|x| *x = -*x);
        }
        c.execute(
            "UPDATE sound_map_layout SET basis = ?1, version = ?2",
            params![serde_json::to_string(&b).unwrap(), project::LAYOUT_VERSION - 1],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(update(&db).unwrap().refit);
    let after = all_points(&db);
    assert_eq!(before.len(), after.len());
    for (p, q) in before.iter().zip(&after) {
        assert_eq!(p.0, q.0);
        assert!((p.1 + q.1).abs() < 1e-4 && (p.2 + q.2).abs() < 1e-4, "{p:?} then {q:?}");
    }
}

#[test]
fn the_fit_one_vector_at_a_time_matches_the_spread_worked_out_directly() {
    let data = clusters(10);
    let vectors: Vec<&[f32]> = data.iter().map(|(_, v)| v.as_slice()).collect();
    let b = project::fit(&vectors, None).unwrap();
    let n = vectors.len() as f64;
    for i in 0..DIMS {
        let mean = vectors.iter().map(|v| v[i] as f64).sum::<f64>() / n;
        let sd = (vectors.iter().map(|v| (v[i] as f64 - mean).powi(2)).sum::<f64>() / n).sqrt().max(0.02);
        assert!((b.mean[i] - mean).abs() < 1e-9 && (b.scale[i] - sd).abs() < 1e-9, "feature {i}");
    }
}

#[test]
fn a_feature_far_outside_what_the_map_was_made_from_cant_throw_a_point_away() {
    let data = clusters(20);
    let vectors: Vec<&[f32]> = data.iter().map(|(_, v)| v.as_slice()).collect();
    let b = project::fit(&vectors, None).unwrap();
    let mut odd = data[0].1.clone();
    odd[2] = 50.0;
    let far = project::place(&b, &odd);
    odd[2] = 5000.0;
    assert_eq!(project::place(&b, &odd), far, "both are held at the limit");
    let bound = (project::CLAMP * 1.4 * (DIMS as f64).sqrt() / (3.0 * b.spread)) as f32;
    assert!(far.0.abs() <= bound && far.1.abs() <= bound);
}

#[test]
fn unusable_analyses_are_marked_so_the_map_settles() {
    let (_d, db) = db();
    for (i, (_, v)) in clusters(1).iter().enumerate() {
        analysed(&db, &format!("t{i}"), "al", v);
    }
    analysed(&db, "short", "al", &[0.5; 4]);
    analysed(&db, "corrupt", "al", &[0.5; DIMS]);
    db.write(|c| {
        c.execute("UPDATE track_analysis SET features = 'not json' WHERE track_id = 'corrupt'", [])?;
        Ok(())
    })
    .unwrap();
    let u = update(&db).unwrap();
    assert_eq!((u.placed, u.skipped), (3, 2));
    let counts = db.read(store::counts).unwrap();
    assert_eq!((counts.placed, counts.analysed), (3, 3), "nothing left looking unplaced");
    assert!(!update(&db).unwrap().changed(), "settled");

    // Analysed again (readable this time): tried again and placed.
    let good = serde_json::to_string(&[0.4f32; DIMS]).unwrap();
    db.write(move |c| {
        c.execute("UPDATE track_analysis SET features = ?1, analysed_at = 2 WHERE track_id = 'corrupt'", [good])?;
        Ok(())
    })
    .unwrap();
    let u = update(&db).unwrap();
    assert_eq!((u.placed, u.skipped), (1, 0));
    assert!(all_points(&db).iter().any(|p| p.0 == "corrupt"));
    let counts = db.read(store::counts).unwrap();
    assert_eq!((counts.placed, counts.analysed), (4, 4));
}

#[test]
fn a_corrupt_stored_projection_is_worked_out_again() {
    let (_d, db) = db();
    for (i, (_, v)) in clusters(2).iter().enumerate() {
        analysed(&db, &format!("t{i}"), "al", v);
    }
    update(&db).unwrap();
    db.write(|c| {
        c.execute("UPDATE sound_map_layout SET basis = '{oops'", [])?;
        Ok(())
    })
    .unwrap();
    assert!(db.read(store::basis).unwrap().is_none());
    assert!(update(&db).unwrap().refit);
    assert!(db.read(store::basis).unwrap().is_some());
    // The wrong shape counts as corrupt too (placing with it would go out of bounds).
    db.write(|c| {
        c.execute(r#"UPDATE sound_map_layout SET basis = '{"mean":[],"scale":[],"axes":[[],[]],"spread":1.0,"count":6}'"#, [])?;
        Ok(())
    })
    .unwrap();
    assert!(db.read(store::basis).unwrap().is_none());
}

#[test]
fn a_big_layout_is_written_in_chunks_and_all_of_it_lands() {
    let (_d, db) = db();
    let n = store::WRITE_CHUNK + 7;
    let mut rng = Rng(42);
    db.write(|c| {
        let tx = c.transaction()?;
        for i in 0..n {
            let id = format!("t{i:05}");
            let v: Vec<f32> = (0..DIMS).map(|_| rng.next()).collect();
            tx.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, duration_ms, part_key)
                 VALUES (?1, ?1, 'al', 'Album', 'ar', 'Artist', 200000, ?1)",
                [&id],
            )?;
            tx.execute(
                "INSERT INTO track_analysis (track_id, part_key, status, version, features, analysed_at) VALUES (?1, ?1, 'ok', 1, ?2, 1)",
                params![id, serde_json::to_string(&v).unwrap()],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let u = update(&db).unwrap();
    assert!(u.refit);
    assert_eq!(u.placed, n);
    assert_eq!(db.read(store::counts).unwrap().placed as usize, n);
}

#[tokio::test]
async fn a_panicking_job_doesnt_stop_the_worker() {
    let w = super::Worker::start();
    let r: crate::error::AppResult<()> = w.run(|| panic!("boom")).await;
    assert!(r.unwrap_err().to_string().contains("boom"));
    assert_eq!(w.run(|| 7).await.unwrap(), 7);
}

#[test]
fn building_is_counted_so_overlapping_updates_dont_clear_each_other() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let n = AtomicUsize::new(0);
    let a = super::Building::start(&n);
    let b = super::Building::start(&n);
    drop(a);
    assert_eq!(n.load(Ordering::Acquire), 1, "the other update is still running");
    drop(b);
    assert_eq!(n.load(Ordering::Acquire), 0);
    // Let go even when the update panics.
    let r = std::panic::catch_unwind(|| {
        let _g = super::Building::start(&n);
        panic!("mid update");
    });
    assert!(r.is_err());
    assert_eq!(n.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn refresh_says_its_done_building_afterwards() {
    let (_d, db) = db();
    analysed(&db, "t0", "al", &[0.5; DIMS]);
    let map = super::SoundMap::new(db);
    assert!(!map.building());
    assert_eq!(map.refresh().await.unwrap().placed, 1);
    assert!(!map.building());
}
