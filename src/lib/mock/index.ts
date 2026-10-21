/**
 * In-memory mock of the Rust core, used outside Tauri (Vite dev server,
 * Playwright, Vitest). Implements every command and event in docs/IPC.md with
 * invented fixture data, simulated playback (position ticks 4×/s, the same
 * queue rules as the core: Previous by context position, resume points,
 * per-context shuffle, context end rewinds, repeat-one drops), sync progress
 * and health changes.
 *
 * URL switches (plain browser only):
 *   ?latency=ms     delay every command (default 120; 0 in tests)
 *   ?signedOut=1    start on the sign-in screen
 *   ?serverPick=1   start signed in with a server still to pick
 *   ?albums=5000    pad the album list to N items (virtual grid perf check)
 *   ?httpArt=1      art URLs in the Windows app's form (http://art.localhost/…?w=&h=)
 *                   instead of generated SVGs, for memory checks that serve
 *                   real JPEGs at the requested size (scripts/measure-memory.mjs)
 *   ?update=1       "Check for updates" finds a new version (ready to install)
 *   ?tagScan=1      the file tag scan starts from nothing read (progress in Settings)
 *   ?feedPortInUse=32540  another program has that port (the now-playing feed says so)
 *   ?link=<url>     open a platter:// link a moment after start, as if the
 *                   OS had handed it over (url-encoded)
 *   ?health=offline start in a given app state (see HEALTH_DEMOS in ./health:
 *                   offline, noInternet, starting, updating, startTimeout,
 *                   relay, remote, signIn, accessRemoved, filesUnavailable,
 *                   noMusicLibrary, diskFull, scanning, goneOffline)
 *   ?downloads=demo the first album and Liked Songs start downloaded
 *   ?downloads=full as demo, with the storage limit already reached
 *   ?view=miniplayer show the miniplayer window's page
 *   ?import=demo    two library import sources on and already imported
 *   ?stats=empty    no demo listening history (the Stats page's empty state)
 *   ?compare=1      Compare stats with friends starts on
 *   ?listen=cantHear  host a listening session where Robin can't hear the song
 *                   (shows "Let them listen from my server")
 *   ?listen=invited a guest who can't hear the host's song, with the host's
 *                   server invite waiting
 */
import type { Backend, HapticKind, Unlisten } from "../backend";
import { compareApproximate, compareMap, compareMapLoop } from "../compare";
import type { LastGet, Tap, TapInput, TapOutcome } from "../types";
import type {
  IsrcRelease,
  MiniplayerGo,
  TagCheckScope,
  TagFixKind,
  TagSaveRequest,
  SetOwned,
  PlayHistorySource,
  WantedAlbum,
  CompareNow,
  CompareReady,
  CompareSide,
  LidarrPickInput,
  LidarrRequestInput,
  CataloguePartKind,
  QueuedGet,
  LidarrFixAction,
  LidarrModeSetting,
  LidarrTarget,
  ShareMap,
  UpgradeUnder,
  CachedLibrary,
  ListenOp,
  ShareLinkAsk,
  ShareOptions,
  DownloadKind,
  AbLoop,
  AddPlace,
  Album,
  AlbumSort,
  Artist,
  ArtistInfo,
  ArtistPage,
  ArtistSort,
  AudioRoute,
  AuthStatus,
  EventMap,
  EventName,
  Health,
  HomeShelfId,
  KeyedInfoSource,
  LibraryItem,
  OutputDevice,
  OutputState,
  SignalPath,
  PinInfo,
  PlayContextArgs,
  PlayerState,
  QueueContextInfo,
  QueueItem,
  RemoteCommand,
  RepeatMode,
  ReleaseSection,
  SearchArgs,
  SearchHit,
  SearchKind,
  SearchResults,
  ServerInfo,
  Settings,
  ShareKind,
  SyncStatus,
  OwnLyrics,
  Lyrics,
  LyricsCandidate,
  LyricsCandidates,
  LyricsCheckStatus,
  LyricsVerdict,
  KeyKind,
  PinRecord,
  MixStyle,
  Track,
  AudioQuality,
  TranscribeModel,
  QueueAt,
  MixSteer,
  HideRequest,
  NewBookmark,
  SmartRules,
  SmartSaveRequest,
  FeedStatus,
  DeckPluginStatus,
  DeckStatus,
  BridgeOverview,
  HomeSides,
  EditFields,
  EditKind,
  ImageKind,
  MatchCandidate,
  ReleaseMark,
  DoctorFinding,
  DoctorKind,
  SoundIdStatus,
  SoundsLike,
  ThemeFile,
  DeviceItemKind,
  DevicePatch,
  SoundMapArea,
  CrateKind,
  DiaryEdit,
  DiaryQuery,
  SubsonicConfirm,
  SubsonicCreds,
} from "../types";
import { DEFAULT_SETTINGS, PHONE_KEEP_RECENT_CHOICES } from "../types";
import { isLosslessCodec } from "../libraryChips";
import { FRAMES, layoutProblem, type Frame } from "../../design/themes/validate";
import { MockCrates } from "./crates";
import { exportSettings, importSettings, type SettingsFile } from "../settingsFile";

/** More lyrics found for a mock song (an LRCLIB hit), for tests and screenshots. */
export interface MockLyricsCandidate {
  key: string;
  origin: LyricsCandidate["origin"];
  album?: string;
  durationMs?: number;
  otherVersion?: boolean;
  lyrics: Lyrics;
}
import { CREDIT_PREFIX, buildArtistLookup, creditId, creditKey, isCreditId, splitCredits } from "../credits";
import { mockArtUrl } from "./art";
import { mockLyrics, mockNowPlayingDetails } from "./details";
import { lyricsFromText } from "../lrc";
import { credits, globalTop, popularTracks, rankPopular } from "./popular";
import {
  ARCHIVE_ARTIST_ID,
  BROKEN_TRACK_TITLE_SUFFIX,
  LEARNED_NAMES,
  buildFixtures,
  mockMatched,
  mockMbCheckOn,
  mockPlexMissed,
  mockUnreleased,
  mulberry32,
  type Fixtures,
} from "./fixtures";
import { mockBio } from "./bio";
import { MOCK_CLOUD_FOLDERS, MockSync } from "./sync";
import { MockDeviceSync } from "./deviceSync";
import { MockImport, importLinkAllowed } from "./importData";
import type { AccountStatus, ImportDecision, ImportSourceId, PlayedHidden, PlayedQuery } from "../importTypes";
import { MockAccount } from "./accountData";
import { mockArtistInfo, mockKeysStatus, mockSetKey, type MockInfoKeys } from "./artistInfo";
import { mockAlbumCredits } from "./albumCredits";
import { demoHealth, isHealthDemo, type HealthDemo } from "./health";
import { OFFLINE_NOTHING_PLAYS, offlineLeftOutMessage } from "../offline";
import * as eq from "./eq";
import * as hifi from "./hifi";
import { mockSignalPath } from "./signal";
import { MockVisualizer } from "./visualizer";
import { MockHome, type ContextPlay } from "./home";
import { mockRecordHome } from "./records";
import { MockHomeUsers } from "./homeUsers";
import { mockBackendCaps } from "./servers";
import { MockLibraries } from "./libraries";
import { MockLikes } from "./likes";
import { buildRecents, seedListens, type MockListen } from "./recents";
import { MockPlaylists } from "./playlists";
import { MockRemote } from "./remote";
import { MockShare, mockQr } from "./share";
import { isDiscordDemo, MockDiscord, type DiscordDemo } from "./discord";
import { MockFileTags } from "./fileTags";
import { MockThemes, checkThemePatch } from "./themes";
import { MockTranscribe } from "./transcribe";
import { MockLyricsShare } from "./lyricsShare";
import { MockDownloads } from "./downloads";
import { MockMixes } from "./mixes";
import { MockSocial } from "./social";
import { MockCircle } from "./circle";
import { MockSharing } from "./sharing";
import { MockBookmarks, MockHidden, MockScrobble, MockSmart } from "./curate";
import { MockPlexEdit } from "./plexEdit";
import { MockTagEdit } from "./tagEdit";
import { MockLidarr, isLidarrDemo, type LidarrDemo } from "./lidarr";
import { MockBring } from "./bring";
import { MockPlayHistory } from "./playhistory";
import { MockLidarrWork } from "./lidarrModes";
import { MOCK_STAGING, MockUpgrades, isUpgradeDemo, type UpgradeDemo } from "./upgrades";
import { MockRadar } from "./radar";
import { MockCatalogue } from "./catalogue";
import { MockStats, mockFriendSummary, seedStatsHistory } from "./stats";
import type { CompareSummary, StatsItemKind, StatsRange, StatsTopKind } from "../statsTypes";
import { MockArchive } from "./archive";
import { MockDoctor } from "./doctor";
import { MockPhysical } from "./physical";
import { MockSleeve } from "./sleeve";
import { MockSides } from "./sides";
import { MockDiary } from "./diary";
import { MockAutomix } from "./automix";
import { MockDevices } from "./devices";
import { MockSoundMap } from "./soundmap";
import { MockJellyfin, plexRecord } from "./jellyfin";
import { MockSubsonic } from "./subsonic";
import { CONSOLE_COLUMNS, clampPane, consoleSortOk } from "../consoleLayout";

/** Record labels for the mock album footers. */
const MOCK_LABELS = ["Northlight Records", "Harbor Sound", "Quiet Hours Music", "Glasshouse Recordings"];

export interface MockOptions {
  latencyMs: number;
  signedOut: boolean;
  /** Signed in, with a server still to pick (?serverPick=1). */
  serverPick?: boolean;
  albumCount?: number;
  /** Art URLs in the app's http://art.localhost form (see the header). */
  httpArt?: boolean;
  /** Run startup timelines (health "starting" → "local", incremental sync). */
  timelines: boolean;
  health?: HealthDemo;
  /** Auto-approve the sign-in PIN after this many ms (null = never). */
  autoApproveMs: number | null;
  /** "Check for updates" finds a new version. */
  updateAvailable?: boolean;
  /** A platter:// link to open after start. */
  link?: string;
  /** Discord status demo: no app id, Discord closed, or a failed upload. */
  discord?: DiscordDemo;
  /** Another program has this port, so the now-playing feed can't open it. */
  feedPortInUse?: number;
  /** Start the file tag scan from nothing read (shows its progress in Settings). */
  tagScan?: boolean;
  /** The other Platter computer is seen through Plex but doesn't answer. */
  platterUnreachable?: boolean;
  /** A phone running Platter that's asleep (?platter=phone), or one that
   * wakes a few seconds in (?platter=phone-wakes). */
  platterPhone?: "asleep" | "wakes";
  /** Long Windows output names, for checking nothing cuts off (?outputs=long). */
  longOutputs?: boolean;
  /** Start with downloads in place (and the limit reached, for "full"). */
  downloads?: "demo" | "full";
  /** Start on this theme (?theme=light). */
  theme?: Settings["theme"];
  /** What the faked Lidarr looks like (?lidarr=clear). */
  lidarr?: LidarrDemo;
  /** Start with a Lidarr upgrade offered on the first album (?upgrades=ready); sets Lidarr up too. */
  upgrades?: UpgradeDemo;
  /** The release radar has found nothing yet (?radar=empty). */
  radarEmpty?: boolean;
  /** Start with two import sources on and imported (?import=demo). */
  importDemo?: boolean;
  /** No demo listening history for stats (?stats=empty). */
  statsEmpty?: boolean;
  /** Only 24 days of listening in the last year, most in the last four weeks (?stats=sparse). */
  statsSparse?: boolean;
  /** Start with Compare stats with friends on (?compare=1). */
  statsCompare?: boolean;
  /** The day Top Songs and Wrapped take as today, "YYYY-MM-DD" (?today=2026-12-05). */
  today?: string;
  /** Start in a listening session that shows sharing (?listen=cantHear or ?listen=invited). */
  listenDemo?: "cantHear" | "invited";
  /** Start on this layout frame (?frame=deck). */
  frame?: Frame;
  /** A library with no music in it (?library=empty). */
  emptyLibrary?: boolean;
  /** Very long album, artist and song titles (?titles=long). */
  longTitles?: boolean;
  /** Every third album has no cover, the playing one too (?art=missing). */
  missingArt?: boolean;
}

const isTestEnv = (() => {
  try {
    return import.meta.env?.MODE === "test";
  } catch {
    return false;
  }
})();

/** Same cap as the core's CONTEXT_MEMORY_CAP. */
const CONTEXT_MEMORY_CAP = 200;
/** Previous restarts the track at or past this point. */
export const PREVIOUS_RESTART_MS = 3000;

interface ResumePoint {
  index: number;
  trackId: string;
  positionMs: number;
}

/** An album's files from its songs, as the core works it out for the format filter. */
export function albumQuality(tracks: Pick<Track, "codec" | "bitDepth" | "sampleRate">[]): AudioQuality | null {
  if (!tracks.length) return null;
  if (tracks.some((t) => !isLosslessCodec(t.codec))) return "lossy";
  return tracks.some((t) => (t.bitDepth ?? 0) > 16 || (t.sampleRate ?? 0) > 48_000) ? "hiRes" : "lossless";
}

/**
 * Screenshot switches over the built library: long titles and missing art.
 * Without them the fixtures come back untouched. (An empty library is
 * emptied after the mock is built; see createMockBackend.)
 */
export function shapeFixtures(fx: Fixtures, o: Partial<MockOptions>): Fixtures {
  if (o.longTitles) {
    fx.albums.forEach((a, i) => {
      if (i % 3 !== 0) return;
      a.title = `${a.title}: The Complete Sessions, Outtakes and Alternate Takes (Deluxe Anniversary Edition)`;
      a.sortTitle = a.title.toLowerCase();
      for (const t of fx.tracksByAlbum.get(a.id) ?? []) {
        t.albumTitle = a.title;
        t.title = `${t.title} (Extended Version, Recorded Live at the Old Exchange Hall)`;
        t.trackArtist = `${t.artistTitle} feat. Mara Quill, Odessa Fenn, The Low Orchard and Jun Takeda`;
      }
    });
    const long = fx.artists[0];
    if (long) {
      long.title = `${long.title} and the Northern Lights Chamber Orchestra`;
      for (const a of fx.albums) if (a.artistId === long.id) a.artistTitle = long.title;
      for (const t of fx.tracks) if (t.artistId === long.id) t.artistTitle = long.title;
    }
  }
  if (o.missingArt) {
    fx.albums.forEach((a, i) => {
      if (i % 3 !== 0) return;
      a.thumb = null;
      for (const t of fx.tracksByAlbum.get(a.id) ?? []) t.thumb = null;
    });
  }
  return fx;
}

export function optionsFromUrl(search: string): MockOptions {
  const p = new URLSearchParams(search);
  return {
    latencyMs: p.has("latency") ? Number(p.get("latency")) : isTestEnv ? 0 : 120,
    signedOut: p.get("signedOut") === "1",
    serverPick: p.get("serverPick") === "1",
    albumCount: p.has("albums") ? Number(p.get("albums")) : undefined,
    httpArt: p.get("httpArt") === "1",
    timelines: !isTestEnv,
    health: isHealthDemo(p.get("health")) ? (p.get("health") as HealthDemo) : undefined,
    autoApproveMs: isTestEnv ? null : 6000,
    updateAvailable: p.get("update") === "1",
    link: p.get("link") ?? undefined,
    discord: isDiscordDemo(p.get("discord")) ? (p.get("discord") as DiscordDemo) : undefined,
    tagScan: p.get("tagScan") === "1",
    feedPortInUse: Number(p.get("feedPortInUse")) || undefined,
    platterUnreachable: p.get("platter") === "unreachable",
    platterPhone: p.get("platter") === "phone" ? "asleep" : p.get("platter") === "phone-wakes" ? "wakes" : undefined,
    longOutputs: p.get("outputs") === "long",
    downloads: p.get("downloads") === "demo" || p.get("downloads") === "full" ? (p.get("downloads") as "demo" | "full") : undefined,
    lidarr: isLidarrDemo(p.get("lidarr")) ? (p.get("lidarr") as LidarrDemo) : isUpgradeDemo(p.get("upgrades")) ? "ready" : undefined,
    upgrades: isUpgradeDemo(p.get("upgrades")) ? (p.get("upgrades") as UpgradeDemo) : undefined,
    radarEmpty: p.get("radar") === "empty",
    theme: (["dark", "light", "system", "midnight", "forest", "sand", "studio", "studioBlack", "sleeve"] as const).find((t) => t === p.get("theme")),
    importDemo: p.get("import") === "demo",
    statsEmpty: p.get("stats") === "empty" || p.get("library") === "empty",
    statsSparse: p.get("stats") === "sparse",
    statsCompare: p.get("compare") === "1",
    today: /^\d{4}-\d{2}-\d{2}$/.test(p.get("today") ?? "") ? (p.get("today") as string) : undefined,
    frame: FRAMES.find((f) => f === p.get("frame")),
    emptyLibrary: p.get("library") === "empty",
    longTitles: p.get("titles") === "long",
    missingArt: p.get("art") === "missing",
    listenDemo: p.get("listen") === "cantHear" || p.get("listen") === "invited" ? (p.get("listen") as "cantHear" | "invited") : undefined,
  };
}

/** Profile picture path. The mock art draws an abstract silhouette for it. */
const AVATAR = "/ext/https://plex.tv/users/mock/avatar?c=1";

const SERVER: ServerInfo = {
  machineId: "mock-machine-01",
  name: "Basement Server",
  owned: true,
  online: true,
  version: "1.42.2",
  connection: "local",
};

/** Long Windows-style names (a gaming headset's virtual devices). */
const LONG_OUTPUTS: OutputDevice[] = [
  { id: "mock:sonar-gaming", name: "SteelSeries Sonar - Gaming (SteelSeries Sonar Virtual Audio Device)", isDefault: true },
  { id: "mock:sonar-media", name: "SteelSeries Sonar - Media (SteelSeries Sonar Virtual Audio Device)", isDefault: false },
  { id: "mock:headset", name: "Headphones (Arctis Nova Pro Wireless)", isDefault: false },
];

/** Invented audio outputs; the first is the system default. */
const MOCK_OUTPUTS: OutputDevice[] = [
  { id: "mock:speakers", name: "Speakers (Built-in Audio)", isDefault: true },
  { id: "mock:headphones", name: "Headphones (USB Audio DAC)", isDefault: false },
  { id: "mock:hdmi", name: "Living Room Receiver (HDMI)", isDefault: false },
];

const clone = <T,>(v: T): T => (typeof structuredClone === "function" ? structuredClone(v) : JSON.parse(JSON.stringify(v)));

export class MockBackend implements Backend {
  readonly kind = "mock" as const;
  readonly fx: Fixtures;
  /** Playlist editing (test hooks: refuseNextWrite, deleteElsewhere). */
  readonly pl: MockPlaylists;
  /** Likes (test hooks: refuseNextWrite, likeElsewhere, clearAll). */
  readonly likes: MockLikes;
  /** Listen on (test hooks: stopElsewhere, loseRemote, setPlayers). */
  readonly remote: MockRemote;
  /** Sharing (test hooks: openLink, failNext). */
  readonly share: MockShare;
  /** Discord status (test hooks: setConnected, setAvailable, failUpload). */
  readonly discord: MockDiscord;
  /** File tags: badges, editions, filters and the scan. */
  readonly tags: MockFileTags;
  /** Lyrics transcription (test hooks: failNextDownload, failNextSong, silent, stepMs). */
  readonly transcribe: MockTranscribe;
  readonly lyricsShare: MockLyricsShare;
  /** Downloads (test hooks: failNext, loseFile, connection, stepMs, steps). */
  readonly downloads: MockDownloads;
  /** Edit in Plex (test hooks: shared, failNext). */
  readonly plexEdit: MockPlexEdit;
  readonly tagEdit: MockTagEdit;
  /** The library doctor's findings (test hooks: ignored, fixed, running, reset). */
  readonly doctor: MockDoctor;
  /** Mixes and radio, made from the fixtures. */
  readonly mixes: MockMixes;
  /** The archive: release status and marks, versions, sections, eras (and its demo artist). */
  readonly archive: MockArchive;
  /** Smooth transitions (player state). */
  private smoothFor: string | null = null;
  /** Friend activity, follows, What's New, listening sessions (test hooks: access, guestJoins, hostGone, hostEnds, addRelease). */
  readonly social: MockSocial;
  /** Shared playlists and Blends (test hooks: relay, ownerOffline, memberAdds, ownerDeletes, failNextJoin, joinWaits, ring, personJoins, blendJoins). */
  readonly circle: MockCircle;
  /** Sharing your music through Plex (test hooks: plexPass, owner, failNext, inviteArrives). */
  readonly sharing: MockSharing;
  readonly syncFile: MockSync;
  readonly deviceSync: MockDeviceSync;
  /** Library import (test hook: failNext). */
  readonly importMock: MockImport;
  /** Bringing music over (test hooks: scenario, notFound, arrive). */
  readonly bring = new MockBring();
  readonly account: MockAccount;
  /** Store links the Missing list opened. */
  readonly importLinksOpened: string[] = [];
  /** Hidden songs and artists. */
  readonly hidden: MockHidden;
  readonly bookmarks: MockBookmarks;
  /** Smart playlists and the AI provider (test hooks: refusePlex, offline). */
  readonly smart: MockSmart;
  /** Scrobbling (test hook: reject). */
  readonly scrobble: MockScrobble;
  readonly playHistory: MockPlayHistory;
  /** Lidarr's setup check (test hook: setDemo). */
  readonly lidarr: MockLidarr;
  readonly lidarrMock: MockLidarrWork;
  /** Lidarr upgrades and the music share mapping (test hook: reset). */
  readonly upgrades: MockUpgrades;
  /** The release radar (test hooks: setEmpty, reset). */
  readonly radar: MockRadar;
  /** Music that isn't in the library (faked MusicBrainz and ListenBrainz). */
  readonly catalogue: MockCatalogue;
  /** The physical collection and the faked Discogs (test hooks: refuse, fail, reset). */
  readonly physical: MockPhysical;
  /** Sleeve notes and credit pages (made up from the mock credits). */
  readonly sleeve: MockSleeve;
  /** Record sides (vinyl editions and splits by hand). */
  readonly sides: MockSides;
  /** The listening diary. */
  readonly diary: MockDiary;
  /** What the last diary export would have written, and in which format. */
  lastDiaryExport: { format: "text" | "csv"; body: string } | null = null;
  /** Mixed playlists and made-up tempos (test hook: reset). */
  readonly automix: MockAutomix;
  /** Sync to a device (test hooks: plugged, fail, stepMs, reset). */
  readonly devices: MockDevices;
  /** The sound map (test hooks: limit, analysing). */
  readonly soundmap: MockSoundMap;
  /** Crates by label, producer, studio, era and genre (from the mock sleeve notes). */
  readonly crateIndex: MockCrates;
  private opts: MockOptions;
  private listeners = new Map<EventName, Set<(p: unknown) => void>>();
  private timers = new Set<ReturnType<typeof setTimeout>>();

  // auth
  private auth: AuthStatus;
  /** Plex Home users (test hooks: notAHome, switches). */
  readonly homeUsersMock = new MockHomeUsers();
  /** The account's servers and libraries (test hook: oneServer). */
  readonly librariesMock = new MockLibraries();
  /** Servers added by address (test hooks: failure, approveAfter, revoke, canTranscode, canDownload). */
  readonly jellyfin = new MockJellyfin();
  /** Navidrome and other Subsonic servers (test hooks: failure, canDownload, revoke, queues). */
  readonly subsonic = new MockSubsonic();
  /** The primary server's tag when one was chosen. */
  private primaryTag: string | null = null;
  // health / sync
  private healthState: Health;
  /** The offline "skipped songs" toast was shown (once per offline spell). */
  private offlineTold = false;
  /** Times the Local Network settings were asked for (tests). */
  localNetworkOpened = 0;
  private sync: SyncStatus = { phase: "idle", done: 0, total: 0, lastSyncAt: Math.floor(Date.now() / 1000) - 3600 };
  // search
  /** Items opened from search, newest first. */
  private recent: { kind: SearchKind; id: string }[] = [];
  private recentSeeded = false;
  private settings: Settings = { ...DEFAULT_SETTINGS };
  /** The themes folder (tests read what export wrote). */
  readonly themes = new MockThemes();
  // output devices (invented)
  private outputs: OutputDevice[] = MOCK_OUTPUTS.map((d) => ({ ...d }));
  /** Headphone correction A/B: off while comparing. */
  correctionBypassed = false;
  private outputSelected: string | null = null;
  /** Bytes in the (pretend) artwork cache. */
  private artUsed = 412 * 1024 * 1024;
  /** Outside artist info: keys entered in Settings, and artists already "fetched". */
  private infoKeys: MockInfoKeys = { keys: {}, rejected: new Set() };
  private infoFetched = new Set<string>();
  private albumCreditsFetched = new Set<string>();
  /** Albums whose sleeve notes lookups have "landed" (browser timelines only). */
  private sleeveFetched = new Set<string>();
  /** Artists whose global Popular list has "arrived" (browser timelines only). */
  private popularFetched = new Set<string>();

  // player
  private status: PlayerState["status"] = "idle";
  private positionMs = 0;
  private volume = 0.7;
  private muted = false;
  private shuffle = false;
  private repeat: RepeatMode = "off";
  /** A-B loop on the current track. */
  private abLoop: AbLoop | null = null;
  private current: QueueItem | null = null;
  private userQueue: QueueItem[] = [];
  private ctxOriginal: QueueItem[] = [];
  private ctxOrder: QueueItem[] = [];
  private ctxPos = -1;
  private history: QueueItem[] = [];
  /** Plays started from an album, artist or playlist (feeds Home). */
  private contextPlays: ContextPlay[] = [];
  /** The listen log: every play here (feeds Recents and Recently played). */
  private listens: MockListen[] = [];
  /** Listening stats over a demo history plus this session's plays. */
  readonly stats: MockStats;
  /** Friends' shared stats summaries (a demo friend). */
  private friends: CompareSummary[] = [];
  private ctxInfo: QueueContextInfo | null = null;
  /** Context ids with shuffle on, least recently used first. */
  private shuffledIds: string[] = [];
  /** Playlist and Liked Songs ids with smart shuffle on (each in shuffledIds too). */
  private smartIds: string[] = [];
  /** Smart shuffle suggestions were picked for this run of the context. */
  private smartBuilt = true;
  /** Resume point per context id; Map order is least recently used first. */
  private resumePoints = new Map<string, ResumePoint>();
  private itemSeq = 1;
  private tick: ReturnType<typeof setInterval> | null = null;
  /** Visualizer frames while watched and playing. */
  readonly visualizer = new MockVisualizer(
    (f) => this.emit("player://spectrum", f),
    () => this.status === "playing",
  );
  private rng = mulberry32(42);

