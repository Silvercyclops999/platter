//! Lidarr can move an album to another edition (release) on its own: with
//! "Automatically Switch Release" on, its daily artist refresh may pick a
//! release with more or fewer tracks, and files on the old one stop
//! counting. Nothing changes on disk.
//!
//! So an album's edition is never taken for granted. Before a request,
//! upgrade, swap or re-match acts on an album, Platter reads the album's
//! current edition again (GET only) and stops with a plain note when it
//! isn't the one the plan was made with. Every read of an album's edition
//! is also compared with the last one seen on this device: Platter never
//! switches editions itself, so a difference is Lidarr's doing, and it goes
//! in the changes log ("Lidarr switched ... to the 19-track edition") with
//! nothing done about it.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::Lidarr;
use super::api::{AlbumDetail, AlbumRelease, LidarrClient};
use super::check::AlbumFolders;
use super::paths::NormPath;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};

/// The last edition seen per album, on this device.
const SEEN_KEY: &str = "lidarr.seenReleases";
/// Most artists one setup check reads album files for.
const MAX_SCAN_ARTISTS: usize = 20;

/// An album's edition as Platter saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeenRelease {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub tracks: i64,
    #[serde(default)]
    pub disambiguation: String,
}

impl SeenRelease {
    pub fn of(r: &AlbumRelease) -> Self {
        Self { id: r.id, title: r.title.clone(), tracks: r.track_count, disambiguation: r.disambiguation.clone() }
    }

    /// The current one among an album's editions.
    pub fn current(releases: &[AlbumRelease]) -> Option<Self> {
        releases.iter().find(|r| r.monitored).map(Self::of)
    }
}

/// "the 19-track edition", or its name when the track count isn't known.
pub fn edition_words(r: &SeenRelease) -> String {
    if r.tracks > 0 {
        format!("the {}-track edition", r.tracks)
    } else if !r.disambiguation.trim().is_empty() {
        format!("the {} edition", r.disambiguation.trim())
    } else {
        "another edition".into()
    }
}

/// What the changes log says about a switch Lidarr made.
pub fn switch_line(album: &str, to: &SeenRelease) -> String {
    format!("Lidarr switched {album} to {}", edition_words(to))
}

/// The note when an album's edition changed under a plan.
pub fn changed_note(album: &str, now: Option<&SeenRelease>, stopped: &str) -> String {
    let to = now.map(edition_words).unwrap_or_else(|| "no edition".into());
    format!("Lidarr moved {album} to {to} since Platter looked at it, so {stopped} Look at it again.")
}

fn short(r: &SeenRelease) -> String {
    let name = if r.disambiguation.trim().is_empty() { r.title.trim() } else { r.disambiguation.trim() };
    match (name.is_empty(), r.tracks > 0) {
        (false, true) => format!("{name}, {} tracks", r.tracks),
        (false, false) => name.to_string(),
        (true, true) => format!("{} tracks", r.tracks),
        (true, false) => format!("Release {}", r.id),
    }
}

impl Lidarr {
    /// Seen editions are kept per Lidarr (its address).
    fn seen_prefix(&self) -> String {
        let (home, away) = self.urls();
        format!("{}#", super::url_key(if home.trim().is_empty() { &away } else { &home }))
    }

