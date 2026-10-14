/**
 * Sidebar library view: filter, sort, list/compact and "recently opened"
 * times. App-only and per device (localStorage), like pins.
 */
import { create } from "zustand";

export type LibKind = "playlist" | "album" | "artist";
export type LibFilter = LibKind | "all";
export type LibSort = "recents" | "alpha";
export type LibView = "list" | "compact";

const KEY = "platter.library-view.v1"; // gitleaks:allow (localStorage key name, not a secret)
const MAX_RECENTS = 300;

interface Saved {
  filter: LibFilter;
  sort: LibSort;
  view: LibView;
  recents: Record<string, number>;
}

function load(): Saved {
  const d: Saved = { filter: "all", sort: "recents", view: "list", recents: {} };
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? { ...d, ...(JSON.parse(raw) as Partial<Saved>) } : d;
  } catch {
    return d;
  }
}

function save(s: Saved) {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* storage unavailable: this session only */
  }
}

interface LibraryViewState extends Saved {
  setFilter: (f: LibFilter) => void;
  setSort: (s: LibSort) => void;
  setView: (v: LibView) => void;
  /** Remember that an item was just opened (drives the Recents sort). */
  touch: (kind: LibKind, id: string, at?: number) => void;
}

const persist = (s: LibraryViewState) => save({ filter: s.filter, sort: s.sort, view: s.view, recents: s.recents });

export const useLibraryView = create<LibraryViewState>()((set, get) => ({
  ...load(),
  setFilter: (filter) => {
    set({ filter });
    persist(get());
  },
  setSort: (sort) => {
    set({ sort });
    persist(get());
  },
  setView: (view) => {
    set({ view });
    persist(get());
  },
  touch: (kind, id, at = Date.now()) => {
    const entries = Object.entries({ ...get().recents, [`${kind}:${id}`]: at });
    // Keep the map bounded: drop the oldest.
    const kept = entries.length > MAX_RECENTS ? entries.sort((a, b) => b[1] - a[1]).slice(0, MAX_RECENTS) : entries;
    set({ recents: Object.fromEntries(kept) });
    persist(get());
  },
}));

export interface LibEntry {
  kind: LibKind;
  id: string;
  title: string;
  subtitle: string;
  thumb: string | null;
  pinned: boolean;
}

/**
 * What the sidebar shows: pinned items first (in pin order), then the rest
 * sorted, all narrowed by the type filter and the search text.
 */
export function libraryEntries(
  items: LibEntry[],
  opts: { filter: LibFilter; sort: LibSort; query: string; recents: Record<string, number> },
): LibEntry[] {
  const q = opts.query.trim().toLowerCase();
  const keep = (e: LibEntry) =>
    (opts.filter === "all" || e.kind === opts.filter) && (!q || e.title.toLowerCase().includes(q) || e.subtitle.toLowerCase().includes(q));
  const pinned = items.filter((e) => e.pinned && keep(e));
  const rest = items
    .map((e, i) => ({ e, i }))
    .filter(({ e }) => !e.pinned && keep(e))
    .sort((a, b) => {
      if (opts.sort === "alpha") return a.e.title.localeCompare(b.e.title, undefined, { sensitivity: "base" });
      const ra = opts.recents[`${a.e.kind}:${a.e.id}`] ?? 0;
      const rb = opts.recents[`${b.e.kind}:${b.e.id}`] ?? 0;
      return rb - ra || a.i - b.i;
    })
    .map(({ e }) => e);
  return [...pinned, ...rest];
}
