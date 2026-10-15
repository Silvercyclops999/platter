//! Followed artists in the sync store, behind the social module's
//! [`FollowStore`], so follows travel in the sync file with the rest.
//! Follows kept by an older build (in the database's key-value table) are
//! moved over once.

use super::store::{FOLLOWS, Op, SyncStore};
use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::AppResult;
use crate::social::follows::{FollowStore, FollowedArtist};

/// Where an older build kept the list.
pub const LEGACY_KEY: &str = "social.follows.v1";

pub struct SyncFollows {
    pub store: SyncStore,
}

impl SyncFollows {
    /// Take the older build's list, once: into a store that has never had
    /// a follow, keeping when each was followed. The old copy is emptied so
    /// it can't come back.
    pub fn migrate(store: &SyncStore, db: &DbHandle) -> AppResult<usize> {
        let Some(raw) = db.read(|c| kv_get(c, LEGACY_KEY))? else { return Ok(0) };
        let old: Vec<FollowedArtist> = serde_json::from_str(&raw).unwrap_or_default();
        if old.is_empty() {
            return Ok(0);
        }
        let mut n = 0;
        if !store.ever_used(&FOLLOWS)? {
            for f in &old {
                let ts = (f.followed_at.max(1) as u64) * 1000;
                if store.seed(&FOLLOWS, &f.id, f, ts)? {
                    n += 1;
                }
            }
        }
        db.write(|c| kv_set(c, LEGACY_KEY, "[]"))?;
        Ok(n)
    }
}

impl FollowStore for SyncFollows {
    /// In the order they were followed.
    fn list(&self) -> AppResult<Vec<FollowedArtist>> {
        let mut all: Vec<FollowedArtist> = self.store.list(&FOLLOWS)?.into_iter().map(|(_, f)| f).collect();
        all.sort_by(|a, b| a.followed_at.cmp(&b.followed_at).then_with(|| a.id.cmp(&b.id)));
        Ok(all)
    }

    fn save(&self, all: &[FollowedArtist]) -> AppResult<()> {
        let cur = self.list()?;
        let mut ops = Vec::new();
        for f in all {
            if !cur.contains(f) {
                ops.push(Op::put(&FOLLOWS, &f.id, f)?);
            }
        }
        for f in &cur {
            if !all.iter().any(|x| x.id == f.id) {
                ops.push(Op::remove(&FOLLOWS, &f.id));
            }
        }
        self.store.write_many(ops)
    }
}
