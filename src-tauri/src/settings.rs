//! App settings (docs/IPC.md `settings_get` / `settings_set`), plus the few
//! commands the Settings page needs (art cache size and clear, app info).
//!
//! Stored as one JSON value in the db `kv` table. Unknown or missing fields
//! fall back to defaults, so older and newer builds can share a database.
//! Values are pushed to the parts of the core that use them ([`Live`]) at
//! startup and on every change; player settings go through the player.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::audio::player::PlayerHandle;
use crate::audio::dsp::{CrossfadeMode, EQ_BANDS, Leveling};
use crate::audio::eqprofile::{self, EqChoice, EqOutput, EqProfile};
use crate::audio::queue::ShuffleStyle;
use crate::bridge::StreamPrefs;
use crate::db::store::{kv_get, kv_set};
use crate::db::{ArtCache, DbHandle, PopularSource};
use crate::downloads::{self, DownloadQuality};
use crate::error::{AppError, AppResult};
use crate::platform::discord::activity::StatusLine;
use crate::platform::discord::{DiscordSettings, DiscordSwitches};
use crate::platform::{PlatformSettings, Switches};
use crate::tracking::TrackingSettings;

const KEY: &str = "app.settings";

/// The project page opened from About.
pub const REPO_URL: &str = "https://github.com/Silvercyclops999/platter";
pub const LICENCE: &str = "GPL-3.0-or-later";

pub const PRECACHE_MIN: u32 = 1;
/// The longest name this computer can show to other devices.
pub const COMPUTER_NAME_MAX: usize = 60;
pub const PRECACHE_MAX: u32 = 50;
pub const ART_CACHE_MIN_MB: u32 = 128;
pub const ART_CACHE_MAX_MB: u32 = 51_200;
const MB: u64 = 1024 * 1024;

/// Streaming quality: the original file, or a transcode cap in kbps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StreamQuality {
    #[default]
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "320")]
    Kbps320,
    #[serde(rename = "256")]
    Kbps256,
    #[serde(rename = "192")]
    Kbps192,
    #[serde(rename = "128")]
    Kbps128,
}

impl StreamQuality {
    /// The cap in kbps, or None for the original file.
    pub fn kbps(self) -> Option<u32> {
        match self {
            Self::Original => None,
            Self::Kbps320 => Some(320),
            Self::Kbps256 => Some(256),
            Self::Kbps192 => Some(192),
            Self::Kbps128 => Some(128),
        }
    }
}

/// Reduce motion: follow the OS, or force it on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReduceMotion {
    #[default]
    System,
    On,
    Off,
}

/// Which Plex star ratings count as liked. Liking always writes five stars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LikedMeans {
    /// Five stars (Plex rating 10).
    #[default]
    Five,
    /// Four stars or more (Plex rating 8 and up).
    FourUp,
}

impl LikedMeans {
    pub fn min_rating(self) -> f64 {
        match self {
            Self::Five => crate::db::likes::LIKED_RATING,
            Self::FourUp => 8.0,
        }
    }
}

/// When track rows show the like control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LikeControlRows {
    /// Liked songs always; the rest on hover or focus.
    #[default]
    Hover,
    Always,
    Off,
}

/// Whether the lyrics view opens by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LyricsMode {
    /// A song with synced lyrics starting while now playing is open opens
    /// the lyrics view.
    #[default]
    AutoShow,
    /// The lyrics view only opens from its button.
    Off,
}

/// Whether songs tagged explicit play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplicitContent {
    #[default]
    Allowed,
    /// Left out of albums, playlists and anything else that plays, with a toast.
    Filtered,
}

/// What plays when the queue runs out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QueueEnd {
    /// Similar music follows (from the mix engine's radio).
    #[default]
    KeepPlaying,
    /// The context rewinds to its first song and stops.
    Stop,
}

/// The key that shows jump hints in keyboard power mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HintKey {
    #[default]
    #[serde(rename = "f")]
    F,
    #[serde(rename = "h")]
    H,
    #[serde(rename = ";")]
    Semicolon,
    #[serde(rename = "'")]
    Quote,
}

/// Hours between daylist refreshes the setting offers.
pub const DAYLIST_HOURS: [u32; 3] = [1, 3, 6];

/// Missing fields (settings saved by an older build) take their defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Skip the local listen log (Plex still gets reports unless
    /// `report_to_plex` is off).
    pub private_session: bool,
    /// Send timeline and scrobble reports to Plex.
    pub report_to_plex: bool,
    /// Linear slider value, 0..1.
    pub volume: f32,
    /// How shuffle orders tracks (spread keeps an artist's songs apart).
    pub shuffle_style: ShuffleStyle,
    /// Streaming quality on the local connection.
    pub quality_home: StreamQuality,
    /// Streaming quality over remote access or the relay.
    pub quality_away: StreamQuality,
    /// Show the format badge (codec, bit depth, sample rate).
    pub show_format_badge: bool,
    /// Albums and playlists resume where they were left.
    pub resume_contexts: bool,
    /// Upcoming tracks kept on disk, 1 to 50.
    pub precache_count: u32,
    /// What ranks the artist page's Popular list.
    pub popular_source: PopularSource,
    /// Artwork cache size in MB.
    pub art_cache_mb: u32,
    /// Album and artist headers tint from artwork.
    pub artwork_colours: bool,
    pub reduce_motion: ReduceMotion,
    /// Chosen output device id; None follows the system default. Changed
    /// with `output_select`, not `settings_set`.
    pub output_device: Option<String>,
    /// Home shows the grid of recent and most played shortcuts.
    pub home_shortcuts: bool,
    /// Home's top takes a colour wash from the shortcuts' artwork (only
    /// while Artwork colours is on).
    pub home_colour_wash: bool,
    /// Show completions and direct results under the search box as you type.
    pub search_suggestions: bool,
    /// Remember items opened from search (shown when the empty box is focused).
    pub search_recents: bool,
    /// Media keys work while Platter is in the background.
    pub media_keys: bool,
    /// Show in the system media controls (Windows media flyout and lock
    /// screen, macOS Now Playing).
    pub media_session: bool,
    /// Closing the window hides it to the tray / menu bar.
    pub close_to_tray: bool,
    /// Start Platter when you sign in to the computer.
    pub launch_at_login: bool,
    /// Right-click or Alt+click on the seek bar sets A-B loop points.
    pub ab_loop_handles: bool,
    /// Near a track's end the player bar shows what plays next.
    pub up_next_line: bool,
    /// Show the volume percentage while it changes.
    pub volume_percent: bool,
    /// Desktop notification on each new track (this device only).
    pub notify_track_change: bool,
    /// Ask before adding a song that's already in the playlist.
    pub playlist_ask_duplicates: bool,
    /// Name for new playlists; `#N` becomes the next free number.
    pub playlist_new_name: String,
    /// Artist info from Wikipedia and Wikidata.
    pub info_wikipedia: bool,
    /// Artist info from Last.fm (needs the user's key, kept in the keychain).
    pub info_lastfm: bool,
    /// Artist images from Fanart.tv (needs the user's key).
    pub info_fanart: bool,
    /// Artist photo and bio from TheAudioDB (needs the user's key).
    pub info_audiodb: bool,
    /// Popular songs from ListenBrainz (Last.fm top tracks when it has none
    /// and Last.fm is on).
    pub info_listenbrainz: bool,
    /// Every name in a track's artist line links to a page, featured
    /// artists without a library entry included.
    pub link_featured: bool,
    /// Which star ratings count as liked.
    pub liked_means: LikedMeans,
    /// When track rows show the like control.
    pub like_control_rows: LikeControlRows,
    /// List Plex players in the device picker (Listen on).
    pub show_plex_players: bool,
    /// Answer as a Plex player so other Plex apps can play here (this
    /// device only; opens a local port).
    pub act_as_plex_player: bool,
    /// Moving playback to or from a player stops it on the device it leaves.
    pub move_stops_old_device: bool,
    /// Crossfade between songs (synced once settings sync arrives).
    pub crossfade: CrossfadeMode,
    /// Length of a fixed crossfade, 1 to 12 s.
    pub crossfade_seconds: u32,
    /// Songs of one album played in order never crossfade.
    pub crossfade_skip_albums: bool,
    /// Loudness leveling (synced once settings sync arrives).
    pub leveling: Leveling,
    /// dB added to every track, -12 to +12. This device only.
    pub preamp_db: f32,
    /// Soft limiter so boosts never clip. This device only.
    pub limiter: bool,
    /// EQ on. This device only, like the rest of the EQ.
    pub eq_enabled: bool,
    /// The active curve, 31 Hz to 16 kHz, in dB.
    pub eq_bands: [f32; EQ_BANDS],
    /// The active preset id, `profile:<name>` or `custom`.
    pub eq_preset: String,
    /// Saved curves. Changed with the eq_profile commands.
    pub eq_profiles: Vec<EqProfile>,
    /// Presets or profiles bound to output devices.
    pub eq_outputs: Vec<EqOutput>,
    /// The choice for output devices without their own.
    pub eq_fallback: Option<EqChoice>,
    /// The output device the EQ last followed.
    pub eq_last_output: Option<String>,
    /// Share menus find public links for released music (iTunes Search and
    /// song.link). Off leaves only Platter links.
    pub share_public_links: bool,
    /// Share cards include a QR code with the Platter link.
    pub share_card_qr: bool,
    /// Share cards for released music include the song.link line.
    pub share_card_song_link: bool,
    /// List Cast devices and AirPlay receivers in the device picker
    /// (Listen on). Off keeps mDNS off.
    pub show_cast_devices: bool,
    /// Show what's playing on Discord (this device only).
    pub discord_status: bool,
    /// Show unreleased songs on Discord (off: nothing while one plays).
    pub discord_show_unreleased: bool,
    /// Unreleased songs show their cover (off: the app icon).
    pub discord_unreleased_art: bool,
    /// Pausing clears the Discord status (off: it shows as paused).
    pub discord_clear_on_pause: bool,
    /// A "Find this song" button (song.link) for released songs.
    pub discord_find_song: bool,
    /// Look up covers on Cover Art Archive (by MusicBrainz id, or a MusicBrainz
    /// search for released albums).
    pub discord_cover_archive: bool,
    /// Upload covers to Imgur with the user's client id (kept in the keychain).
    pub discord_upload_art: bool,
    /// Upload covers to catbox.moe (no key; public and permanent).
    pub discord_upload_catbox: bool,
    /// What Discord's member list shows after "Listening to".
    pub discord_status_line: StatusLine,
    /// The top bar shows the Plex profile picture (this device only).
    pub show_plex_avatar: bool,
    /// Whether the lyrics view opens by itself.
    pub lyrics_mode: LyricsMode,
    /// Look lyrics up on LRCLIB when Plex has none (sends artist, title,
    /// album and length).
    pub lyrics_lrclib: bool,
    /// The user's own lyrics show ahead of Plex's for every song.
    pub lyrics_prefer_own: bool,
    /// Songs without lyrics offer "Add lyrics" in the lyrics view.
    pub lyrics_show_add: bool,
    /// The full-window view hides its controls after 3 s without movement.
    pub full_window_hide_controls: bool,
    /// Read explicit, version and edition tags from the files (this device only).
    pub read_file_tags: bool,
    /// Explicit songs play, or are left out of what plays.
    pub explicit_content: ExplicitContent,
    /// Show fan and AI edits (ARCHIVE_FLAGS).
    pub show_fan_edits: bool,
    /// Albums sharing ARCHIVE_GROUP show as one card with an edition picker.
    pub group_editions: bool,
    /// Drag songs, albums and playlists onto playlists, Liked Songs and the queue.
    pub drag_and_drop: bool,
    /// Transcribe songs without lyrics on this computer (this device only).
    pub transcribe_lyrics: bool,
    /// The speech model transcription uses (this device only).
    pub transcribe_model: crate::lyrics::transcribe::model::ModelKind,
    /// Show lyrics transcribed on this computer.
    pub show_auto_lyrics: bool,
    /// Announce this computer to the user's other Platter devices on the
    /// network and through Plex, and let them play here (this device only).
    pub show_to_my_devices: bool,
    /// The name other devices see. Empty means the OS computer name
    /// (this device only).
    pub computer_name: String,
    /// List AirPlay receivers that need a password or pairing.
    pub show_airplay_locked: bool,
    /// Downloads: the file as it is, or an MP3 (this device only).
    pub download_quality: DownloadQuality,
    /// Downloads stop at this size, in GB (this device only).
    pub download_limit_gb: u32,
    /// Where downloads go. Empty means the app's own folder (this device
    /// only). Changed with `downloads_choose_folder`, not `settings_set`.
    pub download_folder: String,
    /// Download only over the local connection to the server (this device only).
    pub download_home_only: bool,
    /// Keep Liked Songs downloaded; new likes download too (this device only).
    pub download_liked: bool,
    /// Go offline: no network use at all, only music on this computer plays
    /// (this device only).
    pub go_offline: bool,
    /// Home shows the Made for you and mood and genre mix shelves.
    pub home_mixes: bool,
    /// How much of a mix is favourites.
    pub mix_variety: crate::mixes::Variety,
    /// Artist ids left out of mixes and radio.
    pub mix_exclude_artists: Vec<String>,
    /// Genre, style or mood names left out of mixes and radio.
    pub mix_exclude_genres: Vec<String>,
    /// Album ids left out of mixes and radio.
    pub mix_exclude_albums: Vec<String>,
    /// Unreleased music (demos and archive versions) can be in mixes.
    pub mix_unreleased: bool,
    /// Hours between daylist refreshes: 1, 3 or 6.
    pub daylist_hours: u32,
    /// When the queue ends: keep playing similar music, or stop.
    pub queue_end: QueueEnd,
    /// The shuffle button has a third state, smart shuffle.
    pub smart_shuffle: bool,
    /// The top bar's friend activity button and panel.
    pub show_friend_activity: bool,
    /// Tell Plex what's playing now, so friends on the server see it live.
    pub share_activity: bool,
    /// The bell's dot for unseen releases on What's New.
    pub whats_new_announcements: bool,
    /// A desktop notification when a followed artist gets new music.
    pub notify_new_releases: bool,
    /// "Let others change what's playing": guests in a listening session can
    /// play, pause, skip, seek and reorder. Off, they can only add songs.
    pub listen_guests_control: bool,
    /// The listen along relay (an ntfy server); empty means ntfy.sh.
    pub listen_relay: String,
    /// "Invites expire after", hours (1, 24 or 168).
    pub listen_invite_hours: u32,
    /// The name others in a listening session see; empty means the Plex
    /// display name.
    pub listen_name: String,
    /// Discord shows a listening session you host.
    pub discord_listen_along: bool,
    /// Keep app-only state in a sync file in a folder (this device only).
    pub sync_enabled: bool,
    /// The folder the sync file goes in (this device only). Changed with
    /// `sync_file_choose_folder` / `sync_file_use_folder`, not `settings_set`.
    pub sync_folder: String,
    /// What to sync: settings marked Synced (this device only).
    pub sync_settings: bool,
    /// What to sync: library folders and pins (this device only).
    pub sync_library: bool,
    /// What to sync: followed artists and hidden items (this device only).
    pub sync_follows: bool,
    /// What to sync: lyrics you wrote (this device only).
    pub sync_lyrics: bool,
    /// Bookmarks has a row in the library.
    pub bookmarks_in_library: bool,
    /// Scrobble to Last.fm (once signed in).
    pub scrobble_lastfm: bool,
    /// Scrobble to ListenBrainz (with the user token).
    pub scrobble_listenbrainz: bool,
    /// Unreleased music scrobbles too.
    pub scrobble_unreleased: bool,
    /// The AI provider for prompted playlists (none by default).
    pub ai_provider: crate::smart::prompt::AiProvider,
    /// The provider's model.
    pub ai_model: String,
    /// The miniplayer window stays above other windows (this device only).
    pub miniplayer_on_top: bool,
    /// The miniplayer's card takes a colour from the cover (this device only).
    pub miniplayer_tint: bool,
    /// The miniplayer lists what's up next when it's tall enough (this device only).
    pub miniplayer_queue: bool,
    /// Where the miniplayer was last, in logical pixels. Kept by the core,
    /// read only here.
    pub miniplayer_bounds: Option<crate::platform::miniplayer::WindowBounds>,
    /// Keyboard power mode: jump hints, j / k, g g, G and / (main window).
    pub power_keys: bool,
    /// The key that shows the jump hints.
    pub hint_key: HintKey,
    /// Serve what's playing on 127.0.0.1 for widgets (this device only).
    pub now_playing_feed: bool,
    /// The feed's port (this device only).
    pub now_playing_feed_port: u16,
}

