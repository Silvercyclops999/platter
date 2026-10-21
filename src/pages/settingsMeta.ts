/**
 * What the Settings page needs to know about its rows beyond their text:
 * which show by default and which sit under a section's More, where "Learn
 * more" goes, and which settings Reset this section puts back.
 */
import type { Settings } from "../lib/types";

/** The Settings page's groups, in the order the side nav lists them. */
export const GROUPS = [
  "Account and servers",
  "Playback",
  "Sound",
  "Library",
  "Downloads and devices",
  "Appearance",
  "Lyrics",
  "Integrations",
  "Sharing and friends",
  "App",
] as const;
export type GroupName = (typeof GROUPS)[number];

/** The id a group's section and nav link use. */
export const groupId = (g: string) => `settings-${g.toLowerCase().replace(/[^a-z]+/g, "-")}`;

/**
 * Settings with their own page: an integration (a card under Integrations)
 * or a tool moved out of a group (a Manage row stays behind).
 */
export type PageId =
  | "lidarr"
  | "discord"
  | "scrobbling"
  | "artistInfo"
  | "discogs"
  | "tautulli"
  | "ai"
  | "streamDeck"
  | "widgetFeed"
  | "bridge"
  | "libraryImport"
  | "tags"
  | "doctor"
  | "soundId"
  | "hidden"
  | "sync"
  | "devices"
  | "transcription"
  | "looks"
  | "guides";

export interface PageMeta {
  id: PageId;
  title: string;
  group: GroupName;
  /** One line on its card or Manage row. */
  blurb: string;
  /** Cards say Connected / Not set up; Manage rows don't. */
  kind: "integration" | "manage";
}

export const PAGES: readonly PageMeta[] = [
  { id: "lidarr", title: "Lidarr", group: "Integrations", kind: "integration", blurb: "Request albums, catch up on imports and swap in better copies." },
  { id: "discord", title: "Discord", group: "Integrations", kind: "integration", blurb: "Show what you're playing on your Discord profile." },
  { id: "scrobbling", title: "Last.fm and ListenBrainz", group: "Integrations", kind: "integration", blurb: "Send your plays to your profiles there." },
  { id: "artistInfo", title: "Artist info sources", group: "Integrations", kind: "integration", blurb: "Bios, photos, credits and listener numbers for artist pages." },
  { id: "discogs", title: "Discogs", group: "Integrations", kind: "integration", blurb: "Mark the records you own from your Discogs collection." },
  { id: "tautulli", title: "Tautulli", group: "Integrations", kind: "integration", blurb: "Count your plays in Tautulli in Stats and mixes." },
  { id: "ai", title: "AI", group: "Integrations", kind: "integration", blurb: "Turn a description into a prompted playlist." },
  { id: "streamDeck", title: "Stream Deck", group: "Integrations", kind: "integration", blurb: "Control Platter from Stream Deck keys and scripts." },
  { id: "widgetFeed", title: "Desktop widget feed", group: "Integrations", kind: "integration", blurb: "What's playing, for OBS, Rainmeter and other widgets." },
  { id: "bridge", title: "Bridge", group: "Integrations", kind: "integration", blurb: "A helper on your server: lyrics for every song at night, and Lidarr from anywhere." },
  { id: "libraryImport", title: "Library import", group: "Integrations", kind: "integration", blurb: "Bring in playlists, likes and history from other services." },
  { id: "tags", title: "Tags and archive", group: "Library", kind: "manage", blurb: "Music folder, tag checks, release status and archive rules." },
  { id: "doctor", title: "Library doctor", group: "Library", kind: "manage", blurb: "Finds duplicates, small covers and tags worth fixing." },
  { id: "soundId", title: "Identify by sound", group: "Library", kind: "manage", blurb: "Names songs by their audio when tags don't." },
  { id: "hidden", title: "Hidden and not interested", group: "Library", kind: "manage", blurb: "Songs, artists and suggestions you've hidden." },
  { id: "devices", title: "Devices", group: "Downloads and devices", kind: "manage", blurb: "USB drives and players you copy music to." },
  { id: "looks", title: "Looks", group: "Appearance", kind: "manage", blurb: "Ready-made looks, your own, and theme files." },
  { id: "transcription", title: "Lyrics transcription", group: "Lyrics", kind: "manage", blurb: "The speech model and how hard it may work." },
  { id: "sync", title: "Sync", group: "App", kind: "manage", blurb: "Keep your devices in sync, sync with a folder, and copy keys." },
  { id: "guides", title: "Setup guides", group: "App", kind: "manage", blurb: "Short walk-throughs for features that need setting up." },
];

