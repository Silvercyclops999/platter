//! What Lidarr work needs from Plex: its scan settings, a scan of one
//! folder, and whether Plex shows a folder's files yet. Kept behind a trait
//! so the catch-up and promotion logic stay free of the app (a headless
//! build can reuse them) and tests can fake a Plex that only sees an album
//! after a partial scan.

use async_trait::async_trait;

use super::paths::NormPath;
use crate::error::{AppError, AppResult};
use crate::plex::client::{MetaType, ServerPrefs};

/// How many of the newest tracks are looked through for a new folder.
const RECENT: u64 = 400;

#[async_trait]
pub trait PlexServer: Send + Sync {
    async fn prefs(&self) -> AppResult<ServerPrefs>;
    /// Turn "Empty trash automatically after every scan" on or off.
    async fn set_empty_trash(&self, on: bool) -> AppResult<()>;
    /// Scan just this folder (a partial section refresh with the path).
    async fn scan_folder(&self, folder: &str) -> AppResult<()>;
    /// Plex has a file inside this folder.
    async fn sees_folder(&self, folder: &str) -> AppResult<bool>;
}

/// The connected Plex server.
pub struct PlexLive(pub crate::plex::PlexState);

fn unreachable(_: AppError) -> AppError {
    AppError::Network("Platter can't reach your Plex server.".into())
}

#[async_trait]
impl PlexServer for PlexLive {
    async fn prefs(&self) -> AppResult<ServerPrefs> {
        let c = self.0.client().await.map_err(unreachable)?;
        c.server_prefs().await
    }

    async fn set_empty_trash(&self, on: bool) -> AppResult<()> {
        let c = self.0.client().await.map_err(unreachable)?;
        c.set_empty_trash(on).await
    }

    async fn scan_folder(&self, folder: &str) -> AppResult<()> {
        let c = self.0.client().await.map_err(unreachable)?;
        let f = NormPath::parse(folder);
        let id = c
            .music_sections()
            .await
            .map_err(unreachable)?
            .into_iter()
            .find(|s| s.locations.iter().any(|l| f.starts_with(&NormPath::parse(l))))
            .map(|s| s.id)
            .ok_or_else(|| AppError::NotFound(format!("No Plex music library includes {folder}.")))?;
        c.scan_folder(&id, folder).await
    }

    async fn sees_folder(&self, folder: &str) -> AppResult<bool> {
        let c = self.0.client().await.map_err(unreachable)?;
        let f = NormPath::parse(folder);
        for s in c.music_sections().await.map_err(unreachable)? {
            if !s.locations.iter().any(|l| f.starts_with(&NormPath::parse(l))) {
                continue;
            }
            let recent = c.list(&s.id, MetaType::Track, 0, RECENT, &[("sort", "addedAt:desc".to_string())]).await.map_err(unreachable)?;
            let hit = recent
                .items
                .iter()
                .flat_map(|t| t.media.iter().flat_map(|m| m.parts.iter()))
                .filter_map(|p| p.file.as_deref())
                .any(|file| NormPath::parse(file).starts_with(&f));
            if hit {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
pub mod fake {
    use parking_lot::Mutex;

    use super::*;

    /// A Plex that only sees a folder once it was scanned (or scans on its
    /// own when `auto` is on), and logs every scan and setting change.
    #[derive(Default)]
    pub struct FakePlex {
        pub auto: Option<bool>,
        pub empty_trash: Mutex<Option<bool>>,
        pub seen: Mutex<Vec<String>>,
        pub scans: Mutex<Vec<String>>,
        pub trash_changes: Mutex<Vec<bool>>,
    }

    #[async_trait]
    impl PlexServer for FakePlex {
        async fn prefs(&self) -> AppResult<ServerPrefs> {
            Ok(ServerPrefs { auto_scan: self.auto, periodic_scan: None, empty_trash: *self.empty_trash.lock() })
        }

        async fn set_empty_trash(&self, on: bool) -> AppResult<()> {
            *self.empty_trash.lock() = Some(on);
            self.trash_changes.lock().push(on);
            Ok(())
        }

        async fn scan_folder(&self, folder: &str) -> AppResult<()> {
            self.scans.lock().push(folder.to_string());
            self.seen.lock().push(folder.to_string());
            Ok(())
        }

        async fn sees_folder(&self, folder: &str) -> AppResult<bool> {
            let f = NormPath::parse(folder);
            Ok(self.seen.lock().iter().any(|s| f.starts_with(&NormPath::parse(s))))
        }
    }
}
