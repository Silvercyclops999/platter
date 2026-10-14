/**
 * The expanded now-playing view: whether it's open, what fills it (artwork,
 * the artist's image or lyrics), and real OS full screen (which only this
 * view turns on).
 */
import { create } from "zustand";
import { isTauri } from "../lib/backend";

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

/** What fills the expanded view. */
export type FullWindowMode = "artwork" | "artist" | "lyrics";
const MODES: readonly FullWindowMode[] = ["artwork", "artist", "lyrics"];

/** "Remember the expanded view's mode" and the mode it remembers. App-only and per device (localStorage). */
export interface FullWindowPrefs {
  remember: boolean;
  mode: FullWindowMode;
}

export const FULL_WINDOW_PREFS_DEFAULT: FullWindowPrefs = { remember: true, mode: "artwork" };
const PREFS_KEY = "platter.expandedView.v1"; // gitleaks:allow (localStorage key name, not a secret)

export function loadFullWindowPrefs(): FullWindowPrefs {
  const d = FULL_WINDOW_PREFS_DEFAULT;
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw) return { ...d };
    const p = JSON.parse(raw) as Partial<FullWindowPrefs>;
    return {
      remember: typeof p.remember === "boolean" ? p.remember : d.remember,
      mode: MODES.includes(p.mode as FullWindowMode) ? (p.mode as FullWindowMode) : d.mode,
    };
  } catch {
    return { ...d };
  }
}

function savePrefs(p: FullWindowPrefs) {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable: this session only */
  }
}

interface FullWindowState {
  open: boolean;
  /** What fills the view. */
  mode: FullWindowMode;
  /** The window is in OS full screen. */
  osFullscreen: boolean;
  prefs: FullWindowPrefs;
}

export const useFullWindow = create<FullWindowState>()(() => ({ open: false, mode: "artwork", osFullscreen: false, prefs: loadFullWindowPrefs() }));

const set = (p: Partial<FullWindowState>) => useFullWindow.setState(p);

async function setOs(on: boolean) {
  try {
    await os.set(on);
    set({ osFullscreen: await os.get() });
  } catch {
    set({ osFullscreen: false });
  }
}

/** Show a mode, remembering it as the last one used. */
function show(mode: FullWindowMode, open = true) {
  const prefs = useFullWindow.getState().prefs;
  const next = prefs.remember && prefs.mode !== mode ? { ...prefs, mode } : prefs;
  if (next !== prefs) savePrefs(next);
  set({ open, mode, prefs: next });
}

export const fullWindow = {
  /**
   * Open over the window (the player bar stays). With a mode, in that mode;
   * without one, in the mode last used when the user keeps it, else artwork.
   * Already open: switches to the given mode.
   */
  open: (mode?: FullWindowMode) => {
    const st = useFullWindow.getState();
    if (st.open) {
      if (mode && mode !== st.mode) show(mode);
      return;
    }
    show(mode ?? (st.prefs.remember ? st.prefs.mode : "artwork"));
  },
  /** Close, leaving OS full screen first if it's on. */
  close: async () => {
    if (useFullWindow.getState().osFullscreen) await setOs(false);
    set({ open: false, osFullscreen: false });
  },
  /** Switch what fills the open view. */
  setMode: (mode: FullWindowMode) => {
    if (useFullWindow.getState().mode !== mode) show(mode, useFullWindow.getState().open);
  },
  /** The lyrics button: open in lyrics mode, or leave when lyrics already show. */
  toggleLyrics: () => {
    const st = useFullWindow.getState();
    if (st.open && st.mode === "lyrics") void fullWindow.close();
    else fullWindow.open("lyrics");
  },
  /** The expand button: open in the last mode used, or close. */
  toggle: () => {
    if (useFullWindow.getState().open) void fullWindow.close();
    else fullWindow.open();
  },
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
  /** "Remember the expanded view's mode". Off forgets it: the expand button starts on the artwork. */
  setRemember: (remember: boolean) => {
    const st = useFullWindow.getState();
    const prefs: FullWindowPrefs = { remember, mode: remember ? st.mode : "artwork" };
    savePrefs(prefs);
    set({ prefs });
  },
};
