//! Songs got with a tap that Lidarr imported but Plex hasn't shown. The tap
//! follower flags one after its wait (`lidarr::tap`), and clears it when the
//! library has the song. Kept for this run of the app: a restart looks again
//! through the library sync anyway.

use std::sync::LazyLock;

use parking_lot::RwLock;

use super::{Finding, Fix, Kind};
use crate::lidarr::tap::Tap;

static LATE: LazyLock<RwLock<Vec<Finding>>> = LazyLock::new(|| RwLock::new(Vec::new()));

fn id(t: &Tap) -> String {
    format!("notInPlex:{}:{}", t.release_group_id, t.title.to_lowercase())
}

/// The finding for a song Plex hasn't shown.
pub fn finding(t: &Tap) -> Finding {
    Finding {
        id: id(t),
        kind: Kind::NotInPlex,
        artist_id: String::new(),
        artist: t.artist.clone(),
        album_id: String::new(),
        album: t.album.clone(),
        thumb: t.thumb.clone(),
        title: format!("{} isn't in your library yet", t.title),
        text: format!(
            "Lidarr downloaded {} by {}, but your server hasn't shown it. Look again asks your server to catch up.",
            t.title, t.artist
        ),
        evidence: vec![
            format!("Lidarr imported {} into its album folder.", t.album),
            "Your server was asked to scan that folder and still doesn't list the song.".into(),
        ],
        fix: Fix::LookAgain,
        tracks: Vec::new(),
        others: Vec::new(),
        table: Vec::new(),
        set: None,
        album_note: None,
        suggest: None,
    }
}

/// Plex hasn't shown it: list it. Returns whether the list changed.
pub fn flag(t: &Tap) -> bool {
    let f = finding(t);
    let mut w = LATE.write();
    if w.iter().any(|x| x.id == f.id) {
        return false;
    }
    w.push(f);
    true
}

/// The library has it now. Returns whether the list changed.
pub fn clear(t: &Tap) -> bool {
    let id = id(t);
    let mut w = LATE.write();
    let n = w.len();
    w.retain(|x| x.id != id);
    w.len() != n
}

/// Every song waiting on Plex.
pub fn findings() -> Vec<Finding> {
    LATE.read().clone()
}

#[cfg(test)]
pub fn reset() {
    LATE.write().clear();
}
