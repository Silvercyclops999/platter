//! Music servers behind one interface. Each server the app knows is a
//! [`registry::ServerRecord`] in `servers.json`; what it can do is its
//! [`caps::Caps`].

pub mod caps;
pub mod registry;

pub use caps::Caps;
pub use registry::{BackendKind, ServerRecord, ServersFile};
