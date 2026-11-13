//! Per-track loudness (from the server's track detail, or measured by
//! Platter while the song played) and measured silence, kept in the tracks
//! table for leveling and smart crossfades.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};

use crate::audio::dsp::{DEFAULT_GUESS_DB, DEFAULT_TARGET_LUFS, TrackAudio};
use crate::error::AppResult;

/// A song the server had no loudness for is asked again after this long
/// (the server may have analysed it since).
const ASK_AGAIN_S: i64 = 6 * 60 * 60;
/// Songs looked at for the library's typical gain and target.
const REF_ROWS: usize = 2000;
/// How long those stay worked out.
const REF_KEEP: Duration = Duration::from_secs(10 * 60);

/// What a song with no loudness from the server is leveled with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelRef {
    /// The level (LUFS) the server's gains bring songs to, as Platter
    /// measures it.
    pub target_lufs: f32,
    /// The median gain of songs the server has loudness for.
    pub typical_db: f32,
}

static REFS: LazyLock<Mutex<HashMap<String, (Instant, LevelRef)>>> = LazyLock::new(Default::default);

fn median(mut v: Vec<f64>) -> Option<f32> {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2] as f32)
}

/// The library's [`LevelRef`], worked out from the most recent songs with
/// loudness and kept a few minutes.
pub fn level_ref(c: &Connection) -> AppResult<LevelRef> {
    let key = c.path().unwrap_or_default().to_string();
    if key.is_empty() {
        return compute_ref(c);
    }
    if let Some((at, r)) = REFS.lock().get(&key)
        && at.elapsed() < REF_KEEP
    {
        return Ok(*r);
    }
    let r = compute_ref(c)?;
    REFS.lock().insert(key, (Instant::now(), r));
    Ok(r)
}

fn compute_ref(c: &Connection) -> AppResult<LevelRef> {
    let col = |sql: &str| -> AppResult<Vec<f64>> {
        let mut st = c.prepare(sql)?;
        let v = st.query_map([REF_ROWS as i64], |r| r.get::<_, f64>(0))?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    };
    let typical = col(
        "SELECT gain_db FROM tracks WHERE gain_db IS NOT NULL
         ORDER BY loudness_at DESC LIMIT ?1",
    )?;
    let target = col(
        "SELECT gain_db + COALESCE(CASE WHEN measured_key = part_key THEN measured_lufs END, server_lufs) AS t
         FROM tracks WHERE gain_db IS NOT NULL AND t IS NOT NULL
         ORDER BY loudness_at DESC LIMIT ?1",
    )?;
    Ok(LevelRef {
        target_lufs: median(target).unwrap_or(DEFAULT_TARGET_LUFS),
        typical_db: median(typical).unwrap_or(DEFAULT_GUESS_DB),
    })
}

fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// What's stored for a track, and whether the server's loudness was
/// fetched (a fetched track may still have no gain: the server hasn't
/// analysed it; that counts as fetched for [`ASK_AGAIN_S`], or for good
/// once Platter measured the song). It always carries the library's
/// target level and typical gain.
pub fn get(c: &Connection, id: &str) -> AppResult<Option<(TrackAudio, bool)>> {
    let row = c
        .query_row(
            "SELECT gain_db, album_gain_db, peak, album_peak, head_silence_ms, tail_silence_ms, loudness_at,
                    server_lufs, CASE WHEN measured_key = part_key THEN measured_lufs END,
                    CASE WHEN measured_key = part_key THEN measured_peak END
             FROM tracks WHERE id = ?1",
            [id],
            |r| {
                let f = |i: usize| r.get::<_, Option<f64>>(i).map(|v| v.map(|v| v as f32));
                let ms = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.and_then(|v| u64::try_from(v).ok()));
                Ok((
                    TrackAudio {
                        gain_db: f(0)?,
                        album_gain_db: f(1)?,
                        peak: f(2)?,
                        album_peak: f(3)?,
                        head_ms: ms(4)?,
                        tail_ms: ms(5)?,
                        server_lufs: f(7)?,
                        measured_lufs: f(8)?,
                        measured_peak: f(9)?,
                        ..Default::default()
                    },
                    r.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .optional()?;
    let Some((mut a, at)) = row else { return Ok(None) };
    let lv = level_ref(c)?;
    a.target_lufs = Some(lv.target_lufs);
    a.guess_db = Some(lv.typical_db);
    let has_gain = a.gain_db.is_some() || a.album_gain_db.is_some() || a.measured_lufs.is_some();
    let fetched = at.is_some_and(|at| has_gain || now_s() - at < ASK_AGAIN_S);
    Ok(Some((a, fetched)))
}

/// Stores the loudness Platter measured over a whole play of the track's
/// current file.
pub fn set_measured(c: &Connection, id: &str, lufs: f32, peak: f32) -> AppResult<()> {
    c.execute(
        "UPDATE tracks SET measured_lufs = ?2, measured_peak = ?3, measured_key = part_key WHERE id = ?1",
        params![id, f64::from(lufs), f64::from(peak)],
    )?;
    Ok(())
}

/// Stores Plex's loudness for a track and marks it fetched.
pub fn set_loudness(c: &Connection, id: &str, a: &TrackAudio, now: i64) -> AppResult<()> {
    c.execute(
        "UPDATE tracks SET gain_db = ?2, album_gain_db = ?3, peak = ?4, album_peak = ?5, loudness_at = ?6,
                           server_lufs = ?7
         WHERE id = ?1",
        params![
            id,
            a.gain_db.map(f64::from),
            a.album_gain_db.map(f64::from),
            a.peak.map(f64::from),
            a.album_peak.map(f64::from),
            now,
            a.server_lufs.map(f64::from)
        ],
    )?;
    Ok(())
}

/// Stores a track's measured quiet lead-in and run-out.
pub fn set_silence(c: &Connection, id: &str, head_ms: u64, tail_ms: u64) -> AppResult<()> {
    c.execute(
        "UPDATE tracks SET head_silence_ms = ?2, tail_silence_ms = ?3 WHERE id = ?1",
        params![id, head_ms as i64, tail_ms as i64],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbHandle;

    #[test]
    fn loudness_and_silence_round_trip_and_survive_a_sync_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("t.db")).unwrap();
        db.write(|c| {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key)
                 VALUES ('t', 'T', 'a', 'A', 'r', 'R', '/p')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let fresh = TrackAudio {
            guess_db: Some(DEFAULT_GUESS_DB),
            target_lufs: Some(DEFAULT_TARGET_LUFS),
            ..Default::default()
        };
        assert_eq!(db.read(|c| get(c, "t")).unwrap(), Some((fresh, false)));
        assert_eq!(db.read(|c| get(c, "missing")).unwrap(), None);
        let a = TrackAudio {
            gain_db: Some(-10.93),
            album_gain_db: Some(-9.5),
            peak: Some(1.0157),
            album_peak: None,
            ..Default::default()
        };
        db.write(|c| set_loudness(c, "t", &a, now_s())).unwrap();
        db.write(|c| set_silence(c, "t", 1500, 2300)).unwrap();
        // A sync rewriting the track keeps them.
        let mut t = crate::audio::testutil::track("t", "R", "/p");
        t.title = "Renamed".into();
        db.write(|c| {
            let tx = c.transaction()?;
            crate::db::store::upsert_tracks(&tx, &[t], 2)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let (got, fetched) = db.read(|c| get(c, "t")).unwrap().unwrap();
        assert!(fetched);
        assert!((got.gain_db.unwrap() + 10.93).abs() < 1e-4);
        assert_eq!(got.album_peak, None);
        assert_eq!((got.head_ms, got.tail_ms), (Some(1500), Some(2300)));
    }

    fn db_with(ids: &[(&str, &str)]) -> (tempfile::TempDir, DbHandle) {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("t.db")).unwrap();
        db.write(|c| {
            for (id, part) in ids {
                c.execute(
                    "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key)
                     VALUES (?1, 'T', 'a', 'A', 'r', 'R', ?2)",
                    [id, part],
                )?;
            }
            Ok(())
        })
        .unwrap();
        (dir, db)
    }

    #[test]
    fn measured_loudness_is_kept_for_the_file_it_was_measured_on() {
        let (_d, db) = db_with(&[("t", "/p")]);
        db.write(|c| set_measured(c, "t", -9.5, 0.9)).unwrap();
        let (a, fetched) = db.read(|c| get(c, "t")).unwrap().unwrap();
        assert_eq!((a.measured_lufs, a.measured_peak), (Some(-9.5), Some(0.9)));
        assert!(!fetched, "never fetched from the server yet");
        // Fetched long ago with nothing from the server: a measured song
        // isn't asked again.
        db.write(|c| set_loudness(c, "t", &TrackAudio::default(), 1)).unwrap();
        assert!(db.read(|c| get(c, "t")).unwrap().unwrap().1);
        // The file was replaced: the old measurement doesn't count.
        db.write(|c| {
            c.execute("UPDATE tracks SET part_key = '/q' WHERE id = 't'", [])?;
            Ok(())
        })
        .unwrap();
        let (a, _) = db.read(|c| get(c, "t")).unwrap().unwrap();
        assert_eq!((a.measured_lufs, a.measured_peak), (None, None));
    }

    #[test]
    fn a_song_the_server_had_no_loudness_for_is_asked_again_later() {
        let (_d, db) = db_with(&[("t", "/p")]);
        db.write(|c| set_loudness(c, "t", &TrackAudio::default(), now_s() - 60)).unwrap();
        assert!(db.read(|c| get(c, "t")).unwrap().unwrap().1);
        db.write(|c| set_loudness(c, "t", &TrackAudio::default(), now_s() - ASK_AGAIN_S - 1)).unwrap();
        assert!(!db.read(|c| get(c, "t")).unwrap().unwrap().1);
    }

    #[test]
    fn server_loudness_round_trips_and_sets_the_target_and_typical_gain() {
        let (_d, db) = db_with(&[("a", "/a"), ("b", "/b"), ("c", "/c")]);
        // Album gain -11 for all three; loudness -8, -10, -12.
        for (id, l) in [("a", -8.0), ("b", -10.0), ("c", -12.0)] {
            let s = TrackAudio {
                gain_db: Some(-11.0),
                album_gain_db: Some(-11.0),
                server_lufs: Some(l),
                ..Default::default()
            };
            db.write(|c| set_loudness(c, id, &s, now_s())).unwrap();
        }
        let r = db.read(compute_ref).unwrap();
        assert!((r.target_lufs + 21.0).abs() < 1e-4, "{r:?}");
        assert!((r.typical_db + 11.0).abs() < 1e-4, "{r:?}");
        let (a, _) = db.read(|c| get(c, "a")).unwrap().unwrap();
        assert_eq!(a.server_lufs, Some(-8.0));
        assert!(a.target_lufs.is_some() && a.guess_db.is_some());
        // Nothing known: the defaults.
        let (_d2, empty) = db_with(&[]);
        let r = empty.read(compute_ref).unwrap();
        assert_eq!((r.target_lufs, r.typical_db), (DEFAULT_TARGET_LUFS, DEFAULT_GUESS_DB));
    }
}
