/**
 * The only door from the UI to the Rust core (see docs/IPC.md).
 *
 * Inside the Tauri app every call is a Tauri command / event. In a plain
 * browser (Vite dev server, Playwright, Vitest) an in-memory mock with fixture
 * data stands in, so every screen is demoable and testable without a server.
 * Production builds never use the mock, so it is left out of the bundle.
 */
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { artPixelSize } from "./artSize";
import { createMockBackend } from "./mock";
import type {
  DiscogsReport,
  AutomixView,
  MixStyle,
  MixedPlaylists,
  DiscogsStatus,
  PlayHistorySource,
  PlayHistoryStatus,
  TautulliStatus,
  OwnedAlbum,
  SetOwned,
  ShelfStats,
  CatAlbum,
  CatArtist,
  CatalogueAlbumPage,
  CatalogueArtistPage,
  CatalogueSearch,
  CataloguePart,
  CataloguePartKind,
  CataloguePhoto,
  GetAlbumsPlan,
  LidarrGetOutcome,
  MissingAlbum,
  NewToYou,
  WantedAlbum,
  CachedLibrary,
  AbLoop,
  AddPlace,
  RemovedItem,
  RemoteCommand,
  RemotePlayer,
  RemoteState,
  Album,
  AlbumPage,
  AlbumsArgs,
  AppError,
  AppInfo,
  ArtCacheInfo,
  Artist,
  ArtistName,
  ArtistInfo,
  AlbumCredits,
  AlbumReleaseStatus,
  EraPage,
  DoctorKind,
  DoctorReport,
  SoundIdStatus,
  SoundsLike,
  ReleaseMark,
  VersionItem,
  Lineage,
  CompareReady,
  CompareNow,
  CompareSide,
  ArtistPage,
  ArtistsArgs,
  AuthStatus,
  BackendCaps,
  QuickCode,
  QuickPoll,
  ResumeOffer,
  SavedServer,
  ServerCheck,
  SubsonicCheck,
  ServerLibrary,
  EventMap,
  EventName,
  Health,
  HomeUsers,
  LibraryServers,
  LibrarySources,
  HomeData,
  HomeItem,
  RecentsEntry,
  HomeShelfId,
  MixPage,
  MixShelves,
  MixSteer,
  InfoKeyState,
  InfoKeysStatus,
  KeyedInfoSource,
  LibraryItem,
  CardQr,
  MiniplayerState,
  LikedPage,
  Lyrics,
  LyricsCandidates,
  LyricsCheckStatus,
  OwnLyrics,
  NowPlayingDetails,
  OutputState,
  HeadphoneEntry,
  Page,
  PinInfo,
  PlayContextArgs,
  PlayerState,
  Playlist,
  PlaylistPage,
  RepeatMode,
  SearchArgs,
  SearchHit,
  SearchKind,
  SearchResults,
  ServerInfo,
  Settings,
  SettingsImportReport,
  DeepLinkOpen,
  DiscordStatus,
  FeedStatus,
  DeckPluginStatus,
  DeckStatus,
  BridgeOverview,
  BridgePairStart,
  DownloadKind,
  DownloadStatus,
  Device,
  DeviceItemKind,
  DevicePatch,
  DeviceSyncStatus,
  DevicesView,
  SoundMapArea,
  SoundMapChunk,
  SoundMapFound,
  SoundMapInfo,
  SoundMapSong,
  ShareInfo,
  ShareKind,
  SyncStatus,
  SyncFileStatus,
  CloudFolder,
  PinRecord,
  LibraryFolder,
  LibraryFolders,
  KeyKind,
  KeyReceiveState,
  KeySendOutcome,
  UpdateInfo,
  TagIndex,
  TagScanStatus,
  TranscribeModel,
  TranscribeStatus,
  LyricsShareInfo,
  QueueAt,
  Track,
  ActivityPanel,
  FollowedArtist,
  WhatsNew,
  ListenView,
  InviteAccepted,
  PersonWithAccess,
  ReceivedInvite,
  ShareLinkAsk,
  ShareLinkResult,
  ShareLinkView,
  ShareOptions,
  ShareStatus,
  SharingOverview,
  ListenOp,
  CircleView,
  CircleAccount,
  JoinResult,
  SharedInfo,
  InviteLink,
  SharedPage,
  BlendCreated,
  BlendPage,
  AiStatus,
  LidarrMissing,
  AlbumUpgrade,
  ShareMap,
  SwapResult,
  UpgradeBulkResult,
  UpgradeCandidates,
  UpgradeConfirmed,
  UpgradeOffer,
  UpgradeRecord,
  UpgradeUnder,
  LidarrPickInput,
  LidarrPicked,
  LidarrPreview,
  LidarrReport,
  LidarrArtistMode,
  LidarrArtistPrefs,
  LidarrCatchUpReport,
  LidarrChange,
  LidarrFixAction,
  LidarrHuntItem,
  LidarrModeSetting,
  LidarrProposal,
  LidarrRematchPlan,
  LidarrStaged,
  LidarrTarget,
  LidarrWork,
  LidarrRequest,
  LidarrBatchItem,
  BringInStatus,
  LidarrRequestInput,
  LidarrStatus,
  RadarPage,
  AutoArtist,
  Bookmark,
  HiddenItem,
  HideRequest,
  NewBookmark,
  PromptResult,
  Described,
  ScrobbleStatus,
  SmartPage,
  SmartPreview,
  SmartRules,
  SmartSaveRequest,
  SmartSaved,
  SmartSummary,
  ThemeFile,
  ThemeFolder,
  EditAccess,
  EditChange,
  EditFields,
  EditInfo,
  IsrcRelease,
  IsrcReleases,
  MusicFolderCheck,
  PlexLibraryCheck,
  TagCheckReport,
  TagCheckScope,
  TagEditorData,
  TagFixKind,
  TagFixPreview,
  TagSaveRequest,
  TagSaveResult,
  TagUndoInfo,
  EditKind,
  ImageChoice,
  ImageKind,
  MatchCandidate,
} from "./types";
import type {
  ImportDecision,
  ImportOverview,
  ImportReport,
  ImportReviewItem,
  ImportSourceId,
  PlayedHidden,
  PlayedPage,
  PlayedQuery,
  ImportedListPage,
  MissingSong,
} from "./importTypes";
import type {
  CompareResult,
  CompareSummary,
  ItemStats,
  MonthRecap,
  StatsItemKind,
  StatsOverview,
  StatsRange,
  StatsMoments,
  StatsPoint,
  StatsSpan,
  StatsTopKind,
  TopSongsIndex,
  TopSongsList,
  Wrapped,
} from "./statsTypes";

export type Unlisten = () => void;

export interface Backend {
  readonly kind: "tauri" | "mock";

  // Auth & servers
  authStatus(): Promise<AuthStatus>;
  authStart(): Promise<PinInfo>;
  authCancel(): Promise<void>;
  authSignOut(): Promise<void>;
  serversList(): Promise<ServerInfo[]>;
  serverSelect(machineId: string): Promise<void>;
  /** The account's Plex Home users (empty when it isn't a Home). */
  homeUsers(): Promise<HomeUsers>;
  /** Switch to a Home user, with their PIN when they have one. The app restarts as them. */
  homeSwitch(uuid: string, pin?: string): Promise<void>;
  /** Settings: remember the last Home user on this device. */
  homeRememberSet(on: boolean): Promise<void>;
  /** Settings, Library: every server and music library the account can reach. Asks each server. */
  librarySources(): Promise<LibrarySources>;
  /** The servers the library has music from (the sidebar switcher). Doesn't ask the servers. */
  libraryServers(): Promise<LibraryServers>;
  /** What the servers in use can do, and their names. */
  backendCaps(): Promise<BackendCaps>;
  /** Settings, Music servers: every saved server, the primary marked. */
  serversSaved(): Promise<SavedServer[]>;
  serverSetPrimary(tag: string): Promise<void>;
  /** Removes a server added by address (its sign-in and, after a sync, its music). */
  serverRemove(tag: string): Promise<void>;
  /** A server's music libraries and which are included. */
  serverLibraries(tag: string): Promise<ServerLibrary[]>;
  /** Include these libraries; null includes all of them. */
  serverLibrariesSet(tag: string, ids: string[] | null): Promise<void>;
  /** Is there a Jellyfin server at this address, and can it use Quick Connect? */
  jellyfinCheck(url: string): Promise<ServerCheck>;
  /** A Quick Connect code to approve in another Jellyfin app. */
  jellyfinQuickStart(url: string): Promise<QuickCode>;
  /** Asks whether the code was approved; once it is, the server is added. */
  jellyfinQuickPoll(): Promise<QuickPoll>;
  jellyfinQuickCancel(): Promise<void>;
  /** Signs in with a password, used once and never kept. */
  jellyfinSignInPassword(url: string, user: string, password: string): Promise<SavedServer>;
  /** Is there a Navidrome or other Subsonic server at this address, and does it take API keys? */
  subsonicCheck(url: string): Promise<SubsonicCheck>;
  /** Signs in with an API key, or a user name and password (kept in the keychain). */
  subsonicSignIn(url: string, creds: { apiKey?: string; user?: string; password?: string }): Promise<SavedServer>;
  /** Queues other apps saved on servers that keep one. */
  resumeQueues(): Promise<ResumeOffer[]>;
  /** Plays the queue saved on this server, from where it was. */
  resumeQueuePlay(tag: string): Promise<void>;
  /** Saves the queue on the playing song's server now; false when it keeps none. */
  resumeQueueSave(): Promise<boolean>;

  // Library
  syncStatus(): Promise<SyncStatus>;
  syncNow(full?: boolean): Promise<void>;
  home(): Promise<HomeData>;
  /** Everything in one Home shelf, for its "Show all" page. */
  homeShelf(shelf: HomeShelfId, limit?: number): Promise<HomeItem[]>;
  artists(args: ArtistsArgs): Promise<Page<Artist>>;
  /** Every artist name and id (builds the artist links in track lines). */
  artistNames(): Promise<ArtistName[]>;
  albums(args: AlbumsArgs): Promise<Page<Album>>;
  playlists(): Promise<Playlist[]>;
  /** Every playlist, album and artist, for the library sidebar. */
  libraryItems(): Promise<LibraryItem[]>;
  artistPage(id: string): Promise<ArtistPage>;
  albumPage(id: string): Promise<AlbumPage>;
  playlistPage(id: string): Promise<PlaylistPage>;
  /** Songs played on this device, newest first, repeats kept (the queue panel's Recently played). */
  recentTracks(limit?: number): Promise<Track[]>;
  /** Listening sessions and likes for the Recents page, newest first. */
  recents(limit?: number): Promise<RecentsEntry[]>;

  // Mixes and radio (docs/IPC.md "Mixes and radio")
  /** Home's mix shelves (made when due). */
  mixesHome(): Promise<MixShelves>;
  /** A mix and its songs; `steer` leans the same mix one way. */
  mixPage(id: string, steer?: MixSteer | null): Promise<MixPage>;
  /** Make one mix again with new songs. */
  mixRefresh(id: string): Promise<MixPage>;
  /** A radio page: `track:<id>` or `artist:<id>`, about 50 songs starting with the seed (the same `salt`, the same songs). */
  radioPage(seed: string, steer?: MixSteer | null, salt?: number): Promise<MixPage>;

  // Playlist editing (docs/IPC.md "Playlist editing")
  /** A new playlist, named from Settings unless `title` is given, holding `trackIds`. */
  playlistCreate(title?: string, trackIds?: string[]): Promise<Playlist>;
  /** Add songs at the end, the top, or after a row. */
  playlistAdd(id: string, trackIds: string[], place?: AddPlace): Promise<Playlist>;
  /** Which of these songs the playlist already has. */
  playlistDuplicates(id: string, trackIds: string[]): Promise<string[]>;
  playlistRemove(id: string, itemIds: string[]): Promise<RemovedItem[]>;
  /** Move rows (in their order) to just after `after`, or to the top without it. */
  playlistMove(id: string, itemIds: string[], after?: string | null): Promise<void>;
  /** Rename and/or change the description ("" clears it). */
  playlistUpdate(id: string, details: { title?: string; summary?: string }): Promise<Playlist>;
  playlistSetCover(id: string, bytes: Uint8Array, contentType: string): Promise<void>;
  playlistDelete(id: string): Promise<void>;

