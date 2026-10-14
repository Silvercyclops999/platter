import { create } from "zustand";
import type { HeaderColors } from "../lib/color";
import { RIGHT_DEFAULT, RIGHT_MAX, RIGHT_MIN, SIDEBAR_DEFAULT, SIDEBAR_MAX, SIDEBAR_MIN, clamp } from "../shell/layout";

/** Panel widths the user chose. App-only and per device (localStorage), like the library view. */
export interface PanelWidths {
  sidebarWidth: number;
  sidebarCollapsed: boolean;
  rightWidth: number;
}

const PANELS_KEY = "platter.panels.v1"; // gitleaks:allow (localStorage key name, not a secret)

export function loadPanels(): PanelWidths {
  const d: PanelWidths = { sidebarWidth: SIDEBAR_DEFAULT, sidebarCollapsed: false, rightWidth: RIGHT_DEFAULT };
  try {
    const raw = localStorage.getItem(PANELS_KEY);
    if (!raw) return d;
    const p = JSON.parse(raw) as Partial<PanelWidths>;
    return {
      sidebarWidth: typeof p.sidebarWidth === "number" ? clamp(p.sidebarWidth, SIDEBAR_MIN, SIDEBAR_MAX) : d.sidebarWidth,
      sidebarCollapsed: typeof p.sidebarCollapsed === "boolean" ? p.sidebarCollapsed : d.sidebarCollapsed,
      rightWidth: typeof p.rightWidth === "number" ? clamp(p.rightWidth, RIGHT_MIN, RIGHT_MAX) : d.rightWidth,
    };
  } catch {
    return d;
  }
}

function savePanels(p: PanelWidths) {
  try {
    localStorage.setItem(PANELS_KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable: this session only */
  }
}

/** Now-playing panel settings. App-only and per device (localStorage). */
export interface NowPlayingPrefs {
  /** Open the now-playing panel when a song starts (when the right slot is free). */
  openOnStart: boolean;
  /** Show the lyrics card in the panel when the song has lyrics. */
  lyricsCard: boolean;
}

export const NOW_PLAYING_PREFS_DEFAULT: NowPlayingPrefs = { openOnStart: false, lyricsCard: true };
const NOW_PLAYING_KEY = "platter.nowPlaying.v1"; // gitleaks:allow (localStorage key name, not a secret)

export function loadNowPlayingPrefs(): NowPlayingPrefs {
  const d = NOW_PLAYING_PREFS_DEFAULT;
  try {
    const raw = localStorage.getItem(NOW_PLAYING_KEY);
    if (!raw) return { ...d };
    const p = JSON.parse(raw) as Partial<NowPlayingPrefs>;
    return {
      openOnStart: typeof p.openOnStart === "boolean" ? p.openOnStart : d.openOnStart,
      lyricsCard: typeof p.lyricsCard === "boolean" ? p.lyricsCard : d.lyricsCard,
    };
  } catch {
    return { ...d };
  }
}

function saveNowPlayingPrefs(p: NowPlayingPrefs) {
  try {
    localStorage.setItem(NOW_PLAYING_KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable: this session only */
  }
}

const SIDEBAR_ART_KEY = "platter.sidebar-art.v1"; // gitleaks:allow (localStorage key name, not a secret)

function loadSidebarArt(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_ART_KEY) !== "off";
  } catch {
    return true;
  }
}

export interface MenuItem {
  label: string;
  onSelect?: () => void;
  disabled?: boolean;
  danger?: boolean;
  /** Render a separator before this item. */
  separator?: boolean;
  shortcut?: string;
  /** Radio-style option (sort / view menus): shows a check when true. */
  checked?: boolean;
  /** Small group heading rendered before this item. */
  heading?: string;
  /** Checked when the menu opens; the item is left out when this returns false. */
  when?: () => boolean;
}

/** Shown in the main header once the page's own header has scrolled away. */
export interface PageBar {
  title: string;
  playing: boolean;
  onPlay: () => void;
}

export interface ContextMenuState {
  /** Pointer position (or the anchor's bottom-left). */
  x: number;
  y: number;
  /** Element the menu is aligned to (options button, keyboard-opened row). */
  anchor?: { left: number; right: number; top: number; bottom: number };
  items: MenuItem[];
  label: string;
  /** Element to return focus to on close. */
  returnFocus: HTMLElement | null;
}

interface UiState extends PanelWidths {
  /** The right slot shows the queue. At most one of queueOpen / nowPlayingOpen is true. */
  queueOpen: boolean;
  /** The right slot shows the now-playing panel. */
  nowPlayingOpen: boolean;
  /** The right slot shows the output device picker (it takes the slot from either panel). */
  outputOpen: boolean;
  /** What the slot showed before the picker opened; closing the picker returns to it. */
  outputReturn: "queue" | "nowPlaying" | null;
  toggleOutput: () => void;
  setOutputOpen: (v: boolean) => void;
  /** The now-playing view fills the main area (only while nowPlayingOpen). */
  nowPlayingExpanded: boolean;
  nowPlayingPrefs: NowPlayingPrefs;
  toggleNowPlaying: () => void;
  setNowPlayingOpen: (v: boolean) => void;
  setNowPlayingExpanded: (v: boolean) => void;
  setNowPlayingPrefs: (p: Partial<NowPlayingPrefs>) => void;
  paletteOpen: boolean;
  /** Bumped to ask the search field to take focus (Ctrl/⌘+L). */
  searchFocusTick: number;
  menu: ContextMenuState | null;
  /** Header colours for the current page (from its art), null = neutral. */
  tint: HeaderColors | null;
  /** Title and play button for the sticky header (detail pages). */
  pageBar: PageBar | null;
  setPageBar: (b: PageBar | null) => void;
  /** The sign-in screen is open over the app (after Plex rejected the sign-in). */
  signInOpen: boolean;
  setSignInOpen: (v: boolean) => void;
  setTint: (c: HeaderColors | null) => void;
  toggleQueue: () => void;
  setQueueOpen: (v: boolean) => void;
  setPaletteOpen: (v: boolean) => void;
  focusSearch: () => void;
  openMenu: (m: Omit<ContextMenuState, "returnFocus"> & { returnFocus?: HTMLElement | null }) => void;
  closeMenu: () => void;
  /** Save the user's sidebar width and/or rail state (on drag release). */
  setSidebar: (p: { width?: number; collapsed?: boolean }) => void;
  /** Save the user's right panel width (on drag release). */
  setRightWidth: (w: number) => void;
  /** Library sidebar list rows show art (Settings). Per device. */
  sidebarArt: boolean;
  setSidebarArt: (v: boolean) => void;
}

