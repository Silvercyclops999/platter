/**
 * What the Settings page needs to know about its rows beyond their text:
 * which sit under a section's collapsed Advanced row, where "Learn more"
 * goes, and which settings Reset this section puts back.
 */
import type { Settings } from "../lib/types";

/**
 * Rows most people never need, folded under each section's Advanced row.
 * Settings search still finds them (and opens Advanced to show them).
 * The rule: everyday choices stay in view; tuning, numbers, technical
 * names, and follow-up options of something else stay in Advanced.
 */
export const ADVANCED: ReadonlySet<string> = new Set([
  // Account: only matter with a Plex Home or a picture to hide.
  "showPlexAvatar",
  "homeRemember",
  // Audio quality: a display detail.
  "showFormatBadge",
  // Playback: tuning and devices most homes don't have.
  "precacheCount",
  "abLoopHandles",
  "moveStopsOldDevice",
  "showDlnaDevices",
  "showAirplayLocked",
  "computerName",
  // Transitions and Loudness: fine tuning of an everyday choice.
  "crossfadeSkipAlbums",
  "preampDb",
  "limiter",
  // Library: how lists and rows behave, beyond the defaults.
  "subsonicCheckMinutes",
  "popularSource",
  "linkFeatured",
  "playlistAskDuplicates",
  "playlistNewName",
  "likedMeans",
  "bookmarksInLibrary",
  "pinsInLibrary",
  "likeControlRows",
  "statsStrip",
  "monthlyStatsOnProfile",
  "shelfMissingList",
  "dragAndDrop",
  "showDescribePlaylist",
  // Library: tag reading and the archive rules, for people who tag by hand.
  "rescanAfterTagEdit",
  "askBeforeManyFiles",
  "plexLibrarySettings",
  "readFileTags",
  "groupEditions",
  "countUnmatchedUnreleased",
  "checkUnmatchedMusicbrainz",
  "archiveFolderKeywords",
  "releaseStatusTag",
  "releaseStatusValues",
  "compareLevelMatch",
  "doctorChecks",
  "doctorCoverPx",
  "acoustidKey",
  "soundIdScope",
  // Downloads: the home-only rule is a safe default nobody needs to see.
  "downloadHomeOnly",
  // Sync: what goes in the file, and moving keys between devices.
  "syncSettings",
  "syncLibrary",
  "syncFollows",
  "syncLyrics",
  "keysReceive",
  "keysSend",
  // Search: typo forgiveness is on and rarely worth changing.
  "searchTypos",
  // Appearance: look building, small display details and lyrics fine tuning.
  "showLookBuilder",
  "themeFile",
  "sidebarArt",
  "upNextLine",
  "volumePercent",
  "lyricsLiveTiming",
  "lyricsFixTiming",
  "lyricsReleasedForVersions",
  "lyricsChoices",
  "translationService",
  "translateTo",
  "fullWindowHideControls",
  "fullWindowRememberMode",
  "miniplayerTint",
  "miniplayerQueue",
  "transcribeModel",
  "transcribePauseBusy",
  "transcribeFasterAway",
  "transcribeUnreleased",
  "syncTranscriptions",
  "shareTranscriptionsUnreleased",
  // Mixes: the daylist's timing.
  "daylistHours",
  // App: keyboard power use and system integration that's on already.
  "powerKeys",
  "hintKey",
  "notifyTrackChange",
  "mediaSession",
  "mediaKeys",
  // Sharing: what a share card carries.
  "shareCardQr",
  "shareCardSongLink",
  // Social: invite rules and notifications beyond the defaults.
  "listenInviteHours",
  "listenRelay",
  "listenHideUnreleased",
  "radarNotify",
  "notifyNewReleases",
  "dismissedArtists",
  // Integrations: tokens, caches, ports and the finer Discord and Lidarr rules.
  "infoListenbrainzToken",
  "infoCache",
  "nowPlayingFeedPort",
  "scrobbleUnreleased",
  "aiModel",
  "lidarrAwayUrl",
  "lidarrHeaders",
  "lidarrScanAfterImport",
  "lidarrFreeStaging",
  "upgradeOfficialPrefix",
  "autoRequestCap",
  "discordClearOnPause",
  "discordShowUnreleased",
  "discordUnreleasedArt",
  "discordFindSong",
  "discordCoverArchive",
  "discordUploadArt",
  "discordUploadCovers",
  "discordCoverHost",
  "discordStatusLine",
  "discordListenAlong",
]);

/**
 * Whether Advanced starts open. Off in the app (each visit starts tidy); the
 * test setup opens it so tests of a row needn't click Advanced first.
 */
export const advancedView = { startOpen: false };

/** A help page, and optionally a section of it. */
export type HelpRef = readonly [page: string, anchor?: string];