/// Longest model name accepted.
pub const AI_MODEL_MAX: usize = 100;

impl Default for Settings {
    fn default() -> Self {
        Self {
            private_session: false,
            report_to_plex: true,
            volume: 0.7,
            shuffle_style: ShuffleStyle::Spread,
            quality_home: StreamQuality::Original,
            quality_away: StreamQuality::Kbps320,
            show_format_badge: true,
            resume_contexts: true,
            precache_count: 5,
            popular_source: PopularSource::Global,
            art_cache_mb: 2048,
            artwork_colours: true,
            reduce_motion: ReduceMotion::System,
            output_device: None,
            home_shortcuts: true,
            home_colour_wash: true,
            search_suggestions: true,
            search_recents: true,
            media_keys: true,
            media_session: true,
            close_to_tray: false,
            launch_at_login: false,
            ab_loop_handles: true,
            up_next_line: true,
            volume_percent: true,
            notify_track_change: false,
            playlist_ask_duplicates: true,
            playlist_new_name: crate::db::playlists::DEFAULT_NAME.to_string(),
            info_wikipedia: true,
            info_lastfm: false,
            info_fanart: false,
            info_audiodb: false,
            info_listenbrainz: true,
            link_featured: true,
            liked_means: LikedMeans::Five,
            like_control_rows: LikeControlRows::Hover,
            show_plex_players: true,
            act_as_plex_player: false,
            move_stops_old_device: true,
            crossfade: CrossfadeMode::Smart,
            crossfade_seconds: 6,
            crossfade_skip_albums: true,
            leveling: Leveling::Smart,
            preamp_db: 0.0,
            limiter: true,
            eq_enabled: false,
            eq_bands: [0.0; EQ_BANDS],
            eq_preset: "flat".into(),
            eq_profiles: Vec::new(),
            eq_outputs: Vec::new(),
            eq_fallback: None,
            eq_last_output: None,
            share_public_links: true,
            share_card_qr: true,
            share_card_song_link: true,
            show_cast_devices: true,
            discord_status: true,
            discord_show_unreleased: true,
            discord_unreleased_art: true,
            discord_clear_on_pause: true,
            discord_find_song: true,
            discord_cover_archive: true,
            discord_upload_art: false,
            discord_upload_catbox: false,
            discord_status_line: StatusLine::App,
            show_plex_avatar: true,
            lyrics_mode: LyricsMode::AutoShow,
            lyrics_lrclib: false,
            lyrics_prefer_own: false,
            lyrics_show_add: true,
            full_window_hide_controls: true,
            read_file_tags: true,
            explicit_content: ExplicitContent::Allowed,
            show_fan_edits: false,
            group_editions: true,
            drag_and_drop: true,
            transcribe_lyrics: false,
            transcribe_model: Default::default(),
            show_auto_lyrics: true,
            show_to_my_devices: true,
            computer_name: String::new(),
            show_airplay_locked: false,
            download_quality: DownloadQuality::Original,
            download_limit_gb: 20,
            download_folder: String::new(),
            download_home_only: true,
            download_liked: false,
            go_offline: false,
            home_mixes: true,
            mix_variety: crate::mixes::Variety::Balanced,
            mix_exclude_artists: Vec::new(),
            mix_exclude_genres: Vec::new(),
            mix_exclude_albums: Vec::new(),
            mix_unreleased: true,
            daylist_hours: 3,
            queue_end: QueueEnd::KeepPlaying,
            smart_shuffle: false,
            show_friend_activity: true,
            share_activity: true,
            whats_new_announcements: true,
            notify_new_releases: false,
            listen_guests_control: true,
            listen_relay: String::new(),
            listen_invite_hours: 24,
            listen_name: String::new(),
            discord_listen_along: true,
            sync_enabled: false,
            sync_folder: String::new(),
            sync_settings: true,
            sync_library: true,
            sync_follows: true,
            sync_lyrics: true,
            bookmarks_in_library: true,
            scrobble_lastfm: false,
            scrobble_listenbrainz: false,
            scrobble_unreleased: false,
            ai_provider: crate::smart::prompt::AiProvider::None,
            ai_model: crate::smart::prompt::DEFAULT_MODEL.to_string(),
            miniplayer_on_top: true,
            miniplayer_tint: true,
            miniplayer_queue: false,
            miniplayer_bounds: None,
            power_keys: false,
            hint_key: HintKey::F,
            now_playing_feed: false,
            now_playing_feed_port: crate::feed::DEFAULT_PORT,
        }
    }
}

/// Settings that travel in the sync file (camelCase names, as on the wire).
/// Everything else is this device only. Every setting is in exactly one of
/// the two lists (a test checks).
pub const SYNCED_SETTINGS: &[&str] = &[
    "reportToPlex",
    "shuffleStyle",
    "qualityHome",
    "qualityAway",
    "showFormatBadge",
    "resumeContexts",
    "popularSource",
    "artworkColours",
    "reduceMotion",
    "homeShortcuts",
    "homeColourWash",
    "searchSuggestions",
    "abLoopHandles",
    "upNextLine",
    "volumePercent",
    "playlistAskDuplicates",
    "playlistNewName",
    "infoWikipedia",
    "infoLastfm",
    "infoFanart",
    "infoAudiodb",
    "infoListenbrainz",
    "linkFeatured",
    "likedMeans",
    "likeControlRows",
    "showPlexPlayers",
    "moveStopsOldDevice",
    "crossfade",
    "crossfadeSeconds",
    "crossfadeSkipAlbums",
    "leveling",
    "sharePublicLinks",
    "shareCardQr",
    "shareCardSongLink",
    "showCastDevices",
    "lyricsMode",
    "lyricsLrclib",
    "lyricsPreferOwn",
    "lyricsShowAdd",
    "fullWindowHideControls",
    "explicitContent",
    "showFanEdits",
    "groupEditions",
    "dragAndDrop",
    "showAutoLyrics",
    "homeMixes",
    "mixVariety",
    "mixExcludeArtists",
    "mixExcludeGenres",
    "mixExcludeAlbums",
    "mixUnreleased",
    "daylistHours",
    "queueEnd",
    "smartShuffle",
    "showFriendActivity",
    "shareActivity",
    "whatsNewAnnouncements",
    "listenGuestsControl",
    "listenRelay",
    "listenInviteHours",
    "listenName",
    "bookmarksInLibrary",
    "scrobbleUnreleased",
    "aiModel",
    "powerKeys",
    "hintKey",
];

/// Settings kept on this device only.
pub const DEVICE_SETTINGS: &[&str] = &[
    "privateSession",
    "volume",
    "precacheCount",
    "artCacheMb",
    "outputDevice",
    "searchRecents",
    "mediaKeys",
    "mediaSession",
    "closeToTray",
    "launchAtLogin",
    "notifyTrackChange",
    "actAsPlexPlayer",
    "preampDb",
    "limiter",
    "eqEnabled",
    "eqBands",
    "eqPreset",
    "eqProfiles",
    "eqOutputs",
    "eqFallback",
    "eqLastOutput",
    "discordStatus",
    "discordShowUnreleased",
    "discordUnreleasedArt",
    "discordClearOnPause",
    "discordFindSong",
    "discordCoverArchive",
    "discordUploadArt",
    "discordUploadCatbox",
    "discordStatusLine",
    "showPlexAvatar",
    "readFileTags",
    "transcribeLyrics",
    "transcribeModel",
    "showToMyDevices",
    "computerName",
    "showAirplayLocked",
    "downloadQuality",
    "downloadLimitGb",
    "downloadFolder",
    "downloadHomeOnly",
    "downloadLiked",
    "goOffline",
    "notifyNewReleases",
    "discordListenAlong",
    "syncEnabled",
    "syncFolder",
    "syncSettings",
    "syncLibrary",
    "syncFollows",
    "syncLyrics",
    // Signed in, and keys kept, per device.
    "scrobbleLastfm",
    "scrobbleListenbrainz",
    "aiProvider",
    "miniplayerOnTop",
    "miniplayerTint",
    "miniplayerQueue",
    "miniplayerBounds",
    "nowPlayingFeed",
    "nowPlayingFeedPort",
];

