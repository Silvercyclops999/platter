/**
 * Stats outside the Stats page: the "Your stats" menu item and the dialog
 * it opens (a song's, artist's or album's stats strip).
 */
import { BarChart3 } from "lucide-react";
import { create } from "zustand";
import type { StatsItemKind } from "../lib/statsTypes";
import { useSettings } from "./settings";
import type { MenuItem } from "./ui";

export interface StatsSubject {
  kind: StatsItemKind;
  id: string;
  title: string;
}

interface StatsDialogStore {
  subject: StatsSubject | null;
  open: (s: StatsSubject) => void;
  close: () => void;
}

export const useStatsDialog = create<StatsDialogStore>()((set) => ({
  subject: null,
  open: (subject) => set({ subject }),
  close: () => set({ subject: null }),
}));

/** The stats strip (and "Your stats") is on in Settings. */
export const statsStripOn = () => useSettings.getState().settings?.statsStrip !== false;

/** "Your stats" for a song, artist or album menu, while the strip is on. */
export function yourStatsItem(kind: StatsItemKind, id: string, title: string): MenuItem {
  return { label: "Your stats", icon: BarChart3, when: statsStripOn, onSelect: () => useStatsDialog.getState().open({ kind, id, title }) };
}
