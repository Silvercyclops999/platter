/**
 * Put a look's style, fonts and layout on an element (the document root for
 * the app, a preview's own root in the builder). Everything is a data
 * attribute or a custom property that looks.css reads; lists that change
 * order are read by the components themselves (useLayout).
 */
import { CLASSIC_LAYOUT, DEFAULT_FONTS, DEFAULT_STYLE, LOOK, validateTheme, type Layout, type LookFonts, type LookStyle } from "../themes/validate";
import { fontInfo, loadFont } from "./fonts";

export interface LookParts {
  style: LookStyle;
  fonts: LookFonts;
  layout: Layout;
}

export const DEFAULT_PARTS: LookParts = { style: DEFAULT_STYLE, fonts: DEFAULT_FONTS, layout: CLASSIC_LAYOUT };

/** Data attributes and custom properties for a look, as plain maps. */
export function lookAttributes(p: LookParts): { data: Record<string, string>; vars: Record<string, string> } {
  const { style, fonts, layout } = p;
  const cols = layout.trackColumns;
  const allCols = LOOK.layout.trackColumns as string[];
  const classicCols = cols.length === allCols.length && cols.every((c, i) => c === allCols[i]);
  const data: Record<string, string> = {
    density: style.density,
    shadow: style.shadow,
    blur: style.blur,
    border: String(style.border),
    sidebarSide: layout.sidebarSide,
    nowPlaying: layout.nowPlaying,
    playerStyle: layout.playerStyle,
    headerStyle: layout.headerStyle,
    coverShape: layout.coverShape,
    spinRecord: layout.spinRecord ? "on" : "off",
    motion: layout.motion,
    cols: classicCols ? "classic" : "custom",
    colsHide: allCols.filter((c) => !cols.includes(c as never)).join(" "),
  };
  const vars: Record<string, string> = {
    "--look-border": `${style.border}px`,
    "--col-album": String(cols.indexOf("album") + 2),
    "--col-info": String(cols.indexOf("info") + 2),
    "--col-duration": String(cols.indexOf("duration") + 2),
    "--quick-access-cols": String(layout.quickAccess === 6 ? 3 : 4),
  };
  if (fonts.text !== "inter") vars["--font-family"] = fontInfo(fonts.text).family;
  vars["--font-headings"] = fontInfo(fonts.headings).family;
  return { data, vars };
}

const DATA_KEYS = Object.keys(lookAttributes(DEFAULT_PARTS).data);
const VAR_KEYS = ["--look-border", "--col-album", "--col-info", "--col-duration", "--quick-access-cols", "--font-family", "--font-headings"];

/** Put a look on an element (fonts load in the background). */
export function applyLook(el: HTMLElement, p: LookParts): void {
  const { data, vars } = lookAttributes(p);
  for (const k of DATA_KEYS) {
    const v = data[k];
    if (v) el.dataset[`look${k[0].toUpperCase()}${k.slice(1)}`] = v;
    else delete el.dataset[`look${k[0].toUpperCase()}${k.slice(1)}`];
  }
  for (const k of VAR_KEYS) {
    if (k in vars) el.style.setProperty(k, vars[k]);
    else el.style.removeProperty(k);
  }
  void loadFont(p.fonts.text, el.ownerDocument);
  void loadFont(p.fonts.headings, el.ownerDocument);
}

/** The same attributes as React props, for a preview root: spread `attrs`, pass `style`. */
export function lookProps(p: LookParts): { attrs: Record<string, string>; style: Record<string, string> } {
  const { data, vars } = lookAttributes(p);
  const attrs: Record<string, string> = {};
  for (const [k, v] of Object.entries(data)) if (v) attrs[`data-look-${k.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase())}`] = v;
  return { attrs, style: vars };
}

const CACHE_KEY = "platter.look";

/** Remember the look's parts on this computer, for the next start. */
export function cacheLook(p: LookParts): void {
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify(p));
  } catch {
    // Storage can be unavailable; the look still applies once settings load.
  }
}

/**
 * Apply the last look used on this computer before the first paint, so a
 * sidebar on the right doesn't jump over while settings load. Every part is
 * checked again; anything that doesn't check out is left at its default.
 */
export function applyCachedLook(el: HTMLElement = document.documentElement): void {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(CACHE_KEY);
  } catch {
    return;
  }
  if (!raw) return;
  try {
    const c = JSON.parse(raw) as Partial<LookParts>;
    const r = validateTheme("", { version: 1, name: "Cached", base: "dark", style: c.style, fonts: c.fonts, layout: c.layout });
    if (r.ok && !r.theme.notes) applyLook(el, { style: r.theme.style!, fonts: r.theme.fonts!, layout: r.theme.layout! });
  } catch {
    // A broken cache entry is ignored.
  }
}