const panels = (s: UiState): PanelWidths => ({ sidebarWidth: s.sidebarWidth, sidebarCollapsed: s.sidebarCollapsed, rightWidth: s.rightWidth });

export const useUi = create<UiState>()((set, get) => ({
  ...loadPanels(),
  queueOpen: false,
  nowPlayingOpen: false,
  nowPlayingExpanded: false,
  nowPlayingPrefs: loadNowPlayingPrefs(),
  outputOpen: false,
  outputReturn: null,
  toggleOutput: () => get().setOutputOpen(!get().outputOpen),
  setOutputOpen: (v) => {
    const s = get();
    if (v === s.outputOpen) return;
    if (v) {
      const outputReturn = s.queueOpen ? "queue" : s.nowPlayingOpen ? "nowPlaying" : null;
      set({ outputOpen: true, outputReturn, queueOpen: false, nowPlayingOpen: false, nowPlayingExpanded: false });
    } else {
      set({ outputOpen: false, outputReturn: null, queueOpen: s.outputReturn === "queue", nowPlayingOpen: s.outputReturn === "nowPlaying" });
    }
  },
  toggleNowPlaying: () => get().setNowPlayingOpen(!get().nowPlayingOpen),
  setNowPlayingOpen: (v) =>
    set(v ? { nowPlayingOpen: true, queueOpen: false, outputOpen: false, outputReturn: null } : { nowPlayingOpen: false, nowPlayingExpanded: false }),
  setNowPlayingExpanded: (v) =>
    set(v ? { nowPlayingExpanded: true, nowPlayingOpen: true, queueOpen: false, outputOpen: false, outputReturn: null } : { nowPlayingExpanded: false }),
  setNowPlayingPrefs: (p) => {
    const nowPlayingPrefs = { ...get().nowPlayingPrefs, ...p };
    set({ nowPlayingPrefs });
    saveNowPlayingPrefs(nowPlayingPrefs);
  },
  paletteOpen: false,
  searchFocusTick: 0,
  menu: null,
  tint: null,
  signInOpen: false,
  pageBar: null,
  setPageBar: (pageBar) => set({ pageBar }),
  setSignInOpen: (signInOpen) => set({ signInOpen }),
  setTint: (tint) => {
    const cur = get().tint;
    if (cur?.vivid !== tint?.vivid || cur?.shade !== tint?.shade) set({ tint });
  },
  toggleQueue: () => get().setQueueOpen(!get().queueOpen),
  // The queue takes the right slot from the now-playing panel (and its expanded view).
  setQueueOpen: (v) =>
    set(v ? { queueOpen: true, nowPlayingOpen: false, nowPlayingExpanded: false, outputOpen: false, outputReturn: null } : { queueOpen: false }),
  setPaletteOpen: (v) => set({ paletteOpen: v }),
  focusSearch: () => set({ searchFocusTick: get().searchFocusTick + 1 }),
  openMenu: (m) =>
    set({
      menu: {
        ...m,
        returnFocus: m.returnFocus ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null),
      },
    }),
  closeMenu: () => {
    const m = get().menu;
    set({ menu: null });
    m?.returnFocus?.focus?.();
  },
  setSidebar: ({ width, collapsed }) => {
    const s = get();
    const sidebarWidth = width === undefined ? s.sidebarWidth : Math.round(clamp(width, SIDEBAR_MIN, SIDEBAR_MAX));
    const sidebarCollapsed = collapsed ?? s.sidebarCollapsed;
    if (sidebarWidth === s.sidebarWidth && sidebarCollapsed === s.sidebarCollapsed) return;
    set({ sidebarWidth, sidebarCollapsed });
    savePanels(panels(get()));
  },
  setRightWidth: (w) => {
    const rightWidth = Math.round(clamp(w, RIGHT_MIN, RIGHT_MAX));
    if (rightWidth === get().rightWidth) return;
    set({ rightWidth });
    savePanels(panels(get()));
  },
  sidebarArt: loadSidebarArt(),
  setSidebarArt: (sidebarArt) => {
    set({ sidebarArt });
    try {
      localStorage.setItem(SIDEBAR_ART_KEY, sidebarArt ? "on" : "off");
    } catch {
      /* storage unavailable: this session only */
    }
  },
}));

/** Whether the library sidebar's list rows show art, and its setter (Settings). */
export function useSidebarArt(): [boolean, (v: boolean) => void] {
  return [useUi((s) => s.sidebarArt), useUi((s) => s.setSidebarArt)];
}
