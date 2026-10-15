/**
 * The theme in use: worked out from the Theme, Accent colour and Theme file
 * settings and the computer's light or dark mode, and kept on <html>.
 */
import { create } from "zustand";
import { DEFAULT_CHOICE, applyTheme, resolveTheme, type ResolvedTheme, type ThemeBase, type ThemeChoice } from "../design/themes";
import type { Settings } from "../lib/types";
import { useSettings } from "./settings";

const QUERY = "(prefers-color-scheme: dark)";

interface ThemeStore {
  /** The computer is in dark mode (true when it can't be told). */
  systemDark: boolean;
  resolved: ResolvedTheme;
}

export const useTheme = create<ThemeStore>()(() => ({
  systemDark: true,
  resolved: resolveTheme(DEFAULT_CHOICE, true),
}));

export function choiceFrom(s: Settings | null): ThemeChoice {
  if (!s) return DEFAULT_CHOICE;
  return { theme: s.theme ?? "dark", accent: s.accent ?? "ivory", themeFile: s.themeFile ?? null };
}

function update(doc: Document) {
  const { systemDark } = useTheme.getState();
  const resolved = resolveTheme(choiceFrom(useSettings.getState().settings), systemDark);
  useTheme.setState({ resolved });
  applyTheme(resolved, doc);
}

/**
 * Follow the settings and the computer's mode. Before settings load the
 * cached theme (applied in main.tsx) stays. Returns an unsubscribe function.
 */
export function installTheme(win: Window = window): () => void {
  const doc = win.document;
  const mq = typeof win.matchMedia === "function" ? win.matchMedia(QUERY) : null;
  useTheme.setState({ systemDark: mq ? mq.matches : true });
  let last: ThemeChoice | null = null;
  const onSettings = (s: Settings | null) => {
    if (!s) return;
    const c = choiceFrom(s);
    if (last && last.theme === c.theme && last.accent === c.accent && last.themeFile === c.themeFile) return;
    last = c;
    update(doc);
  };
  onSettings(useSettings.getState().settings);
  const unSettings = useSettings.subscribe((st) => onSettings(st.settings));
  const onChange = (e: MediaQueryListEvent) => {
    useTheme.setState({ systemDark: e.matches });
    if (useSettings.getState().settings) update(doc);
  };
  mq?.addEventListener?.("change", onChange);
  return () => {
    unSettings();
    mq?.removeEventListener?.("change", onChange);
  };
}

/** Whether the theme in use has light or dark surfaces. */
export function useThemeBase(): ThemeBase {
  return useTheme((s) => s.resolved.base);
}
