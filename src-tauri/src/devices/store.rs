//! The devices this computer knows, kept in the db's `kv` table as one JSON
//! list. They stay on this computer (sync files don't carry them).

use crate::db::DbHandle;
use crate::db::store::{kv_get, kv_set};
use crate::error::{AppError, AppResult};

use super::{Device, NAME_MAX, SelItem, SelKind};

const KEY: &str = "devices.list";

fn parse(s: Option<String>) -> Vec<Device> {
    s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn load(db: &DbHandle) -> Vec<Device> {
    db.read(|c| kv_get(c, KEY)).ok().map(parse).unwrap_or_default()
}

#[cfg(test)]
pub fn save(db: &DbHandle, list: &[Device]) -> AppResult<()> {
    edit(db, |l| {
        *l = list.to_vec();
        Ok(())
    })
}

/// Reads the list, changes it and saves it while holding the db's writer,
/// so two changes at once (a rename while a sync notes its time) can't undo
/// each other. `f` must not use the db.
pub fn edit<T>(db: &DbHandle, f: impl FnOnce(&mut Vec<Device>) -> AppResult<T>) -> AppResult<T> {
    db.write(|c| {
        let mut list = parse(kv_get(c, KEY)?);
        let out = f(&mut list)?;
        let json = serde_json::to_string(&list).map_err(|e| AppError::Internal(e.to_string()))?;
        kv_set(c, KEY, &json)?;
        Ok(out)
    })
}

/// Changes one device and saves the list. Returns the changed device.
pub fn update(db: &DbHandle, id: &str, f: impl FnOnce(&mut Device) -> AppResult<()>) -> AppResult<Device> {
    edit(db, |list| {
        let d = list
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or_else(|| AppError::NotFound("That device isn't on this computer any more.".into()))?;
        f(d)?;
        Ok(d.clone())
    })
}

/// A device name as typed: trimmed, at most [`NAME_MAX`] characters.
pub fn clean_name(s: &str) -> AppResult<String> {
    let n: String = s.trim().chars().take(NAME_MAX).collect();
    if n.is_empty() {
        return Err(AppError::Invalid("Give the device a name.".into()));
    }
    Ok(n)
}

/// Adds or removes one item in a device's selection. Adding keeps the
/// order things were chosen in; an item already there stays where it is.
pub fn set_selected(d: &mut Device, kind: SelKind, id: &str, title: &str, on: bool) {
    let id = if kind == SelKind::Liked { "" } else { id };
    let at = d.selection.iter().position(|s| s.same(kind, id));
    match (on, at) {
        (true, None) => d.selection.push(SelItem {
            kind,
            id: id.to_string(),
            title: if kind == SelKind::Liked { "Liked Songs".into() } else { title.to_string() },
        }),
        (false, Some(i)) => {
            d.selection.remove(i);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Device {
        Device {
            id: "d1".into(),
            name: "Player".into(),
            volume_id: Some("ABCD-1234".into()),
            volume_label: "PLAYER".into(),
            folder: "Music".into(),
            format: None,
            budget_gb: None,
            selection: vec![],
            last_sync: None,
            last_root: None,
        }
    }

    #[test]
    fn devices_round_trip_through_the_db() {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("d.db")).unwrap();
        assert!(load(&db).is_empty());
        save(&db, &[device()]).unwrap();
        assert_eq!(load(&db), vec![device()]);
        let d = update(&db, "d1", |d| {
            d.name = "Car".into();
            Ok(())
        })
        .unwrap();
        assert_eq!(d.name, "Car");
        assert_eq!(load(&db)[0].name, "Car");
        assert!(update(&db, "nope", |_| Ok(())).is_err());
    }

    #[test]
    fn changes_at_the_same_time_all_land() {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("d.db")).unwrap();
        save(&db, &[device()]).unwrap();
        let threads: Vec<_> = (0..16)
            .map(|i| {
                let db = db.clone();
                std::thread::spawn(move || {
                    update(&db, "d1", |d| {
                        // Slow enough that unguarded writers would overlap.
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        set_selected(d, SelKind::Album, &format!("a{i}"), "x", true);
                        Ok(())
                    })
                    .unwrap();
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(load(&db)[0].selection.len(), 16);
    }

    #[test]
    fn a_change_that_fails_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("d.db")).unwrap();
        save(&db, &[device()]).unwrap();
        let r = update(&db, "d1", |d| {
            d.name = "Half done".into();
            Err(AppError::Invalid("no".into()))
        });
        assert!(r.is_err());
        assert_eq!(load(&db)[0].name, "Player");
    }

    #[test]
    fn names_are_trimmed_and_not_empty() {
        assert_eq!(clean_name("  Car stereo ").unwrap(), "Car stereo");
        assert!(clean_name("   ").is_err());
        assert_eq!(clean_name(&"x".repeat(100)).unwrap().chars().count(), NAME_MAX);
    }

    #[test]
    fn selection_keeps_the_order_chosen() {
        let mut d = device();
        set_selected(&mut d, SelKind::Album, "a1", "First", true);
        set_selected(&mut d, SelKind::Liked, "ignored", "", true);
        set_selected(&mut d, SelKind::Playlist, "p1", "Road", true);
        set_selected(&mut d, SelKind::Album, "a1", "First", true);
        let order: Vec<_> = d.selection.iter().map(|s| (s.kind, s.id.as_str())).collect();
        assert_eq!(order, vec![(SelKind::Album, "a1"), (SelKind::Liked, ""), (SelKind::Playlist, "p1")]);
        assert_eq!(d.selection[1].title, "Liked Songs");
        set_selected(&mut d, SelKind::Liked, "", "", false);
        assert_eq!(d.selection.len(), 2);
    }
}
