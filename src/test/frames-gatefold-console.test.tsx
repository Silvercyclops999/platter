/**
 * The Gatefold and Console frames: each draws its regions and keeps every
 * required part reachable (search, transport, now playing, Settings, the
 * queue), the controls keep the player bar's names and keys, Gatefold's page
 * turns (a fade with reduced motion, none when off) and folds in narrow
 * windows, and Console's columns, sort and pane widths persist.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CLASSIC_LAYOUT, regionsProblem, validateThemeText, type Layout } from "../design/themes/validate";
import { setReducedMotion } from "../design/motion";
import { backend } from "../lib/backend";
import {
  CONSOLE_DEFAULT_COLUMNS,
  PANE_LIMITS,
  clampPane,
  consoleSortOk,
  filterRows,
  nextSort,
  solveConsole,
  sortRows,
  type ConsoleRow,
} from "../lib/consoleLayout";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Settings, type Track } from "../lib/types";
import { LayoutSettings } from "../pages/looks/LayoutCards";
import { AppShell } from "../shell/AppShell";
import { consoleSourceOf, resetConsole, useConsole } from "../shell/ConsoleFrame";
import { regionOf, shellTabOrder } from "../shell/focusOrder";
import { CONSOLE_REGIONS, FRAME_INFO, GATEFOLD_REGIONS, frameRegions, solveGatefold } from "../shell/frames";
import { PAGE_FADE_MS, PAGE_TURN_MS, pageTurnFrames, sleeveLine } from "../shell/GatefoldFrame";
import { player, usePlayer } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;

async function settings(p: Partial<Settings> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...p });
  await loadSettings();
}

async function withTrack(status: "playing" | "paused" = "playing") {
  const s = await mock.playerState();
  act(() => usePlayer.setState({ state: { ...s, status } }));
}

const frame = (f: Layout["frame"], over: Partial<Layout> = {}): Layout => ({ ...structuredClone(CLASSIC_LAYOUT), frame: f, ...over });

const startWidth = window.innerWidth;
function setWidth(w: number) {
  act(() => {
    window.innerWidth = w;
    window.dispatchEvent(new Event("resize"));
  });
}

async function firstAlbumId(): Promise<string> {
  const page = await mock.albums({ sort: "title", offset: 0, limit: 2 } as never);
  return page.items[1].id;
}

beforeEach(async () => {
  useRouter.getState().reset();
  resetConsole();
  await settings();
});
afterEach(() => {
  setWidth(startWidth);
  setReducedMotion(false);
  vi.restoreAllMocks();
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, outputOpen: false, activityOpen: false, helpOpen: false });
});

const track = (id: string, over: Partial<Track> = {}): Track =>
  ({ id, title: id, albumId: "a", albumTitle: "A", artistId: "x", artistTitle: "X", trackArtist: null, durationMs: 1000, viewCount: 0, codec: "mp3", bitDepth: null, sampleRate: null, bitrate: 320, ...over }) as Track;

describe("the arrangements and rules", () => {
  it("pass the slot table, and both frames are ready to pick", () => {
    expect(regionsProblem(GATEFOLD_REGIONS)).toBeNull();
    expect(regionsProblem(CONSOLE_REGIONS)).toBeNull();
    expect(frameRegions(frame("gatefold"))).toBe(GATEFOLD_REGIONS);
    expect(frameRegions(frame("console"))).toBe(CONSOLE_REGIONS);
    expect(FRAME_INFO.filter((f) => f.id === "gatefold" || f.id === "console").every((f) => f.ready)).toBe(true);
    expect(CONSOLE_REGIONS.bottom).toContain("transport");
    expect(GATEFOLD_REGIONS.overlay).toContain("queue");
  });

  it("Gatefold folds its left page under 1000 px", () => {
    expect(solveGatefold(1280).folded).toBe(false);
    expect(solveGatefold(1000).folded).toBe(false);
    expect(solveGatefold(999).folded).toBe(true);
    expect(solveGatefold(800).spine).toBeGreaterThan(solveGatefold(1280).spine);
  });

  it("the page turn is a slide with a crease, a fade with reduced motion", () => {
    const fwd = pageTurnFrames("forward", false);
    expect(fwd.ms).toBe(PAGE_TURN_MS);
    expect(PAGE_TURN_MS).toBeGreaterThanOrEqual(300);
    expect(PAGE_TURN_MS).toBeLessThanOrEqual(400);
    expect(String(fwd.page[0].transform)).toContain("40px");
    expect(String(pageTurnFrames("back", false).page[0].transform)).toContain("-40px");
    expect(fwd.crease).not.toBeNull();
    const fade = pageTurnFrames("forward", true);
    expect(fade.ms).toBe(PAGE_FADE_MS);
    expect(fade.crease).toBeNull();
    expect(fade.page.every((k) => k.transform === undefined)).toBe(true);
  });

  it("the page turn setting travels in a look file and is checked", () => {
    const ok = validateThemeText("g.platter-look", JSON.stringify({ version: 1, name: "G", base: "dark", layout: { frame: "gatefold", pageTurn: false } }));
    expect(ok.ok && ok.theme.layout?.pageTurn).toBe(false);
    const bad = validateThemeText("g.platter-look", JSON.stringify({ version: 1, name: "G", base: "dark", layout: { frame: "gatefold", pageTurn: "yes" } }));
    expect(bad.ok).toBe(false);
    expect(CLASSIC_LAYOUT.pageTurn).toBe(true);
  });

  it("the sleeve line reads like a sleeve", () => {
    const n = { released: { value: "1997-05", sources: [] }, label: { value: "Harbour", sources: [] }, studios: [{ name: "Low Barn", role: "recorded", sources: [] }], recorded: null } as never;
    expect(sleeveLine(n)).toMatch(/^Released .*1997 on Harbour\. Recorded at Low Barn\.$/);
    expect(sleeveLine({ released: null, label: null, studios: [], recorded: null } as never)).toBeNull();
  });

  it("Console sorts up, down, then back to the list's order, with empty values last", () => {
    expect(nextSort("", "year")).toBe("year");
    expect(nextSort("year", "year")).toBe("-year");
    expect(nextSort("-year", "year")).toBe("");
    expect(nextSort("-year", "name")).toBe("name");
    expect(consoleSortOk("-plays")).toBe(true);
    expect(consoleSortOk("mood")).toBe(false);
    const rows: ConsoleRow[] = [
      { track: track("b", { durationMs: 3 }), year: 2001, index: 0 },
      { track: track("a", { durationMs: 1 }), year: null, index: 1 },
      { track: track("c", { durationMs: 2 }), year: 1999, index: 2 },
    ];
    expect(sortRows(rows, "name").map((r) => r.track.id)).toEqual(["a", "b", "c"]);
    expect(sortRows(rows, "-length").map((r) => r.track.id)).toEqual(["b", "c", "a"]);
    expect(sortRows(rows, "year").map((r) => r.track.id)).toEqual(["c", "b", "a"]);
    expect(sortRows(rows, "-year").map((r) => r.track.id)).toEqual(["b", "c", "a"]);
    expect(sortRows(rows, "").map((r) => r.index)).toEqual([0, 1, 2]);
  });

  it("Console's filters keep lossless, hi-res or never played songs", () => {
    const rows: ConsoleRow[] = [
      { track: track("mp3", { viewCount: 3 }), year: null, index: 0 },
      { track: track("flac", { codec: "flac", bitDepth: 16, sampleRate: 44100 }), year: null, index: 1 },
      { track: track("hires", { codec: "flac", bitDepth: 24, sampleRate: 96000, viewCount: 2 }), year: null, index: 2 },
    ];
    expect(filterRows(rows, ["lossless"]).map((r) => r.track.id)).toEqual(["flac", "hires"]);
    expect(filterRows(rows, ["hiRes"]).map((r) => r.track.id)).toEqual(["hires"]);
    expect(filterRows(rows, ["unplayed"]).map((r) => r.track.id)).toEqual(["flac"]);
    expect(filterRows(rows, []).length).toBe(3);
  });

  it("Console gives way: the details pane becomes a drawer, then the tree folds to icons", () => {
    expect(solveConsole(1280, 240, 300)).toMatchObject({ treeMode: "full", detailsMode: "column" });
    expect(solveConsole(1000, 240, 300)).toMatchObject({ treeMode: "full", detailsMode: "drawer" });
    expect(solveConsole(700, 240, 300)).toMatchObject({ treeMode: "rail", detailsMode: "drawer" });
    expect(clampPane("tree", 9999)).toBe(PANE_LIMITS.tree.max);
    expect(clampPane("details", 0)).toBe(PANE_LIMITS.details.min);
  });

  it("the table shows albums, playlists and Liked songs, and leaves other pages be", () => {
    expect(consoleSourceOf({ name: "album", id: "1" })).toEqual({ kind: "album", id: "1" });
    expect(consoleSourceOf({ name: "album", id: "1", tab: "sleeve" })).toBeNull();
    expect(consoleSourceOf({ name: "playlist", id: "p" })).toEqual({ kind: "playlist", id: "p" });
    expect(consoleSourceOf({ name: "liked" })).toEqual({ kind: "liked" });
    expect(consoleSourceOf({ name: "home" })).toBeNull();
  });
});

describe("the Gatefold frame", () => {
  it("has the rail, the now playing page, the spine and the page, with search, Settings and the queue in reach", async () => {
    await settings({ layout: frame("gatefold") });
    render(<AppShell />);
    const spine = await screen.findByTestId("gatefold-spine");
    await withTrack();
    expect(spine).toHaveAccessibleName("Player");
    expect(document.querySelector(".player-bar, .sidebar, .transport-strip")).toBeNull();
    const leaf = screen.getByTestId("gatefold-leaf");
    expect(leaf).toHaveAccessibleName("Now playing");
    expect(within(leaf).getByRole("button", { name: "Now playing view" })).toBeInTheDocument();
    expect(within(leaf).getByRole("slider", { name: /^Seek/ })).toBeInTheDocument();
    expect(within(leaf).getByRole("region", { name: "Sleeve notes" })).toBeInTheDocument();
    for (const name of [/^(Play|Pause)$/, /^Previous$/, /^Next$/, /^Queue$/, /^(Mute|Unmute)$/]) expect(within(spine).getAllByRole("button", { name }).length).toBeGreaterThan(0);
    expect(within(spine).getByRole("slider", { name: "Volume" })).toBeInTheDocument();
    const rail = screen.getByRole("navigation", { name: "Main" });
    for (const name of ["Home", "Search", "Settings"]) expect(within(rail).getByRole("button", { name })).toBeInTheDocument();
    expect(screen.getByRole("main")).toBeInTheDocument();
    // Leaf, spine, then the page.
    expect(leaf.compareDocumentPosition(spine) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(spine.compareDocumentPosition(screen.getByRole("main")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("switches the left page between lyrics and credits with the arrow keys", async () => {
    await settings({ layout: frame("gatefold") });
    render(<AppShell />);
    await withTrack();
    const tabs = await screen.findByRole("tablist", { name: "Lyrics or credits" });
    const lyrics = within(tabs).getByRole("tab", { name: "Lyrics" });
    expect(lyrics).toHaveAttribute("aria-selected", "true");
    lyrics.focus();
    fireEvent.keyDown(lyrics, { key: "ArrowRight" });
    expect(within(tabs).getByRole("tab", { name: "Credits" })).toHaveAttribute("aria-selected", "true");
    expect(within(tabs).getByRole("tab", { name: "Credits" })).toHaveFocus();
  });

  it("opens the queue as a drawer from the spine's handle", async () => {
    const user = userEvent.setup();
    await settings({ layout: frame("gatefold") });
    render(<AppShell />);
    const spine = await screen.findByTestId("gatefold-spine");
    await user.click(within(spine).getByRole("button", { name: "Queue" }));
    expect((await screen.findByTestId("frame-drawer")).querySelector(".queue-panel")).not.toBeNull();
  });

  it("turns the page on navigation, back the other way, and not on the same page", async () => {
    await settings({ layout: frame("gatefold") });
    render(<AppShell />);
    await screen.findByTestId("gatefold-spine");
    const page = document.querySelector<HTMLElement>(".gf-turn")!;
    act(() => nav.go({ name: "library", tab: "albums" }));
    expect(page.dataset.turn).toBe("forward");
    act(() => nav.back());
    expect(page.dataset.turn).toBe("back");
    await waitFor(() => expect(page.dataset.turn).toBeUndefined(), { timeout: 1000 });
    act(() => useRouter.getState().replace({ name: "home" }));
    expect(page.dataset.turn).toBeUndefined();
  });

  it("fades with reduced motion and stays still when the page turn is off", async () => {
    setReducedMotion(true);
    await settings({ layout: frame("gatefold") });
    const { unmount } = render(<AppShell />);
    await screen.findByTestId("gatefold-spine");
    act(() => nav.go({ name: "liked" }));
    expect(document.querySelector<HTMLElement>(".gf-turn")!.dataset.turn).toBe("fade");
    unmount();
    setReducedMotion(false);
    await settings({ layout: frame("gatefold", { pageTurn: false }) });
    render(<AppShell />);
    await screen.findByTestId("gatefold-spine");
    act(() => nav.go({ name: "recents" }));
    expect(document.querySelector<HTMLElement>(".gf-turn")!.dataset.turn).toBeUndefined();
  });

  it("folds the left page into the spine in a narrow window, keeping now playing and the queue", async () => {
    setWidth(900);
    await settings({ layout: frame("gatefold") });
    render(<AppShell />);
    const spine = await screen.findByTestId("gatefold-spine");
    await withTrack();
    expect(screen.queryByTestId("gatefold-leaf")).toBeNull();
    expect(spine).toHaveClass("is-folded");
    expect(within(spine).getByRole("button", { name: "Now playing view" })).toBeInTheDocument();
    expect(within(spine).getByRole("slider", { name: /^Seek/ })).toBeInTheDocument();
    expect(within(spine).getByRole("button", { name: "Queue" })).toBeInTheDocument();
  });

  it("keeps the play shortcut and tabs through the rail, the player, then the page", async () => {
    const toggle = vi.spyOn(player, "toggle").mockImplementation(() => Promise.resolve());
    await settings({ layout: frame("gatefold") });
    render(<AppShell />);
    await screen.findByTestId("gatefold-spine");
    await withTrack();
    fireEvent.keyDown(document.body, { key: " " });
    expect(toggle).toHaveBeenCalled();
    const order = shellTabOrder(document.querySelector(".app") as HTMLElement);
    const regions = order.map((el) => regionOf(el));
    expect([...regions].sort((a, b) => a - b)).toEqual(regions);
    const at = (sel: string) => order.findIndex((el) => el.closest(sel));
    expect(at(".frame-left")).toBeLessThan(at(".gf-leaf"));
    expect(at(".gf-leaf")).toBeLessThan(at(".gf-spine"));
    expect(at(".gf-spine")).toBeLessThan(at(".main"));
  });
});

describe("the Console frame", () => {
  // Wide enough for the details pane as a column.
  beforeEach(() => setWidth(1280));

  it("has the tree, the table, the details pane and the status line, with every required part in reach", async () => {
    const id = await firstAlbumId();
    await settings({ layout: frame("console") });
    render(<AppShell />);
    act(() => nav.go({ name: "album", id }));
    const status = await screen.findByTestId("console-status");
    expect(status).toHaveAccessibleName("Player");
    expect(document.querySelector(".player-bar, .sidebar, .transport-strip")).toBeNull();
    for (const name of [/^(Play|Pause)$/, /^Previous$/, /^Next$/, /^Queue$/, /^(Mute|Unmute)$/]) expect(within(status).getAllByRole("button", { name }).length).toBeGreaterThan(0);
    expect(within(status).getByRole("slider", { name: /^Seek/ })).toBeInTheDocument();
    const tree = screen.getByTestId("console-tree");
    expect(within(tree).getByRole("combobox", { name: "Search" })).toBeInTheDocument();
    expect(within(tree).getByRole("tree", { name: "Library" })).toBeInTheDocument();
    expect(within(tree).getByRole("button", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getByTestId("console-details")).toHaveAccessibleName("Details");
    const grid = await screen.findByRole("grid", { name: /songs$/ });
    await waitFor(() => expect(within(grid).getAllByRole("row").length).toBeGreaterThan(2));
    const heads = within(grid).getAllByRole("columnheader").map((h) => h.textContent);
    expect(heads).toEqual(["Cover", "Name", "Artist", "Album", "Year", "Format", "Length"]);
  });

  it("picks a row for the details pane, moves with the arrow keys and plays with Enter", async () => {
    const play = vi.spyOn(player, "playAlbum").mockImplementation(() => Promise.resolve());
    const id = await firstAlbumId();
    await settings({ layout: frame("console") });
    render(<AppShell />);
    act(() => nav.go({ name: "album", id }));
    const grid = await screen.findByRole("grid", { name: /songs$/ });
    const rows = await waitFor(() => {
      const r = within(grid).getAllByRole("row").slice(1);
      expect(r.length).toBeGreaterThan(2);
      return r;
    });
    fireEvent.click(rows[0]);
    expect(rows[0]).toHaveAttribute("aria-selected", "true");
    const first = useConsole.getState().selected!;
    expect(within(screen.getByTestId("console-details")).getByRole("heading", { name: first.title })).toBeInTheDocument();
    fireEvent.keyDown(rows[0], { key: "ArrowDown" });
    await waitFor(() => expect(useConsole.getState().selected?.id).not.toBe(first.id));
    const second = useConsole.getState().selected!;
    fireEvent.keyDown(grid, { key: "Enter" });
    expect(play).toHaveBeenCalledWith(id, second.id);
  });

  it("sorts by a header and keeps the sort, the columns and the pane widths on this device", async () => {
    const user = userEvent.setup();
    const id = await firstAlbumId();
    await settings({ layout: frame("console") });
    render(<AppShell />);
    act(() => nav.go({ name: "album", id }));
    const grid = await screen.findByRole("grid", { name: /songs$/ });
    await user.click(within(grid).getByRole("button", { name: "Length" }));
    await waitFor(() => expect(useSettings.getState().settings?.consoleSort).toBe("length"));
    expect(within(grid).getByRole("columnheader", { name: /Length/ })).toHaveAttribute("aria-sort", "ascending");
    expect((await mock.settingsGet()).consoleSort).toBe("length");
    await user.click(within(grid).getByRole("button", { name: "Length" }));
    await waitFor(() => expect(useSettings.getState().settings?.consoleSort).toBe("-length"));
    // Pane widths: the handle is a separator that steps with the keys.
    const handle = screen.getByRole("separator", { name: "Resize the tree" });
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    await waitFor(() => expect(useSettings.getState().settings?.consoleTree).toBe(PANE_LIMITS.tree.start + 16));
    fireEvent.keyDown(screen.getByRole("separator", { name: "Resize the details pane" }), { key: "End" });
    await waitFor(() => expect(useSettings.getState().settings?.consoleDetails).toBe(PANE_LIMITS.details.max));
    const saved = await mock.settingsGet();
    expect([saved.consoleTree, saved.consoleDetails]).toEqual([PANE_LIMITS.tree.start + 16, PANE_LIMITS.details.max]);
  });

  it("the tree opens groups with the keys and its filters narrow the table", async () => {
    const id = await firstAlbumId();
    await settings({ layout: frame("console") });
    render(<AppShell />);
    act(() => nav.go({ name: "album", id }));
    const tree = await screen.findByRole("tree", { name: "Library" });
    const filters = within(tree).getByRole("treeitem", { name: /Filters/ });
    expect(filters).toHaveAttribute("aria-expanded", "false");
    act(() => filters.focus());
    fireEvent.keyDown(filters, { key: "ArrowRight" });
    const never = await within(tree).findByRole("treeitem", { name: "Never played" });
    expect(never).toHaveAttribute("aria-checked", "false");
    fireEvent.click(never);
    expect(useConsole.getState().filters).toEqual(["unplayed"]);
    expect(within(tree).getByRole("treeitem", { name: "Never played" })).toHaveAttribute("aria-checked", "true");
  });

  it("in a narrow window the details pane is a drawer from the status line", async () => {
    const user = userEvent.setup();
    setWidth(1000);
    await settings({ layout: frame("console") });
    render(<AppShell />);
    const status = await screen.findByTestId("console-status");
    expect(screen.queryByTestId("console-details")).toBeNull();
    await user.click(within(status).getByRole("button", { name: "Details" }));
    expect(await screen.findByTestId("console-details-drawer")).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("console-details-drawer")).toBeNull());
  });

  it("at 800 px the tree folds to the icon rail, Settings and search still in reach", async () => {
    setWidth(700);
    await settings({ layout: frame("console") });
    render(<AppShell />);
    await screen.findByTestId("console-status");
    expect(screen.queryByTestId("console-tree")).toBeNull();
    const rail = screen.getByRole("navigation", { name: "Main" });
    for (const name of ["Search", "Settings"]) expect(within(rail).getByRole("button", { name })).toBeInTheDocument();
  });
});

describe("Settings > Appearance > Layout for Gatefold and Console", () => {
  it("picks Gatefold and turns the page turn off in the synced layout", async () => {
    const user = userEvent.setup();
    render(<LayoutSettings />);
    await user.click(within(screen.getByRole("radiogroup", { name: "Layouts" })).getByRole("radio", { name: "Gatefold" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.frame).toBe("gatefold"));
    await user.click(within(await screen.findByRole("radiogroup", { name: "Page turn" })).getByRole("radio", { name: "Off" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.pageTurn).toBe(false));
    expect((await mock.settingsGet()).layout.pageTurn).toBe(false);
    expect(screen.getByText(/folds into the spine/)).toBeInTheDocument();
  });

  it("picks Console, chooses its columns for this device, and Reset puts them back", async () => {
    const user = userEvent.setup();
    render(<LayoutSettings />);
    await user.click(within(screen.getByRole("radiogroup", { name: "Layouts" })).getByRole("radio", { name: "Console" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.frame).toBe("console"));
    const cols = await screen.findByRole("group", { name: "Table columns (this device)" });
    await user.click(within(cols).getByRole("button", { name: "Plays" }));
    await user.click(within(cols).getByRole("button", { name: "Year" }));
    await waitFor(() => expect(useSettings.getState().settings?.consoleColumns).toEqual(["artist", "album", "format", "length", "plays"]));
    expect((await mock.settingsGet()).consoleColumns).toEqual(["artist", "album", "format", "length", "plays"]);
    await user.click(screen.getByRole("button", { name: "Reset layout" }));
    await waitFor(() => expect(useSettings.getState().settings?.consoleColumns).toEqual(CONSOLE_DEFAULT_COLUMNS));
  });

  it("refuses columns and sorts that aren't offered", async () => {
    await expect(mock.settingsSet({ consoleColumns: ["mood"] as never })).rejects.toMatchObject({ kind: "invalid" });
    await expect(mock.settingsSet({ consoleSort: "mood" })).rejects.toMatchObject({ kind: "invalid" });
  });
});
