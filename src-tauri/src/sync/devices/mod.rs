//! Keeping the user's own devices in step with each other, with no cloud
//! folder and no server: a computer and a phone paired once swap the sync
//! store's entries both ways, over the local network when they can reach
//! each other and through the sealed relay when they can't.
//!
//! - [`clock`]: how far apart two devices' clocks are, in words.

pub mod clock;
