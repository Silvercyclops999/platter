/**
 * App-wide core state: auth, health, sync, sidebar playlists and a library
 * version counter that pages watch to refetch after `library://changed`.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { AuthStatus, Health, Playlist, SyncStatus } from "../lib/types";
import { useToasts } from "./toasts";

interface AppStore {
  auth: AuthStatus | null;
  health: Health | null;
  sync: SyncStatus | null;
  playlists: Playlist[] | null;
  libraryVersion: number;
  setAuth: (a: AuthStatus) => void;
  setHealth: (h: Health) => void;
  setSync: (s: SyncStatus) => void;
  setPlaylists: (p: Playlist[]) => void;
  bumpLibrary: () => void;
}

export const useApp = create<AppStore>()((set, get) => ({
  auth: null,
  health: null,
  sync: null,
  playlists: null,
  libraryVersion: 0,
  setAuth: (auth) => set({ auth }),
  setHealth: (health) => set({ health }),
  setSync: (sync) => set({ sync }),
  setPlaylists: (playlists) => set({ playlists }),
  bumpLibrary: () => set({ libraryVersion: get().libraryVersion + 1 }),
}));

export function connectApp(b: Backend = backend): () => void {
  let alive = true;
  const unsubs: Array<() => void> = [];
  const keep = (p: Promise<() => void>) => p.then((u) => (alive ? unsubs.push(u) : u()));
  const s = useApp.getState();

  b.authStatus().then((a) => alive && s.setAuth(a)).catch(() => {});
  b.health().then((h) => alive && s.setHealth(h)).catch(() => {});
  b.syncStatus().then((x) => alive && s.setSync(x)).catch(() => {});
  const loadPlaylists = () => b.playlists().then((p) => alive && useApp.getState().setPlaylists(p)).catch(() => {});
  loadPlaylists();

  keep(b.listen("auth://changed", (a) => useApp.getState().setAuth(a)));
  keep(b.listen("health://changed", (h) => useApp.getState().setHealth(h)));
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
    unsubs.forEach((u) => u());
  };
}
