//! Keeping the user's own devices in step with each other, with no cloud
//! folder and no server: a computer and a phone paired once swap the sync
//! store's entries both ways, over the local network when they can reach
//! each other and through the sealed relay when they can't.
//!
//! A device link is a Bridge link (`crate::homeside::link`) with
//! [`Side::Device`](crate::homeside::link::Side::Device) at both ends: the
//! same pairing secret, sealed frames, counters and replay guard. Device
//! links are kept apart from Bridge links (`Links::devices`). Keys and
//! tokens never travel on them: the store refuses secrets, and every send
//! and every merge checks again.
//!
//! - [`protocol`]: the messages and what each end does with them.
//! - [`run`]: listeners, rounds and sending local changes.
//! - [`clock`]: how far apart two devices' clocks are, in words.

pub mod clock;
pub mod protocol;
pub mod run;

#[cfg(test)]
mod tests;

pub use protocol::{SyncDeps, SyncPeer};
pub use run::{DeviceSync, Timing};