/// A partial update; absent fields are left alone.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub private_session: Option<bool>,
    pub report_to_plex: Option<bool>,
    pub volume: Option<f32>,
    pub shuffle_style: Option<ShuffleStyle>,
    pub quality_home: Option<StreamQuality>,
    pub quality_away: Option<StreamQuality>,
    pub show_format_badge: Option<bool>,
    pub resume_contexts: Option<bool>,
    pub precache_count: Option<u32>,
    pub popular_source: Option<PopularSource>,
    pub art_cache_mb: Option<u32>,
    pub artwork_colours: Option<bool>,
    pub reduce_motion: Option<ReduceMotion>,
    pub home_shortcuts: Option<bool>,
    pub home_colour_wash: Option<bool>,
    pub search_suggestions: Option<bool>,
    pub search_recents: Option<bool>,
    pub media_keys: Option<bool>,
    pub media_session: Option<bool>,
    pub close_to_tray: Option<bool>,
    pub launch_at_login: Option<bool>,
    pub ab_loop_handles: Option<bool>,
    pub up_next_line: Option<bool>,
    pub volume_percent: Option<bool>,
    pub notify_track_change: Option<bool>,
    pub playlist_ask_duplicates: Option<bool>,
    pub playlist_new_name: Option<String>,
    pub info_wikipedia: Option<bool>,
    pub info_lastfm: Option<bool>,
    pub info_fanart: Option<bool>,
    pub info_audiodb: Option<bool>,
    pub info_listenbrainz: Option<bool>,
    pub link_featured: Option<bool>,
    pub liked_means: Option<LikedMeans>,
    pub like_control_rows: Option<LikeControlRows>,
    pub show_plex_players: Option<bool>,
    pub act_as_plex_player: Option<bool>,
    pub move_stops_old_device: Option<bool>,
    pub crossfade: Option<CrossfadeMode>,
    pub crossfade_seconds: Option<u32>,
    pub crossfade_skip_albums: Option<bool>,
    pub leveling: Option<Leveling>,
    pub preamp_db: Option<f32>,
    pub limiter: Option<bool>,
    pub eq_enabled: Option<bool>,
    /// Moving bands makes the curve custom.
    pub eq_bands: Option<[f32; EQ_BANDS]>,
    /// Selects a preset or profile (and its curve).
    pub eq_preset: Option<String>,
    pub share_public_links: Option<bool>,
    pub share_card_qr: Option<bool>,
    pub share_card_song_link: Option<bool>,
    pub show_cast_devices: Option<bool>,
    pub discord_status: Option<bool>,
    pub discord_show_unreleased: Option<bool>,
    pub discord_unreleased_art: Option<bool>,
    pub discord_clear_on_pause: Option<bool>,
    pub discord_find_song: Option<bool>,
    pub discord_cover_archive: Option<bool>,
    pub discord_upload_art: Option<bool>,
    pub discord_upload_catbox: Option<bool>,
    pub discord_status_line: Option<StatusLine>,
    pub show_plex_avatar: Option<bool>,
    pub lyrics_mode: Option<LyricsMode>,
    pub lyrics_lrclib: Option<bool>,
    pub lyrics_prefer_own: Option<bool>,
    pub lyrics_show_add: Option<bool>,
    pub full_window_hide_controls: Option<bool>,
    pub read_file_tags: Option<bool>,
    pub explicit_content: Option<ExplicitContent>,
    pub show_fan_edits: Option<bool>,
    pub group_editions: Option<bool>,
    pub drag_and_drop: Option<bool>,
    pub transcribe_lyrics: Option<bool>,
    pub transcribe_model: Option<crate::lyrics::transcribe::model::ModelKind>,
    pub show_auto_lyrics: Option<bool>,
    pub show_to_my_devices: Option<bool>,
    /// Trimmed; empty goes back to the OS computer name.
    pub computer_name: Option<String>,
    pub show_airplay_locked: Option<bool>,
    pub download_quality: Option<DownloadQuality>,
    pub download_limit_gb: Option<u32>,
    pub download_home_only: Option<bool>,
    pub download_liked: Option<bool>,
    pub go_offline: Option<bool>,
    pub home_mixes: Option<bool>,
    pub mix_variety: Option<crate::mixes::Variety>,
    pub mix_exclude_artists: Option<Vec<String>>,
    pub mix_exclude_genres: Option<Vec<String>>,
    pub mix_exclude_albums: Option<Vec<String>>,
    pub mix_unreleased: Option<bool>,
    /// 1, 3 or 6.
    pub daylist_hours: Option<u32>,
    pub queue_end: Option<QueueEnd>,
    pub smart_shuffle: Option<bool>,
    pub show_friend_activity: Option<bool>,
    pub share_activity: Option<bool>,
    pub whats_new_announcements: Option<bool>,
    pub notify_new_releases: Option<bool>,
    pub listen_guests_control: Option<bool>,
    pub listen_relay: Option<String>,
    /// 1, 24 or 168.
    pub listen_invite_hours: Option<u32>,
    pub listen_name: Option<String>,
    pub discord_listen_along: Option<bool>,
    pub sync_enabled: Option<bool>,
    pub sync_settings: Option<bool>,
    pub sync_library: Option<bool>,
    pub sync_follows: Option<bool>,
    pub sync_lyrics: Option<bool>,
    pub bookmarks_in_library: Option<bool>,
    pub scrobble_lastfm: Option<bool>,
    pub scrobble_listenbrainz: Option<bool>,
    pub scrobble_unreleased: Option<bool>,
    pub ai_provider: Option<crate::smart::prompt::AiProvider>,
    pub ai_model: Option<String>,
    pub miniplayer_on_top: Option<bool>,
    pub miniplayer_tint: Option<bool>,
    pub miniplayer_queue: Option<bool>,
    pub power_keys: Option<bool>,
    pub hint_key: Option<HintKey>,
    pub now_playing_feed: Option<bool>,
    pub now_playing_feed_port: Option<u16>,
}

impl SettingsPatch {
    /// Whether the patch changes the miniplayer (both windows hear about it).
    pub fn touches_miniplayer(&self) -> bool {
        self.miniplayer_on_top.is_some() || self.miniplayer_tint.is_some() || self.miniplayer_queue.is_some()
    }
}

impl SettingsPatch {
    /// Whether the patch changes what goes in mixes.
    pub fn touches_mixes(&self) -> bool {
        self.mix_variety.is_some()
            || self.mix_exclude_artists.is_some()
            || self.mix_exclude_genres.is_some()
            || self.mix_exclude_albums.is_some()
            || self.mix_unreleased.is_some()
            || self.daylist_hours.is_some()
            || self.explicit_content.is_some()
            || self.show_fan_edits.is_some()
            || self.liked_means.is_some()
    }

    /// Whether the patch touches the player's sound stage.
    pub fn touches_dsp(&self) -> bool {
        self.crossfade.is_some()
            || self.crossfade_seconds.is_some()
            || self.crossfade_skip_albums.is_some()
            || self.leveling.is_some()
            || self.preamp_db.is_some()
            || self.limiter.is_some()
            || self.eq_enabled.is_some()
            || self.eq_bands.is_some()
            || self.eq_preset.is_some()
    }
}

fn set<T: Copy>(dst: &mut T, v: Option<T>) {
    if let Some(v) = v {
        *dst = v;
    }
}

