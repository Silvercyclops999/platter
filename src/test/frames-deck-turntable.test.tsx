/**
 * The Deck and Turntable frames: each draws its regions and keeps every
 * required part reachable (search, transport, now playing, Settings, the
 * queue), the player's controls keep the player bar's names and keys, the
 * volume knob is a real slider, Deck shows lyrics as a page, Turntable
 * shrinks in narrow windows, their settings persist, and reduced motion
 * turns the spin and the arm into fades.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import deckCss from "../shell/deck.css?raw";
import turntableFrameCss from "../shell/turntable-frame.css?raw";
import turntableCss from "../shell/turntable.css?raw";
import { CLASSIC_LAYOUT, regionsProblem, validateThemeText, type Layout } from "../design/themes/validate";
import { setReducedMotion } from "../design/motion";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type QueueView, type Settings } from "../lib/types";
import { LayoutSettings } from "../pages/looks/LayoutCards";
import { AppShell } from "../shell/AppShell";
import { knobKey } from "../shell/DeckFrame";
import { DECK_REGIONS, TURNTABLE_COLUMN, TURNTABLE_COMPACT, TURNTABLE_REGIONS, frameRegions, solveTurntable } from "../shell/frames";
import { regionOf, shellTabOrder } from "../shell/focusOrder";
import { upNextItems } from "../shell/TurntableFrame";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { player, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;

async function settings(p: Partial<Settings> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...p });
  await loadSettings();
}

/** Something on the platter (the mock's restored song), playing or paused. */
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

beforeEach(async () => {
  useRouter.getState().reset();
  await settings();
});
afterEach(async () => {
  setWidth(startWidth);
  setReducedMotion(false);
  vi.restoreAllMocks();
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, outputOpen: false, activityOpen: false, helpOpen: false });
  if (useFullWindow.getState().open) await act(() => fullWindow.close());
});

describe("the arrangements", () => {
  it("pass the slot table and put the parts where the frames say", () => {
    expect(regionsProblem(DECK_REGIONS)).toBeNull();
    expect(regionsProblem(TURNTABLE_REGIONS)).toBeNull();
    expect(frameRegions(frame("deck"))).toBe(DECK_REGIONS);
    expect(frameRegions(frame("turntable"))).toBe(TURNTABLE_REGIONS);
    // Deck: everything across the top, lyrics as a page, the queue in a drawer.
    expect(DECK_REGIONS.top).toEqual(expect.arrayContaining(["transport", "nowPlaying", "navigation", "search"]));
    expect(DECK_REGIONS.main).toContain("lyrics");
    expect(DECK_REGIONS.overlay).toContain("queue");
    expect(DECK_REGIONS.bottom).toEqual([]);
    // Turntable: the player down the left, nothing at the bottom.
    expect(TURNTABLE_REGIONS.left).toEqual(expect.arrayContaining(["transport", "nowPlaying", "signal"]));
    expect(TURNTABLE_REGIONS.bottom).toEqual([]);
  });

  it("Turntable's column keeps its preset while the page has room, else shrinks to the small platter", () => {
    expect(solveTurntable(1280, "medium")).toEqual({ column: TURNTABLE_COLUMN.medium, compact: false });
    expect(solveTurntable(1000, "medium")).toEqual({ column: TURNTABLE_COLUMN.medium, compact: false });
    expect(solveTurntable(1000, "large")).toEqual({ column: TURNTABLE_COLUMN.large, compact: false });
    expect(solveTurntable(800, "medium")).toEqual({ column: TURNTABLE_COMPACT, compact: true });
    expect(solveTurntable(1280, "small").column).toBeLessThan(solveTurntable(1280, "large").column);
  });

  it("Deck's source key setting travels in a look file and is checked", () => {
    const ok = validateThemeText("d.platter-look", JSON.stringify({ version: 1, name: "D", base: "dark", layout: { frame: "deck", deckLabels: false } }));
    expect(ok.ok && ok.theme.layout?.deckLabels).toBe(false);
    const bad = validateThemeText("d.platter-look", JSON.stringify({ version: 1, name: "D", base: "dark", layout: { frame: "deck", deckLabels: "yes" } }));
    expect(bad.ok).toBe(false);
    expect(CLASSIC_LAYOUT.deckLabels).toBe(true);
  });

  it("up next is your queue first, then the rest, three at most", () => {
    const item = (id: string) => ({ itemId: id, source: "context" as const, track: { id, title: id } as never });
    const q = { userQueue: [item("u1")], nextUp: [item("n1"), item("n2"), item("n3")], history: [] } as QueueView;
    expect(upNextItems(q).map((i) => i.itemId)).toEqual(["u1", "n1", "n2"]);
    expect(upNextItems(null)).toEqual([]);
  });
});

