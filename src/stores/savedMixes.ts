/**
 * Mixes saved to Your Library on the phone. Downloading a mix saves it, and
 * removing the download leaves it saved; the mix page's add button saves or
 * takes it out. Kept on this device only, like pinned pages.
 */
import { create } from "zustand";

export interface SavedMix {
  id: string;
  title: string;
  /** Songs when it was saved (a download's own count wins while there is one). */
  trackCount: number;
  /** When it was saved, for the library's Recents order. */
  savedAt: number;
}

const KEY = "platter.savedMixes";

function load(): SavedMix[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is SavedMix => !!x && typeof x.id === "string" && typeof x.title === "string") : [];
  } catch {
    return [];
  }
}

function store(mixes: SavedMix[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(mixes));
  } catch {
    // Kept for this visit only.
  }
}

interface SavedMixesState {
  mixes: SavedMix[];
}

export const useSavedMixes = create<SavedMixesState>()(() => ({ mixes: load() }));

export const isMixSaved = (id: string) => useSavedMixes.getState().mixes.some((m) => m.id === id);

/** Save a mix (again: its title and count are brought up to date, its place kept). */
export function saveMix(m: { id: string; title: string; trackCount: number }) {
  const cur = useSavedMixes.getState().mixes;
  const was = cur.find((x) => x.id === m.id);
  const mixes = was ? cur.map((x) => (x.id === m.id ? { ...x, title: m.title, trackCount: m.trackCount } : x)) : [{ ...m, savedAt: Date.now() }, ...cur];
  store(mixes);
  useSavedMixes.setState({ mixes });
}

export function unsaveMix(id: string) {
  const mixes = useSavedMixes.getState().mixes.filter((x) => x.id !== id);
  store(mixes);
  useSavedMixes.setState({ mixes });
}

/** For tests: forget every saved mix. */
export function resetSavedMixes() {
  store([]);
  useSavedMixes.setState({ mixes: [] });
}
