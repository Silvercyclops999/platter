//! Saved looks and themes, the same on every device. The look in use goes
//! with the synced settings (theme, accent, the theme file, cover light,
//! the readout face); these are the files in the themes folder to pick
//! from, one entry per file ([`LOOKS`], keyed by file name).
//!
//! - [`publish`] stores files added or changed here, and removes ones
//!   deleted here (a file this device had that's gone from the folder).
//! - [`apply`] writes files from other devices into the folder, checked
//!   like an imported file first, and deletes ones deleted elsewhere.
//!
//! A look file carries a layout, but taking a look file never changes this
//! device's layout: that's the Layout setting, which never syncs.

use std::path::Path;

use serde_json::Value;

use super::store::{LOOKS, Op, SyncStore};
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};
use crate::themes::{self, LOOK_EXT, ThemeFile};

/// The files this device had in the folder after the last publish or apply.
const KNOWN_KEY: &str = "sync.looks.known";

/// A plain file name in the themes folder: no folders, nothing hidden, a
/// look or theme file.
fn safe_name(file: &str) -> bool {
    let lower = file.to_ascii_lowercase();
    !file.is_empty()
        && file.len() <= 120
        && !file.starts_with('.')
        && !file.contains(['/', '\\', ':', '\0'])
        && (lower.ends_with(&format!(".{LOOK_EXT}")) || lower.ends_with(".json"))
}

