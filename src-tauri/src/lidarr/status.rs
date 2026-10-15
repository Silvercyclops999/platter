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
    Imported,
    Failed,
}

impl RequestState {
    /// Nothing more will change without the user.
    pub fn settled(self) -> bool {
        matches!(self, RequestState::Imported | RequestState::Failed)
    }
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
        let failed = q.status.eq_ignore_ascii_case("failed") || state.starts_with("failed");
        return if failed { RequestState::Failed } else { RequestState::Downloading };
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
        HistoryItem { album_id: Some(7), event_type: ev.into(), date: String::new() }
    }

    #[test]
    fn status_mapping() {
        assert_eq!(map_status(&album(false, 0, 10), &[], &[]), RequestState::Requested);
        assert_eq!(map_status(&album(true, 0, 10), &[], &[]), RequestState::Searching);
        assert_eq!(map_status(&album(true, 0, 10), &[q("downloading", "downloading")], &[]), RequestState::Downloading);
        assert_eq!(map_status(&album(true, 0, 10), &[q("completed", "importPending")], &[]), RequestState::Downloading);
        assert_eq!(map_status(&album(true, 0, 10), &[q("failed", "failedPending")], &[]), RequestState::Failed);
        assert_eq!(map_status(&album(true, 0, 10), &[], &[h("grabbed")]), RequestState::Downloading);
        assert_eq!(map_status(&album(true, 0, 10), &[], &[h("downloadFailed"), h("grabbed")]), RequestState::Failed);
        assert_eq!(map_status(&album(true, 0, 10), &[], &[h("downloadImported"), h("grabbed")]), RequestState::Imported);
        assert_eq!(map_status(&album(true, 10, 10), &[q("downloading", "downloading")], &[]), RequestState::Imported, "files win");
        assert_eq!(map_status(&album(true, 3, 10), &[], &[]), RequestState::Searching, "a few files is not imported");
        let other = QueueItem { album_id: Some(8), status: "downloading".into(), ..Default::default() };
        assert_eq!(map_status(&album(true, 0, 10), &[other], &[]), RequestState::Searching, "other albums don't count");
        assert!(RequestState::Imported.settled() && !RequestState::Queued.settled());
    }
}
