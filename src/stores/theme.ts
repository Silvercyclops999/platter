/**
 * The theme in use: worked out from the Theme, Accent colour and Theme file
 * settings and the computer's light or dark mode, and kept on <html>.
 */
import { create } from "zustand";
import { DEFAULT_CHOICE, applyTheme, resolveTheme, safeLayout, type Layout, type ResolvedTheme, type ThemeBase, type ThemeChoice } from "../design/themes";
import { DEFAULT_PARTS, STUDIO_FONTS, applyLook, cacheLook, type LookParts } from "../design/looks/apply";
import { isStudioLook } from "../design/themes/builtins";
import { loadReadoutFace } from "../design/looks/fonts";
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
  return { theme: s.theme ?? "dark", accent: s.accent ?? "ivory", themeFile: s.themeFile ?? null, followSystem: s.followSystem ?? false };
}

/** The look's style, fonts and layout from the settings. */
export function lookPartsFrom(s: Settings | null): LookParts {
  if (!s) return DEFAULT_PARTS;
  const f = s.themeFile;
  // Platter's own looks set page titles in their display face.
  const own = !f && isStudioLook(s.theme) ? STUDIO_FONTS : DEFAULT_PARTS.fonts;
  const layout = safeLayout(s.layout);
  // Record-player motion "both" also spins the side panel's record; "off" stops it.
  const motion = s.recordMotion ?? "off";
  const spinRecord = motion === "off" ? false : layout.spinRecord || motion === "both";
  return { style: f?.style ?? DEFAULT_PARTS.style, fonts: f?.fonts ?? own, layout: spinRecord === layout.spinRecord ? layout : { ...layout, spinRecord } };
}

/**
 * The readout face and cover light, as data attributes on <html> that the
 * Studio looks' CSS reads (Classic and the other themes never use them).
 * The readout face loads the first time a Studio look shows it.
 */
export function applyIdentityOptions(root: HTMLElement, s: Settings | null): void {
  const readout = s?.readoutFace !== false;
  root.dataset.readout = readout ? "on" : "off";
  root.dataset.coverLight = s?.coverLight ?? "on";
  if (readout && root.dataset.identity) void loadReadoutFace(root.ownerDocument);
}

function update(doc: Document) {
  const { systemDark } = useTheme.getState();
  const settings = useSettings.getState().settings;
  const resolved = resolveTheme(choiceFrom(settings), systemDark);
  useTheme.setState({ resolved });
  applyTheme(resolved, doc);
  applyIdentityOptions(doc.documentElement, settings);
  const parts = lookPartsFrom(settings);
  applyLook(doc.documentElement, parts);
  cacheLook(parts);
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
  let lastLayout: unknown = null;
  let lastMotion: unknown = null;
  let lastOptions = "";
  const onSettings = (s: Settings | null) => {
    if (!s) return;
    const c = choiceFrom(s);
    const options = `${s.readoutFace}|${s.coverLight}`;
    if (
      last &&
      last.theme === c.theme &&
      last.accent === c.accent &&
      last.themeFile === c.themeFile &&
      last.followSystem === c.followSystem &&
      lastLayout === s.layout &&
      lastMotion === s.recordMotion &&
      lastOptions === options
    )
      return;
    last = c;
    lastLayout = s.layout;
    lastMotion = s.recordMotion;
    lastOptions = options;
    update(doc);
  };
  onSettings(useSettings.getState().settings);
  const unSettings = useSettings.subscribe((st) => onSettings(st.settings));
  const onChange = (e: MediaQueryListEvent) => {
    useTheme.setState({ systemDark: e.matches });
    if (useSettings.getState().settings) update(doc);
  };
  mq?.addEventListener?.("change", onChange);
  // A phone's web view can answer "light" while the app is still starting,
  // and miss the change while it sleeps in the background: ask again once
  // it has settled and whenever it comes back to the front.
  const recheck = () => {
    if (!mq) return;
    const dark = mq.matches;
    if (dark === useTheme.getState().systemDark) return;
    useTheme.setState({ systemDark: dark });
    if (useSettings.getState().settings) update(doc);
  };
  const onVisible = () => {
    if (doc.visibilityState === "visible") recheck();
  };
  const settle = win.setTimeout(recheck, 600);
  doc.addEventListener("visibilitychange", onVisible);
  win.addEventListener("focus", recheck);
  win.addEventListener("pageshow", recheck);
  return () => {
    unSettings();
    mq?.removeEventListener?.("change", onChange);
    win.clearTimeout(settle);
    doc.removeEventListener("visibilitychange", onVisible);
    win.removeEventListener("focus", recheck);
    win.removeEventListener("pageshow", recheck);
  };
}

/** Whether the theme in use has light or dark surfaces. */
export function useThemeBase(): ThemeBase {
  return useTheme((s) => s.resolved.base);
}

/** The layout in use (Classic until settings load). */
export function useLayout(): Layout {
  return useSettings((s) => s.settings?.layout ?? DEFAULT_PARTS.layout);
}

/**
 * The Reduce motion setting, unless the layout's Motion option asks for
 * less (reduced or off). Under Match system the computer's own
 * reduced-motion setting is always respected.
 */
export function motionPreference(s: { settings: Settings | null }): Settings["reduceMotion"] {
  const motion = s.settings?.layout?.motion;
  if (motion === "reduced" || motion === "off") return "on";
  return s.settings?.reduceMotion ?? "system";
}
