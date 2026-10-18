/**
 * The places a frame's keys and tabs go to: Deck's source keys and
 * Turntable's section tabs. Crates and the shelf show only when they're on,
 * as in the sidebar. Settings is always last, so it's always in reach.
 */
import { Archive, Boxes, History, House, Library, ListMusic, Search, Settings as SettingsIcon } from "lucide-react";
import { useMemo, type ReactNode } from "react";
import type { Route } from "../stores/router";
import { useSettings } from "../stores/settings";

export interface SourceKey {
  key: string;
  label: string;
  icon: ReactNode;
  route: Route;
  /** The routes that light this key up. */
  on: (r: Route) => boolean;
}

const libraryPage = (r: Route) => (r.name === "library" && r.tab !== "playlists") || r.name === "artist" || r.name === "album" || r.name === "discography";

export function useSourceKeys(opts: { search: boolean; size?: number }): SourceKey[] {
  const crates = useSettings((s) => s.settings?.showCrates !== false);
  const shelf = useSettings((s) => s.settings?.showPhysicalCollection !== false);
  const size = opts.size ?? 16;
  const { search } = opts;
  return useMemo(() => {
    const keys: (SourceKey | false)[] = [
      { key: "home", label: "Home", icon: <House size={size} />, route: { name: "home" }, on: (r) => r.name === "home" },
      search && { key: "search", label: "Search", icon: <Search size={size} />, route: { name: "search" }, on: (r) => r.name === "search" },
      { key: "library", label: "Library", icon: <Library size={size} />, route: { name: "library", tab: "albums" }, on: libraryPage },
      crates && { key: "crates", label: "Crates", icon: <Boxes size={size} />, route: { name: "crates" }, on: (r) => r.name === "crates" || r.name === "crate" },
      {
        key: "playlists",
        label: "Playlists",
        icon: <ListMusic size={size} />,
        route: { name: "library", tab: "playlists" },
        on: (r) => (r.name === "library" && r.tab === "playlists") || r.name === "playlist",
      },
      { key: "history", label: "History", icon: <History size={size} />, route: { name: "recents" }, on: (r) => r.name === "recents" },
      shelf && { key: "shelf", label: "Shelf", icon: <Archive size={size} />, route: { name: "shelf" }, on: (r) => r.name === "shelf" },
      { key: "settings", label: "Settings", icon: <SettingsIcon size={size} />, route: { name: "settings" }, on: (r) => r.name === "settings" },
    ];
    return keys.filter((k): k is SourceKey => !!k);
  }, [crates, shelf, search, size]);
}
