//! What the engine needs from the outside world, as traits the integrator
//! implements (with the Plex client and the SQLite cache). The engine never
//! depends on the `plex` module.

use std::collections::HashSet;
use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use super::dsp::TrackAudio;
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
    /// The original file, when [`Self::stream`] gave a transcode capped to
    /// a quality limit (away from home) and the server refused both it and
    /// [`Self::transcode`]. None when the stream already was the original.
    async fn original(&self, _track: &Track) -> Option<AppResult<StreamSource>> {
        None
    }
    /// Loudness data (Plex gain and peak) and silence measured on an earlier
    /// play, if known. Fetched while the stream opens; None plays the track
    /// with only the preamp.
    async fn audio_info(&self, _track: &Track) -> Option<TrackAudio> {
        None
    }
    /// Stores a track's measured quiet lead-in and run-out for next time.
    fn save_silence(&self, _track_id: &str, _head_ms: u64, _tail_ms: u64) {}
}

/// Songs downloaded to this computer. The engine plays a downloaded file
/// before it looks at the pre-cache or the server.
pub trait LocalAudio: Send + Sync {
    /// The downloaded file for the track, when it's on disk.
    fn path(&self, track: &Track) -> Option<std::path::PathBuf>;
    /// Every downloaded track id.
    fn ids(&self) -> Vec<String>;
}

/// A shared [`LocalAudio`] that can sit in the player's config.
#[derive(Clone)]
pub struct LocalFiles(pub std::sync::Arc<dyn LocalAudio>);

impl fmt::Debug for LocalFiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalFiles")
    }
}

/// The kinds of context playback can start from (docs/IPC.md `play_context`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextKind {
    /// An album, or only its songs in `track_ids` (one side of the record),
    /// in album order.
    Album,
    Playlist,
    Artist,
    Tracks,
    Search,
    /// Liked Songs (id "liked"), newest like first, or `track_ids` when
    /// given (the page's sort or filter).
    Liked,
    /// A made-for-you mix (id from `mixes_home`), or `track_ids` when given
    /// (a steered mix).
    Mix,
    /// Song or artist radio (id `track:<id>` or `artist:<id>`). Endless: the
    /// player asks for more as it runs low.
    Radio,
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
    /// While only music stored on this computer can play (server out of
    /// reach), the ids of those tracks; `None` when everything can play.
    async fn playable_now(&self) -> Option<HashSet<String>> {
        None
    }
    /// `n` songs were left out of a play because they need the server.
    fn left_out_offline(&self, _n: usize) {}
    /// More songs for a context that's running out: the next radio batch,
    /// or similar music after a context ends. Empty when there are none.
    async fn more(&self, _req: &MoreRequest) -> AppResult<Vec<Track>> {
        Ok(Vec::new())
    }
    /// Hidden songs and artists: shuffle play skips them (a song picked
    /// directly still plays).
    fn hidden(&self) -> Option<std::sync::Arc<crate::hidden::HiddenSet>> {
        None
    }
}

/// What [`TrackLookup::more`] is asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct MoreRequest {
    pub kind: ContextKind,
    pub id: Option<String>,
    /// The context's songs nearest the end (what "similar" means).
    pub seed_ids: Vec<String>,
    /// Songs already queued or just played.
    pub avoid: Vec<String>,
    pub n: usize,
    /// For smart shuffle (songs to mix in), not for after the context.
    pub smart: bool,
    /// Smart shuffle: artist ids already in the context (left out).
    pub avoid_artists: Vec<String>,
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
