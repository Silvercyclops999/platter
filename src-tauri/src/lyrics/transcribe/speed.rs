//! How fast each model has been on this computer, on the graphics card and
//! on the processor, from the songs transcribed here. The model picker shows
//! it ("about 8 seconds per song"). Kept in `<models>/speeds.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::model::ModelKind;

/// Speeds are told for a song this long.
pub const TYPICAL_SONG_SECS: f64 = 210.0;
/// Shorter runs (the lyrics check's windows) don't count.
pub const MIN_AUDIO_SECS: f64 = 30.0;
/// Weight of the newest run (the rest is the earlier average).
const NEWEST: f64 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Speed {
    /// Seconds of work per second of audio, averaged over the last runs.
    ratio: f64,
    runs: u32,
}

fn key(kind: ModelKind, gpu: bool) -> String {
    let model = serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
    format!("{model}/{}", if gpu { "gpu" } else { "cpu" })
}

/// The measured speeds (None path: kept in memory only).
pub struct Speeds {
    path: Option<PathBuf>,
    map: Mutex<BTreeMap<String, Speed>>,
}

impl Speeds {
    pub fn load(path: Option<PathBuf>) -> Self {
        let map = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { path, map: Mutex::new(map) }
    }

    /// A song of `audio_secs` took `took` with this model on the card or not.
    pub fn record(&self, kind: ModelKind, gpu: bool, audio_secs: f64, took: Duration) {
        if audio_secs < MIN_AUDIO_SECS {
            return;
        }
        let ratio = took.as_secs_f64() / audio_secs;
        let mut map = self.map.lock();
        let e = map.entry(key(kind, gpu)).or_insert(Speed { ratio, runs: 0 });
        if e.runs > 0 {
            e.ratio = e.ratio * (1.0 - NEWEST) + ratio * NEWEST;
        }
        e.runs = e.runs.saturating_add(1);
        if let Some(p) = &self.path
            && let Ok(body) = serde_json::to_vec(&*map)
        {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, p)).is_err() {
                tracing::debug!("transcription speeds not saved");
            }
        }
    }

    /// About how long a typical song takes, when it has been measured.
    pub fn secs_per_song(&self, kind: ModelKind, gpu: bool) -> Option<u32> {
        let s = *self.map.lock().get(&key(kind, gpu))?;
        Some((s.ratio * TYPICAL_SONG_SECS).round().max(1.0) as u32)
    }
}