fn known(store: &SyncStore) -> Vec<String> {
    store.db().read(|c| kv_get(c, KNOWN_KEY)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn set_known(store: &SyncStore, mut files: Vec<String>) -> AppResult<()> {
    files.sort();
    files.dedup();
    let json = serde_json::to_string(&files).map_err(|e| AppError::Internal(e.to_string()))?;
    store.db().write(|c| kv_set(c, KNOWN_KEY, &json))
}

/// The text a theme file is written with: a look's own text, or a plain
/// theme's name, base and tokens.
fn file_text(t: &ThemeFile) -> String {
    if t.is_look() {
        return themes::look_text(t);
    }
    let v = serde_json::json!({ "name": t.name, "base": t.base, "tokens": t.tokens });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// What a file is, as stored: the checked file with nothing left out.
fn stored(t: &ThemeFile) -> ThemeFile {
    ThemeFile { notes: Vec::new(), ..t.clone() }
}

/// Store this device's look and theme files that changed, and remove the
/// ones deleted here. Files that can't be used aren't stored. Returns how
/// many entries changed.
pub fn publish(store: &SyncStore, dir: &Path) -> AppResult<usize> {
    let folder = themes::list_dir(dir);
    let mut ops = Vec::new();
    let mut here = Vec::new();
    for t in &folder.themes {
        if !safe_name(&t.file) {
            continue;
        }
        here.push(t.file.clone());
        let t = stored(t);
        if store.get(&LOOKS, &t.file)?.as_ref() != Some(&t) {
            ops.push(Op::put(&LOOKS, t.file.clone(), &t)?);
        }
    }
    // Unreadable files are still there: not deleted.
    here.extend(folder.invalid.iter().map(|i| i.file.clone()));
    for gone in known(store).into_iter().filter(|f| !here.contains(f)) {
        if store.get(&LOOKS, &gone)?.is_some() {
            ops.push(Op::remove(&LOOKS, gone));
        }
    }
    let n = ops.len();
    store.write_many(ops)?;
    set_known(store, here)?;
    Ok(n)
}

/// Write the looks other devices saved into the folder, and delete the ones
/// deleted elsewhere. A file that fails the checks is left out, and a file
/// here that was never shared isn't touched. Returns how many files changed.
pub fn apply(store: &SyncStore, dir: &Path) -> AppResult<usize> {
    let mut known = known(store);
    let mut changed = 0;
    for (key, t) in store.list(&LOOKS)? {
        if !safe_name(&key) || t.file != key {
            tracing::debug!("looks: skipped a file with an odd name");
            continue;
        }
        if let Err(why) = themes::check(&t) {
            tracing::info!("looks: a look from another device can't be used: {why}");
            continue;
        }
        let text = file_text(&t);
        if text.len() > themes::FILE_MAX_BYTES {
            continue;
        }
        let path = dir.join(&key);
        let same = std::fs::read_to_string(&path).ok().and_then(|old| themes::validate(&key, &old).ok()).is_some_and(|old| stored(&old) == t);
        if !same {
            std::fs::create_dir_all(dir)?;
            std::fs::write(&path, text)?;
            changed += 1;
        }
        known.push(key);
    }
    for key in store.deleted(&LOOKS)? {
        if !safe_name(&key) {
            continue;
        }
        let path = dir.join(&key);
        // Only a file that came through sync or was shared from here.
        if known.contains(&key) && path.is_file() {
            std::fs::remove_file(&path)?;
            changed += 1;
        }
        known.retain(|k| k != &key);
    }
    set_known(store, known)?;
    Ok(changed)
}

/// Whether a stored value is a look file this build can use (for checks).
pub fn usable(v: &Value) -> bool {
    serde_json::from_value::<ThemeFile>(v.clone()).is_ok_and(|t| safe_name(&t.file) && themes::check(&t).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbHandle;

    const LOOK: &str = r##"{"version":1,"name":"Dusk","author":"Sam","base":"dark","tokens":{"color-accent":"#c08a3e"},"style":{"density":"comfortable","shadow":"soft","blur":"off","border":1}}"##;

    fn store(dir: &Path, id: &str) -> SyncStore {
        SyncStore::new(DbHandle::open(dir.join(format!("{id}.db"))).unwrap(), id)
    }

    #[test]
    fn a_look_saved_on_one_device_appears_on_the_other_and_a_delete_follows() {
        let tmp = tempfile::tempdir().unwrap();
        let (pc_dir, phone_dir) = (tmp.path().join("pc"), tmp.path().join("phone"));
        let (pc, phone) = (store(tmp.path(), "pc"), store(tmp.path(), "phone"));
        let saved = themes::import_text(&pc_dir, LOOK).unwrap();
        assert_eq!(publish(&pc, &pc_dir).unwrap(), 1);
        assert_eq!(publish(&pc, &pc_dir).unwrap(), 0, "nothing new");
        // As device sync or the sync file would carry it.
        let entries = pc.entries(&[LOOKS.name()]).unwrap();
        phone.merge(&entries, &[LOOKS.name()]).unwrap();
        assert_eq!(apply(&phone, &phone_dir).unwrap(), 1);
        let there = themes::list_dir(&phone_dir);
        assert_eq!(there.themes.len(), 1);
        assert_eq!(there.themes[0].name, "Dusk");
        assert_eq!(there.themes[0].file, saved.file);
        assert_eq!(apply(&phone, &phone_dir).unwrap(), 0, "written once");
        assert_eq!(publish(&phone, &phone_dir).unwrap(), 0, "doesn't echo back");
        // Deleted on the phone: gone from the computer too.
        std::fs::remove_file(phone_dir.join(&saved.file)).unwrap();
        assert_eq!(publish(&phone, &phone_dir).unwrap(), 1);
        let entries = phone.entries(&[LOOKS.name()]).unwrap();
        pc.merge(&entries, &[LOOKS.name()]).unwrap();
        assert_eq!(apply(&pc, &pc_dir).unwrap(), 1);
        assert!(themes::list_dir(&pc_dir).themes.is_empty());
    }

    #[test]
    fn a_bad_or_odd_look_from_another_device_is_left_out() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        let s = store(tmp.path(), "phone");
        let good = themes::validate("Dusk.platter-look", LOOK).unwrap();
        for key in ["../evil.platter-look", ".hidden.json", "notes.txt", "a/b.json"] {
            s.put(&LOOKS, key, &ThemeFile { file: key.into(), ..good.clone() }).unwrap();
        }
        // A name that doesn't match its key, and a look with CSS in a token.
        s.put(&LOOKS, "Other.platter-look", &good).unwrap();
        let mut css = ThemeFile { file: "Css.platter-look".into(), ..good.clone() };
        css.tokens.insert("color-accent".into(), "url(javascript:1)".into());
        s.put(&LOOKS, "Css.platter-look", &css).unwrap();
        assert_eq!(apply(&s, &dir).unwrap(), 0);
        assert!(!tmp.path().join("evil.platter-look").exists());
        assert!(themes::list_dir(&dir).themes.is_empty());
        assert!(!usable(&serde_json::to_value(&css).unwrap()));
        assert!(usable(&serde_json::to_value(ThemeFile { file: "Dusk.platter-look".into(), ..good }).unwrap()));
    }

    #[test]
    fn a_file_never_shared_isnt_deleted_by_a_delete_from_elsewhere() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        let s = store(tmp.path(), "phone");
        let mine = themes::import_text(&dir, LOOK).unwrap();
        // Another device deleted a file of the same name it had shared.
        s.put(&LOOKS, &mine.file, &stored(&mine)).unwrap();
        s.remove(&LOOKS, &mine.file).unwrap();
        apply(&s, &dir).unwrap();
        assert!(dir.join(&mine.file).is_file(), "this device's own file stays");
    }
}
