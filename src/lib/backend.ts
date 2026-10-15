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
  CatArtist,
  CatalogueAlbumPage,
  CatalogueArtistPage,
  CatalogueSearch,
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
  DeepLinkOpen,
  DiscordStatus,
  FeedStatus,
  DeckStatus,
  DownloadKind,
  DownloadStatus,
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
  QueueAt,
  Track,
  ActivityPanel,
  FollowedArtist,
  WhatsNew,
  ListenView,
  ListenOp,
  AiStatus,
  LidarrMissing,
  LidarrPickInput,
  LidarrPicked,
  LidarrPreview,
  LidarrReport,
  LidarrRequest,
  LidarrRequestInput,
  LidarrStatus,
  RadarPage,
  AutoArtist,
  Bookmark,
  HiddenItem,
  HideRequest,
  NewBookmark,
  PromptResult,
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
  StatsSpan,
  StatsTopKind,
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
  /** The song was transcribed and no words were found. */
  transcribeFoundNothing(trackId: string): Promise<boolean>;

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
  /** Requests on this device, newest first; refresh reads their state from Lidarr. */
  lidarrRequests(refresh: boolean): Promise<LidarrRequest[]>;
  /** Cancel a waiting request, or clear a finished one. */
  lidarrRequestCancel(id: string): Promise<LidarrRequest[]>;
  /** An artist's official releases the library lacks, for the artist page. */
  lidarrMissingReleases(artistId: string, artist: string): Promise<LidarrMissing>;

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

  // Listening stats (docs/IPC.md)
  /** Totals, chart, streak, busiest times and top lists for a range of days. */
  statsOverview(range: StatsRange, limit?: number): Promise<StatsOverview>;
  /** The first and last day with plays and the months with plays. */
  statsSpan(): Promise<StatsSpan>;
  /** One song's, artist's or album's numbers (the stats strip). */
  statsItem(kind: StatsItemKind, id: string): Promise<ItemStats>;
  /** A month's recap ("YYYY-MM"). */
  statsMonthRecap(month: string): Promise<MonthRecap>;
  /** Songs for a top list's Play these and Save as playlist, most played first. */
  statsPlayIds(range: StatsRange, kind: StatsTopKind, limit?: number): Promise<string[]>;
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
    transcribeFoundNothing: (trackId) => call("transcribe_found_nothing", { trackId }),
    downloadsStatus: () => call("downloads_status"),
    downloadedTracks: () => call("downloaded_tracks"),
    downloadsSet: (kind, id, on, opts) =>
      call("downloads_set", { kind, id, on, title: opts?.title ?? null, trackIds: opts?.trackIds ?? null }),
    downloadsRetry: (kind, id) => call("downloads_retry", { kind: kind ?? null, id: id ?? null }),
    downloadsRemoveAll: () => call("downloads_remove_all"),
    downloadsChooseFolder: () => call("downloads_choose_folder"),
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
    hiddenList: () => call("hidden_list"),
    hiddenSet: (item, on) => call("hidden_set", { item, on }),
    bookmarksList: () => call("bookmarks_list"),
    bookmarkAdd: (bookmark) => call("bookmark_add", { bookmark }),
    bookmarkRemove: (id) => call("bookmark_remove", { id }),
    bookmarkOpen: (id) => call("bookmark_open", { id }),
    smartPreview: (rules, picks) => call("smart_preview", { rules, picks: picks ?? null }),
    smartSave: (request) => call("smart_save", { request }),
    smartList: () => call("smart_list"),
    smartPage: (id) => call("smart_page", { id }),
    smartDelete: (id) => call("smart_delete", { id }),
    smartPlexRules: (id) => call("smart_plex_rules", { id }),
    smartPrompt: (prompt) => call("smart_prompt", { prompt }),
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
    catalogueArtist: (mbid) => call("catalogue_artist", { mbid }),
    catalogueAlbum: (releaseGroupId) => call("catalogue_album", { releaseGroupId }),
    catalogueFansAlsoLike: (artistId, name) => call("catalogue_fans_also_like", { artistId, name }),
    newToYou: () => call("new_to_you"),
    importOpenLink: (url) => call("import_open_link", { url }),
    importForget: (source) => call("import_forget", { source }),
    importListRemove: (listId, songId) => call("import_list_remove", { listId, songId }),
    lidarrPick: ({ artist, artistId, title, song, isrc }) => call("lidarr_pick", { artist, artistId: artistId ?? null, title, song, isrc: isrc ?? null }),
    statsOverview: (range, limit) => call("stats_overview", { range, limit: limit ?? null }),
    statsSpan: () => call("stats_span"),
    statsItem: (kind, id) => call("stats_item", { kind, id }),
    statsMonthRecap: (month) => call("stats_month_recap", { month }),
    statsPlayIds: (range, kind, limit) => call("stats_play_ids", { range, kind, limit: limit ?? null }),
    statsCompareSummary: (unreleasedOk, theirUnreleasedOk) => call("stats_compare_summary", { unreleasedOk, theirUnreleasedOk }),
    statsCompare: (theirs, unreleasedOk) => call("stats_compare", { theirs, unreleasedOk }),
    statsFriends: () => call("stats_friends"),
    statsFriendRemove: (name) => call("stats_friend_remove", { name }),
    lidarrRequestPreview: (input) => call("lidarr_request_preview", { input }),
    lidarrRequest: (input) => call("lidarr_request", { input }),
    lidarrRequests: (refresh) => call("lidarr_requests", { refresh }),
    lidarrRequestCancel: (id) => call("lidarr_request_cancel", { id }),
    lidarrMissingReleases: (artistId, artist) => call("lidarr_missing_releases", { artistId, artist }),
    releaseRadar: () => call("release_radar"),
    releaseRadarCheck: () => call("release_radar_check"),
    radarAutoArtists: () => call("radar_auto_artists"),
    radarAutoSet: (id, name, on) => call("radar_auto_set", { id, name, on }),
    miniplayerState: () => call("miniplayer_state"),
    miniplayerSet: (open) => call("miniplayer_set", { open }),
    windowLabel: () => getCurrentWindow().label,
    windowResizeFromCorner: () => getCurrentWindow().startResizeDragging("SouthEast"),

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
