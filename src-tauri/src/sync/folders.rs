//! Library folders. Plex has no folders, so Platter keeps its own: folders
//! (which can hold folders) and which folder each playlist is in. Stored in
//! the sync store, so they travel in the sync file when it's on.
//!
//! Every folder and every placement is its own entry, so edits on two
//! devices merge. Reading tidies what a merge can leave behind: a folder
//! whose parent is gone shows at the top, a parent loop is broken, and a
//! playlist placed in a deleted folder shows at the top.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::store::{FOLDER_ITEMS, FOLDERS, Op, SyncStore};
use crate::error::{AppError, AppResult};

pub const NEW_NAME: &str = "New Folder";
pub const NAME_MAX: usize = 100;
/// How deep folders nest.
pub const DEPTH_MAX: usize = 8;

/// A folder as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub name: String,
    /// The folder it's in; None at the top of the library.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub created_at: u64,
}

/// Where a playlist is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub folder: String,
}

/// A folder for the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub created_at: u64,
}

/// Every folder and which playlists are in them.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folders {
    /// Newest first.
    pub folders: Vec<FolderView>,
    /// Playlist id → folder id (playlists at the top aren't listed).
    pub items: HashMap<String, String>,
}

impl Folders {
    fn get(&self, id: &str) -> Option<&FolderView> {
        self.folders.iter().find(|f| f.id == id)
    }

    /// The folder and every folder above it.
    pub fn ancestors(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut at = self.get(id).and_then(|f| f.parent.clone());
        while let Some(p) = at {
            if out.contains(&p) {
                break;
            }
            at = self.get(&p).and_then(|f| f.parent.clone());
            out.push(p);
        }
        out
    }
}

/// Read and tidy the folders.
pub fn tree(store: &SyncStore) -> AppResult<Folders> {
    let stored: HashMap<String, Folder> = store.list(&FOLDERS)?.into_iter().collect();
    let mut parent: HashMap<String, Option<String>> = stored
        .iter()
        .map(|(id, f)| (id.clone(), f.parent.clone().filter(|p| stored.contains_key(p) && p != id)))
        .collect();
    // Break loops (two devices moving folders into each other): the folder
    // in a loop with the smallest id goes to the top.
    let mut ids: Vec<String> = stored.keys().cloned().collect();
    ids.sort();
    for id in &ids {
        let mut seen = HashSet::new();
        let mut at = Some(id.clone());
        while let Some(cur) = at {
            if !seen.insert(cur.clone()) {
                let looped: Vec<String> = {
                    let mut l = vec![cur.clone()];
                    let mut n = parent[&cur].clone();
                    while let Some(x) = n {
                        if x == cur {
                            break;
                        }
                        l.push(x.clone());
                        n = parent[&x].clone();
                    }
                    l
                };
                if let Some(min) = looped.iter().min() {
                    parent.insert(min.clone(), None);
                }
                break;
            }
            at = parent.get(&cur).cloned().flatten();
        }
    }
    let mut folders: Vec<FolderView> = stored
        .into_iter()
        .map(|(id, f)| FolderView { parent: parent.remove(&id).flatten(), id, name: f.name, created_at: f.created_at })
        .collect();
    folders.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.id.cmp(&b.id)));
    let known: HashSet<&str> = folders.iter().map(|f| f.id.as_str()).collect();
    let items = store
        .list(&FOLDER_ITEMS)?
        .into_iter()
        .filter(|(_, p)| known.contains(p.folder.as_str()))
        .map(|(k, p)| (k, p.folder))
        .collect();
    Ok(Folders { folders, items })
}

fn clean_name(name: &str) -> AppResult<String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(AppError::Invalid("Folders need a name.".into()));
    }
    if n.chars().count() > NAME_MAX {
        return Err(AppError::Invalid("Folder names can be up to 100 characters.".into()));
    }
    Ok(n.to_string())
}

fn must_exist<'a>(t: &'a Folders, id: &str) -> AppResult<&'a FolderView> {
    t.get(id).ok_or_else(|| AppError::NotFound("That folder isn't there any more.".into()))
}

/// A new folder ("New Folder" unless named), at the top or inside `parent`.
pub fn create(store: &SyncStore, name: Option<&str>, parent: Option<&str>) -> AppResult<FolderView> {
    let t = tree(store)?;
    if let Some(p) = parent {
        must_exist(&t, p)?;
        if t.ancestors(p).len() + 2 > DEPTH_MAX {
            return Err(AppError::Invalid("Folders can only go 8 deep.".into()));
        }
    }
    let name = clean_name(name.unwrap_or(NEW_NAME))?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let f = Folder { name, parent: parent.map(str::to_string), created_at: store.now() };
    store.put(&FOLDERS, &id, &f)?;
    Ok(FolderView { id, name: f.name, parent: f.parent, created_at: f.created_at })
}

pub fn rename(store: &SyncStore, id: &str, name: &str) -> AppResult<()> {
    let t = tree(store)?;
    let cur = must_exist(&t, id)?;
    let f = Folder { name: clean_name(name)?, parent: cur.parent.clone(), created_at: cur.created_at };
    store.put(&FOLDERS, id, &f)
}

/// Delete a folder. What was in it (playlists and folders) moves up to
/// where the folder was: playlists live in Plex and are never deleted here.
pub fn delete(store: &SyncStore, id: &str) -> AppResult<()> {
    let t = tree(store)?;
    let gone = must_exist(&t, id)?;
    let up = gone.parent.clone();
    let mut ops = vec![Op::remove(&FOLDERS, id)];
    for f in t.folders.iter().filter(|f| f.parent.as_deref() == Some(id)) {
        ops.push(Op::put(&FOLDERS, &f.id, &Folder { name: f.name.clone(), parent: up.clone(), created_at: f.created_at })?);
    }
    for (playlist, folder) in &t.items {
        if folder == id {
            ops.push(match &up {
                Some(p) => Op::put(&FOLDER_ITEMS, playlist, &Placement { folder: p.clone() })?,
                None => Op::remove(&FOLDER_ITEMS, playlist),
            });
        }
    }
    store.write_many(ops)
}

/// Move a folder into another (None: to the top). Not into itself or a
/// folder inside it.
pub fn move_folder(store: &SyncStore, id: &str, parent: Option<&str>) -> AppResult<()> {
    let t = tree(store)?;
    let cur = must_exist(&t, id)?;
    if let Some(p) = parent {
        must_exist(&t, p)?;
        if p == id || t.ancestors(p).iter().any(|a| a == id) {
            return Err(AppError::Invalid("A folder can't go inside itself.".into()));
        }
    }
    store.put(&FOLDERS, id, &Folder { name: cur.name.clone(), parent: parent.map(str::to_string), created_at: cur.created_at })
}

/// Put a playlist in a folder (None: back to the top of the library).
pub fn place(store: &SyncStore, playlist: &str, folder: Option<&str>) -> AppResult<()> {
    if playlist.is_empty() || playlist == "liked" {
        return Err(AppError::Invalid("Only playlists go in folders.".into()));
    }
    match folder {
        Some(f) => {
            must_exist(&tree(store)?, f)?;
            store.put(&FOLDER_ITEMS, playlist, &Placement { folder: f.to_string() })
        }
        None => store.remove(&FOLDER_ITEMS, playlist).map(|_| ()),
    }
}
