//! Where this computer reaches the files on the music server.
//!
//! Plex and Lidarr name files by their paths on the server
//! (`/data/music/Band/Album/01 Song.mp3`). A computer with the music share
//! mapped reaches the same file by another path (`M:\Band\Album\01 Song.mp3`,
//! `/Volumes/music/Band/...`). The share mapping pairs the two, so anything
//! that works on the files themselves (album upgrades now) knows where to
//! look. It's kept on this computer only: each computer mounts the share in
//! its own place.
//!
//! Nothing writes to the share unless a mapping covers the file.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::lidarr::paths::NormPath;

const KEY: &str = "musicShare.map";
/// Most folder pairs.
pub const MAX_PAIRS: usize = 8;
const MAX_PATH: usize = 400;

/// One server folder and where this computer reaches it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPair {
    /// As Plex sees it.
    pub server: String,
    /// On this computer.
    pub local: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShareMap {
    /// Plex's music folders and where this computer reaches them.
    pub music: Vec<FolderPair>,
    /// The staging folder for upgrades on this computer (Lidarr's own path
    /// for it is the `lidarrStagingFolder` setting).
    pub staging: String,
    /// Where replaced files are kept, outside every music folder. Empty: a
    /// `_to_delete` folder in the library root that Plex is told to ignore.
    pub backup: String,
}

fn clean(p: &str) -> String {
    let p = p.trim();
    if p.len() > 1 { p.trim_end_matches(['/', '\\']).to_string() } else { p.to_string() }
}

fn absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with('/') || p.starts_with("\\\\") || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

fn check(p: &str, what: &str) -> AppResult<()> {
    if p.is_empty() {
        return Ok(());
    }
    if !absolute(p) || p.len() > MAX_PATH || p.chars().any(char::is_control) || p.split(['/', '\\']).any(|x| x == "..") {
        return Err(AppError::Invalid(format!("Write {what} as a full folder path.")));
    }
    Ok(())
}

impl ShareMap {
    /// Trimmed and checked.
    pub fn cleaned(&self) -> AppResult<ShareMap> {
        let mut out = ShareMap { staging: clean(&self.staging), backup: clean(&self.backup), music: Vec::new() };
        for pair in &self.music {
            let (server, local) = (clean(&pair.server), clean(&pair.local));
            if server.is_empty() && local.is_empty() {
                continue;
            }
            if server.is_empty() || local.is_empty() {
                return Err(AppError::Invalid("Give both the folder as Plex sees it and where this computer reaches it.".into()));
            }
            check(&server, "the folder as Plex sees it")?;
            check(&local, "the folder on this computer")?;
            out.music.push(FolderPair { server, local });
        }
        if out.music.len() > MAX_PAIRS {
            return Err(AppError::Invalid(format!("Up to {MAX_PAIRS} music folders.")));
        }
        check(&out.staging, "the staging folder on this computer")?;
        check(&out.backup, "the backup folder")?;
        if !out.backup.is_empty() {
            let b = NormPath::parse(&out.backup);
            if out.music.iter().map(|p| NormPath::parse(&p.local)).any(|m| b.starts_with(&m) || m.starts_with(&b)) {
                return Err(AppError::Invalid(
                    "Pick a backup folder outside your music folder, or leave it empty to use an ignored _to_delete folder.".into(),
                ));
            }
        }
        Ok(out)
    }

    /// A music file (Plex's path) on this computer, when a mapping covers it.
    pub fn music_path(&self, server_path: &str) -> Option<PathBuf> {
        self.music
            .iter()
            .filter_map(|p| map_under(&p.server, &p.local, server_path).map(|l| (NormPath::parse(&p.server).len(), l)))
            .max_by_key(|(n, _)| *n)
            .map(|(_, l)| l)
    }

    /// A file in the staging folder (Lidarr's path) on this computer.
    pub fn staging_path(&self, lidarr_staging: &str, lidarr_path: &str) -> Option<PathBuf> {
        if self.staging.is_empty() || lidarr_staging.trim().is_empty() {
            return None;
        }
        map_under(lidarr_staging, &self.staging, lidarr_path)
    }

    pub fn is_set(&self) -> bool {
        !self.music.is_empty()
    }
}

/// `path` under `server`, moved to the same place under `local`.
pub fn map_under(server: &str, local: &str, path: &str) -> Option<PathBuf> {
    let base = NormPath::parse(server);
    let full = NormPath::parse(path);
    if base.is_empty() || !full.starts_with(&base) || local.trim().is_empty() {
        return None;
    }
    let rest: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.trim().is_empty() && *s != ".").skip(base.len()).collect();
    if rest.contains(&"..") {
        return None;
    }
    let mut out = PathBuf::from(local.trim());
    for s in rest {
        out.push(s);
    }
    Some(out)
}

pub fn load(db: &DbHandle) -> ShareMap {
    db.read(|c| kv_get(c, KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(db: &DbHandle, map: &ShareMap) -> AppResult<ShareMap> {
    let map = map.cleaned()?;
    let json = serde_json::to_string(&map).map_err(|e| AppError::Internal(e.to_string()))?;
    db.write(|c| kv_set(c, KEY, &json))?;
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> ShareMap {
        ShareMap {
            music: vec![
                FolderPair { server: "/data/music".into(), local: "/mnt/music".into() },
                FolderPair { server: "/data/music/Extra".into(), local: "/mnt/extra".into() },
            ],
            staging: "/mnt/upgrades".into(),
            backup: String::new(),
        }
    }

    #[test]
    fn server_paths_map_to_this_computer() {
        let m = map();
        assert_eq!(m.music_path("/data/music/Band/Album/01 Song.mp3"), Some(PathBuf::from("/mnt/music/Band/Album/01 Song.mp3")));
        assert_eq!(m.music_path("/DATA/Music/Band/x.flac"), Some(PathBuf::from("/mnt/music/Band/x.flac")), "any case");
        assert_eq!(m.music_path("/data/music/Extra/y.flac"), Some(PathBuf::from("/mnt/extra/y.flac")), "the closest pair wins");
        assert_eq!(m.music_path("/data/musicals/z.flac"), None);
        assert_eq!(m.music_path("/data/music/../etc/passwd"), None);
        assert_eq!(m.staging_path("/downloads/upgrades", "/downloads/upgrades/Band/Album/01.flac"), Some(PathBuf::from("/mnt/upgrades/Band/Album/01.flac")));
        assert_eq!(m.staging_path("", "/downloads/upgrades/a.flac"), None);
        assert_eq!(ShareMap::default().music_path("/data/music/a.flac"), None, "no mapping, no path");
    }

    #[test]
    fn saved_maps_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("m.db")).unwrap();
        assert_eq!(load(&db), ShareMap::default());
        let saved = save(&db, &ShareMap { music: vec![FolderPair { server: "/data/music/".into(), local: r"M:\".into() }], ..map() }).unwrap();
        assert_eq!(saved.music[0], FolderPair { server: "/data/music".into(), local: "M:".into() });
        assert_eq!(load(&db), saved);
        let half = ShareMap { music: vec![FolderPair { server: "/data/music".into(), local: String::new() }], ..Default::default() };
        assert!(save(&db, &half).is_err());
        let relative = ShareMap { backup: "backups".into(), ..Default::default() };
        assert!(save(&db, &relative).is_err());
        let inside = ShareMap { backup: "/mnt/music/old".into(), ..map() };
        assert!(save(&db, &inside).unwrap_err().to_string().contains("outside your music folder"));
        assert!(save(&db, &ShareMap { backup: "/mnt/backups".into(), ..map() }).is_ok());
    }
}