  // Edit in Plex (docs/IPC.md "Edit in Plex")
  /** Whether editing is on, and which servers this account can edit. */
  plexEditAccess(): Promise<EditAccess>;
  /** An album or artist's editable fields, locks, match and release status. */
  plexEditItem(id: string): Promise<EditInfo>;
  /** The images Plex offers (posters, or an artist's backgrounds). */
  plexEditImages(id: string, which: ImageKind): Promise<ImageChoice[]>;
  plexEditSelectImage(id: string, kind: EditKind, which: ImageKind, key: string): Promise<EditChange>;
  plexEditUploadImage(id: string, kind: EditKind, which: ImageKind, bytes: Uint8Array, contentType: string): Promise<EditChange>;
  plexEditImageLink(id: string, kind: EditKind, which: ImageKind, url: string): Promise<EditChange>;
  plexEditMatches(id: string, title?: string, year?: number | null): Promise<MatchCandidate[]>;
  plexEditMatch(id: string, kind: EditKind, candidate: MatchCandidate): Promise<EditChange>;
  plexEditUnmatch(id: string, kind: EditKind): Promise<EditChange>;
  /** Save the fields; changed ones are locked when that setting is on. */
  plexEditDetails(id: string, fields: EditFields): Promise<EditChange>;
  /** Merge `others` into `target` (which keeps its name); the photo comes from `photoFrom` when given. */
  plexEditMerge(target: string, others: string[], photoFrom?: string | null): Promise<EditChange>;
  /** Merge albums Plex shows apart into `target` (undo splits them again). */
  plexEditMergeAlbums(target: string, others: string[]): Promise<EditChange>;
  plexEditSplit(id: string): Promise<EditChange>;
  plexEditRefresh(id: string, kind: EditKind): Promise<EditChange>;
  plexEditUndo(changeId: string): Promise<EditChange>;
  /** The item's recent changes, newest first. */
  plexEditHistory(id: string): Promise<EditChange[]>;

  // Tag editor (docs/IPC.md "Tag editor")
  /** Pick the music folder with the system dialog; null when cancelled. Saved when files are found in it. */
  tagEditPickMusicFolder(): Promise<MusicFolderCheck | null>;
  /** A typed music folder path (a UNC path); saved when files are found in it. */
  tagEditSetMusicFolder(path: string): Promise<MusicFolderCheck>;
  tagEditClearMusicFolder(): Promise<void>;
  /** The music folder checked now; null when it isn't set. */
  tagEditMusicFolderStatus(): Promise<MusicFolderCheck | null>;
  /** An album's files, or some songs', for the editor. */
  tagEditOpen(albumId: string | null, trackIds: string[]): Promise<TagEditorData>;
  tagEditSave(request: TagSaveRequest): Promise<TagSaveResult>;
  tagEditUndo(id: string): Promise<TagSaveResult>;
  /** The last 20 edits, newest first. */
  tagEditHistory(): Promise<TagUndoInfo[]>;
  tagEditCheck(scope: TagCheckScope): Promise<TagCheckReport>;
  /** "This is fine" on a finding (false brings it back). */
  tagEditCheckFine(id: string, fine: boolean): Promise<void>;
  tagEditFixPreview(kind: TagFixKind, albumId: string): Promise<TagFixPreview>;
  tagEditIsrcReleases(albumId: string): Promise<IsrcReleases>;
  tagEditIsrcPreview(albumId: string, release: IsrcRelease): Promise<TagFixPreview>;
  /** Mark albums (or an artist's era) explicit or clean: the changes to confirm. */
  tagEditExplicitPreview(albumIds: string[], explicit: boolean, era?: { artistId: string; era: string } | null): Promise<TagFixPreview>;
  /** The music library's settings in Plex, read only. */
  tagEditLibrarySettings(): Promise<PlexLibraryCheck[]>;

  // Likes (docs/IPC.md "Likes")
  /** Like (five stars) or unlike (clear the rating); resolves to the ids that changed. */
  likesSet(trackIds: string[], liked: boolean): Promise<string[]>;
  /** Every liked song's id, newest like first. */
  likedIds(): Promise<string[]>;
  likedPage(): Promise<LikedPage>;
  /** Playlists holding the song, most recently changed first. */
  playlistsWithTrack(trackId: string): Promise<string[]>;

  // Now playing
  /** Artist bio, extra credits and lyrics for the now-playing panel. */
  nowPlayingDetails(trackId: string): Promise<NowPlayingDetails>;
  /** The track's lyrics: Plex first, then LRCLIB when its setting is on. Null when there are none. */
  lyrics(trackId: string): Promise<Lyrics | null>;
  /** Lyrics the user wrote for the song in the editor, or null. */
  ownLyrics(trackId: string): Promise<OwnLyrics | null>;
  /** Save the song's own lyrics (LRC or plain lines). */
  ownLyricsSave(trackId: string, text: string, prefer: boolean): Promise<void>;
  /** Remove the song's own lyrics; true when there were some. */
  ownLyricsDelete(trackId: string): Promise<boolean>;
  /** Every song with its own lyrics. */
  ownLyricsIds(): Promise<string[]>;
  /** Save lyrics as an .lrc file through a save dialog. The saved file's name, or null when cancelled. */
  lyricsExport(text: string, fileName: string): Promise<string | null>;
  /** Every set of lyrics found for the song, with what the check said ("Wrong lyrics?"). */
  lyricsCandidates(trackId: string): Promise<LyricsCandidates>;
  /** Where the song's lyrics check is, while there is one. */
  lyricsCheckStatus(trackId: string): Promise<LyricsCheckStatus | null>;
  /** Pick the song's lyrics: a candidate key, "none", or null for automatic. */
  lyricsChoose(trackId: string, pick: string | null): Promise<void>;
  /** The song's lyrics timing offset in ms (positive: later). */
  lyricsOffset(trackId: string, offsetMs: number): Promise<void>;
  /** How many songs have a lyrics choice or offset. */
  lyricsChoicesCount(): Promise<number>;
  /** Forget every lyrics choice and offset; answers how many there were. */
  lyricsChoicesClear(): Promise<number>;
  /** A romanised line per lyrics line (null where there's nothing to romanise). Made on this computer. */
  lyricsPronounce(lines: string[]): Promise<(string | null)[]>;
  /** The lines translated by the service set in Settings (errors when none is set). */
  lyricsTranslate(lines: string[]): Promise<string[]>;

  // Search
  search(args: SearchArgs): Promise<SearchResults>;
  /** Whole artist, album and track names starting with the typed words. */
  searchCompletions(query: string, limit?: number): Promise<string[]>;
  /** Items opened from search, newest first. */
  searchRecents(): Promise<SearchHit[]>;
  searchRecentAdd(kind: SearchKind, id: string): Promise<void>;
  searchRecentRemove(kind: SearchKind, id: string): Promise<void>;
  searchRecentsClear(): Promise<void>;

  // Playback & queue
  playContext(args: PlayContextArgs): Promise<void>;
  playToggle(): Promise<void>;
  playPause(): Promise<void>;
  playResume(): Promise<void>;
  playNext(): Promise<void>;
  playPrevious(): Promise<void>;
  playSeek(positionMs: number): Promise<void>;
  setVolume(volume: number): Promise<void>;
  setMuted(muted: boolean): Promise<void>;
  setShuffle(shuffle: boolean): Promise<void>;
  /** Shuffle preference for a context by id (applies now if it's playing). */
  setContextShuffle(contextId: string, shuffle: boolean): Promise<void>;
  /** Smart shuffle for the playing playlist or Liked Songs. Off turns shuffle off, keeping the order. */
  setSmartShuffle(on: boolean): Promise<void>;
  /** Smart shuffle for a playlist or Liked Songs by id (applies now if it's playing). */
  setContextSmartShuffle(contextId: string, on: boolean): Promise<void>;
  /** Skip the playing smart shuffle suggestion and pick new ones. */
  smartSuggestionRemove(): Promise<void>;
  /** The playing suggestion was added to its playlist (or liked): drop its badge. */
  smartSuggestionKeep(itemId: string): Promise<void>;
  /** Crossfade for the playing context only (a mix's smooth transitions). */
  setSmoothTransitions(on: boolean): Promise<void>;
  setRepeat(repeat: RepeatMode): Promise<void>;
  /** Sets the A-B loop on the playing track (points in either order, 1 s apart or more); null clears it. */
  setAbLoop(abLoop: AbLoop | null): Promise<void>;
  queueAdd(trackIds: string[], at: QueueAt): Promise<void>;
  queueRemove(itemId: string): Promise<void>;
  queueMove(itemId: string, toIndex: number): Promise<void>;
  queueClear(): Promise<void>;
  queueJump(itemId: string): Promise<void>;
  playerState(): Promise<PlayerState>;

  // Output devices
  /** Audio outputs on this computer, the chosen one and the one in use. */
  outputDevices(): Promise<OutputState>;
  /** Play on a device (null: the system default), keeping position and play state. */
  outputSelect(deviceId: string | null): Promise<OutputState>;

  // Listen on (docs/IPC.md "Listen on")
  /** Plex players to play on (empty when the setting hides them). */
  remotePlayers(): Promise<RemotePlayer[]>;
  remoteState(): Promise<RemoteState>;
  /** Move playback to a player at the current position. */
  remoteMove(playerId: string): Promise<RemoteState>;
  /** Bring playback back to this computer at the player's position. */
  remoteReturn(): Promise<RemoteState>;
  /** Play something on the device playing now (instead of playContext while remote). */
  remotePlayContext(args: PlayContextArgs): Promise<RemoteState>;
  /** Jump to a queue entry on the device playing now (instead of queueJump while remote). */
  remoteQueueJump(itemId: string): Promise<RemoteState>;
  /** A player bar control while playing on another device. */
  remoteCommand(command: RemoteCommand): Promise<RemoteState>;
  /** Move to an AirPlay receiver with its password (kept in the keychain once it works). */
  remoteMoveWithPassword(playerId: string, password: string): Promise<RemoteState>;

  // Health and app states
  health(): Promise<Health>;
  /** Ids of tracks stored on this computer (playable without the server). */
  cachedTracks(): Promise<string[]>;
  /** Albums, artists and playlists with a song on this computer. */
  cachedLibrary(): Promise<CachedLibrary>;
  /** macOS only: opens System Settings at Privacy & Security, Local Network. */
  openLocalNetworkSettings(): Promise<void>;

  // Settings
  settingsGet(): Promise<Settings>;
  settingsSet(patch: Partial<Settings>): Promise<void>;
  /** The settings a new install starts with (Reset this section). */
  settingsDefaults(): Promise<Settings>;
  /** Save every setting (never keys, they stay in the keychain) to a file the user picks. The path, or null when cancelled. */
  settingsExport(): Promise<string | null>;
  /** Read settings from a file the user picks and apply them. Null when cancelled. */
  settingsImport(): Promise<SettingsImportReport | null>;
  /** Opens a link from a help page (project pages only) in the browser. */
  helpOpenLink(url: string): Promise<void>;
  /** The theme files in the themes folder, and the ones that can't be used with the reason. */
  themesList(): Promise<ThemeFolder>;
  /** Pick a theme file and copy it into the themes folder. Null when the picker was cancelled. */
  themesImport(): Promise<ThemeFile | null>;
  /** Show the themes folder in Explorer / Finder. */
  themesOpenFolder(): Promise<void>;
  /** Check the text of a look or theme file dropped on the window and add it to the themes folder. */
  themesImportText(text: string): Promise<ThemeFile>;
  /** Save a look into the themes folder; `replace` writes over its own look file. */
  themesSave(look: ThemeFile, replace: boolean): Promise<ThemeFile>;
  /** Export a look to a file the user picks. The file's name, or null when cancelled. */
  themesExport(look: ThemeFile): Promise<string | null>;
  /** Saves the current EQ curve as a profile (replacing one with that name) and selects it. */
  eqProfileSave(name: string): Promise<Settings>;
  eqProfileRename(from: string, to: string): Promise<Settings>;
  eqProfileDelete(name: string): Promise<Settings>;
  /** Uses the selected EQ preset or profile whenever the output in use plays (false removes it). */
  eqUseForOutput(on: boolean): Promise<Settings>;
  /** Headphones in the bundled correction index matching `query` (at most 50). */
  headphoneSearch(query: string): Promise<HeadphoneEntry[]>;
  /** Uses a headphone's correction whenever the output in use plays (null turns it off there). Fetches the profile once. */
  headphoneCorrectionSet(path: string | null): Promise<Settings>;
  /** A/B: plays without the correction while true. Not saved. */
  headphoneCorrectionCompare(bypass: boolean): Promise<void>;
  /** Bit-perfect output on or off for the output in use. */
  bitPerfectSet(on: boolean): Promise<Settings>;
  artCacheInfo(): Promise<ArtCacheInfo>;
  /** Deletes every cached image; resolves to the bytes freed. */
  artCacheClear(): Promise<number>;

