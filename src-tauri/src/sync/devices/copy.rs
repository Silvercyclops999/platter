//! "Copy from my computer": a phone's first full copy from a paired
//! computer. One group at a time (settings, then pins and folders, follows,
//! lyrics, looks, download choices, imported plays and the diary, each when
//! it's kept in step here), with progress, over the local network. It can
//! be stopped; what came stays, and the next round brings the rest.
//!
//! Settings are taken as they are on the computer (they win here and
//! everywhere); every other group merges as usual, so anything changed
//! here later than there is kept. Keys never come this way.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;

use super::run::{DeviceSync, OnCopy};
use crate::error::{AppError, AppResult};
use crate::sync::engine::Groups;

/// Where a first copy is, for the progress line.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyProgress {
    /// The computer it copies from.
    pub name: String,
    /// This step, from 1.
    pub step: usize,
    pub of: usize,
    /// "Copying your pins and folders".
    pub label: String,
}

/// One group's part of a first copy.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopiedGroup {
    pub group: String,
    /// "Pins and folders".
    pub label: String,
    /// Entries taken here.
    pub count: usize,
}

/// How a first copy ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyResult {
    pub name: String,
    pub groups: Vec<CopiedGroup>,
    /// What still comes later, in words (the computer wasn't on this
    /// network, so the rest goes through the relay or waits).
    pub later: Option<String>,
    /// Stopped before the end; what came stays.
    pub cancelled: bool,
}

/// The groups in the order they're copied: name, what the progress line
/// says, and the label for the counts.
const STEPS: [(&str, &str, &str); 8] = [
    ("settings", "your settings", "Settings"),
    ("library", "your pins and folders", "Pins and folders"),
    ("follows", "who you follow", "Follows and hidden items"),
    ("lyrics", "your lyrics", "Lyrics"),
    ("looks", "your looks", "Looks"),
    ("downloads", "what your computer keeps offline", "Download choices"),
    ("plays", "your imported plays", "Imported plays"),
    ("diary", "your diary", "Listening diary"),
];

fn only(group: &str) -> Groups {
    let mut g = Groups { settings: false, library: false, follows: false, lyrics: false, diary: false, looks: false, downloads: false, plays: false };
    match group {
        "settings" => g.settings = true,
        "library" => g.library = true,
        "follows" => g.follows = true,
        "lyrics" => g.lyrics = true,
        "looks" => g.looks = true,
        "downloads" => g.downloads = true,
        "plays" => g.plays = true,
        "diary" => g.diary = true,
        _ => {}
    }
    g
}

fn on(g: &Groups, group: &str) -> bool {
    match group {
        "settings" => g.settings,
        "library" => g.library,
        "follows" => g.follows,
        "lyrics" => g.lyrics,
        "looks" => g.looks,
        "downloads" => g.downloads,
        "plays" => g.plays,
        "diary" => g.diary,
        _ => false,
    }
}

/// The groups a first copy goes through here, in order: (group, progress
/// words, label).
pub fn steps(g: &Groups) -> Vec<(&'static str, &'static str, &'static str)> {
    STEPS.into_iter().filter(|(n, _, _)| on(g, n)).collect()
}

/// Lets go of this copy's stop flag when it ends however it ends. A stop
/// that comes later finds no copy, and sets nothing another round reads.
struct Running<'a>(&'a DeviceSync);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        *self.0.copy_stop.lock() = None;
    }
}

impl DeviceSync {
    /// Called as a first copy moves from one group to the next.
    pub fn set_on_copy(&self, f: OnCopy) {
        *self.on_copy.lock() = Some(f);
    }

    /// Stop a first copy that's running (what came stays).
    pub fn cancel_copy(&self) {
        if let Some(stop) = self.copy_stop.lock().as_ref() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    fn progress(&self, p: &CopyProgress) {
        if let Some(f) = self.on_copy.lock().clone() {
            f(p);
        }
    }

    /// Copy everything this device keeps in step from the paired computer:
    /// settings as they are there, then each other group. `wait` is how
    /// long the settings may take to come back through the relay.
    pub async fn copy_from_computer(&self, wait: Duration) -> AppResult<CopyResult> {
        if !self.is_on() {
            return Err(AppError::Invalid("Turn on Keep my devices in sync first.".into()));
        }
        if self.big_waits() {
            return Err(AppError::Invalid("Copying waits for Wi-Fi. Connect to Wi-Fi, or turn off Send big things only on Wi-Fi.".into()));
        }
        let Ok(_one) = self.copy_lock.try_lock() else {
            return Err(AppError::Invalid("A copy is already running.".into()));
        };
        // This copy's own stop flag: only its rounds read it.
        let stop = Arc::new(AtomicBool::new(false));
        *self.copy_stop.lock() = Some(stop.clone());
        let _running = Running(self);
        let stopped = || stop.load(Ordering::Relaxed);
        let (link, name) = self.pick_computer()?;
        let steps = steps(&(self.peer.deps().groups)());
        if steps.is_empty() {
            return Err(AppError::Invalid("Nothing is kept in sync on this device. Turn on something under What to sync first.".into()));
        }
        let of = steps.len();
        let mut groups = Vec::new();
        let mut later = None;
        let mut cancelled = false;
        for (i, (group, what, label)) in steps.into_iter().enumerate() {
            if stopped() {
                cancelled = true;
                break;
            }
            self.progress(&CopyProgress { name: name.clone(), step: i + 1, of, label: format!("Copying {what}") });
            let ns: Vec<&str> = only(group).namespaces();
            let before = self.peer.taken(&link.id, &ns);
            // Only this group's namespaces, so each step is its own round.
            let Some(hello) = self.peer.hello_for(&link.id, &ns).await else {
                return Err(AppError::Auth("Sign in to Plex to copy from your computer.".into()));
            };
            if group == "settings" {
                match self.copy_settings_on(&link, &name, Some(hello), wait, Some(&stop)).await {
                    Ok(()) => {}
                    // Not there: nothing else will come either.
                    Err(e @ AppError::Network(_)) => return Err(e),
                    Err(e) => {
                        if stopped() {
                            cancelled = true;
                            break;
                        }
                        // No settings there, or not kept in step there: the rest still comes.
                        tracing::info!("device sync: first copy without settings: {e}");
                    }
                }
            } else {
                let left = self.try_lan_with(&link, vec![hello], Some(&stop)).await;
                if !left.is_empty() {
                    // Not on this network: small changes come through the relay, the rest when they meet.
                    self.deliver(&link, left).await;
                    later = Some(if self.relay_allowed() {
                        format!("The rest comes over from {name} in the background, quickest when both are on the same Wi-Fi.")
                    } else {
                        format!("The rest comes over from {name} when both are on your home network.")
                    });
                }
            }
            if stopped() {
                cancelled = true;
            }
            groups.push(CopiedGroup { group: group.to_string(), label: label.to_string(), count: self.peer.taken(&link.id, &ns).saturating_sub(before) });
            if cancelled {
                break;
            }
        }
        // Every group named as it is here again: with a round when the copy
        // ran to its end, and only the hello (no round) when it was stopped.
        if let Some(h) = self.peer.hello_to(&link.id, cancelled).await {
            self.send(&link, vec![h]).await;
        }
        self.status_changed();
        Ok(CopyResult { name, groups, later, cancelled })
    }
}
