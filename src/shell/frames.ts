/**
 * Frames: the shell's arrangement. A frame says which regions exist and what
 * goes in each, from the fixed component set (validate.ts holds the slot
 * table). Classic is the shell Platter has always had and keeps its own code
 * path; every other frame is drawn by the region host from its arrangement
 * here, so a custom frame and a built-in one run the same code.
 *
 * Layout moves things and the look paints them: nothing here sets a colour.
 * Pure functions, so the rules can be tested without a DOM.
 */
import { DEFAULT_REGIONS, type Frame, type FrameComponent, type Layout, type RegionName, type RegionSize, type Regions } from "../design/themes/validate";

export interface FrameInfo {
  id: Frame;
  name: string;
  /** One line for the picker and the Layout cards. */
  line: string;
  /** Built and ready to pick, or still to come. */
  ready: boolean;
}

/** Index first: it's the default. Classic and Console sit under More in the first-run step. */
export const FRAME_INFO: readonly FrameInfo[] = [
  { id: "index", name: "Index", line: "Sections down the left, the player strip across the top, the queue in a drawer.", ready: true },
  { id: "deck", name: "Deck", line: "A faceplate across the top with a two-line display, source keys under it.", ready: true },
  { id: "turntable", name: "Turntable", line: "The player is a column on the left: platter, tonearm and what's next.", ready: true },
  { id: "gatefold", name: "Gatefold", line: "Two pages: now playing on the left, browsing on the right.", ready: false },
  { id: "classic", name: "Classic", line: "Your library on the left and the player bar along the bottom.", ready: true },
  { id: "console", name: "Console", line: "A tree, a dense table and a details pane, for big libraries.", ready: false },
  { id: "custom", name: "Custom", line: "Choose what goes where yourself.", ready: true },
];

export const frameInfo = (id: Frame): FrameInfo => FRAME_INFO.find((f) => f.id === id) ?? FRAME_INFO[0];

/** Names for components, as the region editor shows them. */
export const COMPONENT_NAME: Record<FrameComponent, string> = {
  navigation: "Sections and playlists",
  search: "Search",
  transport: "Player controls",
  nowPlaying: "Now playing",
  queue: "Queue",
  lyrics: "Lyrics",
  signal: "Format and signal plates",
  credits: "Credits and sleeve notes",
  filters: "Filters",
  library: "Library pages",
};

export const REGION_LABEL: Record<RegionName, string> = {
  top: "Top",
  left: "Left",
  main: "Main",
  right: "Right",
  bottom: "Bottom",
  overlay: "Drawer",
};

/** Index's arrangement, with its strip at the top or the bottom. */
export function indexRegions(transport: Layout["transport"]): Regions {
  const strip: FrameComponent[] = ["transport", "nowPlaying", "signal"];
  return {
    top: transport === "top" ? strip : [],
    left: ["search", "navigation"],
    main: ["library"],
    right: [],
    bottom: transport === "bottom" ? strip : [],
    // The queue, lyrics and credits open as a drawer from the strip.
    overlay: ["queue", "lyrics", "credits"],
  };
}

/**
 * Deck: the faceplate (now playing, transport, format plates) and under it
 * the source keys with search, all across the top. The page is full width;
 * lyrics open as a page, the queue and credits as a drawer.
 */
export const DECK_REGIONS: Regions = {
  top: ["nowPlaying", "transport", "signal", "navigation", "search"],
  left: [],
  main: ["library", "lyrics"],
  right: [],
  bottom: [],
  overlay: ["queue", "credits"],
};

/**
 * Turntable: the player is the left column (platter, plates, transport, up
 * next); browsing on the right has its own section tabs and search. Nothing
 * at the bottom. The queue, lyrics and credits open as a drawer.
 */
export const TURNTABLE_REGIONS: Regions = {
  top: ["navigation", "search"],
  left: ["nowPlaying", "signal", "transport"],
  main: ["library"],
  right: [],
  bottom: [],
  overlay: ["queue", "lyrics", "credits"],
};

/**
 * Sketches of the frames still to come, so the picker can show their shape.
 * Never used to draw the app.
 */
const SKETCH: Partial<Record<Frame, Regions>> = {
  gatefold: { top: [], left: ["nowPlaying", "navigation"], main: ["library", "search"], right: [], bottom: ["transport", "signal"], overlay: ["queue"] },
  console: { top: [], left: ["navigation", "filters"], main: ["library", "search"], right: ["credits", "signal"], bottom: ["transport", "nowPlaying"], overlay: ["queue"] },
};

