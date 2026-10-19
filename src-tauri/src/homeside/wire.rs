//! What a desktop and a Bridge say to each other, and what two of the
//! user's own devices say on a device link, inside sealed frames (`link`).
//! Nothing here is ever sent in the clear. A message kind or field this
//! build doesn't know is ignored, never an error.

use serde::{Deserialize, Serialize};

use crate::lidarr::waves_bridge::{WavesPlan, WavesProgress};
use crate::sync::store::{Digests, EntryMap, Stamp};

/// How the Bridge reaches the music folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MusicAccess {
    /// Not mounted: no promotions or swaps.
    #[default]
    None,
    /// Mounted read-only: reads only.
    ReadOnly,
    /// Mounted read-write: promotions and swaps can run.
    ReadWrite,
}

/// What a desktop hands its Bridge, sealed: how to reach Lidarr and Plex,
/// and the Lidarr settings the work follows. Kept on the Bridge's data
/// volume (secrets in a file only it can read).
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeConfig {
    /// The desktop's name, for the Bridge's log.
    #[serde(default)]
    pub desktop: String,
    /// Lidarr's home address (the Bridge is at home).
    #[serde(default)]
    pub lidarr_url: String,
    #[serde(default)]
    pub lidarr_key: Option<String>,
    #[serde(default)]
    pub plex_token: Option<String>,
    /// The Plex server (its machine id).
    #[serde(default)]
    pub plex_server: Option<String>,
    #[serde(default)]
    pub staging_folder: String,
    #[serde(default)]
    pub scan_after_import: bool,
    #[serde(default)]
    pub free_staging: bool,
    #[serde(default)]
    pub practice_until: i64,
    #[serde(default)]
    pub upgrades_ask: bool,
}

impl std::fmt::Debug for BridgeConfig {
    // Never prints the key or the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeConfig").field("desktop", &self.desktop).finish_non_exhaustive()
    }
}

/// The last catch-up, in short.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatchUpLine {
    /// Unix seconds.
    pub at: i64,
    pub handled: usize,
    pub acted: usize,
    #[serde(default)]
    pub scanned: usize,
    #[serde(default)]
    pub announced: Vec<String>,
    #[serde(default)]
    pub healed: usize,
    #[serde(default)]
    pub paused: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// How a Bridge is doing, for Settings > Bridge.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    pub name: String,
    pub version: String,
    /// When it said this (unix seconds).
    pub at: i64,
    /// One plain line: what it's doing now.
    pub doing: String,
    #[serde(default)]
    pub last_catch_up: Option<CatchUpLine>,
    #[serde(default)]
    pub music: MusicAccess,
    #[serde(default)]
    pub plex: bool,
    #[serde(default)]
    pub lidarr: bool,
    /// Something the user should look at.
    #[serde(default)]
    pub problem: Option<String>,
}

/// One message.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Msg {
    /// From the Bridge, after pairing and on every start. `waves`: it can
    /// run Bring in my listening.
    Hello {
        name: String,
        version: String,
        music: MusicAccess,
        #[serde(default)]
        waves: bool,
    },
    /// From a desktop.
    Config(Box<BridgeConfig>),
    /// Shared Lidarr state (the sync store's `lidarr` namespace), both ways.
    Entries { entries: EntryMap },
    /// From the Bridge.
    Status(Box<BridgeStatus>),
    /// The home Platter route: a desktop away from home asks the Bridge to
    /// read Lidarr.
    Get { id: u64, path: String, query: Vec<(String, String)> },
    /// The home Platter route: one of Lidarr's fixed writes, checked again
    /// on the Bridge.
    Write { id: u64, method: String, path: String, body: String },
    /// Lidarr's answer (body base64).
    Answer { id: u64, status: u16, body: String },
    /// Lidarr couldn't be reached, or the Bridge refused.
    Failed { id: u64, message: String },
    /// Either side forgets the other.
    Unpair,
    /// From a desktop: Bring in my listening's album list and settings.
    WavesPlan(Box<WavesPlan>),
    /// From the Bridge: how the waves are going.
    WavesProgress(Box<WavesProgress>),
    /// Between two of the user's own devices: who this is, its clock, and
    /// a digest of each namespace it keeps in step. `answer`: the reply to
    /// the other end's hello (it isn't answered again).
    SyncHello(Box<SyncHello>),
    /// The stamps of every entry in one namespace (no values).
    SyncDiff { ns: String, keys: Vec<Stamp> },
    /// Entries this end wants from the other one.
    SyncWant { ns: String, keys: Vec<String> },
    /// Entries, in batches (`links::batches`).
    SyncEntries { entries: EntryMap },
    /// "Copy settings from my computer": these entries as they are there.
    SyncCopyWant { ns: String, keys: Vec<String> },
    /// The answer to a [`Msg::SyncCopyWant`], in one message (empty when
    /// the other device has none of them).
    SyncCopy { entries: EntryMap },
    /// The device unpaired.
    SyncBye,
    /// A message from a newer build this one doesn't know. Ignored.
    #[serde(other)]
    Unknown,
}

/// The device link protocol this build speaks. A newer peer speaks the
/// lower of the two; nothing refuses a link for being newer.
pub const PROTO: u32 = 1;

/// A device's hello on the device link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncHello {
    pub proto: u32,
    /// The device's id (its Plex client id).
    pub device: String,
    pub name: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub version: String,
    /// The sender's clock when it sent this (ms).
    pub clock_ms: u64,
    /// The groups it keeps in step, for the status line.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Its account's tag; a device of another account is refused.
    #[serde(default)]
    pub acct: String,
    pub digests: Digests,
    /// Namespaces it knows but has switched off: nothing in them is sent
    /// to it. A namespace in neither list is one it doesn't know yet; it
    /// keeps and passes those on.
    #[serde(default)]
    pub skip: Vec<String>,
    #[serde(default)]
    pub answer: bool,
}

impl Msg {
    /// For logs: the kind only.
    pub fn kind(&self) -> &'static str {
        match self {
            Msg::Hello { .. } => "hello",
            Msg::Config(_) => "config",
            Msg::Entries { .. } => "entries",
            Msg::Status(_) => "status",
            Msg::Get { .. } => "get",
            Msg::Write { .. } => "write",
            Msg::Answer { .. } => "answer",
            Msg::Failed { .. } => "failed",
            Msg::Unpair => "unpair",
            Msg::WavesPlan(_) => "wavesPlan",
            Msg::WavesProgress(_) => "wavesProgress",
            Msg::SyncHello(_) => "syncHello",
            Msg::SyncDiff { .. } => "syncDiff",
            Msg::SyncWant { .. } => "syncWant",
            Msg::SyncEntries { .. } => "syncEntries",
            Msg::SyncCopyWant { .. } => "syncCopyWant",
            Msg::SyncCopy { .. } => "syncCopy",
            Msg::SyncBye => "syncBye",
            Msg::Unknown => "unknown",
        }
    }
}
