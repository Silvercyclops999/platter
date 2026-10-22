//! What a desktop and a Bridge say to each other, and what two of the
//! user's own devices say on a device link, inside sealed frames (`link`).
//! Nothing here is ever sent in the clear. A message kind or field this
//! build doesn't know is ignored, never an error.

use serde::{Deserialize, Serialize};

use crate::lidarr::request::RequestRecord;
use crate::lidarr::waves_bridge::{WavesPlan, WavesProgress};
use crate::radar::bridge::RadarPlan;
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
    /// The desktop's Platter version, for the Bridge's log. Empty from
    /// builds before 0.2.31, which didn't send it.
    #[serde(default)]
    pub version: String,
    /// Lidarr's home address (the Bridge is at home).
    #[serde(default)]
    pub lidarr_url: String,
    #[serde(default)]
    pub lidarr_key: Option<String>,
    /// Never sent by this build: older desktops sent the Plex account's
    /// sign-in here. A Bridge that still holds one gives it up once
    /// (`run::Bridge::migrate_plex`).
    #[serde(default)]
    pub plex_token: Option<String>,
    /// The Plex server (its machine id).
    #[serde(default)]
    pub plex_server: Option<String>,
    /// That server's own access token, only when Plex gives it one that
    /// isn't the account's sign-in (a server shared with the user). An
    /// owned server's comes back as the account's, so it's never sent; the
    /// Bridge signs in to Plex on its own instead.
    #[serde(default)]
    pub plex_server_token: Option<String>,
    /// The server's addresses, best first, for the server token.
    #[serde(default)]
    pub plex_connections: Vec<String>,
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
    /// Lyrics for every song: transcribe songs without lyrics at night.
    /// None from builds before it (the Bridge keeps it off then).
    #[serde(default)]
    pub lyrics: Option<bool>,
    /// The night hours that work runs in.
    #[serde(default)]
    pub lyrics_hours: NightHours,
    /// The computer's offset from UTC (seconds east), so night is the
    /// user's night whatever time zone the container is in.
    #[serde(default)]
    pub utc_offset_s: i64,
    /// "Lyrics from LRCLIB when Plex has none" on the computer: the Bridge
    /// asks LRCLIB too before transcribing a song.
    #[serde(default)]
    pub lrclib: bool,
    /// Lidarr from anywhere: the Bridge does Lidarr work. None from builds
    /// before the switch (on).
    #[serde(default)]
    pub lidarr: Option<bool>,
    /// The desktop's Plex client id, so a Bridge it paired with a code
    /// isn't linked to it a second time by account. Empty from older builds.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub device: String,
}

/// When the Bridge transcribes (in the computer's time zone).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NightHours {
    /// 11 PM to 6 AM.
    Late,
    /// 1 AM to 7 AM.
    #[default]
    Night,
    /// 2 AM to 8 AM.
    Early,
    /// Any time the server isn't busy.
    Always,
}

impl NightHours {
    /// (first hour, hour it stops), local; None: any time.
    pub fn span(self) -> Option<(u32, u32)> {
        match self {
            NightHours::Late => Some((23, 6)),
            NightHours::Night => Some((1, 7)),
            NightHours::Early => Some((2, 8)),
            NightHours::Always => None,
        }
    }

    /// Whether a unix time is inside the hours, at this offset from UTC.
    pub fn contains(self, at: i64, utc_offset_s: i64) -> bool {
        let Some((from, to)) = self.span() else { return true };
        let hour = ((at + utc_offset_s).rem_euclid(86_400) / 3600) as u32;
        if from <= to { (from..to).contains(&hour) } else { hour >= from || hour < to }
    }
}

/// Where the Bridge's lyrics work is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LyricsState {
    /// Switched off.
    #[default]
    Off,
    /// This Bridge was built without transcription.
    Unavailable,
    /// No music folder (or MUSIC_PLEX_PATH) to read songs from.
    NoMusic,
    /// The speech model is downloading.
    Downloading,
    /// Waiting for the night hours.
    Waiting,
    /// The server is busy: waiting until it's calm.
    Busy,
    Working,
    /// Every song it can do has lyrics.
    Done,
    /// Something needs looking at (`problem` says what).
    Problem,
}