impl Settings {
    pub fn apply(&mut self, p: &SettingsPatch) -> AppResult<()> {
        if let Some(v) = p.volume {
            if !v.is_finite() {
                return Err(AppError::Invalid("volume must be a number".into()));
            }
            self.volume = v.clamp(0.0, 1.0);
        }
        set(&mut self.private_session, p.private_session);
        set(&mut self.report_to_plex, p.report_to_plex);
        set(&mut self.shuffle_style, p.shuffle_style);
        set(&mut self.quality_home, p.quality_home);
        set(&mut self.quality_away, p.quality_away);
        set(&mut self.show_format_badge, p.show_format_badge);
        set(&mut self.resume_contexts, p.resume_contexts);
        set(&mut self.precache_count, p.precache_count);
        set(&mut self.popular_source, p.popular_source);
        set(&mut self.art_cache_mb, p.art_cache_mb);
        set(&mut self.artwork_colours, p.artwork_colours);
        set(&mut self.reduce_motion, p.reduce_motion);
        set(&mut self.home_shortcuts, p.home_shortcuts);
        set(&mut self.home_colour_wash, p.home_colour_wash);
        set(&mut self.search_suggestions, p.search_suggestions);
        set(&mut self.search_recents, p.search_recents);
        set(&mut self.media_keys, p.media_keys);
        set(&mut self.media_session, p.media_session);
        set(&mut self.close_to_tray, p.close_to_tray);
        set(&mut self.launch_at_login, p.launch_at_login);
        set(&mut self.ab_loop_handles, p.ab_loop_handles);
        set(&mut self.up_next_line, p.up_next_line);
        set(&mut self.volume_percent, p.volume_percent);
        set(&mut self.notify_track_change, p.notify_track_change);
        set(&mut self.playlist_ask_duplicates, p.playlist_ask_duplicates);
        set(&mut self.info_wikipedia, p.info_wikipedia);
        set(&mut self.info_lastfm, p.info_lastfm);
        set(&mut self.info_fanart, p.info_fanart);
        set(&mut self.info_audiodb, p.info_audiodb);
        set(&mut self.info_listenbrainz, p.info_listenbrainz);
        set(&mut self.link_featured, p.link_featured);
        set(&mut self.liked_means, p.liked_means);
        set(&mut self.like_control_rows, p.like_control_rows);
        set(&mut self.show_plex_players, p.show_plex_players);
        set(&mut self.act_as_plex_player, p.act_as_plex_player);
        set(&mut self.move_stops_old_device, p.move_stops_old_device);
        set(&mut self.share_public_links, p.share_public_links);
        set(&mut self.share_card_qr, p.share_card_qr);
        set(&mut self.share_card_song_link, p.share_card_song_link);
        set(&mut self.miniplayer_on_top, p.miniplayer_on_top);
        set(&mut self.miniplayer_tint, p.miniplayer_tint);
        set(&mut self.miniplayer_queue, p.miniplayer_queue);
        set(&mut self.power_keys, p.power_keys);
        set(&mut self.hint_key, p.hint_key);
        set(&mut self.now_playing_feed, p.now_playing_feed);
        if let Some(port) = p.now_playing_feed_port {
            if port < crate::feed::PORT_MIN {
                return Err(AppError::Invalid("Pick a port from 1024 to 65535.".into()));
            }
            self.now_playing_feed_port = port;
        }
        set(&mut self.show_cast_devices, p.show_cast_devices);
        set(&mut self.discord_status, p.discord_status);
        set(&mut self.discord_show_unreleased, p.discord_show_unreleased);
        set(&mut self.discord_unreleased_art, p.discord_unreleased_art);
        set(&mut self.discord_clear_on_pause, p.discord_clear_on_pause);
        set(&mut self.discord_find_song, p.discord_find_song);
        set(&mut self.discord_cover_archive, p.discord_cover_archive);
        set(&mut self.discord_upload_art, p.discord_upload_art);
        set(&mut self.discord_upload_catbox, p.discord_upload_catbox);
        set(&mut self.discord_status_line, p.discord_status_line);
        set(&mut self.show_plex_avatar, p.show_plex_avatar);
        set(&mut self.lyrics_mode, p.lyrics_mode);
        set(&mut self.lyrics_lrclib, p.lyrics_lrclib);
        set(&mut self.lyrics_prefer_own, p.lyrics_prefer_own);
        set(&mut self.lyrics_show_add, p.lyrics_show_add);
        set(&mut self.full_window_hide_controls, p.full_window_hide_controls);
        set(&mut self.read_file_tags, p.read_file_tags);
        set(&mut self.explicit_content, p.explicit_content);
        set(&mut self.show_fan_edits, p.show_fan_edits);
        set(&mut self.group_editions, p.group_editions);
        set(&mut self.drag_and_drop, p.drag_and_drop);
        set(&mut self.transcribe_lyrics, p.transcribe_lyrics);
        set(&mut self.transcribe_model, p.transcribe_model);
        set(&mut self.show_auto_lyrics, p.show_auto_lyrics);
        set(&mut self.show_to_my_devices, p.show_to_my_devices);
        set(&mut self.show_airplay_locked, p.show_airplay_locked);
        set(&mut self.download_quality, p.download_quality);
        set(&mut self.download_limit_gb, p.download_limit_gb);
        set(&mut self.download_home_only, p.download_home_only);
        set(&mut self.download_liked, p.download_liked);
        set(&mut self.go_offline, p.go_offline);
        set(&mut self.home_mixes, p.home_mixes);
        set(&mut self.mix_variety, p.mix_variety);
        set(&mut self.mix_unreleased, p.mix_unreleased);
        set(&mut self.queue_end, p.queue_end);
        set(&mut self.smart_shuffle, p.smart_shuffle);
        set(&mut self.sync_enabled, p.sync_enabled);
        set(&mut self.sync_settings, p.sync_settings);
        set(&mut self.sync_library, p.sync_library);
        set(&mut self.sync_follows, p.sync_follows);
        set(&mut self.sync_lyrics, p.sync_lyrics);
        if let Some(h) = p.daylist_hours {
            if !DAYLIST_HOURS.contains(&h) {
                return Err(AppError::Invalid("The daylist refreshes every 1, 3 or 6 hours.".into()));
            }
            self.daylist_hours = h;
        }
        let tidy = |v: &Vec<String>| {
            let mut out: Vec<String> = Vec::new();
            for x in v.iter().map(|x| x.trim()).filter(|x| !x.is_empty()) {
                if !out.iter().any(|o| o.eq_ignore_ascii_case(x)) {
                    out.push(x.to_string());
                }
            }
            out
        };
        if let Some(v) = &p.mix_exclude_artists {
            self.mix_exclude_artists = tidy(v);
        }
        if let Some(v) = &p.mix_exclude_genres {
            self.mix_exclude_genres = tidy(v);
        }
        if let Some(v) = &p.mix_exclude_albums {
            self.mix_exclude_albums = tidy(v);
        }
        set(&mut self.show_friend_activity, p.show_friend_activity);
        set(&mut self.share_activity, p.share_activity);
        set(&mut self.whats_new_announcements, p.whats_new_announcements);
        set(&mut self.notify_new_releases, p.notify_new_releases);
        set(&mut self.listen_guests_control, p.listen_guests_control);
        if let Some(r) = &p.listen_relay {
            self.listen_relay = if r.trim().is_empty() {
                String::new()
            } else {
                crate::social::listen::invite::relay_url(r)
                    .ok_or_else(|| AppError::Invalid("The relay is a web address like https://ntfy.example.com.".into()))?
            };
        }
        if let Some(h) = p.listen_invite_hours {
            if !crate::social::listen::INVITE_HOURS.contains(&h) {
                return Err(AppError::Invalid("Invites expire after 1 hour, 24 hours or 7 days.".into()));
            }
            self.listen_invite_hours = h;
        }
        if let Some(n) = &p.listen_name {
            let n = n.trim();
            if n.chars().count() > crate::social::listen::NAME_MAX || n.chars().any(char::is_control) {
                return Err(AppError::Invalid("Names can be up to 40 characters.".into()));
            }
            self.listen_name = n.to_string();
        }
        set(&mut self.discord_listen_along, p.discord_listen_along);
        set(&mut self.bookmarks_in_library, p.bookmarks_in_library);
        set(&mut self.scrobble_lastfm, p.scrobble_lastfm);
        set(&mut self.scrobble_listenbrainz, p.scrobble_listenbrainz);
        set(&mut self.scrobble_unreleased, p.scrobble_unreleased);
        set(&mut self.ai_provider, p.ai_provider);
        if let Some(m) = &p.ai_model {
            let m = m.trim();
            let ok = m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '@' | '/'));
            if m.chars().count() > AI_MODEL_MAX || !ok {
                return Err(AppError::Invalid("Model names are letters, numbers, dots and dashes.".into()));
            }
            self.ai_model = if m.is_empty() { crate::smart::prompt::DEFAULT_MODEL.to_string() } else { m.to_string() };
        }
        if let Some(name) = &p.computer_name {
            let name = name.trim();
            if name.chars().count() > COMPUTER_NAME_MAX {
                return Err(AppError::Invalid("Computer names can be up to 60 characters.".into()));
            }
            self.computer_name = name.to_string();
        }
        if let Some(name) = &p.playlist_new_name {
            let name = name.trim();
            if name.chars().count() > crate::db::playlists::TITLE_MAX {
                return Err(AppError::Invalid("Playlist names can be up to 100 characters.".into()));
            }
            self.playlist_new_name = name.to_string();
        }
        eqprofile::apply_patch(self, p)?;
        self.clamp();
        Ok(())
    }

    /// Keep ranged values in range (also fixes hand-edited stored values).
    fn clamp(&mut self) {
        self.precache_count = self.precache_count.clamp(PRECACHE_MIN, PRECACHE_MAX);
        self.art_cache_mb = self.art_cache_mb.clamp(ART_CACHE_MIN_MB, ART_CACHE_MAX_MB);
        self.download_limit_gb = self.download_limit_gb.clamp(downloads::LIMIT_MIN_GB, downloads::LIMIT_MAX_GB);
        if !DAYLIST_HOURS.contains(&self.daylist_hours) {
            self.daylist_hours = 3;
        }
        eqprofile::clamp(self);
    }

    /// Crossfade, leveling, EQ and limiter for the player.
    pub fn dsp(&self) -> crate::audio::dsp::DspSettings {
        eqprofile::dsp(self)
    }

    /// The lowest Plex rating that counts as liked.
    pub fn liked_min_rating(&self) -> f64 {
        self.liked_means.min_rating()
    }

    pub fn art_cache_bytes(&self) -> u64 {
        self.art_cache_mb as u64 * MB
    }

    pub fn platform(&self) -> PlatformSettings {
        PlatformSettings {
            media_keys: self.media_keys,
            media_session: self.media_session,
            close_to_tray: self.close_to_tray,
            launch_at_login: self.launch_at_login,
        }
    }

    /// What shapes the mixes.
    pub fn mix_options(&self) -> crate::mixes::MixOptions {
        crate::mixes::MixOptions {
            variety: self.mix_variety,
            exclude_artists: self.mix_exclude_artists.clone(),
            exclude_genres: self.mix_exclude_genres.clone(),
            exclude_albums: self.mix_exclude_albums.clone(),
            include_unreleased: self.mix_unreleased,
            daylist_hours: self.daylist_hours,
            tag_view: self.tag_view(),
            liked_min: self.liked_min_rating(),
            // The mix service adds the hidden songs and artists itself.
            hidden: Default::default(),
        }
    }

    /// How file tags shape lists and play contexts.
    pub fn tag_view(&self) -> crate::tags::TagView {
        crate::tags::TagView {
            group_editions: self.group_editions,
            show_edits: self.show_fan_edits,
            filter_explicit: self.explicit_content == ExplicitContent::Filtered,
        }
    }

    /// What the download queue follows.
    pub fn download_prefs(&self) -> downloads::Prefs {
        downloads::Prefs {
            quality: self.download_quality,
            limit_bytes: self.download_limit_gb as u64 * 1024 * MB,
            home_only: self.download_home_only,
            offline: self.go_offline,
        }
    }

    /// The downloads folder: the chosen one, or `default`.
    pub fn download_dir(&self, default: &std::path::Path) -> std::path::PathBuf {
        if self.download_folder.trim().is_empty() {
            default.to_path_buf()
        } else {
            std::path::PathBuf::from(&self.download_folder)
        }
    }

    /// What the sync engine follows. On with no folder chosen counts as off.
    pub fn sync_config(&self) -> crate::sync::engine::SyncConfig {
        let folder = self.sync_folder.trim();
        crate::sync::engine::SyncConfig {
            enabled: self.sync_enabled && !folder.is_empty(),
            folder: (!folder.is_empty()).then(|| std::path::PathBuf::from(folder)),
            groups: crate::sync::engine::Groups {
                settings: self.sync_settings,
                library: self.sync_library,
                follows: self.sync_follows,
                lyrics: self.sync_lyrics,
            },
        }
    }

    pub fn discord(&self) -> DiscordSettings {
        DiscordSettings {
            // Go offline hides the status too.
            enabled: self.discord_status && !self.go_offline,
            show_unreleased: self.discord_show_unreleased,
            unreleased_art: self.discord_unreleased_art,
            clear_on_pause: self.discord_clear_on_pause,
            find_song: self.discord_find_song,
            cover_archive: self.discord_cover_archive,
            upload_art: self.discord_upload_art,
            upload_catbox: self.discord_upload_catbox,
            status_line: self.discord_status_line,
            private_session: self.private_session,
            listen_along: self.discord_listen_along,
        }
    }
}

/// Read settings; a missing or unreadable value gives the defaults.
pub fn load(db: &DbHandle) -> Settings {
    let mut s: Settings = db
        .read(|c| kv_get(c, KEY))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    s.clamp();
    s
}

pub fn save(db: &DbHandle, s: &Settings) -> AppResult<()> {
    let json = serde_json::to_string(s).map_err(|e| AppError::Internal(e.to_string()))?;
    db.write(|c| kv_set(c, KEY, &json))
}

/// The parts of the core that read settings directly (no player round trip).
#[derive(Clone, Default)]
pub struct Live {
    pub tracking: TrackingSettings,
    pub streams: Arc<StreamPrefs>,
    /// The art cache size the disk monitor returns to after trimming.
    pub art_cap: Arc<AtomicU64>,
    pub art: Option<ArtCache>,
    /// Media controls, tray and launch at login.
    pub platform: Switches,
    /// Desktop notification on track change.
    pub notify_track_change: Arc<AtomicBool>,
    /// Which artist info sources are on.
    pub artist_info: crate::artistinfo::Switches,
    /// Listen on: players in the picker, Platter as a player, stop on move.
    pub remote: crate::remote::RemoteSwitches,
    /// Public links for released music in the Share menus.
    pub share_public_links: Arc<AtomicBool>,
    /// Discord status.
    pub discord: DiscordSwitches,
    /// Lyrics from LRCLIB when Plex has none.
    pub lyrics_lrclib: Arc<AtomicBool>,
    /// The user's own lyrics ahead of Plex's.
    pub lyrics_prefer_own: Arc<AtomicBool>,
    /// Show lyrics transcribed on this computer.
    pub lyrics_show_auto: Arc<AtomicBool>,
    /// The file tag scan, once it exists (it starts after settings load).
    pub tag_scanner: Arc<parking_lot::Mutex<Option<crate::tags::scan::TagScanner>>>,
    /// Lyrics transcription, once it exists.
    pub transcriber: Arc<parking_lot::Mutex<Option<crate::lyrics::transcribe::Transcriber>>>,
    /// The download queue, once it exists.
    pub downloads: Arc<parking_lot::Mutex<Option<downloads::Downloads>>>,
    /// The Plex connection (Go offline stops it).
    pub plex: Option<crate::plex::PlexState>,
    /// The mix engine, once it exists.
    pub mixes: Arc<parking_lot::Mutex<Option<crate::mixes::MixService>>>,
    /// Friend activity, What's New and listening sessions.
    pub social: crate::social::SocialSwitches,
    /// The sync file settings the sync engine follows.
    pub sync: crate::sync::engine::ConfigSwitch,
    /// Scrobbling.
    pub scrobble: crate::scrobble::ScrobbleSwitches,
    /// The now-playing feed for widgets.
    pub feed: crate::feed::FeedSwitch,
}

