//! Where a request stands, from Lidarr's album, queue and history (reads).

use serde::{Deserialize, Serialize};

use super::api::{Album, HistoryItem, QueueItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestState {
    /// Waiting until Platter can reach Lidarr.
    Queued,
    /// In Lidarr, not monitored (or not yet).
    Requested,
    /// Monitored and searched; nothing grabbed yet.
    Searching,
    Downloading,
    /// Downloaded; Lidarr is moving it into place.
    Importing,
    Imported,
    Failed,
    /// Searched again and again without anything turning up. "Search
    /// again" starts over.
    NotFound,
}

impl RequestState {
    /// Nothing more will change without the user.
    pub fn settled(self) -> bool {
        matches!(self, RequestState::Imported | RequestState::Failed | RequestState::NotFound)
    }

    /// Lidarr is still working on it (worth following closely).
    pub fn in_flight(self) -> bool {
        matches!(self, RequestState::Requested | RequestState::Searching | RequestState::Downloading | RequestState::Importing)
    }
}

/// How far a download in the queue has got, 0 to 100.
pub fn progress(album_id: i64, queue: &[QueueItem]) -> Option<u8> {
    let (mut size, mut left) = (0.0_f64, 0.0_f64);
    for q in queue.iter().filter(|q| q.album_id == Some(album_id)) {
        size += q.size.unwrap_or(0.0).max(0.0);
        left += q.sizeleft.unwrap_or(0.0).max(0.0);
    }
    if size <= 0.0 {
        return None;
    }
    Some((((size - left.min(size)) / size) * 100.0).round().clamp(0.0, 100.0) as u8)
}

fn has_files(a: &Album) -> bool {
    a.statistics.as_ref().is_some_and(|s| s.track_file_count > 0 && (s.percent_of_tracks >= 100.0 || (s.track_count > 0 && s.track_file_count >= s.track_count)))
}

/// The state of one album. `queue` and `history` may hold other albums'
/// entries; only this album's count. History is newest first.
pub fn map_status(album: &Album, queue: &[QueueItem], history: &[HistoryItem]) -> RequestState {
    if has_files(album) {
        return RequestState::Imported;
    }
    let mine = |id: Option<i64>| id == Some(album.id);
    if let Some(q) = queue.iter().find(|q| mine(q.album_id)) {
        let state = q.tracked_download_state.as_deref().unwrap_or_default().to_ascii_lowercase();
        // Lidarr won't try an import that failed again: it's failed, not importing.
        let failed = q.status.eq_ignore_ascii_case("failed") || state.starts_with("failed") || state == "importfailed";
        let importing = matches!(state.as_str(), "importpending" | "importing" | "imported") || q.status.eq_ignore_ascii_case("completed");
        return if failed {
            RequestState::Failed
        } else if importing {
            RequestState::Importing
        } else {
            RequestState::Downloading
        };
    }
    for h in history.iter().filter(|h| h.album_id.is_none() || mine(h.album_id)) {
        match h.event_type.as_str() {
            "downloadImported" | "trackFileImported" => return RequestState::Imported,
            "downloadFailed" | "albumImportIncomplete" | "importFailed" => return RequestState::Failed,
            "grabbed" => return RequestState::Downloading,
            _ => {}
        }
    }
    if album.monitored { RequestState::Searching } else { RequestState::Requested }
}

#[cfg(test)]
mod tests {
    use super::super::api::AlbumStatistics;
    use super::*;

    fn album(monitored: bool, files: i64, of: i64) -> Album {
        Album {
            id: 7,
            monitored,
            statistics: Some(AlbumStatistics { track_file_count: files, track_count: of, percent_of_tracks: if of > 0 { files as f64 * 100.0 / of as f64 } else { 0.0 } }),
            ..Default::default()
        }
    }

    fn q(status: &str, state: &str) -> QueueItem {
        QueueItem { album_id: Some(7), status: status.into(), tracked_download_state: Some(state.into()), ..Default::default() }
    }

    fn h(ev: &str) -> HistoryItem {
        HistoryItem { album_id: Some(7), event_type: ev.into(), ..Default::default() }
    }

    #[test]
    fn status_mapping() {
        assert_eq!(map_status(&album(false, 0, 10), &[], &[]), RequestState::Requested);
        assert_eq!(map_status(&album(true, 0, 10), &[], &[]), RequestState::Searching);
        assert_eq!(map_status(&album(true, 0, 10), &[q("downloading", "downloading")], &[]), RequestState::Downloading);
        assert_eq!(map_status(&album(true, 0, 10), &[q("completed", "importPending")], &[]), RequestState::Importing);
        assert_eq!(map_status(&album(true, 0, 10), &[q("downloading", "importing")], &[]), RequestState::Importing);
        assert_eq!(map_status(&album(true, 0, 10), &[q("failed", "failedPending")], &[]), RequestState::Failed);
        assert_eq!(map_status(&album(true, 0, 10), &[q("completed", "importFailed")], &[]), RequestState::Failed, "not importing forever");
        assert_eq!(map_status(&album(true, 0, 10), &[], &[h("grabbed")]), RequestState::Downloading);
        assert_eq!(map_status(&album(true, 0, 10), &[], &[h("downloadFailed"), h("grabbed")]), RequestState::Failed);
        assert_eq!(map_status(&album(true, 0, 10), &[], &[h("downloadImported"), h("grabbed")]), RequestState::Imported);
        assert_eq!(map_status(&album(true, 10, 10), &[q("downloading", "downloading")], &[]), RequestState::Imported, "files win");
        assert_eq!(map_status(&album(true, 3, 10), &[], &[]), RequestState::Searching, "a few files is not imported");
        let other = QueueItem { album_id: Some(8), status: "downloading".into(), ..Default::default() };
        assert_eq!(map_status(&album(true, 0, 10), &[other], &[]), RequestState::Searching, "other albums don't count");
        assert!(RequestState::Imported.settled() && !RequestState::Queued.settled() && RequestState::NotFound.settled());
        assert!(RequestState::Importing.in_flight() && !RequestState::Queued.in_flight());
    }

    #[test]
    fn download_progress() {
        let item = |id, size: f64, left: f64| QueueItem { album_id: Some(id), size: Some(size), sizeleft: Some(left), ..Default::default() };
        assert_eq!(progress(7, &[item(7, 200.0, 120.0)]), Some(40));
        assert_eq!(progress(7, &[item(7, 100.0, 0.0), item(7, 100.0, 100.0)]), Some(50), "parts add up");
        assert_eq!(progress(7, &[item(8, 100.0, 0.0)]), None, "other albums don't count");
        assert_eq!(progress(7, &[QueueItem { album_id: Some(7), ..Default::default() }]), None, "no size yet");
    }
}
