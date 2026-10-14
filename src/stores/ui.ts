import { create } from "zustand";

export interface MenuItem {
  label: string;
  onSelect?: () => void;
  disabled?: boolean;
  danger?: boolean;
  /** Render a separator before this item. */
  separator?: boolean;
  shortcut?: string;
}

export interface ContextMenuState {
  x: number;
  y: number;
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
  /** Header tint for the current page (CSS color), null = neutral. */
  tint: string | null;
  setTint: (c: string | null) => void;
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
  setTint: (tint) => get().tint !== tint && set({ tint }),
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