/// Push settings to the tracker, stream requests, Popular and the art cache.
pub fn publish(s: &Settings, live: &Live, db: &DbHandle) {
    // Go offline turns off everything that reaches out to the network.
    let on = !s.go_offline;
    if let Some(plex) = &live.plex {
        plex.set_offline_by_choice(s.go_offline);
    }
    if let Some(d) = live.downloads.lock().as_ref() {
        d.set_prefs(s.download_prefs());
    }
    live.tracking.private_session.store(s.private_session, Ordering::Relaxed);
    live.tracking.report_to_plex.store(s.report_to_plex, Ordering::Relaxed);
    live.tracking.share_activity.store(s.share_activity, Ordering::Relaxed);
    {
        let so = &live.social;
        so.show_activity.store(s.show_friend_activity && on, Ordering::Relaxed);
        so.whats_new.store(s.whats_new_announcements, Ordering::Relaxed);
        so.notify_releases.store(s.notify_new_releases, Ordering::Relaxed);
        so.listen.guests_control.store(s.listen_guests_control, Ordering::Relaxed);
        *so.listen.relay.write() = s.listen_relay.clone();
        so.listen.invite_hours.store(s.listen_invite_hours, Ordering::Relaxed);
        *so.listen.name.write() = s.listen_name.clone();
        so.private_session.store(s.private_session, Ordering::Relaxed);
    }
    {
        let sc = &live.scrobble;
        sc.lastfm.store(s.scrobble_lastfm, Ordering::Relaxed);
        sc.listenbrainz.store(s.scrobble_listenbrainz, Ordering::Relaxed);
        sc.unreleased.store(s.scrobble_unreleased, Ordering::Relaxed);
        sc.private_session.store(s.private_session, Ordering::Relaxed);
    }
    live.feed.set(crate::feed::FeedConfig {
        enabled: s.now_playing_feed,
        port: s.now_playing_feed_port,
        private_session: s.private_session,
    });
    live.streams.set(s.quality_home.kbps(), s.quality_away.kbps());
    live.notify_track_change.store(s.notify_track_change, Ordering::Relaxed);
    live.share_public_links.store(s.share_public_links && on, Ordering::Relaxed);
    live.lyrics_lrclib.store(s.lyrics_lrclib && on, Ordering::Relaxed);
    live.lyrics_prefer_own.store(s.lyrics_prefer_own, Ordering::Relaxed);
    live.lyrics_show_auto.store(s.show_auto_lyrics, Ordering::Relaxed);
    if let Some(t) = live.transcriber.lock().as_ref() {
        t.set_options(s.transcribe_lyrics, s.transcribe_model);
    }
    if let Some(m) = live.mixes.lock().as_ref() {
        m.set_options(s.mix_options());
    }
    // Global with no source for it on quietly ranks by plays.
    db.set_popular_source(if s.info_listenbrainz || s.info_lastfm { s.popular_source } else { PopularSource::MyPlays });
    db.set_link_featured(s.link_featured);
    {
        use crate::artistinfo::InfoSource as Src;
        let ai = &live.artist_info;
        ai.set(Src::Wikipedia, s.info_wikipedia && on);
        ai.set(Src::Lastfm, s.info_lastfm && on);
        ai.set(Src::Fanart, s.info_fanart && on);
        ai.set(Src::Audiodb, s.info_audiodb && on);
        ai.set(Src::Listenbrainz, s.info_listenbrainz && on);
    }
    live.platform.set(s.platform());
    live.sync.set(s.sync_config());
    live.discord.set(s.discord());
    db.set_tag_view(s.tag_view());
    if let Some(scan) = live.tag_scanner.lock().as_ref() {
        scan.set_enabled(s.read_file_tags);
    }
    live.remote.set(s.show_plex_players && on, s.move_stops_old_device, s.act_as_plex_player && on);
    live.remote.set_show_cast(s.show_cast_devices && on);
    live.remote.set_peer(s.show_to_my_devices && on, &s.computer_name, s.show_airplay_locked);
    let cap = s.art_cache_bytes();
    let old = live.art_cap.swap(cap, Ordering::Relaxed);
    if let Some(art) = &live.art
        && (old != cap || art.cap() > cap)
        && let Err(e) = art.set_cap(cap)
    {
        tracing::warn!("art cache resize failed: {e}");
    }
}

/// Managed state for the settings commands.
#[derive(Clone)]
pub struct SettingsState {
    db: DbHandle,
    live: Live,
    lock: Arc<tokio::sync::Mutex<()>>,
    /// Synced settings go here after every save (once attached).
    sync: Arc<parking_lot::Mutex<Option<crate::sync::SyncStore>>>,
}

impl SettingsState {
    /// Load stored settings and publish them.
    pub fn new(db: DbHandle, live: Live) -> Self {
        publish(&load(&db), &live, &db);
        Self { db, live, lock: Default::default(), sync: Default::default() }
    }

    /// Send synced settings through the sync store from now on. Existing
    /// choices are seeded the first time.
    pub async fn attach_sync(&self, store: crate::sync::SyncStore) -> AppResult<()> {
        let s = self.get().await?;
        store
            .run(move |st| {
                crate::sync::settings::seed(st, &s)?;
                crate::sync::settings::push(st, &s)
            })
            .await?;
        *self.sync.lock() = Some(store);
        Ok(())
    }

    /// The sync settings as they are now.
    pub fn live(&self) -> &Live {
        &self.live
    }

    async fn push_synced(&self, s: &Settings) {
        let Some(store) = self.sync.lock().clone() else { return };
        let s = s.clone();
        if let Err(e) = store.run(move |st| crate::sync::settings::push(st, &s)).await {
            tracing::warn!("synced settings not stored: {e}");
        }
    }

    pub async fn get(&self) -> AppResult<Settings> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || load(&db))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))
    }

    /// Apply a patch, persist it and publish it.
    pub async fn set(&self, patch: SettingsPatch) -> AppResult<Settings> {
        let _guard = self.lock.lock().await;
        let mut s = self.get().await?;
        s.apply(&patch)?;
        let (db, saved) = (self.db.clone(), s.clone());
        tokio::task::spawn_blocking(move || save(&db, &saved))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))??;
        let (live, db) = (self.live.clone(), self.db.clone());
        let published = s.clone();
        // Shrinking the art cache deletes files.
        tokio::task::spawn_blocking(move || publish(&published, &live, &db))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        self.push_synced(&s).await;
        Ok(s)
    }

    /// Change settings with `f` (it returns whether anything changed) and
    /// save them. Returns the settings if they changed.
    pub async fn modify(&self, f: impl FnOnce(&mut Settings) -> AppResult<bool>) -> AppResult<Option<Settings>> {
        let _guard = self.lock.lock().await;
        let mut s = self.get().await?;
        if !f(&mut s)? {
            return Ok(None);
        }
        s.clamp();
        let (db, saved) = (self.db.clone(), s.clone());
        tokio::task::spawn_blocking(move || save(&db, &saved))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))??;
        self.push_synced(&s).await;
        Ok(Some(s))
    }

    /// [`modify`](Self::modify), then publish the result.
    pub async fn modify_and_publish(&self, f: impl FnOnce(&mut Settings) -> AppResult<bool>) -> AppResult<Option<Settings>> {
        let out = self.modify(f).await?;
        if let Some(s) = &out {
            let (live, db, s) = (self.live.clone(), self.db.clone(), s.clone());
            tokio::task::spawn_blocking(move || publish(&s, &live, &db))
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
        Ok(out)
    }

    /// Store the chosen output device (None: the system default).
    pub async fn set_output_device(&self, id: Option<String>) -> AppResult<()> {
        let _guard = self.lock.lock().await;
        let mut s = self.get().await?;
        if s.output_device == id {
            return Ok(());
        }
        s.output_device = id;
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || save(&db, &s))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
    }
}

/// Push the player's settings (the ones the patch touched).
async fn apply_to_player(player: &PlayerHandle, s: &Settings, patch: &SettingsPatch) -> AppResult<()> {
    if patch.volume.is_some() {
        player.set_volume(s.volume).await?;
    }
    if patch.shuffle_style.is_some() {
        player.set_shuffle_style(s.shuffle_style).await?;
    }
    if patch.resume_contexts.is_some() {
        player.set_resume_contexts(s.resume_contexts).await?;
    }
    if patch.precache_count.is_some() {
        player.set_precache_count(s.precache_count as usize).await?;
    }
    // Turning the loop handles off clears a loop that's set.
    if patch.touches_dsp() {
        player.set_dsp(s.dsp()).await?;
    }
    if patch.ab_loop_handles == Some(false) {
        player.set_ab_loop(None).await?;
    }
    if patch.queue_end.is_some() {
        player.set_autoplay(s.queue_end == QueueEnd::KeepPlaying).await?;
    }
    // Turning the setting off ends smart shuffle (plain shuffle stays).
    if patch.smart_shuffle == Some(false) && player.state().await.is_ok_and(|p| p.smart_shuffle) {
        player.set_smart_shuffle(false).await?;
        player.set_shuffle(true).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn settings_get(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
) -> AppResult<Settings> {
    let mut s = settings.get().await?;
    // The player owns the live volume.
    if let Ok(p) = player.state().await {
        s.volume = p.volume;
    }
    Ok(s)
}

#[tauri::command]
pub async fn settings_set(
    settings: State<'_, SettingsState>,
    player: State<'_, PlayerHandle>,
    downloads: State<'_, downloads::Downloads>,
    mixes: State<'_, crate::mixes::MixService>,
    app: AppHandle,
    patch: SettingsPatch,
) -> AppResult<()> {
    let mini = patch.touches_miniplayer();
    apply_patch(&settings, &player, &downloads, &mixes, patch).await?;
    if mini {
        // The miniplayer has its own window; tell both, and keep it on top or not.
        let s = settings.get().await?;
        crate::platform::miniplayer::settings_changed(&app, &s);
    }
    Ok(())
}

/// A settings change and everything that follows it (Settings page, or
/// synced settings arriving from another device).
pub async fn apply_patch(
    settings: &SettingsState,
    player: &PlayerHandle,
    downloads: &downloads::Downloads,
    mixes: &crate::mixes::MixService,
    patch: SettingsPatch,
) -> AppResult<()> {
    let s = settings.set(patch.clone()).await?;
    // Download Liked Songs is the Liked Songs download itself.
    if let Some(on) = patch.download_liked {
        let key = downloads::ItemKey::new(downloads::ItemKind::Liked, "");
        if on {
            downloads.add(key, "Liked Songs".into(), None).await?;
        } else {
            downloads.remove(key).await?;
        }
    }
    if patch.touches_mixes() {
        // Made again in the background; Home hears `mixes://changed`.
        let m = mixes.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = m.ensure_fresh(crate::mixes::service::now_ms()).await {
                tracing::warn!("mixes not made: {e}");
            }
        });
    }
    apply_to_player(player, &s, &patch).await
}

/// Art cache usage for the Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtCacheInfo {
    pub used_bytes: u64,
    pub cap_bytes: u64,
}

/// Delete every cached image. Returns the bytes freed.
pub fn clear_art(art: &ArtCache) -> AppResult<u64> {
    let before = art.size();
    art.clear()?;
    Ok(before.saturating_sub(art.size()))
}

#[tauri::command]
pub async fn art_cache_info(art: State<'_, ArtCache>) -> AppResult<ArtCacheInfo> {
    Ok(ArtCacheInfo { used_bytes: art.size(), cap_bytes: art.cap() })
}

#[tauri::command]
pub async fn art_cache_clear(art: State<'_, ArtCache>) -> AppResult<u64> {
    let art = art.inner().clone();
    tokio::task::spawn_blocking(move || clear_art(&art))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
}

