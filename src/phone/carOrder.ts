/**
 * CarPlay lists playlists the way the phone's Library does: pinned first,
 * then the Library's sort (Recents, A to Z, your own order...). The order is
 * worked out here, where the Library's view settings live, and handed to the
 * core whenever it changes.
 */
import { useEffect, useMemo } from "react";
import { backend } from "../lib/backend";
import { useApp } from "../stores/app";
import { effectiveSort, libraryEntries, toEntries, useLibraryView, type LibEntry, type LibSort } from "../stores/library";
import { usePins } from "../stores/pins";
import { useSettings } from "../stores/settings";
import { mergePins } from "../shell/useLibrary";
import { phoneRows } from "./libraryRows";

/** The playlist ids in the order the phone's Library lists them under Playlists. */
export function carPlaylistOrder(entries: LibEntry[], sort: LibSort, recents: Record<string, number>, customOrder: string[]): string[] {
  const shown = libraryEntries(entries, { filter: "playlist", sort, query: "", recents, customOrder });
  return phoneRows(shown, effectiveSort(sort, "playlist"))
    .filter((e) => e.kind === "playlist" && !e.mix && !e.liked)
    .map((e) => e.id);
}

/** Keeps the core's copy of the order current (once the library has loaded). */
export function useCarPlaylistOrder() {
  // A library that failed to load isn't an empty one: keep the last order.
  const items = useLibraryView((s) => (s.loadFailed ? null : s.items));
  const sort = useLibraryView((s) => s.sort);
  const recents = useLibraryView((s) => s.recents);
  const customOrder = useLibraryView((s) => s.customOrder);
  const owner = useApp((s) => s.auth?.username ?? null);
  const pinsOn = useSettings((s) => s.settings?.pinsInLibrary !== false);
  const allPins = usePins((s) => s.pins);
  const key = useMemo(() => {
    if (items === null) return null;
    const entries = mergePins(toEntries(items, owner), pinsOn ? allPins : []);
    return carPlaylistOrder(entries, sort, recents, customOrder).join("\n");
  }, [items, owner, pinsOn, allPins, sort, recents, customOrder]);
  useEffect(() => {
    if (key === null) return;
    // Settles first: Recents moves on every play.
    const t = setTimeout(() => void backend.playlistOrderSet(key ? key.split("\n") : []).catch(() => {}), 2000);
    return () => clearTimeout(t);
  }, [key]);
}
