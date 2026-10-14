//! "Your server is up, but your music files aren't available."
//!
//! When the disk array behind Plex is stopped or a drive is unmounted, the
//! server still answers but every file read fails. Once a minute while
//! connected, a few random tracks are checked with
//! `/library/metadata/{id}?checkFiles=1` (a metadata call, so it never
//! interrupts a playing stream, which a ranged file request would). Playback
//! failures are a second signal: several skipped tracks in a row trigger a
//! check right away. While files are unavailable the check keeps running
//! every minute and clears the banner once they're back.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::audio::PlayerHandle;
use crate::db::DbHandle;
use crate::error::AppResult;
use crate::plex::PlexState;

pub const CHECK_EVERY: Duration = Duration::from_secs(60);
/// How often the player's Problems list is looked at.
const PROBLEMS_EVERY: Duration = Duration::from_secs(5);
/// This many new playback failures in one look trigger a check now.
const FAILURE_BURST: usize = 2;
pub const SAMPLE: usize = 3;

/// Reads one track's file state from the server.
#[async_trait]
pub trait FileProbe: Send + Sync {
    /// `Some(true)` readable, `Some(false)` missing or unreadable, `None`
    /// unknown (the server didn't say, or the request failed).
    async fn readable(&self, track_id: &str) -> Option<bool>;
}

/// Combine per-track answers. Files are unavailable only when every track
/// that gave an answer is unreadable: one missing file is a "Skipped" toast,
/// not a dead array. `None` when nothing answered (keep the last verdict).
pub fn verdict(answers: &[Option<bool>]) -> Option<bool> {
    let known: Vec<bool> = answers.iter().flatten().copied().collect();
    if known.is_empty() {
        None
    } else {
        Some(known.iter().any(|ok| *ok))
    }
}

/// Check a sample of tracks.
pub async fn check(probe: &dyn FileProbe, ids: &[String]) -> Option<bool> {
    let mut answers = Vec::with_capacity(ids.len());
    for id in ids {
        answers.push(probe.readable(id).await);
    }
    verdict(&answers)
}

/// Counts playback failures that point at the files (not found, server
/// error, read error) since the last look at the Problems list.
#[derive(Default)]
pub struct FailureWatch {
    last_at: i64,
}

impl FailureWatch {
    /// `problems` is `(at, kind)` for every problem so far (the list is
    /// capped, so timestamps, not its length, tell what's new).
    pub fn new_file_failures(&mut self, problems: &[(i64, String)]) -> usize {
        let last = self.last_at;
        if let Some(max) = problems.iter().map(|p| p.0).max() {
            self.last_at = self.last_at.max(max);
        }
        problems
            .iter()
            .filter(|(at, kind)| *at > last && matches!(kind.as_str(), "notFound" | "server" | "io"))
            .count()
    }
}

struct PlexProbe(PlexState);

#[async_trait]
impl FileProbe for PlexProbe {
    async fn readable(&self, track_id: &str) -> Option<bool> {
        let client = self.0.client().await.ok()?;
        client.file_readable(track_id).await.ok().flatten()
    }
}

async fn sample_ids(db: &DbHandle) -> AppResult<Vec<String>> {
    db.read_async(|c| {
        let mut st = c.prepare("SELECT id FROM tracks ORDER BY random() LIMIT ?1")?;
        let ids = st
            .query_map([SAMPLE as i64], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    })
    .await
}

async fn run_check(plex: &PlexState, db: &DbHandle) {
    if !plex.connected() {
        return;
    }
    let ids = match sample_ids(db).await {
        Ok(ids) if !ids.is_empty() => ids,
        _ => return,
    };
    if let Some(ok) = check(&PlexProbe(plex.clone()), &ids).await {
        if ok != plex.files_available() {
            tracing::info!("music files available: {ok}");
        }
        plex.set_files_available(ok);
    }
}

/// Background loop. Also re-runs the library sync every minute while the
/// server has no music library, so one that's added later shows up.
pub fn spawn(plex: PlexState, db: DbHandle, player: PlayerHandle) {
    let player = Arc::new(player);
    tauri::async_runtime::spawn(async move {
        let mut watch = FailureWatch::default();
        let mut since_check = CHECK_EVERY; // check soon after start
        loop {
            tokio::time::sleep(PROBLEMS_EVERY).await;
            since_check += PROBLEMS_EVERY;
            let problems: Vec<(i64, String)> =
                player.problems().into_iter().map(|p| (p.at, p.kind)).collect();
            let burst = watch.new_file_failures(&problems) >= FAILURE_BURST;
            if burst || since_check >= CHECK_EVERY {
                since_check = Duration::ZERO;
                run_check(&plex, &db).await;
                if plex.connected() && plex.no_music_library() {
                    let _ = db.start_sync(false);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fake(HashMap<String, Option<bool>>);

    #[async_trait]
    impl FileProbe for Fake {
        async fn readable(&self, id: &str) -> Option<bool> {
            self.0.get(id).copied().flatten()
        }
    }

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| i.to_string()).collect()
    }

    #[tokio::test]
    async fn array_stopped_means_unavailable() {
        // Every sampled file is unreadable (array stopped, drive unmounted).
        let f = Fake((0..3).map(|i| (i.to_string(), Some(false))).collect());
        assert_eq!(check(&f, &ids(3)).await, Some(false));
    }

    #[tokio::test]
    async fn one_missing_file_is_not_an_outage() {
        let mut m: HashMap<_, _> = (0..3).map(|i| (i.to_string(), Some(true))).collect();
        m.insert("1".into(), Some(false));
        assert_eq!(check(&Fake(m), &ids(3)).await, Some(true));
    }

    #[tokio::test]
    async fn no_answers_keep_the_last_verdict() {
        let f = Fake(HashMap::new());
        assert_eq!(check(&f, &ids(3)).await, None);
        assert_eq!(verdict(&[None, Some(false)]), Some(false));
    }

    #[test]
    fn failure_watch_counts_only_new_file_failures() {
        let mut w = FailureWatch::default();
        let p = |v: &[(i64, &str)]| v.iter().map(|(a, k)| (*a, k.to_string())).collect::<Vec<_>>();
        assert_eq!(w.new_file_failures(&p(&[(1, "notFound")])), 1);
        assert_eq!(w.new_file_failures(&p(&[(1, "notFound")])), 0);
        assert_eq!(
            w.new_file_failures(&p(&[(1, "notFound"), (2, "decode"), (3, "server"), (4, "io")])),
            2
        );
        // capped list: the oldest dropped, one new at the end
        assert_eq!(w.new_file_failures(&p(&[(2, "decode"), (3, "server"), (4, "io"), (5, "server")])), 1);
    }
}
