/**
 * Filter chips for the library's albums, artists and songs: the kind of
 * release, the files (lossless, hi-res), the decade, and added recently.
 * One chip of each group at a time; groups combine. Chips only show when
 * they would leave something to see.
 *
 * Remembering the last chips is a setting kept on this device
 * (localStorage), with the chips themselves.
 */
import { create } from "zustand";
import type { AudioQuality, LibraryItem, ReleaseSection, Track } from "./types";

/** The kinds of release a type chip picks. */
export type ReleaseType = "albums" | "singlesEps" | "live" | "compilations" | "archive";
export const RELEASE_TYPES: ReleaseType[] = ["albums", "singlesEps", "live", "compilations", "archive"];
export const TYPE_LABEL: Record<ReleaseType, string> = {
  albums: "Albums",
  singlesEps: "EPs and singles",
  live: "Live",
  compilations: "Compilations",
  archive: "Demos and unreleased",
};

export type FormatChip = "lossless" | "hiRes";
export const FORMAT_CHIPS: FormatChip[] = ["lossless", "hiRes"];
export const FORMAT_LABEL: Record<FormatChip, string> = { lossless: "Lossless", hiRes: "Hi-res" };

/** "Added recently" means in the last this many days. */
export const RECENT_DAYS = 30;

export interface LibChips {
  type: ReleaseType | null;
  format: FormatChip | null;
  /** First year of the decade (1990 for the 1990s). */
  decade: number | null;
  recent: boolean;
}

export const NO_CHIPS: LibChips = { type: null, format: null, decade: null, recent: false };

/** The views that have chips. The archive is albums with its type fixed, and keeps its own. */
export type ChipView = "albums" | "artists" | "songs" | "archive";
export const CHIP_VIEWS: ChipView[] = ["albums", "artists", "songs", "archive"];

/** What a chip looks at, for one album (an artist or song is judged by its albums). */
export interface ChipFacts {
  type: ReleaseType | null;
  quality: AudioQuality | null;
  year: number | null;
  /** Unix seconds it entered the library. */
  addedAt: number | null;
}

const LOSSLESS = new Set(["flac", "alac", "pcm", "wav", "aiff", "ape", "wavpack", "dsd", "dsf", "dff", "tta"]);

export function isLosslessCodec(codec: string | null | undefined): boolean {
  return !!codec && LOSSLESS.has(codec.toLowerCase());
}

/** One song's files. */
export function trackQuality(t: Pick<Track, "codec" | "bitDepth" | "sampleRate">): AudioQuality | null {
  if (!t.codec) return null;
  if (!isLosslessCodec(t.codec)) return "lossy";
  return (t.bitDepth ?? 0) > 16 || (t.sampleRate ?? 0) > 48_000 ? "hiRes" : "lossless";
}

/** The type chip a release belongs under, from its section (or its format when there's none). */
export function releaseType(section: ReleaseSection | null | undefined, format?: string | null): ReleaseType {
  switch (section) {
    case "singlesEps":
      return "singlesEps";
    case "live":
      return "live";
    case "compilations":
    case "soundtracks":
    case "remixes":
      return "compilations";
    case "demos":
    case "mixtapes":
    case "unreleased":
      return "archive";
    case "albums":
      return "albums";
    default:
      return format === "ep" || format === "single" ? "singlesEps" : "albums";
  }
}

export function albumFacts(i: Pick<LibraryItem, "section" | "format" | "quality" | "year" | "addedAt">): ChipFacts {
  return { type: releaseType(i.section, i.format), quality: i.quality ?? null, year: i.year ?? null, addedAt: i.addedAt ?? null };
}

export function decadeOf(year: number): number {
  return Math.floor(year / 10) * 10;
}

export function decadeLabel(decade: number): string {
  return `${decade}s`;
}

export function chipsOn(c: LibChips): boolean {
  return c.type !== null || c.format !== null || c.decade !== null || c.recent;
}

/** One album's facts against the chips. */
export function matchesChips(f: ChipFacts, c: LibChips, nowSec: number): boolean {
  if (c.type && f.type !== c.type) return false;
  if (c.format === "hiRes" && f.quality !== "hiRes") return false;
  if (c.format === "lossless" && f.quality !== "lossless" && f.quality !== "hiRes") return false;
  if (c.decade !== null && (f.year === null || decadeOf(f.year) !== c.decade)) return false;
  if (c.recent && (f.addedAt === null || nowSec - f.addedAt > RECENT_DAYS * 86_400)) return false;
  return true;
}

/** Something with several albums (an artist) matches when one of them does, all chips together. */
export function anyMatches(facts: ChipFacts[], c: LibChips, nowSec: number): boolean {
  return !chipsOn(c) || facts.some((f) => matchesChips(f, c, nowSec));
}