describe("the Deck frame", () => {
  it("has the faceplate, the source rail with search and Settings, the page, and no player bar or sidebar", async () => {
    await settings({ layout: frame("deck") });
    render(<AppShell />);
    const face = await screen.findByTestId("deck-faceplate");
    expect(face).toHaveAccessibleName("Player");
    expect(document.querySelector(".player-bar, .sidebar, .frame-left, .transport-strip")).toBeNull();
    const rail = screen.getByRole("navigation", { name: "Main" });
    for (const name of ["Home", "Search", "Library", "Playlists", "History", "Settings"]) expect(within(rail).getByRole("button", { name })).toBeInTheDocument();
    expect(within(rail).getByRole("combobox", { name: "Search" })).toBeInTheDocument();
    // Faceplate, then rail, then the page.
    const main = screen.getByRole("main");
    expect(face.compareDocumentPosition(rail) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(rail.compareDocumentPosition(main) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await userEvent.setup().click(within(rail).getByRole("button", { name: "History" }));
    const { entries, index } = useRouter.getState();
    expect(entries[index].route).toEqual({ name: "recents" });
    expect(within(rail).getByRole("button", { name: "History" })).toHaveAttribute("aria-current", "page");
  });

  it("keeps the player bar's controls and names, with the volume as a knob that is a slider", async () => {
    await settings({ layout: frame("deck") });
    render(<AppShell />);
    const face = within(await screen.findByTestId("deck-faceplate"));
    for (const name of [/^(Play|Pause)$/, /^Previous$/, /^Next$/, /^Queue$/, /^Lyrics|lyrics/, /^Connect to a device$/, /^(Mute|Unmute)$/]) expect(face.getAllByRole("button", { name }).length).toBeGreaterThan(0);
    expect(face.getByRole("slider", { name: /^Seek/ })).toBeInTheDocument();
    const knob = face.getByRole("slider", { name: "Volume" });
    expect(knob).toHaveAttribute("aria-valuemin", "0");
    expect(knob).toHaveAttribute("aria-valuemax", "100");
    expect(knob.getAttribute("aria-valuetext")).toMatch(/^\d+%$/);
    expect(knob).toHaveAttribute("tabindex", "0");
  });

  it("the knob steps with arrows, Page keys, Home and End, and turns with the wheel", async () => {
    expect(knobKey("ArrowUp", 0.5)).toBe(0.6);
    expect(knobKey("ArrowRight", 0.95)).toBe(1);
    expect(knobKey("PageDown", 0.5)).toBe(0.4);
    expect(knobKey("ArrowLeft", 0.05)).toBe(0);
    expect(knobKey("Home", 0.5)).toBe(0);
    expect(knobKey("End", 0.5)).toBe(1);
    expect(knobKey("a", 0.5)).toBeNull();
    const input = vi.spyOn(player, "volumeInput").mockImplementation(() => Promise.resolve());
    const commit = vi.spyOn(player, "volumeCommit").mockImplementation(() => Promise.resolve());
    const nudge = vi.spyOn(player, "nudgeVolume").mockImplementation(() => Promise.resolve());
    await settings({ layout: frame("deck") });
    render(<AppShell />);
    await withTrack();
    const knob = within(await screen.findByTestId("deck-faceplate")).getByRole("slider", { name: "Volume" });
    const now = Number(knob.getAttribute("aria-valuenow")) / 100;
    fireEvent.keyDown(knob, { key: "ArrowUp" });
    expect(input).toHaveBeenLastCalledWith(Math.min(1, Math.round((now + 0.1) * 100) / 100));
    expect(commit).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(knob, { key: "Home" });
    expect(commit).toHaveBeenLastCalledWith(0);
    fireEvent.wheel(knob, { deltaY: -100, deltaMode: 0 });
    expect(nudge).toHaveBeenCalledWith(0.1);
  });

  it("the play shortcut works with the faceplate in place of the bar", async () => {
    const toggle = vi.spyOn(player, "toggle").mockImplementation(() => Promise.resolve());
    await settings({ layout: frame("deck") });
    render(<AppShell />);
    await screen.findByTestId("deck-faceplate");
    fireEvent.keyDown(document.body, { key: " " });
    expect(toggle).toHaveBeenCalled();
  });

  it("opens the queue as a drawer and lyrics as a page under the faceplate", async () => {
    const user = userEvent.setup();
    await settings({ layout: frame("deck") });
    render(<AppShell />);
    const face = within(await screen.findByTestId("deck-faceplate"));
    await user.click(face.getByRole("button", { name: "Queue" }));
    expect((await screen.findByTestId("frame-drawer")).querySelector(".queue-panel")).not.toBeNull();
    act(() => useUi.setState({ queueOpen: false }));
    act(() => fullWindow.open("lyrics"));
    const page = await screen.findByRole("main", { name: "Lyrics" });
    expect(page.closest(".frame-page")).not.toBeNull();
    // The expanded view stays shut; the faceplate stays in reach.
    expect(screen.queryByRole("dialog", { name: "Expanded now playing" })).toBeNull();
    expect(screen.getByTestId("deck-faceplate")).toBeInTheDocument();
    await user.click(within(page).getByRole("button", { name: "Close lyrics" }));
    await waitFor(() => expect(screen.queryByRole("main", { name: "Lyrics" })).toBeNull());
  });

  it("keeps the source keys' names when their labels are off", async () => {
    await settings({ layout: frame("deck", { deckLabels: false }) });
    render(<AppShell />);
    const rail = await screen.findByRole("navigation", { name: "Main" });
    expect(rail).toHaveClass("is-bare");
    expect(rail.querySelector(".deck-key__label")).toBeNull();
    for (const name of ["Home", "Library", "Settings"]) expect(within(rail).getByRole("button", { name })).toBeInTheDocument();
  });

  it("tabs through the window row, the faceplate, the rail, then the page", async () => {
    await settings({ layout: frame("deck") });
    render(<AppShell />);
    await screen.findByTestId("deck-faceplate");
    const order = shellTabOrder(document.querySelector(".app") as HTMLElement);
    const regions = order.map((el) => regionOf(el));
    expect([...regions].sort((a, b) => a - b)).toEqual(regions);
    const firstFace = order.findIndex((el) => el.closest(".deck-face"));
    const firstRail = order.findIndex((el) => el.closest(".deck-rail"));
    const firstPage = order.findIndex((el) => el.closest(".main"));
    expect(firstFace).toBeGreaterThan(-1);
    expect(firstFace).toBeLessThan(firstRail);
    expect(firstRail).toBeLessThan(firstPage);
  });

  it("folds the faceplate and the rail with container queries in narrow windows", () => {
    expect(deckCss).toMatch(/@container deck-face \(max-width: 1180px\)/);
    // The rail goes to icons before its keys run out of room (with Archive, at about 1040 px).
    expect(deckCss).toMatch(/@container deck-rail \(max-width: 1040px\)/);
    // Only the display line uses a mono face.
    expect(deckCss.match(/font-family:[^;]*monospace/g)).toHaveLength(1);
    expect(deckCss.slice(deckCss.indexOf(".deck-display__line {"), deckCss.indexOf(".deck-display__line--title"))).toMatch(/monospace/);
  });
});

describe("the Turntable frame", () => {
  it("has the player column, tabs and search on the browsing side, and no bar at the bottom", async () => {
    await settings({ layout: frame("turntable") });
    render(<AppShell />);
    const column = await screen.findByTestId("turntable-column");
    expect(column).toHaveAccessibleName("Player");
    expect(document.querySelector(".player-bar, .sidebar, .frame-edge--bottom, .transport-strip")).toBeNull();
    expect(column.querySelector(".turntable")).not.toBeNull();
    for (const name of [/^(Play|Pause)$/, /^Previous$/, /^Next$/, /^Queue$/, /^(Mute|Unmute)$/]) expect(within(column).getByRole("button", { name })).toBeInTheDocument();
    expect(within(column).getByRole("slider", { name: "Volume" })).toBeInTheDocument();
    expect(within(column).getByRole("slider", { name: /^Seek/ })).toBeInTheDocument();
    const tabs = screen.getByRole("navigation", { name: "Main" });
    for (const name of ["Home", "Library", "Playlists", "History", "Settings"]) expect(within(tabs).getByRole("button", { name })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Search" })).toBeInTheDocument();
    expect(screen.getByRole("main")).toBeInTheDocument();
    // The column holds the medium width by default.
    expect((document.querySelector(".app") as HTMLElement).style.getPropertyValue("--frame-left")).toBe(`${TURNTABLE_COLUMN.medium}px`);
  });

  it("shows up next as a stack of sleeves that opens the queue drawer", async () => {
    const user = userEvent.setup();
    await settings({ layout: frame("turntable") });
    render(<AppShell />);
    const column = await screen.findByTestId("turntable-column");
    await withTrack();
    const next = await within(column).findByRole("button", { name: /^Up next: .*Open the queue$/ });
    await user.click(next);
    expect((await screen.findByTestId("frame-drawer")).querySelector(".queue-panel")).not.toBeNull();
    expect(next).toHaveAttribute("aria-pressed", "true");
  });

  it("lowers the tonearm on play: the record player follows the play state", async () => {
    await settings({ layout: frame("turntable") });
    render(<AppShell />);
    const column = await screen.findByTestId("turntable-column");
    await withTrack("paused");
    const tt = () => column.querySelector(".turntable")!;
    expect(tt()).toHaveAttribute("data-state", "paused");
    expect(column.querySelector(".tt-plate--speed")).not.toHaveClass("is-on");
    await withTrack("playing");
    expect(tt()).toHaveAttribute("data-state", "playing");
    // The motor lamp (brass) is lit only while the record turns.
    expect(column.querySelector(".tt-plate--speed")).toHaveClass("is-on");
  });

  it("shrinks to a small platter with the transport in a narrow window", async () => {
    setWidth(800);
    await settings({ layout: frame("turntable") });
    render(<AppShell />);
    const column = await screen.findByTestId("turntable-column");
    expect(column).toHaveClass("is-compact");
    expect((document.querySelector(".app") as HTMLElement).style.getPropertyValue("--frame-left")).toBe(`${TURNTABLE_COMPACT}px`);
    expect(within(column).getByRole("button", { name: /^(Play|Pause)$/ })).toBeInTheDocument();
    expect(within(column).queryByRole("region", { name: "Up next" })).toBeNull();
    // Settings and the queue stay in reach.
    expect(within(screen.getByRole("navigation", { name: "Main" })).getByRole("button", { name: "Settings" })).toBeInTheDocument();
    expect(within(column).getByRole("button", { name: "Queue" })).toBeInTheDocument();
  });

  it("follows the column width setting", async () => {
    await settings({ layout: frame("turntable"), turntableColumn: "large" });
    render(<AppShell />);
    await screen.findByTestId("turntable-column");
    expect((document.querySelector(".app") as HTMLElement).style.getPropertyValue("--frame-left")).toBe(`${TURNTABLE_COLUMN.large}px`);
  });

  it("reduced motion holds the record still and puts the arm in place", async () => {
    setReducedMotion(true);
    expect(turntableCss).toMatch(/\[data-reduced-motion="true"\] :is\(\.turntable__arm, \.turntable__arm-shadow\) \{ transition: none; \}/);
    expect(turntableFrameCss).toMatch(/\[data-reduced-motion="true"\] \.tt-tab::after/);
    await settings({ layout: frame("turntable") });
    render(<AppShell />);
    const column = await screen.findByTestId("turntable-column");
    const record = column.querySelector<HTMLElement>(".turntable__record")!;
    await new Promise((r) => setTimeout(r, 60));
    expect(record.style.transform === "" || /rotate\(0\.00deg\)$/.test(record.style.transform)).toBe(true);
  });
});

describe("Settings > Appearance > Layout for Deck and Turntable", () => {
  it("picks Deck, turns its key names off, and keeps that in the synced layout", async () => {
    const user = userEvent.setup();
    render(<LayoutSettings />);
    await user.click(within(screen.getByRole("radiogroup", { name: "Layouts" })).getByRole("radio", { name: "Deck" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.frame).toBe("deck"));
    await user.click(within(await screen.findByRole("radiogroup", { name: "Source key names" })).getByRole("radio", { name: "Icons only" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.deckLabels).toBe(false));
    expect((await mock.settingsGet()).layout.deckLabels).toBe(false);
    expect(screen.getByText(/faceplate folds to two rows/)).toBeInTheDocument();
  });

  it("picks Turntable, sets its column width for this device, and Reset puts it back", async () => {
    const user = userEvent.setup();
    render(<LayoutSettings />);
    await user.click(within(screen.getByRole("radiogroup", { name: "Layouts" })).getByRole("radio", { name: "Turntable" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.frame).toBe("turntable"));
    await user.click(within(await screen.findByRole("radiogroup", { name: "Player column (this device)" })).getByRole("radio", { name: "Wide" }));
    await waitFor(() => expect(useSettings.getState().settings?.turntableColumn).toBe("large"));
    expect((await mock.settingsGet()).turntableColumn).toBe("large");
    await user.click(screen.getByRole("button", { name: "Reset layout" }));
    await waitFor(() => expect(useSettings.getState().settings?.turntableColumn).toBe("medium"));
    expect(useSettings.getState().settings?.layout.frame).toBe("turntable");
  });

  it("refuses a column width that isn't offered", async () => {
    await expect(mock.settingsSet({ turntableColumn: "huge" as never })).rejects.toMatchObject({ kind: "invalid" });
  });
});