  // App
  appInfo(): Promise<AppInfo>;
  /** Opens the project page in the default browser. */
  openProjectPage(): Promise<void>;

  // Updates
  updateCheck(): Promise<UpdateInfo>;
  updateInstallOnRestart(): Promise<void>;

  // Artist info (outside sources)
  /** Outside info for an artist; `artistinfo://changed` fires when more arrives. */
  artistInfo(id: string): Promise<ArtistInfo>;
  /** An album's MusicBrainz date and credits; `albumcredits://changed` fires when a lookup finishes. */
  albumCredits(id: string): Promise<AlbumCredits>;
  /** Whether an album counts as released and why; `releasestatus://changed` fires when it changes. */
  albumReleaseStatus(id: string): Promise<AlbumReleaseStatus>;

  // Archive (nothing here writes to Plex or files)
  /** Mark an album or song unreleased or released, or clear the mark (null). */
  releaseMarkSet(kind: "album" | "track", id: string, mark: ReleaseMark | null): Promise<void>;
  /** A song and its other versions, released ones first. */
  trackVersions(id: string): Promise<VersionItem[]>;
  /** Every version of a song on one timeline, oldest first (undated last). */
  trackLineage(id: string): Promise<Lineage>;
  /**
   * Put both versions' whole files on this computer, one after the other
   * (`compare://progress`), then open Compare. Leveling follows the
   * Compare level matching setting until `compareEnd`.
   */
  comparePrepare(aId: string, bId: string): Promise<CompareReady>;
  /** Play one side at the moment the other is at (the start the first time). */
  compareSwitch(side: CompareSide, play?: boolean): Promise<CompareNow>;
  /** Set the loop region in the playing version's time (null clears it). */
  compareLoop(ab: AbLoop | null): Promise<AbLoop | null>;
  /** Close Compare: the user's own sound settings come back. */
  compareEnd(): Promise<void>;
  /** An artist's era: its songs by album or build, newest first. */
  eraPage(artistId: string, name: string): Promise<EraPage>;
  /** Move an album to a section of the user's own (null: back to its own). */
  albumSectionSet(albumId: string, artistId: string, name: string | null): Promise<void>;
  /** An artist's own section names. */
  artistSections(artistId: string): Promise<string[]>;

  // Library doctor (nothing here writes to Plex or files)
  /** Every finding, with what's ignored; `doctor://changed` fires when they change. */
  doctorReport(): Promise<DoctorReport>;
  /** How many findings to fix (the badge). */
  doctorCount(): Promise<number>;
  /** Ignore a finding (it follows you through the sync file), or bring it back. */
  doctorIgnore(id: string, kind: DoctorKind, title: string, ignore: boolean): Promise<void>;
  /** Check every artist again soon. */
  doctorRun(): Promise<void>;
  /** Identify by sound: whether there's a key, and progress. */
  soundIdStatus(): Promise<SoundIdStatus>;
  /** Save the user's own AcoustID key in the keychain (null removes it). */
  soundIdKeySet(key: string | null): Promise<SoundIdStatus>;
  /** What a song sounds like, if it was identified by sound (a hint only). */
  trackSoundsLike(id: string): Promise<SoundsLike | null>;
  artistInfoKeys(): Promise<InfoKeysStatus>;
  /** Save a source's key in the keychain (null removes it); answers whether it was refused. */
  artistInfoKeySet(source: KeyedInfoSource, key: string | null): Promise<InfoKeyState>;
  /** Forget cached artist info; resolves to how many artists had some. */
  artistInfoCacheClear(): Promise<number>;
  /** Open a source or licence link in the default browser. */
  /** With `artistId`, that artist's own listed links (their website) open too. */
  openInfoLink(url: string, artistId?: string): Promise<void>;

  // Sharing (docs/IPC.md "Sharing")
  /** The Platter link (with a start time for songs) and whether a public link can be had. */
  shareInfo(kind: ShareKind, id: string, atSeconds?: number): Promise<ShareInfo>;
  /** Find the public link for a released song or album (notFound when there's no sure match). */
  sharePublicLink(kind: ShareKind, id: string): Promise<string>;
  /** The platter:// link opened last, once (null when none is waiting). */
  deepLinkTake(): Promise<DeepLinkOpen | null>;
  /** The QR code for an item's Platter link (with the start time for a song). */
  shareCardQr(kind: ShareKind, id: string, atSeconds?: number): Promise<CardQr>;
  /** Put a card on the clipboard (its RGBA pixels). */
  shareCardCopy(rgba: Uint8Array, width: number, height: number): Promise<void>;
  /** Save a card (PNG bytes) through a save dialog; the file's name, or null when cancelled. */
  shareCardSave(png: Uint8Array, name: string): Promise<string | null>;

  // Discord status (docs/IPC.md "Discord")
  discordStatus(): Promise<DiscordStatus>;
  /** The now-playing feed for widgets: serving, off, or why its port couldn't open. */
  feedStatus(): Promise<FeedStatus>;
  /** The Stream Deck control API: on or off, the pairing code, paired. */
  deckStatus(): Promise<DeckStatus>;
  /** Forget the paired Stream Deck and make a new code. */
  deckRevoke(): Promise<DeckStatus>;
  /** Tell the Stream Deck keys what the window shows (the lyrics). */
  deckUiReport(report: { lyrics: boolean }): Promise<void>;
  /** Forget the paired script or command line and show a new automation code. */
  automationRevoke(): Promise<DeckStatus>;
  /** The "Allow Stream Deck to control Platter?" prompt showing now (its id), if any. */
  deckPairPrompt(): Promise<string | null>;
  /** Answer that prompt; false when it had already gone. */
  deckPairAnswer(id: string, allow: boolean): Promise<boolean>;
  /** Whether the Stream Deck app is here and the Platter plugin installed and current. */
  deckPluginStatus(): Promise<DeckPluginStatus>;
  /** Open the plugin bundled with Platter, so the Stream Deck app installs it. */
  deckPluginInstall(): Promise<void>;
  /** Open Elgato's download page for the Stream Deck app. */
  deckAppDownload(): Promise<void>;

  // Platter Bridge (docs/IPC.md "Platter Bridge")
  /** Paired Bridges, a pair code waiting to be used, queued Lidarr changes. */
  bridgeOverview(): Promise<BridgeOverview>;
  /** "Pair a Bridge": a new code, good for an hour. */
  bridgePairStart(): Promise<BridgePairStart>;
  bridgePairCancel(): Promise<void>;
  /** Forget a Bridge (and tell it to forget this computer). */
  bridgeUnpair(id: string): Promise<BridgeOverview>;
  /** Say yes to a Lidarr settings change a Bridge queued. */
  bridgeQueuedApply(id: string): Promise<BridgeOverview>;
  bridgeQueuedDismiss(id: string): Promise<BridgeOverview>;
  /** Send the Bridges the current Lidarr and Plex settings. */
  bridgeSendSettings(): Promise<void>;
  /** Save the Imgur client ID in the keychain (null removes it); answers whether Imgur refused it. */
  discordImgurKeySet(key: string | null): Promise<InfoKeyState>;

  // File tags (docs/IPC.md "File tags")
  /** Badges, album tags and edition groups read from the files. */
  tagsIndex(): Promise<TagIndex>;
  /** Progress of the background tag scan. */
  tagScanStatus(): Promise<TagScanStatus>;

  // Lyrics transcription (docs/IPC.md "Lyrics transcription")
  transcribeStatus(): Promise<TranscribeStatus>;
  /** Start downloading a model; progress comes as `transcribe://status`. */
  transcribeModelDownload(model: TranscribeModel): Promise<void>;
  transcribeModelCancel(): Promise<void>;
  transcribeModelDelete(model: TranscribeModel): Promise<void>;
  /** Transcribe one song now (downloads the chosen model first if needed). */
  transcribeTrack(trackId: string): Promise<void>;
  /** Queue an album's songs without lyrics; answers how many. */
  transcribeAlbum(albumId: string): Promise<number>;
  /** Queue every unreleased song without lyrics in the background; answers how many. */
  transcribeUnreleased(): Promise<number>;
  /** Cancel one song, or everything (no id). */
  transcribeCancel(trackId?: string): Promise<void>;
  /** Pause or resume the batch (kept across a restart). */
  transcribePause(paused: boolean): Promise<void>;
  /** Songs done with another model while the chosen one downloaded, again with it. */
  transcribeRedo(): Promise<number>;
  /** The song was transcribed and no words were found. */
  transcribeFoundNothing(trackId: string): Promise<boolean>;

  // Sharing lyrics (docs/IPC.md "Sharing lyrics")
  /** Whether to offer sharing the song's transcription to LRCLIB, and what would be sent. */
  lyricsShareInfo(trackId: string): Promise<LyricsShareInfo>;
  /** The user checked the song's lyrics (opened the editor, or played it through with them showing). */
  lyricsShareReviewed(trackId: string): Promise<void>;
  /** Publish to LRCLIB after the user confirmed. Resolves once published. */
  lyricsSharePublish(trackId: string): Promise<void>;
  lyricsShareCancel(trackId: string): Promise<void>;

  // Downloads (docs/IPC.md "Downloads")
  downloadsStatus(): Promise<DownloadStatus>;
  /** Ids of every downloaded song. */
  downloadedTracks(): Promise<string[]>;
  /** Turn an album's, playlist's, Liked Songs' or mix's download on or off. A mix sends its songs. */
  downloadsSet(kind: DownloadKind, id: string, on: boolean, opts?: { title?: string; trackIds?: string[] }): Promise<DownloadStatus>;
  /** Failed and missing songs try again (one item's, or all). */
  downloadsRetry(kind?: DownloadKind, id?: string): Promise<void>;
  /** Delete every download; resolves to the bytes freed. */
  downloadsRemoveAll(): Promise<number>;
  /** Pick a new folder (the files move there); null when cancelled. */
  downloadsChooseFolder(): Promise<string | null>;

  // Sync to a device (docs/IPC.md "Devices")
  /** Known devices (and where each is plugged in), new removable drives and the running sync. */
  devicesList(): Promise<DevicesView>;
  /** Remember a drive as a device with its music folder. */
  devicesAdd(root: string, name: string, folder: string): Promise<Device>;
  /** Pick the music folder on a drive; null when cancelled. */
  devicesChooseFolder(root: string): Promise<string | null>;
  devicesUpdate(id: string, patch: DevicePatch): Promise<Device>;
  /** Add something to a device's selection, or take it off. */
  devicesSelect(id: string, kind: DeviceItemKind, itemId: string, title: string, on: boolean): Promise<Device>;
  /** Forget a device on this computer (its files stay on it). */
  devicesForget(id: string): Promise<void>;
  /** Start syncing; progress comes as `devices://progress`. */
  devicesSync(id: string): Promise<DeviceSyncStatus>;
  devicesPause(on: boolean): Promise<void>;
  devicesCancel(): Promise<void>;
  devicesStatus(): Promise<DeviceSyncStatus | null>;

  // Sound map (docs/IPC.md "Sound map")
  /** How far along the map is; also places songs analysed since the last update. */
  soundmapInfo(): Promise<SoundMapInfo>;
  /** Up to `limit` points after `after` (song id order), within `area` when given. */
  soundmapPoints(after?: string | null, limit?: number, area?: SoundMapArea | null): Promise<SoundMapChunk>;
  /** Artists and songs on the map matching `query`. */
  soundmapFind(query: string): Promise<SoundMapFound[]>;
  /** Title, artist and cover for the hover card. */
  soundmapSong(id: string): Promise<SoundMapSong>;

