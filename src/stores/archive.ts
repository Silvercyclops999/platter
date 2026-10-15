/**
 * The archive's dialogs: Other versions of a song, and Move to section for
 * an album. Mounted once in the shell; this says which one is open.
 */
import { create } from "zustand";
import type { Album, Track } from "../lib/types";
import type { MenuItem } from "./ui";

interface ArchiveUi {
  /** The song whose other versions are showing. */
  versionsOf: Track | null;
  /** The album being moved to a section of the user's own. */
  moving: Album | null;
  set: (p: Partial<Pick<ArchiveUi, "versionsOf" | "moving">>) => void;
}

export const useArchiveUi = create<ArchiveUi>()((set) => ({
  versionsOf: null,
  moving: null,
  set: (p) => set(p),
}));

export const openVersions = (t: Track) => useArchiveUi.getState().set({ versionsOf: t });
export const openMoveToSection = (a: Album) => useArchiveUi.getState().set({ moving: a });

/** "Other versions (N)" for a song's menu, when it has any. */
export function otherVersionsItem(t: Track): MenuItem[] {
  const n = t.otherVersions ?? 0;
  return n > 0 ? [{ label: `Other versions (${n})`, onSelect: () => openVersions(t) }] : [];
}

/** "Move to section…" for an album's menu (an artist's own albums only). */
export function moveToSectionItem(a: Album): MenuItem {
  return { label: "Move to section…", onSelect: () => openMoveToSection(a) };
}
