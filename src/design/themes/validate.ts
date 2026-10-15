/**
 * Theme and look file rules, shared with the core (src-tauri/src/themes.rs
 * reads the same contract.json). Both are data only. A theme file has a name,
 * a base (dark or light) and token values. A look file adds a format version,
 * an author, a style, bundled fonts and a layout. Only known names are taken,
 * and only plain colours, pixel lengths and the listed choices: no CSS, no
 * url(), no scripts. In a look, names Platter doesn't know are left out with
 * a note; a value that isn't allowed refuses the file with the reason.
 */
import contract from "./contract.json";

export type ThemeBase = "dark" | "light";
export type TokenKind = "colour" | "length";

/** Every token a theme may set, with the kind of value it takes. */
export const THEME_TOKENS = contract.tokens as Record<string, TokenKind>;
export const THEME_TOKEN_NAMES = Object.keys(THEME_TOKENS);

export const LOOK = contract.look;
export const LOOK_VERSION = LOOK.version;
export const LOOK_EXT = "platter-look";

export type FontId = "inter" | "atkinson" | "manrope" | "fraunces";
export type Density = "compact" | "comfortable" | "spacious";
export type Shadow = "flat" | "subtle" | "soft";
export type Blur = "off" | "low" | "medium";

export interface LookStyle {
  density: Density;
  shadow: Shadow;
  blur: Blur;
  /** Border weight in pixels, 0 to LOOK.borderMax. */
  border: number;
}

export interface LookFonts {
  text: FontId;
  headings: FontId;
}

export type SidebarSection = "servers" | "filters" | "tools" | "status";
export type PlayerButton = "like" | "lyrics" | "queue" | "devices" | "volume" | "miniplayer" | "fullWindow";
export type HomeShelfKey = "madeForYou" | "recentlyPlayed" | "mixMoods" | "mixGenres" | "jumpBackIn" | "playlists" | "recentlyAdded" | "topArtists";
export type TrackColumn = "album" | "info" | "duration";

/** Layout options. Lists hold the shown items in order; one left out is hidden. */
export interface Layout {
  sidebarSide: "left" | "right";
  sidebarCollapsed: boolean;
  sidebarSections: SidebarSection[];
  nowPlaying: "right" | "left" | "off";
  playerStyle: "classic" | "compact" | "largeArt";
  playerButtons: PlayerButton[];
  homeShelves: HomeShelfKey[];
  homeGreeting: boolean;
  quickAccess: 4 | 6 | 8;
  headerStyle: "bigArt" | "banner" | "minimal";
  trackColumns: TrackColumn[];
  coverShape: "square" | "rounded" | "circle";
  spinRecord: boolean;
  motion: "full" | "reduced" | "off";
}

export const DEFAULT_STYLE: LookStyle = { density: "comfortable", shadow: "flat", blur: "off", border: 0 };
export const DEFAULT_FONTS: LookFonts = { text: "inter", headings: "inter" };

/** Classic: the layout Platter has always had, and the default. */
export const CLASSIC_LAYOUT: Layout = {
  sidebarSide: "left",
  sidebarCollapsed: false,
  sidebarSections: [...LOOK.layout.sidebarSections] as SidebarSection[],
  nowPlaying: "right",
  playerStyle: "classic",
  playerButtons: [...LOOK.layout.playerButtons] as PlayerButton[],
  homeShelves: [...LOOK.layout.homeShelves] as HomeShelfKey[],
  homeGreeting: false,
  quickAccess: 8,
  headerStyle: "bigArt",
  trackColumns: [...LOOK.layout.trackColumns] as TrackColumn[],
  coverShape: "square",
  spinRecord: false,
  motion: "full",
};

/** A validated theme or look file (also what the Theme file setting holds). */
export interface UserTheme {
  /** The file's name in the themes folder. */
  file: string;
  name: string;
  base: ThemeBase;
  tokens: Record<string, string>;
  author?: string;
  style?: LookStyle;
  fonts?: LookFonts;
  layout?: Layout;
  /** Things in the file that were left out (names Platter doesn't know). */
  notes?: string[];
}

export type ThemeCheck = { ok: true; theme: UserTheme } | { ok: false; reason: string };

export const THEME_FILE_MAX_BYTES = 64 * 1024;
export const THEME_NAME_MAX = 60;
const VALUE_MAX = 64;
const FIELDS = ["name", "base", "tokens", "author"];
const LOOK_FIELDS = ["version", "name", "author", "base", "tokens", "style", "fonts", "layout"];

