//! The music folder: where this computer sees the server's music files (a
//! mapped drive or UNC path on Windows, a mounted share on a Mac).
//!
//! Plex gives every file's path as the server sees it
//! (`/data/music/Artist/Album/01 Song.flac`). The part they share with the
//! music folder is the end of the path: the longest end of a server path
//! that's there under the music folder decides which start of the server
//! paths stands for the music folder. Paths are handled as text, so a Mac
//! can map a Windows server's paths and the other way round.

use serde::{Deserialize, Serialize};

/// Server paths starting with `server` are under `local` on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub server: String,
    pub local: String,
}

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// Windows paths: a drive letter, a UNC path, or backslashes.
fn windows_like(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') || p.starts_with("\\\\") || (p.contains('\\') && !p.contains('/'))
}

/// The separator paths under this folder use.
pub fn sep_of(local: &str) -> char {
    if windows_like(local) { '\\' } else { '/' }
}

/// A path's parts with where each starts.
fn parts(p: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in p.char_indices() {
        if is_sep(c) {
            if let Some(s) = start.take() {
                out.push((s, &p[s..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push((s, &p[s..]));
    }
    out
}

/// `local` with `rest` (parts) under it.
pub fn join(local: &str, rest: &[&str]) -> String {
    let sep = sep_of(local);
    let base = local.trim_end_matches(is_sep);
    let mut out = base.to_string();
    for r in rest {
        out.push(sep);
        out.push_str(r);
    }
    if out.is_empty() { sep.to_string() } else { out }
}

impl Mapping {
    /// Where a server path is on this computer, or None when it's outside
    /// the mapped folder.
    pub fn to_local(&self, server_path: &str) -> Option<String> {
        let case_blind = windows_like(&self.server) || windows_like(server_path);
        let mine = parts(&self.server);
        let theirs = parts(server_path);
        if theirs.len() <= mine.len() {
            return None;
        }
        let same = mine.iter().zip(&theirs).all(|((_, a), (_, b))| if case_blind { a.eq_ignore_ascii_case(b) } else { a == b });
        // "/data" and "data" differ: both absolute or neither.
        let rooted = |p: &str| p.starts_with(is_sep);
        if !same || rooted(&self.server) != rooted(server_path) {
            return None;
        }
        let rest: Vec<&str> = theirs[mine.len()..].iter().map(|(_, s)| *s).collect();
        Some(join(&self.local, &rest))
    }
}

/// The mapping for a music folder: the first sample whose end is found
/// under `local` (the longest end tried first) sets it. None when no
/// sample is there.
pub fn detect(local: &str, samples: &[String], exists: impl Fn(&str) -> bool) -> Option<Mapping> {
    for s in samples {
        let ps = parts(s);
        // Never the bare file name alone: at least its folder must match too.
        for k in 0..ps.len().saturating_sub(1) {
            let rest: Vec<&str> = ps[k..].iter().map(|(_, p)| *p).collect();
            if exists(&join(local, &rest)) {
                let server = s[..ps[k].0].trim_end_matches(is_sep);
                let server = if server.is_empty() && s.starts_with(is_sep) { &s[..1] } else { server };
                return Some(Mapping { server: server.to_string(), local: local.to_string() });
            }
        }
    }
    None
}

/// One file the check looked at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub server_path: String,
    pub local_path: Option<String>,
    pub found: bool,
    /// Found, but its size isn't what Plex says (another copy of the file).
    pub size_differs: bool,
}

/// The music folder's state for Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderCheck {
    pub ok: bool,
    pub found: usize,
    pub checked: usize,
    /// "Connected, 5 of 5 files found" or what's wrong.
    pub message: String,
    pub mapping: Option<Mapping>,
    pub probes: Vec<Probe>,
}

/// Checks a mapping against files Plex has (server path, size): each one
/// must be there with the size Plex has for it.
pub fn verify(mapping: Option<&Mapping>, files: &[(String, Option<u64>)], size_of: impl Fn(&str) -> Option<u64>) -> FolderCheck {
    let Some(m) = mapping else {
        return FolderCheck {
            ok: false,
            found: 0,
            checked: files.len(),
            message: "None of the songs Platter checked are in this folder. Pick the folder that holds your artist folders, or the folder above it.".into(),
            mapping: None,
            probes: Vec::new(),
        };
    };
    let mut probes = Vec::new();
    for (path, want) in files {
        let local = m.to_local(path);
        let size = local.as_deref().and_then(&size_of);
        probes.push(Probe {
            server_path: path.clone(),
            found: size.is_some(),
            size_differs: matches!((size, want), (Some(a), Some(b)) if a != *b),
            local_path: local,
        });
    }
    let found = probes.iter().filter(|p| p.found && !p.size_differs).count();
    let checked = probes.len();
    let differ = probes.iter().filter(|p| p.size_differs).count();
    let message = if checked == 0 {
        "Plex hasn't sent any songs yet. Check again after the library syncs.".to_string()
    } else if found == checked {
        format!("Connected, {found} of {checked} files found")
    } else if differ > 0 {
        format!(
            "{found} of {checked} files found. {differ} {} a different size than Plex has: this folder may hold another copy of your music.",
            if differ == 1 { "has" } else { "have" }
        )
    } else {
        format!("{found} of {checked} files found. The others aren't in this folder: it may only hold part of your library.")
    };
    FolderCheck { ok: checked > 0 && found == checked, found, checked, message, mapping: Some(m.clone()), probes }
}