  // Social (docs/IPC.md "Friend activity, following and listening sessions")
  /** Who else on the server listened lately (owner only). `fresh` skips the 20 s cache. */
  friendActivity(fresh?: boolean): Promise<ActivityPanel>;
  followsList(): Promise<FollowedArtist[]>;
  /** Follow or unfollow an artist (library or credited); resolves to the whole list. */
  followSet(artist: { id: string; title: string; thumb?: string | null }, on: boolean): Promise<FollowedArtist[]>;
  whatsNew(): Promise<WhatsNew>;
  /** What's New was seen: its items stop being new. */
  whatsNewSeen(): Promise<void>;
  listenState(): Promise<ListenView>;
  /** Host a listening session; the view carries the first invite. */
  listenStart(): Promise<ListenView>;
  /** The invite to send; `fresh` makes a new one (each link works once). */
  listenInvite(fresh?: boolean): Promise<ListenView>;
  /** End the session you host, or leave the one you're in. */
  listenStop(): Promise<void>;
  /** Join from an invite link. */
  listenJoin(link: string): Promise<ListenView>;
  /** Add songs to the session's queue (a guest's go to the host). */
  listenAdd(trackIds: string[]): Promise<void>;
  /** Play, pause, skip, seek or reorder for everyone (a guest's go to the host). */
  listenControl(op: ListenOp): Promise<void>;
  /** Remove someone from the session you host. */
  listenRemove(personId: string): Promise<ListenView>;
  /** "Let them listen from my server" for someone in the session you host. */
  listenOfferServer(personId: string): Promise<ListenView>;

  // Sharing your music through Plex (docs/IPC.md "Sharing")
  /** Your server's music libraries, their unreleased albums, and Plex Pass. */
  sharingOverview(): Promise<SharingOverview>;
  /** Share music libraries with a Plex username or email. */
  sharingShare(invitee: string, options: ShareOptions): Promise<ShareStatus>;
  /** Everyone your server is shared with. */
  sharingPeople(): Promise<PersonWithAccess[]>;
  /** Take someone's access away. */
  sharingRemove(id: string): Promise<void>;
  /** A link or QR code that lets someone ask for your music. */
  sharingLink(options: ShareOptions): Promise<ShareLinkView>;
  /** Open someone's share link. */
  sharingOpenLink(link: string): Promise<ShareLinkResult>;
  /** People who opened your share links, waiting for a yes. */
  sharingLinkAsks(): Promise<ShareLinkAsk[]>;
  /** Share with one of them, as the link was made; the result in words. */
  sharingLinkApprove(id: string): Promise<string>;
  sharingLinkDeny(id: string): Promise<void>;
  /** Server invites you received (`fresh` asks Plex). */
  sharingInvites(fresh?: boolean): Promise<ReceivedInvite[]>;
  /** Accept one: their server and its music join your library. */
  sharingAccept(id: string): Promise<InviteAccepted>;

  // Shared playlists and Blends (docs/IPC.md "Shared playlists and Blends")
  /** Shared playlists, Blends, direct invites and the relay. */
  circleState(): Promise<CircleView>;
  /** People on your server or Plex Home you can invite without a link. */
  circlePeople(): Promise<CircleAccount[]>;
  /** Join from a platter://collab/ or platter://blend/ link. */
  circleJoin(link: string): Promise<JoinResult>;
  /** Join an invite that came straight to you. */
  circleAccept(id: string): Promise<JoinResult>;
  circleDismiss(id: string): Promise<void>;
  /** Your playlist's sharing (null: not shared). */
  collabInfo(playlistId: string): Promise<SharedInfo | null>;
  /** A one-use invite link to your playlist. */
  collabInviteLink(playlistId: string, role: "collaborator" | "listener"): Promise<InviteLink>;
  /** Invite someone on your server or Home straight away. */
  collabInvitePerson(playlistId: string, key: string, role: "collaborator" | "listener"): Promise<void>;
  /** A playlist shared with you (`shared:<group>`). */
  collabPage(id: string): Promise<SharedPage>;
  /** As a collaborator: these go to the owner, now or when they're next online. */
  collabAdd(id: string, trackIds: string[]): Promise<void>;
  /** As a collaborator: remove songs you added. */
  collabRemove(id: string, keys: string[]): Promise<void>;
  collabLeave(id: string): Promise<void>;
  /** Show or hide a playlist shared with you in your library (you stay in it). */
  collabSetInLibrary(id: string, on: boolean): Promise<void>;
  collabSetRole(playlistId: string, personId: string, role: "collaborator" | "listener"): Promise<void>;
  collabRemovePerson(playlistId: string, personId: string): Promise<void>;
  collabSetPublic(playlistId: string, isPublic: boolean): Promise<void>;
  /** A Blend with you in it; the people picked are invited directly, the link is for anyone else. */
  blendCreate(keys: string[]): Promise<BlendCreated>;
  blendInviteLink(id: string): Promise<InviteLink>;
  blendInvitePerson(id: string, key: string): Promise<void>;
  blendPage(id: string): Promise<BlendPage>;
  /** Hidden in this Blend: it doesn't come back when the Blend is made again. */
  blendHide(id: string, trackId: string): Promise<void>;
  blendRefresh(id: string): Promise<BlendPage>;
  /** Deletes the Blend here and stops sharing. */
  blendLeave(id: string): Promise<void>;

  // Sync file, library folders and keys (docs/IPC.md "Sync, folders and keys")
  syncFileStatus(): Promise<SyncFileStatus>;
  /** Run a sync pass now. */
  syncFileNow(): Promise<SyncFileStatus>;
  /** OneDrive, iCloud Drive, Google Drive and Dropbox folders found on this computer. */
  syncFileFolders(): Promise<CloudFolder[]>;
  /** Sync with this folder and turn sync on. */
  syncFileUseFolder(path: string): Promise<SyncFileStatus>;
  /** Pick a folder in a dialog and sync with it; null when cancelled. */
  syncFileChooseFolder(): Promise<SyncFileStatus | null>;
  /** Start a new sync file when the old one can't be read (it's kept aside). */
  syncFileReplace(): Promise<SyncFileStatus>;
  pinsGet(): Promise<PinRecord[]>;
  /** Pin, or unpin when pinned; resolves to every pin, newest first. */
  pinsToggle(pin: PinRecord): Promise<PinRecord[]>;
  /** Pins an older build kept in the window's storage (taken once). */
  pinsImport(pins: PinRecord[]): Promise<PinRecord[]>;
  /** Playlists with Mix on, and the transitions picked for their gaps. */
  automixMixedGet(): Promise<MixedPlaylists>;
  /** Mix on or off for a playlist; resolves to every mixed playlist. */
  automixMixSet(id: string, on: boolean): Promise<MixedPlaylists>;
  /** The transition between two songs of a mixed playlist ("auto" clears the pick). */
  automixStyleSet(id: string, fromId: string, toId: string, style: MixStyle): Promise<MixedPlaylists>;
  /** Tempo per song and what each gap does, for songs in the order shown. */
  automixView(playlistId: string | null, trackIds: string[]): Promise<AutomixView>;
  foldersGet(): Promise<LibraryFolders>;
  /** A new folder ("New Folder" unless named), at the top or inside `parent`. */
  folderCreate(name?: string | null, parent?: string | null): Promise<LibraryFolder>;
  folderRename(id: string, name: string): Promise<void>;
  /** Delete a folder; what was in it moves up. Playlists are never deleted. */
  folderDelete(id: string): Promise<void>;
  folderMove(id: string, parent: string | null): Promise<void>;
  /** Put a playlist in a folder (null: back to the top). */
  playlistFolderSet(playlistId: string, folderId: string | null): Promise<void>;
  /** The keys this device has. */
  keysAvailable(): Promise<KeyKind[]>;
  /** Show a code to copy keys to this device. */
  keysReceiveStart(): Promise<KeyReceiveState>;
  keysReceiveState(): Promise<KeyReceiveState>;
  keysReceiveCancel(): Promise<void>;
  /** Send the ticked keys to the device showing `code`. */
  keysSend(code: string, kinds: KeyKind[]): Promise<KeySendOutcome>;
  // Hide, bookmarks, smart playlists, scrobbling (docs/IPC.md)
  /** Hidden songs and artists, newest first. */
  hiddenList(): Promise<HiddenItem[]>;
  /** Hide or unhide; resolves to the whole list. */
  hiddenSet(item: HideRequest, on: boolean): Promise<HiddenItem[]>;
  bookmarksList(): Promise<Bookmark[]>;
  /** A song at a time, or a page. The same one again resolves to the one already there. */
  bookmarkAdd(bookmark: NewBookmark): Promise<Bookmark>;
  bookmarkRemove(id: string): Promise<Bookmark[]>;
  /** A song bookmark plays from its time; resolves to the bookmark (a page one opens in the UI). */
  bookmarkOpen(id: string): Promise<Bookmark>;
  /** How many songs the rules give now (and picks first), with the first of them. */
  smartPreview(rules: SmartRules, picks?: string[]): Promise<SmartPreview>;
  /** Save to Plex when it can hold the rules, otherwise as a Platter smart playlist. */
  smartSave(request: SmartSaveRequest): Promise<SmartSaved>;
  /** Platter smart playlists. */
  smartList(): Promise<SmartSummary[]>;
  smartPage(id: string): Promise<SmartPage>;
  smartDelete(id: string): Promise<void>;
  /** A Plex smart playlist's rules for the editor (fails when they don't map). */
  smartPlexRules(id: string): Promise<SmartRules>;
  /** A description turned into rules and picked songs (the AI provider). */
  smartPrompt(prompt: string): Promise<PromptResult>;
  /** A description read into rules on this computer (nothing is sent anywhere). */
  smartDescribe(text: string): Promise<Described>;
  aiStatus(): Promise<AiStatus>;
  /** Save (or with null remove) the AI provider's key in the OS keychain. */
  aiKeySet(key: string | null): Promise<AiStatus>;
  scrobbleStatus(): Promise<ScrobbleStatus>;
  lastfmSecretSet(secret: string | null): Promise<ScrobbleStatus>;
  /** Opens Last.fm's approval page in the browser. */
  lastfmSignInStart(): Promise<ScrobbleStatus>;
  /** After approving on last.fm. */
  lastfmSignInFinish(): Promise<ScrobbleStatus>;
  lastfmSignOut(): Promise<ScrobbleStatus>;
  /** Check the ListenBrainz token (after saving it). */
  listenbrainzCheck(): Promise<ScrobbleStatus>;
  /** Lidarr: what's set up (addresses are in settings, the key and proxy headers in the keychain). */
  lidarrStatus(): Promise<LidarrStatus>;
  /** Save (or with null remove) Lidarr's API key in the OS keychain. */
  lidarrKeySet(key: string | null): Promise<LidarrStatus>;
  /** Save extra headers for the away address as Name: value lines (null or blank removes them). */
  lidarrHeadersSet(text: string | null): Promise<LidarrStatus>;
  /** The read-only setup check (home address first, then away). */
  lidarrCheck(): Promise<LidarrReport>;
  /** Say an empty Lidarr root folder is (or isn't) separate from the Plex music folder. */
  lidarrConfirmRoot(path: string, confirmed: boolean): Promise<void>;
  /** Which official release group to request for an album, or the album a song comes from (MusicBrainz), or why there's none. */
  lidarrPick(input: LidarrPickInput): Promise<LidarrPicked>;
  /** What a Lidarr request will do and where its files will land. Runs the full check; reads only. */
  lidarrRequestPreview(input: LidarrRequestInput): Promise<LidarrPreview>;
  /** Send a request to Lidarr (or keep it waiting when Lidarr can't be reached). */
  lidarrRequest(input: LidarrRequestInput): Promise<LidarrRequest>;
  /** One-click Get: the request with every check, or why it needs the user (first request ever, or a check said no). */
  lidarrGet(input: LidarrRequestInput): Promise<LidarrGetOutcome>;
  /** "Search again" for an album nothing turned up for (one AlbumSearch; the schedule starts over). */
  lidarrSearchAgain(id: string): Promise<LidarrRequest>;
  /** Take albums off the "Get albums" queue (all of them when the list is empty). */
  getAlbumsUnqueue(releaseGroupIds: string[]): Promise<BringInStatus>;
  /** Requests on this device, newest first; refresh reads their state from Lidarr. */
  lidarrRequests(refresh: boolean): Promise<LidarrRequest[]>;
  /** "Request these": up to 10 albums the user confirmed at once, each with the full check. */
  lidarrRequestBatch(inputs: LidarrRequestInput[]): Promise<LidarrBatchItem[]>;
  /** "Bring in my listening": waves, today's count, why it's paused, what needs you. */
  bringInStatus(): Promise<BringInStatus>;
  /** Pause it, or carry on. */
  bringInPause(paused: boolean): Promise<BringInStatus>;
  /** A "Needs you" album: try it again, or leave it be. */
  bringInNeedsYou(key: string, retry: boolean): Promise<BringInStatus>;
  /** Cancel a waiting request, or clear a finished one. */
  lidarrRequestCancel(id: string): Promise<LidarrRequest[]>;
  /** An artist's official releases the library lacks, for the artist page. */
  lidarrMissingReleases(artistId: string, artist: string): Promise<LidarrMissing>;
  /** Upgrades: whether the album menu offers "Look for a better copy", and the album's upgrade. */
  lidarrUpgradeAlbum(albumId: string): Promise<AlbumUpgrade>;
  /** Ask Lidarr for a better copy of an album, into the staging folder (full setup check first). */
  lidarrUpgradeRequest(albumId: string): Promise<UpgradeRecord>;
  /** Ask for better copies of every released album under a quality. */
  lidarrUpgradeBulk(under: UpgradeUnder): Promise<UpgradeBulkResult>;
  /** Upgrades on this computer, newest first; refresh follows them in Lidarr. */
  lidarrUpgrades(refresh: boolean): Promise<UpgradeRecord[]>;
  /** What swapping would do now: songs that upgrade and songs kept, with why. */
  lidarrUpgradeOffer(id: string): Promise<UpgradeOffer>;
  /** Swap the better copies in (null: every song that upgrades). */
  lidarrUpgradeSwap(id: string, trackIds: string[] | null): Promise<SwapResult>;
  /** Put the old files of the last swap back exactly. */
  lidarrUpgradeUndo(id: string): Promise<UpgradeRecord>;
  /** Clear an upgrade from the list. */
  lidarrUpgradeClear(id: string): Promise<UpgradeRecord[]>;
  /** The album's songs and the MusicBrainz releases it could be, each with its tracklist. */
  lidarrUpgradeCandidates(albumId: string): Promise<UpgradeCandidates>;
  /** Confirm (or with null forget) which release group an album is. Upgrades need it. */
  lidarrUpgradeConfirm(albumId: string, releaseGroupId: string | null): Promise<UpgradeConfirmed | null>;
  /** The release groups an artist is limited to (empty: no limit). */
  lidarrAllowlist(artist: string, artistId: string | null): Promise<string[]>;
  lidarrAllowlistSet(artist: string, artistId: string | null, releaseGroupIds: string[]): Promise<string[]>;
  /** Every artist's mode, with why. */
  lidarrModes(): Promise<LidarrArtistMode[]>;
  /** Set an artist's mode; Managed needs confirmations = 2. */
  lidarrModeSet(key: string, name: string, mode: LidarrModeSetting, confirmations: number): Promise<LidarrArtistPrefs>;
  lidarrArtistChoices(): Promise<Record<string, LidarrArtistPrefs>>;
  lidarrArtistChoicesSet(key: string, name: string, autoAdd: boolean | null, hunt: boolean, target: LidarrTarget): Promise<LidarrArtistPrefs>;
  /** Show what a fix would change (reads only). */
  lidarrChangePreview(action: LidarrFixAction, input: string | null): Promise<LidarrProposal>;
  lidarrHiresPreview(): Promise<LidarrProposal>;
  lidarrChangeApply(id: string): Promise<LidarrChange>;
  lidarrChanges(): Promise<LidarrChange[]>;
  lidarrChangePutBack(id: string): Promise<LidarrChange>;
  lidarrRematchPreview(folder: string): Promise<LidarrRematchPlan>;
  lidarrRematchApply(folder: string, releaseId: number | null): Promise<LidarrRematchPlan>;
  lidarrWork(): Promise<LidarrWork>;
  lidarrCatchUp(): Promise<LidarrCatchUpReport>;
  lidarrResumeProtected(): Promise<void>;
  lidarrStagedAdd(id: string, always: boolean): Promise<LidarrStaged>;
  lidarrStagedUndo(id: string, trashOk: boolean): Promise<LidarrStaged>;
  lidarrHunt(): Promise<LidarrHuntItem[]>;
  /** Where this computer reaches the music share. */
  musicShareGet(): Promise<ShareMap>;
  musicShareSet(map: ShareMap): Promise<ShareMap>;
  /** Plex's music folders as Plex sees them. */
  musicSharePlexFolders(): Promise<string[]>;

