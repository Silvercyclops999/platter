/**
 * Looks: which of the look builder and the "Pick a look" step is open, and
 * applying a ready-made look. A look file dropped on the window lands here
 * too.
 */
import { create } from "zustand";
import type { ReadyLook } from "../design/looks/readymade";
import { BUILTIN_THEMES, isStudioLook } from "../design/themes/builtins";
import { LOOK_EXT } from "../design/themes/validate";
import { backend } from "../lib/backend";
import type { Settings, ThemeFile, ThemeSetting } from "../lib/types";
import { useTheme } from "./theme";
import { updateSettings, useSettings } from "./settings";
import { toast, useToasts } from "./toasts";
import { failText } from "../lib/errorText";

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

/**
 * The settings change for picking a theme. Picking one the computer's mode
 * would swap away (a light theme while it's dark) turns Follow system off,
 * so the theme picked is the one shown.
 */
export function themePatch(theme: ThemeSetting, s: Settings | null, systemDark: boolean): Partial<Settings> {
  if (theme === "system" || !s?.followSystem) return { theme };
  const dark = BUILTIN_THEMES[theme].base === "dark";
  return dark === systemDark ? { theme } : { theme, followSystem: false };
}

/** Use a ready-made look. */
export function applyReadyLook(r: ReadyLook): Promise<unknown> {
  if (r.source.kind === "builtin")
    return updateSettings({ ...themePatch(r.source.theme, useSettings.getState().settings, useTheme.getState().systemDark), themeFile: null });
  return updateSettings(lookPatch(r.source.look));
}

/**
 * Try Platter's new look: Studio with its brass accent, following the
 * computer's light or dark mode, with the record player and tonearm seek bar.
 */
export const NEW_LOOK_PATCH: Partial<Settings> = {
  theme: "studio",
  accent: "brass",
  themeFile: null,
  followSystem: true,
  recordMotion: "full",
  tonearmSeek: true,
  newLookOffered: true,
};

/**
 * The one-time offer of the new look is due: settings from before Studio
 * that aren't on one of its looks and haven't seen the offer.
 */
export function newLookOfferDue(s: Settings | null): boolean {
  if (!s || s.newLookOffered) return false;
  return !(!s.themeFile && isStudioLook(s.theme));
}

/** True when the ready-made look is the one in use. */
export function isReadyLookInUse(r: ReadyLook, s: Settings | null): boolean {
  if (!s) return r.source.kind === "builtin" && r.source.theme === "studio";
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
    toast.error(failText(`Couldn't add that look`, e));
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
