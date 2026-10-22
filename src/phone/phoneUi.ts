/**
 * The phone shell's own state: the tab, whether now playing and full screen
 * lyrics are open, and the bottom sheets. Sheets stack: the sleep timer
 * opens on top of the queue and closing it goes back to the queue.
 */
import { create } from "zustand";
import type { Album, Playlist, Track } from "../lib/types";
import { sameRoute, useRouter, type Route } from "../stores/router";
import type { LibFilter, LibKind } from "../stores/library";
import { stackSignals } from "./motion";

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
  /** `queueItemId`: opened from a queue row, so the sheet offers Remove from queue. */
  | { kind: "track"; track: Track; inPlaylist?: InPlaylist; queueItemId?: string }
  | { kind: "album"; album: Album }
  | { kind: "playlist"; playlist: Playlist }
  | { kind: "addTo"; track: Track }
  /** Every version of a song in the library, each playable. */
  | { kind: "versions"; track: Track }
  | { kind: "librarySort" }
  /** The library's filters: its types, then Downloaded. */
  | { kind: "libraryFilter"; kinds: LibKind[]; filter: LibFilter; downloaded: boolean }
  | { kind: "create" }
  /** What happens to the sound between the file and the output. */
  | { kind: "signal" }
  /** A followed artist: get their new albums or just show them, or unfollow. */
  | { kind: "following"; artist: { id: string; title: string; thumb: string | null } };

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
  /**
   * A tab: its first page. Tapping the tab you're on goes back to its first
   * page, or, already there, scrolls it to the top.
   */
  tab(id: PhoneTab) {
    const t = TABS.find((x) => x.id === id)!;
    const from = usePhoneUi.getState().tab;
    const r = useRouter.getState();
    if (from === id && r.entries.length === 1 && sameRoute(r.entries[0].route, t.route)) {
      stackSignals.toTop?.();
      return;
    }
    stackSignals.tabSwitch = { from, to: id };
    usePhoneUi.setState({ tab: id });
    r.reset(t.route);
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
  if (route.name === "profile" || route.name === "settings" || route.name === "stats" || route.name === "diary" || route.name === "downloads" || route.name === "help") return "you";
  return current;
}
