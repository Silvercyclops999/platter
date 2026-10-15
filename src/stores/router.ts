/**
 * Small history-based router (no URL): a stack of routes plus an index, so
 * back/forward work like a browser. Scroll offsets per entry live in MainView.
 */
import { create } from "zustand";
import type { HomeShelfId, ReleaseSection } from "../lib/types";
import { fullWindow } from "./fullWindow";

export type LibraryTab = "artists" | "albums" | "playlists";

export type Route =
  | { name: "home" }
  | { name: "search"; query?: string }
  /** Wide library; a tab opens it filtered to that type. */
  | { name: "library"; tab?: LibraryTab }
  | { name: "artist"; id: string }
  /** An artist's whole discography, optionally filtered to one release type. */
  | { name: "discography"; id: string; type?: ReleaseSection }
  | { name: "album"; id: string }
  | { name: "playlist"; id: string }
  /** Liked Songs. */
  | { name: "liked" }
  | { name: "settings" }
  /** What was played here, by day and listening session. */
  | { name: "recents" }
  /** New releases by followed artists (the top bar's bell). */
  | { name: "whatsNew" }
  /** Lyrics for the playing song: never a page, it opens the expanded view in lyrics mode. */
  | { name: "lyrics" }
  /** A Home shelf's "Show all" grid. */
  | { name: "homeShelf"; shelf: HomeShelfId }
  /** An artist page shelf's "Show all" grid. */
  | { name: "artistShelf"; id: string; shelf: "appearsOn" | "fansAlsoLike" }
  /** A made-for-you mix (Daily Mix, daylist, a genre mix...). */
  | { name: "mix"; id: string }
  /** Song or artist radio: `track:<id>` or `artist:<id>`. */
  | { name: "radio"; seed: string }
  /** Bookmarked songs and pages. */
  | { name: "bookmarks" }
  /** Library import: the reports and review queue, one imported list (`list:<id>`), or the Missing songs (`missing`). */
  | { name: "import"; view?: string };

export interface HistoryEntry {
  key: number;
  route: Route;
}

interface RouterState {
  entries: HistoryEntry[];
  index: number;
  navigate: (route: Route) => void;
  /** Update the current entry in place (e.g. search query, library tab). */
  replace: (route: Route) => void;
  back: () => void;
  forward: () => void;
  reset: (route?: Route) => void;
  /** Forget every history entry for a page that's gone (Back and Forward skip it). */
  drop: (route: Route) => void;
}

let seq = 1;
const entry = (route: Route): HistoryEntry => ({ key: seq++, route });

export function sameRoute(a: Route, b: Route): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export const useRouter = create<RouterState>()((set, get) => ({
  entries: [entry({ name: "home" })],
  index: 0,
  navigate: (route) => {
    if (route.name === "lyrics") return fullWindow.open("lyrics");
    const { entries, index } = get();
    if (sameRoute(entries[index].route, route)) return;
    const next = entries.slice(0, index + 1);
    next.push(entry(route));
    // Keep history bounded.
    const trimmed = next.length > 100 ? next.slice(next.length - 100) : next;
    set({ entries: trimmed, index: trimmed.length - 1 });
  },
  replace: (route) => {
    if (route.name === "lyrics") return fullWindow.open("lyrics");
    const { entries, index } = get();
    const next = [...entries];
    next[index] = { ...next[index], route };
    set({ entries: next });
  },
  back: () => {
    const { index } = get();
    if (index > 0) set({ index: index - 1 });
  },
  forward: () => {
    const { index, entries } = get();
    if (index < entries.length - 1) set({ index: index + 1 });
  },
  reset: (route = { name: "home" }) => {
    if (route.name === "lyrics") {
      fullWindow.open("lyrics");
      route = { name: "home" };
    }
    set({ entries: [entry(route)], index: 0 });
  },
  drop: (route) => {
    const { entries, index } = get();
    const kept = entries.filter((e) => !sameRoute(e.route, route));
    if (kept.length === entries.length) return;
    // The entry that was showing, or the nearest one before it that stays.
    const before = entries.slice(0, index + 1).filter((e) => !sameRoute(e.route, route)).length;
    const next = kept.length ? kept : [entry({ name: "home" })];
    set({ entries: next, index: Math.max(0, Math.min(before - 1, next.length - 1)) });
  },
}));

export const useCurrentEntry = () => useRouter((s) => s.entries[s.index]);
export const useCanGoBack = () => useRouter((s) => s.index > 0);
export const useCanGoForward = () => useRouter((s) => s.index < s.entries.length - 1);

/** Imperative helpers for non-React code (shortcuts, context menus). */
export const nav = {
  go: (route: Route) => useRouter.getState().navigate(route),
  back: () => useRouter.getState().back(),
  forward: () => useRouter.getState().forward(),
};
