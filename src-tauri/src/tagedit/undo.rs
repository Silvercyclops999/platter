//! The last tag edits, with what each file held before, so they can be
//! undone while the files are as the edit left them.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::fields::Field;

/// Edits kept.
pub const KEEP: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoFile {
    pub track_id: String,
    pub server_path: String,
    pub local_path: String,
    /// The fields the edit changed and what they held (None: not set).
    pub before: BTreeMap<Field, Option<String>>,
    /// The file's stamp right after the edit.
    pub stamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoEntry {
    pub id: String,
    /// Unix ms.
    pub at: i64,
    /// "Edited tags on Demo Album", "Filled disc tags".
    pub label: String,
    /// The custom status tag the edit wrote, if any.
    pub custom_key: Option<String>,
    pub files: Vec<UndoFile>,
}

/// The log, kept in a JSON file. A log that won't read starts empty.
pub struct UndoLog {
    path: Option<PathBuf>,
    pub entries: Vec<UndoEntry>,
}

impl UndoLog {
    pub fn open(path: Option<PathBuf>) -> Self {
        let entries = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { path, entries }
    }

    fn save(&self) {
        let Some(p) = &self.path else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_vec(&self.entries) {
            Ok(b) => {
                let tmp = p.with_extension("json.tmp");
                if std::fs::write(&tmp, b).and_then(|_| std::fs::rename(&tmp, p)).is_err() {
                    tracing::warn!("tag edit log wasn't saved");
                }
            }
            Err(e) => tracing::warn!("tag edit log: {e}"),
        }
    }

    /// Newest first; the oldest past [`KEEP`] go.
    pub fn push(&mut self, e: UndoEntry) {
        self.entries.insert(0, e);
        self.entries.truncate(KEEP);
        self.save();
    }

    pub fn take(&mut self, id: &str) -> Option<UndoEntry> {
        let i = self.entries.iter().position(|e| e.id == id)?;
        let e = self.entries.remove(i);
        self.save();
        Some(e)
    }
}
