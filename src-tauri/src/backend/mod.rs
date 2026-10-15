//! Music servers behind one interface. Each server the app knows is a
//! [`registry::ServerRecord`] in `servers.json`; what it can do is its
//! [`caps::Caps`].

pub mod backends;
pub mod caps;
pub mod commands;
pub mod plex;
pub mod registry;
pub mod routed;
pub mod server;

pub use backends::Backends;
pub use caps::Caps;
pub use server::MusicServer;
pub use registry::{BackendKind, ServerRecord, ServersFile};

#[cfg(test)]
mod tests;
