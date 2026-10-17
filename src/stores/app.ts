/**
 * App-wide core state: auth, health, sync, sidebar playlists and a library
 * version counter that pages watch to refetch after `library://changed`.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import { buildArtistLookup, type ArtistLookup } from "../lib/credits";
import type { ArtistName, AuthStatus, Health, Playlist, SavedServer, SyncStatus } from "../lib/types";
import { watchMixed } from "./mixed";
import { outputLostMessage, useOutput } from "./output";
import { loadSettings, useSettings } from "./settings";
import { useToasts } from "./toasts";
import { loadPins } from "./pins";
import { loadFolders, onStoreChanged } from "./folders";

interface AppStore {
  auth: AuthStatus | null;
  /** Saved servers (Plex and those added by address); null until known. */
  servers: SavedServer[] | null;
  setServers: (s: SavedServer[]) => void;
  health: Health | null;
  sync: SyncStatus | null;
  playlists: Playlist[] | null;
  libraryVersion: number;
  /** Track ids stored on this computer; fetched while only those can play. */
  cached: Set<string>;
  /** Library artist names → ids, for the artist links in track lines. */
  artistLookup: ArtistLookup;
  setArtistNames: (names: ArtistName[]) => void;
  setAuth: (a: AuthStatus) => void;
  setHealth: (h: Health) => void;
  setSync: (s: SyncStatus) => void;
  setPlaylists: (p: Playlist[]) => void;
  bumpLibrary: () => void;
  setCached: (ids: string[]) => void;
}

export const useApp = create<AppStore>()((set, get) => ({
  auth: null,
  servers: null,
  setServers: (servers) => set({ servers }),
  health: null,
  sync: null,
  playlists: null,
  libraryVersion: 0,
  cached: new Set(),
  artistLookup: new Map(),
  setArtistNames: (names) => set({ artistLookup: buildArtistLookup(names) }),
  setAuth: (auth) => set({ auth }),
  setHealth: (health) => set({ health }),
  setSync: (sync) => set({ sync }),
  setPlaylists: (playlists) => set({ playlists }),
  bumpLibrary: () => set({ libraryVersion: get().libraryVersion + 1 }),
  setCached: (ids) => set({ cached: new Set(ids) }),
}));

/** While only cached music can play, this list says which tracks those are. */
export const CACHED_REFRESH_MS = 30_000;

/** True when a track can't play right now (dimmed in lists). */
export function isUnavailable(trackId: string, s: Pick<AppStore, "health" | "cached"> = useApp.getState()): boolean {
  return !!s.health?.cachedOnly && !s.cached.has(trackId);
}

export function connectApp(b: Backend = backend): () => void {
  let alive = true;
  const unsubs: Array<() => void> = [];
  const keep = (p: Promise<() => void>) => p.then((u) => (alive ? unsubs.push(u) : u()));
  const s = useApp.getState();

  b.authStatus().then((a) => alive && s.setAuth(a)).catch(() => {});
  const loadServers = () =>
    b.serversSaved().then(
      (list) => alive && useApp.getState().setServers(list),
      () => alive && useApp.getState().servers === null && useApp.getState().setServers([]),
    );
  loadServers();
  keep(b.listen("backends://changed", loadServers));
  keep(b.listen("auth://changed", loadServers));
  // Cached-track list: fetched when entering a cached-only state, refreshed
  // while it lasts.
  let cachedTimer: ReturnType<typeof setInterval> | null = null;
  const loadCached = () => b.cachedTracks().then((ids) => alive && useApp.getState().setCached(ids)).catch(() => {});
  const onHealth = (h: Health) => {
    const was = !!useApp.getState().health?.cachedOnly;
    useApp.getState().setHealth(h);
    if (h.cachedOnly && !was) {
      loadCached();
      if (!cachedTimer) cachedTimer = setInterval(loadCached, CACHED_REFRESH_MS);
    } else if (!h.cachedOnly && cachedTimer) {
      clearInterval(cachedTimer);
      cachedTimer = null;
    }
  };
  b.health().then((h) => alive && onHealth(h)).catch(() => {});
  b.syncStatus().then((x) => alive && s.setSync(x)).catch(() => {});
  loadSettings(b);
  void loadPins(b);
  void loadFolders(b);
  const loadPlaylists = () => b.playlists().then((p) => alive && useApp.getState().setPlaylists(p)).catch(() => {});
  loadPlaylists();
  keep(watchMixed(b));
  const loadArtistNames = () =>
    b.artistNames().then((n) => alive && useApp.getState().setArtistNames(n)).catch(() => {});
  loadArtistNames();

  keep(b.listen("auth://changed", (a) => useApp.getState().setAuth(a)));
  keep(b.listen("health://changed", onHealth));
  keep(b.listen("sync://progress", (x) => useApp.getState().setSync(x)));
  keep(
    b.listen("library://changed", () => {
      useApp.getState().bumpLibrary();
      loadPlaylists();
      loadArtistNames();
    }),
  );
  // Artist info can teach names with separators ("Tyler, The Maker").
  let namesTimer: ReturnType<typeof setTimeout> | null = null;
  keep(
    b.listen("artistinfo://changed", () => {
      if (namesTimer) clearTimeout(namesTimer);
      namesTimer = setTimeout(loadArtistNames, 2000);
    }),
  );
  keep(b.listen("toast", (t) => useToasts.getState().push(t, t.kind === "error" ? 6000 : 4000)));
  keep(b.listen("output://changed", (o) => useOutput.getState().set(o)));
  // The core moved the EQ to the profile bound to a new output device.
  keep(b.listen("settings://changed", (s) => useSettings.getState().setLocal(s)));
  // Pins and folders changed (here, or another device through the sync file).
  keep(b.listen("syncstore://changed", (c) => onStoreChanged(c, b)));
  keep(b.listen("output://lost", (l) => useToasts.getState().push({ kind: "info", message: outputLostMessage(l.device) }, 6000)));
  keep(
    b.listen("update://ready", (u) =>
      useToasts.getState().push(
        {
          kind: "info",
          message: `Platter ${u.version} is ready. Restart to install it.`,
          action: { label: "Restart", run: () => void b.updateInstallOnRestart() },
        },
        0,
      ),
    ),
  );

  return () => {
    alive = false;
    if (cachedTimer) clearInterval(cachedTimer);
    if (namesTimer) clearTimeout(namesTimer);
    unsubs.forEach((u) => u());
  };
}