  constructor(opts: Partial<MockOptions> = {}) {
    this.opts = { ...optionsFromUrl(""), ...opts };
    if (this.opts.longOutputs) this.outputs = LONG_OUTPUTS.map((d) => ({ ...d }));
    this.fx = shapeFixtures(buildFixtures(), this.opts);
    this.pl = new MockPlaylists(
      this.fx,
      (id) => this.trackById(id),
      () => {
        this.emit("library://changed", null);
        // Downloaded playlists follow their edits straight away, as in the core.
        this.downloads.resync();
      },
      (message) => this.emit("toast", { kind: "error", message }),
    );
    this.likes = new MockLikes(
      this.fx,
      () => this.settings.likedMeans,
      (ids) => {
        this.emit("likes://changed", { ids });
        this.downloads.resync();
      },
      (message) => this.emit("toast", { kind: "error", message }),
    );
    this.contextPlays = this.seedContextPlays();
    this.listens = seedListens(this.fx, this.likes.ids());
    this.stats = new MockStats(this.fx, this.opts.statsEmpty ? [] : seedStatsHistory(this.fx, Date.now(), this.opts.statsSparse), () => this.listens, () => this.settings);
    this.friends = this.opts.statsEmpty ? [] : [mockFriendSummary(this.fx, new Set(this.stats.plays().map((p) => p.trackId)))];
    this.share = new MockShare(
      this.fx,
      () => this.settings,
      (id) => this.fx.playlists.some((p) => p.id === id),
      () => this.emit("deeplink://open", null),
    );
    this.discord = new MockDiscord(this.opts.discord, (s) => this.emit("discord://status", s));
    this.plexEdit = new MockPlexEdit(
      this.fx,
      () => this.settings,
      () => this.later(0, () => this.emit("library://changed", null)),
    );
    this.tagEdit = new MockTagEdit(
      this.fx,
      () => this.settings,
      (patch) => {
        this.settings = { ...this.settings, ...patch };
        this.emit("settings://changed", { ...this.settings, volume: this.volume });
      },
      () => this.later(0, () => this.emit("tags://changed", null)),
    );
    this.doctor = new MockDoctor(this.fx, () => this.settings);
    this.tagEdit.seed = (id) => this.doctor.tagsOf(id);
    this.physical = new MockPhysical(this.fx, () => this.settings, () => this.later(0, () => this.emit("physical://changed", {})));
    this.sleeve = new MockSleeve(this.fx, () => this.settings, () => this.physical.status().saved);
    this.sides = new MockSides(this.fx);
    this.diary = new MockDiary(this.fx);
    this.automix = new MockAutomix(
      (id) => this.trackById(id) ?? undefined,
      () => this.later(0, () => this.emit("syncstore://changed", { namespaces: ["mixedPlaylists"], remote: false })),
    );
    this.soundmap = new MockSoundMap(this.fx, (a) =>
      mockUnreleased(a, this.settings.countUnmatchedUnreleased !== false, mockMbCheckOn(this.settings) && mockPlexMissed(this.fx.albums, a)),
    );
    this.crateIndex = new MockCrates(this.fx, this.sleeve, () => this.settings, (ids) => this.soundmap.hasAny(ids));
    this.devices = new MockDevices(
      this.fx,
      () => this.likes.ids(),
      (ms, fn) => this.later(ms, fn),
      (event, payload) => (event === "devices://progress" ? this.emit(event, payload!) : this.emit("devices://changed", null)),
    );
    this.tags = new MockFileTags(
      this.fx,
      () => this.settings,
      {
        emitProgress: (s) => this.emit("tags://progress", s),
        emitChanged: () => this.emit("tags://changed", null),
        later: (ms, fn) => this.later(ms, fn),
      },
      !this.opts.tagScan,
    );
    // The archive demo artist joins after the tags mock is built, with its own tags.
    this.archive = new MockArchive(this.fx, () => this.settings);
    this.tags.addBadges(this.archive.install());
    this.mixes = new MockMixes(this.fx, (list) => this.tags.forPlay(list).tracks);
    this.transcribe = new MockTranscribe({
      settings: () => this.settings,
      emitStatus: (s) => this.emit("transcribe://status", s),
      emitDone: (trackId, found) => this.emit("transcribe://done", { trackId, found }),
      later: (ms, fn) => this.later(ms, fn),
      title: (id) => this.trackById(id)?.title,
      transcribed: (id) => !!this.own.get(id)?.auto,
      hasLyrics: (id) => {
        const t = this.trackById(id);
        const own = this.own.get(id);
        return (!!own && !own.auto) || (!!t && !this.lyricsDown && !!mockLyrics(t));
      },
      save: (trackId, text, model) => {
        const own = this.own.get(trackId);
        if (own && !own.auto) return;
        this.own.set(trackId, { trackId, text, prefer: false, updatedAt: Math.floor(Date.now() / 1000), auto: model });
      },
      albumTracks: (albumId) => (this.fx.tracksByAlbum.get(albumId) ?? []).map((t) => t.id).filter((id) => !this.own.has(id)),
      unreleasedTracks: () =>
        this.fx.albums
          .filter((a) =>
            mockUnreleased(
              a,
              this.settings.countUnmatchedUnreleased !== false,
              mockMbCheckOn(this.settings) && mockPlexMissed(this.fx.albums, a),
            ),
          )
          .flatMap((a) => this.fx.tracksByAlbum.get(a.id) ?? [])
          .map((t) => t.id)
          .filter((id) => !this.own.has(id)),
    });
    this.lyricsShare = new MockLyricsShare({
      settings: () => this.settings,
      track: (id) => this.trackById(id),
      own: (id) => this.own.get(id),
      unreleased: (t) => {
        const a = this.fx.albums.find((x) => x.id === t.albumId);
        return (
          !!a && mockUnreleased(a, this.settings.countUnmatchedUnreleased !== false, mockMbCheckOn(this.settings) && mockPlexMissed(this.fx.albums, a))
        );
      },
      later: (ms, fn) => this.later(ms, fn),
    });
    this.downloads = new MockDownloads({
      settings: () => this.settings,
      emitStatus: (st) => this.emit("downloads://status", st),
      emitChanged: () => this.emit("downloads://changed", null),
      later: (ms, fn) => this.later(ms, fn),
      itemTracks: (kind, id) => {
        if (kind === "album") return this.fx.tracksByAlbum.get(id)?.map((t) => t.id) ?? null;
        if (kind === "playlist") return this.fx.playlists.some((p) => p.id === id) ? [...(this.fx.playlistTracks.get(id) ?? [])] : null;
        if (kind === "liked") return this.likes.ids();
        // A mix follows its mix as it is made again (a radio isn't kept).
        if (kind === "mix") return this.mixes.page(id)?.tracks.map((t) => t.id) ?? null;
        return null;
      },
      durationMs: (id) => this.trackById(id)?.durationMs ?? 200_000,
      offline: () => !!this.healthState?.cachedOnly,
    });
    if (this.opts.theme) this.settings = { ...this.settings, theme: this.opts.theme };
    if (this.opts.frame) this.settings = { ...this.settings, layout: { ...this.settings.layout, frame: this.opts.frame }, layoutOffered: true };
    if (this.opts.statsCompare) this.settings = { ...this.settings, statsCompare: true };
    this.importMock = new MockImport(this.fx, () => this.settings.importSources);
    this.account = new MockAccount(
      () => this.settings.importSources.includes("streaming-account"),
      () => this.importMock.accountRun(),
      () => this.importMock.accountLastRead(),
    );
    if (this.opts.importDemo) {
      this.settings = { ...this.settings, importSources: ["streaming", "transfer"] };
      this.importMock.run("streaming", "Downloads/my_data.zip");
      this.importMock.run("transfer", "Downloads/library.csv");
    }
    if (this.opts.lidarr === "ready" || this.opts.lidarr === "offline") this.settings = { ...this.settings, lidarrUrl: "http://192.168.x.x:8686" };
    if (this.opts.downloads) {
      this.downloads.seedDone("album", this.fx.albums[0].id, this.fx.albums[0].title);
      this.downloads.seedDone("liked", "", "Liked Songs");
      this.settings = { ...this.settings, downloadLiked: true };
      if (this.opts.downloads === "full") {
        this.settings = { ...this.settings, downloadLimitGb: 1 };
        this.downloads.usedOffset = 1024 ** 3;
      }
    }
    this.syncFile = new MockSync({
      settings: () => this.settings,
      setSettings: (patch) => {
        this.settings = { ...this.settings, ...patch };
        this.emit("settings://changed", { ...this.settings, volume: this.volume });
      },
      emitStatus: (s) => this.emit("sync-file://status", s),
      emitChanged: (c) => this.emit("syncstore://changed", c),
      emitKeys: (s) => this.emit("keys://receive", s),
    });
    this.deviceSync = new MockDeviceSync({
      settings: () => this.settings,
      setSettings: (patch) => {
        this.settings = { ...this.settings, ...patch };
        this.emit("settings://changed", { ...this.settings, volume: this.volume });
      },
      emitStatus: (s) => this.emit("device-sync://status", s),
      emitPair: (s) => this.emit("device-sync://pair", s),
      emitCopy: (p) => this.emit("device-sync://copy", p),
    });
    this.social = new MockSocial(this.fx, {
      settings: () => this.settings,
      emitFollows: (all) => this.emit("follows://changed", all),
      emitBadge: (unseen) => this.emit("whatsnew://changed", { unseen: this.settings.whatsNewAnnouncements ? unseen : 0 }),
      emitListen: (v) => this.emit("listen://state", v),
      later: (ms, fn) => this.later(ms, fn),
      demo: () => !!this.opts.timelines,
      queueAdd: (ids) => void this.queueAdd(ids, "end"),
      control: (op) => {
        if (op.op === "play") void this.act(() => this.resumeNow());
        else if (op.op === "pause") void this.act(() => this.pauseNow());
        else if (op.op === "next") void this.playNext();
        else if (op.op === "previous") void this.playPrevious();
        else if (op.op === "seek") void this.playSeek(op.ms);
      },
      shareWithGuest: (u) => {
        const s = this.settings;
        return this.sharing.share(u, { libraries: s.shareDefaultLibraries.length ? s.shareDefaultLibraries : ["3"], hideUnreleased: s.shareHideUnreleased, allowDownloads: s.shareAllowDownloads });
      },
      hostShared: (host) => this.sharing.inviteArrives(host),
    });
    this.sharing = new MockSharing({
      settings: () => this.settings,
      emitInvites: (all) => this.emit("sharing://invites", all),
      emitNotice: (message) => this.emit("sharing://notice", { message }),
      later: (ms, fn) => this.later(ms, fn),
      accepted: () => this.social.hostCopyHere(),
    });
    this.circle = new MockCircle(this.fx, {
      settings: () => this.settings,
      // Before sign-in state is read (the first reset): the mock user.
      me: () => ({ name: this.auth?.username ?? "listener", avatar: this.auth?.avatar ?? null }),
      people: () => [
        ...this.homeUsersMock
          .list()
          .users.filter((u) => !u.current)
          .map((u) => ({ key: `home:${u.uuid}`, name: u.name, avatar: u.avatar, home: true })),
        { key: "user:robin", name: "Robin", avatar: "/ext/https://plex.tv/users/mock1/avatar?c=1", home: false },
        { key: "user:jules", name: "Jules", avatar: "/ext/https://plex.tv/users/mock2/avatar?c=1", home: false },
      ],
      emit: (v) => this.emit("circle://changed", v),
      notice: (message) => this.emit("circle://notice", { message }),
      later: (ms, fn) => this.later(ms, fn),
      demo: () => !!this.opts.timelines,
      track: (id) => this.trackById(id),
      tracks: () => this.fx.tracks,
      playlist: (id) => this.fx.playlists.find((p) => p.id === id),
      itemIds: (id) => this.pl.itemIds(id),
      trackIds: (id) => this.fx.playlistTracks.get(id) ?? [],
      add: (id, ids) => void this.pl.add(id, ids),
    });
    this.hidden = new MockHidden((all) => this.emit("hidden://changed", all));
    this.bookmarks = new MockBookmarks(
      {
        emit: (all) => this.emit("bookmarks://changed", all),
        playAt: (id, ms) => {
          this.loadContext({ context: { kind: "tracks", trackIds: [id] }, startTrackId: id }, true);
          this.positionMs = ms;
          this.emitState();
        },
      },
      this.fx,
    );
    this.smart = new MockSmart(this.fx, {
      settings: () => this.settings,
      emit: (all) => this.emit("smart://changed", all),
      libraryChanged: () => this.later(0, () => this.emit("library://changed", null)),
      track: (id) => this.trackById(id),
      album: (id) => this.fx.albums.find((a) => a.id === id),
    });
    this.lidarr = new MockLidarr(this.opts.lidarr);
    this.lidarrMock = new MockLidarrWork(
      () => Math.floor(Date.now() / 1000),
      () => this.settings.lidarrPracticeUntil,
    );
    this.upgrades = new MockUpgrades({
      album: (id) => this.fx.albums.find((a) => a.id === id),
      albums: () => this.fx.albums,
      tracks: (id) => this.fx.tracksByAlbum.get(id) ?? [],
      settings: () => this.settings,
      lidarrReady: () => this.lidarr.status(this.settings).ready,
      now: () => Math.floor(Date.now() / 1000),
    });
    if (this.opts.upgrades) {
      this.settings = { ...this.settings, lidarrStagingFolder: MOCK_STAGING };
      this.upgrades.reset(this.opts.upgrades);
    }
    this.radar = new MockRadar(
      this.fx,
      {
        settings: () => this.settings,
        setSettings: (patch) => {
          this.settings = { ...this.settings, ...patch };
          this.emit("settings://changed", { ...this.settings, volume: this.volume });
        },
        requests: () => this.lidarr.list(),
        lidarrReady: () => this.lidarr.status(this.settings).ready,
        emit: () => this.emit("radar://changed", null),
      },
      this.opts.radarEmpty,
    );
    this.catalogue = new MockCatalogue(this.fx, {
      settings: () => this.settings,
      offline: () => !!this.healthState?.cachedOnly,
      requests: () => this.lidarr.list(),
      lidarrSetUp: () => {
        const st = this.lidarr.status(this.settings);
        return (st.urlSet || st.awaySet) && st.hasKey;
      },
    });
    this.scrobble = new MockScrobble((queued) => this.emit("scrobble://status", { queued }));
    this.playHistory = new MockPlayHistory(
      () => this.settings,
      () => this.scrobble.status().lastfm.signedIn,
    );
    const link = this.opts.link;
    if (link) this.later(1500, () => this.share.openLink(link));
    this.remote = new MockRemote(
      {
        local: () => this.snapshot(),
        pauseLocal: () => this.pauseNow(),
        resumeAt: (id, ms, play) => this.resumeAt(id, ms, play),
        emitState: (st) => this.emit("remote://state", st),
        emitPlayers: (p) => this.emit("remote://players", p),
        toast: (message) => this.emit("toast", { kind: "error", message }),
        settings: () => this.settings,
        later: (ms, fn) => this.later(ms, fn),
      },
      this.fx.tracks[7],
      this.opts.platterUnreachable,
      this.opts.platterPhone,
    );
    this.auth = this.opts.signedOut
      ? { state: "signedOut", needsServerPick: false }
      : this.opts.health === "signIn"
        ? { state: "signedOut", username: "listener", avatar: AVATAR, needsServerPick: false, reauthRequired: true }
        : this.opts.serverPick
          ? { state: "signedIn", username: "listener", avatar: AVATAR, needsServerPick: true }
          : { state: "signedIn", username: "listener", avatar: AVATAR, server: SERVER, needsServerPick: false };
    const startStatus = this.opts.health ?? (this.opts.timelines ? "starting" : "local");
    this.healthState = this.makeHealth(startStatus);

    // Something in the player bar from the start, paused (like a restored session).
    const firstAlbum = this.fx.albums[0];
    this.loadContext({ context: { kind: "album", id: firstAlbum.id } }, false);
    this.positionMs = 62_000;

    if (this.opts.timelines) this.runStartup();
    if (this.opts.listenDemo) this.startListenDemo(this.opts.listenDemo);
  }

  /** ?listen=cantHear: hosting, Robin can't hear the song. ?listen=invited: a guest with the host's invite waiting. */
  private startListenDemo(kind: "cantHear" | "invited") {
    if (kind === "cantHear") {
      this.social.start();
      this.social.guestJoins("Robin");
      this.social.guestJoins("Casey");
      this.social.guestCantHear("Robin", true);
      return;
    }
    this.social.join("platter://join/AV8MKp57HUw4puL5CxfT5EXMOOHwS3stlqDkyLHT9XopaPBRgA");
    this.social.songNotHere("Basement Tapes No. 4");
    this.sharing.inviteArrives(this.social.listenState().hostName ?? "Jules");
  }

  // ---------------------------------------------------------------- infra

  private delay<T>(fn: () => T): Promise<T> {
    const ms = this.opts.latencyMs;
    return new Promise((resolve, reject) => {
      const run = () => {
        try {
          resolve(clone(fn()));
        } catch (e) {
          reject(e);
        }
      };
      if (ms <= 0) queueMicrotask(run);
      else this.later(ms * (0.6 + this.rng() * 0.8), run);
    });
  }

  private later(ms: number, fn: () => void) {
    const t = setTimeout(() => {
      this.timers.delete(t);
      fn();
    }, ms);
    this.timers.add(t);
  }

  emit<E extends EventName>(event: E, payload: EventMap[E]) {
    this.listeners.get(event)?.forEach((cb) => cb(clone(payload)));
  }

  listen<E extends EventName>(event: E, cb: (payload: EventMap[E]) => void): Promise<Unlisten> {
    let set = this.listeners.get(event);
    if (!set) this.listeners.set(event, (set = new Set()));
    // Each subscription its own entry: the same callback listened twice
    // (React's double effects) and unlistened once still hears events.
    const fn = (p: unknown) => (cb as (p: unknown) => void)(p);
    set.add(fn);
    return Promise.resolve(() => set!.delete(fn));
  }

  artUrl(path: string, w: number, h: number): string {
    if (this.opts.httpArt) return `http://art.localhost/${encodeURIComponent(path)}?w=${Math.round(w)}&h=${Math.round(h)}`;
    return mockArtUrl(path);
  }

  /** Stop all timers (tests). */
  dispose() {
    this.timers.forEach(clearTimeout);
    this.timers.clear();
    this.stopTick();
    this.visualizer.stop();
    this.remote.dispose();
    this.listeners.clear();
  }

  private notFound(what: string): never {
    throw { kind: "notFound", message: `${what} not found` };
  }

  // ---------------------------------------------------------------- startup timeline

  private runStartup() {
    this.later(700, () => this.setHealth(this.opts.health ?? "local"));
    const h = this.opts.health;
    if (h && h !== "local" && h !== "remote" && h !== "relay" && h !== "scanning") return; // nothing to sync from
    // incremental sync
    const total = 340;
    let done = 0;
    this.later(900, () => {
      this.sync = { ...this.sync, phase: "incremental", done: 0, total };
      this.emit("sync://progress", this.sync);
      const step = () => {
        done = Math.min(total, done + 34);
        this.sync = { ...this.sync, done };
        this.emit("sync://progress", this.sync);
        if (done < total) this.later(250, step);
        else
          this.later(200, () => {
            this.sync = { phase: "idle", done: 0, total: 0, lastSyncAt: Math.floor(Date.now() / 1000) };
            this.emit("sync://progress", this.sync);
            this.emit("library://changed", null);
          });
      };
      this.later(250, step);
    });
  }

  private makeHealth(demo: HealthDemo): Health {
    return demoHealth(demo, { name: SERVER.name, version: SERVER.version ?? "" });
  }

  /** Test/demo hook: change the app state (emits health://changed, and a
   *  "back" toast when leaving offline, like the core). */
  setHealth(demo: HealthDemo, patch: Partial<Health> = {}) {
    const wasOffline = this.healthState.banner?.kind === "offline" || this.healthState.banner?.kind === "noInternet";
    this.healthState = { ...this.makeHealth(demo), ...patch };
    this.emit("health://changed", this.healthState);
    this.downloads?.poke();
    const s = this.healthState.status;
    if (wasOffline && (s === "local" || s === "remote" || s === "relay")) this.emit("toast", { kind: "info", message: "Your server is back." });
  }

  /** Test/demo hook: Plex rejected the saved sign-in (expired or revoked). */
  expireSignIn() {
    this.auth = { state: "signedOut", username: "listener", avatar: AVATAR, needsServerPick: false, reauthRequired: true };
    this.emit("auth://changed", this.auth);
    this.setHealth("signIn");
  }

  /** Mock: the first two albums are on this computer (pre-cached), and whatever is downloaded. */
  private cachedIds(): string[] {
    const albums = new Set(this.fx.albums.slice(0, 2).map((a) => a.id));
    const ids = new Set(this.fx.tracks.filter((t) => albums.has(t.albumId)).map((t) => t.id));
    for (const id of this.downloads.downloadedIds()) ids.add(id);
    return [...ids];
  }
  cachedTracks() {
    return this.delay(() => this.cachedIds());
  }
  cachedLibrary() {
    return this.delay((): CachedLibrary => {
      const ids = new Set(this.cachedIds());
      const tracks = this.fx.tracks.filter((t) => ids.has(t.id));
      const albumIds = new Set(tracks.map((t) => t.albumId));
      return {
        albums: this.fx.albums.filter((a) => albumIds.has(a.id)),
        artistIds: [...new Set(tracks.map((t) => t.artistId))],
        playlistIds: this.fx.playlists
          .filter((p) => (this.fx.playlistTracks.get(p.id) ?? []).some((id) => ids.has(id)))
          .map((p) => p.id),
      };
    });
  }
  /** The core's offline play rule; null while everything can play. */
  private keepPlayable(tracks: Track[], start: string | null): { tracks: Track[]; start: string | null } | null {
    if (!this.healthState.cachedOnly) {
      this.offlineTold = false;
      return null;
    }
    const ok = new Set(this.cachedIds());
    const kept = tracks.filter((t) => ok.has(t.id));
    const leftOut = tracks.length - kept.length;
    let from = start;
    if (start && !ok.has(start)) {
      const at = Math.max(0, tracks.findIndex((t) => t.id === start));
      from = [...tracks.slice(at), ...tracks.slice(0, at)].find((t) => ok.has(t.id))?.id ?? null;
    }
    if (kept.length && leftOut > 0 && !this.offlineTold) {
      this.offlineTold = true;
      this.emit("toast", { kind: "info", message: offlineLeftOutMessage(leftOut) });
    }
    return { tracks: kept, start: from };
  }
  /** Test/demo hook: macOS refusing the home network (see setHealth). */
  openLocalNetworkSettings() {
    return this.delay(() => {
      this.localNetworkOpened += 1;
    });
  }

  // ---------------------------------------------------------------- auth

  authStatus() {
    return this.delay(() => this.auth);
  }
  authStart(): Promise<PinInfo> {
    this.auth = { state: "pending", needsServerPick: false };
    this.emit("auth://changed", this.auth);
    if (this.opts.autoApproveMs != null) this.later(this.opts.autoApproveMs, () => this.approvePin());
    return this.delay(() => ({
      code: "K7QX",
      authUrl: "https://app.plex.tv/auth#?code=K7QX",
      expiresAt: Math.floor(Date.now() / 1000) + 600,
    }));
  }
  /** Test/demo hook: the user approved the PIN in their browser. */
  approvePin() {
    if (this.auth.state !== "pending") return;
    this.auth = { state: "signedIn", username: "listener", avatar: AVATAR, server: SERVER, needsServerPick: false };
    this.emit("auth://changed", this.auth);
  }
  authCancel() {
    this.auth = { state: "signedOut", needsServerPick: false };
    this.emit("auth://changed", this.auth);
    return this.delay(() => undefined);
  }
  authSignOut() {
    this.auth = { state: "signedOut", needsServerPick: false };
    this.emit("auth://changed", this.auth);
    return this.delay(() => undefined);
  }
  homeUsers() {
    return this.delay(() => this.homeUsersMock.list());
  }
  homeSwitch(uuid: string, pin?: string) {
    return this.delay(() => {
      const was = this.homeUsersMock.current().uuid;
      const to = this.homeUsersMock.switchTo(uuid, pin);
      if (to.uuid === was) return;
      // The app restarts as them; here the switching screen stays up for
      // about as long as a restart takes, then the name and picture change.
      this.emit("home://changed", { restarting: true });
      const done = () => {
        this.auth = { ...this.auth, username: to.name, avatar: to.avatar };
        this.emit("auth://changed", this.auth);
        this.emit("home://changed", { restarting: false });
        this.emit("toast", { kind: "info", message: `Switched to ${to.name}` });
      };
      if (this.opts.latencyMs <= 0) done();
      else this.later(1500, done);
    });
  }
  homeRememberSet(on: boolean) {
    return this.delay(() => {
      this.homeUsersMock.remember = on;
    });
  }
  backendCaps() {
    return this.delay(() => {
      const plexIn = this.auth.state === "signedIn";
      const base = mockBackendCaps(this.auth.server?.name ?? null);
      const primary = this.savedServers().find((s) => s.primary)?.tag ?? null;
      const servers = [...(plexIn ? base.servers : []), ...this.jellyfin.caps(), ...this.subsonic.caps()].map((s) => ({ ...s, primary: s.tag === primary }));
      const caps = { ...base.caps };
      // What any server in use can do, as the core reports it.
      for (const k of Object.keys(caps) as (keyof typeof caps)[]) caps[k] = (plexIn && caps[k]) || servers.some((s) => s.caps[k]);
      return { caps, servers };
    });
  }
  /** Plex (when signed in) and the servers added by address, the primary marked. */
  private savedServers() {
    const plexIn = this.auth.state === "signedIn";
    const list = [...(plexIn ? [plexRecord(this.auth.server?.name ?? "Plex server", false)] : []), ...this.jellyfin.servers, ...this.subsonic.servers];
    const primary = list.find((s) => s.tag === this.primaryTag)?.tag ?? list[0]?.tag ?? null;
    return list.map((s) => ({ ...s, primary: s.tag === primary }));
  }
  private serversChanged() {
    this.emit("backends://changed", {});
  }
  serversSaved() {
    return this.delay(() => this.savedServers());
  }
  serverSetPrimary(tag: string) {
    return this.delay(() => {
      if (!this.savedServers().some((s) => s.tag === tag)) throw { kind: "notFound", message: "That server isn't in Platter any more." };
      this.primaryTag = tag;
      this.serversChanged();
    });
  }
  serverRemove(tag: string) {
    return this.delay(() => {
      if (tag === "0a1b2c3d") throw { kind: "invalid", message: "Sign out of Plex in Settings, Account." };
      if (this.subsonic.owns(tag)) this.subsonic.remove(tag);
      else this.jellyfin.remove(tag);
      if (this.primaryTag === tag) this.primaryTag = null;
      this.serversChanged();
    });
  }
  serverLibraries(tag: string) {
    return this.delay(() => {
      const libs = this.jellyfin.libraries[tag] ?? this.subsonic.libraries[tag];
      if (!libs) throw { kind: "notFound", message: "That server isn't in Platter any more." };
      return libs;
    });
  }
  serverLibrariesSet(tag: string, ids: string[] | null) {
    return this.delay(() => {
      if (this.subsonic.owns(tag)) this.subsonic.setLibraries(tag, ids);
      else this.jellyfin.setLibraries(tag, ids);
      this.serversChanged();
    });
  }
  jellyfinCheck(url: string) {
    return this.delay(() => this.jellyfin.check(url));
  }
  jellyfinQuickStart(url: string) {
    return this.delay(() => this.jellyfin.quickStart(url));
  }
  jellyfinQuickPoll() {
    return this.delay(() => {
      const r = this.jellyfin.quickPoll();
      if (r.state === "done") this.serversChanged();
      return r;
    });
  }
  jellyfinQuickCancel() {
    return this.delay(() => this.jellyfin.quickCancel());
  }
  jellyfinSignInPassword(url: string, user: string, password: string) {
    return this.delay(() => {
      const s = this.jellyfin.password(url, user, password);
      this.serversChanged();
      return s;
    });
  }
  subsonicCheck(url: string) {
    return this.delay(() => this.subsonic.check(url));
  }
  subsonicSignIn(url: string, creds: SubsonicCreds, tag?: string, confirm?: SubsonicConfirm) {
    return this.delay(() => {
      const s = this.subsonic.signIn(url, creds, tag, confirm);
      this.serversChanged();
      return s;
    });
  }
  subsonicChangeAddress(tag: string, url: string, creds: SubsonicCreds, confirm?: SubsonicConfirm) {
    return this.delay(() => {
      const s = this.subsonic.changeAddress(tag, url, creds, confirm);
      this.serversChanged();
      return s;
    });
  }
  resumeQueues() {
    return this.delay(() => this.subsonic.offers());
  }
  resumeQueuePlay(tag: string) {
    return this.delay(() => this.subsonic.play(tag));
  }
  resumeQueueSave() {
    return this.delay(() => this.subsonic.servers.length > 0);
  }
  /** Test hook: the primary choice and servers added by address forgotten. */
  resetServers() {
    this.jellyfin.reset();
    this.subsonic.reset();
    this.primaryTag = null;
  }
  librarySources() {
    return this.delay(() => this.librariesMock.sources(this.settings.libraries));
  }
  libraryServers() {
    return this.delay(() => this.librariesMock.servers(this.settings.libraries, this.settings.serverView));
  }
  serversList() {
    return this.delay(() => [SERVER, { ...SERVER, machineId: "mock-machine-02", name: "Cabin NAS", owned: false, online: false, connection: null }]);
  }
  serverSelect(machineId: string) {
    return this.delay(() => {
      if (machineId !== SERVER.machineId) throw { kind: "invalid", message: "Server is offline" };
      this.auth = { ...this.auth, server: SERVER, needsServerPick: false };
      this.emit("auth://changed", this.auth);
    });
  }

