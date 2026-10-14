//! Per-track loudness (from Plex track detail) and measured silence, kept
//! in the tracks table for leveling and smart crossfades.

use rusqlite::{Connection, OptionalExtension, params};

use crate::audio::dsp::TrackAudio;
use crate::error::AppResult;

/// What's stored for a track, and whether Plex's loudness was fetched yet
/// (a fetched track may still have no gain: Plex hasn't analysed it).
pub fn get(c: &Connection, id: &str) -> AppResult<Option<(TrackAudio, bool)>> {
    let row = c
        .query_row(
            "SELECT gain_db, album_gain_db, peak, album_peak, head_silence_ms, tail_silence_ms, loudness_at
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
                    },
                    r.get::<_, Option<i64>>(6)?.is_some(),
                ))
            },
        )
        .optional()?;
    Ok(row)
}

/// Stores Plex's loudness for a track and marks it fetched.
pub fn set_loudness(c: &Connection, id: &str, a: &TrackAudio, now: i64) -> AppResult<()> {
    c.execute(
        "UPDATE tracks SET gain_db = ?2, album_gain_db = ?3, peak = ?4, album_peak = ?5, loudness_at = ?6
         WHERE id = ?1",
        params![
            id,
            a.gain_db.map(f64::from),
            a.album_gain_db.map(f64::from),
            a.peak.map(f64::from),
            a.album_peak.map(f64::from),
            now
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
        assert_eq!(db.read(|c| get(c, "t")).unwrap(), Some((TrackAudio::default(), false)));
        assert_eq!(db.read(|c| get(c, "missing")).unwrap(), None);
        let a = TrackAudio {
            gain_db: Some(-10.93),
            album_gain_db: Some(-9.5),
            peak: Some(1.0157),
            album_peak: None,
            ..Default::default()
        };
        db.write(|c| set_loudness(c, "t", &a, 5)).unwrap();
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
}
