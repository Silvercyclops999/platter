//! Writing one file safely.
//!
//! 1. Read the file again; refuse when it changed since the editor read it
//!    (its size, modified time or tags differ from the stamp).
//! 2. Copy it to a temp file in the same folder and write the tags there.
//! 3. Read the copy back: the tags must be what was asked and its length
//!    the original's.
//! 4. Check the original once more, then replace it with the copy.
//!
//! Any failure leaves the original as it was and removes the copy. The
//! file keeps its name and folder.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Values;
use super::doc::{Ctx, Doc};
use super::fields::Field;

/// Changes for one file: a field's new value, empty to remove it.
pub type Edits = BTreeMap<Field, String>;

/// Files in the folder changed this recently by something else mean
/// another program is at work there.
pub const BUSY_WINDOW: Duration = Duration::from_secs(60);

/// What a file is like when read: size, modified time and its tags.
pub fn stamp(path: &Path, values: &Values) -> io::Result<String> {
    let m = std::fs::metadata(path)?;
    let mtime = m.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).unwrap_or_default();
    let mut h = Sha256::new();
    for (f, v) in values {
        h.update(format!("{f:?}={v}\n").as_bytes());
    }
    let hex: String = h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect();
    Ok(format!("{}:{}:{hex}", m.len(), mtime.as_nanos()))
}

/// Why a file wasn't written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Refusal {
    /// It changed since the editor read it.
    Changed,
    /// Another program changed its folder in the last minute.
    Busy,
    /// The share is read-only or Platter may not write there.
    ReadOnly,
    /// Another program has it open.
    Locked,
    /// It isn't there any more.
    Gone,
    /// Anything else (a tag that wouldn't write, a copy that didn't check out).
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub why: Refusal,
    pub message: String,
}

impl Refused {
    fn new(why: Refusal, message: impl Into<String>) -> Self {
        Self { why, message: message.into() }
    }
}

/// What the user reads for a failed file operation.
pub fn refusal_for(e: &io::Error, name: &str) -> Refused {
    // Windows: ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
    if matches!(e.raw_os_error(), Some(32 | 33)) || e.kind() == io::ErrorKind::ResourceBusy {
        return Refused::new(Refusal::Locked, format!("{name} is open in another program. Close it and try again."));
    }
    match e.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => Refused::new(
            Refusal::ReadOnly,
            format!("Platter can't write to the folder {name} is in. The share may be read-only for this account."),
        ),
        io::ErrorKind::NotFound => {
            Refused::new(Refusal::Gone, format!("{name} isn't in the music folder any more. Check that the share is connected."))
        }
        _ => Refused::new(Refusal::Failed, format!("{name} wasn't saved: {e}")),
    }
}

/// Points in a write where tests can make it fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Copy,
    Tags,
    Replace,
}

pub trait Hooks: Send + Sync {
    fn at(&self, _stage: Stage, _temp: &Path) -> io::Result<()> {
        Ok(())
    }
}

pub struct NoHooks;
impl Hooks for NoHooks {}

/// Files Platter itself wrote and when, so its own writes don't make a
/// folder look busy.
#[derive(Default)]
pub struct Recent(Mutex<HashMap<PathBuf, SystemTime>>);

pub static RECENT: LazyLock<Recent> = LazyLock::new(Recent::default);

impl Recent {
    pub fn note(&self, path: &Path) {
        if let Ok(t) = std::fs::metadata(path).and_then(|m| m.modified()) {
            let mut m = self.0.lock();
            m.retain(|_, at| at.elapsed().map(|e| e < BUSY_WINDOW * 2).unwrap_or(true));
            m.insert(path.to_path_buf(), t);
        }
    }

    fn ours(&self, path: &Path, t: SystemTime) -> bool {
        self.0.lock().get(path).is_some_and(|at| *at == t)
    }
}

/// A temp file this module made (left behind only if Platter was closed mid-write).
pub fn is_temp(name: &str) -> bool {
    name.starts_with('.') && name.contains(".platter-tmp-")
}

/// Whether another program changed a file in this folder within
/// [`BUSY_WINDOW`]. Times far in the future (a server clock that's off) don't count.
pub fn folder_busy(dir: &Path, now: SystemTime, recent: &Recent) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if is_temp(&name) {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let Ok(t) = m.modified() else { continue };
        let fresh = match now.duration_since(t) {
            Ok(age) => age < BUSY_WINDOW,
            Err(ahead) => ahead.duration() < BUSY_WINDOW,
        };
        if fresh && !recent.ours(&e.path(), t) {
            return true;
        }
    }
    false
}

pub const BUSY_MESSAGE: &str = "Another program is changing this folder. Try again shortly.";

/// One file written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The fields that changed and what they held before (absent: not set).
    pub before: BTreeMap<Field, Option<String>>,
    /// Every field after the write.
    pub after: Values,
    /// The file's stamp after the write.
    pub stamp: String,
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
}