/** "Learn more" for a row, by its key. */
const BY_KEY: Record<string, HelpRef> = {
  account: ["getting-started", "signing-in-with-plex"],
  libraries: ["getting-started", "picking-libraries"],
  servers: ["connecting", "switching-servers"],
  qualityHome: ["connecting", "home-and-away-from-home"],
  qualityAway: ["connecting", "home-and-away-from-home"],
  downloadQuality: ["downloads-and-offline", "download-quality"],
  privateSession: ["privacy", "private-session-and-go-offline"],
  reportPlays: ["privacy", "private-session-and-go-offline"],
  goOffline: ["downloads-and-offline", "go-offline"],
  catalogueSearch: ["search", "music-that-isnt-in-your-library"],
  searchTypos: ["search", "forgiving-search"],
  look: ["getting-started", "picking-a-look"],
  lyricsMode: ["lyrics-and-transcription", "where-lyrics-come-from"],
  lyricsLrclib: ["lyrics-and-transcription", "lrclib"],
  lyricsCheckAudio: ["lyrics-and-transcription", "checking-lyrics-against-the-song"],
  lyricsFixTiming: ["lyrics-and-transcription", "timing-fixes"],
  lyricsChoices: ["lyrics-and-transcription", "wrong-lyrics"],
  showPronunciation: ["lyrics-and-transcription", "translation-and-pronunciation"],
  translationService: ["lyrics-and-transcription", "translation-and-pronunciation"],
  transcribeLyrics: ["lyrics-and-transcription", "transcription"],
  transcribeModel: ["lyrics-and-transcription", "the-model-download"],
  transcribeProcessor: ["lyrics-and-transcription", "processor-and-graphics-card"],
  shareTranscriptions: ["lyrics-and-transcription", "sharing-transcriptions"],
  sharePublicLinks: ["listening-together", "share-links"],
  shareActivity: ["listening-together", "friend-activity"],
  discordStatus: ["getting-started", "discord"],
  discordListenAlong: ["listening-together", "discord"],
  lidarrUrl: ["lidarr", "connecting-lidarr"],
  lidarrCheck: ["lidarr", "the-safety-check"],
  lidarrArtists: ["lidarr", "managed-and-protected-artists"],
  lidarrWork: ["lidarr", "practice-mode"],
  lidarrStagingFolder: ["lidarr", "the-staging-folder"],
  lidarrUpgrades: ["lidarr", "upgrades"],
  lidarrRequests: ["lidarr", "requests"],
  autoRequest: ["lidarr", "auto-requests"],
  musicFolder: ["tag-editor", "set-up-the-music-folder"],
  checkTags: ["tag-editor", "check-tags"],
  releaseStatusTag: ["archive-and-release-status", "release-status-tag"],
  archiveFolderKeywords: ["archive-and-release-status", "folder-keywords"],
  showLineage: ["archive-and-release-status", "versions-and-lineage"],
  showEraCards: ["archive-and-release-status", "eras"],
  countUnmatchedUnreleased: ["archive-and-release-status", "how-platter-decides"],
  statsPage: ["stats-and-wrapped", "opening-stats"],
  statsCompare: ["stats-and-wrapped", "compare-with-a-friend"],
  tautulliUrl: ["stats-and-wrapped", "plays-from-before-platter"],
  crossfade: ["playing-and-queue", "crossfade"],
  leveling: ["playing-and-queue", "loudness-leveling"],
  eqEnabled: ["playing-and-queue", "equalizer"],
  smartShuffle: ["playing-and-queue", "shuffle"],
  keyboardShortcuts: ["playing-and-queue", "keyboard-shortcuts"],
  miniplayerOnTop: ["playing-and-queue", "miniplayer"],
  downloadsStorage: ["downloads-and-offline", "storage-and-the-limit"],
  listenGuestsControl: ["listening-together", "what-guests-hear-and-do"],
  listenRelay: ["listening-together", "the-relay"],
  infoWikipedia: ["privacy", "off-until-you-turn-them-on"],
  nowPlayingFeed: ["privacy", "off-until-you-turn-them-on"],
};

/** Groups that get a "?" beside their heading (busier screens). */
export const GROUP_HELP: Readonly<Record<string, HelpRef>> = {
  "Lyrics transcription": ["lyrics-and-transcription", "transcription"],
  "Listening sessions": ["listening-together"],
  Lidarr: ["lidarr"],
  "Tag editor": ["tag-editor"],
  Archive: ["archive-and-release-status"],
};

/** Every row's Learn more target, for the tests. */
export const HELP_BY_KEY: Readonly<Record<string, HelpRef>> = BY_KEY;

export function helpFor(key: string): HelpRef | undefined {
  return BY_KEY[key];
}

/**
 * Rows whose control sets more than the setting named by its key (or a
 * setting under another name). Reset this section uses it.
 */
const ROW_SETTINGS: Record<string, (keyof Settings)[]> = {
  artCache: ["artCacheMb"],
  downloadLimit: ["downloadLimitGb"],
  crossfade: ["crossfade", "crossfadeSeconds"],
  eqEnabled: ["eqEnabled", "eqBands", "eqPreset"],
  mixExclude: ["mixExcludeArtists", "mixExcludeGenres", "mixExcludeAlbums"],
  lidarrWork: ["lidarrPracticeUntil"],
};

/**
 * Never reset: addresses, folders and devices you set up on purpose (and
 * keys, which aren't settings at all and stay in the keychain).
 */
export const NOT_RESET: ReadonlySet<string> = new Set([
  "musicFolder",
  "musicFolderServer",
  "downloadFolder",
  "syncFolder",
  "libraries",
  "serverView",
  "outputDevice",
  "lidarrUrl",
  "lidarrAwayUrl",
  "lidarrStagingFolder",
  "tautulliUrl",
  "translationService",
  "computerName",
  "volume",
]);

/** The settings a section's rows control, for Reset this section. */
export function settingsOfRows(rowKeys: string[], defaults: Settings): (keyof Settings)[] {
  const out = new Set<keyof Settings>();
  for (const k of rowKeys) {
    for (const s of ROW_SETTINGS[k] ?? (k in defaults ? [k as keyof Settings] : [])) {
      if (!NOT_RESET.has(s as string)) out.add(s);
    }
  }
  return [...out];
}

/** The patch that puts those settings back to how a new install has them. */
export function resetPatch(keys: (keyof Settings)[], defaults: Settings): Partial<Settings> {
  const p: Partial<Settings> = {};
  for (const k of keys) (p as Record<string, unknown>)[k] = defaults[k];
  return p;
}
