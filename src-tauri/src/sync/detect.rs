//! Finding the cloud folders this computer already syncs (OneDrive, iCloud
//! Drive, Google Drive, Dropbox), to suggest one for the sync file. This
//! only looks: it never creates or writes anything.

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFolder {
    /// OneDrive, iCloud Drive, Google Drive or Dropbox.
    pub provider: String,
    pub path: String,
}

/// What detection reads, so tests can fake a computer.
pub struct Probe<'a> {
    pub home: Option<PathBuf>,
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub is_dir: &'a dyn Fn(&Path) -> bool,
    pub read: &'a dyn Fn(&Path) -> Option<String>,
    pub list: &'a dyn Fn(&Path) -> Vec<PathBuf>,
    pub windows: bool,
}

fn push(out: &mut Vec<CloudFolder>, provider: &str, p: PathBuf) {
    let path = p.to_string_lossy().to_string();
    if !out.iter().any(|f| f.path.eq_ignore_ascii_case(&path)) {
        out.push(CloudFolder { provider: provider.into(), path });
    }
}

/// Dropbox writes where its folder is to `info.json`.
fn dropbox_from_info(json: &str) -> Vec<PathBuf> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    ["personal", "business"]
        .iter()
        .filter_map(|k| v[*k]["path"].as_str())
        .map(PathBuf::from)
        .collect()
}

/// The cloud folders found, in a stable order.
pub fn detect(p: &Probe) -> Vec<CloudFolder> {
    let mut out = Vec::new();
    let dir = |path: &Path| (p.is_dir)(path);
    if p.windows {
        for var in ["OneDriveConsumer", "OneDrive", "OneDriveCommercial"] {
            if let Some(v) = (p.env)(var).filter(|v| !v.is_empty()) {
                let path = PathBuf::from(v);
                if dir(&path) {
                    push(&mut out, "OneDrive", path);
                }
            }
        }
        if let Some(h) = &p.home {
            let icloud = h.join("iCloudDrive");
            if dir(&icloud) {
                push(&mut out, "iCloud Drive", icloud);
            }
        }
        // Google Drive for desktop mounts as G: by default. Other letters
        // aren't tried: a disconnected network drive can stall the check.
        let my = PathBuf::from("G:\\My Drive");
        if dir(&my) {
            push(&mut out, "Google Drive", my);
        }
        if let Some(h) = &p.home {
            let g = h.join("Google Drive");
            if dir(&g) {
                push(&mut out, "Google Drive", g);
            }
        }
        for base in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(b) = (p.env)(base)
                && let Some(json) = (p.read)(&PathBuf::from(b).join("Dropbox").join("info.json"))
            {
                for d in dropbox_from_info(&json) {
                    if dir(&d) {
                        push(&mut out, "Dropbox", d);
                    }
                }
            }
        }
    } else if let Some(h) = &p.home {
        let icloud = h.join("Library").join("Mobile Documents").join("com~apple~CloudDocs");
        if dir(&icloud) {
            push(&mut out, "iCloud Drive", icloud);
        }
        // Newer OneDrive, Google Drive and Dropbox live under CloudStorage.
        let mut cloud = (p.list)(&h.join("Library").join("CloudStorage"));
        cloud.sort();
        for d in cloud {
            let name = d.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
            if name.starts_with("OneDrive") && dir(&d) {
                push(&mut out, "OneDrive", d);
            } else if name.starts_with("GoogleDrive") {
                let my = d.join("My Drive");
                if dir(&my) {
                    push(&mut out, "Google Drive", my);
                }
            } else if name.starts_with("Dropbox") && dir(&d) {
                push(&mut out, "Dropbox", d);
            }
        }
        if let Some(json) = (p.read)(&h.join(".dropbox").join("info.json")) {
            for d in dropbox_from_info(&json) {
                if dir(&d) {
                    push(&mut out, "Dropbox", d);
                }
            }
        }
        let dropbox = h.join("Dropbox");
        if dir(&dropbox) {
            push(&mut out, "Dropbox", dropbox);
        }
    }
    out
}

/// The cloud folders on this computer.
pub fn detect_here() -> Vec<CloudFolder> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    detect(&Probe {
        home,
        env: &|k| std::env::var(k).ok(),
        is_dir: &|p| p.is_dir(),
        read: &|p| std::fs::read_to_string(p).ok(),
        list: &|p| {
            std::fs::read_dir(p)
                .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
                .unwrap_or_default()
        },
        windows: cfg!(windows),
    })
}
