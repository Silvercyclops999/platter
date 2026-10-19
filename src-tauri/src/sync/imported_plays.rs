//! Plays brought in from elsewhere (Plex history, Tautulli, Last.fm, a
//! listening history file) on one device count on every paired device, so
//! stats and history read the same everywhere. A day of one source's plays
//! is one entry ([`IMPORTED_PLAYS`], keyed `<source>:<day>:<part>`, at most
//! [`PART`] plays each). Plays are only ever added: a play is the same play
//! on every device (its source and id), so taking one twice adds nothing.
//!
//! - [`publish`] stores the days whose plays differ from the store.
//! - [`apply`] adds plays from other devices, matched to this device's
//!   library later like any imported play. Running [`publish`] after it
//!   shares the union, so two devices that imported different parts of a
//!   history both end with all of it.
//!
//! Between paired devices only: years of history are too big for the sync
//! file (`Groups::file_namespaces`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::store::{IMPORTED_PLAYS, Op, SyncStore};
use crate::db::listens::{ImportSource, ImportedPlay, insert_imported};
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};

/// Plays in one entry at most.
pub const PART: usize = 250;
const DAY_MS: i64 = 86_400_000;
/// Which entries were taken here: key to (time, device).
const APPLIED_KEY: &str = "sync.plays.applied";

/// One source's plays on one day (or part of it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayDay {
    pub plays: Vec<SyncedPlay>,
}

/// One play, as it travels (the library match is left behind: each device
/// matches against its own library).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncedPlay {
    /// Its id in its source.
    pub id: String,
    /// Unix ms.
    pub at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
}

fn key(source: &str, day: i64, part: usize) -> String {
    format!("{source}:{day}:{part}")
}

fn source_of(key: &str) -> Option<ImportSource> {
    ImportSource::parse(key.split(':').next()?)
}