impl LyricsState {
    /// The Bridge is doing (or about to do) the background work, so
    /// computers leave it to the Bridge.
    pub fn takes_over(self) -> bool {
        matches!(self, LyricsState::Downloading | LyricsState::Waiting | LyricsState::Busy | LyricsState::Working | LyricsState::Done)
    }
}

/// The Bridge's lyrics work, for Settings > Bridge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsLine {
    pub state: LyricsState,
    /// Songs with lyrics made on the Bridge (no words found included).
    #[serde(default)]
    pub done: usize,
    /// Songs still to look at, about.
    #[serde(default)]
    pub left: usize,
    #[serde(default)]
    pub hours: NightHours,
    /// The song being transcribed, by title.
    #[serde(default)]
    pub song: Option<String>,
    #[serde(default)]
    pub problem: Option<String>,
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
    /// Settings set on the Bridge itself (its environment), which win over
    /// what a computer sends: `lidarrUrl`, `lidarrKey`, `plexUrl`, `plexToken`.
    #[serde(default)]
    pub from_env: Vec<String>,
    /// Waiting to be let into Plex: the code to enter at plex.tv/link.
    #[serde(default)]
    pub plex_link: Option<String>,
    /// What MUSIC_PLEX_PATH should be, when the music folder is mounted and
    /// it isn't set to that.
    #[serde(default)]
    pub music_plex_path: Option<String>,
    /// Lyrics for every song. None from a Bridge before it.
    #[serde(default)]
    pub lyrics: Option<LyricsLine>,
    /// The staging folder it gives Lidarr for protected artists, so phones
    /// and computers don't need one set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staging_folder: Option<String>,
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
        /// It can run the daily look for followed artists (`crate::radar::bridge`).
        #[serde(default)]
        radar: bool,
        /// The Bridge's Plex client id (empty from older builds).
        #[serde(default, skip_serializing_if = "String::is_empty")]
        device: String,
    },
    /// From a desktop.
    Config(Box<BridgeConfig>),
    /// Shared Lidarr state (the sync store's `lidarr` namespace), both ways.
    Entries { entries: EntryMap },
    /// From the Bridge.
    Status(Box<BridgeStatus>),
    /// The home Platter route: a desktop away from home asks the Bridge to
    /// read Lidarr. Also on a device link, to a computer that offers the
    /// home side (its hello says `lidarr`).
    Get { id: u64, path: String, query: Vec<(String, String)> },
    /// The home Platter route: one of Lidarr's fixed writes, checked again
    /// on the Bridge (or the computer answering on a device link).
    Write { id: u64, method: String, path: String, body: String },
    /// Lidarr's answer (body base64).
    Answer { id: u64, status: u16, body: String },
    /// Lidarr couldn't be reached, or the home side refused.
    Failed { id: u64, message: String },
    /// Either side forgets the other.
    Unpair,
    /// From a desktop: Bring in my listening's album list and settings.
    WavesPlan(Box<WavesPlan>),
    /// From the Bridge: how the waves are going.
    WavesProgress(Box<WavesProgress>),
    /// From an owner's device: the followed artists whose new albums are
    /// got, and the settings (`crate::radar::bridge`).
    RadarPlan(Box<RadarPlan>),
    /// From the Bridge: what its daily look asked for lately.
    RadarMade { records: Vec<RequestRecord> },
    /// From a computer: the songs it has lyrics for already (its own or
    /// transcribed), so the Bridge doesn't transcribe them. Ids only.
    LyricsHave { ids: Vec<String> },
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
    /// The sender is back in touch after a long time apart from this
    /// device: so both ends catch up the same careful way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apart: Option<Apart>,
    /// A round over some namespaces only (a first copy, one group at a
    /// time): it never counts as being in step.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
    /// This device answers Lidarr requests for the user's other devices
    /// (Let my other devices use Lidarr through this computer). Left out
    /// when not, so an older build's hello and a newer one's read the same.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lidarr: bool,
}

/// When the two were last in step by the sender's clock (ms), and what
/// they kept in step then.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Apart {
    pub since: u64,
    #[serde(default)]
    pub ns: Vec<String>,
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
            Msg::RadarPlan(_) => "radarPlan",
            Msg::RadarMade { .. } => "radarMade",
            Msg::LyricsHave { .. } => "lyricsHave",
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
