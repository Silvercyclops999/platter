/**
 * The release radar as the UI sees it: the page (Home's shelf reads the same
 * list), read again on `radar://changed`, and the artists with auto-request on.
 */
import { useEffect, useState } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { AppError, AutoArtist, RadarPage } from "../lib/types";
import { loadSettings, useSettings } from "./settings";
import { toast } from "./toasts";

/** The radar page; null while it loads (or when it can't be read). Read again when it changes. */
export function useRadar(): { page: RadarPage | null; reload: () => void } {
  const on = useSettings((s) => s.settings?.releaseRadar !== false);
  const home = useSettings((s) => s.settings?.lidarrUrl);
  const [version, setVersion] = useState(0);
  const [page, setPage] = useState<RadarPage | null>(null);
  useEffect(() => {
    let un: (() => void) | undefined;
    let alive = true;
    backend.listen("radar://changed", () => setVersion((v) => v + 1)).then((u) => (alive ? (un = u) : u()));
    return () => {
      alive = false;
      un?.();
    };
  }, []);
  useEffect(() => {
    let alive = true;
    backend.releaseRadar().then(
      (p) => alive && setPage(p),
      () => alive && setPage(null),
    );
    return () => {
      alive = false;
    };
  }, [on, home, version]);
  return { page, reload: () => setVersion((v) => v + 1) };
}

interface AutoState {
  artists: AutoArtist[];
  loaded: boolean;
  load: () => Promise<void>;
  set: (id: string, name: string, on: boolean) => Promise<boolean>;
}

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

export const useAutoRequest = create<AutoState>()((set, get) => ({
  artists: [],
  loaded: false,
  load: async () => {
    try {
      set({ artists: await backend.radarAutoArtists(), loaded: true });
    } catch {
      set({ loaded: true });
    }
  },
  set: async (id, name, on) => {
    try {
      set({ artists: await backend.radarAutoSet(id, name, on) });
      // Turning it on for an artist turns on auto-request as a whole.
      if (on && useSettings.getState().settings?.autoRequest === false) await loadSettings();
      toast.info(on ? `Auto-request is on for ${name}` : `Auto-request is off for ${name}`);
      return true;
    } catch (e) {
      toast.error(errText(e) || "Couldn't change auto-request");
      void get().load();
      return false;
    }
  },
}));

/** Whether auto-request is on for an artist (loads the list once). */
export function useAutoOn(id: string): boolean {
  const on = useAutoRequest((s) => s.artists.some((a) => a.id === id));
  const loaded = useAutoRequest((s) => s.loaded);
  useEffect(() => {
    if (!loaded) void useAutoRequest.getState().load();
  }, [loaded]);
  return on;
}