  // Release radar (MusicBrainz, about once a day; `radar://changed` when it changes)
  /** New releases from followed and most played artists. */
  releaseRadar(): Promise<RadarPage>;
  /** Look now instead of waiting for the daily look. */
  releaseRadarCheck(): Promise<RadarPage>;
  /** Artists with auto-request on. */
  radarAutoArtists(): Promise<AutoArtist[]>;
  /** Turn auto-request on or off for an artist (on also turns auto-request on in Settings). */
  radarAutoSet(id: string, name: string, on: boolean): Promise<AutoArtist[]>;

  // Discover: music that isn't in the library (MusicBrainz and ListenBrainz, paced and cached in the core)
  /** Artists, albums and songs from MusicBrainz the library lacks; `shown` false while off or offline. */
  catalogueSearch(query: string): Promise<CatalogueSearch>;
  /** One kind of music the library lacks, from Lidarr's search when it's set up (else MusicBrainz); asked for all three at once. */
  catalogueSearchPart(query: string, part: CataloguePartKind): Promise<CataloguePart>;
  /** A photo for an artist outside the library (Lidarr, Wikimedia Commons, TheAudioDB); empty when none has one. */
  catalogueArtistPhoto(mbid: string, name: string): Promise<CataloguePhoto>;
  /** "Get albums": what it would queue for an artist, most popular first. */
  catalogueGetAlbumsPlan(mbid: string, name: string, kinds: string[]): Promise<GetAlbumsPlan>;
  /** "Get albums": queue them, paced like Bring in my listening. */
  catalogueGetAlbums(mbid: string, name: string, kinds: string[]): Promise<{ queued: number; plan: GetAlbumsPlan }>;
  /** A page for an artist outside the library. `artist.libraryId` set: open their library page instead. */
  catalogueArtist(mbid: string): Promise<CatalogueArtistPage>;
  /** A page for an album outside the library (MusicBrainz release group id). */
  catalogueAlbum(releaseGroupId: string): Promise<CatalogueAlbumPage>;
  /** Artists outside the library for a library artist's "Fans also like". */
  catalogueFansAlsoLike(artistId: string, name: string): Promise<CatArtist[]>;
  /** Artists like the ones played most that aren't in the library. */
  newToYou(): Promise<NewToYou>;
  /** Albums added to the Missing list by hand. */
  importMissingAlbums(): Promise<MissingAlbum[]>;

  // Physical collection (marks in the sync store; Discogs token in the keychain)
  /** Every album marked as owned, by artist then title. `physical://changed` fires when they change. */
  physicalList(): Promise<OwnedAlbum[]>;
  /** Mark an album as owned with these copies; no copies takes the mark off. */
  physicalSet(albumId: string, owned: SetOwned): Promise<OwnedAlbum | null>;
  /** The shelf in numbers (the shelf page). */
  physicalStats(): Promise<ShelfStats>;
  /** The MusicBrainz album a record on the shelf is (to request it); null when MusicBrainz doesn't know it. */
  physicalFindRelease(artist: string, title: string): Promise<CatAlbum | null>;
  /** Play history from Plex, Tautulli and Last.fm: each source's import. */
  playHistoryStatus(): Promise<PlayHistoryStatus>;
  /** Look for new plays from every source now (failed ones too). */
  playHistoryUpdate(): Promise<PlayHistoryStatus>;
  /** Stop the read going on; it goes on from the same place later. */
  playHistoryCancel(): Promise<PlayHistoryStatus>;
  /** Delete the plays brought in from one source (only that source's). */
  playHistoryRemove(source: PlayHistorySource): Promise<PlayHistoryStatus>;
  tautulliStatus(): Promise<TautulliStatus>;
  /** Save Tautulli's API key in the keychain, or null to remove it. */
  tautulliKeySet(key: string | null): Promise<TautulliStatus>;
  discogsStatus(): Promise<DiscogsStatus>;
  /** Save the personal token (Discogs checks it first), or null to remove it. */
  discogsTokenSet(token: string | null): Promise<DiscogsStatus>;
  /** Read the Discogs collection and mark the albums the library has. */
  discogsImport(): Promise<DiscogsReport>;
  importMissingAlbumAdd(album: WantedAlbum): Promise<WantedAlbum[]>;
  importMissingAlbumRemove(releaseGroupId: string): Promise<WantedAlbum[]>;

  // Library import (nothing here writes to Plex)
  importOverview(): Promise<ImportOverview>;
  /** File or folder picker for an import; null when cancelled. */
  importChoose(folder: boolean): Promise<string | null>;
  /** Import from a file, folder or zip for a source that's turned on. */
  importRun(source: ImportSourceId, path: string): Promise<ImportReport>;
  importReviewQueue(): Promise<ImportReviewItem[]>;
  importReview(songId: number, decision: ImportDecision): Promise<void>;
  importListPage(id: string): Promise<ImportedListPage>;
  importMissing(): Promise<MissingSong[]>;
  /** Open a store search page from the Missing list. */
  importOpenLink(url: string): Promise<void>;
  /** Forget everything imported from a source. */
  importForget(source: ImportSourceId): Promise<void>;
  /** Take one song out of an imported list. */
  importListRemove(listId: string, songId: number): Promise<void>;
  /** A page of "Played a lot, not in your library": imported history with no library match. */
  importPlayed(query: PlayedQuery): Promise<PlayedPage>;
  /** Hide (or bring back) a song, an album or a whole artist on "Played a lot". Synced. */
  importPlayedHide(kind: PlayedHidden["kind"], artist: string, name: string | null, hide: boolean): Promise<PlayedHidden[]>;
  /** What's hidden from "Played a lot", newest first. */
  importPlayedHidden(): Promise<PlayedHidden[]>;

  // Listening stats (docs/IPC.md)
  /** Totals, chart, streak, busiest times and top lists for a range of days. */
  statsOverview(range: StatsRange, limit?: number): Promise<StatsOverview>;
  /** The first and last day with plays and the months with plays. */
  statsSpan(): Promise<StatsSpan>;
  /** First song, most replayed day, biggest discovery and a comeback for a range. */
  statsMoments(range: StatsRange): Promise<StatsMoments>;
  /** Each day with plays from `from` to `to` ("YYYY-MM-DD", at most 400 days), oldest first. */
  statsDays(from: string, to: string): Promise<StatsPoint[]>;
  /** One song's, artist's or album's numbers (the stats strip). */
  statsItem(kind: StatsItemKind, id: string): Promise<ItemStats>;
  /** A month's recap ("YYYY-MM"). */
  statsMonthRecap(month: string): Promise<MonthRecap>;
  /** Songs for a top list's Play these and Save as playlist, most played first. */
  statsPlayIds(range: StatsRange, kind: StatsTopKind, limit?: number): Promise<string[]>;
  /** Which Top Songs lists and Wrapped years there are (Wrapped and Top Songs must be on). */
  topSongsIndex(): Promise<TopSongsIndex>;
  /** A year's Top Songs list, or All-Time for null. */
  topSongsList(year: number | null): Promise<TopSongsList>;
  /** A year's Wrapped. */
  statsWrapped(year: number): Promise<Wrapped>;
  /** Your summary to share with a friend (Compare must be on in Settings). */
  statsCompareSummary(unreleasedOk: boolean, theirUnreleasedOk: boolean): Promise<CompareSummary>;
  /** Compare a friend's summary with yours. */
  statsCompare(theirs: CompareSummary, unreleasedOk: boolean): Promise<CompareResult>;
  /** Summaries friends shared with you. */
  statsFriends(): Promise<CompareSummary[]>;
  /** Forget a friend's shared summary. */
  statsFriendRemove(name: string): Promise<CompareSummary[]>;