fn temp_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    let tag = uuid::Uuid::new_v4().simple().to_string();
    path.with_file_name(format!(".{stem}.platter-tmp-{}{ext}", &tag[..8]))
}

/// The length of a file's audio, in ms.
fn length_ms(path: &Path) -> Option<u128> {
    use lofty::file::AudioFile;
    lofty::read_from_path(path).ok().map(|f| f.properties().duration().as_millis())
}

/// Writes `edits` to the file at `path` when it's still as `expected` says.
/// Returns Ok(None) when nothing would change.
pub fn write_file(path: &Path, expected: &str, edits: &Edits, ctx: &Ctx, hooks: &dyn Hooks) -> Result<Option<Written>, Refused> {
    let name = name_of(path);
    let mut doc = match Doc::read(path) {
        Ok(d) => d,
        Err(_) if !path.exists() => return Err(refusal_for(&io::Error::from(io::ErrorKind::NotFound), &name)),
        Err(e) => return Err(Refused::new(Refusal::Failed, e.to_string())),
    };
    let values = doc.values(ctx);
    let now = stamp(path, &values).map_err(|e| refusal_for(&e, &name))?;
    if now != expected {
        return Err(Refused::new(Refusal::Changed, format!("{name} changed since the editor opened. Open it again to see the new tags.")));
    }
    let mut before = BTreeMap::new();
    let mut want = values.clone();
    for (f, v) in edits {
        let old = values.get(f).cloned();
        if old.as_deref().unwrap_or("") == v.as_str() {
            continue;
        }
        before.insert(*f, old);
        if v.is_empty() {
            want.remove(f);
        } else {
            want.insert(*f, v.clone());
        }
    }
    if before.is_empty() {
        return Ok(None);
    }
    for f in before.keys() {
        doc.set(*f, edits.get(f).map(String::as_str).unwrap_or(""), ctx)
            .map_err(|e| Refused::new(Refusal::Failed, e.to_string()))?;
    }

    let temp = temp_path(path);
    let cleanup = |r: Refused| {
        let _ = std::fs::remove_file(&temp);
        r
    };
    hooks.at(Stage::Copy, &temp).and_then(|_| std::fs::copy(path, &temp)).map_err(|e| cleanup(refusal_for(&e, &name)))?;
    hooks.at(Stage::Tags, &temp).map_err(|e| cleanup(refusal_for(&e, &name)))?;
    doc.save(&temp).map_err(|e| cleanup(Refused::new(Refusal::Failed, format!("{name} wasn't saved: {e}"))))?;

    // The copy must read back as asked, with the original's length.
    let back = Doc::read(&temp).map(|d| d.values(ctx)).map_err(|e| cleanup(Refused::new(Refusal::Failed, e.to_string())))?;
    if back != want {
        let diff: Vec<&str> =
            Field::ALL.iter().filter(|f| back.get(f) != want.get(f)).map(|f| f.label()).collect();
        return Err(cleanup(Refused::new(
            Refusal::Failed,
            format!("{name} wasn't saved: {} didn't read back as written.", diff.join(", ")),
        )));
    }
    if length_ms(&temp) != length_ms(path) {
        return Err(cleanup(Refused::new(Refusal::Failed, format!("{name} wasn't saved: the copy's length didn't match."))));
    }

    // Last look at the original before it's replaced.
    let again = Doc::read(path).map(|d| d.values(ctx)).ok().and_then(|v| stamp(path, &v).ok());
    if again.as_deref() != Some(expected) {
        return Err(cleanup(Refused::new(Refusal::Changed, format!("{name} changed while saving. Nothing was written to it."))));
    }
    hooks.at(Stage::Replace, &temp).and_then(|_| replace(&temp, path)).map_err(|e| cleanup(refusal_for(&e, &name)))?;
    RECENT.note(path);
    let stamp = stamp(path, &want).map_err(|e| refusal_for(&e, &name))?;
    Ok(Some(Written { before, after: want, stamp }))
}

/// Moves the temp file over the original, keeping its permissions.
fn replace(temp: &Path, path: &Path) -> io::Result<()> {
    if let Ok(m) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(temp, m.permissions());
    }
    std::fs::rename(temp, path)
}

/// Reads a file for the editor: its fields and stamp.
pub fn read_file(path: &Path, ctx: &Ctx) -> Result<(Doc, Values, String), Refused> {
    let name = name_of(path);
    let doc = match Doc::read(path) {
        Ok(d) => d,
        Err(_) if !path.exists() => return Err(refusal_for(&io::Error::from(io::ErrorKind::NotFound), &name)),
        Err(e) => return Err(Refused::new(Refusal::Failed, e.to_string())),
    };
    let values = doc.values(ctx);
    let s = stamp(path, &values).map_err(|e| refusal_for(&e, &name))?;
    Ok((doc, values, s))
}
