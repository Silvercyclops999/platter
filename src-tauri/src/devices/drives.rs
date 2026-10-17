//! The drives mounted on this computer: removable ones are offered as new
//! devices, and every one is checked when looking for a known device.
//!
//! Windows: drive letters, with the volume serial number. macOS: the
//! volumes under /Volumes other than the start-up disk, with the volume
//! UUID from `diskutil`. Linux: /media and /run/media, no id (the manifest
//! on the drive recognises it instead).

use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    /// Where it's mounted (`E:\`, `/Volumes/PLAYER`).
    pub root: PathBuf,
    /// Its own name ("" when it has none).
    pub label: String,
    /// Serial number or UUID, when the system gives one.
    pub id: Option<String>,
    /// A USB stick, a card, a player: something meant to be unplugged.
    pub removable: bool,
    pub free: Option<u64>,
    pub total: Option<u64>,
}

/// Every mounted volume (slow-ish: call it when looking, not in a loop).
pub fn list() -> Vec<Volume> {
    let mut v = imp::list();
    for x in &mut v {
        x.free = fs4::available_space(&x.root).ok();
        x.total = fs4::total_space(&x.root).ok();
    }
    v
}

/// The volume a path is on (the longest root it starts with).
pub fn volume_of(path: &std::path::Path, volumes: &[Volume]) -> Option<Volume> {
    let lower = |p: &std::path::Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let p = lower(path);
    volumes
        .iter()
        .filter(|v| p.starts_with(lower(&v.root)))
        .max_by_key(|v| v.root.as_os_str().len())
        .cloned()
}

#[cfg(windows)]
mod imp {
    use super::Volume;
    use std::path::PathBuf;
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    use windows::core::PCWSTR;

    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;

    pub fn list() -> Vec<Volume> {
        // SAFETY: no arguments.
        let mask = unsafe { GetLogicalDrives() };
        let mut out = Vec::new();
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let root = format!("{}:\\", (b'A' + i as u8) as char);
            let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: a NUL-terminated root path that lives through the calls.
            let kind = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
            if kind != DRIVE_REMOVABLE && kind != DRIVE_FIXED {
                continue;
            }
            let mut name = [0u16; 261];
            let mut serial = 0u32;
            // SAFETY: buffers sized as passed; an empty card slot just fails.
            let ok = unsafe { GetVolumeInformationW(PCWSTR(wide.as_ptr()), Some(&mut name), Some(&mut serial), None, None, None) };
            if ok.is_err() {
                continue;
            }
            let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
            out.push(Volume {
                root: PathBuf::from(&root),
                label: String::from_utf16_lossy(&name[..len]),
                id: Some(format!("{:04X}-{:04X}", serial >> 16, serial & 0xFFFF)),
                removable: kind == DRIVE_REMOVABLE,
                free: None,
                total: None,
            });
        }
        out
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Volume;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    fn uuid(root: &Path) -> Option<String> {
        let out = std::process::Command::new("/usr/sbin/diskutil").arg("info").arg(root).output().ok()?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.trim().strip_prefix("Volume UUID:").map(|v| v.trim().to_string()))
            .filter(|v| !v.is_empty())
    }

    pub fn list() -> Vec<Volume> {
        let boot = std::fs::metadata("/").map(|m| m.dev()).ok();
        let Ok(rd) = std::fs::read_dir("/Volumes") else { return vec![] };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let root = e.path();
            let Ok(meta) = std::fs::metadata(&root) else { continue };
            if !meta.is_dir() || Some(meta.dev()) == boot {
                continue;
            }
            out.push(Volume {
                label: e.file_name().to_string_lossy().into_owned(),
                id: uuid(&root),
                root,
                removable: true,
                free: None,
                total: None,
            });
        }
        out
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::Volume;

    pub fn list() -> Vec<Volume> {
        let user = std::env::var("USER").unwrap_or_default();
        let mut out = Vec::new();
        for base in [format!("/media/{user}"), format!("/run/media/{user}")] {
            let Ok(rd) = std::fs::read_dir(&base) else { continue };
            for e in rd.flatten() {
                if e.path().is_dir() {
                    out.push(Volume {
                        root: e.path(),
                        label: e.file_name().to_string_lossy().into_owned(),
                        id: None,
                        removable: true,
                        free: None,
                        total: None,
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(root: &str) -> Volume {
        Volume { root: PathBuf::from(root), label: String::new(), id: None, removable: true, free: None, total: None }
    }

    #[test]
    fn a_path_belongs_to_its_longest_root() {
        let vols = [vol("/Volumes/A"), vol("/Volumes/AB"), vol("/")];
        assert_eq!(volume_of(std::path::Path::new("/Volumes/AB/Music"), &vols).unwrap().root, PathBuf::from("/Volumes/AB"));
        assert_eq!(volume_of(std::path::Path::new("/home/x"), &vols).unwrap().root, PathBuf::from("/"));
    }

    #[test]
    fn listing_drives_does_not_fail() {
        // Whatever this machine has; it must simply not panic.
        let _ = list();
    }
}