/// Every imported play here as entries.
fn all_days(store: &SyncStore) -> AppResult<BTreeMap<String, PlayDay>> {
    let rows: Vec<(String, SyncedPlay)> = store.db().read(|c| {
        let mut st = c.prepare(
            "SELECT source, source_id, played_at, ms_played, title, artist, album FROM imported_plays
             ORDER BY source, played_at, source_id",
        )?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    SyncedPlay {
                        id: r.get(1)?,
                        at: r.get(2)?,
                        ms: r.get::<_, Option<i64>>(3)?.map(|v| v.max(0) as u64),
                        title: r.get(4)?,
                        artist: r.get(5)?,
                        album: r.get(6)?,
                    },
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;
    let mut days: BTreeMap<(String, i64), Vec<SyncedPlay>> = BTreeMap::new();
    for (source, p) in rows {
        if ImportSource::parse(&source).is_none() || p.id.is_empty() || p.id.len() > 200 {
            continue;
        }
        days.entry((source, p.at.div_euclid(DAY_MS))).or_default().push(p);
    }
    let mut out = BTreeMap::new();
    for ((source, day), plays) in days {
        for (i, chunk) in plays.chunks(PART).enumerate() {
            out.insert(key(&source, day, i), PlayDay { plays: chunk.to_vec() });
        }
    }
    Ok(out)
}

/// Store the days whose imported plays differ from the store. Returns how
/// many entries changed.
pub fn publish(store: &SyncStore) -> AppResult<usize> {
    let mut ops = Vec::new();
    for (k, day) in all_days(store)? {
        if store.get(&IMPORTED_PLAYS, &k)?.as_ref() != Some(&day) {
            ops.push(Op::put(&IMPORTED_PLAYS, k, &day)?);
        }
    }
    let n = ops.len();
    // In a few transactions, so a long history doesn't hold the database.
    let mut ops = ops.into_iter().peekable();
    while ops.peek().is_some() {
        store.write_many(ops.by_ref().take(500).collect())?;
    }
    Ok(n)
}

type Applied = BTreeMap<String, (u64, String)>;

fn applied(store: &SyncStore) -> Applied {
    store.db().read(|c| kv_get(c, APPLIED_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Add the plays other devices brought in that this one doesn't have yet.
/// Entries taken before are skipped. Returns how many plays were new.
pub fn apply(store: &SyncStore, now_ms: i64) -> AppResult<usize> {
    let mut done = applied(store);
    let entries = store.entries(&[IMPORTED_PLAYS.name()])?;
    let Some(entries) = entries.get(IMPORTED_PLAYS.name()) else { return Ok(0) };
    let mut plays = Vec::new();
    let mut taken = Vec::new();
    for (k, e) in entries {
        let stamp = (e.ts, e.device.clone());
        if done.get(k) == Some(&stamp) {
            continue;
        }
        taken.push((k.clone(), stamp));
        let (Some(source), Some(v)) = (source_of(k), e.value.as_ref()) else { continue };
        let Ok(day) = serde_json::from_value::<PlayDay>(v.clone()) else { continue };
        for p in day.plays.into_iter().take(PART) {
            let title = p.title.trim().to_string();
            if title.is_empty() || p.id.is_empty() {
                continue;
            }
            plays.push(ImportedPlay {
                source,
                source_id: p.id,
                played_at: p.at,
                ms_played: p.ms,
                title,
                artist: p.artist.filter(|a| !a.trim().is_empty()),
                album: p.album.filter(|a| !a.trim().is_empty()),
                rating_key: None,
            });
        }
    }
    if taken.is_empty() {
        return Ok(0);
    }
    let added = store.db().write(|c| insert_imported(c, &plays, now_ms))?;
    done.extend(taken);
    let json = serde_json::to_string(&done).map_err(|e| AppError::Internal(e.to_string()))?;
    store.db().write(|c| kv_set(c, APPLIED_KEY, &json))?;
    if added > 0 {
        tracing::info!(added, "plays imported on another device were added");
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbHandle;

    fn play(source: ImportSource, id: &str, at: i64, title: &str) -> ImportedPlay {
        ImportedPlay { source, source_id: id.into(), played_at: at, ms_played: Some(200_000), title: title.into(), artist: Some("Low".into()), album: None, rating_key: Some("123".into()) }
    }

    fn count(s: &SyncStore) -> i64 {
        s.db().read(|c| Ok(c.query_row("SELECT COUNT(*) FROM imported_plays", [], |r| r.get(0))?)).unwrap()
    }

    fn carry(from: &SyncStore, to: &SyncStore) {
        let e = from.entries(&[IMPORTED_PLAYS.name()]).unwrap();
        to.merge(&e, &[IMPORTED_PLAYS.name()]).unwrap();
    }

    #[test]
    fn plays_imported_on_one_device_count_on_the_other_once() {
        let tmp = tempfile::tempdir().unwrap();
        let s = |id: &str| SyncStore::new(DbHandle::open(tmp.path().join(format!("{id}.db"))).unwrap(), id);
        let (pc, phone) = (s("pc"), s("phone"));
        let day = 20_000 * DAY_MS;
        let mut plays: Vec<ImportedPlay> = (0..300).map(|i| play(ImportSource::Lastfm, &format!("lf{i}"), day + i * 1000, "Lullaby")).collect();
        plays.push(play(ImportSource::PlexHistory, "px1", day + 5, "Sunflower"));
        pc.db().write(|c| insert_imported(c, &plays, 1)).unwrap();
        assert_eq!(publish(&pc).unwrap(), 3, "a day of Last.fm in two parts, and one of Plex history");
        assert_eq!(publish(&pc).unwrap(), 0);
        carry(&pc, &phone);
        assert_eq!(apply(&phone, 2).unwrap(), 301);
        assert_eq!(count(&phone), 301);
        assert_eq!(apply(&phone, 3).unwrap(), 0, "taken once");
        // The library match stays behind: the phone matches against its own.
        let unmatched: i64 = phone.db().read(|c| Ok(c.query_row("SELECT COUNT(*) FROM imported_plays WHERE rating_key IS NULL", [], |r| r.get(0))?)).unwrap();
        assert_eq!(unmatched, 301);
        // Again from scratch (a lost record of what was taken): still once.
        phone.db().write(|c| kv_set(c, APPLIED_KEY, "{}")).unwrap();
        assert_eq!(apply(&phone, 4).unwrap(), 0);
        assert_eq!(count(&phone), 301);
        assert_eq!(publish(&phone).unwrap(), 0, "the same plays make the same entries");
    }

    #[test]
    fn two_devices_with_different_parts_of_a_history_both_end_with_all_of_it() {
        let tmp = tempfile::tempdir().unwrap();
        let s = |id: &str| SyncStore::new(DbHandle::open(tmp.path().join(format!("{id}.db"))).unwrap(), id);
        let (pc, mac) = (s("pc"), s("mac"));
        let day = 20_001 * DAY_MS;
        pc.db().write(|c| insert_imported(c, &[play(ImportSource::Lastfm, "a", day + 1, "A")], 1)).unwrap();
        mac.db().write(|c| insert_imported(c, &[play(ImportSource::Lastfm, "b", day + 2, "B")], 1)).unwrap();
        publish(&pc).unwrap();
        publish(&mac).unwrap();
        // The same day's entry from each: one wins, the other side adds the
        // winner's plays and shares the union.
        carry(&pc, &mac);
        carry(&mac, &pc);
        for _ in 0..2 {
            apply(&pc, 5).unwrap();
            apply(&mac, 5).unwrap();
            publish(&pc).unwrap();
            publish(&mac).unwrap();
            carry(&pc, &mac);
            carry(&mac, &pc);
        }
        apply(&pc, 6).unwrap();
        apply(&mac, 6).unwrap();
        assert_eq!((count(&pc), count(&mac)), (2, 2));
    }

    #[test]
    fn a_damaged_or_odd_entry_adds_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let s = SyncStore::new(DbHandle::open(tmp.path().join("p.db")).unwrap(), "phone");
        let p = |id: &str, title: &str| SyncedPlay { id: id.into(), at: 1, ms: None, title: title.into(), artist: None, album: None };
        s.put(&IMPORTED_PLAYS, "nowhere:1:0", &PlayDay { plays: vec![p("x", "X")] }).unwrap();
        s.put(&IMPORTED_PLAYS, "lastfm:1:0", &PlayDay { plays: vec![p("", "No id"), p("y", "  ")] }).unwrap();
        assert_eq!(apply(&s, 1).unwrap(), 0);
        assert_eq!(count(&s), 0);
    }
}
