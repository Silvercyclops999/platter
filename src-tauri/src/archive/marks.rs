//! The user's marks and sections, kept as app state in the sync store, so
//! they follow the user to other devices (last change wins per item). They
//! are never written to files here.
//!
//! A mark answers "is this unreleased?" for an album or a song: Confirm
//! (unreleased) or Not unreleased (released). It beats every guess and
//! loses to the file's own tags (`db::release::assess`). The marks are
//! loaded into `db::release` so every place that asks gets the same answer.

use std::collections::HashMap;

use crate::db::release::{self, Mark};
use crate::error::{AppError, AppResult};
use crate::sync::SyncStore;
use crate::sync::store::{ALBUM_SECTIONS, RELEASE_MARKS};

use super::CustomSection;

/// Longest section name.
pub const SECTION_NAME_MAX: usize = 40;

/// Load every mark from the store into the release rule.
pub fn load(store: &SyncStore) -> AppResult<usize> {
    let marks: HashMap<String, Mark> = store.list(&RELEASE_MARKS)?.into_iter().collect();
    let n = marks.len();
    release::set_marks(marks);
    Ok(n)
}

/// Mark an album or a song (`kind` "album" or "track"), or clear the mark
/// with None. The rule sees it at once.
pub fn set(store: &SyncStore, kind: &str, id: &str, mark: Option<Mark>) -> AppResult<()> {
    if !matches!(kind, "album" | "track") || id.trim().is_empty() {
        return Err(AppError::Invalid("Only albums and songs can be marked.".into()));
    }
    let key = release::mark_key(kind, id);
    match mark {
        Some(m) => store.put(&RELEASE_MARKS, &key, &m)?,
        None => {
            store.remove(&RELEASE_MARKS, &key)?;
        }
    }
    load(store)?;
    Ok(())
}

/// A section name as stored: trimmed, 1 to [`SECTION_NAME_MAX`] characters.
pub fn section_name(name: &str) -> AppResult<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > SECTION_NAME_MAX || n.chars().any(char::is_control) {
        return Err(AppError::Invalid(format!("Section names are 1 to {SECTION_NAME_MAX} characters.")));
    }
    Ok(n.to_string())
}

/// Move an album to a section of the user's own, or back to its own
/// section with None.
pub fn set_section(store: &SyncStore, album_id: &str, artist_id: &str, name: Option<&str>) -> AppResult<()> {
    if album_id.trim().is_empty() {
        return Err(AppError::Invalid("No album to move.".into()));
    }
    match name {
        Some(n) => {
            let name = section_name(n)?;
            // The spelling the artist's other albums use, when the name is the same.
            let name = section_names(store, artist_id)?.into_iter().find(|x| x.eq_ignore_ascii_case(&name)).unwrap_or(name);
            store.put(&ALBUM_SECTIONS, album_id, &CustomSection { name, artist_id: artist_id.to_string() })
        }
        None => store.remove(&ALBUM_SECTIONS, album_id).map(|_| ()),
    }
}

/// Album id to section name, for one artist (or all with None).
pub fn custom_map(store: &SyncStore, artist_id: Option<&str>) -> AppResult<HashMap<String, String>> {
    Ok(store
        .list(&ALBUM_SECTIONS)?
        .into_iter()
        .filter(|(_, s)| artist_id.is_none_or(|a| s.artist_id == a))
        .map(|(k, s)| (k, s.name))
        .collect())
}

/// An artist's own section names, A to Z.
pub fn section_names(store: &SyncStore, artist_id: &str) -> AppResult<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    for (_, name) in custom_map(store, Some(artist_id))? {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    Ok(names)
}