/** The chips worth offering for these facts: each one would leave something to see. */
export interface ChipOptions {
  types: ReleaseType[];
  formats: FormatChip[];
  decades: number[];
  recent: boolean;
}

export function chipOptions(facts: ChipFacts[], nowSec: number): ChipOptions {
  const types = new Set<ReleaseType>();
  const decades = new Set<number>();
  let lossless = false;
  let hiRes = false;
  let recent = false;
  for (const f of facts) {
    if (f.type) types.add(f.type);
    if (f.year) decades.add(decadeOf(f.year));
    if (f.quality === "hiRes") hiRes = true;
    if (f.quality === "lossless" || f.quality === "hiRes") lossless = true;
    if (f.addedAt !== null && nowSec - f.addedAt <= RECENT_DAYS * 86_400) recent = true;
  }
  return {
    // A type chip only helps when there's more than one kind.
    types: types.size > 1 ? RELEASE_TYPES.filter((t) => types.has(t)) : [],
    formats: FORMAT_CHIPS.filter((f) => (f === "hiRes" ? hiRes : lossless)),
    decades: decades.size > 1 ? [...decades].sort((a, b) => b - a) : [],
    recent,
  };
}

/** The chips after one is pressed: pressing the chosen one clears its group. */
export function pressFacet(c: LibChips, group: keyof LibChips, value: ReleaseType | FormatChip | number | boolean): LibChips {
  if (group === "recent") return { ...c, recent: !c.recent };
  return { ...c, [group]: c[group] === value ? null : value };
}

// ---------------------------------------------------------------- remembering

export interface ChipPrefs {
  /** Keep the last chips per view across restarts. */
  remember: boolean;
  chips: Record<ChipView, LibChips>;
}

export const CHIP_PREFS_KEY = "platter.library-chips.v1"; // gitleaks:allow (localStorage key name, not a secret)

const emptyViews = (): Record<ChipView, LibChips> => ({ albums: { ...NO_CHIPS }, artists: { ...NO_CHIPS }, songs: { ...NO_CHIPS }, archive: { ...NO_CHIPS } });

function cleanChips(raw: unknown): LibChips {
  const p = (raw && typeof raw === "object" ? raw : {}) as Partial<Record<keyof LibChips, unknown>>;
  return {
    type: RELEASE_TYPES.includes(p.type as ReleaseType) ? (p.type as ReleaseType) : null,
    format: FORMAT_CHIPS.includes(p.format as FormatChip) ? (p.format as FormatChip) : null,
    decade: typeof p.decade === "number" && Number.isInteger(p.decade) && p.decade % 10 === 0 && p.decade > 1800 && p.decade < 2200 ? p.decade : null,
    recent: p.recent === true,
  };
}

/** The kept prefs; anything unreadable falls back to remembering, with no chips. */
export function parseChipPrefs(raw: string | null): ChipPrefs {
  try {
    if (!raw) return { remember: true, chips: emptyViews() };
    const p = JSON.parse(raw) as { remember?: unknown; chips?: Record<string, unknown> };
    const remember = typeof p.remember === "boolean" ? p.remember : true;
    const chips = emptyViews();
    if (remember && p.chips && typeof p.chips === "object") for (const v of CHIP_VIEWS) chips[v] = cleanChips(p.chips[v]);
    return { remember, chips };
  } catch {
    return { remember: true, chips: emptyViews() };
  }
}

function load(): ChipPrefs {
  try {
    return parseChipPrefs(localStorage.getItem(CHIP_PREFS_KEY));
  } catch {
    return parseChipPrefs(null);
  }
}

function save(p: ChipPrefs) {
  try {
    localStorage.setItem(CHIP_PREFS_KEY, JSON.stringify(p.remember ? p : { remember: false }));
  } catch {
    /* storage unavailable: this session only */
  }
}

interface ChipState {
  remember: boolean;
  /** The chips in use, per view. */
  chips: Record<ChipView, LibChips>;
  set: (view: ChipView, chips: LibChips) => void;
  setRemember: (on: boolean) => void;
  /** Read the kept prefs again (tests, and a fresh start). */
  reload: () => void;
}

export const useLibraryChips = create<ChipState>()((set, get) => {
  const p = load();
  return {
    remember: p.remember,
    chips: p.chips,
    set: (view, chips) => {
      const next = { ...get().chips, [view]: chips };
      set({ chips: next });
      save({ remember: get().remember, chips: next });
    },
    setRemember: (on) => {
      set({ remember: on });
      save({ remember: on, chips: get().chips });
    },
    reload: () => {
      const q = load();
      set({ remember: q.remember, chips: q.chips });
    },
  };
});
