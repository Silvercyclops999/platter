/**
 * Small history-based router (no URL): a stack of routes plus an index, so
 * back/forward work like a browser. Scroll offsets per entry live in MainView.
 */
import { create } from "zustand";
import type { HomeShelfId, ReleaseSection } from "../lib/types";
import { fullWindow, useFullWindow } from "./fullWindow";

export type LibraryTab = "artists" | "albums" | "playlists";

/** The Stats page's tabs. */
export type StatsTab = "overview" | "recaps" | "compare";

export type Route =
  | { name: "home" }
  | { name: "search"; query?: string }
  /** Wide library; a tab opens it filtered to that type. */
  | { name: "library"; tab?: LibraryTab }
  /** An artist page, filtered to one era when `era` is set (Back clears it). */
  | { name: "artist"; id: string; era?: string }
  /** An artist's whole discography, optionally filtered to one release type (or `custom:<name>`). */
  | { name: "discography"; id: string; type?: ReleaseSection | `custom:${string}` }
  /** Every version of a song on one timeline (`id`: any of its versions). */
  | { name: "lineage"; id: string }
  /** One era of an artist (ARCHIVE_ERA). */
  | { name: "era"; id: string; era: string }
  | { name: "album"; id: string }
  | { name: "playlist"; id: string }
  /** Liked Songs. */
  | { name: "liked" }
  | { name: "settings" }
  /** What was played here, by day and listening session. */
  | { name: "recents" }
  /** New releases by followed artists (the top bar's bell). */
  | { name: "whatsNew" }
  /** New releases on MusicBrainz from followed and most played artists. */
  | { name: "releaseRadar" }
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
  /** Listening stats: a tab, optionally one month's range, or a friend to compare with. */
  | { name: "stats"; tab?: StatsTab; month?: string; friend?: string }
  /** Your profile: who you are, this month's listening, playlists and friends. */
  | { name: "profile" }
  /** A year's Top Songs list, or All-Time without a year. */
  | { name: "topSongs"; year?: number }
  /** Library import: the reports and review queue, one imported list (`list:<id>`), or the Missing songs (`missing`). */
  | { name: "import"; view?: string }
  /** The library doctor: things worth fixing in the library. */
  | { name: "doctor" }
  /** Sync to a device: drives and players music is copied to. */
  | { name: "devices" }
  /** The sound map: songs placed by how they sound. */
  | { name: "soundMap" }
  /** An artist that isn't in the library (MusicBrainz artist id). */
  | { name: "catalogueArtist"; mbid: string }
  /** An album that isn't in the library (MusicBrainz release group id). */
  | { name: "catalogueAlbum"; id: string }
  /** Artists like the ones played most that aren't in the library. */
  | { name: "newToYou" }
  /** Albums owned on vinyl, CD or cassette, and what a Discogs import found. */
  | { name: "shelf" }
  /** Create a Blend: pick people to combine tastes with. */
  | { name: "blendNew" };

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

/** Going to a page (or back, or forward) leaves the expanded view. */
function leaveExpanded() {
  if (useFullWindow.getState().open) void fullWindow.close();
}

export const useRouter = create<RouterState>()((set, get) => ({
  entries: [entry({ name: "home" })],
  index: 0,
  navigate: (route) => {
    if (route.name === "lyrics") return fullWindow.open("lyrics");
    leaveExpanded();
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
    leaveExpanded();
    const { index } = get();
    if (index > 0) set({ index: index - 1 });
  },
  forward: () => {
    leaveExpanded();
    const { index, entries } = get();
    if (index < entries.length - 1) set({ index: index + 1 });
  },
  reset: (route = { name: "home" }) => {
    if (route.name === "lyrics") {
      fullWindow.open("lyrics");
      route = { name: "home" };
    } else leaveExpanded();
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