/// Version and licence for the App and About sections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub licence: String,
    pub repo_url: String,
}

#[tauri::command]
pub async fn app_info(app: AppHandle) -> AppResult<AppInfo> {
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        licence: LICENCE.into(),
        repo_url: REPO_URL.into(),
    })
}

/// Open the project page in the default browser.
#[tauri::command]
pub async fn open_project_page(app: AppHandle) -> AppResult<()> {
    app.opener()
        .open_url(REPO_URL, None::<&str>)
        .map_err(|e| AppError::Internal(format!("open browser: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::source::ArtBytes;
    use crate::db::ArtFetcher;

    fn db() -> (DbHandle, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (DbHandle::open(dir.path().join("s.db")).unwrap(), dir)
    }

    struct NoArt;
    #[async_trait::async_trait]
    impl ArtFetcher for NoArt {
        async fn fetch(&self, _: &str, _: u32, _: u32) -> AppResult<ArtBytes> {
            Err(AppError::NotFound("art".into()))
        }
    }

    fn art(dir: &std::path::Path) -> ArtCache {
        ArtCache::open(dir.join("art"), 2048 * MB, Arc::new(NoArt)).unwrap()
    }

    fn image(n: usize) -> ArtBytes {
        ArtBytes { bytes: vec![7u8; n].into(), content_type: "image/jpeg".into() }
    }

    #[test]
    fn defaults_when_nothing_stored() {
        let (db, _d) = db();
        let s = load(&db);
        assert_eq!(s, Settings::default());
        assert!(s.report_to_plex);
        assert!(!s.private_session);
        assert_eq!(s.quality_home, StreamQuality::Original);
        assert_eq!(s.quality_away, StreamQuality::Kbps320);
        assert!(s.show_format_badge && s.resume_contexts && s.artwork_colours);
        assert_eq!(s.precache_count, 5);
        assert_eq!(s.popular_source, PopularSource::Global);
        assert_eq!(s.art_cache_mb, 2048);
        assert_eq!(s.reduce_motion, ReduceMotion::System);
        assert!(s.search_suggestions && s.search_recents);
        assert!(s.media_keys && s.media_session && !s.close_to_tray && !s.launch_at_login);
        assert!(s.ab_loop_handles && s.up_next_line && s.volume_percent);
        assert!(!s.notify_track_change);
        assert!(s.info_wikipedia && !s.info_lastfm && !s.info_fanart && !s.info_audiodb);
        assert!(s.info_listenbrainz && s.link_featured);
        assert!(s.playlist_ask_duplicates);
        assert_eq!(s.playlist_new_name, "My playlist #N");
        assert_eq!((s.liked_means, s.like_control_rows), (LikedMeans::Five, LikeControlRows::Hover));
        assert_eq!(s.liked_min_rating(), 10.0);
        assert!(s.share_public_links);
        assert!(s.discord_status && s.discord_show_unreleased && s.discord_unreleased_art && s.discord_clear_on_pause);
        assert!(s.discord_find_song && s.discord_cover_archive && !s.discord_upload_art);
        assert_eq!(s.discord_status_line, StatusLine::App);
        assert!(!s.discord_upload_catbox);
        assert!(s.show_plex_avatar);
        assert!(s.read_file_tags && s.group_editions && !s.show_fan_edits);
        assert!(s.drag_and_drop);
        // Transcription is opt-in (it downloads a model); its results show.
        assert!(!s.transcribe_lyrics && s.show_auto_lyrics);
        assert_eq!(s.transcribe_model, crate::lyrics::transcribe::model::ModelKind::Small);
        assert_eq!(s.explicit_content, ExplicitContent::Allowed);
        assert!(s.home_mixes && s.mix_unreleased);
        assert_eq!(s.mix_variety, crate::mixes::Variety::Balanced);
        assert!(s.mix_exclude_artists.is_empty() && s.mix_exclude_genres.is_empty() && s.mix_exclude_albums.is_empty());
        assert_eq!(s.daylist_hours, 3);
        assert_eq!(s.queue_end, QueueEnd::KeepPlaying);
        assert!(!s.smart_shuffle, "smart shuffle is opt-in");
        assert!(!s.sync_enabled && s.sync_folder.is_empty(), "the sync file is opt-in");
        assert!(s.sync_settings && s.sync_library && s.sync_follows && s.sync_lyrics);
        assert!(!s.power_keys, "keyboard power mode is opt-in");
        assert!(!s.now_playing_feed, "the widget feed is opt-in");
        assert_eq!(s.now_playing_feed_port, 32540);
        assert_eq!(s.hint_key, HintKey::F);
    }

    #[test]
    fn the_feed_follows_its_settings_and_the_private_session() {
        let live = Live::default();
        let (db, _d) = db();
        let mut s = Settings::default();
        publish(&s, &live, &db);
        assert!(!live.feed.get().enabled);
        s.apply(&SettingsPatch { now_playing_feed: Some(true), now_playing_feed_port: Some(40000), private_session: Some(true), ..Default::default() })
            .unwrap();
        publish(&s, &live, &db);
        assert_eq!(live.feed.get(), crate::feed::FeedConfig { enabled: true, port: 40000, private_session: true });
        // Low ports are refused and leave the port alone.
        assert!(s.apply(&SettingsPatch { now_playing_feed_port: Some(80), ..Default::default() }).is_err());
        assert_eq!(s.now_playing_feed_port, 40000);
        // The feed stays on this computer.
        assert!(DEVICE_SETTINGS.contains(&"nowPlayingFeed") && DEVICE_SETTINGS.contains(&"nowPlayingFeedPort"));
    }

    #[test]
    fn hint_keys_are_the_characters_on_the_wire() {
        for (k, wire) in [(HintKey::F, "f"), (HintKey::H, "h"), (HintKey::Semicolon, ";"), (HintKey::Quote, "'")] {
            assert_eq!(serde_json::to_value(k).unwrap(), serde_json::json!(wire));
        }
        let p: SettingsPatch = serde_json::from_value(serde_json::json!({ "powerKeys": true, "hintKey": ";" })).unwrap();
        let mut s = Settings::default();
        s.apply(&p).unwrap();
        assert!(s.power_keys);
        assert_eq!(s.hint_key, HintKey::Semicolon);
        assert!(serde_json::from_value::<SettingsPatch>(serde_json::json!({ "hintKey": "x" })).is_err());
    }

    #[test]
    fn every_setting_is_synced_or_this_device_only() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        let names: Vec<&String> = v.as_object().unwrap().keys().collect();
        for n in &names {
            let synced = SYNCED_SETTINGS.contains(&n.as_str());
            let device = DEVICE_SETTINGS.contains(&n.as_str());
            assert!(synced ^ device, "{n} must be in exactly one scope list");
        }
        for n in SYNCED_SETTINGS.iter().chain(DEVICE_SETTINGS) {
            assert!(names.iter().any(|x| x == n), "{n} isn't a setting");
        }
        // Every synced setting can be applied as a patch (that's how it arrives).
        for n in SYNCED_SETTINGS {
            let mut one = serde_json::Map::new();
            one.insert(n.to_string(), v[*n].clone());
            let p: SettingsPatch = serde_json::from_value(serde_json::Value::Object(one)).unwrap();
            assert_ne!(p, SettingsPatch::default(), "{n} isn't in SettingsPatch");
        }
        // Folders, the output and the sync settings themselves stay here.
        for n in ["syncFolder", "syncEnabled", "downloadFolder", "computerName", "outputDevice", "eqProfiles"] {
            assert!(DEVICE_SETTINGS.contains(&n));
        }
    }

    #[test]
    fn sync_config_follows_the_settings() {
        let mut s = Settings::default();
        assert!(!s.sync_config().enabled);
        s.sync_enabled = true;
        assert!(!s.sync_config().enabled, "no folder yet");
        s.sync_folder = "  /tmp/cloud ".into();
        let c = s.sync_config();
        assert!(c.enabled);
        assert_eq!(c.folder, Some(std::path::PathBuf::from("/tmp/cloud")));
        s.sync_lyrics = false;
        assert!(!s.sync_config().groups.lyrics);
    }

    #[test]
    fn mix_settings_apply_and_reach_the_mix_engine() {
        let (db, _d) = db();
        let live = Live::default();
        let mixes = crate::mixes::MixService::new(db.clone());
        *live.mixes.lock() = Some(mixes.clone());
        let mut s = Settings::default();
        s.apply(&SettingsPatch {
            mix_variety: Some(crate::mixes::Variety::Adventurous),
            mix_exclude_genres: Some(vec![" Rock ".into(), "rock".into(), "".into(), "Jazz".into()]),
            mix_exclude_artists: Some(vec!["12".into()]),
            mix_unreleased: Some(false),
            daylist_hours: Some(6),
            queue_end: Some(QueueEnd::Stop),
            explicit_content: Some(ExplicitContent::Filtered),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(s.mix_exclude_genres, vec!["Rock", "Jazz"], "trimmed, no repeats in any case");
        publish(&s, &live, &db);
        let o = mixes.options();
        assert_eq!(o.variety, crate::mixes::Variety::Adventurous);
        assert_eq!(o.exclude_artists, vec!["12"]);
        assert!(!o.include_unreleased && o.tag_view.filter_explicit);
        assert_eq!(o.daylist_hours, 6);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["queueEnd"], "stop");
        assert_eq!(json["mixVariety"], "adventurous");
        assert!(s.apply(&SettingsPatch { daylist_hours: Some(2), ..Default::default() }).is_err());
        assert!(SettingsPatch { mix_unreleased: Some(true), ..Default::default() }.touches_mixes());
        assert!(!SettingsPatch { home_mixes: Some(false), ..Default::default() }.touches_mixes());
        // Friend activity, sharing, What's New, guests adding and the Discord
        // session are on; new release notifications are off.
        assert!(s.show_friend_activity && s.share_activity && s.whats_new_announcements);
        assert!(!s.notify_new_releases);
        assert!(s.listen_guests_control && s.discord_listen_along);
        // Listen along: ntfy.sh, invites good for a day, the Plex name.
        assert!(s.listen_relay.is_empty() && s.listen_name.is_empty());
        assert_eq!(s.listen_invite_hours, 24);
        for bad in ["ftp://relay.example.org", "relay.example.org", "https://user:pw@relay.example.org", "https://relay.example.org/?x=1"] {
            assert!(s.apply(&SettingsPatch { listen_relay: Some(bad.into()), ..Default::default() }).is_err(), "{bad}");
        }
        assert!(s.apply(&SettingsPatch { listen_invite_hours: Some(5), ..Default::default() }).is_err());
        assert!(s.apply(&SettingsPatch { listen_name: Some("x".repeat(41)), ..Default::default() }).is_err());
        s.apply(&SettingsPatch { listen_relay: Some(" ".into()), ..Default::default() }).unwrap();
        assert!(s.listen_relay.is_empty(), "blank goes back to the default relay");
    }

    #[test]
    fn file_tag_settings_reach_the_library_view() {
        let (db, _d) = db();
        let live = Live::default();
        let mut s = Settings::default();
        publish(&s, &live, &db);
        assert_eq!(db.tag_view(), crate::tags::TagView::default());
        s.apply(&SettingsPatch {
            explicit_content: Some(ExplicitContent::Filtered),
            show_fan_edits: Some(true),
            group_editions: Some(false),
            ..Default::default()
        })
        .unwrap();
        publish(&s, &live, &db);
        let v = db.tag_view();
        assert!(v.filter_explicit && v.show_edits && !v.group_editions);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["explicitContent"], "filtered");
        assert_eq!(json["readFileTags"], true);
    }

    #[test]
    fn discord_settings_publish_with_the_private_session() {
        let (db, _d) = db();
        let live = Live::default();
        let mut s = Settings::default();
        s.apply(&SettingsPatch {
            discord_status: Some(false),
            discord_clear_on_pause: Some(false),
            discord_upload_art: Some(true),
            discord_status_line: Some(StatusLine::Artist),
            discord_upload_catbox: Some(true),
            private_session: Some(true),
            ..Default::default()
        })
        .unwrap();
        publish(&s, &live, &db);
        let d = live.discord.get();
        assert!(!d.enabled && !d.clear_on_pause && d.upload_art && d.private_session);
        assert_eq!(d.status_line, StatusLine::Artist);
        assert!(d.upload_catbox);
        assert_eq!(serde_json::to_value(&s).unwrap()["discordStatusLine"], "artist");
        assert!(d.show_unreleased && d.unreleased_art && d.find_song && d.cover_archive);
    }

    #[test]
    fn settings_saved_by_an_older_build_load_with_new_defaults() {
        let (db, _d) = db();
        db.write(|c| {
            kv_set(c, KEY, r#"{"privateSession":true,"reportToPlex":false,"volume":0.3,"shuffleStyle":"standard"}"#)
        })
        .unwrap();
        let s = load(&db);
        assert!(s.private_session);
        assert!(!s.report_to_plex);
        assert_eq!(s.shuffle_style, ShuffleStyle::Standard);
        let d = Settings::default();
        assert_eq!(
            (s.quality_home, s.quality_away, s.show_format_badge, s.resume_contexts),
            (d.quality_home, d.quality_away, d.show_format_badge, d.resume_contexts)
        );
        assert_eq!((s.precache_count, s.art_cache_mb), (5, 2048));
        assert_eq!((s.popular_source, s.artwork_colours, s.reduce_motion), (PopularSource::Global, true, ReduceMotion::System));
    }

    #[test]
    fn new_feature_settings_default_and_check_the_model_name() {
        let mut s = Settings::default();
        assert!(s.bookmarks_in_library);
        assert!(!s.scrobble_lastfm && !s.scrobble_listenbrainz && !s.scrobble_unreleased);
        assert_eq!(s.ai_provider, crate::smart::prompt::AiProvider::None);
        assert_eq!(s.ai_model, crate::smart::prompt::DEFAULT_MODEL);
        let bad = SettingsPatch { ai_model: Some("rm -rf /".into()), ..Default::default() };
        assert!(s.apply(&bad).is_err());
        s.apply(&SettingsPatch { ai_model: Some("  ".into()), ..Default::default() }).unwrap();
        assert_eq!(s.ai_model, crate::smart::prompt::DEFAULT_MODEL, "empty goes back to the default");
        // Older stored settings without the new fields read as the defaults.
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert!(old.bookmarks_in_library && old.ai_provider == crate::smart::prompt::AiProvider::None);
    }

    #[tokio::test]
    async fn set_persists_across_reopen_and_publishes() {
        let (db, dir) = db();
        let live = Live::default();
        let st = SettingsState::new(db.clone(), live.clone());
        st.set(SettingsPatch {
            private_session: Some(true),
            report_to_plex: Some(false),
            volume: Some(0.25),
            shuffle_style: Some(ShuffleStyle::Standard),
            quality_home: Some(StreamQuality::Kbps256),
            quality_away: Some(StreamQuality::Original),
            show_format_badge: Some(false),
            resume_contexts: Some(false),
            precache_count: Some(12),
            popular_source: Some(PopularSource::MyPlays),
            art_cache_mb: Some(512),
            artwork_colours: Some(false),
            reduce_motion: Some(ReduceMotion::On),
            home_shortcuts: Some(false),
            home_colour_wash: Some(false),
            search_suggestions: Some(false),
            search_recents: Some(false),
            media_keys: Some(false),
            media_session: Some(false),
            close_to_tray: Some(true),
            launch_at_login: Some(true),
            ab_loop_handles: Some(false),
            up_next_line: Some(false),
            volume_percent: Some(false),
            notify_track_change: Some(true),
            playlist_ask_duplicates: Some(false),
            playlist_new_name: Some("  Mix #N ".into()),
            info_wikipedia: Some(false),
            info_lastfm: Some(true),
            info_fanart: Some(true),
            info_audiodb: Some(true),
            info_listenbrainz: Some(false),
            link_featured: Some(false),
            liked_means: Some(LikedMeans::FourUp),
            like_control_rows: Some(LikeControlRows::Always),
            show_plex_players: Some(false),
            act_as_plex_player: Some(true),
            move_stops_old_device: Some(false),
            crossfade: Some(CrossfadeMode::Fixed),
            crossfade_seconds: Some(9),
            crossfade_skip_albums: Some(false),
            leveling: Some(Leveling::Album),
            preamp_db: Some(-3.5),
            limiter: Some(false),
            eq_enabled: Some(true),
            eq_bands: Some([1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, -2.0, -1.0]),
            eq_preset: None,
            share_public_links: Some(false),
            show_cast_devices: Some(false),
            discord_status: Some(false),
            discord_show_unreleased: Some(false),
            discord_unreleased_art: Some(false),
            discord_clear_on_pause: Some(false),
            discord_find_song: Some(false),
            discord_cover_archive: Some(false),
            discord_upload_art: Some(true),
            discord_upload_catbox: Some(true),
            discord_status_line: Some(StatusLine::Song),
            show_plex_avatar: Some(false),
            lyrics_mode: Some(LyricsMode::Off),
            lyrics_lrclib: Some(true),
            lyrics_prefer_own: Some(true),
            lyrics_show_add: Some(false),
            full_window_hide_controls: Some(false),
            read_file_tags: Some(false),
            explicit_content: Some(ExplicitContent::Filtered),
            show_fan_edits: Some(true),
            group_editions: Some(false),
            drag_and_drop: Some(false),
            transcribe_lyrics: Some(true),
            transcribe_model: Some(crate::lyrics::transcribe::model::ModelKind::Medium),
            show_auto_lyrics: Some(false),
            show_to_my_devices: Some(false),
            computer_name: Some("Den".into()),
            show_airplay_locked: Some(true),
            download_quality: Some(DownloadQuality::Kbps192),
            download_limit_gb: Some(50),
            download_home_only: Some(false),
            download_liked: Some(true),
            go_offline: Some(false),
            home_mixes: Some(false),
            mix_variety: Some(crate::mixes::Variety::Familiar),
            mix_exclude_artists: Some(vec!["7".into()]),
            mix_exclude_genres: None,
            mix_exclude_albums: Some(vec![]),
            mix_unreleased: Some(false),
            daylist_hours: Some(1),
            queue_end: Some(QueueEnd::Stop),
            smart_shuffle: Some(true),
            show_friend_activity: Some(false),
            share_activity: Some(false),
            whats_new_announcements: Some(false),
            notify_new_releases: Some(true),
            listen_guests_control: Some(false),
            listen_relay: Some(" https://relay.example.org/ ".into()),
            listen_invite_hours: Some(1),
            listen_name: Some(" DJ Sam ".into()),
            discord_listen_along: Some(false),
            bookmarks_in_library: Some(false),
            scrobble_lastfm: Some(true),
            scrobble_listenbrainz: Some(true),
            scrobble_unreleased: Some(true),
            ai_provider: Some(crate::smart::prompt::AiProvider::Anthropic),
            ai_model: Some(" claude-sonnet-5-5 ".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        // Scrobbling sees its switches, and the private session.
        assert!(live.scrobble.lastfm.load(Ordering::Relaxed) && live.scrobble.listenbrainz.load(Ordering::Relaxed));
        assert!(live.scrobble.unreleased.load(Ordering::Relaxed) && live.scrobble.private_session.load(Ordering::Relaxed));
        // Friend activity, What's New and listening sessions see their switches.
        assert!(!live.social.show_activity.load(Ordering::Relaxed));
        assert!(!live.social.whats_new.load(Ordering::Relaxed));
        assert!(live.social.notify_releases.load(Ordering::Relaxed));
        assert!(!live.social.listen.guests_control.load(Ordering::Relaxed));
        assert_eq!(*live.social.listen.relay.read(), "https://relay.example.org");
        assert_eq!(live.social.listen.invite_hours.load(Ordering::Relaxed), 1);
        assert_eq!(*live.social.listen.name.read(), "DJ Sam");
        assert!(live.social.private_session.load(Ordering::Relaxed));
        assert!(!live.tracking.share_activity.load(Ordering::Relaxed));
        assert!(!live.discord.get().listen_along);
        st.set(SettingsPatch {
            sync_enabled: Some(true),
            sync_settings: Some(false),
            sync_library: Some(true),
            sync_follows: Some(false),
            sync_lyrics: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        // On with no folder chosen counts as off.
        assert!(!live.sync.get().enabled);
        assert!(!live.sync.get().groups.settings && live.sync.get().groups.library && !live.sync.get().groups.follows);
        assert!(!live.lyrics_show_auto.load(Ordering::Relaxed));
        assert!(!live.share_public_links.load(Ordering::Relaxed));
        assert!(!live.remote.show_cast());
        assert!(!live.discord.get().enabled && live.discord.get().upload_art && live.discord.get().private_session);
        {
            use crate::artistinfo::InfoSource as Src;
            // The artist info service sees the switches at once.
            assert!(!live.artist_info.get(Src::Wikipedia));
            assert!(live.artist_info.get(Src::Lastfm) && live.artist_info.get(Src::Fanart) && live.artist_info.get(Src::Audiodb));
            assert!(!live.artist_info.get(Src::Listenbrainz));
        }
        assert!(live.tracking.private_session.load(Ordering::Relaxed));
        assert!(!live.tracking.report_to_plex.load(Ordering::Relaxed));
        assert_eq!((live.streams.home(), live.streams.away()), (Some(256), None));
        assert_eq!(db.popular_source(), PopularSource::MyPlays);
        assert!(!db.link_featured());
        assert_eq!(live.art_cap.load(Ordering::Relaxed), 512 * MB);
        // Media controls, tray and launch at login get the change at once.
        assert_eq!(live.platform.get(), PlatformSettings { media_keys: false, media_session: false, close_to_tray: true, launch_at_login: true });

        // Partial patch leaves other fields alone.
        st.set(SettingsPatch { report_to_plex: Some(true), ..Default::default() }).await.unwrap();
        drop(st);
        drop(db);

        let db2 = DbHandle::open(dir.path().join("s.db")).unwrap();
        let fresh = Live::default();
        let st2 = SettingsState::new(db2.clone(), fresh.clone());
        let s = st2.get().await.unwrap();
        assert!(s.private_session);
        assert!(s.report_to_plex);
        assert!(s.transcribe_lyrics && !s.show_auto_lyrics);
        assert_eq!(s.transcribe_model, crate::lyrics::transcribe::model::ModelKind::Medium);
        assert!(!fresh.lyrics_show_auto.load(Ordering::Relaxed));
        assert!(!s.info_wikipedia && s.info_lastfm && s.info_fanart && s.info_audiodb);
        assert!(!s.info_listenbrainz && !s.link_featured);
        assert!(fresh.artist_info.get(crate::artistinfo::InfoSource::Lastfm));
        assert!(!s.show_plex_players && s.act_as_plex_player && !s.move_stops_old_device);
        assert!(!s.share_public_links && !fresh.share_public_links.load(Ordering::Relaxed));
        assert!(!fresh.remote.show_players() && *fresh.remote.allow_casting.borrow());
        assert_eq!(s.volume, 0.25);
        assert!(!s.read_file_tags && s.show_fan_edits && !s.group_editions);
        assert!(!s.drag_and_drop);
        assert_eq!((s.download_quality, s.download_limit_gb), (DownloadQuality::Kbps192, 50));
        assert!(!s.download_home_only && s.download_liked && !s.go_offline);
        assert_eq!(s.explicit_content, ExplicitContent::Filtered);
        assert!(db2.tag_view().filter_explicit, "the library view follows at start");
        assert_eq!(s.shuffle_style, ShuffleStyle::Standard);
        assert_eq!((s.quality_home, s.quality_away), (StreamQuality::Kbps256, StreamQuality::Original));
        assert!(!s.show_format_badge && !s.resume_contexts && !s.artwork_colours);
        assert_eq!((s.precache_count, s.art_cache_mb), (12, 512));
        assert_eq!((s.popular_source, s.reduce_motion), (PopularSource::MyPlays, ReduceMotion::On));
        assert!(!s.search_suggestions && !s.search_recents);
        assert!(!s.show_plex_avatar);
        assert!(!s.media_keys && !s.media_session && s.close_to_tray && s.launch_at_login);
        assert_eq!(
            fresh.platform.get(),
            PlatformSettings { media_keys: false, media_session: false, close_to_tray: true, launch_at_login: true }
        );
        assert!(!s.ab_loop_handles && !s.up_next_line && !s.volume_percent && s.notify_track_change);
        assert!(!s.playlist_ask_duplicates);
        assert_eq!(s.playlist_new_name, "Mix #N");
        assert_eq!((s.crossfade, s.crossfade_seconds, s.crossfade_skip_albums), (CrossfadeMode::Fixed, 9, false));
        assert_eq!((s.leveling, s.preamp_db, s.limiter, s.eq_enabled), (Leveling::Album, -3.5, false, true));
        assert_eq!(s.eq_bands[2], 3.0);
        assert_eq!(s.eq_preset, "custom");
        assert_eq!((s.liked_means, s.like_control_rows), (LikedMeans::FourUp, LikeControlRows::Always));
        assert_eq!(s.liked_min_rating(), 8.0);
        // Loaded at startup into the live switches.
        assert!(fresh.notify_track_change.load(Ordering::Relaxed));
        assert!(fresh.tracking.private_session.load(Ordering::Relaxed));
        assert!(fresh.tracking.report_to_plex.load(Ordering::Relaxed));
        assert_eq!(fresh.streams.home(), Some(256));
        assert_eq!(db2.popular_source(), PopularSource::MyPlays);
    }

    #[test]
    fn ranges_are_clamped_and_volume_must_be_finite() {
        let mut s = Settings::default();
        s.apply(&SettingsPatch { volume: Some(3.0), ..Default::default() }).unwrap();
        assert_eq!(s.volume, 1.0);
        s.apply(&SettingsPatch { volume: Some(-1.0), ..Default::default() }).unwrap();
        assert_eq!(s.volume, 0.0);
        assert!(s.apply(&SettingsPatch { volume: Some(f32::NAN), ..Default::default() }).is_err());
        s.apply(&SettingsPatch { precache_count: Some(0), art_cache_mb: Some(1), ..Default::default() }).unwrap();
        assert_eq!((s.precache_count, s.art_cache_mb), (1, ART_CACHE_MIN_MB));
        s.apply(&SettingsPatch { precache_count: Some(500), ..Default::default() }).unwrap();
        assert_eq!(s.precache_count, 50);
    }

    #[test]
    fn unknown_and_missing_fields_fall_back_to_defaults() {
        let (db, _d) = db();
        db.write(|c| kv_set(c, KEY, r#"{"privateSession":true,"somethingNew":1,"precacheCount":999}"#)).unwrap();
        let s = load(&db);
        assert!(s.private_session);
        assert!(s.report_to_plex);
        assert_eq!(s.precache_count, PRECACHE_MAX);
        db.write(|c| kv_set(c, KEY, "not json")).unwrap();
        assert_eq!(load(&db), Settings::default());
    }

    #[test]
    fn wire_format_is_camel_case() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(v["privateSession"], false);
        assert_eq!(v["reportToPlex"], true);
        assert_eq!(v["shuffleStyle"], "spread");
        assert_eq!(v["qualityHome"], "original");
        assert_eq!(v["qualityAway"], "320");
        assert_eq!(v["showFormatBadge"], true);
        assert_eq!(v["showPlexAvatar"], true);
        assert_eq!(v["resumeContexts"], true);
        assert_eq!(v["precacheCount"], 5);
        assert_eq!(v["popularSource"], "global");
        assert_eq!(v["artCacheMb"], 2048);
        assert_eq!(v["artworkColours"], true);
        assert_eq!(v["reduceMotion"], "system");
        assert_eq!(v["searchSuggestions"], true);
        assert_eq!(v["searchRecents"], true);
        assert_eq!(v["mediaKeys"], true);
        assert_eq!(v["mediaSession"], true);
        assert_eq!(v["closeToTray"], false);
        assert_eq!(v["launchAtLogin"], false);
        assert_eq!(v["abLoopHandles"], true);
        assert_eq!(v["upNextLine"], true);
        assert_eq!(v["volumePercent"], true);
        assert_eq!(v["notifyTrackChange"], false);
        assert_eq!(v["infoWikipedia"], true);
        assert_eq!(v["infoLastfm"], false);
        assert_eq!(v["sharePublicLinks"], true);
        let p: SettingsPatch = serde_json::from_str(r#"{"shuffleStyle":"standard"}"#).unwrap();
        assert_eq!(p.shuffle_style, Some(ShuffleStyle::Standard));
        let p: SettingsPatch =
            serde_json::from_str(r#"{"qualityAway":"128","popularSource":"myPlays","reduceMotion":"off"}"#).unwrap();
        assert_eq!(p.quality_away, Some(StreamQuality::Kbps128));
        assert_eq!(p.popular_source, Some(PopularSource::MyPlays));
        assert_eq!(p.reduce_motion, Some(ReduceMotion::Off));
        assert_eq!(p.report_to_plex, None);
    }

    #[tokio::test]
    async fn output_device_is_stored_per_device_and_survives_other_changes() {
        let (db, dir) = db();
        let st = SettingsState::new(db.clone(), Live::default());
        assert_eq!(st.get().await.unwrap().output_device, None);
        st.set_output_device(Some("wasapi:usb".into())).await.unwrap();
        // A normal patch keeps it; a patch can't set it.
        let p: SettingsPatch = serde_json::from_str(r#"{"reportToPlex":false,"outputDevice":"x"}"#).unwrap();
        st.set(p).await.unwrap();
        drop(st);
        drop(db);
        let db2 = DbHandle::open(dir.path().join("s.db")).unwrap();
        let s = load(&db2);
        assert_eq!(s.output_device.as_deref(), Some("wasapi:usb"));
        assert!(!s.report_to_plex);
        assert!(serde_json::to_value(Settings::default()).unwrap()["outputDevice"].is_null());
        let st2 = SettingsState::new(db2, Live::default());
        st2.set_output_device(None).await.unwrap();
        assert_eq!(st2.get().await.unwrap().output_device, None);
    }

    #[test]
    fn quality_maps_to_kbps() {
        assert_eq!(StreamQuality::Original.kbps(), None);
        assert_eq!(StreamQuality::Kbps192.kbps(), Some(192));
    }

    #[tokio::test]
    async fn art_cache_size_setting_resizes_the_cache() {
        let (db, dir) = db();
        let cache = art(dir.path());
        cache.store("/a", 64, 64, &image(400_000)).unwrap();
        cache.store("/b", 64, 64, &image(400_000)).unwrap();
        let live = Live { art: Some(cache.clone()), ..Default::default() };
        let st = SettingsState::new(db, live.clone());
        assert_eq!(cache.cap(), 2048 * MB, "default cap published at startup");
        st.set(SettingsPatch { art_cache_mb: Some(ART_CACHE_MIN_MB), ..Default::default() }).await.unwrap();
        assert_eq!(cache.cap(), ART_CACHE_MIN_MB as u64 * MB);
        assert_eq!(live.art_cap.load(Ordering::Relaxed), ART_CACHE_MIN_MB as u64 * MB);
    }

    #[test]
    fn clear_cache_removes_every_image_and_reports_the_space() {
        let dir = tempfile::tempdir().unwrap();
        let cache = art(dir.path());
        cache.store("/a", 64, 64, &image(1000)).unwrap();
        cache.store("/b", 64, 64, &image(3000)).unwrap();
        assert_eq!(cache.size(), 4000);
        assert_eq!(clear_art(&cache).unwrap(), 4000);
        assert_eq!(cache.size(), 0);
        assert!(cache.lookup("/a", 64, 64).unwrap().is_none());
        assert_eq!(clear_art(&cache).unwrap(), 0);
    }

    #[test]
    fn listen_on_settings_default_and_reach_the_switches() {
        let s = Settings::default();
        assert!(s.show_plex_players && !s.act_as_plex_player && s.move_stops_old_device);
        let (db, _d) = db();
        let live = Live::default();
        let mut rx = live.remote.allow_casting.subscribe();
        let mut s = load(&db);
        s.apply(&SettingsPatch {
            show_plex_players: Some(false),
            act_as_plex_player: Some(true),
            move_stops_old_device: Some(false),
            ..Default::default()
        })
        .unwrap();
        publish(&s, &live, &db);
        assert!(!live.remote.show_players() && !live.remote.stop_old());
        assert!(rx.has_changed().unwrap());
        assert!(*rx.borrow_and_update());
    }

    #[test]
    fn platter_device_settings_default_and_reach_the_switches() {
        let s = Settings::default();
        assert!(s.show_to_my_devices && s.computer_name.is_empty() && !s.show_airplay_locked);
        let (db, _d) = db();
        let live = Live::default();
        publish(&s, &live, &db);
        assert!(*live.remote.announce.borrow());
        let mut rx = live.remote.announce.subscribe();
        let mut s = load(&db);
        s.apply(&SettingsPatch {
            show_to_my_devices: Some(false),
            computer_name: Some("  Studio  ".into()),
            show_airplay_locked: Some(true),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(s.computer_name, "Studio");
        publish(&s, &live, &db);
        assert!(rx.has_changed().unwrap());
        assert!(!*rx.borrow_and_update());
        assert_eq!(live.remote.computer_name(), "Studio");
        assert!(live.remote.show_airplay_locked());
        let long = "x".repeat(COMPUTER_NAME_MAX + 1);
        assert!(s.apply(&SettingsPatch { computer_name: Some(long), ..Default::default() }).is_err());
        // Empty goes back to the OS name.
        s.apply(&SettingsPatch { computer_name: Some(" ".into()), ..Default::default() }).unwrap();
        publish(&s, &live, &db);
        assert_eq!(live.remote.computer_name(), crate::remote::peer::os_computer_name());
    }

    #[tokio::test]
    async fn global_popular_with_no_source_on_ranks_by_plays() {
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("s.db")).unwrap();
        let st = SettingsState::new(db.clone(), Live::default());
        st.set(SettingsPatch { info_listenbrainz: Some(false), ..Default::default() }).await.unwrap();
        assert_eq!(db.popular_source(), PopularSource::MyPlays);
        st.set(SettingsPatch { info_lastfm: Some(true), ..Default::default() }).await.unwrap();
        assert_eq!(db.popular_source(), PopularSource::Global);
        st.set(SettingsPatch { link_featured: Some(false), ..Default::default() }).await.unwrap();
        assert!(!db.link_featured());
    }

    #[test]
    fn lyrics_settings_default_and_reach_the_switch() {
        let s = Settings::default();
        assert_eq!(s.lyrics_mode, LyricsMode::AutoShow);
        assert!(!s.lyrics_lrclib, "LRCLIB is off until the user turns it on");
        assert!(s.full_window_hide_controls);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["lyricsMode"], "autoShow");
        let (db, _d) = db();
        let live = Live::default();
        let mut s = load(&db);
        s.apply(&serde_json::from_str(r#"{"lyricsMode":"off","lyricsLrclib":true,"fullWindowHideControls":false}"#).unwrap())
            .unwrap();
        publish(&s, &live, &db);
        assert!(live.lyrics_lrclib.load(Ordering::Relaxed));
        assert_eq!(s.lyrics_mode, LyricsMode::Off);
        // Own lyrics: Plex's first, and "Add lyrics" offered, until changed.
        assert!(!s.lyrics_prefer_own && s.lyrics_show_add);
        assert!(!live.lyrics_prefer_own.load(Ordering::Relaxed));
        s.apply(&serde_json::from_str(r#"{"lyricsPreferOwn":true,"lyricsShowAdd":false}"#).unwrap()).unwrap();
        publish(&s, &live, &db);
        assert!(live.lyrics_prefer_own.load(Ordering::Relaxed));
        assert!(!s.lyrics_show_add);
        assert!(!s.full_window_hide_controls);
    }
}
