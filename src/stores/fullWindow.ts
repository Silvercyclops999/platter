/**
 * The full-window now-playing view: whether it's open, whether its lyrics
 * side shows, and real OS full screen (which only this view turns on).
 */
import { create } from "zustand";
import { isTauri } from "../lib/backend";

/** At this window width and up, lyrics sit beside the cover; below, they take turns with it. */
export const FULL_WINDOW_WIDE = 1000;

/** The window's own full screen: Tauri's window in the app, the page's in a browser. */
export interface OsFullscreen {
  set(on: boolean): Promise<void>;
  get(): Promise<boolean>;
}

const tauriFullscreen: OsFullscreen = {
  async set(on) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setFullscreen(on);
  },
  async get() {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return getCurrentWindow().isFullscreen();
  },
};

const browserFullscreen: OsFullscreen = {
  async set(on) {
    if (on && !document.fullscreenElement) await document.documentElement.requestFullscreen?.();
    if (!on && document.fullscreenElement) await document.exitFullscreen?.();
  },
  async get() {
    return !!document.fullscreenElement;
  },
};

let os: OsFullscreen = isTauri() ? tauriFullscreen : browserFullscreen;

/** Tests swap in a fake. */
export function setOsFullscreenImpl(impl: OsFullscreen) {
  os = impl;
}

interface FullWindowState {
  open: boolean;
  /** The lyrics side shows (when the song has lyrics). */
  lyrics: boolean;
  /** The window is in OS full screen. */
  osFullscreen: boolean;
}

export const useFullWindow = create<FullWindowState>()(() => ({ open: false, lyrics: true, osFullscreen: false }));

const set = (p: Partial<FullWindowState>) => useFullWindow.setState(p);

async function setOs(on: boolean) {
  try {
    await os.set(on);
    set({ osFullscreen: await os.get() });
  } catch {
    set({ osFullscreen: false });
  }
}

export const fullWindow = {
  /** Open over the whole window. Lyrics start beside the cover on wide windows, hidden on narrow ones. */
  open: () => {
    if (useFullWindow.getState().open) return;
    set({ open: true, lyrics: typeof window === "undefined" || window.innerWidth >= FULL_WINDOW_WIDE });
  },
  /** Close, leaving OS full screen first if it's on. */
  close: async () => {
    if (useFullWindow.getState().osFullscreen) await setOs(false);
    set({ open: false, osFullscreen: false });
  },
  toggleLyrics: () => set({ lyrics: !useFullWindow.getState().lyrics }),
  toggleOsFullscreen: () => setOs(!useFullWindow.getState().osFullscreen),
  /**
   * Esc: in OS full screen it leaves full screen first (never a trap); a
   * second Esc closes the view.
   */
  escape: async () => {
    if (useFullWindow.getState().osFullscreen) await setOs(false);
    else await fullWindow.close();
  },
  /** The window left full screen some other way (OS gesture, F11). */
  syncOs: async () => {
    try {
      set({ osFullscreen: await os.get() });
    } catch {
      /* keep what we had */
    }
  },
};