  // Miniplayer window
  miniplayerState(): Promise<MiniplayerState>;
  /** Open or close the miniplayer; answers whether it's open now. */
  miniplayerSet(open: boolean): Promise<MiniplayerState>;
  /** Which window this UI is in ("main" or "miniplayer"). */
  windowLabel(): string;
  /** Start resizing this window from its bottom-right corner (the grip). */
  windowResizeFromCorner(): Promise<void>;
  /** The main window's page has drawn its first frame; `background` is the look's colour (#rrggbb). */
  windowReady(background: string | null): Promise<void>;

  /** Subscribe to a core event. Resolves to an unsubscribe function. */
  listen<E extends EventName>(event: E, cb: (payload: EventMap[E]) => void): Promise<Unlisten>;

  /** URL for a Plex image path at a size in device pixels (see ./artSize). */
  artUrl(path: string, w: number, h: number): string;
}

// ------------------------------------------------------------------ Tauri

function toAppError(e: unknown): AppError {
  if (e && typeof e === "object" && "kind" in e && "message" in e) return e as AppError;
  return { kind: "internal", message: typeof e === "string" ? e : String(e) };
}

/** A command whose body is raw bytes (images), with string headers. */
async function invokeRaw<T>(cmd: string, body: Uint8Array, headers: Record<string, string>): Promise<T> {
  try {
    return await invoke<T>(cmd, body, { headers });
  } catch (e) {
    throw toAppError(e);
  }
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toAppError(e);
  }
}