export function pageMeta(id: string | undefined): PageMeta | undefined {
  return PAGES.find((p) => p.id === id);
}

/**
 * How the page is laid out. Tests turn `inlinePages` on so a test of one
 * row needn't open that row's page first (the layout tests turn it off).
 */
export const settingsView = { inlinePages: false };

/** Descriptions stay on one line; longer text goes behind "?". */
export const DESCRIPTION_MAX = 100;

/**
 * The rows a section shows by default: what most people use. Every other
 * row of a section's main view sits under its More, which search still
 * reaches (marking the row "In More"). A new row goes under More unless it
 * is added here, so a section stays short.
 */
export const COMMON: ReadonlySet<string> = new Set([
  // Account and servers: who you are and which server.
  "account",
  "servers",
  "libraries",
  // Playback: the switches people actually change.
  "queueEnd",
  "smartShuffle",
  "crossfade",
  "automix",
  "visualizerButton",
  // Sound: quality, where it plays, loudness and the equalizer.
  "qualityHome",
  "qualityAway",
  "outputDevice",
  "leveling",
  "eqEnabled",
  // Library: syncing, search beyond the library, mixes and Stats.
  "resync",
  "catalogueSearch",
  "homeMixes",
  "statsPage",
  "releaseRadar",
  "explicitContent",
  // Downloads and devices: the basics, and going offline.
  "downloadQuality",
  "downloadsStorage",
  "downloadLiked",
  "goOffline",
  "showDevices",
  // Appearance: look, layout and colour.
  "layout",
  "theme",
  "phoneLook",
  "accent",
  "artworkColours",
  "reduceMotion",
  // Lyrics: whether they show, where they come from, and writing them down.
  "lyricsMode",
  "lyricsLrclib",
  "transcribeLyrics",
  // Sharing and friends: privacy and the main ways to share.
  "privateSession",
  "sharePublicLinks",
  "showFriendActivity",
  "collabPlaylists",
  "shareMyMusic",
  "sharePeople",
  // App: updates, notifications, the tray and start-up, and About.
  "updates",
  "notifications",
  "closeToTray",
  "launchAtLogin",
  "about",
  // The phone's own: mobile data for downloads.
  "phoneMobileDownloads",
]);

/** Tool pages whose Manage row sits under its section's More (rarely needed). */
export const MANAGE_IN_MORE: ReadonlySet<PageId> = new Set<PageId>(["tags", "soundId", "hidden", "devices", "guides"]);

/**
 * Whether a row sits under More: on a section's main view, any row that isn't
 * common; on an integration's or tool's own page, the finer ones (ADVANCED).
 */
export function inMore(row: { key: string; page?: string }): boolean {
  return row.page ? ADVANCED.has(row.key) : !COMMON.has(row.key);
}

/** Where each section's More is remembered open, on this device. */
export const MORE_OPEN_KEY = "platter.settings.moreOpen";

