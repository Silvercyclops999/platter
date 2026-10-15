/**
 * Looks: which of the look builder and the "Pick a look" step is open, and
 * applying a ready-made look. A look file dropped on the window lands here
 * too.
 */
import { create } from "zustand";
import type { ReadyLook } from "../design/looks/readymade";
import { LOOK_EXT } from "../design/themes/validate";
import { backend } from "../lib/backend";
import type { AppError, Settings, ThemeFile } from "../lib/types";
import { updateSettings, useSettings } from "./settings";
import { toast, useToasts } from "./toasts";

export type BuilderTab = "colours" | "shape" | "layout";

interface LooksState {
  /** The builder is open on this tab. */
  builder: BuilderTab | null;
  /** The "Pick a look" step is open. */
  pick: boolean;
  /** Bumped when the themes folder changes (a save, an import or a drop). */
  folderVersion: number;
}

export const useLooks = create<LooksState>()(() => ({ builder: null, pick: false, folderVersion: 0 }));

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

export const looks = {
  openBuilder(tab: BuilderTab = "colours") {
    useLooks.setState({ builder: tab, pick: false });
  },
  closeBuilder() {
    useLooks.setState({ builder: null });
  },
  openPick() {
    useLooks.setState({ pick: true });
  },
  /** Close the step; it won't open by itself again on this computer. */
  closePick() {
    useLooks.setState({ pick: false });
    if (!useSettings.getState().settings?.lookStepDone) void updateSettings({ lookStepDone: true });
  },
  folderChanged() {
    useLooks.setState((s) => ({ folderVersion: s.folderVersion + 1 }));
  },
};

/** The settings change that puts a look to use. A look without a layout keeps the current one. */
export function lookPatch(look: ThemeFile): Partial<Settings> {
  const held: ThemeFile = { ...look };
  delete held.notes;
  return held.layout ? { themeFile: held, layout: held.layout } : { themeFile: held };
}

/** Use a ready-made look. */
export function applyReadyLook(r: ReadyLook): Promise<unknown> {
  if (r.source.kind === "builtin") return updateSettings({ theme: r.source.theme, themeFile: null });
  return updateSettings(lookPatch(r.source.look));
}

/** True when the ready-made look is the one in use. */
export function isReadyLookInUse(r: ReadyLook, s: Settings | null): boolean {
  if (!s) return r.source.kind === "builtin" && r.source.theme === "dark";
  if (r.source.kind === "builtin") return !s.themeFile && s.theme === r.source.theme;
  return !!s.themeFile && s.themeFile.file === r.source.look.file && s.themeFile.name === r.source.look.name;
}

/** True for a file name a look or theme can be dropped as. */
export function isLookFileName(name: string): boolean {
  const n = name.toLowerCase();
  return n.endsWith(`.${LOOK_EXT}`) || n.endsWith(".json");
}

/**
 * Add a look or theme file's text to the themes folder (a file dropped on
 * the window). Says what happened, with a button to use it.
 */
export async function importLookText(text: string): Promise<ThemeFile | null> {
  try {
    const look = await backend.themesImportText(text);
    looks.folderChanged();
    const left = look.notes?.length ? ` ${look.notes.length === 1 ? "One part" : `${look.notes.length} parts`} Platter doesn't know were left out.` : "";
    useToasts.getState().push({ kind: "info", message: `Added ${look.name} to your looks.${left}`, action: { label: "Use it", run: () => void updateSettings(lookPatch(look)) } }, 8000);
    return look;
  } catch (e) {
    toast.error(`Couldn't add that look${errText(e) ? `: ${errText(e)}` : ""}`);
    return null;
  }
}

/**
 * First run: once settings have loaded, offer the "Pick a look" step unless
 * it was shown or skipped on this computer before.
 */
export function offerLookStep(): void {
  const check = (s: Settings | null) => {
    if (!s) return false;
    if (!s.lookStepDone) looks.openPick();
    return true;
  };
  if (check(useSettings.getState().settings)) return;
  const un = useSettings.subscribe((st) => {
    if (check(st.settings)) un();
  });
}
