/**
 * The physical collection: which albums the user owns and on what, loaded
 * once and kept current from `physical://changed` (and from the sync store
 * when another device changes it). Badges, the library's On the shelf
 * chip and the album page read it; nothing shows while "Show physical
 * collection" is off.
 */
import { useEffect, useMemo } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { DiscogsStatus, OwnedAlbum, OwnedCopy, PhysicalFormat, SetOwned } from "../lib/types";
import { useSettings } from "./settings";
import { toast } from "./toasts";
import { failText } from "../lib/errorText";

export const FORMATS: PhysicalFormat[] = ["vinyl", "cd", "cassette", "other"];

/** Labels for a format on its own ("Vinyl") and inside a sentence ("on vinyl"). */
export const FORMAT_LABEL: Record<PhysicalFormat, string> = { vinyl: "Vinyl", cd: "CD", cassette: "Cassette", other: "Other" };
const IN_SENTENCE: Record<PhysicalFormat, string> = { vinyl: "vinyl", cd: "CD", cassette: "cassette", other: "another format" };

/** "On vinyl", "On vinyl and CD", "On vinyl, CD and cassette". */
export function shelfLabel(formats: PhysicalFormat[]): string {
  const words = FORMATS.filter((f) => formats.includes(f)).map((f) => IN_SENTENCE[f]);
  if (words.length === 0) return "";
  if (words.length === 1) return `On ${words[0]}`;
  return `On ${words.slice(0, -1).join(", ")} and ${words[words.length - 1]}`;
}

/** A new, empty copy. */
export const blankCopy = (format: PhysicalFormat = "vinyl"): OwnedCopy => ({ format, pressing: "", colour: "", condition: "", notes: "" });

interface PhysicalState {
  /** Album id → what's owned; null until loaded. */
  owned: Map<string, OwnedAlbum> | null;
  discogs: DiscogsStatus | null;
  load: () => Promise<void>;
  loadDiscogs: () => Promise<void>;
}

export const usePhysical = create<PhysicalState>()((set) => ({
  owned: null,
  discogs: null,
  load: async () => {
    try {
      const all = await backend.physicalList();
      set({ owned: new Map(all.map((o) => [o.albumId, o])) });
    } catch {
      set((s) => ({ owned: s.owned ?? new Map() }));
    }
  },
  loadDiscogs: async () => {
    try {
      set({ discogs: await backend.discogsStatus() });
    } catch {
      /* keeps what it had */
    }
  },
}));

let started = false;

/** Load the marks and follow changes (once). */
export function ensurePhysical() {
  if (started) return;
  started = true;
  const reload = () => {
    void usePhysical.getState().load();
    void usePhysical.getState().loadDiscogs();
  };
  void backend.listen("physical://changed", reload);
  void backend.listen("syncstore://changed", (c) => {
    if (c.namespaces.includes("physical")) void usePhysical.getState().load();
  });
  reload();
}

/** Show physical collection is on (the default). */
export const useShowPhysical = () => useSettings((s) => s.settings?.showPhysicalCollection !== false);

/** What's owned of an album (null when nothing, or while the setting is off). */
export function useOwned(albumId: string | null | undefined): OwnedAlbum | null {
  const on = useShowPhysical();
  useEffect(() => ensurePhysical(), []);
  const o = usePhysical((s) => (albumId ? (s.owned?.get(albumId) ?? null) : null));
  return on ? o : null;
}

/** Every owned album's id (null while the setting is off or not loaded yet). */
export function useOwnedIds(): Set<string> | null {
  const on = useShowPhysical();
  useEffect(() => ensurePhysical(), []);
  const owned = usePhysical((s) => s.owned);
  return useMemo(() => (on && owned ? new Set(owned.keys()) : null), [on, owned]);
}

/** Save what's owned of an album; no copies takes the mark off. */
export async function saveOwned(albumId: string, s: SetOwned): Promise<boolean> {
  try {
    const o = await backend.physicalSet(albumId, s);
    usePhysical.setState((st) => {
      const next = new Map(st.owned ?? []);
      if (o) next.set(albumId, o);
      else next.delete(albumId);
      return { owned: next };
    });
    toast.info(o ? `${shelfLabel(o.formats)}: ${s.title}` : `${s.title} is off your shelf`);
    return true;
  } catch (e) {
    toast.error(failText(`Couldn't update your shelf for ${s.title}`, e));
    return false;
  }
}