  // ---------------------------------------------------------------- library

  syncStatus() {
    return this.delay(() => this.sync);
  }
  syncNow(full?: boolean) {
    return this.delay(() => {
      if (full && this.sync.phase === "idle") this.runSync(["artists", "albums", "tracks"]);
    });
  }

  /** Simulated sync: each phase counts up, then idle + library://changed. */
  private runSync(phases: SyncStatus["phase"][]) {
    const [phase, ...rest] = phases;
    if (!phase) {
      this.sync = { phase: "idle", done: 0, total: 0, lastSyncAt: Math.floor(Date.now() / 1000) };
      this.emit("sync://progress", this.sync);
      this.emit("library://changed", null);
      return;
    }
    const total = phase === "artists" ? this.fx.artists.length : phase === "albums" ? this.fx.albums.length : this.fx.tracks.length;
    const step = Math.max(1, Math.ceil(total / 6));
    let done = 0;
    const tick = () => {
      this.sync = { ...this.sync, phase, done, total };
      this.emit("sync://progress", this.sync);
      if (done >= total) this.later(200, () => this.runSync(rest));
      else {
        done = Math.min(total, done + step);
        this.later(250, tick);
      }
    };
    tick();
  }
  /**
   * The server added an album (and maybe its artist) while the app is open:
   * it lands in the library and pages hear `library://changed`, as a live
   * update does in the core.
   */
  libraryAdded(album: Album, artist?: Artist) {
    if (artist && !this.fx.artists.some((a) => a.id === artist.id)) this.fx.artists.push(artist);
    this.fx.albums.push(album);
    if (!this.fx.tracksByAlbum.has(album.id)) this.fx.tracksByAlbum.set(album.id, []);
    this.emit("library://changed", null);
  }
  home() {
    return this.delay(() => new MockHome(this.fx, this.contextPlays, this.listens).home());
  }
  homeShelf(shelf: HomeShelfId, limit?: number) {
    return this.delay(() => new MockHome(this.fx, this.contextPlays, this.listens).shelf(shelf, limit));
  }
  /** Invented history: a few playlist and artist plays around the newest album play. */
  private seedContextPlays(): ContextPlay[] {
    const newest = Math.max(0, ...this.fx.albums.map((a) => a.lastViewedAt ?? 0));
    const pl = this.fx.playlists;
    const ar = this.fx.artists;
    const out: ContextPlay[] = [];
    if (pl[0]) out.push({ kind: "playlist", id: pl[0].id, at: newest + 3600 }, { kind: "playlist", id: pl[0].id, at: newest - 5 * 86400 });
    if (ar[3]) out.push({ kind: "artist", id: ar[3].id, at: newest - 1800 });
    if (pl[2]) out.push({ kind: "playlist", id: pl[2].id, at: newest - 2 * 86400 });
    if (pl[4]) out.push({ kind: "playlist", id: pl[4].id, at: newest - 9 * 86400 });
    return out;
  }
  artists(args: { sort: ArtistSort; offset: number; limit: number }) {
    return this.delay(() => {
      const list = [...this.fx.artists];
      if (args.sort === "name") list.sort((a, b) => a.sortTitle.localeCompare(b.sortTitle));
      if (args.sort === "plays") list.sort((a, b) => b.viewCount - a.viewCount);
      return { items: list.slice(args.offset, args.offset + args.limit), total: list.length };
    });
  }
  artistNames() {
    return this.delay(() => [
      ...this.fx.artists.map((a) => ({ id: a.id, title: a.title })),
      ...LEARNED_NAMES.map((n) => ({ id: creditId(n), title: n })),
    ]);
  }
  private allAlbums(): Album[] {
    const base = this.fx.albums;
    const n = this.opts.albumCount;
    if (!n || n <= base.length) return base;
    const out = [...base];
    for (let i = base.length; i < n; i++) {
      const src = base[i % base.length];
      out.push({
        ...src,
        title: `${src.title} (${Math.floor(i / base.length) + 1})`,
        sortTitle: `${src.sortTitle} ${i}`,
        thumb: `${src.thumb}/v${i}`,
        addedAt: src.addedAt - i * 3600,
      });
    }
    return out;
  }
  albums(args: { sort: AlbumSort; offset: number; limit: number; section?: ReleaseSection }) {
    return this.delay(() => {
      let list = this.tags.visibleAlbums(this.allAlbums());
      if (args.section) list = list.filter((a) => a.section === args.section);
      list = [...list];
      const by: Record<AlbumSort, (a: Album, b: Album) => number> = {
        name: (a, b) => a.sortTitle.localeCompare(b.sortTitle),
        artist: (a, b) => a.artistTitle.localeCompare(b.artistTitle) || (a.year ?? 0) - (b.year ?? 0),
        year: (a, b) => (b.year ?? 0) - (a.year ?? 0),
        recent: (a, b) => b.addedAt - a.addedAt,
        plays: (a, b) => b.viewCount - a.viewCount,
      };
      list.sort(by[args.sort]);
      return { items: list.slice(args.offset, args.offset + args.limit), total: list.length };
    });
  }
  playlists() {
    return this.delay(() => this.fx.playlists);
  }
  libraryItems() {
    return this.delay(() => {
      const albums = this.allAlbums();
      const byArtist = new Map<string, { added: number | null; played: number | null }>();
      // The large-library demo repeats albums; keep their ids unique here.
      const items: LibraryItem[] = albums.map((a, i) => {
        const d = byArtist.get(a.artistId) ?? { added: null, played: null };
        d.added = d.added === null ? a.addedAt : Math.min(d.added, a.addedAt);
        d.played = Math.max(d.played ?? 0, a.lastViewedAt ?? 0) || null;
        byArtist.set(a.artistId, d);
        return {
          kind: "album",
          id: i < this.fx.albums.length ? a.id : `${a.id}-${i}`,
          title: a.title,
          sortTitle: a.sortTitle,
          creator: a.artistTitle,
          creatorId: a.artistId,
          thumb: a.thumb,
          smart: false,
          format: a.format,
          addedAt: a.addedAt,
          playedAt: a.lastViewedAt,
          tint: a.tint ?? null,
          trackCount: a.trackCount,
          year: a.year,
          section: a.section,
          quality: albumQuality(this.fx.tracksByAlbum.get(a.id) ?? []),
        };
      });
      for (const a of this.fx.artists) {
        const d = byArtist.get(a.id);
        items.push({ kind: "artist", id: a.id, title: a.title, sortTitle: a.sortTitle, thumb: a.thumb, smart: false, addedAt: d?.added ?? null, playedAt: d?.played ?? null });
      }
      for (const p of this.fx.playlists) {
        items.push({ kind: "playlist", id: p.id, title: p.title, sortTitle: p.title, thumb: p.thumb, smart: p.smart, addedAt: null, playedAt: null });
      }
      return items;
    });
  }
  /** Library names plus learned names with separators, as the core sends them. */
  private nameLookup() {
    return buildArtistLookup([...this.fx.artists, ...LEARNED_NAMES.map((n) => ({ id: creditId(n), title: n }))]);
  }
  /** Like the core: Popular leaves out hidden songs, and hidden artists' songs on other pages. */
  private popularVisible(t: Track, pageId: string) {
    return !this.hidden.hides({ id: t.id, artistId: "" }) && (t.artistId === pageId || !this.hidden.hides({ id: "", artistId: t.artistId }));
  }
  /** Global ranking only while a source for it is on (the core's rule). */
  private popularSource() {
    const s = this.settings;
    return s.infoListenbrainz || s.infoLastfm ? s.popularSource : "myPlays";
  }
  artistPage(id: string) {
    return this.delay(() => {
      const lookup = this.nameLookup();
      if (isCreditId(id)) return this.creditedPage(id, lookup);
      const artist = this.fx.artists.find((a) => a.id === id) ?? this.notFound("Artist");
      const popular = this.archive.annotate(
        popularTracks(this.fx.tracks.filter((t) => this.popularVisible(t, id)), artist, lookup, this.popularSource()),
      );
      const albums = this.tags.visibleAlbums(this.fx.albums.filter((a) => a.artistId === id));
      // Sections by release type and status, then the user's own (archive).
      const sections = this.archive.sections(albums);
      const eras = this.archive.eras(id);
      const plays = this.fx.tracks.filter((t) => t.artistId === id).reduce((n, t) => n + t.viewCount, 0);
      const idx = this.fx.artists.indexOf(artist);
      // Most artists have a biography; every third has none. Similar artists
      // are the next two in the library; the last few have none.
      const about = this.plexBio(idx, artist.title);
      const similar = idx >= this.fx.artists.length - 3 ? [] : [1, 2].map((d) => this.fx.artists[(idx + d) % this.fx.artists.length]);
      const appearsOn = this.albumsOf(this.fx.tracks.filter((t) => t.artistId !== id && credits(t, artist.title, lookup)));
      // In the browser a first open waits for the global list, like the core.
      const popularPending = !!this.opts.timelines && this.popularSource() === "global" && !this.popularFetched.has(id);
      // A few of the artist's top songs the library doesn't have, placed among its own.
      const popularElsewhere =
        this.popularSource() === "global"
          ? [
              { title: "Night Drive", release: `${artist.title} Live`, after: 1 },
              { title: "Paper Lanterns", release: null, after: 3 },
            ]
          : [];
      return { artist, popular, sections, plays, about, similar, appearsOn, popularPending, popularElsewhere, eras };
    });
  }
  /** Featured names with no library artist, for search (while "Link featured artists" is on). */
  private creditedArtists(): Artist[] {
    if (!this.settings.linkFeatured) return [];
    const lookup = this.nameLookup();
    const byKey = new Map<string, Artist>();
    for (const t of this.fx.tracks) {
      if (!t.trackArtist) continue;
      for (const c of splitCredits(t.trackArtist, lookup)) {
        const key = creditKey(c.name);
        if (this.fx.artists.some((a) => creditKey(a.title) === key)) continue;
        const a = byKey.get(key) ?? { id: creditId(c.name), title: c.name, sortTitle: c.name, thumb: null, art: null, viewCount: 0 };
        a.viewCount += t.viewCount;
        byKey.set(key, a);
      }
    }
    return [...byKey.values()];
  }
  private albumsOf(tracks: Track[]): Album[] {
    const ids = [...new Set(tracks.map((t) => t.albumId))];
    return this.fx.albums.filter((a) => ids.includes(a.id)).sort((a, b) => (b.year ?? -1) - (a.year ?? -1) || b.addedAt - a.addedAt);
  }
  /** A featured name's generated page: the songs that credit it. */
  private creditedPage(id: string, lookup: ReturnType<typeof buildArtistLookup>): ArtistPage {
    const key = id.slice(CREDIT_PREFIX.length);
    const tracks = key ? this.fx.tracks.filter((t) => credits(t, key, lookup)) : [];
    if (!tracks.length) return this.notFound("Artist");
    const title = splitCredits(tracks[0].trackArtist!, lookup).find((c) => creditKey(c.name) === key)?.name ?? key;
    const plays = tracks.reduce((n, t) => n + t.viewCount, 0);
    const shown = tracks.filter((t) => this.popularVisible(t, id));
    const popular = rankPopular(shown, this.popularSource() === "global" ? globalTop(shown) : null);
    const artist: Artist = { id, title, sortTitle: title, thumb: null, art: null, viewCount: plays };
    return { artist, popular, sections: [], plays, about: null, similar: [], appearsOn: this.albumsOf(tracks) };
  }
  albumPage(id: string) {
    return this.delay(() => {
      const album = this.fx.albums.find((a) => a.id === id) ?? this.notFound("Album");
      const tracks = this.archive.annotate(this.tags.albumTracks(this.fx.tracksByAlbum.get(id) ?? []));
      const editions = new Set(this.tags.index().groups[this.tags.index().albums[id]?.group ?? ""]?.map((e) => e.albumId) ?? []);
      // Plex has a release date for most albums and a label for some.
      const n = Number(id) || 0;
      const released = album.year ? `${album.year}-${String((n % 12) + 1).padStart(2, "0")}-${String((n % 27) + 1).padStart(2, "0")}` : null;
      // Albums Plex couldn't match have no label.
      const studio = n % 3 === 0 || !mockMatched(album) ? null : MOCK_LABELS[n % MOCK_LABELS.length];
      return {
        album,
        tracks,
        totalDurationMs: tracks.reduce((s, t) => s + t.durationMs, 0),
        artistThumb: this.fx.artists.find((a) => a.id === album.artistId)?.thumb ?? null,
        released,
        studio,
        archive: this.archive.albumArchive(album),
        moreByArtist: this.tags
          .visibleAlbums(this.fx.albums)
          .filter((a) => a.artistId === album.artistId && a.id !== id && !editions.has(a.id)),
      };
    });
  }
  /** The listen log: what played here, newest first, repeats kept. */
  recentTracks(limit = 50) {
    return this.delay(() =>
      [...this.listens]
        .sort((a, b) => b.at - a.at)
        .map((l) => this.trackById(l.trackId))
        .filter((t): t is Track => !!t)
        .slice(0, limit),
    );
  }
  recents(limit?: number) {
    return this.delay(() => buildRecents(this.fx, this.listens, this.likes.likedTracks(), (id) => this.trackById(id), Date.now(), limit));
  }
  playlistPage(id: string) {
    return this.delay(() => {
      const playlist = this.fx.playlists.find((p) => p.id === id) ?? this.notFound("Playlist");
      const ids = this.fx.playlistTracks.get(id) ?? [];
      const itemIds = this.pl.itemIds(id);
      const rows = ids.map((tid, i) => ({ track: this.trackById(tid), itemId: itemIds[i] ?? `it-${id}-${i}` })).filter((r) => r.track);
      return { playlist, tracks: this.archive.annotate(rows.map((r) => r.track!)), itemIds: rows.map((r) => r.itemId) };
    });
  }

  // ---------------------------------------------------------------- playlist editing

  playlistCreate(title?: string, trackIds: string[] = []) {
    return this.delay(() => this.pl.create(this.settings.playlistNewName, title, trackIds));
  }
  playlistAdd(id: string, trackIds: string[], place?: AddPlace) {
    return this.delay(() => this.pl.add(id, trackIds, place));
  }
  playlistDuplicates(id: string, trackIds: string[]) {
    return this.delay(() => this.pl.duplicates(id, trackIds));
  }
  playlistRemove(id: string, itemIds: string[]) {
    return this.delay(() => this.pl.remove(id, itemIds));
  }
  playlistMove(id: string, itemIds: string[], after?: string | null) {
    return this.delay(() => this.pl.move(id, itemIds, after));
  }
  playlistUpdate(id: string, details: { title?: string; summary?: string }) {
    return this.delay(() => this.pl.update(id, details));
  }
  playlistSetCover(id: string, bytes: Uint8Array, contentType: string) {
    return this.delay(() => this.pl.setCover(id, bytes, contentType));
  }
  playlistDelete(id: string) {
    return this.delay(() => {
      this.pl.delete(id);
      this.circle.playlistDeleted(id);
    });
  }

  // ---------------------------------------------------------------- edit in Plex

  tagEditPickMusicFolder() {
    return this.delay(() => this.tagEdit.setFolder("/Volumes/Music"));
  }
  tagEditSetMusicFolder(path: string) {
    return this.delay(() => this.tagEdit.setFolder(path));
  }
  tagEditClearMusicFolder() {
    return this.delay(() => {
      this.settings = { ...this.settings, musicFolder: "", musicFolderServer: "" };
      this.emit("settings://changed", { ...this.settings, volume: this.volume });
    });
  }
  tagEditMusicFolderStatus() {
    return this.delay(() => this.tagEdit.status());
  }
  tagEditOpen(albumId: string | null, trackIds: string[]) {
    return this.delay(() => this.tagEdit.open(albumId, trackIds));
  }
  tagEditSave(request: TagSaveRequest) {
    return this.delay(() => {
      const r = this.tagEdit.save(request);
      if (r.saved > 0) {
        const albums = new Set(request.files.map((f) => this.fx.tracks.find((t) => t.id === f.trackId)?.albumId).filter((x): x is string => !!x));
        let any = false;
        for (const id of albums) any = this.doctor.resolveTags(id, this.tagEdit.open(id, []).files) || any;
        if (any) this.emitDoctor();
      }
      return r;
    });
  }
  tagEditUndo(id: string) {
    return this.delay(() => this.tagEdit.undo(id));
  }
  tagEditHistory() {
    return this.delay(() => this.tagEdit.history());
  }
  tagEditCheck(scope: TagCheckScope) {
    return this.delay(() => this.tagEdit.check(scope));
  }
  tagEditCheckFine(id: string, fine: boolean) {
    return this.delay(() => this.tagEdit.checkFine(id, fine));
  }
  tagEditFixPreview(kind: TagFixKind, albumId: string) {
    return this.delay(() => this.tagEdit.fixPreview(kind, albumId));
  }
  tagEditIsrcReleases(albumId: string) {
    return this.delay(() => this.tagEdit.isrcReleases(albumId));
  }
  tagEditIsrcPreview(albumId: string, release: IsrcRelease) {
    return this.delay(() => this.tagEdit.isrcPreview(albumId, release));
  }
  tagEditExplicitPreview(albumIds: string[], explicit: boolean, era?: { artistId: string; era: string } | null) {
    return this.delay(() => this.tagEdit.explicitPreview(albumIds, explicit, era));
  }
  tagEditLibrarySettings() {
    return this.delay(() => this.tagEdit.librarySettings());
  }
  plexEditAccess() {
    return this.delay(() => this.plexEdit.access());
  }
  plexEditItem(id: string) {
    return this.delay(() => this.plexEdit.item(id));
  }
  plexEditImages(id: string, which: ImageKind) {
    return this.delay(() => this.plexEdit.images(id, which));
  }
  plexEditSelectImage(id: string, kind: EditKind, which: ImageKind, key: string) {
    return this.delay(() => this.doctorFixed(this.plexEdit.selectImage(id, kind, which, key), id, which === "poster" ? "cover" : null));
  }
  plexEditUploadImage(id: string, kind: EditKind, which: ImageKind, bytes: Uint8Array, contentType: string) {
    return this.delay(() => this.doctorFixed(this.plexEdit.uploadImage(id, kind, which, bytes, contentType), id, which === "poster" ? "cover" : null));
  }
  plexEditImageLink(id: string, kind: EditKind, which: ImageKind, url: string) {
    return this.delay(() => this.doctorFixed(this.plexEdit.imageLink(id, kind, which, url), id, which === "poster" ? "cover" : null));
  }
  /** A fix from the library doctor ran: its finding goes away. */
  private doctorFixed<T>(out: T, albumId: string, fix: DoctorFinding["fix"] | null): T {
    if (fix && this.doctor.resolve(albumId, fix)) this.emitDoctor();
    return out;
  }
  private emitDoctor() {
    this.later(0, () => this.emit("doctor://changed", { count: this.doctor.count(), running: this.doctor.running }));
  }
  plexEditMatches(id: string, title?: string, year?: number | null) {
    return this.delay(() => this.plexEdit.matches(id, title, year));
  }
  plexEditMatch(id: string, kind: EditKind, candidate: MatchCandidate) {
    return this.delay(() => this.doctorFixed(this.plexEdit.match(id, kind, candidate), id, "match"));
  }
  plexEditUnmatch(id: string, kind: EditKind) {
    return this.delay(() => this.plexEdit.unmatch(id, kind));
  }
  plexEditDetails(id: string, fields: EditFields) {
    return this.delay(() => this.plexEdit.details(id, fields));
  }
  plexEditMerge(target: string, others: string[], photoFrom?: string | null) {
    return this.delay(() => this.plexEdit.merge(target, others, photoFrom));
  }
  plexEditMergeAlbums(target: string, others: string[]) {
    return this.delay(() => this.doctorFixed(this.plexEdit.mergeAlbums(target, others), target, "mergeAlbums"));
  }
  plexEditSplit(id: string) {
    return this.delay(() => this.plexEdit.split(id));
  }
  plexEditRefresh(id: string, kind: EditKind) {
    return this.delay(() => this.plexEdit.refresh(id, kind));
  }
  plexEditUndo(changeId: string) {
    return this.delay(() => this.plexEdit.undo(changeId));
  }
  plexEditHistory(id: string) {
    return this.delay(() => this.plexEdit.history(id));
  }

  // ---------------------------------------------------------------- likes

  likesSet(trackIds: string[], liked: boolean) {
    return this.delay(() => this.likes.set(trackIds, liked));
  }
  likedIds() {
    return this.delay(() => this.likes.ids());
  }
  likedPage() {
    return this.delay(() => this.likes.page());
  }
  playlistsWithTrack(trackId: string) {
    return this.delay(() => this.likes.playlistsWith(trackId));
  }

  nowPlayingDetails(trackId: string) {
    return this.delay(() => {
      const track = this.trackById(trackId) ?? this.notFound("Track");
      return mockNowPlayingDetails(track, this.fx.artists.find((a) => a.id === track.artistId));
    });
  }

  /** Lyrics sources have nothing for any song (the view says there are none). */
  lyricsDown = false;
  /** Lyrics sources can't be reached (app states: "Couldn't get lyrics", with Retry). */
  lyricsUnreachable = false;
  /** Songs the sources call instrumentals. */
  lyricsInstrumental = new Set<string>();
  /** Songs "Look again" was asked for. */
  lookedAgain: string[] = [];
  lyricsLookAgain(trackId: string) {
    return this.delay(() => {
      this.lookedAgain.push(trackId);
    });
  }

  /** Lyrics written in the editor, by track id (the core keeps them in SQLite). */
  own = new Map<string, OwnLyrics>();
  /** Files "saved" by Export .lrc (null result: the save dialog was cancelled). */
  exported: { text: string; fileName: string }[] = [];
  exportCancels = false;

  /** Lyrics choices per song (the core keeps them in SQLite and the sync file). */
  lyricsChoices = new Map<string, { pick?: string; offsetMs: number }>();
  /** More lyrics found for a song (LRCLIB hits), for tests and screenshots. */
  lyricsExtra = new Map<string, MockLyricsCandidate[]>();
  /** What the check against the song said, per song and candidate key. */
  lyricsVerdicts = new Map<string, Record<string, LyricsVerdict>>();