export function loadMoreOpen(): Set<string> {
  try {
    const v = JSON.parse(localStorage.getItem(MORE_OPEN_KEY) ?? "[]");
    return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

export function saveMoreOpen(open: ReadonlySet<string>) {
  try {
    localStorage.setItem(MORE_OPEN_KEY, JSON.stringify([...open]));
  } catch {
    // Kept for this visit only.
  }
}

/**
 * On a page, rows most people never need, folded under its More (on a
 * section's main view, COMMON decides instead; the section names below
 * still say what each holds). Settings search still finds them.
 * The rule: everyday choices stay in view; tuning, numbers, technical
 * names, and follow-up options of something else go under More.
 */
export const ADVANCED: ReadonlySet<string> = new Set([
  // Library import: what bringing music over does as albums arrive, and its pace.
  "bringLikeArrivals",
  "bringFillPlaylists",
  "bringInPerDay",
  // Account and servers: only matter with Subsonic, a Plex Home or a picture to hide.
  "subsonicCheckMinutes",
  "saveQueueOnServer",
  "showPlexAvatar",
  "homeRemember",
  // Playback: tuning and the finer Automix rules.
  "precacheCount",
  "abLoopHandles",
  "crossfadeSkipAlbums",
  "automixWhere",
  "automixStyle",
  "flipPause",
  // Sound: display details and fine tuning of loudness.
  "showFormatBadge",
  "preampDb",
  "limiter",
  // Library: how lists, search and mixes behave beyond the defaults.
  "popularSource",
  "linkFeatured",
  "libraryChipsRemember",
  "playlistAskDuplicates",
  "playlistNewName",
  "likedMeans",
  "bookmarksInLibrary",
  "pinsInLibrary",
  "spineWidthByTracks",
  "likeControlRows",
  "shelfMissingList",
  "shelfOpen",
  "sleeveLookups",
  "showVinylSides",
  "showCrates",
  "crateShelves",
  "keepDiary",
  "showFanEdits",
  "artistAliases",
  "groupEditions",
  "showDescribePlaylist",
  "showSoundMap",
  "dragAndDrop",
  "searchSuggestions",
  "searchTypos",
  "searchRecents",
  "catalogueSource",
  "askBeforeGet",
  "getAlbumsKinds",
  "discographyChip",
  "discographySort",
  "discographyReverse",
  "discographyView",
  "mixExclude",
  "daylistHours",
  "fansAlsoLikeCatalogue",
  "homeNewToYou",
  "includePlexHistory",
  // Tags and archive: tag reading and the archive rules, for people who tag by hand.
  "rescanAfterTagEdit",
  "askBeforeManyFiles",
  "plexLibrarySettings",
  "readFileTags",
  "countUnmatchedUnreleased",
  "checkUnmatchedMusicbrainz",
  "archiveFolderKeywords",
  "releaseStatusTag",
  "releaseStatusValues",
  "compareLevelMatch",
  // Library doctor and identify by sound: their numbers and keys.
  "doctorCoverPx",
  "acoustidKey",
  "soundIdScope",
  // Downloads and devices: safe defaults and names nobody needs to see.
  "downloadFolder",
  "downloadHomeOnly",
  "computerName",
  "moveStopsOldDevice",
  // Appearance: small display details and the miniplayer's finer choices.
  "sidebarArt",
  "upNextLine",
  "volumePercent",
  "followSystem",
  "tonearmSeek",
  "readoutFace",
  "coverLight",
  "layoutOffered",
  "homeRecordsFirst",
  "homeColourWash",
  "homeOnThisDay",
  "homeTimeTravel",
  "fullWindowHideControls",
  "fullWindowRememberMode",
  "miniplayerTint",
  "miniplayerQueue",
  "miniplayerSideBySide",
  // The phone shows the cover in now playing, never a record player.
  "recordMotion",
  // Visualizer: its tuning (intensity included), the panel strip and resting on battery.
  "visualizerFps",
  "visualizerSmoothing",
  "visualizerIntensity",
  "visualizerPanelStrip",
  "visualizerBatteryPause",
  // Looks: building and files.
  "showLookBuilder",
  "themeFile",
  // Lyrics: fine tuning and translation.
  "lyricsLiveTiming",
  "lyricsReleasedForVersions",
  "lyricsChoices",
  "translationService",
  "translateTo",
  "nowPlayingLyricsCard",
  // Lyrics transcription: background work.
  "transcribePauseBusy",
  "transcribeFasterAway",
  "transcribeUnreleased",
  "syncTranscriptions",
  // Integrations: tokens, caches, ports and the finer Discord and Lidarr rules.
  "infoListenbrainzToken",
  "infoCache",
  "nowPlayingFeedPort",
  "aiModel",
  "lidarrAwayUrl",
  "lidarrHeaders",
  "homeSideLidarr",
  "lidarrScanAfterImport",
  "lidarrFreeStaging",
  "lidarrTapToGet",
  "lidarrPreferFastest",
  "lidarrPlayArriving",
  "upgradeOfficialPrefix",
  "autoRequestCap",
  "discordClearOnPause",
  "discordStatusLine",
  "discordListenAlong",
  // Platter Bridge: each benefit's switch and the night hours, under its card's More.
  "bridgeLyrics",
  "bridgeLyricsHours",
  "bridgeLidarr",
  // Sharing and friends: share cards, sessions, invites, and unreleased music feature by feature.
  "blends",
  "shareCardQr",
  "shareCardSongLink",
  "shareDefaultLibraries",
  "shareHideUnreleased",
  "shareAllowDownloads",
  "shareAcceptInvites",
  "listenGuestsControl",
  "listenInviteHours",
  "listenName",
  "listenRelay",
  "shareOfferServer",
  "discordShowUnreleased",
  "discordUnreleasedArt",
  "mixUnreleased",
  "scrobbleUnreleased",
  "listenHideUnreleased",
  "shareUnreleased",
  "shareTranscriptionsUnreleased",
  // Sync: what goes in the file, and moving keys between devices.
  "syncSettings",
  "syncLibrary",
  "syncFollows",
  "syncLyrics",
  "syncLooks",
  "syncDownloads",
  "syncPlays",
  "keysReceive",
  "keysSend",
  // App: keyboard power use, system integration that's on already, help and cache.
  "mediaKeys",
  "powerKeys",
  "hintKey",
  "showSetupGuides",
  "showHelpButtons",
  "artCache",
]);

/**
 * Whether More starts open everywhere. Off in the app (each section opens as
 * it was left); the test setup opens it so tests of a row needn't click More first.
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
  catalogueSource: ["search", "music-that-isnt-in-your-library"],
  askBeforeGet: ["search", "music-that-isnt-in-your-library"],
  getAlbumsKinds: ["search", "music-that-isnt-in-your-library"],
  searchTypos: ["search", "forgiving-search"],
  look: ["getting-started", "picking-a-look"],
  layout: ["layouts", "picking-a-layout"],
  layoutOffered: ["layouts", "picking-a-layout"],
  libraryChipsRemember: ["your-library", "remember-library-filters"],
  spineWidthByTracks: ["getting-started", "record-shelf"],
  lyricsMode: ["lyrics-and-transcription", "where-lyrics-come-from"],
  lyricsLrclib: ["lyrics-and-transcription", "lrclib"],
  lyricsChoices: ["lyrics-and-transcription", "wrong-lyrics"],
  showPronunciation: ["lyrics-and-transcription", "translation-and-pronunciation"],
  translationService: ["lyrics-and-transcription", "translation-and-pronunciation"],
  transcribeLyrics: ["lyrics-and-transcription", "transcription"],
  shareTranscriptions: ["lyrics-and-transcription", "sharing-transcriptions"],
  sharePublicLinks: ["listening-together", "share-links"],
  shareMyMusic: ["listening-together", "sharing-your-music"],
  sharePeople: ["listening-together", "people-with-access"],
  shareHideUnreleased: ["listening-together", "keeping-unreleased-music-home"],
  shareAcceptInvites: ["listening-together", "invites-to-you"],
  shareOfferServer: ["listening-together", "let-them-listen-from-my-server"],
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
  unreleasedSharing: ["listening-together", "keeping-unreleased-music-home"],
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
  artistInfoSources: ["privacy", "off-until-you-turn-them-on"],
  showSleeveNotes: ["getting-started", "sleeve-notes"],
  showVinylSides: ["getting-started", "record-sides"],
  flipPause: ["getting-started", "record-sides"],
  sleeveLookups: ["getting-started", "where-the-facts-come-from"],
  nowPlayingFeed: ["privacy", "off-until-you-turn-them-on"],
};

/** Groups that get a "?" beside their heading (busier screens). */
export const GROUP_HELP: Readonly<Record<string, HelpRef>> = {
  "Lyrics transcription": ["lyrics-and-transcription", "transcription"],
  "Listening sessions": ["listening-together"],
  Lidarr: ["lidarr"],
  "Tags and archive": ["tag-editor"],
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
  // Rows that hold a checklist or one choice over several settings.
  statsPage: ["statsPage", "statsStrip", "monthlyStats", "statsCompare", "wrappedTopSongs", "topSongsShowPlays"],
  showDevices: ["showPlexPlayers", "showCastDevices", "showDlnaDevices", "showAirplayLocked"],
  artistInfoSources: ["infoWikipedia", "infoMusicbrainz", "infoListenbrainz", "infoListenbrainzStats"],
  notifications: ["notifyNewReleases", "radarNotify", "whatsNewAnnouncements", "notifyTrackChange"],
  discordCoverSource: ["discordCoverArchive", "discordUploadCovers", "discordUploadArt", "discordCoverHost"],
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

/**
 * Rows that only make sense on a computer: the tray, the window, global
 * keys, the miniplayer, layouts, folders on disk, this computer's name and
 * work that needs a desktop processor. The phone layout leaves them out.
 */
export const DESKTOP_ONLY: ReadonlySet<string> = new Set([
  // Integrations: a phone isn't the computer at home that answers for Lidarr.
  "homeSideLidarr",
  // App: the tray, start-up, keys and the desktop updater.
  "closeToTray",
  "launchAtLogin",
  "keyboardShortcuts",
  "mediaKeys",
  "powerKeys",
  "hintKey",
  "updates",
  "settingsFile",
  // Appearance: layouts, the window, the sidebar, the player bar and the miniplayer.
  "layout",
  "layoutOffered",
  "sidebarArt",
  "upNextLine",
  "volumePercent",
  "fullWindowHideControls",
  "fullWindowRememberMode",
  "miniplayerOnTop",
  "miniplayerTint",
  "miniplayerQueue",
  "miniplayerSideBySide",
  // The phone shows the cover in now playing, never a record player.
  "recordMotion",
  // Looks: building looks and theme files want a mouse and a folder.
  "makeOwn",
  "showLookBuilder",
  "themeFile",
  // Library: mouse dragging and the shelf's spines.
  "dragAndDrop",
  "spineWidthByTracks",
  // Sound: the phone picks its output through the system.
  "outputDevice",
  // Downloads and devices: folders on disk. (Being a Plex player and this
  // device's name work on a phone too; the Devices page is USB drives.)
  "downloadFolder",
  // Lyrics: transcription runs on a computer.
  "transcribeLyrics",
  // Discord lives on the computer.
  "discordShowUnreleased",
  "discordUnreleasedArt",
  // Visualizer: the phone has no side panel.
  "visualizerPanelStrip",
]);

/** Pages the phone layout leaves out, for the same reason. */
export const DESKTOP_ONLY_PAGES: ReadonlySet<PageId> = new Set<PageId>(["discord", "streamDeck", "widgetFeed", "devices", "tags", "soundId", "transcription"]);

/** Whether a row shows in the phone layout. */
export function onPhone(row: { key: string; page?: PageId }): boolean {
  return !DESKTOP_ONLY.has(row.key) && !PHONE_REPLACED.has(row.key) && !(row.page && DESKTOP_ONLY_PAGES.has(row.page));
}

/** Rows the phone shows its own way: Theme and Follow system become one Look row. */
export const PHONE_REPLACED: ReadonlySet<string> = new Set(["theme", "followSystem"]);

/** A row's words said for a phone: "this computer" becomes "this phone". */
export function phoneText(text: string): string {
  return text
    .replace(/\b(this|your|the) computer's\b/gi, (_, w: string) => `${w} phone's`)
    .replace(/\b(this|your|the) computer\b/gi, (_, w: string) => `${w} phone`)
    .replace(/\bon your computers\b/gi, "on your devices")
    .replace(/\byour computers\b/gi, "your devices")
    .replace(/\bon this drive\b/gi, "on this phone")
    .replace(/\bDefault Device\b/g, "this phone's speaker")
    .replace(/,?\s*unless the queue is open there\b/gi, "")
    .replace(/\s+on the right\b/gi, "")
    .replace(/\bas a dot on the bell\b/gi, "with a dot on What's new")
    .replace(/ {2,}/g, " ")
    .replace(/ ([,.])/g, "$1");
}

/** The same row with its words said for a phone. */
export function phoneWords<T extends { title: string; description: string; more?: string }>(row: T): T {
  return { ...row, title: phoneText(row.title), description: phoneText(row.description), more: row.more === undefined ? undefined : phoneText(row.more) };
}

/** One line under each group on the phone's list of sections. */
export const GROUP_BLURB: Readonly<Record<GroupName, string>> = {
  "Account and servers": "Your sign-in, servers and libraries",
  Playback: "Shuffle, crossfade, Automix and the queue",
  Sound: "Quality, loudness and the equalizer",
  Library: "Search, mixes and what pages show",
  "Downloads and devices": "Offline music, mobile data and other players",
  Appearance: "Your look, colours and motion",
  Lyrics: "Where lyrics come from and how they show",
  Integrations: "Last.fm, Lidarr and other services",
  "Sharing and friends": "Share links, sessions and friends",
  App: "Notifications, help and the cache",
};
