//! App-state watchers that sit between modules: is the music on the server
//! readable, is this computer online, did it just wake up, is the disk full,
//! what did the last library sync find, and crash reports. Each watcher
//! reports into `PlexState`'s health (banner + status dot) or a toast.
//!
//! The decision logic of each one is a pure function or small struct with
//! unit tests; the `spawn_*` loops only feed them real inputs.

pub mod cached;
pub mod crash;
pub mod disk;
pub mod files;
pub mod library;
pub mod net;
