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
import { artPixelSize } from "./artSize";
import { createMockBackend } from "./mock";
import type {
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
  ArtistPage,
  ArtistsArgs,
  AuthStatus,
  EventMap,
  EventName,
  Health,
  HomeData,
  HomeItem,
  HomeShelfId,
  InfoKeyState,
  InfoKeysStatus,
  KeyedInfoSource,
  LibraryItem,
  LikedPage,
  Lyrics,
  OwnLyrics,
  NowPlayingDetails,
  OutputState,
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
  ShareInfo,
  ShareKind,
  SyncStatus,
  UpdateInfo,
  TagIndex,
  TagScanStatus,
  TranscribeModel,
  TranscribeStatus,
  QueueAt,
} from "./types";

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

  // Settings
  settingsGet(): Promise<Settings>;
  settingsSet(patch: Partial<Settings>): Promise<void>;
  /** Saves the current EQ curve as a profile (replacing one with that name) and selects it. */
  eqProfileSave(name: string): Promise<Settings>;
  eqProfileRename(from: string, to: string): Promise<Settings>;
  eqProfileDelete(name: string): Promise<Settings>;
  /** Uses the selected EQ preset or profile whenever the output in use plays (false removes it). */
  eqUseForOutput(on: boolean): Promise<Settings>;
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
  artistInfoKeys(): Promise<InfoKeysStatus>;
  /** Save a source's key in the keychain (null removes it); answers whether it was refused. */
  artistInfoKeySet(source: KeyedInfoSource, key: string | null): Promise<InfoKeyState>;
  /** Forget cached artist info; resolves to how many artists had some. */
  artistInfoCacheClear(): Promise<number>;
  /** Open a source or licence link in the default browser. */
  openInfoLink(url: string): Promise<void>;

  // Sharing (docs/IPC.md "Sharing")
  /** The Platter link (with a start time for songs) and whether a public link can be had. */
  shareInfo(kind: ShareKind, id: string, atSeconds?: number): Promise<ShareInfo>;
  /** Find the public link for a released song or album (notFound when there's no sure match). */
  sharePublicLink(kind: ShareKind, id: string): Promise<string>;
  /** The platter:// link opened last, once (null when none is waiting). */
  deepLinkTake(): Promise<DeepLinkOpen | null>;

  // Discord status (docs/IPC.md "Discord")
  discordStatus(): Promise<DiscordStatus>;
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
    playlistCreate: (title, trackIds) => call("playlist_create", { title, trackIds }),
    playlistAdd: (id, trackIds, place) => call("playlist_add", { id, trackIds, after: place?.after, top: place?.top }),
    playlistDuplicates: (id, trackIds) => call("playlist_duplicates", { id, trackIds }),
    playlistRemove: (id, itemIds) => call("playlist_remove", { id, itemIds }),
    playlistMove: (id, itemIds, after) => call("playlist_move", { id, itemIds, after: after ?? undefined }),
    playlistUpdate: (id, details) => call("playlist_update", { id, title: details.title, summary: details.summary }),
    playlistSetCover: (id, bytes, contentType) => call("playlist_set_cover", { id, bytes: Array.from(bytes), contentType }),
    playlistDelete: (id) => call("playlist_delete", { id }),
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
    settingsGet: () => call("settings_get"),
    settingsSet: (patch) => call("settings_set", { patch }),
    eqProfileSave: (name) => call("eq_profile_save", { name }),
    eqProfileRename: (from, to) => call("eq_profile_rename", { from, to }),
    eqProfileDelete: (name) => call("eq_profile_delete", { name }),
    eqUseForOutput: (on) => call("eq_use_for_output", { on }),
    artCacheInfo: () => call("art_cache_info"),
    artCacheClear: () => call("art_cache_clear"),
    appInfo: () => call("app_info"),
    openProjectPage: () => call("open_project_page"),
    updateCheck: () => call("update_check"),
    updateInstallOnRestart: () => call("update_install_on_restart"),
    artistInfo: (id) => call("artist_info", { id }),
    artistInfoKeys: () => call("artist_info_keys"),
    artistInfoKeySet: (source, key) => call("artist_info_key_set", { source, key }),
    artistInfoCacheClear: () => call("artist_info_cache_clear"),
    openInfoLink: (url) => call("open_info_link", { url }),
    shareInfo: (kind, id, atSeconds) => call("share_info", { kind, id, atSeconds }),
    sharePublicLink: (kind, id) => call("share_public_link", { kind, id }),
    deepLinkTake: () => call("deep_link_take"),
    discordStatus: () => call("discord_status"),
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