    fn seen(&self) -> HashMap<String, SeenRelease> {
        self.inner.db.read(|c| kv_get(c, SEEN_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Note the editions just read for these albums (id, title, editions).
    /// Any album whose edition isn't the one last seen was switched by
    /// Lidarr: logged, nothing done. Returns the lines logged.
    pub(super) fn observe_releases(&self, albums: &[(i64, String, Vec<AlbumRelease>)]) -> Vec<String> {
        let prefix = self.seen_prefix();
        let mut seen = self.seen();
        let mut lines = Vec::new();
        let mut dirty = false;
        for (id, title, releases) in albums {
            let Some(now) = SeenRelease::current(releases) else { continue };
            let key = format!("{prefix}{id}");
            match seen.get(&key) {
                Some(was) if was.id == now.id => continue,
                Some(was) => {
                    let line = switch_line(title, &now);
                    let _ = self.log_by_lidarr(&line, &short(was), &format!("{}. Platter didn't change anything.", short(&now)));
                    tracing::info!(album = id, from = was.id, to = now.id, "lidarr switched an album's edition");
                    lines.push(line);
                }
                None => {}
            }
            seen.insert(key, now);
            dirty = true;
        }
        if dirty && let Ok(json) = serde_json::to_string(&seen) {
            let _ = self.inner.db.write(|c| kv_set(c, SEEN_KEY, &json));
        }
        lines
    }

    /// The album and its current edition right now (GET only), noted.
    pub(super) async fn release_now(&self, client: &LidarrClient, album_id: i64) -> AppResult<(AlbumDetail, Option<SeenRelease>)> {
        let d = client.album_detail(album_id).await?;
        self.observe_releases(&[(d.id.max(album_id), d.title.clone(), d.releases.clone())]);
        let now = SeenRelease::current(&d.releases);
        Ok((d, now))
    }

    /// Stop when the album's edition isn't `planned` any more. `stopped`
    /// says what Platter didn't do ("Platter didn't search for it.").
    pub(super) async fn same_release(&self, client: &LidarrClient, album_id: i64, planned: Option<i64>, stopped: &str) -> AppResult<()> {
        let (d, now) = self.release_now(client, album_id).await?;
        if now.as_ref().map(|r| r.id) == planned {
            return Ok(());
        }
        Err(AppError::Invalid(changed_note(&d.title, now.as_ref(), stopped)))
    }

    /// For the setup check: Managed artists' albums whose folders hold files
    /// Lidarr couldn't match (extra discs, bonus tracks on another edition),
    /// with their files' folders and whether Lidarr may switch their edition.
    /// Reads only, and only for artists with such files.
    pub(super) async fn album_folders(&self, client: &LidarrClient, snap: &super::check::LidarrSnapshot, protected_ids: &[i64]) -> Vec<AlbumFolders> {
        let mut artists: Vec<&super::api::Artist> = Vec::new();
        for a in snap.artists.iter().filter(|a| !protected_ids.contains(&a.id) && !a.path.trim().is_empty()) {
            let root = NormPath::parse(&a.path);
            if snap.unmapped.iter().any(|f| NormPath::parse(&f.path).starts_with(&root)) {
                artists.push(a);
            }
        }
        let mut out = Vec::new();
        for a in artists.into_iter().take(MAX_SCAN_ARTISTS) {
            let (files, albums) = futures::join!(client.artist_track_files(a.id), client.albums_of(a.id));
            let (Ok(files), Ok(albums)) = (files, albums) else { continue };
            self.observe_releases(&albums.iter().filter(|x| !x.releases.is_empty()).map(|x| (x.id, x.title.clone(), x.releases.clone())).collect::<Vec<_>>());
            let mut by_album: HashMap<i64, BTreeSet<String>> = HashMap::new();
            for f in &files {
                if let (Some(id), Some(dir)) = (f.album_id.filter(|i| *i > 0), super::modes::parent_raw(&f.path)) {
                    by_album.entry(id).or_default().insert(dir);
                }
            }
            for al in albums {
                let Some(dirs) = by_album.remove(&al.id) else { continue };
                out.push(AlbumFolders {
                    album_id: al.id,
                    title: al.title,
                    artist_path: a.path.clone(),
                    any_release_ok: al.any_release_ok.unwrap_or(false),
                    folders: dirs.into_iter().collect(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(id: i64, tracks: i64, on: bool) -> AlbumRelease {
        AlbumRelease { id, title: "Panes".into(), monitored: on, track_count: tracks, ..Default::default() }
    }

    #[test]
    fn words_for_editions() {
        let r = SeenRelease::of(&rel(1, 19, true));
        assert_eq!(switch_line("Panes", &r), "Lidarr switched Panes to the 19-track edition");
        let named = SeenRelease { tracks: 0, disambiguation: "Newsletter Edit".into(), ..r.clone() };
        assert_eq!(edition_words(&named), "the Newsletter Edit edition");
        assert!(changed_note("Panes", Some(&r), "Platter didn't search for it.").contains("moved Panes to the 19-track edition"));
        assert_eq!(SeenRelease::current(&[rel(1, 13, false), rel(2, 19, true)]).map(|r| r.id), Some(2));
    }
}
