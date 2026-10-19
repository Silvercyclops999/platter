//! App-only state across devices: the sync file, library folders and pins,
//! and copying keys from one device to another.
//!
//! Plex holds likes, playlists, history and play queues. What only Platter
//! knows (settings marked Synced, pins, library folders, followed artists,
//! hidden and snoozed items, bookmarks, lyrics the user wrote or
//! transcribed) lives in the
//! [`SyncStore`]. Without a sync file it stays on this device. With one,
//! the [`engine`] keeps the store and the file in step.
//!
//! # Using the store from a feature
//!
//! Pick (or add) a namespace in [`store`] with the type its values have,
//! then read and write through the store; never write the sync file
//! yourself:
//!
//! ```ignore
//! use crate::sync::{SyncStore, store::BOOKMARKS, store::Bookmark};
//! store.put(&BOOKMARKS, &id, &Bookmark { position_ms, title, ..Default::default() })?;
//! let all: Vec<(String, Bookmark)> = store.list(&BOOKMARKS)?;
//! store.remove(&BOOKMARKS, &id)?;
//! let mut changes = store.subscribe(); // Changed { namespaces, remote }
//! ```
//!
//! The calls are blocking (one small SQLite write); from async code wrap
//! them in [`SyncStore::run`]. Every value is JSON and must not carry a
//! secret (fields named like token, password, key or secret are refused).
//! A new namespace goes in [`store::NAMESPACES`] and in one of the
//! [`engine::Groups`] so the user can choose whether it syncs.
//!
//! - [`store`]: the typed store and its namespaces.
//! - [`file`]: the sync file on disk (atomic writes, conflicted copies).
//! - [`engine`]: the passes between the store and the file.
//! - [`settings`]: settings marked Synced through the store.
//! - [`pins`], [`folders`], [`lyrics`], [`transcripts`], [`follows`]: features on top of the store
//!   (followed artists through the social module's `FollowStore`).
//! - [`detect`]: finding cloud folders to suggest.
//! - [`keycopy`]: copying keys between devices (never through the file).
//! - [`commands`]: the Tauri commands (docs/IPC.md "Sync, folders and keys").

pub mod commands;
pub mod detect;
pub mod devices;
pub mod engine;
pub mod file;
pub mod folders;
pub mod follows;
pub mod keycopy;
pub mod lyrics;
pub mod mixed;
pub mod pins;
pub mod settings;
pub mod store;
pub mod transcripts;

#[cfg(test)]
mod digest_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transcripts_tests;

pub use store::{Changed, Ns, SyncStore};
