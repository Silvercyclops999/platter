//! Playback engine: HTTP range streaming, decode-ahead, output, queue.
//!
//! Integration (done by the integrator in `lib.rs`):
//! 1. Implement [`source::TrackResolver`] (stream/transcode URLs from the
//!    Plex client) and [`source::TrackLookup`] (ids → `Track`s from the DB).
//! 2. `PlayerHandle::spawn(rt, resolver, lookup, Box::new(CpalOutput::new()),
//!    Arc::new(TauriSink(app.handle().clone())), PlayerConfig { .. })` and
//!    `app.manage(handle)`.
//! 3. Register the commands in [`commands`].
//! 4. Subscribe to [`player::PlayerHandle::subscribe_tracking`] for play tracking.

pub mod autoeq;
pub mod automix;
pub mod cache;
pub mod commands;
pub mod compare;
pub mod convert;
pub mod decode;
mod devguard;
pub mod dsp;
pub mod engine;
pub mod eqprofile;
pub mod events;
mod exclusive;
pub mod hifi;
pub mod gate;
pub mod http;
pub mod netsink;
pub mod output;
pub mod persist;
pub mod player;
pub mod queue;
pub mod signal;
pub mod sleep;
pub mod source;
pub mod visual;

pub use events::{EventSink, PlayerState, PlayerStatus, TauriSink, TrackingEvent};
pub use output::{AudioOutput, CpalOutput};
pub use player::{PlayerConfig, PlayerHandle};
pub use source::{PlayContext, StreamSource, TrackLookup, TrackResolver};

#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod automix_tests;
#[cfg(test)]
mod dsp_tests;
#[cfg(test)]
mod hifi_tests;
#[cfg(test)]
mod player_tests;
#[cfg(test)]
mod signal_tests;
#[cfg(test)]
mod smoke;
#[cfg(test)]
mod visual_tests;
#[cfg(test)]
pub(crate) mod testutil;
