//! A download Platter asked for that Lidarr couldn't import (a whole CD in
//! one file with a cue sheet, tracks missing) sits in Lidarr's queue for
//! good: Lidarr won't try it again. Platter clears it: out of the queue and
//! the download client, that copy blocklisted so it's never grabbed again,
//! and the album searched again. Up to [`MAX_CLEARS`] copies per request,
//! each one in the Changes log. Practice mode only logs what it would do;
//! with the setting off Platter only says so.

use super::Lidarr;
use super::api::{HistoryItem, QueueItem};
use super::request::{RETRY_EVERY, RequestRecord};
use super::write::LidarrWriter;
use crate::error::AppResult;
use crate::jellyfin::types::unix_s;

/// Most copies cleared for one request before Platter stops and says so.
pub const MAX_CLEARS: usize = 3;
/// Clears that don't go out in a row before Platter leaves it to the user.
pub const MAX_CLEAR_FAILS: u32 = 3;
/// How far apart the device's and Lidarr's clocks may be.
const CLOCK_SLACK_S: i64 = 60;

/// What happened to a download Lidarr couldn't import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cleared {
    /// Cleared; the album is searched again through the usual checks. The
    /// request's new message.
    Yes(String),
    /// Left as it is for now (practice mode, or the clear didn't go out and
    /// is tried again later).
    Waits(String),
    /// Left for the user: the setting is off, or too many copies failed.
    No(String),
}

/// Lidarr's queue without the downloads Platter cleared: Lidarr can list
/// one a little longer, or again with a failed state.
pub fn without_cleared(queue: &[QueueItem], cleared: &[String]) -> Vec<QueueItem> {
    queue.iter().filter(|q| !q.download_id.as_ref().is_some_and(|d| cleared.contains(d))).cloned().collect()
}

/// A download Lidarr couldn't import is this request's to clear only when
/// Platter added the album and Lidarr got the download after the request
/// went out (its time in the queue, or its grab in history). One the user
/// started before, or one Lidarr can't tell apart, is never touched.
pub fn ours(r: &RequestRecord, q: &QueueItem, history: &[HistoryItem]) -> bool {
    if r.already_in_lidarr || q.id.is_none() {
        return false;
    }
    let Some(dl) = q.download_id.as_deref().filter(|d| !d.trim().is_empty()) else { return false };
    let grabbed = || {
        history
            .iter()
            .filter(|h| h.event_type.eq_ignore_ascii_case("grabbed") && h.download_id.as_deref() == Some(dl))
            .filter_map(|h| unix_s(&h.date))
            .min()
    };
    // A request that waited here went out with its first search.
    let sent = r.searched_at.unwrap_or(r.created_at).max(r.created_at);
    q.added.as_deref().and_then(unix_s).or_else(grabbed).is_some_and(|t| t >= sent - CLOCK_SLACK_S)
}

/// Lidarr's reason, in lower case for the middle of a sentence.
pub fn reason(q: &QueueItem) -> String {
    q.why().unwrap_or_else(|| "Lidarr couldn't import it".into()).to_lowercase()
}

/// The message while practice mode holds a clear back.
pub fn practice_note(lower: &str) -> String {
    format!("Lidarr couldn't import the copy it got ({lower}). Practice mode: Platter clears it and looks for another once practice ends.")
}

impl Lidarr {
    /// Clear one download of `r`'s ([`ours`] already said so).
    pub(super) async fn clear_failed(&self, writer: &LidarrWriter, r: &mut RequestRecord, q: &QueueItem, album_id: i64, now: i64) -> AppResult<Cleared> {
        let why = q.why().unwrap_or_else(|| "Lidarr couldn't import it".into());
        let copy = q.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "The download".into());
        let lower = why.to_lowercase();
        if !crate::settings::load(&self.inner.db).lidarr_clear_failed {
            return Ok(Cleared::No(format!(
                "Lidarr couldn't import the copy it got ({lower}). Clear it on Lidarr's Activity page, or turn on clearing in Settings."
            )));
        }
        let (Some(queue_id), Some(download_id)) = (q.id, q.download_id.clone()) else {
            return Ok(Cleared::No(format!("Lidarr couldn't import the copy it got ({lower}). Lidarr's Activity page has it.")));
        };
        if r.cleared.len() >= MAX_CLEARS {
            return Ok(Cleared::No(format!(
                "{MAX_CLEARS} copies Lidarr got couldn't be imported (the last: {lower}). Platter stopped trying; Lidarr's Activity page has it."
            )));
        }
        let what = format!("Cleared a download of {} by {} that Lidarr couldn't import", r.title, r.artist);
        let before = format!("{copy}: {why}");
        let after = "Out of Lidarr's queue and the download client, that copy blocklisted, and another searched for";
        if self.practice_active() {
            if !r.practiced_clears.contains(&queue_id) {
                self.log_practice(&format!("Would clear a download of {} by {} that Lidarr couldn't import", r.title, r.artist), &before, after)?;
                r.practiced_clears.push(queue_id);
            }
            return Ok(Cleared::Waits(practice_note(&lower)));
        }
        let later = format!("Lidarr couldn't import the copy it got ({lower}). Platter couldn't clear it yet and tries again soon.");
        if r.clear_after.is_some_and(|t| now < t) {
            return Ok(Cleared::Waits(later));
        }
        if let Err(e) = writer.clear_failed(queue_id).await {
            r.clear_fails += 1;
            tracing::info!(album = album_id, queue = queue_id, fails = r.clear_fails, error = %e, "lidarr request: couldn't clear a download that couldn't be imported");
            if r.clear_fails >= MAX_CLEAR_FAILS {
                r.clear_after = None;
                return Ok(Cleared::No(format!(
                    "Lidarr couldn't import the copy it got ({lower}), and Platter couldn't clear it. Clear it on Lidarr's Activity page."
                )));
            }
            // 5 minutes, then 10: a refused clear isn't sent every look.
            r.clear_after = Some(now + RETRY_EVERY.as_secs() as i64 * (1 << (r.clear_fails - 1)));
            return Ok(Cleared::Waits(later));
        }
        r.clear_fails = 0;
        r.clear_after = None;
        if !r.cleared.contains(&download_id) {
            r.cleared.push(download_id);
        }
        if let Err(e) = self.log_change(&what, &before, after, false, None) {
            tracing::warn!(album = album_id, error = %e, "lidarr request: cleared a download, and the changes log didn't take it");
        }
        tracing::info!(album = album_id, queue = queue_id, title = %r.title, why = %why, "lidarr request: cleared a download that couldn't be imported");
        // Searched again like the first search, through the same checks as
        // every search again; the schedule starts over.
        r.searched_at = Some(now);
        r.searches = 1;
        r.progress = None;
        r.next_search_at = Some(now);
        Ok(Cleared::Yes(format!("The copy Lidarr got couldn't be imported ({lower}). Platter cleared it and is looking for another.")))
    }
}
