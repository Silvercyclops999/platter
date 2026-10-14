import { create } from "zustand";
import type { HeaderColors } from "../lib/color";

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

interface UiState {
  queueOpen: boolean;
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
}

export const useUi = create<UiState>()((set, get) => ({
  queueOpen: false,
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
  toggleQueue: () => set({ queueOpen: !get().queueOpen }),
  setQueueOpen: (v) => set({ queueOpen: v }),
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
}));