/** The arrangement a layout's frame draws (null for Classic, which has its own shell). */
export function frameRegions(l: Pick<Layout, "frame" | "transport" | "regions">): Regions | null {
  switch (l.frame) {
    case "classic":
      return null;
    case "custom":
      return l.regions;
    case "index":
      return indexRegions(l.transport);
    case "deck":
      return DECK_REGIONS;
    case "turntable":
      return TURNTABLE_REGIONS;
    default:
      // Gatefold and Console aren't built yet: Index stands in.
      return indexRegions(l.transport);
  }
}

/** The arrangement to sketch in a preview of a frame. */
export function sketchRegions(frame: Frame, l: Pick<Layout, "transport" | "regions">): Regions | null {
  if (frame === "classic") return null;
  if (frame === "custom") return l.regions;
  if (frame === "index") return indexRegions(l.transport);
  if (frame === "deck") return DECK_REGIONS;
  if (frame === "turntable") return TURNTABLE_REGIONS;
  return SKETCH[frame] ?? indexRegions("top");
}

/** The region that holds a component, or null. */
export function regionOf(r: Regions, c: FrameComponent): RegionName | null {
  for (const name of Object.keys(r) as RegionName[]) if (r[name].includes(c)) return name;
  return null;
}

/** Left region widths for each preset (px). The rail is the icon-only state. */
export const LEFT_SIZES: Record<RegionSize, number> = { small: 208, medium: 240, large: 288 };
export const RIGHT_SIZES: Record<RegionSize, number> = { small: 300, medium: 340, large: 400 };
export const FRAME_RAIL = 64;
/** The page keeps at least this much width while anything else can give way. */
export const FRAME_MAIN_MIN = 560;
/** Edge padding and the gap between columns (matches frames.css). */
const FRAME_EDGES = 3 * 8;

export type LeftMode = "full" | "rail";
export type RightMode = "column" | "drawer";

export interface FrameSolve {
  left: number;
  leftMode: LeftMode;
  /** Where an open right-hand panel goes: a column beside the page, or a drawer over it. */
  rightMode: RightMode;
  right: number;
}

/**
 * Share the window for a frame. Every frame gives way in the same order:
 * the right column becomes a drawer first, then the left region becomes an
 * icon rail. Auto states are derived, never stored.
 */
export function solveFrame(input: { windowWidth: number; regions: Regions; leftSize: RegionSize; rightSize: RegionSize }): FrameSolve {
  const { windowWidth: w, regions } = input;
  const hasLeft = regions.left.length > 0;
  const hasRight = regions.right.length > 0;
  let left = hasLeft ? LEFT_SIZES[input.leftSize] : 0;
  const right = RIGHT_SIZES[input.rightSize];
  // 1. The right column docks only when the page keeps its width beside it.
  const rightMode: RightMode = hasRight && w - FRAME_EDGES - left - right >= FRAME_MAIN_MIN ? "column" : "drawer";
  // 2. Then the left region folds to icons.
  let leftMode: LeftMode = "full";
  if (hasLeft && w - FRAME_EDGES - left < FRAME_MAIN_MIN + 80) {
    leftMode = "rail";
    left = FRAME_RAIL;
  }
  return { left, leftMode, rightMode, right };
}

/** Turntable's player column for each width preset, and its small state (px). */
export const TURNTABLE_COLUMN: Record<RegionSize, number> = { small: 288, medium: 336, large: 400 };
export const TURNTABLE_COMPACT = 216;

/**
 * Turntable's column in a window: its preset width while the browsing side
 * keeps a usable width beside it, else the small platter with transport.
 */
export function solveTurntable(windowWidth: number, size: RegionSize): { column: number; compact: boolean } {
  const full = TURNTABLE_COLUMN[size] ?? TURNTABLE_COLUMN.medium;
  if (windowWidth - FRAME_EDGES - full >= FRAME_MAIN_MIN) return { column: full, compact: false };
  return { column: TURNTABLE_COMPACT, compact: true };
}

/** The order a frame gives way in narrow windows, for the Layout settings. */
export const COLLAPSE_ORDER: readonly string[] = ["The right panel becomes a drawer", "The left side folds to icons", "Panels open as drawers"];

export { DEFAULT_REGIONS };