  /** Plex's lyrics, then the user's own, unless they asked for theirs first. */
  private autoLyrics(track: Track): Lyrics | null {
    let own = this.own.get(track.id);
    // Transcriptions hide when turned off, and never go ahead of Plex's.
    if (own?.auto && !this.settings.showAutoLyrics) own = undefined;
    const mine = own ? lyricsFromText(own.text) : null;
    if (mine && own?.auto) mine.source = "auto";
    if (mine && !own?.auto && (own!.prefer || this.settings.lyricsPreferOwn)) return { ...mine, pick: "user" };
    const plex = this.lyricsDown || this.lyricsUnreachable ? null : mockLyrics(track);
    const extra = this.lyricsDown || this.lyricsUnreachable ? [] : (this.lyricsExtra.get(track.id) ?? []);
    const verdicts = this.lyricsVerdicts.get(track.id) ?? {};
    const found = [...(plex ? [{ key: "server", lyrics: plex }] : []), ...extra];
    // The first that matches the song, else the first not ruled out.
    const pick = found.find((c) => verdicts[c.key] === "match" || verdicts[c.key] === "matchOffset") ?? found.find((c) => verdicts[c.key] !== "noMatch");
    if (!pick) {
      if (mine) return { ...mine, pick: own?.auto ? "auto" : "user" };
      return found.length ? { synced: false, lines: [], notice: "noneMatched" } : null;
    }
    const v = verdicts[pick.key];
    const checked = v === "match" || v === "matchOffset";
    const others = found.filter((c) => c !== pick && verdicts[c.key] !== "noMatch");
    const disagree = !checked && others.some((c) => (c.lyrics.lines.join(" ") !== pick.lyrics.lines.join(" ")));
    const shown: Lyrics = { ...pick.lyrics, pick: pick.key, check: checked ? "checked" : "notChecked", ...(disagree ? { disagree } : {}) };
    // A live take with lyrics found by artist and title: the timing is another version's.
    const album = this.fx.albums.find((a) => a.id === track.albumId);
    const live = !!album && (album.subformats.includes("live") || /\(live|- live|unplugged/i.test(track.title));
    if (!shown.synced || !live || checked) return shown;
    if (this.settings.lyricsLiveTiming) return { ...shown, borrowedTiming: true };
    // A transcription of this recording beats borrowed timing.
    if (own?.auto && mine?.synced) return { ...mine, pick: "auto" };
    return { ...shown, synced: false, timed: undefined, borrowedTiming: true };
  }

  /** Every set of lyrics found for a song, with its key. */
  private lyricsFound(track: Track): { key: string; lyrics: Lyrics; c: Omit<LyricsCandidate, "key" | "preview" | "synced" | "source"> }[] {
    const out: { key: string; lyrics: Lyrics; c: Omit<LyricsCandidate, "key" | "preview" | "synced" | "source"> }[] = [];
    const plex = this.lyricsDown || this.lyricsUnreachable ? null : mockLyrics(track);
    if (plex) out.push({ key: "server", lyrics: plex, c: { origin: "server", album: track.albumTitle, durationMs: track.durationMs } });
    const own = this.own.get(track.id);
    const mine = own ? lyricsFromText(own.text) : null;
    if (own && mine) out.push({ key: own.auto ? "auto" : "user", lyrics: { ...mine, source: own.auto ? "auto" : "user" }, c: { origin: own.auto ? "auto" : "user" } });
    for (const e of this.lyricsExtra.get(track.id) ?? []) out.push({ key: e.key, lyrics: e.lyrics, c: { origin: e.origin, album: e.album, durationMs: e.durationMs, otherVersion: e.otherVersion } });
    return out;
  }

  lyrics(trackId: string) {
    return this.delay((): Lyrics | null => {
      const track = this.trackById(trackId) ?? this.notFound("Track");
      const choice = this.lyricsChoices.get(trackId);
      if (choice?.pick === "none") return { synced: false, lines: [], notice: "chosenNone" };
      let l: Lyrics | null = null;
      const picked = choice?.pick ? this.lyricsFound(track).find((f) => f.key === choice.pick) : undefined;
      if (picked) {
        const v = this.lyricsVerdicts.get(trackId)?.[picked.key];
        l = { ...picked.lyrics, pick: picked.key, chosen: true, ...(v === "match" || v === "matchOffset" ? { check: "checked" as const } : {}) };
      } else {
        l = this.autoLyrics(track);
      }
      if (!l && this.lyricsUnreachable) return { synced: false, lines: [], notice: "unavailable" };
      if (!l && this.lyricsInstrumental.has(trackId)) return { synced: false, lines: [], notice: "instrumental" };
      const offset = choice?.offsetMs ?? 0;
      if (l && offset && l.synced && l.timed) {
        const move = (t: number) => Math.max(0, t + offset);
        l = {
          ...l,
          offsetMs: offset,
          timed: l.timed.map((t) => ({ ...t, startMs: move(t.startMs), words: t.words?.map((w) => ({ ...w, startMs: move(w.startMs) })) })),
        };
      }
      return l;
    });
  }
  /** Check statuses the tests set. */
  checkStatuses = new Map<string, LyricsCheckStatus>();
  lyricsCheckStatus(trackId: string) {
    return this.delay(() => this.checkStatuses.get(trackId) ?? null);
  }
  lyricsCandidates(trackId: string) {
    return this.delay((): LyricsCandidates => {
      const track = this.trackById(trackId) ?? this.notFound("Track");
      const shown = this.autoLyrics(track)?.pick;
      const choice = this.lyricsChoices.get(trackId);
      const verdicts = this.lyricsVerdicts.get(trackId) ?? {};
      const status = this.transcribe.status();
      const items = this.lyricsFound(track).map(({ key, lyrics, c }) => ({
        key,
        ...c,
        source: lyrics.source ?? "plex",
        synced: lyrics.synced,
        ...(verdicts[key] ? { check: verdicts[key] } : {}),
        preview: (lyrics.timed?.map((t) => t.text) ?? lyrics.lines).filter(Boolean).slice(0, 3),
        ...(key === (choice?.pick ?? shown) ? { shown: true } : {}),
      }));
      return {
        items,
        ...(choice?.pick ? { pick: choice.pick } : {}),
        offsetMs: choice?.offsetMs ?? 0,
        canCheck: this.settings.lyricsCheckAudio && status.available && status.models.some((m) => m.installed),
      };
    });
  }
  lyricsChoose(trackId: string, pick: string | null) {
    return this.delay(() => {
      this.trackById(trackId) ?? this.notFound("Track");
      const cur = this.lyricsChoices.get(trackId);
      const offsetMs = cur && (cur.pick ?? null) === pick ? cur.offsetMs : 0;
      if (!pick && !offsetMs) this.lyricsChoices.delete(trackId);
      else this.lyricsChoices.set(trackId, { ...(pick ? { pick } : {}), offsetMs });
    });
  }
  lyricsOffset(trackId: string, offsetMs: number) {
    return this.delay(() => {
      this.trackById(trackId) ?? this.notFound("Track");
      if (Math.abs(offsetMs) > 60_000) throw { kind: "invalid", message: "That offset is too far." };
      const cur = this.lyricsChoices.get(trackId);
      const next = { ...(cur?.pick ? { pick: cur.pick } : {}), offsetMs: Math.round(offsetMs / 100) * 100 };
      if (!next.pick && !next.offsetMs) this.lyricsChoices.delete(trackId);
      else this.lyricsChoices.set(trackId, next);
    });
  }
  lyricsChoicesCount() {
    return this.delay(() => this.lyricsChoices.size);
  }
  lyricsChoicesClear() {
    return this.delay(() => {
      const n = this.lyricsChoices.size;
      this.lyricsChoices.clear();
      return n;
    });
  }
  ownLyrics(trackId: string) {
    return this.delay(() => this.own.get(trackId) ?? null);
  }
  ownLyricsSave(trackId: string, text: string, prefer: boolean) {
    return this.delay(() => {
      this.trackById(trackId) ?? this.notFound("Track");
      const clean = text.replace(/\r\n?/g, "\n").trim();
      if (!lyricsFromText(clean)) throw { kind: "invalid", message: "There are no lyrics to save." };
      this.own.set(trackId, { trackId, text: clean, prefer, updatedAt: Math.floor(Date.now() / 1000) });
    });
  }
  ownLyricsDelete(trackId: string) {
    return this.delay(() => this.own.delete(trackId));
  }
  ownLyricsIds() {
    return this.delay(() => [...this.own.keys()].sort());
  }
  /** Makes the mock translation service unreachable. */
  translateFails = false;
  lyricsPronounce(lines: string[]) {
    // A stand-in: lines outside the Latin range get "~" plus the line.
    return this.delay(() => lines.map((l) => (/[^\u0000-ɏ\s]/.test(l) ? `~${l}` : null)));
  }
  lyricsTranslate(lines: string[]) {
    return this.delay(() => {
      if (!this.settings.translationService) throw { kind: "invalid", message: "Set a translation service in Settings first." };
      if (this.translateFails) throw { kind: "network", message: "Couldn't reach the translation service." };
      return lines.map((l) => (l.trim() ? `[${this.settings.translateTo}] ${l}` : ""));
    });
  }
  lyricsExport(text: string, fileName: string) {
    return this.delay(() => {
      if (this.exportCancels) return null;
      const name = fileName.replace(/[<>:"/\\|?*]/g, "_").trim() + ".lrc";
      this.exported.push({ text, fileName: name });
      return name;
    });
  }

  /** A play for the listen log, credited to the context it came from. */
  private logListen(track: Track) {
    if (this.compareHeld) return; // Compare plays never count
    const c = this.ctxInfo;
    const kind: MockListen["kind"] =
      c && (c.kind === "album" || c.kind === "artist" || c.kind === "playlist") && c.id ? c.kind : c?.kind === "liked" ? "liked" : "songs";
    const at = Date.now();
    this.listens.push({ trackId: track.id, at, end: at + track.durationMs, kind, id: kind === "liked" || kind === "songs" ? undefined : c?.id ?? undefined });
  }

  private trackById(id: string): Track | undefined {
    return this.fx.tracks.find((t) => t.id === id);
  }

  // ---------------------------------------------------------------- search

  search(args: SearchArgs) {
    const started = performance.now();
    return this.delay((): SearchResults => {
      const q = args.query.trim().toLowerCase();
      const limit = args.limit ?? 20;
      const empty: SearchResults = { top: null, songs: [], artists: [], albums: [], playlists: [], tookMs: 0, source: "local" };
      if (!q) return empty;
      const score = (s: string) => {
        const l = s.toLowerCase();
        if (l === q) return 3;
        if (l.startsWith(q)) return 2;
        if (l.split(/\s+/).some((w) => w.startsWith(q))) return 1.5;
        return l.includes(q) ? 1 : 0;
      };
      const rank = <T,>(items: T[], key: (t: T) => string, pop: (t: T) => number) =>
        items
          .map((t) => ({ t, s: score(key(t)) }))
          .filter((x) => x.s > 0)
          .sort((a, b) => b.s - a.s || pop(b.t) - pop(a.t))
          .map((x) => x.t);
      const f = args.filter;
      const artists = f === "all" || f === "artists" ? rank([...this.fx.artists, ...this.creditedArtists()], (a) => a.title, (a) => a.viewCount) : [];
      const albums = f === "all" || f === "albums" ? rank(this.fx.albums, (a) => a.title, (a) => a.viewCount) : [];
      const songs =
        f === "all" || f === "songs"
          ? rank(this.fx.tracks, (t) => `${t.title}`, (t) => t.viewCount).concat(
              rank(this.fx.tracks, (t) => t.artistTitle, (t) => t.viewCount).filter((t) => score(t.title) === 0),
            )
          : [];
      const playlists = f === "all" || f === "playlists" ? rank(this.fx.playlists, (p) => p.title, () => 0) : [];
      let top: SearchResults["top"] = null;
      const cands: [number, NonNullable<SearchResults["top"]>][] = [];
      if (artists[0]) cands.push([score(artists[0].title) + 0.3, { kind: "artist", artist: artists[0] }]);
      if (albums[0]) cands.push([score(albums[0].title) + 0.2, { kind: "album", album: albums[0] }]);
      if (songs[0] && score(songs[0].title) > 0) cands.push([score(songs[0].title), { kind: "track", track: songs[0] }]);
      if (playlists[0]) cands.push([score(playlists[0].title) + 0.1, { kind: "playlist", playlist: playlists[0] }]);
      if (cands.length) top = cands.sort((a, b) => b[0] - a[0])[0][1];
      return this.archive.groupSearch({
        top,
        songs: songs.slice(0, limit),
        artists: artists.slice(0, limit),
        albums: albums.slice(0, limit),
        playlists: playlists.slice(0, limit),
        tookMs: Math.round((performance.now() - started) * 10) / 10,
        source: "local",
      });
    });
  }
  searchCompletions(query: string, limit = 4) {
    return this.delay(() => {
      const fold = (s: string) =>
        s
          .normalize("NFD")
          .replace(/\p{M}/gu, "")
          .toLowerCase()
          .split(/[^\p{L}\p{N}]+/u)
          .filter(Boolean)
          .join(" ");
      const q = fold(query.slice(0, 200));
      if (!q) return [] as string[];
      const cands: [string, number][] = [
        ...this.fx.artists.map((a): [string, number] => [a.title, a.viewCount]),
        ...this.fx.albums.map((a): [string, number] => [a.title, a.viewCount]),
        ...this.fx.tracks.map((t): [string, number] => [t.title, t.viewCount]),
      ];
      cands.sort((a, b) => b[1] - a[1] || [...a[0]].length - [...b[0]].length);
      const seen = new Set<string>();
      const out: string[] = [];
      for (const [title] of cands) {
        const n = fold(title);
        if (!n.startsWith(q) || seen.has(n)) continue;
        seen.add(n);
        out.push(title);
        if (out.length >= Math.min(limit, 10)) break;
      }
      return out;
    });
  }
  /** A few remembered items so the dropdown is demoable (not in tests). */
  private seedRecents() {
    if (this.recentSeeded) return;
    this.recentSeeded = true;
    if (isTestEnv) return;
    const fx = this.fx;
    this.recent = [
      fx.albums[0] && { kind: "album" as const, id: fx.albums[0].id },
      fx.artists[1] && { kind: "artist" as const, id: fx.artists[1].id },
      fx.tracks[3] && { kind: "track" as const, id: fx.tracks[3].id },
      fx.playlists[0] && { kind: "playlist" as const, id: fx.playlists[0].id },
    ].filter((x): x is { kind: SearchKind; id: string } => !!x);
  }
  searchRecents() {
    return this.delay(() => {
      this.seedRecents();
      const out: SearchHit[] = [];
      for (const r of this.recent) {
        if (r.kind === "artist") {
          const artist = this.fx.artists.find((a) => a.id === r.id);
          if (artist) out.push({ kind: "artist", artist });
        } else if (r.kind === "album") {
          const album = this.fx.albums.find((a) => a.id === r.id);
          if (album) out.push({ kind: "album", album });
        } else if (r.kind === "track") {
          const track = this.trackById(r.id);
          if (track) out.push({ kind: "track", track });
        } else {
          const playlist = this.fx.playlists.find((p) => p.id === r.id);
          if (playlist) out.push({ kind: "playlist", playlist });
        }
      }
      return clone(out);
    });
  }
  searchRecentAdd(kind: SearchKind, id: string) {
    return this.delay(() => {
      this.seedRecents();
      if (!id) return;
      this.recent = [{ kind, id }, ...this.recent.filter((r) => !(r.kind === kind && r.id === id))].slice(0, 20);
    });
  }
  searchRecentRemove(kind: SearchKind, id: string) {
    return this.delay(() => {
      this.seedRecents();
      this.recent = this.recent.filter((r) => !(r.kind === kind && r.id === id));
    });
  }
  searchRecentsClear() {
    return this.delay(() => {
      this.recentSeeded = true;
      this.recent = [];
    });
  }

  // ---------------------------------------------------------------- playback

  private item(track: Track, source: QueueItem["source"]): QueueItem {
    return { itemId: `q${this.itemSeq++}`, track, source };
  }

  /** Plain random order (Fisher-Yates). */
  private shuffled<T>(arr: T[]): T[] {
    const a = [...arr];
    for (let i = a.length - 1; i > 0; i--) {
      const j = Math.floor(this.rng() * (i + 1));
      [a[i], a[j]] = [a[j], a[i]];
    }
    return a;
  }

  /** Spread: each artist's tracks evenly spaced, random offset and jitter
   *  (the core's Spread style, minus its recent-play weighting). */
  private spread(items: QueueItem[]): QueueItem[] {
    const groups = new Map<string, QueueItem[]>();
    for (const it of items) {
      const k = (it.track.trackArtist ?? it.track.artistTitle).toLowerCase();
      let g = groups.get(k);
      if (!g) groups.set(k, (g = []));
      g.push(it);
    }
    const placed: [number, QueueItem][] = [];
    for (const g of groups.values()) {
      const n = g.length;
      const offset = this.rng() / n;
      this.shuffled(g).forEach((it, j) => placed.push([offset + j / n + ((this.rng() - 0.5) * 0.2) / n, it]));
    }
    return placed.sort((a, b) => a[0] - b[0]).map((p) => p[1]);
  }

  private shuffleOrder(items: QueueItem[]): QueueItem[] {
    return this.settings.shuffleStyle === "standard" ? this.shuffled(items) : this.spread(items);
  }

  private shufflePref(id: string): boolean {
    return this.shuffledIds.includes(id);
  }

  private storeShufflePref(id: string, on: boolean) {
    this.shuffledIds = this.shuffledIds.filter((s) => s !== id);
    if (on) this.shuffledIds = [...this.shuffledIds, id].slice(-CONTEXT_MEMORY_CAP);
    else this.smartIds = this.smartIds.filter((s) => s !== id);
  }

  private storeSmartPref(id: string, on: boolean) {
    this.smartIds = this.smartIds.filter((s) => s !== id);
    if (on) {
      if (!this.shufflePref(id)) this.storeShufflePref(id, true);
      this.smartIds = [...this.smartIds, id].slice(-CONTEXT_MEMORY_CAP);
    }
  }

  /** Like the core: on for a shuffled playlist or Liked Songs with its preference set. */
  private smartOn(): boolean {
    const c = this.ctxInfo;
    return (
      this.settings.smartShuffle &&
      this.shuffle &&
      !!c?.id &&
      (c.kind === "playlist" || c.kind === "liked") &&
      this.smartIds.includes(c.id)
    );
  }

  /**
   * Like the core: one suggestion after every two playlist songs (counting
   * the playing one), never after the last; built once per run.
   */
  private buildSmart(cur: QueueItem | null = this.current) {
    if (this.smartBuilt || !this.smartOn()) return;
    this.smartBuilt = true;
    this.dropSmart();
    const rest = this.ctxOrder.slice(this.ctxPos + 1);
    let left = rest.filter((i) => i.source === "context").length;
    let count = cur?.source === "context" ? 1 : 0;
    const slots = new Set<number>();
    rest.forEach((it, k) => {
      if (it.source !== "context") return;
      count++;
      left--;
      if (count >= 2 && left > 0) {
        slots.add(k);
        count = 0;
      }
    });
    if (!slots.size) return;
    const own = this.ctxOriginal.filter((i) => i.source === "context");
    const artists = new Set(own.map((i) => i.track.artistId));
    const avoid = new Set([...this.ctxOriginal, ...this.history, ...this.userQueue].map((i) => i.track.id));
    const seeds = own.filter((_, k) => k % Math.max(1, Math.floor(own.length / 30)) === 0).map((i) => i.track.id);
    const found = this.mixes
      .similar(seeds, avoid, 400, `smart${this.itemSeq}`)
      .filter((t) => !artists.has(t.artistId) && !this.hidden.hides(t));
    // Songs that aren't liked first; a few per artist while there are others.
    const liked = new Set(this.likes.ids());
    const ordered = [...found.filter((t) => !liked.has(t.id)), ...found.filter((t) => liked.has(t.id))];
    const per = new Map<string, number>();
    const picks: Track[] = [];
    for (const t of ordered) {
      if (picks.length >= slots.size) break;
      const n = per.get(t.artistId) ?? 0;
      if (n >= 3) continue;
      per.set(t.artistId, n + 1);
      picks.push(t);
    }
    for (const t of ordered) {
      if (picks.length >= slots.size) break;
      if (!picks.includes(t)) picks.push(t);
    }
    const out: QueueItem[] = [];
    rest.forEach((it, k) => {
      out.push(it);
      if (slots.has(k) && picks.length) {
        const added = this.item(picks.shift()!, "smart");
        this.ctxOriginal.push(added);
        out.push(added);
      }
    });
    this.ctxOrder = [...this.ctxOrder.slice(0, this.ctxPos + 1), ...out];
  }

  /** Remember where the current context is (track change, pause, ticks). */
  private notePosition() {
    if (this.compareHeld) return;
    const id = this.ctxInfo?.id;
    const cur = this.current;
    if (!id || !cur) return;
    let rp: ResumePoint | null = null;
    if (cur.source === "context") {
      const at = this.ctxOrder[this.ctxPos];
      if (at?.itemId === cur.itemId) rp = { index: this.ctxOriginal.indexOf(at), trackId: at.track.id, positionMs: this.positionMs };
    } else {
      // A queued song is playing: the context picks up at its next track.
      const nxt = this.ctxOrder[this.ctxPos + 1];
      if (nxt) rp = { index: this.ctxOriginal.indexOf(nxt), trackId: nxt.track.id, positionMs: 0 };
    }
    if (!rp) return;
    this.resumePoints.delete(id);
    this.resumePoints.set(id, rp);
    while (this.resumePoints.size > CONTEXT_MEMORY_CAP) this.resumePoints.delete(this.resumePoints.keys().next().value!);
  }

  private resumeStart(id: string): { index: number; positionMs: number } | null {
    const rp = this.resumePoints.get(id);
    if (!rp) return null;
    if (this.ctxOriginal[rp.index]?.track.id === rp.trackId) return { index: rp.index, positionMs: rp.positionMs };
    const i = this.ctxOriginal.findIndex((it) => it.track.id === rp.trackId);
    return i >= 0 ? { index: i, positionMs: rp.positionMs } : null;
  }

  private contextTracks(args: PlayContextArgs): { tracks: Track[]; info: QueueContextInfo } {
    const c = args.context;
    switch (c.kind) {
      case "album": {
        const album = this.fx.albums.find((a) => a.id === c.id) ?? this.notFound("Album");
        // One side of the record: those songs, in album order.
        const all = this.fx.tracksByAlbum.get(album.id) ?? [];
        const tracks = c.trackIds ? all.filter((t) => c.trackIds!.includes(t.id)) : all;
        return { tracks, info: { kind: "album", id: album.id, title: album.title } };
      }
      case "playlist": {
        const pl = this.fx.playlists.find((p) => p.id === c.id) ?? this.notFound("Playlist");
        const ids = this.fx.playlistTracks.get(pl.id) ?? [];
        return { tracks: ids.map((i) => this.trackById(i)!).filter(Boolean), info: { kind: "playlist", id: pl.id, title: pl.title } };
      }
      case "liked":
        return { tracks: c.trackIds ? c.trackIds.map((i) => this.trackById(i)!).filter(Boolean) : this.likes.likedTracks(), info: { kind: "liked", id: "liked", title: "Liked Songs" } };
      case "artist": {
        if (c.id && isCreditId(c.id)) {
          const lookup = this.nameLookup();
          const key = c.id.slice(CREDIT_PREFIX.length);
          const tracks = this.fx.tracks.filter((t) => credits(t, key, lookup));
          if (!tracks.length) this.notFound("Artist");
          const title = splitCredits(tracks[0].trackArtist!, lookup).find((x) => creditKey(x.name) === key)?.name ?? key;
          return { tracks, info: { kind: "artist", id: c.id, title } };
        }
        const ar = this.fx.artists.find((a) => a.id === c.id) ?? this.notFound("Artist");
        const tracks = this.fx.tracks.filter((t) => t.artistId === ar.id).sort((a, b) => b.viewCount - a.viewCount);
        return { tracks, info: { kind: "artist", id: ar.id, title: ar.title } };
      }
      case "mix": {
        const page = this.mixes.page(c.id ?? "") ?? this.notFound("Mix");
        const tracks = c.trackIds ? c.trackIds.map((i) => this.trackById(i)!).filter(Boolean) : page.tracks;
        return { tracks, info: { kind: "mix", id: page.mix.id, title: page.mix.title } };
      }
      case "radio": {
        const page = c.trackIds ? null : this.mixes.radio(c.id ?? "");
        if (!c.trackIds && !page) this.notFound("Radio");
        const [k, rid] = (c.id ?? "").split(":");
        const name = k === "track" ? this.trackById(rid)?.title : this.fx.artists.find((a) => a.id === rid)?.title;
        const tracks = c.trackIds ? c.trackIds.map((i) => this.trackById(i)!).filter(Boolean) : page!.tracks;
        return { tracks, info: { kind: "radio", id: c.id ?? null, title: `${name ?? "Song"} Radio` } };
      }
      default: {
        const tracks = (c.trackIds ?? []).map((i) => this.trackById(i)!).filter(Boolean);
        return { tracks, info: { kind: c.kind, id: c.id ?? null, title: c.kind === "search" ? "Search results" : "Tracks" } };
      }
    }
  }

  private loadContext(args: PlayContextArgs, play: boolean) {
    const resolved = this.contextTracks(args);
    const info = resolved.info;
    // Like the core: the clicked row picks which copy of a song listed twice starts.
    let startCopy = 0;
    const at = args.startIndex;
    if (at !== undefined && at < resolved.tracks.length && (!args.startTrackId || resolved.tracks[at].id === args.startTrackId)) {
      const sid = resolved.tracks[at].id;
      args = { ...args, startTrackId: sid };
      startCopy = resolved.tracks.slice(0, at).filter((t) => t.id === sid).length;
    }
    // Like the core: explicit songs (when filtered) and hidden edits leave, with one toast.
    const forPlay = this.tags.forPlay(resolved.tracks);
    const { message } = forPlay;
    let tracks = forPlay.tracks;
    if (!tracks.length) {
      if (message) this.emit("toast", { kind: "error", message: "Nothing here can play: every song is left out by your explicit content or fan edit settings." });
      return;
    }
    // Like the core: while the server is out of reach, only songs on this
    // computer play; a start song that can't moves to the next that can.
    const offline = this.keepPlayable(tracks, args.startTrackId ?? null);
    if (offline) {
      if (!offline.tracks.length) {
        this.emit("toast", { kind: "error", message: OFFLINE_NOTHING_PLAYS });
        return;
      }
      tracks = offline.tracks;
      if (offline.start !== args.startTrackId) startCopy = 0;
      args = { ...args, startTrackId: offline.start ?? undefined };
    }
    if (message && play) this.emit("toast", { kind: "info", message });
    // Like the core: shuffled play (asked for or the stored choice) leaves out hidden songs and artists, but not the picked one.
    if (args.shuffle ?? (info.id ? this.shufflePref(info.id) : false)) {
      const kept = tracks.filter((t) => t.id === args.startTrackId || !this.hidden.hides(t));
      if (kept.length) tracks = kept;
    }
    this.notePosition(); // where the outgoing context was left
    const id = info.id ?? null;
    const kind = args.context.kind;
    if (play && id && !this.compareHeld && (kind === "album" || kind === "artist" || kind === "playlist"))
      this.contextPlays.push({ kind, id, at: Math.floor(Date.now() / 1000) });
    if (args.shuffle !== undefined) {
      this.shuffle = args.shuffle;
      if (id) this.storeShufflePref(id, args.shuffle);
    } else {
      this.shuffle = id ? this.shufflePref(id) : false;
    }
    if (this.current) this.pushHistory(this.current);
    this.ctxOriginal = tracks.map((t) => this.item(t, "context"));
    this.ctxInfo = info;
    const copies = args.startTrackId ? this.ctxOriginal.filter((i) => i.track.id === args.startTrackId) : [];
    let first = copies[startCopy] ?? copies[0];
    let offset = 0;
    if (!first && this.shuffle) first = this.ctxOriginal[Math.floor(this.rng() * this.ctxOriginal.length)];
    if (!first) {
      const r = id && this.settings.resumeContexts ? this.resumeStart(id) : null;
      first = this.ctxOriginal[r?.index ?? 0];
      offset = r?.positionMs ?? 0;
    }
    if (this.shuffle) {
      this.ctxOrder = [first, ...this.shuffleOrder(this.ctxOriginal.filter((i) => i !== first))];
      this.ctxPos = 0;
    } else {
      this.ctxOrder = [...this.ctxOriginal];
      this.ctxPos = this.ctxOrder.indexOf(first);
    }
    // A new start picks new suggestions.
    this.smartBuilt = false;
    this.buildSmart(this.ctxOrder[this.ctxPos]);
    this.setCurrent(this.ctxOrder[this.ctxPos], play, offset);
  }

  /** Like the core's refill: more songs at the end of the context. */
  private refill(): boolean {
    const avoid = new Set([...this.ctxOrder, ...this.history, ...this.userQueue].map((i) => i.track.id));
    const radio = this.ctxInfo?.kind === "radio";
    const seeds = this.ctxOrder.slice(Math.max(0, this.ctxPos - 9), this.ctxPos + 1).map((i) => i.track.id);
    const more = this.mixes.similar(seeds, avoid, 20, `${this.itemSeq}`).filter((t) => !this.hidden.hides(t));
    if (!more.length) return false;
    const items = more.map((t) => this.item(t, radio ? "context" : "autoplay"));
    this.ctxOriginal.push(...items);
    this.ctxOrder.push(...items);
    return true;
  }

  private ctxKey(): string | null {
    return this.ctxInfo ? `${this.ctxInfo.kind}|${this.ctxInfo.id ?? ""}` : null;
  }

  private pushHistory(item: QueueItem) {
    this.history.push(item);
    if (this.history.length > 50) this.history.shift();
  }

  private setCurrent(item: QueueItem | null, play: boolean, startMs = 0) {
    // Like the core: a loop belongs to one track.
    if (item !== this.current) this.abLoop = null;
    const started = !!item && play && item !== this.current;
    if (started) this.logListen(item.track);
    this.current = item;
    this.positionMs = startMs;
    if (item && item.track.title.endsWith(BROKEN_TRACK_TITLE_SUFFIX) && play) {
      // Spec 07: one file missing/corrupt → toast + skip.
      // The UI turns player://error into the "Skipped" toast.
      this.emit("player://error", { trackId: item.track.id, kind: "io", message: "Skipped: file unavailable" });
      this.advance(false);
      return;
    }
    this.status = item ? (play ? "playing" : "paused") : "idle";
    this.notePosition();
    if (this.status === "playing") this.startTick();
    else this.stopTick();
    this.emitState();
    // Like the core: songs without lyrics are written down as they play.
    if (started) this.transcribe.follow(item.track.id, [...this.userQueue, ...this.ctxOrder.slice(this.ctxPos + 1)].slice(0, 2).map((q) => q.track.id));
  }

  private advance(natural: boolean, manual = false) {
    if (natural && this.repeat === "one" && this.current) {
      this.positionMs = 0;
      this.emitState();
      return;
    }
    // Next in repeat-one skips on and drops to repeat-all.
    if (manual && this.repeat === "one") this.repeat = "all";
    if (this.userQueue.length) {
      if (this.current) this.pushHistory(this.current);
      this.setCurrent(this.userQueue.shift()!, true);
    } else if (this.ctxPos + 1 < this.ctxOrder.length) {
      if (this.current) this.pushHistory(this.current);
      this.ctxPos++;
      this.setCurrent(this.ctxOrder[this.ctxPos], true);
    } else if (this.repeat === "all" && this.ctxOrder.length) {
      // Repeat-all replays the same (shuffled) order.
      if (this.current) this.pushHistory(this.current);
      this.ctxPos = 0;
      this.setCurrent(this.ctxOrder[0], true);
    } else if (this.ctxOrder.length && (this.ctxInfo?.kind === "radio" || this.settings.queueEnd === "keepPlaying") && this.refill()) {
      // A radio goes on; other contexts end in similar music (keep playing).
      this.advance(natural, manual);
    } else {
      // The context ended (repeat off): back to its first track, paused at 0.
      if (this.ctxOrder.length) {
        if (this.current) this.pushHistory(this.current);
        this.ctxPos = 0;
        this.current = this.ctxOrder[0];
      }
      this.status = "paused";
      this.positionMs = 0;
      this.stopTick();
      this.notePosition();
      this.emitState();
    }
  }

  private startTick() {
    if (this.tick) return;
    this.tick = setInterval(() => {
      if (this.status !== "playing" || !this.current) return;
      this.positionMs += 250;
      if (this.positionMs % 5000 === 0) this.notePosition();
      const d = this.current.track.durationMs;
      // A-B loop: at B (or the end), back to A.
      if (this.abLoop && this.positionMs >= Math.min(this.abLoop.bMs, d)) this.positionMs = this.abLoop.aMs;
      if (this.positionMs >= d) {
        this.advance(true);
        return;
      }
      this.emit("player://position", { positionMs: this.positionMs, durationMs: d, bufferedMs: Math.min(d, this.positionMs + 30_000) });
    }, 250);
  }

  private stopTick() {
    if (this.tick) clearInterval(this.tick);
    this.tick = null;
  }

  private snapshot(): PlayerState {
    return {
      status: this.status,
      track: this.current?.track ?? null,
      positionMs: this.positionMs,
      durationMs: this.current?.track.durationMs ?? 0,
      volume: this.volume,
      muted: this.muted,
      shuffle: this.shuffle,
      shuffledContexts: this.shuffledIds,
      repeat: this.repeat,
      abLoop: this.abLoop,
      smartShuffle: this.smartOn(),
      smartContexts: this.smartIds,
      smoothTransitions: !!this.smoothFor && this.smoothFor === this.ctxKey(),
      queue: {
        current: this.current,
        userQueue: this.userQueue,
        nextUp: this.ctxOrder.slice(this.ctxPos + 1),
        history: this.history,
        context: this.ctxInfo,
      },
    };
  }

  private emitState() {
    this.emit("player://state", this.snapshot());
    this.emitSignal();
  }

  private lastSignal = "";
  private signalSnapshot(): SignalPath {
    const o = this.outputState();
    const t = this.current?.track ?? null;
    const cap = this.settings.qualityHome === "original" ? null : Number(this.settings.qualityHome);
    return mockSignalPath({
      track: t,
      settings: this.settings,
      device: o.current,
      exclusive: o.exclusive ?? { state: "off" },
      volume: this.volume,
      muted: this.muted,
      transcodeKbps: t && cap !== null && (t.bitrate ?? Infinity) > cap ? cap : null,
    });
  }
  /** `audio://signal-path`, when it changed (like the core). */
  private emitSignal() {
    const p = this.signalSnapshot();
    const key = JSON.stringify(p);
    if (key === this.lastSignal) return;
    this.lastSignal = key;
    this.emit("audio://signal-path", p);
  }
  signalPath() {
    return this.delay(() => this.signalSnapshot());
  }
  visualizerWatch(on: boolean, fps: number, scope: boolean, pauseOnBattery: boolean) {
    void pauseOnBattery; // the browser has no battery to rest on
    return Promise.resolve(this.visualizer.watch(on, fps, scope));
  }

  private act(fn: () => void) {
    return this.delay(() => {
      fn();
    });
  }

  playContext(args: PlayContextArgs) {
    return this.act(() => this.loadContext(args, true));
  }
  playToggle() {
    return this.act(() => (this.status === "playing" ? this.pauseNow() : this.resumeNow()));
  }
  playPause() {
    return this.act(() => this.pauseNow());
  }
  /** Test/demo hook: headphones unplugged / Bluetooth dropped. The core
   *  pauses; Play resumes on whatever output is there now. */
  loseDevice() {
    if (this.status === "buffering") this.status = "playing";
    this.pauseNow();
  }
  playResume() {
    return this.act(() => this.resumeNow());
  }

  // ---------------------------------------------------------------- output devices

  private outputState(): OutputState {
    const sel = this.outputs.find((d) => d.id === this.outputSelected);
    const current = sel ?? this.outputs.find((d) => d.isDefault) ?? null;
    const t = this.current?.track;
    return {
      devices: this.outputs,
      selected: this.outputSelected,
      current,
      exclusive: hifi.exclusiveFor(this.settings, current, t?.sampleRate, t?.bitDepth),
    };
  }
  outputDevices() {
    return this.delay(() => this.outputState());
  }
  outputSelect(deviceId: string | null) {
    return this.delay(() => {
      if (deviceId !== null && !this.outputs.some((d) => d.id === deviceId)) this.notFound("Output device");
      // Same as the core: position and play state carry over.
      this.outputSelected = deviceId;
      this.settings.outputDevice = deviceId;
      const s = this.outputState();
      this.emitOutput();
      return s;
    });
  }
  // ---------------------------------------------------------------- listen on

  remotePlayers() {
    return this.delay(() => this.remote.players());
  }
  remoteState() {
    return this.delay(() => this.remote.state());
  }
  remoteMove(playerId: string) {
    return this.delay(() => this.remote.move(playerId));
  }
  remoteReturn() {
    return this.delay(() => this.remote.returnHere());
  }
  remotePlayContext(args: PlayContextArgs) {
    return this.delay(() => {
      this.loadContext(args, false);
      return this.remote.resend();
    });
  }
  remoteQueueJump(itemId: string) {
    return this.delay(() => {
      const it = [...this.userQueue, ...this.ctxOrder].find((i) => i.itemId === itemId) ?? this.notFound("Queue item");
      this.resumeAt(it.track.id, 0, false);
      return this.remote.resend();
    });
  }
  remoteMoveWithPassword(playerId: string, password: string) {
    return this.delay(() => this.remote.moveWithPassword(playerId, password));
  }
  remoteCommand(command: RemoteCommand) {
    return this.delay(() => this.remote.command(command));
  }
  /** Local playback onto this song at this spot (back from another device). */
  private resumeAt(trackId: string, ms: number, play: boolean) {
    if (this.current?.track.id === trackId) {
      this.positionMs = ms;
      if (play) this.resumeNow();
      else this.pauseNow();
      this.emitState();
      return;
    }
    const u = this.userQueue.findIndex((i) => i.track.id === trackId);
    if (u >= 0) {
      if (this.current) this.pushHistory(this.current);
      const it = this.userQueue[u];
      this.userQueue = this.userQueue.slice(u + 1);
      this.setCurrent(it, play, ms);
      return;
    }
    const c = this.ctxOrder.findIndex((i, k) => k > this.ctxPos && i.track.id === trackId);
    if (c >= 0) {
      if (this.current) this.pushHistory(this.current);
      this.ctxPos = c;
      this.setCurrent(this.ctxOrder[c], play, ms);
      return;
    }
    this.loadContext({ context: { kind: "tracks", trackIds: [trackId] }, startTrackId: trackId }, play);
    this.positionMs = ms;
    this.emitState();
  }

  /** `output://changed`, and the EQ follows the device like in the core. */
  private emitOutput() {
    const s = this.outputState();
    this.emit("output://changed", s);
    this.emitSignal();
    if (eq.outputChanged(this.settings, s.current)) this.emit("settings://changed", { ...this.settings, volume: this.volume });
  }
  /** Test/demo hook: an output device is unplugged. If it was the chosen
   *  one, the core pauses, falls back to the system default and says so. */
  unplugOutput(id: string) {
    const gone = this.outputs.find((d) => d.id === id);
    if (!gone) return;
    this.outputs = this.outputs.filter((d) => d.id !== id);
    if (gone.isDefault && this.outputs[0]) this.outputs = this.outputs.map((d, i) => ({ ...d, isDefault: i === 0 }));
    const chosen = this.outputSelected === id;
    const followedDefault = this.outputSelected === null && gone.isDefault;
    if (chosen) this.outputSelected = null;
    if (chosen || followedDefault) this.loseDevice();
    this.emitOutput();
    if (chosen) this.emit("output://lost", { device: gone.name });
  }
  /** Test/demo hook: a device is plugged in. */
  plugOutput(device: OutputDevice) {
    this.outputs = [...this.outputs.filter((d) => d.id !== device.id), device];
    this.emitOutput();
  }
  /** Test hook: audio settings back to their defaults (profiles and bindings too). */
  resetAudio() {
    const d = DEFAULT_SETTINGS;
    Object.assign(this.settings, {
      crossfade: d.crossfade,
      crossfadeSeconds: d.crossfadeSeconds,
      crossfadeSkipAlbums: d.crossfadeSkipAlbums,
      automix: d.automix,
      automixWhere: d.automixWhere,
      automixStyle: d.automixStyle,
      leveling: d.leveling,
      preampDb: d.preampDb,
      limiter: d.limiter,
      eqEnabled: d.eqEnabled,
      eqBands: [...d.eqBands],
      eqPreset: d.eqPreset,
      eqProfiles: [],
      eqOutputs: [],
      eqFallback: null,
      eqLastOutput: null,
    });
  }
  /** Test hook: replaces the device list (what the system reports). */
  setOutputs(devices: OutputDevice[]) {
    this.outputs = devices.map((d) => ({ ...d }));
    this.emitOutput();
  }
  /** Test hook: the starting devices, following the system default. */
  resetOutputs() {
    this.outputs = MOCK_OUTPUTS.map((d) => ({ ...d }));
    this.outputSelected = null;
    this.settings.outputDevice = null;
  }
  private pauseNow() {
    if (this.status !== "playing") return;
    this.status = "paused";
    this.stopTick();
    this.notePosition();
    this.emitState();
  }
  private resumeNow() {
    if (!this.current) return;
    this.status = "playing";
    this.startTick();
    this.emitState();
  }
  playNext() {
    return this.act(() => this.advance(false, true));
  }
  playPrevious() {
    return this.act(() => {
      const restart = () => {
        this.positionMs = 0;
        this.emitState();
      };
      // At or past 3 s: restart, and repeat-one is kept.
      if (this.positionMs >= PREVIOUS_RESTART_MS || !this.current) return restart();
      // Earlier: repeat-one drops to repeat-all, and Previous goes to the
      // previous context position (never a queued song). On a playing
      // queued song that is the context track before it; the song is dropped.
      if (this.repeat === "one") this.repeat = "all";
      const target = this.current.source === "user" ? this.ctxPos : this.ctxPos - 1;
      if (target < 0 || target >= this.ctxOrder.length) return restart();
      this.pushHistory(this.current);
      this.ctxPos = target;
      this.setCurrent(this.ctxOrder[target], true);
    });
  }
  playSeek(positionMs: number) {
    return this.act(() => {
      this.positionMs = Math.max(0, Math.min(positionMs, this.current?.track.durationMs ?? 0));
      // Seeking outside the loop's span clears it.
      if (this.abLoop && (this.positionMs < this.abLoop.aMs || this.positionMs > this.abLoop.bMs)) this.abLoop = null;
      this.emitState();
    });
  }
  setVolume(volume: number) {
    return this.act(() => {
      // Like the core: the level and mute are separate; setting one leaves the other.
      this.volume = Math.max(0, Math.min(1, volume));
      this.emitState();
    });
  }
  setMuted(muted: boolean) {
    return this.act(() => {
      this.muted = muted;
      this.emitState();
    });
  }
  setShuffle(shuffle: boolean) {
    return this.act(() => this.applyShuffle(shuffle));
  }
  setSmartShuffle(on: boolean) {
    return this.act(() => {
      const c = this.ctxInfo;
      if (!c?.id || (c.kind !== "playlist" && c.kind !== "liked")) throw { kind: "invalid", message: "Smart shuffle is for playlists and Liked Songs" };
      this.applySmart(c.id, on);
    });
  }
  setContextSmartShuffle(contextId: string, on: boolean) {
    return this.act(() => this.applySmart(contextId, on));
  }
  /** Like the core: on also shuffles; off keeps the playing song and the shuffled order. */
  private applySmart(id: string, on: boolean) {
    const c = this.ctxInfo;
    const playing = c?.id === id && (c.kind === "playlist" || c.kind === "liked");
    if (!playing) {
      if (on) this.storeSmartPref(id, true);
      else this.storeShufflePref(id, false);
    } else if (on) {
      if (!this.shuffle) this.applyShuffle(true);
      this.storeSmartPref(id, true);
      this.smartBuilt = false;
      this.buildSmart();
    } else {
      this.dropSmart();
      this.storeShufflePref(id, false);
      this.shuffle = false;
    }
    this.emitState();
  }
  smartSuggestionRemove() {
    return this.act(() => {
      if (this.current?.source !== "smart") throw { kind: "invalid", message: "No suggestion is playing" };
      this.dropSmart();
      this.smartBuilt = false;
      this.advance(false, true);
      this.buildSmart();
      this.emitState();
    });
  }
  smartSuggestionKeep(itemId: string) {
    return this.act(() => {
      const it = [this.current, ...this.ctxOriginal].find((i) => i?.itemId === itemId && i.source === "smart");
      if (!it) throw { kind: "notFound", message: `suggestion ${itemId}` };
      it.source = "context";
      // The core sends a fresh copy, so the bar sees the change.
      if (this.current?.itemId === itemId) this.current = { ...this.current, source: "context" };
      this.emitState();
    });
  }
  private dropSmart() {
    this.ctxOrder = this.ctxOrder.filter((i, k) => k <= this.ctxPos || i.source !== "smart");
  }
  setSmoothTransitions(on: boolean) {
    return this.act(() => {
      this.smoothFor = on ? this.ctxKey() : null;
      this.emitState();
    });
  }
  mixesHome() {
    return this.delay(() => this.mixes.shelves());
  }
  /** Mixes were made again: the UI hears it, and downloaded mixes follow (as in the core). */
  mixesChanged() {
    this.emit("mixes://changed", null);
    this.downloads.resync();
  }
  mixPage(id: string, steer?: MixSteer | null) {
    return this.delay(() => this.withoutHidden(this.mixes.page(id, steer ?? null) ?? this.notFound("Mix")));
  }
  /** Like the core: hidden songs and artists leave mixes and radio. */
  private withoutHidden<P extends { tracks: Track[]; familiar: boolean[]; mix: { trackCount: number; durationMs: number } }>(page: P): P {
    const keep = page.tracks.map((t) => !this.hidden.hides(t));
    if (keep.every(Boolean)) return page;
    const tracks = page.tracks.filter((_, i) => keep[i]);
    return { ...page, tracks, familiar: page.familiar.filter((_, i) => keep[i]), mix: { ...page.mix, trackCount: tracks.length, durationMs: tracks.reduce((s, t) => s + t.durationMs, 0) } };
  }
  mixRefresh(id: string) {
    return this.delay(() => {
      const page = this.mixes.refresh(id) ?? this.notFound("Mix");
      this.mixesChanged();
      return page;
    });
  }
  radioPage(seed: string, steer?: MixSteer | null, salt?: number) {
    return this.delay(() => this.withoutHidden(this.mixes.radio(seed, steer ?? null, salt ?? 0) ?? this.notFound("Radio")));
  }
  /** Shuffle for the playing context; remembered for its id. */
  private applyShuffle(shuffle: boolean) {
    if (this.ctxInfo?.id) this.storeShufflePref(this.ctxInfo.id, shuffle);
    if (!shuffle) this.dropSmart();
    if (shuffle !== this.shuffle) {
      this.shuffle = shuffle;
      const at = this.ctxPos >= 0 ? this.ctxOrder[this.ctxPos] : undefined;
      if (shuffle) {
        // Every other track (earlier ones too) dealt into a new order.
        const live = new Set(this.ctxOrder);
        const others = this.ctxOriginal.filter((i) => i !== at && live.has(i));
        this.ctxOrder = at ? [at, ...this.shuffleOrder(others)] : this.shuffleOrder(others);
        this.ctxPos = at ? 0 : -1;
      } else {
        // Back to original order, continuing after the current position.
        const live = new Set(this.ctxOrder);
        this.ctxOrder = this.ctxOriginal.filter((i) => live.has(i));
        this.ctxPos = at ? this.ctxOrder.indexOf(at) : -1;
      }
    }
    this.emitState();
  }
  /** Shuffle preference for a context: applies now if it's playing,
   *  otherwise the next time it plays. */
  setContextShuffle(contextId: string, shuffle: boolean) {
    return this.act(() => {
      if (this.ctxInfo?.id === contextId) return this.applyShuffle(shuffle);
      this.storeShufflePref(contextId, shuffle);
      this.emitState();
    });
  }
  setRepeat(repeat: RepeatMode) {
    return this.act(() => {
      this.repeat = repeat;
      this.emitState();
    });
  }
  setAbLoop(abLoop: AbLoop | null) {
    return this.act(() => {
      if (abLoop) {
        if (!this.current) throw { kind: "invalid", message: "nothing is playing" };
        const d = this.current.track.durationMs;
        const a = Math.min(abLoop.aMs, abLoop.bMs, d);
        const b = Math.min(Math.max(abLoop.aMs, abLoop.bMs), d);
        if (b - a < 1000) throw { kind: "invalid", message: "B must be at least 1 s after A" };
        this.abLoop = { aMs: Math.round(a), bMs: Math.round(b) };
      } else {
        this.abLoop = null;
      }
      this.emitState();
    });
  }
  queueAdd(trackIds: string[], at: QueueAt) {
    return this.act(() => {
      const items = trackIds.map((id) => this.trackById(id)).filter((t): t is Track => !!t).map((t) => this.item(t, "user"));
      const i = at === "next" ? 0 : at === "end" ? this.userQueue.length : Math.max(0, Math.min(at.index, this.userQueue.length));
      this.userQueue = [...this.userQueue.slice(0, i), ...items, ...this.userQueue.slice(i)];
      if (!this.current && this.userQueue.length) this.setCurrent(this.userQueue.shift()!, false);
      this.emitState();
    });
  }
  queueRemove(itemId: string) {
    return this.act(() => {
      this.userQueue = this.userQueue.filter((i) => i.itemId !== itemId);
      const idx = this.ctxOrder.findIndex((i) => i.itemId === itemId);
      if (idx > this.ctxPos) this.ctxOrder.splice(idx, 1);
      this.emitState();
    });
  }
  queueMove(itemId: string, toIndex: number) {
    return this.act(() => {
      const from = this.userQueue.findIndex((i) => i.itemId === itemId);
      if (from < 0) return;
      const [it] = this.userQueue.splice(from, 1);
      this.userQueue.splice(Math.max(0, Math.min(toIndex, this.userQueue.length)), 0, it);
      this.emitState();
    });
  }
  queueClear() {
    return this.act(() => {
      this.userQueue = [];
      this.emitState();
    });
  }
  queueJump(itemId: string) {
    return this.act(() => {
      const u = this.userQueue.findIndex((i) => i.itemId === itemId);
      if (u >= 0) {
        if (this.current) this.pushHistory(this.current);
        const it = this.userQueue[u];
        this.userQueue = this.userQueue.slice(u + 1);
        this.current = null;
        this.setCurrent(it, true);
        return;
      }
      const c = this.ctxOrder.findIndex((i) => i.itemId === itemId);
      if (c > this.ctxPos) {
        if (this.current) this.pushHistory(this.current);
        this.ctxPos = c;
        this.current = null;
        this.setCurrent(this.ctxOrder[c], true);
        return;
      }
      const h = this.history.findIndex((i) => i.itemId === itemId);
      if (h >= 0) {
        const it = this.history[h];
        if (this.current) this.pushHistory(this.current);
        this.current = null;
        this.setCurrent({ ...it, itemId: `q${this.itemSeq++}` }, true);
      }
    });
  }
  playerState() {
    return this.delay(() => this.snapshot());
  }

  // ---------------------------------------------------------------- health, settings, updates

  health() {
    return this.delay(() => this.healthState);
  }
  settingsGet() {
    return this.delay(() => ({ ...this.settings, volume: this.volume }));
  }
  settingsSet(patch: Partial<Settings>) {
    return this.delay(() => {
      this.librariesMock.check(patch.libraries, patch.serverView);
      const next = { ...this.settings, ...patch };
      if (patch.libraries) next.libraries = [...new Set(patch.libraries.map((e) => e.trim()))];
      // Profiles and bindings change only through their own calls.
      next.eqProfiles = this.settings.eqProfiles;
      next.eqOutputs = this.settings.eqOutputs;
      next.eqFallback = this.settings.eqFallback;
      next.eqLastOutput = this.settings.eqLastOutput;
      next.eqPreset = this.settings.eqPreset;
      next.eqBands = this.settings.eqBands;
      eq.applyAudioPatch(next, patch);
      // Same ranges as the core.
      next.precacheCount = Math.min(50, Math.max(1, Math.round(next.precacheCount)));
      next.artCacheMb = Math.min(51_200, Math.max(128, Math.round(next.artCacheMb)));
      // Only outputSelect changes the output device.
      next.outputDevice = this.settings.outputDevice;
      if (patch.listenInviteHours !== undefined && ![1, 24, 168].includes(patch.listenInviteHours)) {
        throw { kind: "invalid", message: "Invites expire after 1 hour, 24 hours or 7 days." };
      }
      if (patch.doctorCoverPx !== undefined && ![300, 500, 600, 800, 1000, 1400].includes(patch.doctorCoverPx)) {
        throw { kind: "invalid", message: "Pick one of the cover sizes offered." };
      }
      if (patch.autoRequestCap !== undefined && ![1, 2, 3, 5, 10].includes(patch.autoRequestCap)) {
        throw { kind: "invalid", message: "Pick one of the daily limits offered." };
      }
      if (patch.phoneKeepRecent !== undefined && !(PHONE_KEEP_RECENT_CHOICES as readonly number[]).includes(patch.phoneKeepRecent)) {
        throw { kind: "invalid", message: "Keep 0, 25, 50, 100, 250 or 500 recent songs." };
      }
      if (patch.subsonicCheckMinutes !== undefined && ![1, 5, 15, 30, 60].includes(patch.subsonicCheckMinutes)) {
        throw { kind: "invalid", message: "Pick one of the times offered." };
      }
      if (patch.layout !== undefined) {
        const why = layoutProblem(patch.layout);
        if (why) throw { kind: "invalid", message: why };
      }
      if (patch.turntableColumn !== undefined && !["small", "medium", "large"].includes(patch.turntableColumn)) {
        throw { kind: "invalid", message: "Pick one of the widths offered." };
      }
      // The Console layout's table and panes, checked and clamped as the core does.
      if (patch.consoleColumns !== undefined) {
        if (!Array.isArray(patch.consoleColumns) || patch.consoleColumns.some((c) => !CONSOLE_COLUMNS.includes(c))) {
          throw { kind: "invalid", message: "Pick columns from the ones offered." };
        }
        next.consoleColumns = [...new Set(patch.consoleColumns)];
      }
      if (patch.consoleSort !== undefined && !consoleSortOk(patch.consoleSort)) throw { kind: "invalid", message: "Sort by one of the table's columns." };
      next.consoleTree = clampPane("tree", next.consoleTree);
      next.consoleDetails = clampPane("details", next.consoleDetails);
      if (patch.doctorChecks !== undefined) next.doctorChecks = { ...DEFAULT_SETTINGS.doctorChecks, ...patch.doctorChecks };
      if (patch.listenRelay !== undefined) {
        const r = patch.listenRelay.trim().replace(/\/+$/, "");
        if (r && !/^https?:\/\/[^\s/?#@]+(\/[^\s?#]*)?$/.test(r)) throw { kind: "invalid", message: "The relay is a web address like https://ntfy.example.com." };
        next.listenRelay = r;
      }
      if (patch.listenName !== undefined) next.listenName = patch.listenName.trim();
      if (patch.tautulliUrl !== undefined) {
        const u = patch.tautulliUrl.trim().replace(/\/+$/, "");
        if (u && !/^https?:\/\/[^\s/?#@]+(\/[^\s?#]*)?$/.test(u)) throw { kind: "invalid", message: "Tautulli's address is a web address like http://192.168.x.x:8181." };
        next.tautulliUrl = u;
      }
      if (patch.lidarrStagingFolder !== undefined) {
        const f = patch.lidarrStagingFolder.trim().replace(/(.)[\\/]+$/, "$1");
        if (f && !/^(\/|\\\\|[A-Za-z]:[\\/])./.test(f)) throw { kind: "invalid", message: "Write the folder's full path as Lidarr sees it, like /downloads/upgrades." };
        next.lidarrStagingFolder = f;
      }
      for (const k of ["lidarrUrl", "lidarrAwayUrl"] as const) {
        if (patch[k] === undefined) continue;
        const u = patch[k].trim().replace(/\/+$/, "");
        if (u && !/^https?:\/\/[^\s/?#@]+(\/[^\s?#]*)?$/.test(u)) throw { kind: "invalid", message: "Lidarr's address is a web address like http://192.168.x.x:8686." };
        next[k] = u;
      }
      // Known sources only, each once, in their listed order (as the core).
      if (patch.bringInWaves) {
        const w = [...new Set(patch.bringInWaves)].filter((n) => Number.isInteger(n) && n >= 1 && n <= 100_000).sort((a, b) => b - a);
        if (!w.length || w.length !== patch.bringInWaves.length || w.length > 5) throw { kind: "invalid", message: "Waves are 1 to 5 different play counts, like 100, 50, 20." };
        next.bringInWaves = w;
      }
      if (patch.importSources) next.importSources = ["streaming", "streaming-account", "transfer", "lists"].filter((id) => patch.importSources!.includes(id));

      if (patch.nowPlayingFeedPort !== undefined) {
        const port = patch.nowPlayingFeedPort;
        if (!Number.isInteger(port) || port < 1024 || port > 65535) throw { kind: "invalid", message: "Pick a port from 1024 to 65535." };
      }
      checkThemePatch(patch);
      if (patch.accent !== undefined && patch.accent.startsWith("#")) next.accent = patch.accent.toUpperCase();
      this.settings = next;
      this.playHistory.follow();
      if (patch.nowPlayingFeed !== undefined || patch.nowPlayingFeedPort !== undefined) {
        const st = this.feed();
        this.later(0, () => this.emit("feed://status", st));
      }
      if (patch.allowStreamDeck !== undefined || patch.allowAutomation !== undefined) {
        const st = this.deck();
        this.later(0, () => this.emit("deck://status", st));
      }
      if (patch.listenGuestsControl !== undefined || patch.shareOfferServer !== undefined) this.social.settingsChanged();
      // Another server's music, or more libraries: every list reloads.
      if (patch.serverView !== undefined || patch.libraries !== undefined) this.later(0, () => this.emit("library://changed", null));
      this.artUsed = Math.min(this.artUsed, next.artCacheMb * 1024 * 1024);
      if (typeof patch.volume === "number") this.volume = Math.min(1, Math.max(0, patch.volume));
      if (patch.readFileTags !== undefined) this.tags.settingsChanged();
      // Sound settings change the signal path.
      this.later(0, () => this.emitSignal());
      const mixKeys = ["mixVariety", "mixExcludeArtists", "mixExcludeGenres", "mixExcludeAlbums", "mixUnreleased", "daylistHours", "explicitContent", "showFanEdits"];
      if (mixKeys.some((k) => k in patch)) {
        this.mixes.invalidate();
        this.later(0, () => this.mixesChanged());
      }
      if (patch.smartShuffle !== undefined) {
        // Off takes smart shuffle out everywhere; on builds suggestions again.
        if (patch.smartShuffle) this.smartBuilt = false;
        else this.dropSmart();
        this.later(0, () => {
          this.buildSmart();
          this.emitState();
        });
      }
      if (patch.transcribeLyrics !== undefined || patch.transcribeModel !== undefined || patch.transcribeGpu !== undefined) this.transcribe.settingsChanged();
      if (patch.downloadLiked !== undefined && patch.downloadLiked !== this.downloads.isOn("liked", ""))
        this.downloads.set("liked", "", patch.downloadLiked, "Liked Songs");
      next.downloadLimitGb = Math.min(4096, Math.max(1, Math.round(next.downloadLimitGb)));
      // Only downloadsChooseFolder changes the folder.
      next.downloadFolder = this.settings.downloadFolder;
      // Only the sync folder commands change the sync folder.
      next.syncFolder = this.settings.syncFolder;
      const syncKeys = ["syncEnabled", "syncSettings", "syncLibrary", "syncFollows", "syncLyrics"];
      if (syncKeys.some((k) => k in patch)) this.later(0, () => this.emit("sync-file://status", this.syncFile.status()));
      if (patch.goOffline !== undefined && patch.goOffline !== (this.healthState.banner?.kind === "goneOffline"))
        this.later(0, () => this.setHealth(patch.goOffline ? "goneOffline" : "local"));
      this.later(0, () => this.downloads.poke());
      if (patch.abLoopHandles === false && this.abLoop) {
        this.abLoop = null;
        this.emitState();
      }
    });
  }
  private changeEq(f: (s: Settings) => void) {
    return this.delay(() => {
      const next: Settings = { ...this.settings, eqProfiles: this.settings.eqProfiles.map((p) => ({ ...p })) };
      f(next);
      this.settings = next;
      // EQ, correction and bit-perfect all show in the signal path.
      this.later(0, () => this.emitSignal());
      return { ...next, volume: this.volume };
    });
  }
  eqProfileSave(name: string) {
    return this.changeEq((s) => eq.saveAs(s, name));
  }
  eqProfileRename(from: string, to: string) {
    return this.changeEq((s) => eq.rename(s, from, to));
  }
  eqProfileDelete(name: string) {
    return this.changeEq((s) => eq.remove(s, name));
  }
  /** Where the phone's sound goes; tests set it and call emitRoute. */
  route: AudioRoute | null = null;
  audioRoute() {
    return this.delay(() => this.route);
  }
  emitRoute(route: AudioRoute | null) {
    this.route = route;
    this.emit("audio://route", route);
  }
  headphoneSearch(query: string) {
    return this.delay(() => hifi.search(query));
  }
  headphoneCorrectionSet(path: string | null) {
    return this.changeEq((s) => {
      const d = this.outputState().current;
      if (!d) throw { kind: "notFound", message: "No output device on this computer is in use." };
      const entry = path === null ? null : hifi.MOCK_HEADPHONES.find((h) => h.path === path);
      if (entry === undefined) throw { kind: "notFound", message: `headphone ${path}` };
      hifi.setCorrection(s, d, entry);
    });
  }
  headphoneCorrectionCompare(bypass: boolean) {
    return this.delay(() => {
      this.correctionBypassed = bypass;
    });
  }
  bitPerfectSet(on: boolean) {
    const r = this.changeEq((s) => {
      const d = this.outputState().current;
      if (!d) throw { kind: "notFound", message: "No output device on this computer is in use." };
      hifi.setBitPerfect(s, d, on);
    });
    void r.then(() => this.emit("output://changed", this.outputState()), () => {});
    return r;
  }
  eqUseForOutput(on: boolean) {
    return this.changeEq((s) => {
      const d = this.outputState().current;
      if (!d) throw { kind: "notFound", message: "No output device is in use." };
      eq.useForOutput(s, d, on);
    });
  }
  artCacheInfo() {
    return this.delay(() => ({ usedBytes: this.artUsed, capBytes: this.settings.artCacheMb * 1024 * 1024 }));
  }
  artCacheClear() {
    return this.delay(() => {
      const freed = this.artUsed;
      this.artUsed = 0;
      return freed;
    });
  }
  /** What appInfo says this app's version is (tests set it). */
  appVersion = "0.2.20"; // the same as the mock Bridge, so the mock shows no version line
  appInfo() {
    return this.delay(() => ({ version: this.appVersion, licence: "GPL-3.0-or-later", repoUrl: "https://github.com/Silvercyclops999/platter", form: "desktop" as const }));
  }
  openProjectPage() {
    return this.delay(() => undefined);
  }
  showRoutePicker() {
    return this.delay(() => undefined);
  }
  haptic(_kind: HapticKind) {
    return Promise.resolve();
  }
  setStatusBarStyle(_dark: boolean) {
    return Promise.resolve();
  }
  updateCheck() {
    return this.delay(() =>
      this.opts.updateAvailable
        ? { available: true, version: "0.2.0", notes: null }
        : { available: false, version: null, notes: null },
    );
  }
  updateInstallOnRestart() {
    return this.delay(() => undefined);
  }

  // ---------------------------------------------------------------- artist info

  /** Plex bio as artistPage gives it (every third artist has none). */
  private plexBio(idx: number, title: string): string | null {
    return idx % 3 === 2 ? null : mockBio(title, idx);
  }

  artistInfo(id: string) {
    return this.delay((): ArtistInfo => {
      if (isCreditId(id)) {
        // A featured guest: the made-up services know nothing about them.
        this.creditedPage(id, this.nameLookup());
        const blank: ArtistInfo = {
          id,
          bio: null,
          bioSource: null,
          origin: null,
          yearsActive: null,
          members: [],
          tags: [],
          listeners: null,
          scrobbles: null,
          listenersFrom: null,
          related: [],
          photo: null,
          banner: null,
          logo: null,
          credits: [],
          gallery: [],
          links: [],
          bioLinks: [],
        };
        return blank;
      }
      const artist = this.fx.artists.find((a) => a.id === id) ?? this.notFound("Artist");
      const idx = this.fx.artists.indexOf(artist);
      // In the browser the first open answers with Plex only, then the
      // "background fetch" lands, like the core.
      const fetched = !this.opts.timelines || this.infoFetched.has(id);
      if (!fetched) {
        this.infoFetched.add(id);
        // The Popular list lands first, then the rest.
        this.later(400, () => {
          this.popularFetched.add(id);
          this.emit("artistinfo://changed", { id });
          this.emit("artistinfo://popular", { id });
        });
        this.later(600, () => this.emit("artistinfo://changed", { id }));
      }
      return mockArtistInfo(artist, idx, this.plexBio(idx, artist.title), this.fx.artists, this.settings, this.infoKeys, fetched);
    });
  }
  albumCredits(id: string) {
    return this.delay(() => {
      const album = this.fx.albums.find((a) => a.id === id) ?? this.notFound("Album");
      const tracks = this.fx.tracksByAlbum.get(id) ?? [];
      const answer = mockAlbumCredits(album, tracks, this.fx.artists, this.settings.infoMusicbrainz);
      // In the browser the first open is "pending", then the lookup lands, like the core.
      if (answer.state === "found" && this.opts.timelines && !this.albumCreditsFetched.has(id)) {
        this.albumCreditsFetched.add(id);
        this.later(700, () => this.emit("albumcredits://changed", { id }));
        return { ...answer, state: "pending" as const, released: null, editionDate: null, label: null, credits: [], tracks: [], url: null };
      }
      return answer;
    });
  }
  sleeveNotes(id: string) {
    return this.delay(() => {
      const album = this.fx.albums.find((a) => a.id === id) ?? this.notFound("Album");
      // In the browser the first open has tags and Plex only, then the lookups land, like the core.
      const fetched = !this.opts.timelines || this.sleeveFetched.has(id);
      const notes = this.sleeve.notes(album, fetched);
      if (!fetched && notes.lookups.online && !notes.lookups.unreleased) {
        this.sleeveFetched.add(id);
        this.later(800, () => this.emit("sleeve://changed", { id }));
        return { ...notes, looking: true };
      }
      return notes;
    });
  }
  songCredits(id: string) {
    return this.delay(() => {
      const track = this.fx.tracks.find((t) => t.id === id) ?? this.notFound("Song");
      const fetched = !this.opts.timelines || this.sleeveFetched.has(track.albumId);
      const out = this.sleeve.song(track, fetched);
      if (!fetched && out.lookups.online && !out.lookups.unreleased) {
        this.sleeveFetched.add(track.albumId);
        this.later(800, () => this.emit("sleeve://changed", { id: track.albumId }));
        return { ...out, looking: true };
      }
      return out;
    });
  }
  creditPage(name: string) {
    return this.delay(() => this.sleeve.creditPage(name));
  }
  crates(kind: CrateKind) {
    return this.delay(() => this.crateIndex.list(kind));
  }
  cratePage(kind: CrateKind, key: string) {
    return this.delay(() => this.crateIndex.page(kind, key));
  }
  albumCrates(id: string) {
    return this.delay(() => this.crateIndex.shelves(id));
  }
  albumSides(id: string) {
    return this.delay(() => {
      if (!this.fx.albums.some((a) => a.id === id)) this.notFound("Album");
      return this.sides.get(id);
    });
  }
  setAlbumSides(id: string, starts: string[] | null) {
    return this.delay(() => {
      if (!this.fx.albums.some((a) => a.id === id)) this.notFound("Album");
      const out = this.sides.set(id, starts);
      this.later(0, () => this.emit("syncstore://changed", { namespaces: ["albumSides"], remote: false }));
      return out;
    });
  }
  diaryEntries(query?: DiaryQuery | null) {
    return this.delay(() => this.diary.list(query));
  }
  diaryEntryEdit(id: string, edit: DiaryEdit) {
    return this.delay(() => {
      const out = this.diary.edit(id, edit);
      this.later(0, () => this.emit("diary://changed", null));
      return out;
    });
  }
  diaryEntryDelete(id: string) {
    return this.delay(() => {
      const gone = this.diary.delete(id);
      if (gone) this.later(0, () => this.emit("diary://changed", null));
      return gone;
    });
  }
  diaryLogAlbum(id: string) {
    return this.delay(() => {
      if (this.settings.keepDiary === false) throw { kind: "invalid", message: "Turn on Keep a listening diary to log albums." };
      const out = this.diary.log(id);
      this.later(0, () => this.emit("diary://changed", null));
      return out;
    });
  }
  albumFirstHeard(id: string) {
    return this.delay(() => this.diary.firstHeard(id));
  }
  diaryExport(format: "text" | "csv", query?: DiaryQuery | null) {
    return this.delay(() => {
      this.lastDiaryExport = { format, body: this.diary.exportText(query) };
      return `Listening diary.${format === "csv" ? "csv" : "txt"}`;
    });
  }
  albumReleaseStatus(id: string) {
    return this.delay(() => {
      const album = this.fx.albums.find((a) => a.id === id) ?? this.notFound("Album");
      return this.archive.status(album);
    });
  }

  // ---------------------------------------------------------------- archive

  releaseMarkSet(kind: "album" | "track", id: string, mark: ReleaseMark | null) {
    return this.delay(() => {
      this.archive.setMark(kind, id, mark);
      const album = kind === "album" ? this.fx.albums.find((a) => a.id === id) : this.fx.albums.find((a) => a.id === this.trackById(id)?.albumId);
      if (album) {
        this.later(0, () => {
          this.emit("releasestatus://changed", { id: album.id });
          this.emit("artist://changed", { id: album.artistId });
        });
      }
    });
  }
  trackVersions(id: string) {
    return this.delay(() => {
      if (!this.trackById(id)) this.notFound("Song");
      const items = this.archive.versions(id);
      return this.settings.showFanEdits ? items : items.filter((v) => v.current || !this.tags.index().tracks[v.track.id]?.edit);
    });
  }
  trackLineage(id: string) {
    return this.delay(() => {
      if (!this.trackById(id)) this.notFound("Song");
      const l = this.archive.lineage(id) ?? this.notFound("Song");
      const hidden = (tid: string) => !this.settings.showFanEdits && !!this.tags.index().tracks[tid]?.edit;
      return { ...l, nodes: l.nodes.filter((n) => n.current || !hidden(n.track.id)) };
    });
  }

  // ---- Compare: like the core, both files come in one after the other,
  // then switches map the moment (by proportion past 5% apart).

  /** Test hook: make preparing fail (the server dropped the file). */
  compareFails = false;
  private compare: { a: Track; b: Track; side: CompareSide | null; ab: AbLoop | null; run: number } | null = null;
  private compareRun = 0;
  /** The user's listening, set aside the first time Compare plays. */
  private compareHeld: {
    status: PlayerState["status"];
    positionMs: number;
    shuffle: boolean;
    repeat: RepeatMode;
    abLoop: AbLoop | null;
    current: QueueItem | null;
    userQueue: QueueItem[];
    ctxOriginal: QueueItem[];
    ctxOrder: QueueItem[];
    ctxPos: number;
    history: QueueItem[];
    ctxInfo: QueueContextInfo | null;
  } | null = null;

  private holdListening() {
    if (this.compareHeld) return;
    this.notePosition();
    const playing = this.status === "playing" || this.status === "loading" || this.status === "buffering";
    this.compareHeld = {
      status: playing ? "playing" : this.status,
      positionMs: this.positionMs,
      shuffle: this.shuffle,
      repeat: this.repeat,
      abLoop: this.abLoop,
      current: this.current,
      userQueue: [...this.userQueue],
      ctxOriginal: [...this.ctxOriginal],
      ctxOrder: [...this.ctxOrder],
      ctxPos: this.ctxPos,
      history: [...this.history],
      ctxInfo: this.ctxInfo,
    };
  }

  private unholdListening() {
    const h = this.compareHeld;
    if (!h) return;
    this.compareHeld = null;
    Object.assign(this, h);
    if (h.status === "playing") this.startTick();
    else this.stopTick();
    this.emitState();
  }

  comparePrepare(aId: string, bId: string): Promise<CompareReady> {
    const run = ++this.compareRun;
    const a = this.trackById(aId);
    const b = this.trackById(bId);
    if (!a || !b) return Promise.reject({ kind: "notFound", message: "Song not found" });
    if (aId === bId) return Promise.reject({ kind: "invalid", message: "Pick two different versions." });
    const steps = [0, 0.35, 0.7, 1];
    const step = Math.max(10, this.opts.latencyMs);
    return new Promise((resolve, reject) => {
      let i = 0;
      const tick = () => {
        if (run !== this.compareRun) return reject({ kind: "invalid", message: "Compare was closed." });
        const side: CompareSide = i < steps.length ? "a" : "b";
        if (this.compareFails && side === "b") {
          // Like the core: a failed prepare leaves the listening as it was.
          this.compare = null;
          this.unholdListening();
          return reject({ kind: "network", message: `Couldn't get ${b.title}: http 404` });
        }
        this.emit("compare://progress", { side, fraction: steps[i % steps.length] });
        i += 1;
        if (i < steps.length * 2) return this.later(step, tick);
        const levelAvailable = !a.codec?.includes("mp3") && !b.codec?.includes("mp3");
        this.compare = { a, b, side: null, ab: null, run };
        resolve({
          aId,
          bId,
          approximate: compareApproximate(a.durationMs, b.durationMs),
          levelMatched: this.settings.compareLevelMatch && levelAvailable,
          levelAvailable,
        });
      };
      this.later(step, tick);
    });
  }
  compareSwitch(side: CompareSide, play?: boolean): Promise<CompareNow> {
    return this.delay(() => {
      const c = this.compare;
      if (!c) throw { kind: "invalid", message: "Compare isn't open." };
      const to = side === "a" ? c.a : c.b;
      const from = c.side ? (c.side === "a" ? c.a : c.b) : null;
      const playingFrom = !!from && this.current?.track.id === from.id;
      let pos = playingFrom ? compareMap(this.positionMs, from!.durationMs, to.durationMs) : 0;
      const ab = c.ab && side === "b" ? compareMapLoop(c.ab, c.a.durationMs, c.b.durationMs) : c.ab;
      if (ab && (pos < ab.aMs || pos >= ab.bMs)) pos = ab.aMs;
      const wasPlaying = this.status === "playing";
      this.holdListening();
      this.loadContext({ context: { kind: "tracks", trackIds: [to.id] }, startTrackId: to.id }, play ?? (!playingFrom || wasPlaying));
      this.positionMs = pos;
      this.abLoop = ab;
      c.side = side;
      this.emitState();
      return { side, positionMs: pos, abLoop: ab };
    });
  }
  compareLoop(ab: AbLoop | null): Promise<AbLoop | null> {
    return this.delay(() => {
      const c = this.compare;
      if (!c) throw { kind: "invalid", message: "Compare isn't open." };
      if (ab && ab.bMs <= ab.aMs) throw { kind: "invalid", message: "The loop has to end after it starts." };
      c.ab = ab && c.side === "b" ? compareMapLoop(ab, c.b.durationMs, c.a.durationMs) : ab;
      const now = c.ab && c.side === "b" ? compareMapLoop(c.ab, c.a.durationMs, c.b.durationMs) : c.ab;
      this.abLoop = now;
      this.emitState();
      return now;
    });
  }
  compareEnd() {
    return this.delay(() => {
      this.compareRun += 1;
      this.compare = null;
      this.unholdListening();
    });
  }

  eraPage(artistId: string, name: string) {
    return this.delay(() => this.archive.eraPage(artistId, name) ?? this.notFound("Era"));
  }
  albumSectionSet(albumId: string, artistId: string, name: string | null) {
    return this.delay(() => {
      this.archive.setSection(albumId, artistId, name);
      this.later(0, () => this.emit("artist://changed", { id: artistId }));
    });
  }
  artistSections(artistId: string) {
    return this.delay(() => this.archive.sectionNames(artistId));
  }

  // ---------------------------------------------------------------- library doctor

  doctorReport() {
    return this.delay(() => this.doctor.report());
  }
  doctorCount() {
    return this.delay(() => this.doctor.count());
  }
  doctorIgnore(id: string, kind: DoctorKind, title: string, ignore: boolean) {
    return this.delay(() => {
      this.doctor.ignore(id, kind, title, ignore);
      this.emitDoctor();
    });
  }
  doctorRun() {
    return this.delay(() => {
      this.doctor.running = true;
      this.emitDoctor();
      this.later(1200, () => {
        this.doctor.running = false;
        this.emitDoctor();
      });
    });
  }
  artistInfoKeys() {
    return this.delay(() => mockKeysStatus(this.infoKeys));
  }
  artistInfoKeySet(source: KeyedInfoSource, key: string | null) {
    return this.delay(() => {
      // The ListenBrainz token is the one scrobbling uses too.
      if (source === "listenbrainz") this.scrobble.setListenBrainzToken(!!key?.trim());
      return mockSetKey(this.infoKeys, source, key);
    });
  }
  artistInfoCacheClear() {
    return this.delay(() => {
      const n = this.infoFetched.size;
      this.infoFetched.clear();
      this.albumCreditsFetched.clear();
      this.sleeveFetched.clear();
      this.popularFetched.clear();
      return n;
    });
  }
  openInfoLink(url: string, _artistId?: string) {
    return this.delay(() => {
      if (typeof window !== "undefined" && !isTestEnv) window.open(url, "_blank", "noopener");
    });
  }

  // ---------------------------------------------------------------- sharing

  shareInfo(kind: ShareKind, id: string, atSeconds?: number) {
    return this.delay(() => this.share.info(kind, id, atSeconds));
  }
  sharePublicLink(kind: ShareKind, id: string) {
    return this.delay(() => this.share.publicLink(kind, id));
  }
  // ---------------------------------------------------------------- social

  friendActivity(_fresh?: boolean) {
    return this.delay(() => this.social.activity());
  }
  followsList() {
    return this.delay(() => this.social.list());
  }
  followSet(artist: { id: string; title: string; thumb?: string | null }, on: boolean) {
    return this.delay(() => this.social.set(artist, on));
  }
  whatsNew() {
    return this.delay(() => this.social.whatsNew());
  }
  whatsNewSeen() {
    return this.delay(() => this.social.markSeen());
  }
  // ---------------------------------------------------------------- shared playlists and Blends

  circleState() {
    return this.delay(() => this.circle.view());
  }
  circlePeople() {
    return this.delay(() => this.circle.people());
  }
  circleJoin(link: string) {
    return this.delay(() => this.circle.join(link.trim()));
  }
  circleAccept(id: string) {
    return this.delay(() => this.circle.accept(id));
  }
  circleDismiss(id: string) {
    return this.delay(() => this.circle.dismiss(id));
  }
  collabInfo(playlistId: string) {
    return this.delay(() => this.circle.info(playlistId));
  }
  collabInviteLink(playlistId: string, role: "collaborator" | "listener") {
    return this.delay(() => this.circle.inviteLink(playlistId, role));
  }
  collabInvitePerson(playlistId: string, key: string, role: "collaborator" | "listener") {
    return this.delay(() => this.circle.invitePerson(playlistId, key, role));
  }
  collabPage(id: string) {
    return this.delay(() => this.circle.page(id));
  }
  collabAdd(id: string, trackIds: string[]) {
    return this.delay(() => this.circle.add(id, trackIds));
  }
  collabRemove(id: string, keys: string[]) {
    return this.delay(() => this.circle.remove(id, keys));
  }
  collabLeave(id: string) {
    return this.delay(() => this.circle.leave(id));
  }
  collabSetInLibrary(id: string, on: boolean) {
    return this.delay(() => this.circle.setInLibrary(id, on));
  }
  collabSetRole(playlistId: string, personId: string, role: "collaborator" | "listener") {
    return this.delay(() => this.circle.setRole(playlistId, personId, role));
  }
  collabRemovePerson(playlistId: string, personId: string) {
    return this.delay(() => this.circle.removePerson(playlistId, personId));
  }
  collabSetPublic(playlistId: string, isPublic: boolean) {
    return this.delay(() => this.circle.setPublic(playlistId, isPublic));
  }
  blendCreate(keys: string[]) {
    return this.delay(() => this.circle.blendCreate(keys));
  }
  blendInviteLink(id: string) {
    return this.delay(() => this.circle.blendInviteLink(id));
  }
  blendInvitePerson(id: string, key: string) {
    return this.delay(() => this.circle.blendInvitePerson(id, key));
  }
  blendPage(id: string) {
    return this.delay(() => this.circle.blendPage(id));
  }
  blendHide(id: string, trackId: string) {
    return this.delay(() => this.circle.blendHide(id, trackId));
  }
  blendRefresh(id: string) {
    return this.delay(() => this.circle.blendRefresh(id));
  }
  blendLeave(id: string) {
    return this.delay(() => this.circle.blendLeave(id));
  }

  listenState() {
    return this.delay(() => this.social.listenState());
  }
  listenStart() {
    return this.delay(() => this.social.start());
  }
  listenStop() {
    return this.delay(() => this.social.stop());
  }
  listenJoin(link: string) {
    return this.delay(() => this.social.join(link));
  }
  listenAdd(trackIds: string[]) {
    return this.delay(() => this.social.add(trackIds));
  }
  listenInvite(fresh?: boolean) {
    return this.delay(() => this.social.invite(fresh ?? false));
  }
  listenControl(op: ListenOp) {
    return this.delay(() => this.social.control(op));
  }
  listenRemove(personId: string) {
    return this.delay(() => this.social.remove(personId));
  }
  listenOfferServer(personId: string) {
    return this.delay(() => this.social.offerServer(personId));
  }

  // ---------------------------------------------------------------- sharing

  sharingOverview() {
    return this.delay(() => this.sharing.overview());
  }
  sharingShare(invitee: string, options: ShareOptions) {
    return this.delay(() => this.sharing.share(invitee, options));
  }
  sharingPeople() {
    return this.delay(() => this.sharing.peopleList());
  }
  sharingRemove(id: string) {
    return this.delay(() => this.sharing.remove(id));
  }
  sharingLink(options: ShareOptions) {
    return this.delay(() => this.sharing.link(options));
  }
  sharingOpenLink(link: string) {
    return this.delay(() => this.sharing.openLink(link));
  }
  sharingLinkAsks() {
    return this.delay(() => [] as ShareLinkAsk[]);
  }
  sharingLinkApprove(_id: string) {
    return this.delay(() => "Shared.");
  }
  sharingLinkDeny(_id: string) {
    return this.delay(() => undefined);
  }
  sharingInvites() {
    return this.delay(() => this.sharing.invitesList());
  }
  sharingAccept(id: string) {
    return this.delay(() => this.sharing.accept(id));
  }

  deepLinkTake() {
    return this.delay(() => this.share.take());
  }

  // ---------------------------------------------------------------- share cards

  /** Cards put on the clipboard and saved (test hooks), and a save dialog that cancels. */
  cardsCopied: { width: number; height: number; bytes: number }[] = [];
  cardsSaved: { name: string; bytes: number }[] = [];
  cardSaveCancels = false;
  shareCardQr(kind: ShareKind, id: string, atSeconds?: number) {
    return this.delay(() => {
      const text = this.share.info(kind, id, atSeconds).appLink;
      return { ...mockQr(text), text };
    });
  }
  shareCardCopy(rgba: Uint8Array, width: number, height: number) {
    return this.delay(() => {
      if (rgba.length !== width * height * 4) throw { kind: "invalid", message: "That card image is incomplete." };
      this.cardsCopied.push({ width, height, bytes: rgba.length });
    });
  }
  shareCardSave(png: Uint8Array, name: string) {
    return this.delay(() => {
      if (this.cardSaveCancels) return null;
      const file = `${name.replace(/[<>:"/\\|?*]/g, " ").replace(/\s+/g, " ").trim() || "Platter card"}.png`;
      this.cardsSaved.push({ name: file, bytes: png.length });
      return file;
    });
  }

  // ---------------------------------------------------------------- miniplayer

  /** The miniplayer window is open (pretend: no second window in the browser). */
  miniplayerOpen = false;
  /** Test hook: the next open fails like a window that couldn't be made. */
  miniplayerFailsNext = false;
  miniplayerState() {
    return this.delay(() => ({ open: this.miniplayerOpen }));
  }
  miniplayerSet(open: boolean) {
    return this.delay(() => {
      if (open && this.miniplayerFailsNext) {
        this.miniplayerFailsNext = false;
        throw { kind: "internal", message: "The miniplayer couldn't open." };
      }
      if (open !== this.miniplayerOpen) {
        this.miniplayerOpen = open;
        this.emit("miniplayer://changed", { open });
      }
      return { open };
    });
  }
  /** Pages the miniplayer asked the main window to open, in order. */
  miniplayerWent: MiniplayerGo[] = [];
  miniplayerGo(kind: MiniplayerGo["kind"], id: string) {
    return this.delay(() => {
      this.miniplayerWent.push({ kind, id });
      this.emit("miniplayer://go", { kind, id });
    });
  }
  /** `?view=miniplayer` shows the miniplayer page in the browser. */
  windowLabel() {
    const view = typeof location !== "undefined" ? new URLSearchParams(location.search).get("view") : null;
    return view === "miniplayer" ? "miniplayer" : "main";
  }
  windowResizeFromCorner() {
    return Promise.resolve();
  }
  /** Background colours the page reported with `windowReady`, in order. */
  readySignals: Array<string | null> = [];
  windowReady(background: string | null) {
    this.readySignals.push(background);
    return Promise.resolve();
  }

  // ---------------------------------------------------------------- sync file, folders, keys

  syncFileStatus() {
    return this.delay(() => this.syncFile.status());
  }
  syncFileNow() {
    return this.delay(() => this.syncFile.status());
  }
  syncFileFolders() {
    return this.delay(() => MOCK_CLOUD_FOLDERS);
  }
  syncFileUseFolder(path: string) {
    return this.delay(() => this.syncFile.useFolder(path));
  }
  syncFileChooseFolder() {
    // Stands in for the folder dialog: picks an invented folder.
    return this.delay(() => this.syncFile.useFolder("C:\\Users\\listener\\Documents\\Synced"));
  }
  themesList() {
    return this.delay(() => this.themes.list());
  }
  themesImport() {
    // Stands in for the file picker: adds an invented theme file.
    return this.delay(() => this.themes.import());
  }
  themesOpenFolder() {
    return this.delay(() => undefined);
  }
  themesImportText(text: string) {
    return this.delay(() => this.themes.importText(text));
  }
  themesSave(look: ThemeFile, replace: boolean) {
    return this.delay(() => this.themes.save(look, replace));
  }
  /** The last settings file "saved", and the file the next import "opens" (tests set it; null cancels). */
  settingsFileSaved: SettingsFile | null = null;
  settingsFileToOpen: unknown = null;
  /** Links help pages opened, newest last (tests read them). */
  helpLinksOpened: string[] = [];
  helpOpenLink(url: string) {
    return this.delay(() => {
      this.helpLinksOpened.push(url);
    });
  }
  settingsDefaults() {
    return this.delay(() => ({ ...DEFAULT_SETTINGS }));
  }
  settingsExport() {
    // Stands in for the save dialog.
    return this.delay(() => {
      this.settingsFileSaved = exportSettings({ ...this.settings, volume: this.volume });
      return "Platter settings.json";
    });
  }
  async settingsImport() {
    if (this.settingsFileToOpen === null) return this.delay(() => null);
    const { patch, report } = importSettings(this.settingsFileToOpen);
    await this.settingsSet(patch);
    return report;
  }
  themesExport(look: ThemeFile) {
    // Stands in for the save dialog.
    return this.delay(() => this.themes.export(look));
  }
  syncFileReplace() {
    return this.delay(() => this.syncFile.replace());
  }
  pinsGet() {
    return this.delay(() => this.syncFile.pinsList());
  }
  pinsToggle(pin: PinRecord) {
    return this.delay(() => this.syncFile.pinsToggle(pin));
  }
  pinsImport(pins: PinRecord[]) {
    return this.delay(() => this.syncFile.pinsImport(pins));
  }
  automixMixedGet() {
    return this.delay(() => this.automix.list());
  }
  automixMixSet(id: string, on: boolean) {
    return this.delay(() => this.automix.set(id, on));
  }
  automixStyleSet(id: string, fromId: string, toId: string, style: MixStyle) {
    return this.delay(() => this.automix.setStyle(id, fromId, toId, style));
  }
  automixView(playlistId: string | null, trackIds: string[]) {
    return this.delay(() => this.automix.view(playlistId, trackIds));
  }
  foldersGet() {
    return this.delay(() => this.syncFile.tree());
  }
  folderCreate(name?: string | null, parent?: string | null) {
    return this.delay(() => this.syncFile.create(name, parent));
  }
  folderRename(id: string, name: string) {
    return this.delay(() => this.syncFile.rename(id, name));
  }
  folderDelete(id: string) {
    return this.delay(() => this.syncFile.remove(id));
  }
  folderMove(id: string, parent: string | null) {
    return this.delay(() => this.syncFile.move(id, parent));
  }
  playlistFolderSet(playlistId: string, folderId: string | null) {
    return this.delay(() => this.syncFile.place(playlistId, folderId));
  }
  keysAvailable() {
    return this.delay(() => [...this.syncFile.available]);
  }
  keysReceiveStart() {
    return this.delay(() => this.syncFile.receiveStart());
  }
  keysReceiveState() {
    return this.delay(() => this.syncFile.receiveState());
  }
  keysReceiveCancel() {
    return this.delay(() => this.syncFile.receiveCancel());
  }
  keysSend(code: string, kinds: KeyKind[]) {
    return this.delay(() => this.syncFile.send(code, kinds));
  }
  deviceSyncStatus() {
    return this.delay(() => this.deviceSync.overview());
  }
  deviceSyncNow() {
    return this.delay(() => this.deviceSync.syncNow());
  }
  deviceSyncForget(id: string) {
    return this.delay(() => this.deviceSync.forget(id));
  }
  deviceSyncQuarantined() {
    return this.delay(() => [...this.deviceSync.held]);
  }
  deviceSyncTakeQuarantined() {
    return this.delay(() => this.deviceSync.takeHeld());
  }
  devicePairStart() {
    return this.delay(() => this.deviceSync.pairStart());
  }
  devicePairCancel() {
    return this.delay(() => this.deviceSync.pairCancel());
  }
  devicePairConfirm(yes: boolean) {
    return this.delay(() => this.deviceSync.pairConfirm(yes));
  }
  devicePairState() {
    return this.delay(() => this.deviceSync.pairState());
  }
  devicePairJoin(code: string) {
    return this.delay(() => this.deviceSync.join(code));
  }
  deviceDownloadSuggestions() {
    return this.delay(() => [...this.deviceSync.suggestions]);
  }
  deviceDownloadDismiss(kind: DownloadKind, id: string | null) {
    return this.delay(() => this.deviceSync.dismiss(kind, id));
  }
  pairCopyCancel() {
    return this.delay(() => this.deviceSync.cancelCopy());
  }
  deviceSyncNetwork(metered: boolean | null) {
    return this.delay(() => this.deviceSync.network(metered));
  }
  pairCopySettings() {
    return this.delay(() => null).then(() => this.deviceSync.copySettings());
  }

  // ---------------------------------------------------------------- hide, bookmarks, smart playlists, scrobbling

  hiddenList() {
    return this.delay(() => this.hidden.list());
  }
  hiddenSet(item: HideRequest, on: boolean) {
    return this.delay(() => this.hidden.set(item, on));
  }
  bookmarksList() {
    return this.delay(() => this.bookmarks.list());
  }
  bookmarkAdd(bookmark: NewBookmark) {
    return this.delay(() => this.bookmarks.add(bookmark));
  }
  bookmarkRemove(id: string) {
    return this.delay(() => this.bookmarks.remove(id));
  }
  bookmarkOpen(id: string) {
    return this.delay(() => this.bookmarks.open(id));
  }
  smartPreview(rules: SmartRules, picks?: string[]) {
    return this.delay(() => this.smart.preview(rules, picks ?? []));
  }
  smartSave(request: SmartSaveRequest) {
    return this.delay(() => this.smart.save(request));
  }
  smartList() {
    return this.delay(() => this.smart.list());
  }
  smartPage(id: string) {
    return this.delay(() => this.smart.page(id));
  }
  smartDelete(id: string) {
    return this.delay(() => this.smart.delete(id));
  }
  smartPlexRules(id: string) {
    return this.delay(() => this.smart.plexRulesOf(id));
  }
  smartPrompt(prompt: string) {
    return this.delay(() => this.smart.prompt(prompt));
  }
  smartDescribe(text: string) {
    return this.delay(() => this.smart.describe(text));
  }
  aiStatus() {
    return this.delay(() => this.smart.aiStatus());
  }
  aiKeySet(key: string | null) {
    return this.delay(() => this.smart.aiKeySet(key));
  }
  scrobbleStatus() {
    return this.delay(() => this.scrobble.status());
  }
  lastfmSecretSet(secret: string | null) {
    return this.delay(() => this.scrobble.setSecret(secret));
  }
  lastfmSignInStart() {
    return this.delay(() => this.scrobble.start());
  }
  lastfmSignInFinish() {
    return this.delay(() => this.scrobble.finish());
  }
  lastfmSignOut() {
    return this.delay(() => this.scrobble.signOut());
  }
  listenbrainzCheck() {
    return this.delay(() => this.scrobble.check());
  }
  lidarrStatus() {
    return this.delay(() => this.lidarr.status(this.settings));
  }
  releaseRadar() {
    return this.delay(() => this.radar.page());
  }
  releaseRadarCheck() {
    return this.delay(() => this.radar.check());
  }
  radarAutoArtists() {
    return this.delay(() => this.radar.autoArtists());
  }
  radarAutoSet(id: string, name: string, on: boolean) {
    return this.delay(() => this.radar.setAuto(id, name, on));
  }
  lidarrKeySet(key: string | null) {
    return this.delay(() => this.lidarr.setKey(this.settings, key));
  }
  lidarrHeadersSet(text: string | null) {
    return this.delay(() => this.lidarr.setHeaders(this.settings, text));
  }
  lidarrCheck() {
    return this.delay(() => this.lidarr.check(this.settings, Math.floor(Date.now() / 1000)));
  }
  lidarrConfirmRoot(path: string, confirmed: boolean) {
    return this.delay(() => this.lidarr.confirmRoot(this.settings, path, confirmed));
  }
  lidarrRequestPreview(input: LidarrRequestInput) {
    return this.delay(() => this.lidarr.preview(this.settings, input));
  }
  lidarrRequest(input: LidarrRequestInput) {
    return this.delay(() => this.lidarr.request(this.settings, input, Math.floor(Date.now() / 1000)));
  }
  lidarrRequests(_refresh: boolean) {
    return this.delay(() => this.lidarr.list());
  }
  lidarrGet(input: LidarrRequestInput) {
    return this.delay(() => this.lidarr.get(this.settings, input, Math.floor(Date.now() / 1000)));
  }
  /** Taps in the mock: the song is searched, downloaded and lands in a few seconds. */
  taps: Tap[] = [];
  lastGet: LastGet | null = null;
  lidarrTap(input: TapInput) {
    return this.delay((): TapOutcome => {
      if (!this.settings.lidarrTapToGet) throw { kind: "invalid", message: "Getting songs with a tap is off. Turn it on in Settings > Lidarr > More." };
      const same = this.taps.find((t) => t.phase !== "failed" && t.releaseGroupId === (input.releaseGroupId ?? "") && t.title === input.title);
      this.taps.forEach((t) => (t.wanted = t === same));
      if (same) return { tap: { ...same }, needs: null };
      const out = this.lidarr.get(this.settings, {
        releaseGroupId: input.releaseGroupId ?? "",
        title: input.album ?? input.title,
        artist: input.artist,
        artistId: input.artistId ?? null,
        artistMbid: input.artistMbid ?? null,
        typed: input.typed ?? null,
      } as LidarrRequestInput, Math.floor(Date.now() / 1000));
      if (!out.record) return { tap: null, needs: out.needs };
      const tap: Tap = {
        id: `tap-${this.taps.length + 1}`,
        title: input.title,
        artist: input.artist,
        album: input.album ?? out.record.title,
        releaseGroupId: input.releaseGroupId ?? "",
        thumb: input.thumb ?? null,
        durationMs: input.durationMs ?? null,
        phase: "searching",
        progress: null,
        message: null,
        requestId: out.record.id,
        timings: { foundMs: null, downloadedMs: null, playingMs: null, inLibraryMs: null },
        wanted: true,
        fromDownload: false,
      };
      this.taps.unshift(tap);
      this.followTap(tap, Date.now());
      return { tap: { ...tap }, needs: null };
    });
  }
  private followTap(tap: Tap, t0: number) {
    const step = (ms: number, f: () => void) =>
      setTimeout(() => {
        f();
        this.emit("lidarr://tap", { ...tap });
      }, ms);
    step(700, () => {
      tap.phase = "downloading";
      tap.progress = 0;
      tap.timings.foundMs = Date.now() - t0;
    });
    for (let i = 1; i <= 5; i++) step(700 + i * 500, () => (tap.progress = i * 20));
    step(3400, () => {
      tap.phase = "playing";
      tap.progress = null;
      tap.fromDownload = true;
      tap.timings.downloadedMs = tap.timings.playingMs = Date.now() - t0;
    });
    step(5000, () => {
      tap.phase = "ready";
      tap.fromDownload = false;
      tap.timings.inLibraryMs = Date.now() - t0;
      this.lastGet = { title: tap.title, at: Math.floor(Date.now() / 1000), timings: { ...tap.timings } };
    });
  }
  lidarrTaps() {
    return this.delay(() => this.taps.map((t) => ({ ...t })));
  }
  lidarrLastGet() {
    return this.delay(() => this.lastGet);
  }
  lidarrSearchAgain(id: string) {
    return this.delay(() => this.lidarr.searchAgain(id, Math.floor(Date.now() / 1000)));
  }
  /** "Get albums" in the mock: queued albums, sent at once when Lidarr is ready. */
  getQueued: QueuedGet[] = [];
  getAlbumsUnqueue(releaseGroupIds: string[]) {
    return this.delay(() => {
      this.getQueued = releaseGroupIds.length ? this.getQueued.filter((q) => !releaseGroupIds.includes(q.releaseGroupId)) : [];
      return this.bringInStatus_();
    });
  }
  catalogueSearchPart(query: string, part: CataloguePartKind) {
    // Lidarr answers at once; MusicBrainz is paced, so its parts come later.
    const slow = this.catalogue.source(part) === "musicBrainz" ? (part === "songs" ? 500 : 250) : 0;
    return new Promise<void>((r) => setTimeout(r, slow)).then(() => this.delay(() => this.catalogue.searchPart(query, part)));
  }
  catalogueArtistPhoto(mbid: string, _name: string) {
    return this.delay(() => this.catalogue.photo(mbid));
  }
  featuredArtist(id: string, name: string) {
    return this.delay(() => this.catalogue.featured(id, name));
  }
  featuredArtistPhoto(id: string, name: string) {
    return this.delay(() => {
      const f = this.catalogue.featured(id, name);
      return { photo: f.photo, credit: f.photoCredit };
    });
  }
  catalogueGetAlbumsPlan(mbid: string, _name: string, kinds: string[]) {
    return this.delay(() => this.catalogue.getAlbumsPlan(mbid, kinds));
  }
  catalogueGetAlbums(mbid: string, _name: string, kinds: string[]) {
    return this.delay(() => {
      const plan = this.catalogue.getAlbumsPlan(mbid, kinds);
      const fresh = plan.albums.filter((a) => !this.getQueued.some((q) => q.releaseGroupId === a.releaseGroupId));
      this.getQueued.push(...fresh.map((a) => ({ releaseGroupId: a.releaseGroupId, title: a.title, artist: plan.artist, artistMbid: plan.artistMbid, artistId: null })));
      return { queued: fresh.length, plan };
    });
  }
  /** Mock "Bring in my listening": made-up progress while the setting is on. */
  bringIn = { userPaused: false, paused: null as string | null, bridge: null as string | null, needsYou: [{ key: "album:mira vale\u001fsecond bloom", artist: "Mira Vale", album: "Second Bloom", plays: 9, reason: "MusicBrainz lists this only as a bootleg or promo, not an official release. Platter never asks Lidarr for those.", at: 0 }] };
  private bringInStatus_() {
    const s = this.settings;
    return {
      on: s.bringIn,
      practice: false,
      ready: !!(s.lidarrUrl || s.lidarrAwayUrl),
      waves: s.bringInWaves,
      wave: 0,
      threshold: s.bringInWaves[0] ?? null,
      done: 12,
      remaining: 339,
      today: 7,
      perDay: s.bringInPerDay,
      paused: this.bringIn.paused,
      userPaused: this.bringIn.userPaused,
      finished: false,
      requestedTotal: 11,
      needsYou: this.bringIn.needsYou,
      practiced: [],
      queued: this.getQueued,
      bridge: this.bringIn.bridge,
    };
  }
  bringInStatus() {
    return this.delay(() => this.bringInStatus_());
  }
  bringInPause(paused: boolean) {
    return this.delay(() => {
      this.bringIn.userPaused = paused;
      return this.bringInStatus_();
    });
  }
  bringInNeedsYou(key: string, _retry: boolean) {
    return this.delay(() => {
      this.bringIn.needsYou = this.bringIn.needsYou.filter((n) => n.key !== key);
      return this.bringInStatus_();
    });
  }
  lidarrRequestBatch(inputs: LidarrRequestInput[]) {
    return this.delay(() => {
      const once = inputs.filter((x, i) => inputs.findIndex((y) => y.releaseGroupId.toLowerCase() === x.releaseGroupId.toLowerCase()) === i);
      if (!once.length) throw { kind: "invalid", message: "Pick at least one album to request." };
      if (once.length > 10) throw { kind: "invalid", message: "Request up to 10 albums at a time." };
      const now = Math.floor(Date.now() / 1000);
      return once.map((input) => {
        try {
          return { releaseGroupId: input.releaseGroupId, title: input.title, record: this.lidarr.request(this.settings, input, now), error: null };
        } catch (e) {
          return { releaseGroupId: input.releaseGroupId, title: input.title, record: null, error: (e as { message?: string }).message ?? "Not sent" };
        }
      });
    });
  }
  lidarrRequestCancel(id: string) {
    return this.delay(() => this.lidarr.cancel(id));
  }
  lidarrPick(input: LidarrPickInput) {
    return this.delay(() => this.lidarr.pick(this.settings, input));
  }
  lidarrUpgradeAlbum(albumId: string) {
    return this.delay(() => this.upgrades.forAlbum(albumId));
  }
  lidarrUpgradeRequest(albumId: string) {
    return this.delay(() => this.upgrades.request(albumId));
  }
  lidarrUpgradeBulk(under: UpgradeUnder) {
    return this.delay(() => this.upgrades.bulk(under));
  }
  lidarrUpgrades(refresh: boolean) {
    return this.delay(() => this.upgrades.list(refresh));
  }
  lidarrUpgradeOffer(id: string) {
    return this.delay(() => this.upgrades.offer(id));
  }
  lidarrUpgradeSwap(id: string, trackIds: string[] | null) {
    return this.delay(() => this.upgrades.swap(id, trackIds));
  }
  lidarrUpgradeUndo(id: string) {
    return this.delay(() => this.upgrades.undo(id));
  }
  lidarrUpgradeClear(id: string) {
    return this.delay(() => this.upgrades.clear(id));
  }
  lidarrUpgradeCandidates(albumId: string) {
    return this.delay(() => this.upgrades.candidates(albumId));
  }
  lidarrUpgradeConfirm(albumId: string, releaseGroupId: string | null) {
    return this.delay(() => this.upgrades.confirm(albumId, releaseGroupId));
  }
  lidarrAllowlist(artist: string, _artistId: string | null) {
    return this.delay(() => [...(this.upgrades.allowlists.get(artist.toLowerCase()) ?? [])]);
  }
  lidarrAllowlistSet(artist: string, _artistId: string | null, releaseGroupIds: string[]) {
    return this.delay(() => {
      const ids = [...new Set(releaseGroupIds.map((r) => r.trim().toLowerCase()).filter(Boolean))].sort();
      if (ids.length) this.upgrades.allowlists.set(artist.toLowerCase(), ids);
      else this.upgrades.allowlists.delete(artist.toLowerCase());
      return ids;
    });
  }
  musicShareGet() {
    return this.delay(() => structuredClone(this.upgrades.share));
  }
  musicShareSet(map: ShareMap) {
    return this.delay(() => this.upgrades.setShare(map));
  }
  musicSharePlexFolders() {
    return this.delay(() => ["/data/media/music"]);
  }
  lidarrMissingReleases(artistId: string, _artist: string) {
    return this.delay(() => this.lidarr.missing(this.settings, artistId));
  }
  lidarrModes() {
    return this.delay(() => this.lidarrMock.modes());
  }
  lidarrModeSet(key: string, name: string, mode: LidarrModeSetting, confirmations: number) {
    return this.delay(() => this.lidarrMock.setMode(key, name, mode, confirmations));
  }
  lidarrArtistChoices() {
    return this.delay(() => ({ ...this.lidarrMock.prefs }));
  }
  lidarrArtistChoicesSet(key: string, name: string, autoAdd: boolean | null, hunt: boolean, target: LidarrTarget) {
    return this.delay(() => this.lidarrMock.setChoices(key, name, autoAdd, hunt, target));
  }
  lidarrChangePreview(action: LidarrFixAction, input: string | null) {
    return this.delay(() => this.lidarrMock.propose(action, input));
  }
  lidarrHiresPreview() {
    return this.delay(() => this.lidarrMock.hiresPreview());
  }
  lidarrChangeApply(id: string) {
    return this.delay(() => this.lidarrMock.apply(id));
  }
  lidarrChanges() {
    return this.delay(() => this.lidarrMock.changes.map((c) => ({ ...c })));
  }
  lidarrChangePutBack(id: string) {
    return this.delay(() => this.lidarrMock.putBack(id));
  }
  lidarrRematchPreview(folder: string) {
    return this.delay(() => this.lidarrMock.rematch(folder));
  }
  lidarrRematchApply(folder: string, _releaseId: number | null) {
    return this.delay(() => this.lidarrMock.rematch(folder));
  }
  lidarrWork() {
    return this.delay(() => this.lidarrMock.work());
  }
  lidarrCatchUp() {
    return this.delay(() => this.lidarrMock.catchUp());
  }
  lidarrResumeProtected() {
    return this.delay(() => this.lidarrMock.resume());
  }
  lidarrStagedAdd(id: string, always: boolean) {
    return this.delay(() => this.lidarrMock.stagedAdd(id, always));
  }
  lidarrStagedUndo(id: string, trashOk: boolean) {
    return this.delay(() => this.lidarrMock.stagedUndo(id, trashOk));
  }
  lidarrHunt() {
    return this.delay(() => this.lidarrMock.hunt());
  }

  // ---------------------------------------------------------------- listening stats

  statsOverview(range: StatsRange, limit?: number) {
    return this.delay(() => this.stats.overview(range, limit));
  }
  statsSpan() {
    return this.delay(() => this.stats.span());
  }
  statsMoments(range: StatsRange) {
    return this.delay(() => this.stats.moments(range));
  }
  statsDays(from: string, to: string) {
    return this.delay(() => this.stats.days(from, to));
  }
  statsItem(kind: StatsItemKind, id: string) {
    return this.delay(() => this.stats.item(kind, id));
  }
  statsMonthRecap(month: string) {
    return this.delay(() => this.stats.monthRecap(month));
  }
  statsPlayIds(range: StatsRange, kind: StatsTopKind, limit?: number) {
    return this.delay(() => this.stats.playIds(range, kind, limit));
  }
  topSongsIndex() {
    return this.delay(() => this.stats.topSongsIndex(this.opts.today));
  }
  homeOnThisDay() {
    return this.delay(() => this.stats.onThisDay(this.opts.today));
  }
  homeTimeTravel() {
    return this.delay(() => this.stats.timeTravel());
  }
  homeRecords() {
    return this.delay(() => mockRecordHome(this.fx, this.settings));
  }
  topSongsList(year: number | null) {
    return this.delay(() => this.stats.topSongsList(year, this.opts.today));
  }
  statsWrapped(year: number) {
    return this.delay(() => this.stats.wrapped(year));
  }
  artistAliases() {
    return this.delay(() => this.stats.aliasList());
  }
  artistAliasSet(a: { key: string; name?: string; sameAs?: string; sameAsName?: string }, on: boolean) {
    return this.delay(() => this.stats.aliasSet(a, on));
  }
  artistAliasApart(p: { key: string; name?: string; other: string; otherName?: string }, on: boolean) {
    return this.delay(() => this.stats.aliasApart(p, on));
  }
  artistAliasAutoPairs() {
    return this.delay(() => (this.settings.artistAliasesAuto === false ? [] : [...this.stats.autoPairs]));
  }
  statsPlayedArtists(query: string, limit?: number) {
    return this.delay(() => this.stats.playedArtists(query, limit));
  }
  statsCompareSummary(unreleasedOk: boolean, theirUnreleasedOk: boolean) {
    return this.delay(() => this.stats.compareSummary(unreleasedOk, theirUnreleasedOk));
  }
  statsCompare(theirs: CompareSummary, unreleasedOk: boolean) {
    return this.delay(() => this.stats.compare(theirs, unreleasedOk));
  }
  statsFriends() {
    return this.delay(() => {
      if (!this.settings.statsCompare) throw { kind: "invalid", message: "Comparing stats with friends is off in Settings." };
      return clone(this.friends);
    });
  }
  statsFriendRemove(name: string) {
    return this.delay(() => {
      this.friends = this.friends.filter((f) => f.name !== name);
      return clone(this.friends);
    });
  }

  // ---------------------------------------------------------------- library import

  importOverview() {
    return this.delay(() => {
      const o = this.importMock.overview();
      return { ...o, missingCount: o.missingCount + this.catalogue.missingCount };
    });
  }
  importMissingAlbums() {
    return this.delay(() => this.catalogue.missingAlbums());
  }
  importMissingAlbumAdd(album: WantedAlbum) {
    return this.delay(() => this.catalogue.addMissing(album));
  }
  importMissingAlbumRemove(releaseGroupId: string) {
    return this.delay(() => this.catalogue.removeMissing(releaseGroupId));
  }

  // ---------------------------------------------------------------- discover

  catalogueSearch(query: string) {
    return this.delay(() => this.catalogue.search(query));
  }
  catalogueArtist(mbid: string) {
    return this.delay(() => this.catalogue.artistPage(mbid));
  }
  catalogueAlbum(releaseGroupId: string) {
    return this.delay(() => this.catalogue.albumPage(releaseGroupId));
  }
  catalogueFansAlsoLike(artistId: string, _name: string) {
    return this.delay(() => this.catalogue.fansAlsoLike(artistId));
  }
  newToYou() {
    return this.delay(() => this.catalogue.newToYou());
  }

  // ---------------------------------------------------------------- physical collection

  physicalList() {
    return this.delay(() => this.physical.list());
  }
  physicalSet(albumId: string, owned: SetOwned) {
    return this.delay(() => this.physical.set(albumId, owned));
  }
  physicalStats() {
    return this.delay(() => this.physical.stats());
  }
  physicalFindRelease(artist: string, title: string) {
    return this.delay(() => this.catalogue.findAlbum(artist, title));
  }
  playHistoryStatus() {
    return this.delay(() => this.playHistory.status());
  }
  playHistoryUpdate() {
    return this.delay(() => {
      const st = this.playHistory.update();
      this.later(0, () => this.emit("playhistory://status", st));
      return st;
    });
  }
  playHistoryCancel() {
    return this.delay(() => this.playHistory.status());
  }
  playHistoryRemove(source: PlayHistorySource) {
    return this.delay(() => this.playHistory.remove(source));
  }
  tautulliStatus() {
    return this.delay(() => this.playHistory.tautulli());
  }
  tautulliKeySet(key: string | null) {
    return this.delay(() => this.playHistory.setKey(key));
  }
  discogsStatus() {
    return this.delay(() => this.physical.status());
  }
  discogsTokenSet(token: string | null) {
    return this.delay(() => this.physical.setToken(token));
  }
  discogsImport() {
    return this.delay(() => this.physical.import());
  }
  importChoose(folder: boolean): Promise<string | null> {
    return this.delay(() => (folder ? "Downloads/my_data" : "Downloads/my_data.zip"));
  }
  importRun(source: ImportSourceId, path: string) {
    return this.delay(() => this.importMock.run(source, path));
  }
  private bringReady() {
    return this.lidarr.status(this.settings).ready;
  }
  bringStatus() {
    return this.delay(() => this.bring.status(this.bringReady()));
  }
  bringRead(path: string) {
    return this.delay(() => this.bring.read(path, this.bringReady()));
  }
  bringReadDropped(_bytes: Uint8Array, name: string) {
    return this.delay(() => this.bring.read(name, this.bringReady()));
  }
  bringAdopt(source: ImportSourceId) {
    return this.delay(() => this.bring.adopt(source, this.bringReady()));
  }
  bringGet(keys: string[] | null) {
    return this.delay(() => this.bring.get(keys, this.bringReady()));
  }
  bringLidarrGuesses() {
    return this.delay(() => ["http://10.20.30.40:8686", "http://localhost:8686"]);
  }
  bringWaitingRows(playlistId: string) {
    return this.delay(() => this.bring.waitingRows(playlistId));
  }
  importReviewQueue() {
    return this.delay(() => this.importMock.reviewQueue());
  }
  importReview(songId: number, decision: ImportDecision) {
    return this.delay(() => this.importMock.review(songId, decision));
  }
  importListPage(id: string) {
    return this.delay(() => this.importMock.listPage(id));
  }
  importMissing() {
    return this.delay(() => this.importMock.missingSongs());
  }
  importOpenLink(url: string) {
    return this.delay(() => {
      if (!importLinkAllowed(url)) throw { kind: "invalid", message: "link not allowed" };
      this.importLinksOpened.push(url);
    });
  }
  importListRemove(listId: string, songId: number) {
    return this.delay(() => this.importMock.removeFromList(listId, songId));
  }
  importPlayed(query: PlayedQuery) {
    return this.delay(() => {
      const f = (s: string) => s.trim().replace(/\s+/g, " ").toLowerCase();
      const reqs = this.lidarr.list();
      return this.importMock.played(query, (artist, album) => reqs.find((r) => f(r.artist) === f(artist) && f(r.title) === f(album))?.state ?? null);
    });
  }
  importPlayedHide(kind: PlayedHidden["kind"], artist: string, name: string | null, hide: boolean) {
    return this.delay(() => this.importMock.hide(kind, artist, name, hide));
  }
  importPlayedHidden() {
    return this.delay(() => this.importMock.hiddenList());
  }
  importForget(source: ImportSourceId) {
    return this.delay(() => this.importMock.forget(source));
  }
  private accountChanged(st: AccountStatus) {
    this.later(0, () => this.emit("streaming-account://status", st));
    return st;
  }
  streamingAccountStatus() {
    return this.delay(() => this.account.status());
  }
  streamingAccountSignIn() {
    const going = this.account.signIn();
    this.accountChanged(this.account.status());
    return this.delay(() => null)
      .then(() => going)
      .then((st) => {
        if (st.state === "connected") void this.account.read().then((r) => this.accountChanged(r));
        return this.accountChanged(clone(st));
      });
  }
  streamingAccountRead() {
    const going = this.account.read();
    this.accountChanged(this.account.status());
    return this.delay(() => null)
      .then(() => going)
      .then((st) => this.accountChanged(clone(st)));
  }
  streamingAccountCancel() {
    return this.delay(() => this.accountChanged(this.account.cancel()));
  }
  streamingAccountSignOut() {
    return this.delay(() => this.accountChanged(this.account.signOut()));
  }
  streamingAccountClientIdSet(id: string | null) {
    return this.delay(() => this.accountChanged(this.account.setClientId(id)));
  }

  // ---------------------------------------------------------------- discord

  discordStatus() {
    return this.delay(() => this.discord.status());
  }

  // ---------------------------------------------------------------- now-playing feed

  /** The feed as the core would report it for the current settings. */
  private feed(): FeedStatus {
    const s = this.settings;
    if (!s.nowPlayingFeed) return { state: "off" };
    const port = s.nowPlayingFeedPort;
    if (port === this.opts.feedPortInUse) return { state: "portInUse", port };
    return { state: "on", port, url: `http://127.0.0.1:${port}/now-playing.json` };
  }
  feedStatus() {
    return this.delay(() => this.feed());
  }

  // ---------------------------------------------------------------- platter bridge

  /** Settings > Bridge: a pair code is answered by a Bridge after a moment. */
  bridgeState: BridgeOverview = { bridges: [], pairing: null, queued: [] };
  /** How long the demo Bridge takes to answer a pair code (ms). */
  bridgePairAfterMs = 2500;
  bridgeOverview() {
    return this.delay(() => this.bridgeState);
  }
  /** Settings > Lidarr > Home side: computers at home that answer (tests set it) and the Bridges. */
  homeComputers: { name: string; on: boolean }[] = [];
  lidarrHomeSides() {
    return this.delay((): HomeSides => {
      const sides: HomeSides["sides"] = [
        ...this.homeComputers.map((c) => ({ kind: "computer" as const, name: c.name, words: c.on ? "Your computer at home, on now" : "Your computer at home, off or away now", on: c.on, answers: false })),
        ...this.bridgeState.bridges.map((b) => ({ kind: "bridge" as const, name: b.name, words: b.online ? "Platter Bridge, on all the time" : "Platter Bridge, not heard from lately", on: b.online, answers: false })),
      ];
      const first = sides.find((s) => s.on);
      if (first) first.answers = true;
      const offering = this.settings.homeSideLidarr && this.lidarr.status(this.settings).hasKey && !!(this.settings.lidarrUrl.trim() || this.settings.lidarrAwayUrl.trim());
      const away = !!this.settings.lidarrAwayUrl.trim();
      const words = this.bridgeState.bridges.length
        ? "Your Bridge isn't answering, so Lidarr can't be reached away from home yet. Platter is linking to it again by itself. If this is still here in a few minutes, check that the Bridge is running on your server."
        : "Lidarr isn't reachable away from home. Turn on a computer at home or add the Bridge.";
      const problem = !first && !offering && !away ? words : null;
      return { offering, sides, problem };
    });
  }
  bridgePairStart() {
    return this.delay(() => {
      const now = Math.floor(Date.now() / 1000);
      const pairing = { code: "PB1-q3Vx8bN2kLm9TzR4wYc7HdE1fGj5sAo6pUi0eWn3vXb8yZaQk", expiresAt: now + 3600 };
      this.bridgeState = { ...this.bridgeState, pairing };
      this.later(this.bridgePairAfterMs, () => {
        if (this.bridgeState.pairing?.code !== pairing.code) return;
        const at = Math.floor(Date.now() / 1000);
        this.bridgeState = {
          ...this.bridgeState,
          pairing: null,
          bridges: [
            ...this.bridgeState.bridges,
            {
              id: `b${this.bridgeState.bridges.length + 1}`,
              name: "Platter Bridge",
              version: "0.2.20",
              pairedAt: at,
              lastSeen: at,
              online: true,
              music: "readWrite",
              status: {
                name: "Platter Bridge",
                version: "0.2.20",
                at,
                doing: "Waiting for the next catch-up",
                lastCatchUp: { at, handled: 3, acted: 1, scanned: 1, announced: ["Second Wind"], healed: 0, paused: null, error: null },
                music: "readWrite",
                plex: true,
                lidarr: true,
                problem: null,
                fromEnv: [],
                plexLink: null,
                musicPlexPath: null,
                lyrics: { state: "working", done: 1204, left: 3398, hours: "night", song: "Second Wind", problem: null },
              },
            },
          ],
        };
        this.emit("bridge://changed", null);
      });
      return pairing;
    });
  }
  bridgePairCancel() {
    return this.delay(() => {
      this.bridgeState = { ...this.bridgeState, pairing: null };
    });
  }
  bridgeUnpair(id: string) {
    return this.delay(() => {
      this.bridgeState = { ...this.bridgeState, bridges: this.bridgeState.bridges.filter((b) => b.id !== id) };
      return this.bridgeState;
    });
  }
  bridgeQueuedApply(id: string) {
    return this.delay(() => {
      this.bridgeState = { ...this.bridgeState, queued: this.bridgeState.queued.filter((q) => q.id !== id) };
      return this.bridgeState;
    });
  }
  bridgeQueuedDismiss(id: string) {
    return this.bridgeQueuedApply(id);
  }
  bridgeSendSettings() {
    return this.delay(() => undefined);
  }
  /** A Bridge on the same Plex account turns up by itself (no code). */
  bridgeFound() {
    void this.bridgeJoin("PBB-q3Vx8bN2kLm9TzR4wYc7HdE1fGj5sAo6pUi0eWn3vXb8yZaQk").then(() => {
      const bridges = this.bridgeState.bridges;
      this.bridgeState = { ...this.bridgeState, bridges: bridges.map((b, i) => (i === bridges.length - 1 ? { ...b, auto: true } : b)) };
      this.emit("bridge://changed", null);
    });
  }
  /** "Add the Bridge": a code the Bridge showed (PBB-) pairs it after a moment. */
  bridgeJoin(code: string) {
    return this.delay(() => {
      const c = code.replace(/\s+/g, "");
      if (!c.startsWith("PBB-") || c.length < 20) {
        throw { kind: "invalid", message: "That isn't a Bridge code. Copy it again from the Bridge's log: it starts with PBB-." };
      }
      const at = Math.floor(Date.now() / 1000);
      this.bridgeState = {
        ...this.bridgeState,
        bridges: [
          ...this.bridgeState.bridges,
          {
            id: `b${this.bridgeState.bridges.length + 1}`,
            name: "Shed Bridge",
            version: "0.2.20",
            pairedAt: at,
            lastSeen: at,
            online: true,
            music: "readOnly",
            status: {
              name: "Shed Bridge",
              version: "0.2.20",
              at,
              doing: "Waiting for the next catch-up",
              lastCatchUp: null,
              music: "readOnly",
              plex: false,
              lidarr: true,
              problem: null,
              fromEnv: ["lidarrUrl", "lidarrKey"],
              plexLink: "K3QZ",
              musicPlexPath: "/data/media/Music",
              lyrics: { state: "waiting", done: 0, left: 4602, hours: "night", song: null, problem: null },
            },
          },
        ],
      };
      this.emit("bridge://changed", null);
      return "Shed Bridge";
    });
  }

  // ---------------------------------------------------------------- stream deck

  private deckCode = "482913";
  private deckPaired = false;
  private autoCode = "250617";
  autoPaired = false;
  private deck(): DeckStatus {
    const deck = this.settings.allowStreamDeck;
    const auto = this.settings.allowAutomation;
    const paired = { paired: this.deckPaired, automationPaired: this.autoPaired };
    if (!deck && !auto) return { state: "off", code: null, automationCode: null, ...paired };
    return { state: "on", port: 32541, code: deck ? this.deckCode : null, automationCode: auto ? this.autoCode : null, ...paired };
  }
  automationRevoke() {
    return this.delay(() => {
      this.autoPaired = false;
      this.autoCode = String((Number(this.autoCode) + 7919) % 1000000).padStart(6, "0");
      const st = this.deck();
      this.later(0, () => this.emit("deck://status", st));
      return st;
    });
  }
  deckStatus() {
    return this.delay(() => this.deck());
  }
  /** What the window last told the Stream Deck keys. */
  deckUi: { lyrics: boolean } = { lyrics: false };
  deckUiReport(report: { lyrics: boolean }) {
    return this.delay(() => {
      this.deckUi = report;
    });
  }
  deckRevoke() {
    return this.delay(() => {
      this.deckPaired = false;
      this.deckCode = String((Number(this.deckCode) + 104729) % 1000000).padStart(6, "0");
      const st = this.deck();
      this.later(0, () => this.emit("deck://status", st));
      return st;
    });
  }
  /** Test hooks: what the plugin detector finds, and what was opened. */
  readonly deckPlugin = { status: { state: "install", version: "0.2.0.0" } as DeckPluginStatus, opened: 0, downloadOpened: 0 };
  private deckPrompt: string | null = null;
  /** Test hook: the Stream Deck asks to pair. */
  askDeckPairing(id = "req1") {
    this.deckPrompt = id;
    this.emit("deck://pair-request", { id });
  }
  deckPairPrompt() {
    return this.delay(() => this.deckPrompt);
  }
  deckPairAnswer(id: string, allow: boolean) {
    return this.delay(() => {
      if (id !== this.deckPrompt) return false;
      this.deckPrompt = null;
      this.later(0, () => this.emit("deck://pair-request", { id: null }));
      if (allow) {
        this.deckPaired = true;
        const st = this.deck();
        this.later(0, () => this.emit("deck://status", st));
      }
      return true;
    });
  }
  deckPluginStatus() {
    return this.delay(() => this.deckPlugin.status);
  }
  deckPluginInstall() {
    return this.delay(() => {
      if (this.deckPlugin.status.state === "noApp" || this.deckPlugin.status.state === "notBundled") throw new Error("notFound");
      this.deckPlugin.opened++;
    });
  }
  deckAppDownload() {
    return this.delay(() => {
      this.deckPlugin.downloadOpened++;
    });
  }
  discordImgurKeySet(key: string | null) {
    return this.delay(() => this.discord.setKey(key));
  }

  // ---------------------------------------------------------------- identify by sound

  /** Test hooks: whether the build has a key, the user's key, a refused key. */
  readonly soundId = { appKey: false, userKey: null as string | null, refused: false };

  private soundIdState(): SoundIdStatus {
    const s = this.settings;
    const scope = s.soundIdScope ?? "untagged";
    const total = scope === "all" ? this.fx.tracks.length : Math.round(this.fx.tracks.length / 4);
    const on = s.identifyBySound === true && !s.goOffline;
    const configured = this.soundId.appKey || this.soundId.userKey !== null;
    const done = on && configured ? Math.round(total * 0.6) : 0;
    return {
      on,
      appKey: this.soundId.appKey,
      userKey: this.soundId.userKey !== null,
      keyRefused: this.soundId.refused,
      configured,
      running: false,
      paused: false,
      total,
      done,
      identified: Math.round(done * 0.7),
    };
  }
  soundIdStatus() {
    return this.delay(() => this.soundIdState());
  }
  soundIdKeySet(key: string | null) {
    return this.delay(() => {
      const k = key?.trim() ?? "";
      if (k && !/^[A-Za-z0-9_-]{1,100}$/.test(k)) throw { kind: "invalid", message: "That doesn't look like an AcoustID application key." };
      this.soundId.userKey = k || null;
      this.soundId.refused = false;
      return this.soundIdState();
    });
  }
  trackSoundsLike(id: string) {
    return this.delay((): SoundsLike | null => {
      if (this.settings.identifyBySound !== true) return null;
      const t = this.fx.tracks.find((x) => x.id === id);
      if (!t || t.artistId !== ARCHIVE_ARTIST_ID) return null;
      const released = this.fx.tracks.find((x) => x.artistId !== ARCHIVE_ARTIST_ID);
      return {
        trackId: id,
        recordingMbid: "0f0e0d0c-0b0a-4909-8807-060504030201",
        title: released?.title ?? "Night Drive",
        artist: released?.artistTitle ?? "Ana",
        album: released?.albumTitle ?? null,
        score: 91,
        unreleased: true,
        inLibrary: released ? { trackId: released.id, albumId: released.albumId } : null,
      };
    });
  }

  // ---------------------------------------------------------------- file tags

  tagsIndex() {
    return this.delay(() => this.tags.index());
  }
  tagScanStatus() {
    return this.delay(() => this.tags.scanStatus());
  }

  transcribeStatus() {
    return this.delay(() => this.transcribe.status());
  }
  transcribeModelDownload(model: TranscribeModel) {
    return this.delay(() => this.transcribe.downloadModel(model));
  }
  transcribeModelCancel() {
    return this.delay(() => this.transcribe.cancelDownload());
  }
  transcribeModelDelete(model: TranscribeModel) {
    return this.delay(() => this.transcribe.deleteModel(model));
  }
  transcribeTrack(trackId: string) {
    return this.delay(() => {
      this.trackById(trackId) ?? this.notFound("Track");
      this.transcribe.transcribeTrack(trackId);
    });
  }
  transcribeAlbum(albumId: string) {
    return this.delay(() => this.transcribe.transcribeAlbum(albumId));
  }
  transcribeUnreleased() {
    return this.delay(() => this.transcribe.transcribeUnreleased());
  }
  transcribeCancel(trackId?: string) {
    return this.delay(() => this.transcribe.cancel(trackId));
  }
  transcribePause(paused: boolean) {
    return this.delay(() => this.transcribe.pause(paused));
  }
  transcribeRedo() {
    return this.delay(() => this.transcribe.redo());
  }
  transcribeFoundNothing(trackId: string) {
    return this.delay(() => this.transcribe.foundNothing(trackId));
  }

  lyricsShareInfo(trackId: string) {
    return this.delay(() => this.lyricsShare.info(trackId));
  }
  lyricsShareReviewed(trackId: string) {
    return this.delay(() => this.lyricsShare.markReviewed(trackId));
  }
  lyricsSharePublish(trackId: string) {
    return this.delay(() => null).then(() => this.lyricsShare.publish(trackId));
  }
  lyricsShareCancel(trackId: string) {
    return this.delay(() => this.lyricsShare.cancel(trackId));
  }

  // ---------------------------------------------------------------- downloads

  downloadsStatus() {
    return this.delay(() => this.downloads.status());
  }
  downloadedTracks() {
    return this.delay(() => this.downloads.downloadedIds());
  }
  downloadsSet(kind: DownloadKind, id: string, on: boolean, opts?: { title?: string; trackIds?: string[] }) {
    return this.delay(() => {
      // Liked Songs is the "Download Liked Songs" setting.
      if (kind === "liked") this.settings = { ...this.settings, downloadLiked: on };
      return this.downloads.set(kind, id, on, opts?.title, opts?.trackIds);
    });
  }
  downloadsRetry(kind?: DownloadKind, id?: string) {
    return this.delay(() => this.downloads.retry(kind, id));
  }
  downloadsRemoveAll() {
    return this.delay(() => {
      this.settings = { ...this.settings, downloadLiked: false };
      return this.downloads.removeAll();
    });
  }
  downloadsChooseFolder() {
    return this.delay(() => {
      const folder = this.downloads.chooseFolder();
      this.settings = { ...this.settings, downloadFolder: folder };
      return folder as string | null;
    });
  }

  // ---------------------------------------------------------------- sync to a device

  devicesList() {
    return this.delay(() => this.devices.view());
  }
  devicesAdd(root: string, name: string, folder: string) {
    return this.delay(() => this.devices.add(root, name, folder));
  }
  devicesChooseFolder(_root: string) {
    return this.delay(() => "Music" as string | null);
  }
  devicesUpdate(id: string, patch: DevicePatch) {
    return this.delay(() => this.devices.update(id, patch));
  }
  devicesSelect(id: string, kind: DeviceItemKind, itemId: string, title: string, on: boolean) {
    return this.delay(() => this.devices.select(id, kind, itemId, title, on));
  }
  devicesForget(id: string) {
    return this.delay(() => this.devices.forget(id));
  }
  devicesSync(id: string) {
    return this.delay(() => this.devices.sync(id));
  }
  devicesPause(on: boolean) {
    return this.delay(() => this.devices.pause(on));
  }
  devicesCancel() {
    return this.delay(() => this.devices.cancel());
  }
  devicesStatus() {
    return this.delay(() => this.devices.status);
  }

  // ---------------------------------------------------------------- sound map

  soundmapInfo() {
    return this.delay(() => this.soundmap.info());
  }
  soundmapPoints(after?: string | null, limit?: number, area?: SoundMapArea | null) {
    return this.delay(() => this.soundmap.points(after, limit, area));
  }
  soundmapFind(query: string) {
    return this.delay(() => this.soundmap.find(query));
  }
  soundmapSong(id: string) {
    return this.delay(() => {
      const t = this.trackById(id);
      return this.soundmap.song(t, t ? (this.fx.albums.find((a) => a.id === t.albumId)?.thumb ?? null) : null);
    });
  }
}

export function createMockBackend(opts?: Partial<MockOptions>): MockBackend {
  const fromUrl = typeof window !== "undefined" ? optionsFromUrl(window.location.search) : optionsFromUrl("");
  const mock = new MockBackend({ ...fromUrl, ...opts });
  // An empty library: the parts that read the fixtures see nothing.
  if ({ ...fromUrl, ...opts }.emptyLibrary) {
    const fx = mock.fx;
    fx.artists.length = 0;
    fx.albums.length = 0;
    fx.tracks.length = 0;
    fx.playlists.length = 0;
    fx.playlistTracks.clear();
    fx.tracksByAlbum.clear();
  }
  if (typeof window !== "undefined") (window as unknown as { __platterMock?: MockBackend }).__platterMock = mock;
  return mock;
}
