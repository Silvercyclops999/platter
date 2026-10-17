//! Playlists with Mix on, and the transition picked for each gap, keyed by
//! playlist id. Turning Mix off removes the playlist (and its picks).

use super::store::{MIXED, Op, SyncStore};
use crate::audio::automix::{MixStyle, Mixed, MixedPlaylist, gap_key};
use crate::error::{AppError, AppResult};

fn check(id: &str) -> AppResult<()> {
    if id.is_empty() || id.len() > 200 {
        return Err(AppError::Invalid("That isn't a playlist.".into()));
    }
    Ok(())
}

pub fn list(store: &SyncStore) -> AppResult<Mixed> {
    Ok(store.list(&MIXED)?.into_iter().collect())
}

/// Mix on or off for a playlist.
pub fn set(store: &SyncStore, id: &str, on: bool) -> AppResult<Mixed> {
    check(id)?;
    let have = store.get(&MIXED, id)?;
    match (on, have) {
        (true, None) => store.write_many(vec![Op::put(&MIXED, id, &MixedPlaylist::default())?])?,
        (false, Some(_)) => {
            store.remove(&MIXED, id)?;
        }
        _ => {}
    }
    list(store)
}

/// The transition between two songs of a mixed playlist (Auto removes the pick).
pub fn set_style(store: &SyncStore, id: &str, from: &str, to: &str, style: MixStyle) -> AppResult<Mixed> {
    check(id)?;
    let mut p = store
        .get(&MIXED, id)?
        .ok_or_else(|| AppError::Invalid("Turn on mix for this playlist first.".into()))?;
    let k = gap_key(from, to);
    if style == MixStyle::Auto {
        p.styles.remove(&k);
    } else {
        p.styles.insert(k, style);
    }
    store.write_many(vec![Op::put(&MIXED, id, &p)?])?;
    list(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::store::SyncStore;

    #[test]
    fn mix_on_off_and_styles_per_gap() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::DbHandle::open(dir.path().join("lib.db")).unwrap();
        let store = SyncStore::new(db, "pc");
        assert!(list(&store).unwrap().is_empty());
        let m = set(&store, "pl1", true).unwrap();
        assert!(m.contains_key("pl1"));
        assert!(set_style(&store, "other", "a", "b", MixStyle::Lift).is_err(), "Mix is off there");
        let m = set_style(&store, "pl1", "a", "b", MixStyle::Echo).unwrap();
        assert_eq!(m["pl1"].style("a", "b"), MixStyle::Echo);
        assert_eq!(m["pl1"].style("b", "c"), MixStyle::Auto);
        let m = set_style(&store, "pl1", "a", "b", MixStyle::Auto).unwrap();
        assert!(m["pl1"].styles.is_empty());
        // Turning it on again keeps the picks; off drops the playlist.
        set_style(&store, "pl1", "x", "y", MixStyle::None).unwrap();
        assert_eq!(set(&store, "pl1", true).unwrap()["pl1"].style("x", "y"), MixStyle::None);
        assert!(set(&store, "pl1", false).unwrap().is_empty());
    }
}
