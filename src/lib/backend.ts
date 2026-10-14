/**
 * The only door from the UI to the Rust core (see docs/IPC.md).
 *
 * Inside the Tauri app every call is a Tauri command / event. In a plain
 * browser (Vite dev server, Playwright, Vitest) an in-memory mock with fixture
 * data stands in, so every screen is demoable and testable without a server.
 */
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { createMockBackend } from "./mock";
import type {
  Album,
  AlbumPage,
  AlbumsArgs,
  AppError,
  Artist,
  ArtistPage,
  ArtistsArgs,
  AuthStatus,
  EventMap,
  EventName,
  Health,
  HomeData,
  Page,
  PinInfo,
  PlayContextArgs,
  PlayerState,
  Playlist,
  PlaylistPage,
  RepeatMode,
  SearchArgs,
  SearchResults,
  ServerInfo,
  Settings,
  SyncStatus,
  UpdateInfo,
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
  artists(args: ArtistsArgs): Promise<Page<Artist>>;
  albums(args: AlbumsArgs): Promise<Page<Album>>;
  playlists(): Promise<Playlist[]>;
  artistPage(id: string): Promise<ArtistPage>;
  albumPage(id: string): Promise<AlbumPage>;
  playlistPage(id: string): Promise<PlaylistPage>;

  // Search
  search(args: SearchArgs): Promise<SearchResults>;
  recentSearches(): Promise<string[]>;
  recentSearchAdd(query: string): Promise<void>;
  recentSearchesClear(): Promise<void>;

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
  setRepeat(repeat: RepeatMode): Promise<void>;
  queueAdd(trackIds: string[], at: "next" | "end"): Promise<void>;
  queueRemove(itemId: string): Promise<void>;
  queueMove(itemId: string, toIndex: number): Promise<void>;
  queueClear(): Promise<void>;
  queueJump(itemId: string): Promise<void>;
  playerState(): Promise<PlayerState>;

  // Health and app states
  health(): Promise<Health>;
  /** Ids of tracks stored on this computer (playable without the server). */
  cachedTracks(): Promise<string[]>;

  // Settings
  settingsGet(): Promise<Settings>;
  settingsSet(patch: Partial<Settings>): Promise<void>;

  // Updates
  updateCheck(): Promise<UpdateInfo>;
  updateInstallOnRestart(): Promise<void>;

  /** Subscribe to a core event. Resolves to an unsubscribe function. */
  listen<E extends EventName>(event: E, cb: (payload: EventMap[E]) => void): Promise<Unlisten>;

  /** URL for a Plex image path at a display size (CSS px). */
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

/** Art is requested at 2× the display size, rounded to 8px buckets
 *  so nearby sizes share cache entries. */
export function artPixelSize(css: number): number {
  return Math.max(8, Math.ceil((css * 2) / 8) * 8);
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
    artists: (args) => call("artists", { ...args }),
    albums: (args) => call("albums", { ...args }),
    playlists: () => call("playlists"),
    artistPage: (id) => call("artist_page", { id }),
    albumPage: (id) => call("album_page", { id }),
    playlistPage: (id) => call("playlist_page", { id }),

    search: (args) => call("search", { ...args }),
    recentSearches: () => call("recent_searches"),
    recentSearchAdd: (query) => call("recent_search_add", { query }),
    recentSearchesClear: () => call("recent_searches_clear"),

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
    setRepeat: (repeat) => call("set_repeat", { repeat }),
    queueAdd: (trackIds, at) => call("queue_add", { trackIds, at }),
    queueRemove: (itemId) => call("queue_remove", { itemId }),
    queueMove: (itemId, toIndex) => call("queue_move", { itemId, toIndex }),
    queueClear: () => call("queue_clear"),
    queueJump: (itemId) => call("queue_jump", { itemId }),
    playerState: () => call("player_state"),

    health: () => call("health"),
    cachedTracks: () => call("cached_tracks"),
    settingsGet: () => call("settings_get"),
    settingsSet: (patch) => call("settings_set", { patch }),
    updateCheck: () => call("update_check"),
    updateInstallOnRestart: () => call("update_install_on_restart"),

    listen: async (event, cb) => {
      const un = await tauriListen<EventMap[typeof event]>(event, (e) => cb(e.payload));
      return un;
    },

    artUrl: (path, w, h) => {
      // convertFileSrc yields art://localhost/<enc> (macOS) or
      // http://art.localhost/<enc> (Windows WebView2).
      const base = convertFileSrc(path, "art");
      return `${base}?w=${artPixelSize(w)}&h=${artPixelSize(h)}`;
    },
  };
}

// ------------------------------------------------------------------ selection

export function isTauri(win: unknown = typeof window !== "undefined" ? window : undefined): boolean {
  return !!win && typeof win === "object" && "__TAURI_INTERNALS__" in win;
}

export function selectBackend(win?: unknown): Backend {
  return isTauri(win ?? (typeof window !== "undefined" ? window : undefined))
    ? createTauriBackend()
    : createMockBackend();
}

export const backend: Backend = selectBackend();

/** Image URL for a Plex art path at a display size (CSS px). */
export function artUrl(path: string | null | undefined, w: number, h: number = w): string | undefined {
  if (!path) return undefined;
  return backend.artUrl(path, w, h);
}