export function createTauriBackend(): Backend {
  return {
    kind: "tauri",
    authStatus: () => call("auth_status"),
    authStart: () => call("auth_start"),
    authCancel: () => call("auth_cancel"),
    authSignOut: () => call("auth_sign_out"),
    serversList: () => call("servers_list"),
    serverSelect: (machineId) => call("server_select", { machineId }),
    homeUsers: () => call("home_users"),
    homeSwitch: (uuid, pin) => call("home_switch", { uuid, pin: pin ?? null }),
    homeRememberSet: (on) => call("home_remember_set", { on }),
    librarySources: () => call("library_sources"),
    libraryServers: () => call("library_servers"),
    backendCaps: () => call("backend_caps"),
    serversSaved: () => call("servers_saved"),
    serverSetPrimary: (tag) => call("server_set_primary", { tag }),
    serverRemove: (tag) => call("server_remove", { tag }),
    serverLibraries: (tag) => call("server_libraries", { tag }),
    serverLibrariesSet: (tag, ids) => call("server_libraries_set", { tag, ids }),
    jellyfinCheck: (url) => call("jellyfin_check", { url }),
    jellyfinQuickStart: (url) => call("jellyfin_quick_start", { url }),
    jellyfinQuickPoll: () => call("jellyfin_quick_poll"),
    jellyfinQuickCancel: () => call("jellyfin_quick_cancel"),
    jellyfinSignInPassword: (url, user, password) => call("jellyfin_sign_in_password", { url, user, password }),
    subsonicCheck: (url) => call("subsonic_check", { url }),
    subsonicSignIn: (url, creds) =>
      call("subsonic_sign_in", { url, user: creds.user ?? null, apiKey: creds.apiKey ?? null, password: creds.password ?? null }),
    resumeQueues: () => call("resume_queues"),
    resumeQueuePlay: (tag) => call("resume_queue_play", { tag }),
    resumeQueueSave: () => call("resume_queue_save"),

    syncStatus: () => call("sync_status"),
    syncNow: (full) => call("sync_now", { full }),
    home: () => call("home"),
    homeShelf: (shelf, limit) => call("home_shelf", { shelf, limit }),
    artists: (args) => call("artists", { ...args }),
    artistNames: () => call("artist_names"),
    albums: (args) => call("albums", { ...args }),
    playlists: () => call("playlists"),
    libraryItems: () => call("library_items"),
    artistPage: (id) => call("artist_page", { id }),
    albumPage: (id) => call("album_page", { id }),
    playlistPage: (id) => call("playlist_page", { id }),
    recentTracks: (limit) => call("recent_tracks", { limit }),
    recents: (limit) => call("recents", { limit }),
    mixesHome: () => call("mixes_home", { tzOffsetMin: -new Date().getTimezoneOffset() }),
    mixPage: (id, steer) => call("mix_page", { id, steer: steer ?? null, tzOffsetMin: -new Date().getTimezoneOffset() }),
    mixRefresh: (id) => call("mix_refresh", { id }),
    radioPage: (seed, steer, salt) => call("radio_page", { seed, steer: steer ?? null, salt: salt ?? null }),
    playlistCreate: (title, trackIds) => call("playlist_create", { title, trackIds }),
    playlistAdd: (id, trackIds, place) => call("playlist_add", { id, trackIds, after: place?.after, top: place?.top }),
    playlistDuplicates: (id, trackIds) => call("playlist_duplicates", { id, trackIds }),
    playlistRemove: (id, itemIds) => call("playlist_remove", { id, itemIds }),
    playlistMove: (id, itemIds, after) => call("playlist_move", { id, itemIds, after: after ?? undefined }),
    playlistUpdate: (id, details) => call("playlist_update", { id, title: details.title, summary: details.summary }),
    playlistSetCover: (id, bytes, contentType) => call("playlist_set_cover", { id, bytes: Array.from(bytes), contentType }),
    playlistDelete: (id) => call("playlist_delete", { id }),
    plexEditAccess: () => call("plex_edit_access"),
    plexEditItem: (id) => call("plex_edit_item", { id }),
    plexEditImages: (id, which) => call("plex_edit_images", { id, which }),
    plexEditSelectImage: (id, kind, which, key) => call("plex_edit_select_image", { id, kind, which, key }),
    plexEditUploadImage: (id, kind, which, bytes, contentType) =>
      call("plex_edit_upload_image", { id, kind, which, bytes: Array.from(bytes), contentType }),
    plexEditImageLink: (id, kind, which, url) => call("plex_edit_image_link", { id, kind, which, url }),
    plexEditMatches: (id, title, year) => call("plex_edit_matches", { id, title: title ?? null, year: year ?? null }),
    plexEditMatch: (id, kind, c) => call("plex_edit_match", { id, kind, guid: c.guid, name: c.name, year: c.year ?? null }),
    plexEditUnmatch: (id, kind) => call("plex_edit_unmatch", { id, kind }),
    plexEditDetails: (id, fields) => call("plex_edit_details", { id, fields }),
    plexEditMerge: (target, others, photoFrom) => call("plex_edit_merge", { target, others, photoFrom: photoFrom ?? null }),
    plexEditMergeAlbums: (target, others) => call("plex_edit_merge_albums", { target, others }),
    plexEditSplit: (id) => call("plex_edit_split", { id }),
    plexEditRefresh: (id, kind) => call("plex_edit_refresh", { id, kind }),
    plexEditUndo: (changeId) => call("plex_edit_undo", { changeId }),
    plexEditHistory: (id) => call("plex_edit_history", { id }),
    tagEditPickMusicFolder: () => call("tagedit_pick_music_folder"),
    tagEditSetMusicFolder: (path) => call("tagedit_set_music_folder", { path }),
    tagEditClearMusicFolder: () => call("tagedit_clear_music_folder"),
    tagEditMusicFolderStatus: () => call("tagedit_music_folder_status"),
    tagEditOpen: (albumId, trackIds) => call("tagedit_open", { albumId, trackIds }),
    tagEditSave: (request) => call("tagedit_save", { request }),
    tagEditUndo: (id) => call("tagedit_undo", { id }),
    tagEditHistory: () => call("tagedit_history"),
    tagEditCheck: (scope) => call("tagedit_check", { scope }),
    tagEditCheckFine: (id, fine) => call("tagedit_check_fine", { id, fine }),
    tagEditFixPreview: (kind, albumId) => call("tagedit_fix_preview", { kind, albumId }),
    tagEditIsrcReleases: (albumId) => call("tagedit_isrc_releases", { albumId }),
    tagEditIsrcPreview: (albumId, release) => call("tagedit_isrc_preview", { albumId, release }),
    tagEditExplicitPreview: (albumIds, explicit, era) =>
      call("tagedit_explicit_preview", { albumIds, explicit, artistId: era?.artistId ?? null, era: era?.era ?? null }),
    tagEditLibrarySettings: () => call("tagedit_library_settings"),
    likesSet: (trackIds, liked) => call("likes_set", { trackIds, liked }),
    likedIds: () => call("liked_ids"),
    likedPage: () => call("liked_page"),
    playlistsWithTrack: (trackId) => call("playlists_with_track", { trackId }),
    nowPlayingDetails: (trackId) => call("now_playing_details", { trackId }),
    lyrics: (trackId) => call("lyrics_get", { trackId }),
    ownLyrics: (trackId) => call("lyrics_own_get", { trackId }),
    ownLyricsSave: (trackId, text, prefer) => call("lyrics_own_save", { trackId, text, prefer }),
    ownLyricsDelete: (trackId) => call("lyrics_own_delete", { trackId }),
    ownLyricsIds: () => call("lyrics_own_ids"),
    lyricsExport: (text, fileName) => call("lyrics_export", { text, fileName }),
    lyricsCandidates: (trackId) => call("lyrics_candidates", { trackId }),
    lyricsCheckStatus: (trackId) => call("lyrics_check_status", { trackId }),
    lyricsChoose: (trackId, pick) => call("lyrics_choose", { trackId, pick }),
    lyricsOffset: (trackId, offsetMs) => call("lyrics_offset", { trackId, offsetMs }),
    lyricsChoicesCount: () => call("lyrics_choices_count"),
    lyricsChoicesClear: () => call("lyrics_choices_clear"),
    lyricsPronounce: (lines) => call("lyrics_pronounce", { lines }),
    lyricsTranslate: (lines) => call("lyrics_translate", { lines }),

    search: (args) => call("search", { ...args }),
    searchCompletions: (query, limit) => call("search_completions", { query, limit }),
    searchRecents: () => call("search_recents"),
    searchRecentAdd: (kind, id) => call("search_recent_add", { kind, id }),
    searchRecentRemove: (kind, id) => call("search_recent_remove", { kind, id }),
    searchRecentsClear: () => call("search_recents_clear"),

    playContext: (args) => call("play_context", { ...args }),
    playToggle: () => call("play_toggle"),
    playPause: () => call("play_pause"),
    playResume: () => call("play_resume"),
    playNext: () => call("play_next"),
    playPrevious: () => call("play_previous"),
    playSeek: (positionMs) => call("play_seek", { positionMs }),
    setVolume: (volume) => call("set_volume", { volume }),
    setMuted: (muted) => call("set_muted", { muted }),
    setShuffle: (shuffle) => call("set_shuffle", { shuffle }),
    setContextShuffle: (contextId, shuffle) => call("set_context_shuffle", { contextId, shuffle }),
    setSmartShuffle: (on) => call("set_smart_shuffle", { on }),
    setContextSmartShuffle: (contextId, on) => call("set_context_smart_shuffle", { contextId, on }),
    smartSuggestionRemove: () => call("smart_suggestion_remove"),
    smartSuggestionKeep: (itemId) => call("smart_suggestion_keep", { itemId }),
    setSmoothTransitions: (on) => call("set_smooth_transitions", { on }),
    setRepeat: (repeat) => call("set_repeat", { repeat }),
    setAbLoop: (abLoop) => call("set_ab_loop", { abLoop }),
    queueAdd: (trackIds, at) => call("queue_add", { trackIds, at }),
    queueRemove: (itemId) => call("queue_remove", { itemId }),
    queueMove: (itemId, toIndex) => call("queue_move", { itemId, toIndex }),
    queueClear: () => call("queue_clear"),
    queueJump: (itemId) => call("queue_jump", { itemId }),
    playerState: () => call("player_state"),
    outputDevices: () => call("output_devices"),
    outputSelect: (deviceId) => call("output_select", { deviceId }),
    remotePlayers: () => call("remote_players"),
    remoteState: () => call("remote_state"),
    remoteMove: (playerId) => call("remote_move", { playerId }),
    remoteReturn: () => call("remote_return"),
    remoteQueueJump: (itemId) => call("remote_queue_jump", { itemId }),
    remotePlayContext: (args) => call("remote_play_context", { context: args.context, startTrackId: args.startTrackId }),
    remoteCommand: (command) => call("remote_command", { command }),
    remoteMoveWithPassword: (playerId, password) => call("remote_move_with_password", { playerId, password }),

    health: () => call("health"),
    cachedTracks: () => call("cached_tracks"),
    cachedLibrary: () => call("cached_library"),
    openLocalNetworkSettings: () => call("open_local_network_settings"),
    settingsGet: () => call("settings_get"),
    settingsSet: (patch) => call("settings_set", { patch }),
    settingsDefaults: () => call("settings_defaults"),
    settingsExport: () => call("settings_export"),
    settingsImport: () => call("settings_import"),
    helpOpenLink: (url) => call("help_open_link", { url }),
    themesList: () => call("themes_list"),
    themesImport: () => call("themes_import"),
    themesOpenFolder: () => call("themes_open_folder"),
    themesImportText: (text) => call("themes_import_text", { text }),
    themesSave: (look, replace) => call("themes_save", { look, replace }),
    themesExport: (look) => call("themes_export", { look }),
    eqProfileSave: (name) => call("eq_profile_save", { name }),
    eqProfileRename: (from, to) => call("eq_profile_rename", { from, to }),
    eqProfileDelete: (name) => call("eq_profile_delete", { name }),
    eqUseForOutput: (on) => call("eq_use_for_output", { on }),
    headphoneSearch: (query) => call("headphone_search", { query }),
    headphoneCorrectionSet: (path) => call("headphone_correction_set", { path }),
    headphoneCorrectionCompare: (bypass) => call("headphone_correction_compare", { bypass }),
    bitPerfectSet: (on) => call("bit_perfect_set", { on }),
    artCacheInfo: () => call("art_cache_info"),
    artCacheClear: () => call("art_cache_clear"),
    appInfo: () => call("app_info"),
    openProjectPage: () => call("open_project_page"),
    updateCheck: () => call("update_check"),
    updateInstallOnRestart: () => call("update_install_on_restart"),
    artistInfo: (id) => call("artist_info", { id }),
    albumCredits: (id) => call("album_credits", { id }),
    albumReleaseStatus: (id) => call("album_release_status", { id }),
    releaseMarkSet: (kind, id, mark) => call("release_mark_set", { kind, id, mark }),
    trackVersions: (id) => call("track_versions", { id }),
    trackLineage: (id) => call("track_lineage", { id }),
    comparePrepare: (aId, bId) => call("compare_prepare", { aId, bId }),
    compareSwitch: (side, play) => call("compare_switch", { side, play: play ?? null }),
    compareLoop: (ab) => call("compare_loop", { ab }),
    compareEnd: () => call("compare_end"),
    eraPage: (artistId, name) => call("era_page", { artistId, name }),
    albumSectionSet: (albumId, artistId, name) => call("album_section_set", { albumId, artistId, name }),
    artistSections: (artistId) => call("artist_sections", { artistId }),
    doctorReport: () => call("doctor_report"),
    doctorCount: () => call("doctor_count"),
    doctorIgnore: (id, kind, title, ignore) => call("doctor_ignore", { id, kind, title, ignore }),
    doctorRun: () => call("doctor_run"),
    soundIdStatus: () => call("sound_id_status"),
    soundIdKeySet: (key) => call("sound_id_key_set", { key }),
    trackSoundsLike: (id) => call("track_sounds_like", { id }),
    artistInfoKeys: () => call("artist_info_keys"),
    artistInfoKeySet: (source, key) => call("artist_info_key_set", { source, key }),
    artistInfoCacheClear: () => call("artist_info_cache_clear"),
    openInfoLink: (url, artistId) => call("open_info_link", { url, artistId: artistId ?? null }),
    shareInfo: (kind, id, atSeconds) => call("share_info", { kind, id, atSeconds }),
    sharePublicLink: (kind, id) => call("share_public_link", { kind, id }),
    deepLinkTake: () => call("deep_link_take"),
    shareCardQr: (kind, id, atSeconds) => call("share_card_qr", { kind, id, atSeconds: atSeconds ?? null }),
    shareCardCopy: (rgba, width, height) =>
      invokeRaw("share_card_copy", rgba, { "x-card-width": String(width), "x-card-height": String(height) }),
    shareCardSave: (png, name) => invokeRaw("share_card_save", png, { "x-card-name": encodeURIComponent(name) }),
    discordStatus: () => call("discord_status"),
    feedStatus: () => call("feed_status"),
    deckStatus: () => call("deck_status"),
    deckRevoke: () => call("deck_revoke"),
    deckUiReport: (report) => call("deck_ui_report", { report }),
    automationRevoke: () => call("automation_revoke"),
    deckPairPrompt: () => call("deck_pair_prompt"),
    deckPairAnswer: (id, allow) => call("deck_pair_answer", { id, allow }),
    deckPluginStatus: () => call("deck_plugin_status"),
    deckPluginInstall: () => call("deck_plugin_install"),
    bridgeOverview: () => call("bridge_overview"),
    bridgePairStart: () => call("bridge_pair_start"),
    bridgePairCancel: () => call("bridge_pair_cancel"),
    bridgeUnpair: (id) => call("bridge_unpair", { id }),
    bridgeQueuedApply: (id) => call("bridge_queued_apply", { id }),
    bridgeQueuedDismiss: (id) => call("bridge_queued_dismiss", { id }),
    bridgeSendSettings: () => call("bridge_send_settings"),
    deckAppDownload: () => call("deck_app_download"),
    discordImgurKeySet: (key) => call("discord_imgur_key_set", { key }),
    tagsIndex: () => call("tags_index"),
    tagScanStatus: () => call("tag_scan_status"),
    transcribeStatus: () => call("transcribe_status"),
    transcribeModelDownload: (model) => call("transcribe_model_download", { model }),
    transcribeModelCancel: () => call("transcribe_model_cancel"),
    transcribeModelDelete: (model) => call("transcribe_model_delete", { model }),
    transcribeTrack: (trackId) => call("transcribe_track", { trackId }),
    transcribeAlbum: (albumId) => call("transcribe_album", { albumId }),
    transcribeUnreleased: () => call("transcribe_unreleased"),
    transcribeCancel: (trackId) => call("transcribe_cancel", { trackId: trackId ?? null }),
    transcribePause: (paused) => call("transcribe_pause", { paused }),
    transcribeRedo: () => call("transcribe_redo"),
    transcribeFoundNothing: (trackId) => call("transcribe_found_nothing", { trackId }),
    lyricsShareInfo: (trackId) => call("lyrics_share_info", { trackId }),
    lyricsShareReviewed: (trackId) => call("lyrics_share_reviewed", { trackId }),
    lyricsSharePublish: (trackId) => call("lyrics_share_publish", { trackId }),
    lyricsShareCancel: (trackId) => call("lyrics_share_cancel", { trackId }),
    downloadsStatus: () => call("downloads_status"),
    downloadedTracks: () => call("downloaded_tracks"),
    downloadsSet: (kind, id, on, opts) =>
      call("downloads_set", { kind, id, on, title: opts?.title ?? null, trackIds: opts?.trackIds ?? null }),
    downloadsRetry: (kind, id) => call("downloads_retry", { kind: kind ?? null, id: id ?? null }),
    downloadsRemoveAll: () => call("downloads_remove_all"),
    downloadsChooseFolder: () => call("downloads_choose_folder"),
    devicesList: () => call("devices_list"),
    devicesAdd: (root, name, folder) => call("devices_add", { root, name, folder }),
    devicesChooseFolder: (root) => call("devices_choose_folder", { root }),
    devicesUpdate: (id, patch) => call("devices_update", { id, patch }),
    devicesSelect: (id, kind, itemId, title, on) => call("devices_select", { id, kind, itemId, title, on }),
    devicesForget: (id) => call("devices_forget", { id }),
    devicesSync: (id) => call("devices_sync", { id }),
    devicesPause: (on) => call("devices_pause", { on }),
    devicesCancel: () => call("devices_cancel"),
    devicesStatus: () => call("devices_status"),
    soundmapInfo: () => call("soundmap_info"),
    soundmapPoints: (after, limit, area) => call("soundmap_points", { after: after ?? null, limit: limit ?? null, area: area ?? null }),
    soundmapFind: (query) => call("soundmap_find", { query }),
    soundmapSong: (id) => call("soundmap_song", { id }),
    friendActivity: (fresh) => call("friend_activity", { fresh: fresh ?? null }),
    followsList: () => call("follows_list"),
    syncFileStatus: () => call("sync_file_status"),
    syncFileNow: () => call("sync_file_now"),
    syncFileFolders: () => call("sync_file_folders"),
    syncFileUseFolder: (path) => call("sync_file_use_folder", { path }),
    syncFileChooseFolder: () => call("sync_file_choose_folder"),
    syncFileReplace: () => call("sync_file_replace"),
    pinsGet: () => call("pins_get"),
    pinsToggle: (pin) => call("pins_toggle", { pin }),
    pinsImport: (pins) => call("pins_import", { pins }),
    automixMixedGet: () => call("automix_mixed_get"),
    automixMixSet: (id, on) => call("automix_mix_set", { id, on }),
    automixStyleSet: (id, fromId, toId, style) => call("automix_style_set", { id, fromId, toId, style }),
    automixView: (playlistId, trackIds) => call("automix_view", { playlistId, trackIds }),
    foldersGet: () => call("folders_get"),
    folderCreate: (name, parent) => call("folder_create", { name: name ?? null, parent: parent ?? null }),
    folderRename: (id, name) => call("folder_rename", { id, name }),
    folderDelete: (id) => call("folder_delete", { id }),
    folderMove: (id, parent) => call("folder_move", { id, parent }),
    playlistFolderSet: (playlistId, folderId) => call("playlist_folder_set", { playlistId, folderId }),
    keysAvailable: () => call("keys_available"),
    keysReceiveStart: () => call("keys_receive_start"),
    keysReceiveState: () => call("keys_receive_state"),
    keysReceiveCancel: () => call("keys_receive_cancel"),
    keysSend: (code, kinds) => call("keys_send", { code, kinds }),
    followSet: (artist, on) => call("follow_set", { id: artist.id, title: artist.title, thumb: artist.thumb ?? null, on }),
    whatsNew: () => call("whats_new"),
    whatsNewSeen: () => call("whats_new_seen"),
    listenState: () => call("listen_state"),
    listenStart: () => call("listen_start"),
    listenStop: () => call("listen_stop"),
    listenJoin: (link) => call("listen_join", { link }),
    listenAdd: (trackIds) => call("listen_add", { trackIds }),
    listenInvite: (fresh) => call("listen_invite", { fresh: fresh ?? false }),
    listenControl: (op) => call("listen_control", { op }),
    listenRemove: (personId) => call("listen_remove", { personId }),
    circleState: () => call("circle_state"),
    circlePeople: () => call("circle_people"),
    circleJoin: (link) => call("circle_join", { link }),
    circleAccept: (id) => call("circle_accept", { id }),
    circleDismiss: (id) => call("circle_dismiss", { id }),
    collabInfo: (playlistId) => call("collab_info", { playlistId }),
    collabInviteLink: (playlistId, role) => call("collab_invite_link", { playlistId, role }),
    collabInvitePerson: (playlistId, key, role) => call("collab_invite_person", { playlistId, key, role }),
    collabPage: (id) => call("collab_page", { id }),
    collabAdd: (id, trackIds) => call("collab_add", { id, trackIds }),
    collabRemove: (id, keys) => call("collab_remove", { id, keys }),
    collabLeave: (id) => call("collab_leave", { id }),
    collabSetInLibrary: (id, on) => call("collab_set_in_library", { id, on }),
    collabSetRole: (playlistId, personId, role) => call("collab_set_role", { playlistId, personId, role }),
    collabRemovePerson: (playlistId, personId) => call("collab_remove_person", { playlistId, personId }),
    collabSetPublic: (playlistId, isPublic) => call("collab_set_public", { playlistId, public: isPublic }),
    blendCreate: (keys) => call("blend_create", { keys }),
    blendInviteLink: (id) => call("blend_invite_link", { id }),
    blendInvitePerson: (id, key) => call("blend_invite_person", { id, key }),
    blendPage: (id) => call("blend_page", { id }),
    blendHide: (id, trackId) => call("blend_hide", { id, trackId }),
    blendRefresh: (id) => call("blend_refresh", { id }),
    blendLeave: (id) => call("blend_leave", { id }),
    listenOfferServer: (personId) => call("listen_offer_server", { personId }),
    sharingOverview: () => call("sharing_overview"),
    sharingShare: (invitee, options) => call("sharing_share", { invitee, options }),
    sharingPeople: () => call("sharing_people"),
    sharingRemove: (id) => call("sharing_remove", { id }),
    sharingLink: (options) => call("sharing_link", { options }),
    sharingOpenLink: (link) => call("sharing_open_link", { link }),
    sharingLinkAsks: () => call("sharing_link_asks"),
    sharingLinkApprove: (id) => call("sharing_link_approve", { id }),
    sharingLinkDeny: (id) => call("sharing_link_deny", { id }),
    sharingInvites: (fresh) => call("sharing_invites", { fresh: fresh ?? true }),
    sharingAccept: (id) => call("sharing_accept", { id }),
    hiddenList: () => call("hidden_list"),
    hiddenSet: (item, on) => call("hidden_set", { item, on }),
    bookmarksList: () => call("bookmarks_list"),
    bookmarkAdd: (bookmark) => call("bookmark_add", { bookmark }),
    bookmarkRemove: (id) => call("bookmark_remove", { id }),
    bookmarkOpen: (id) => call("bookmark_open", { id }),
    smartPreview: (rules, picks) => call("smart_preview", { rules, picks: picks ?? null, tzOffsetMin: -new Date().getTimezoneOffset() }),
    smartSave: (request) => call("smart_save", { request }),
    smartList: () => call("smart_list"),
    smartPage: (id) => call("smart_page", { id, tzOffsetMin: -new Date().getTimezoneOffset() }),
    smartDelete: (id) => call("smart_delete", { id }),
    smartPlexRules: (id) => call("smart_plex_rules", { id }),
    smartPrompt: (prompt) => call("smart_prompt", { prompt }),
    smartDescribe: (text) => call("smart_describe", { text, tzOffsetMin: -new Date().getTimezoneOffset() }),
    aiStatus: () => call("ai_status"),
    aiKeySet: (key) => call("ai_key_set", { key }),
    scrobbleStatus: () => call("scrobble_status"),
    lastfmSecretSet: (secret) => call("lastfm_secret_set", { secret }),
    lastfmSignInStart: () => call("lastfm_sign_in_start"),
    lastfmSignInFinish: () => call("lastfm_sign_in_finish"),
    lastfmSignOut: () => call("lastfm_sign_out"),
    listenbrainzCheck: () => call("listenbrainz_check"),
    lidarrStatus: () => call("lidarr_status"),
    lidarrKeySet: (key) => call("lidarr_key_set", { key }),
    lidarrHeadersSet: (text) => call("lidarr_headers_set", { text }),
    lidarrCheck: () => call("lidarr_check"),
    lidarrConfirmRoot: (path, confirmed) => call("lidarr_confirm_root", { path, confirmed }),
    importOverview: () => call("import_overview"),
    importChoose: (folder) => call("import_choose", { folder }),
    importRun: (source, path) => call("import_run", { source, path }),
    importReviewQueue: () => call("import_review_queue"),
    importReview: (songId, decision) => call("import_review", { songId, decision }),
    importListPage: (id) => call("import_list_page", { id }),
    importMissing: () => call("import_missing"),
    importMissingAlbums: () => call("import_missing_albums"),
    importMissingAlbumAdd: (album) => call("import_missing_album_add", { album }),
    importMissingAlbumRemove: (releaseGroupId) => call("import_missing_album_remove", { releaseGroupId }),
    catalogueSearch: (query) => call("catalogue_search", { query }),
    catalogueSearchPart: (query, part) => call("catalogue_search_part", { query, part }),
    catalogueArtistPhoto: (mbid, name) => call("catalogue_artist_photo", { mbid, name }),
    catalogueGetAlbumsPlan: (mbid, name, kinds) => call("catalogue_get_albums_plan", { mbid, name, kinds }),
    catalogueGetAlbums: (mbid, name, kinds) => call("catalogue_get_albums", { mbid, name, kinds }),
    catalogueArtist: (mbid) => call("catalogue_artist", { mbid }),
    catalogueAlbum: (releaseGroupId) => call("catalogue_album", { releaseGroupId }),
    catalogueFansAlsoLike: (artistId, name) => call("catalogue_fans_also_like", { artistId, name }),
    newToYou: () => call("new_to_you"),
    physicalList: () => call("physical_list"),
    physicalSet: (albumId, owned) => call("physical_set", { albumId, owned }),
    physicalStats: () => call("physical_stats"),
    physicalFindRelease: (artist, title) => call("physical_find_release", { artist, title }),
    playHistoryStatus: () => call("play_history_status"),
    playHistoryUpdate: () => call("play_history_update"),
    playHistoryCancel: () => call("play_history_cancel"),
    playHistoryRemove: (source) => call("play_history_remove", { source }),
    tautulliStatus: () => call("tautulli_status"),
    tautulliKeySet: (key) => call("tautulli_key_set", { key }),
    discogsStatus: () => call("discogs_status"),
    discogsTokenSet: (token) => call("discogs_token_set", { token }),
    discogsImport: () => call("discogs_import"),
    importOpenLink: (url) => call("import_open_link", { url }),
    importForget: (source) => call("import_forget", { source }),
    importListRemove: (listId, songId) => call("import_list_remove", { listId, songId }),
    importPlayed: (query) => call("import_played", { query }),
    importPlayedHide: (kind, artist, name, hide) => call("import_played_hide", { kind, artist, name, hide }),
    importPlayedHidden: () => call("import_played_hidden"),
    lidarrPick: ({ artist, artistId, title, song, isrc }) => call("lidarr_pick", { artist, artistId: artistId ?? null, title, song, isrc: isrc ?? null }),
    statsOverview: (range, limit) => call("stats_overview", { range, limit: limit ?? null }),
    statsSpan: () => call("stats_span"),
    statsMoments: (range) => call("stats_moments", { range }),
    statsDays: (from, to) => call("stats_days", { from, to }),
    statsItem: (kind, id) => call("stats_item", { kind, id }),
    statsMonthRecap: (month) => call("stats_month_recap", { month }),
    statsPlayIds: (range, kind, limit) => call("stats_play_ids", { range, kind, limit: limit ?? null }),
    topSongsIndex: () => call("top_songs_index"),
    topSongsList: (year) => call("top_songs_list", { year }),
    statsWrapped: (year) => call("stats_wrapped", { year }),
    statsCompareSummary: (unreleasedOk, theirUnreleasedOk) => call("stats_compare_summary", { unreleasedOk, theirUnreleasedOk }),
    statsCompare: (theirs, unreleasedOk) => call("stats_compare", { theirs, unreleasedOk }),
    statsFriends: () => call("stats_friends"),
    statsFriendRemove: (name) => call("stats_friend_remove", { name }),
    lidarrRequestPreview: (input) => call("lidarr_request_preview", { input }),
    lidarrRequest: (input) => call("lidarr_request", { input }),
    lidarrGet: (input) => call("lidarr_get", { input }),
    lidarrSearchAgain: (id) => call("lidarr_search_again", { id }),
    getAlbumsUnqueue: (releaseGroupIds) => call("get_albums_unqueue", { releaseGroupIds }),
    lidarrRequests: (refresh) => call("lidarr_requests", { refresh }),
    lidarrRequestBatch: (inputs) => call("lidarr_request_batch", { inputs }),
    bringInStatus: () => call("bring_in_status"),
    bringInPause: (paused) => call("bring_in_pause", { paused }),
    bringInNeedsYou: (key, retry) => call("bring_in_needs_you", { key, retry }),
    lidarrRequestCancel: (id) => call("lidarr_request_cancel", { id }),
    lidarrMissingReleases: (artistId, artist) => call("lidarr_missing_releases", { artistId, artist }),
    lidarrUpgradeAlbum: (albumId) => call("lidarr_upgrade_album", { albumId }),
    lidarrUpgradeRequest: (albumId) => call("lidarr_upgrade_request", { albumId }),
    lidarrUpgradeBulk: (under) => call("lidarr_upgrade_bulk", { under }),
    lidarrUpgrades: (refresh) => call("lidarr_upgrades", { refresh }),
    lidarrUpgradeOffer: (id) => call("lidarr_upgrade_offer", { id }),
    lidarrUpgradeSwap: (id, trackIds) => call("lidarr_upgrade_swap", { id, trackIds }),
    lidarrUpgradeUndo: (id) => call("lidarr_upgrade_undo", { id }),
    lidarrUpgradeClear: (id) => call("lidarr_upgrade_clear", { id }),
    lidarrUpgradeCandidates: (albumId) => call("lidarr_upgrade_candidates", { albumId }),
    lidarrUpgradeConfirm: (albumId, releaseGroupId) => call("lidarr_upgrade_confirm", { albumId, releaseGroupId }),
    lidarrAllowlist: (artist, artistId) => call("lidarr_allowlist", { artist, artistId }),
    lidarrAllowlistSet: (artist, artistId, releaseGroupIds) => call("lidarr_allowlist_set", { artist, artistId, releaseGroupIds }),
    lidarrModes: () => call("lidarr_modes"),
    lidarrModeSet: (key, name, mode, confirmations) => call("lidarr_mode_set", { key, name, mode, confirmations }),
    lidarrArtistChoices: () => call("lidarr_artist_choices"),
    lidarrArtistChoicesSet: (key, name, autoAdd, hunt, target) => call("lidarr_artist_choices_set", { key, name, autoAdd, hunt, target }),
    lidarrChangePreview: (action, input) => call("lidarr_change_preview", { action, input }),
    lidarrHiresPreview: () => call("lidarr_hires_preview"),
    lidarrChangeApply: (id) => call("lidarr_change_apply", { id }),
    lidarrChanges: () => call("lidarr_changes"),
    lidarrChangePutBack: (id) => call("lidarr_change_put_back", { id }),
    lidarrRematchPreview: (folder) => call("lidarr_rematch_preview", { folder }),
    lidarrRematchApply: (folder, releaseId) => call("lidarr_rematch_apply", { folder, releaseId }),
    lidarrWork: () => call("lidarr_work"),
    lidarrCatchUp: () => call("lidarr_catch_up"),
    lidarrResumeProtected: () => call("lidarr_resume_protected"),
    lidarrStagedAdd: (id, always) => call("lidarr_staged_add", { id, always }),
    lidarrStagedUndo: (id, trashOk) => call("lidarr_staged_undo", { id, trashOk }),
    lidarrHunt: () => call("lidarr_hunt"),
    musicShareGet: () => call("music_share_get"),
    musicShareSet: (map) => call("music_share_set", { map }),
    musicSharePlexFolders: () => call("music_share_plex_folders"),
    releaseRadar: () => call("release_radar"),
    releaseRadarCheck: () => call("release_radar_check"),
    radarAutoArtists: () => call("radar_auto_artists"),
    radarAutoSet: (id, name, on) => call("radar_auto_set", { id, name, on }),
    miniplayerState: () => call("miniplayer_state"),
    miniplayerSet: (open) => call("miniplayer_set", { open }),
    windowLabel: () => getCurrentWindow().label,
    windowResizeFromCorner: () => getCurrentWindow().startResizeDragging("SouthEast"),
    windowReady: (background) => call("window_ready", { background }),

    listen: async (event, cb) => {
      const un = await tauriListen<EventMap[typeof event]>(event, (e) => cb(e.payload));
      return un;
    },

    artUrl: (path, w, h) => {
      // convertFileSrc yields art://localhost/<enc> (macOS) or
      // http://art.localhost/<enc> (Windows WebView2).
      const base = convertFileSrc(path, "art");
      return `${base}?w=${Math.round(w)}&h=${Math.round(h)}`;
    },
  };
}

// ------------------------------------------------------------------ selection

export function isTauri(win: unknown = typeof window !== "undefined" ? window : undefined): boolean {
  return !!win && typeof win === "object" && "__TAURI_INTERNALS__" in win;
}

export function selectBackend(win?: unknown): Backend {
  // PROD is a build-time constant, so the mock is tree-shaken out of releases.
  if (import.meta.env.PROD) return createTauriBackend();
  return isTauri(win ?? (typeof window !== "undefined" ? window : undefined))
    ? createTauriBackend()
    : createMockBackend();
}

export const backend: Backend = selectBackend();

/** Image URL for a Plex art path shown at a size in CSS px (scaled for the display). */
export function artUrl(path: string | null | undefined, w: number, h: number = w): string | undefined {
  if (!path) return undefined;
  return backend.artUrl(path, artPixelSize(w), artPixelSize(h));
}
