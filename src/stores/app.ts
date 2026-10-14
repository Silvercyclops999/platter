/**
 * App-wide core state: auth, health, sync, sidebar playlists and a library
 * version counter that pages watch to refetch after `library://changed`.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { AuthStatus, Health, Playlist, SyncStatus } from "../lib/types";
import { loadSettings } from "./settings";
import { useToasts } from "./toasts";

interface AppStore {
  auth: AuthStatus | null;
  health: Health | null;
  sync: SyncStatus | null;
  playlists: Playlist[] | null;
  libraryVersion: number;
  /** Track ids stored on this computer; fetched while only those can play. */
  cached: Set<string>;
  setAuth: (a: AuthStatus) => void;
  setHealth: (h: Health) => void;
  setSync: (s: SyncStatus) => void;
  setPlaylists: (p: Playlist[]) => void;
  bumpLibrary: () => void;
  setCached: (ids: string[]) => void;
}

export const useApp = create<AppStore>()((set, get) => ({
  auth: null,
  health: null,
  sync: null,
  playlists: null,
  libraryVersion: 0,
  cached: new Set(),
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
  const loadPlaylists = () => b.playlists().then((p) => alive && useApp.getState().setPlaylists(p)).catch(() => {});
  loadPlaylists();

  keep(b.listen("auth://changed", (a) => useApp.getState().setAuth(a)));
  keep(b.listen("health://changed", onHealth));
  keep(b.listen("sync://progress", (x) => useApp.getState().setSync(x)));
  keep(
    b.listen("library://changed", () => {
      useApp.getState().bumpLibrary();
      loadPlaylists();
    }),
  );
  keep(b.listen("toast", (t) => useToasts.getState().push(t, t.kind === "error" ? 6000 : 4000)));
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
    unsubs.forEach((u) => u());
  };
}
