/**
 * The Credits dialog: which song it's open for. "Credits" sits in every
 * song's menu while sleeve notes are on.
 */
import { create } from "zustand";
import type { Track } from "../lib/types";
import { useSettings } from "./settings";
import type { MenuItem } from "./ui";

interface SongCreditsState {
  track: Track | null;
}

export const useSongCreditsDialog = create<SongCreditsState>()(() => ({ track: null }));

export function openSongCredits(track: Track) {
  useSongCreditsDialog.setState({ track });
}

export function closeSongCredits() {
  useSongCreditsDialog.setState({ track: null });
}

/** "Credits" for a song's menu, while sleeve notes are on. */
export function creditsItem(track: Track): MenuItem {
  return {
    label: "Credits",
    when: () => useSettings.getState().settings?.showSleeveNotes !== false,
    onSelect: () => openSongCredits(track),
  };
}
