//! What the engine needs from the outside world, as traits the integrator
//! implements (with the Plex client and the SQLite cache). The engine never
//! depends on the `plex` module.

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::model::Track;

/// Where to fetch one track's audio. The Plex token travels only inside
/// `headers`; `Debug` redacts them and the engine never logs them.
#[derive(Clone, Default)]
pub struct StreamSource {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Byte size if known (Plex `Part.size`). Used for buffered estimates.
    pub size: Option<u64>,
    /// Container / extension hint for the probe, e.g. "flac", "mp3", "m4a",
    /// "ogg", "wav". `None` lets the probe sniff.
    pub codec_hint: Option<String>,
}

impl fmt::Debug for StreamSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the query string: Plex URLs can carry a token there too.
        let url = self.url.split('?').next().unwrap_or("");
        f.debug_struct("StreamSource")
            .field("url", &url)
            .field("headers", &format_args!("[{} redacted]", self.headers.len()))
            .field("size", &self.size)
            .field("codec_hint", &self.codec_hint)
            .finish()
    }
}

/// Resolves a track to a playable stream. Implemented by the integrator with
/// the Plex client (direct part URL / universal transcoder).
#[async_trait]
pub trait TrackResolver: Send + Sync {
    /// The original file (direct play), e.g. `{server}/library/parts/...`.
    async fn stream(&self, track: &Track) -> AppResult<StreamSource>;
    /// A transcoded fallback (e.g. MP3/AAC or FLAC from the universal
    /// transcoder) used when the original can't be decoded.
    async fn transcode(&self, track: &Track) -> AppResult<StreamSource>;
}

/// The kinds of context playback can start from (docs/IPC.md `play_context`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextKind {
    Album,
    Playlist,
    Artist,
    Tracks,
    Search,
    /// Liked Songs (id "liked"), newest like first.
    Liked,
}

/// `play_context.context` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayContext {
    pub kind: ContextKind,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub track_ids: Option<Vec<String>>,
}

/// Tracks of a resolved context, in context order.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedContext {
    pub tracks: Vec<Track>,
    /// Display title ("Album name", "Playlist name"...), if known.
    pub title: Option<String>,
}

/// Turns ids into `Track`s. Implemented by the integrator with the SQLite
/// cache (falling back to Plex).
#[async_trait]
pub trait TrackLookup: Send + Sync {
    /// Tracks for the given ids, in the given order. Unknown ids are skipped.
    async fn tracks(&self, ids: &[String]) -> AppResult<Vec<Track>>;
    /// All tracks of a context in play order (album: disc/track order;
    /// artist: popular-then-albums or however the integrator defines it;
    /// `tracks`/`search`: the given `track_ids`).
    async fn context(&self, ctx: &PlayContext) -> AppResult<ResolvedContext>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_headers_and_query() {
        let s = StreamSource {
            url: "http://srv/library/parts/1/2/file.flac?X-Plex-Token=secret".into(),
            headers: vec![("X-Plex-Token".into(), "secret".into())],
            size: Some(10),
            codec_hint: Some("flac".into()),
        };
        let d = format!("{s:?}");
        assert!(!d.contains("secret"), "{d}");
        assert!(d.contains("file.flac"));
    }

    #[test]
    fn context_deserializes_ipc_shape() {
        let c: PlayContext =
            serde_json::from_str(r#"{"kind":"tracks","trackIds":["1","2"]}"#).unwrap();
        assert_eq!(c.kind, ContextKind::Tracks);
        assert_eq!(c.track_ids.unwrap().len(), 2);
    }
}
