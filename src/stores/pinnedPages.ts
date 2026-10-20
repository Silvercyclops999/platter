/**
 * Pages pinned to Your Library from Home's Explore row (Crates, Sound map).
 * Kept on this device only; a page that's turned off just doesn't show.
 */
import { create } from "zustand";

export type PinnablePage = "crates" | "soundMap";

export const PINNABLE: Record<PinnablePage, { title: string; sub: string }> = {
  crates: { title: "Crates", sub: "By label, producer and era" },
  soundMap: { title: "Sound map", sub: "Songs that sound alike" },
};

const KEY = "platter.pinnedPages";

function load(): PinnablePage[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is PinnablePage => x === "crates" || x === "soundMap") : [];
  } catch {
    return [];
  }
}

function save(pages: PinnablePage[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(pages));
  } catch {
    // Kept for this visit only.
  }
}

interface PinnedPagesState {
  pages: PinnablePage[];
  toggle: (p: PinnablePage) => void;
}

export const usePinnedPages = create<PinnedPagesState>()((set, get) => ({
  pages: load(),
  toggle: (p) => {
    const cur = get().pages;
    const pages = cur.includes(p) ? cur.filter((x) => x !== p) : [...cur, p];
    save(pages);
    set({ pages });
  },
}));
