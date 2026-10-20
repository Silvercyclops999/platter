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

/// Least unreadable answers that can mean the files are gone. One track's
/// missing file (deleted, or replaced by a new copy the server hasn't caught
/// up with) is a "Skipped" toast, not a stopped disk array.
pub const MIN_UNREADABLE: usize = 2;

/// Combine per-track answers. Files are unavailable only when at least
/// [`MIN_UNREADABLE`] tracks answered and every one is unreadable. `None`
/// when that can't be told (keep the last verdict).
pub fn verdict(answers: &[Option<bool>]) -> Option<bool> {
    let known: Vec<bool> = answers.iter().flatten().copied().collect();
    if known.iter().any(|ok| *ok) {
        Some(true)
    } else if known.len() >= MIN_UNREADABLE {
        Some(false)
    } else {
        None
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

/// [`check`], with a second, different sample having to agree before files
/// that were available count as unavailable: greying out the whole library
/// is a big step. `available` is the current verdict; `resample` draws
/// another sample.
pub async fn confirmed<F, Fut>(probe: &dyn FileProbe, ids: &[String], available: bool, resample: F) -> Option<bool>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = AppResult<Vec<String>>>,
{
    let first = check(probe, ids).await;
    if first != Some(false) || !available {
        return first;
    }
    let again = match resample().await {
        Ok(ids) if !ids.is_empty() => check(probe, &ids).await,
        _ => None,
    };
    if again == Some(false) {
        Some(false)
    } else {
        tracing::info!("music files check: one sample couldn't be read but another could (or didn't say); files still count as available");
        None
    }
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

/// Asks Plex whether the track's file is there (`checkFiles`).
pub struct PlexProbe(pub PlexState);

#[async_trait]
impl FileProbe for PlexProbe {
    async fn readable(&self, track_id: &str) -> Option<bool> {
        let client = self.0.client_for(track_id).await.ok()?;
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

async fn run_check(plex: &PlexState, db: &DbHandle, probe: &dyn FileProbe) {
    if !plex.connected() {
        return;
    }
    let ids = match sample_ids(db).await {
        Ok(ids) if !ids.is_empty() => ids,
        _ => return,
    };
    if let Some(ok) = confirmed(probe, &ids, plex.files_available(), || sample_ids(db)).await {
        if ok != plex.files_available() {
            tracing::info!("music files available: {ok}");
        }
        plex.set_files_available(ok);
    }
}

/// Playback counts as recent for this long after its last event.
const ACTIVE_FOR: Duration = Duration::from_secs(30);

/// When the loop next wakes: every few seconds around playback (to catch a
/// burst of failures), otherwise only for the once-a-minute file check.
fn next_look(active: bool, last_look: tokio::time::Instant, last_check: tokio::time::Instant) -> tokio::time::Instant {
    let check = last_check + CHECK_EVERY;
    if active { check.min(last_look + PROBLEMS_EVERY) } else { check }
}

/// Background loop. Also re-runs the library sync every minute while the
/// server has no music library, so one that's added later shows up.
/// `probe` asks each track's server about its file.
pub fn spawn(plex: PlexState, db: DbHandle, player: PlayerHandle, probe: Arc<dyn FileProbe>) {
    let player = Arc::new(player);
    let mut tracking = player.subscribe_tracking();
    tauri::async_runtime::spawn(async move {
        use tokio::time::Instant;
        let mut watch = FailureWatch::default();
        let start = Instant::now();
        // Check soon after start.
        let mut last_check = start.checked_sub(CHECK_EVERY - PROBLEMS_EVERY).unwrap_or(start);
        let mut last_look = start;
        // Playback failures only happen while something is being played, so
        // the Problems list is looked at often only around playback.
        let mut last_playback: Option<Instant> = None;
        let mut events_open = true;
        loop {
            let now = Instant::now();
            let active = last_playback.is_some_and(|at| now.duration_since(at) < ACTIVE_FOR);
            let at = next_look(active, last_look, last_check);
            tokio::select! {
                _ = tokio::time::sleep_until(at) => {}
                ev = tracking.recv(), if events_open => {
                    match ev {
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => events_open = false,
                        _ => last_playback = Some(Instant::now()),
                    }
                    continue;
                }
            }
            // With the screen off the periodic check waits; around playback
            // (a burst of failures) it still runs.
            if !active {
                crate::platform::foreground::settled().await;
            }
            last_look = Instant::now();
            let problems: Vec<(i64, String)> =
                player.problems().into_iter().map(|p| (p.at, p.kind)).collect();
            let burst = watch.new_file_failures(&problems) >= FAILURE_BURST;
            if burst || last_check.elapsed() >= CHECK_EVERY {
                last_check = Instant::now();
                run_check(&plex, &db, probe.as_ref()).await;
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
        // One unreadable answer can be one missing file: not enough to tell.
        assert_eq!(verdict(&[None, Some(false), None]), None);
        assert_eq!(verdict(&[Some(false), None, Some(false)]), Some(false));
        assert_eq!(verdict(&[None, Some(true), None]), Some(true));
    }

    #[tokio::test]
    async fn files_go_unavailable_only_when_a_second_sample_agrees() {
        let mut m: HashMap<String, Option<bool>> = (0..3).map(|i| (i.to_string(), Some(false))).collect();
        m.extend((10..13).map(|i| (i.to_string(), Some(true))));
        let f = Fake(m);
        let bad = ids(3);
        let good: Vec<String> = (10..13).map(|i| i.to_string()).collect();
        // A sample of tracks whose files went (replaced copies, say), then a
        // sample that plays: still available.
        let g = good.clone();
        assert_eq!(confirmed(&f, &bad, true, || async move { Ok(g) }).await, None);
        // Both samples unreadable: unavailable.
        let b = bad.clone();
        assert_eq!(confirmed(&f, &bad, true, || async move { Ok(b) }).await, Some(false));
        // Already unavailable: one readable sample brings it back, no second look needed.
        assert_eq!(confirmed(&f, &good, false, || async { Ok(Vec::new()) }).await, Some(true));
        // The second sample couldn't be drawn: don't grey anything out.
        assert_eq!(confirmed(&f, &bad, true, || async { Err(crate::error::AppError::Internal("db".into())) }).await, None);
    }

    #[test]
    fn idle_player_wakes_only_for_the_minute_check() {
        let t0 = tokio::time::Instant::now();
        // Nothing playing: next wake is the file check, a minute out.
        assert_eq!(next_look(false, t0, t0), t0 + CHECK_EVERY);
        // Around playback: every few seconds.
        assert_eq!(next_look(true, t0, t0), t0 + PROBLEMS_EVERY);
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