const NUM = String.raw`\d{1,3}(?:\.\d+)?`;
const HEX = /^#(?:[0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
const RGB = new RegExp(
  String.raw`^rgba?\(\s*${NUM}%?\s*[,\s]\s*${NUM}%?\s*[,\s]\s*${NUM}%?\s*(?:[,/]\s*${NUM}%?\s*)?\)$`,
  "i",
);
const HSL = new RegExp(
  String.raw`^hsla?\(\s*${NUM}(?:deg)?\s*[,\s]\s*${NUM}%\s*[,\s]\s*${NUM}%\s*(?:[,/]\s*${NUM}%?\s*)?\)$`,
  "i",
);
const LENGTH = /^(?:0|\d{1,3}(?:\.\d+)?px)$/;

export function isColourValue(v: string): boolean {
  return HEX.test(v) || RGB.test(v) || HSL.test(v);
}

export function isLengthValue(v: string): boolean {
  return LENGTH.test(v);
}

/** Why a token value isn't allowed, or null when it's fine. */
export function tokenValueProblem(name: string, value: unknown): string | null {
  const kind = THEME_TOKENS[name];
  if (typeof value !== "string") return `The value of "${name}" isn't text.`;
  const v = value.trim();
  if (/url\s*\(/i.test(v)) return `"${name}" uses url(). Theme files can't load anything.`;
  if (/[;{}<>\\]|\/\*|var\s*\(|expression\s*\(|@import|javascript:/i.test(v))
    return `"${name}" has CSS in it. Values are plain colours or pixel sizes.`;
  if (v.length > VALUE_MAX) return `The value of "${name}" is too long.`;
  if (kind === "colour" && !isColourValue(v)) return `"${name}" needs a colour like #1A1A1A or rgb(26 26 26).`;
  if (kind === "length" && !isLengthValue(v)) return `"${name}" needs a size in pixels like 8px.`;
  return null;
}

/** Strip an optional leading "--" from a token name. */
const bare = (k: string) => k.replace(/^--/, "");
const isObject = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const plainText = (v: string) => [...v].length <= THEME_NAME_MAX && !/[\u0000-\u001f\u007f<>]/.test(v);
/** A key from a file, made safe and short enough to show in a note. */
const shownKey = (k: string) => [...k.replace(/[\u0000-\u001f\u007f]/g, "")].slice(0, 40).join("");

class Refused extends Error {}

type Header = { name: string; author?: string; base: ThemeBase };

function header(o: Record<string, unknown>): Header {
  if (typeof o.name !== "string" || !o.name.trim()) throw new Refused("It needs a name.");
  const name = o.name.trim();
  if (!plainText(name)) throw new Refused(`The name can be up to ${THEME_NAME_MAX} plain characters.`);
  let author: string | undefined;
  if (o.author !== undefined && o.author !== null) {
    if (typeof o.author !== "string") throw new Refused("The author isn't text.");
    if (!plainText(o.author.trim())) throw new Refused(`The author can be up to ${THEME_NAME_MAX} plain characters.`);
    author = o.author.trim() || undefined;
  }
  if (o.base !== "dark" && o.base !== "light") throw new Refused('The base must be "dark" or "light".');
  return { name, author, base: o.base };
}

function pick<T extends string>(field: string, v: unknown, allowed: readonly string[]): T {
  if (typeof v === "string" && allowed.includes(v)) return v as T;
  throw new Refused(`"${field}" must be one of: ${allowed.join(", ")}.`);
}

function flag(field: string, v: unknown): boolean {
  if (typeof v !== "boolean") throw new Refused(`"${field}" must be true or false.`);
  return v;
}

function idList<T extends string>(field: string, v: unknown, allowed: readonly string[]): T[] {
  if (!Array.isArray(v)) throw new Refused(`"${field}" must be a list.`);
  const out: T[] = [];
  for (const item of v) {
    const id = pick<T>(field, item, allowed);
    if (out.includes(id)) throw new Refused(`"${field}" lists "${id}" twice.`);
    out.push(id);
  }
  return out;
}

function object(field: string, v: unknown): Record<string, unknown> {
  if (!isObject(v)) throw new Refused(`"${field}" must be an object.`);
  return v;
}

function parseStyle(v: unknown, notes: string[]): LookStyle {
  const s: LookStyle = { ...DEFAULT_STYLE };
  for (const [k, val] of Object.entries(object("style", v))) {
    if (k === "density") s.density = pick("style.density", val, LOOK.density);
    else if (k === "shadow") s.shadow = pick("style.shadow", val, LOOK.shadow);
    else if (k === "blur") s.blur = pick("style.blur", val, LOOK.blur);
    else if (k === "border") {
      if (typeof val !== "number" || !Number.isInteger(val) || val < 0 || val > LOOK.borderMax)
        throw new Refused(`"style.border" must be a whole number from 0 to ${LOOK.borderMax}.`);
      s.border = val;
    } else notes.push(`Left out style.${shownKey(k)}: Platter doesn't know it.`);
  }
  return s;
}

function parseFonts(v: unknown, notes: string[]): LookFonts {
  const f: LookFonts = { ...DEFAULT_FONTS };
  for (const [k, val] of Object.entries(object("fonts", v))) {
    if (k === "text") f.text = pick("fonts.text", val, LOOK.fonts);
    else if (k === "headings") f.headings = pick("fonts.headings", val, LOOK.fonts);
    else notes.push(`Left out fonts.${shownKey(k)}: Platter doesn't know it.`);
  }
  return f;
}

function parseLayout(v: unknown, notes: string[]): Layout {
  const c = LOOK.layout;
  const l: Layout = structuredClone(CLASSIC_LAYOUT);
  for (const [k, val] of Object.entries(object("layout", v))) {
    const f = `layout.${shownKey(k)}`;
    switch (k) {
      case "sidebarSide":
        l.sidebarSide = pick(f, val, c.sidebarSide);
        break;
      case "sidebarCollapsed":
        l.sidebarCollapsed = flag(f, val);
        break;
      case "sidebarSections":
        l.sidebarSections = idList(f, val, c.sidebarSections);
        break;
      case "nowPlaying":
        l.nowPlaying = pick(f, val, c.nowPlaying);
        break;
      case "playerStyle":
        l.playerStyle = pick(f, val, c.playerStyle);
        break;
      case "playerButtons":
        l.playerButtons = idList(f, val, c.playerButtons);
        break;
      case "homeShelves":
        l.homeShelves = idList(f, val, c.homeShelves);
        break;
      case "homeGreeting":
        l.homeGreeting = flag(f, val);
        break;
      case "quickAccess":
        if (typeof val !== "number" || !c.quickAccess.includes(val)) throw new Refused(`"${f}" must be 4, 6 or 8.`);
        l.quickAccess = val as Layout["quickAccess"];
        break;
      case "headerStyle":
        l.headerStyle = pick(f, val, c.headerStyle);
        break;
      case "trackColumns":
        l.trackColumns = idList(f, val, c.trackColumns);
        break;
      case "coverShape":
        l.coverShape = pick(f, val, c.coverShape);
        break;
      case "spinRecord":
        l.spinRecord = flag(f, val);
        break;
      case "motion":
        l.motion = pick(f, val, c.motion);
        break;
      default:
        notes.push(`Left out layout.${shownKey(k)}: Platter doesn't know it.`);
    }
  }
  return l;
}

/** Why a layout can't be used, or null when it can. Unknown names count too. */
export function layoutProblem(l: unknown): string | null {
  try {
    const notes: string[] = [];
    parseLayout(l, notes);
    return notes[0] ?? null;
  } catch (e) {
    if (e instanceof Refused) return e.message;
    throw e;
  }
}

/** A stored layout, or Classic when it's missing or doesn't check out. */
export function safeLayout(l: unknown): Layout {
  if (!isObject(l)) return structuredClone(CLASSIC_LAYOUT);
  try {
    return parseLayout(l, []);
  } catch {
    return structuredClone(CLASSIC_LAYOUT);
  }
}

function validateLook(file: string, o: Record<string, unknown>): UserTheme {
  const v = o.version;
  if (typeof v === "number" && Number.isInteger(v) && v > LOOK_VERSION)
    throw new Refused(`This look was made for a newer Platter (format ${v}). Update Platter to use it.`);
  if (typeof v !== "number" || !Number.isInteger(v) || v < 1) throw new Refused(`The version must be a whole number from 1 to ${LOOK_VERSION}.`);
  const notes: string[] = [];
  for (const k of Object.keys(o)) if (!LOOK_FIELDS.includes(k)) notes.push(`Left out "${shownKey(k)}": Platter doesn't know it.`);
  const { name, author, base } = header(o);
  const tokens: Record<string, string> = {};
  if (o.tokens !== undefined) {
    if (!isObject(o.tokens)) throw new Refused('"tokens" must be an object.');
    const entries = Object.entries(o.tokens);
    if (entries.length > THEME_TOKEN_NAMES.length * 2) throw new Refused("It has far more tokens than Platter knows.");
    for (const [k, val] of entries) {
      const n = bare(k);
      if (!(n in THEME_TOKENS)) {
        notes.push(`Left out the token "${shownKey(k)}": Platter doesn't know it.`);
        continue;
      }
      const problem = tokenValueProblem(n, val);
      if (problem) throw new Refused(problem);
      tokens[n] = (val as string).trim();
    }
  }
  const theme: UserTheme = { file, name, base, tokens };
  if (author) theme.author = author;
  if (o.style !== undefined) theme.style = parseStyle(o.style, notes);
  if (o.fonts !== undefined) theme.fonts = parseFonts(o.fonts, notes);
  if (o.layout !== undefined) theme.layout = parseLayout(o.layout, notes);
  if (notes.length) theme.notes = notes;
  return theme;
}

/** Check a theme or look file's parsed JSON. A file with a version is a look. */
export function validateTheme(file: string, data: unknown): ThemeCheck {
  if (!isObject(data)) return { ok: false, reason: "It isn't a theme: expected an object with name, base and tokens." };
  try {
    if ("version" in data) return { ok: true, theme: validateLook(file, data) };
    for (const k of Object.keys(data))
      if (!FIELDS.includes(k)) return { ok: false, reason: `Unknown field "${k}". Theme files have name, base and tokens.` };
    const { name, author, base } = header(data);
    if (!isObject(data.tokens)) return { ok: false, reason: "It needs a tokens object." };
    const tokens: Record<string, string> = {};
    const entries = Object.entries(data.tokens);
    if (entries.length > THEME_TOKEN_NAMES.length) return { ok: false, reason: "It has more tokens than Platter knows." };
    for (const [k, v] of entries) {
      const n = bare(k);
      if (!(n in THEME_TOKENS)) return { ok: false, reason: `Unknown token "${k}".` };
      const problem = tokenValueProblem(n, v);
      if (problem) return { ok: false, reason: problem };
      tokens[n] = (v as string).trim();
    }
    const theme: UserTheme = { file, name, base, tokens };
    if (author) theme.author = author;
    return { ok: true, theme };
  } catch (e) {
    if (e instanceof Refused) return { ok: false, reason: e.message };
    throw e;
  }
}

/** Check a theme or look file's text. */
export function validateThemeText(file: string, text: string): ThemeCheck {
  if (new TextEncoder().encode(text).length > THEME_FILE_MAX_BYTES) return { ok: false, reason: "The file is too big for a theme (64 KB at most)." };
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    return { ok: false, reason: "It isn't valid JSON." };
  }
  return validateTheme(file, data);
}

/** True for a look (rather than an older theme file). */
export function isLook(t: UserTheme): boolean {
  return t.file.toLowerCase().endsWith(LOOK_EXT) || !!(t.author || t.style || t.fonts || t.layout);
}

/**
 * The text of a look file: only the look itself (name, author, base, tokens,
 * style, fonts, layout), so a shared look carries nothing else.
 */
export function lookText(t: UserTheme): string {
  const o: Record<string, unknown> = { version: LOOK_VERSION, name: t.name };
  if (t.author) o.author = t.author;
  o.base = t.base;
  o.tokens = Object.fromEntries(Object.entries(t.tokens).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
  if (t.style) o.style = { density: t.style.density, shadow: t.style.shadow, blur: t.style.blur, border: t.style.border };
  if (t.fonts) o.fonts = { text: t.fonts.text, headings: t.fonts.headings };
  if (t.layout) o.layout = { ...t.layout };
  return JSON.stringify(o, null, 2);
}

/** The core's check on a look held in settings: nothing may be left out. */
export function checkHeldTheme(t: UserTheme): string | null {
  if (!isLook(t)) {
    const r = validateTheme(t.file, { name: t.name, base: t.base, tokens: t.tokens });
    return r.ok ? null : r.reason;
  }
  const r = validateThemeText(t.file, lookText(t));
  if (!r.ok) return r.reason;
  return r.theme.notes?.[0] ?? null;
}
