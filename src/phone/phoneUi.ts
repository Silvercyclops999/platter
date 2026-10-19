/**
 * The phone shell's own state: the tab, whether now playing and full screen
 * lyrics are open, and the bottom sheets. Sheets stack: the sleep timer
 * opens on top of the queue and closing it goes back to the queue.
 */
import { create } from "zustand";
import type { Album, Playlist, Track } from "../lib/types";
import { useRouter, type Route } from "../stores/router";

export type PhoneTab = "home" | "search" | "library" | "you";

export const TABS: { id: PhoneTab; label: string; route: Route }[] = [
  { id: "home", label: "Home", route: { name: "home" } },
  { id: "search", label: "Search", route: { name: "search" } },
  { id: "library", label: "Library", route: { name: "library" } },
  { id: "you", label: "You", route: { name: "profile" } },
];

/** A song's place in a playlist page, so its sheet can offer "Remove from this playlist". */
export interface InPlaylist {
  playlist: Playlist;
  itemId: string;
  order: string[];
}

export type Sheet =
  | { kind: "queue" }
  | { kind: "sleep" }
  | { kind: "device" }
  | { kind: "track"; track: Track; inPlaylist?: InPlaylist }
  | { kind: "album"; album: Album }
  | { kind: "addTo"; track: Track }
  | { kind: "librarySort" }
  | { kind: "create" };

interface PhoneUi {
  tab: PhoneTab;
  nowPlaying: boolean;
  lyrics: boolean;
  sheets: Sheet[];
  /** Library shows crates instead of the mixed list (its Crates pill). */
  crates: boolean;
  /** Home's now-playing card is on screen (the mini player steps aside). */
  homeCard: boolean;
}

export const usePhoneUi = create<PhoneUi>()(() => ({ tab: "home", nowPlaying: false, lyrics: false, sheets: [], crates: false, homeCard: false }));

export const phone = {
  /** A tab: its first page. Tapping the tab you're on goes back to its top. */
  tab(id: PhoneTab) {
    const t = TABS.find((x) => x.id === id)!;
    usePhoneUi.setState({ tab: id });
    useRouter.getState().reset(t.route);
  },
  openPlayer: () => usePhoneUi.setState({ nowPlaying: true }),
  closePlayer: () => usePhoneUi.setState({ nowPlaying: false, lyrics: false, sheets: [] }),
  openLyrics: () => usePhoneUi.setState({ lyrics: true }),
  closeLyrics: () => usePhoneUi.setState({ lyrics: false }),
  /** Open a sheet on top of the ones open. */
  push: (s: Sheet) => usePhoneUi.setState((u) => ({ sheets: [...u.sheets, s] })),
  /** Open a sheet by itself. */
  show: (s: Sheet) => usePhoneUi.setState({ sheets: [s] }),
  /** Close the top sheet; the one under it shows again. */
  pop: () => usePhoneUi.setState((u) => ({ sheets: u.sheets.slice(0, -1) })),
  closeSheets: () => usePhoneUi.setState({ sheets: [] }),
  /** Go to a page from inside the player or a sheet: both close first. */
  go(route: Route) {
    usePhoneUi.setState({ nowPlaying: false, lyrics: false, sheets: [] });
    useRouter.getState().navigate(route);
  },
};

/** The tab a route belongs to when it is reached some other way. */
export function tabOf(route: Route, current: PhoneTab): PhoneTab {
  if (route.name === "home" || route.name === "search" || route.name === "library") return route.name;
  if (route.name === "crates" || route.name === "crate") return "library";
  if (route.name === "profile" || route.name === "settings" || route.name === "stats" || route.name === "diary") return "you";
  return current;
}
