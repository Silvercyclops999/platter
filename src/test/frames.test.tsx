/**
 * Layouts: the frame model and its checks, old looks loading as Classic,
 * the region host drawing Index and custom frames with every required part
 * reachable, the transport strip keeping the player bar's controls, the
 * first-run "Pick a layout and a look" step, the one-time offer for people
 * on Classic, and Settings > Appearance > Layout.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import framesCss from "../shell/frames.css?raw";
import { CLASSIC_LAYOUT, DEFAULT_REGIONS, INDEX_LAYOUT, regionsProblem, safeLayout, validateThemeText, type Layout, type Regions } from "../design/themes/validate";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Settings } from "../lib/types";
import { PickLookDialog } from "../pages/looks/LookCards";
import { LayoutSettings, placeComponent } from "../pages/looks/LayoutCards";
import { AppShell } from "../shell/AppShell";
import { LayoutOffer } from "../shell/LayoutOffer";
import { FRAME_MAIN_MIN, FRAME_RAIL, FRAME_INFO, frameRegions, indexRegions, solveFrame } from "../shell/frames";
import { FOCUS_REGIONS, shellTabOrder, regionOf } from "../shell/focusOrder";
import { framePatch, layoutOfferDue, looks, skipPatch, useLooks } from "../stores/looks";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;

async function settings(p: Partial<Settings> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...p });
  await loadSettings();
}

const index = (over: Partial<Layout> = {}): Layout => ({ ...structuredClone(INDEX_LAYOUT), ...over });

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
afterEach(() => {
  setWidth(startWidth);
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, outputOpen: false, activityOpen: false, helpOpen: false });
  useLooks.setState({ pick: false, builder: null });
});

describe("the frame model", () => {
  it("a look without a frame loads as Classic, and Classic is still the stored default", () => {
    const r = validateThemeText("old.platter-look", JSON.stringify({ version: 1, name: "Old", base: "dark", layout: { sidebarSide: "right" } }));
    expect(r.ok && r.theme.layout?.frame).toBe("classic");
    expect(r.ok && r.theme.layout?.sidebarSide).toBe("right");
    expect(safeLayout({ quickAccess: 6 }).frame).toBe("classic");
    expect(safeLayout({ frame: "cinema" })).toEqual(CLASSIC_LAYOUT);
    expect(CLASSIC_LAYOUT.frame).toBe("classic");
    expect(INDEX_LAYOUT.frame).toBe("index");
  });

  it("refuses arrangements that break the slot table or drop a required part, with the reason", () => {
    expect(regionsProblem(DEFAULT_REGIONS)).toBeNull();
    const without = (c: "queue" | "search" | "transport" | "nowPlaying" | "navigation") => placeComponent(DEFAULT_REGIONS, c, null);
    expect(regionsProblem(without("queue"))).toMatch(/needs "queue"/);
    expect(regionsProblem(without("search"))).toMatch(/needs "search"/);
    expect(regionsProblem(without("transport"))).toMatch(/needs "transport"/);
    expect(regionsProblem(without("nowPlaying"))).toMatch(/needs "nowPlaying"/);
    expect(regionsProblem(without("navigation"))).toMatch(/needs "navigation"/);
    expect(regionsProblem({ ...DEFAULT_REGIONS, right: ["queue", "library"], main: [] } as Regions)).toMatch(/can't go in right/);
    expect(regionsProblem({ ...DEFAULT_REGIONS, spine: ["transport"] })).toMatch(/isn't a region/);
    expect(regionsProblem({ ...DEFAULT_REGIONS, top: ["transport", "nowPlaying", "visualiser"] })).toMatch(/layout\.regions\.top/);
  });

  it("Index puts the strip top or bottom and the queue, lyrics and credits in a drawer; Classic has no arrangement", () => {
    expect(frameRegions(CLASSIC_LAYOUT)).toBeNull();
    const top = indexRegions("top");
    expect(top.top).toContain("transport");
    expect(top.bottom).toEqual([]);
    expect(top.left).toEqual(["search", "navigation"]);
    expect(top.overlay).toEqual(["queue", "lyrics", "credits"]);
    expect(regionsProblem(top)).toBeNull();
    const bottom = indexRegions("bottom");
    expect(bottom.bottom).toContain("transport");
    expect(bottom.top).toEqual([]);
    const custom = { ...CLASSIC_LAYOUT, frame: "custom" as const, regions: { ...DEFAULT_REGIONS, top: [], bottom: ["transport", "nowPlaying"] } as Regions };
    expect(frameRegions(custom)?.bottom).toEqual(["transport", "nowPlaying"]);
  });

  it("gives way in narrow windows: the right column turns into a drawer, then the left folds to icons", () => {
    const regions = { ...DEFAULT_REGIONS };
    const at = (w: number) => solveFrame({ windowWidth: w, regions, leftSize: "medium", rightSize: "medium" });
    expect(at(1280)).toMatchObject({ leftMode: "full", rightMode: "column", left: 240 });
    expect(at(1000)).toMatchObject({ leftMode: "full", rightMode: "drawer" });
    expect(at(800)).toMatchObject({ leftMode: "rail", rightMode: "drawer", left: FRAME_RAIL });
    for (const w of [1280, 1000, 800]) {
      const s = at(w);
      expect(w - 24 - s.left - (s.rightMode === "column" ? s.right : 0)).toBeGreaterThanOrEqual(FRAME_MAIN_MIN);
    }
    // Size presets stay inside their clamps.
    expect(solveFrame({ windowWidth: 1600, regions, leftSize: "large", rightSize: "small" })).toMatchObject({ left: 288, right: 300 });
  });

  it("lists the six frames plus Custom, with only the built ones ready", () => {
    expect(FRAME_INFO.map((f) => f.id)).toEqual(["index", "deck", "turntable", "gatefold", "classic", "console", "custom"]);
    expect(FRAME_INFO.filter((f) => f.ready).map((f) => f.id)).toEqual(["index", "deck", "turntable", "gatefold", "classic", "console", "custom"]);
  });

  it("panels slide in with movement scaled by the motion setting, so reduced motion fades", () => {
    expect(framesCss).toMatch(/@keyframes frame-panel-in \{\s*from \{ opacity: 0; translate: calc\(24px \* var\(--motion-move\)\) 0; \}/);
    expect(framesCss).toMatch(/@keyframes frame-fade-in/);
  });
});

describe("the Index frame", () => {
  it("has the sections sidebar with search, the strip on top of the page and no player bar or top bar", async () => {
    await settings({ layout: index() });
    render(<AppShell />);
    const strip = await screen.findByTestId("transport-strip");
    expect(document.querySelector(".player-bar")).toBeNull();
    expect(document.querySelector(".topbar")).toBeNull();
    expect(document.querySelector(".sidebar")).toBeNull();
    const navEl = screen.getByRole("navigation", { name: "Main" });
    expect(within(navEl).getByRole("combobox", { name: "Search" })).toBeInTheDocument();
    for (const name of ["Home", "Search", "Artists", "Albums", "Settings"]) expect(within(navEl).getByRole("button", { name: new RegExp(`^${name}$`) })).toBeInTheDocument();
    // The strip comes before the page, and the page holds the main view.
    const main = screen.getByRole("main");
    expect(strip.compareDocumentPosition(main) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(strip.closest(".frame-edge--top")).not.toBeNull();
  });

  it("keeps the player bar's controls and accessible names in the strip", async () => {
    await settings({ layout: index() });
    render(<AppShell />);
    const strip = within(await screen.findByRole("complementary", { name: "Player" }));
    for (const name of [/^(Play|Pause)$/, /^Previous$/, /^Next$/, /^Queue$/, /^Connect to a device$/, /^(Mute|Unmute)$/]) expect(strip.getByRole("button", { name })).toBeInTheDocument();
    expect(strip.getByRole("slider", { name: /^Seek/ })).toBeInTheDocument();
    expect(strip.getByRole("slider", { name: "Volume" })).toBeInTheDocument();
    // The seek line is always the tonearm here.
    expect(strip.getByRole("slider", { name: /^Seek/ }).closest(".slider--tonearm")).not.toBeNull();
  });

  it("opens the queue as a drawer from the strip, and Escape closes it", async () => {
    const user = userEvent.setup();
    await settings({ layout: index() });
    render(<AppShell />);
    const strip = within(await screen.findByTestId("transport-strip"));
    await user.click(strip.getByRole("button", { name: "Queue" }));
    const drawer = await screen.findByTestId("frame-drawer");
    expect(drawer.querySelector(".queue-panel")).not.toBeNull();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByTestId("frame-drawer")).toBeNull());
    expect(useUi.getState().queueOpen).toBe(false);
  });

  it("moves the strip under the page with the transport option at the bottom", async () => {
    await settings({ layout: index({ transport: "bottom" }) });
    render(<AppShell />);
    const strip = await screen.findByTestId("transport-strip");
    expect(strip.closest(".frame-edge--bottom")).not.toBeNull();
    expect(screen.getByRole("main").compareDocumentPosition(strip) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("folds the sections to an icon rail at 800 px, keeping search, sections and Settings reachable", async () => {
    setWidth(800);
    await settings({ layout: index() });
    render(<AppShell />);
    const navEl = await screen.findByRole("navigation", { name: "Main" });
    expect(navEl).toHaveClass("is-rail");
    for (const name of ["Search", "Home", "Albums", "Settings"]) expect(within(navEl).getAllByRole("button", { name }).length).toBeGreaterThan(0);
    expect(screen.getByTestId("transport-strip")).toBeInTheDocument();
  });

  it("tabs through the window row, the sections, the strip, then the page", async () => {
    await settings({ layout: index() });
    render(<AppShell />);
    await screen.findByTestId("transport-strip");
    const order = shellTabOrder(document.querySelector(".app") as HTMLElement).map((el) => regionOf(el));
    const firstOf = (r: number) => order.indexOf(r);
    expect(FOCUS_REGIONS[3]).toContain(".transport-strip");
    expect(firstOf(1)).toBeLessThan(firstOf(2));
    expect(firstOf(2)).toBeLessThan(firstOf(3));
    expect(firstOf(3)).toBeLessThan(firstOf(4));
    // Monotonic: nothing from an earlier region comes after a later one.
    expect([...order].sort((a, b) => a - b)).toEqual(order);
  });
});

describe("custom frames", () => {
  it("draw from the arrangement: navigation as keys on top, the player along the bottom, the queue in place of the page", async () => {
    const user = userEvent.setup();
    const regions: Regions = { top: ["navigation", "search"], left: [], main: ["library", "queue"], right: [], bottom: ["transport", "nowPlaying", "signal"], overlay: [] };
    expect(regionsProblem(regions)).toBeNull();
    await settings({ layout: { ...structuredClone(CLASSIC_LAYOUT), frame: "custom", regions } });
    render(<AppShell />);
    const strip = await screen.findByTestId("transport-strip");
    expect(strip.closest(".frame-edge--bottom")).not.toBeNull();
    expect(document.querySelector(".frame-left")).toBeNull();
    expect(document.querySelector(".frame-keys")).not.toBeNull();
    await user.click(within(strip).getByRole("button", { name: "Queue" }));
    await waitFor(() => expect(document.querySelector(".frame-page__panel .queue-panel")).not.toBeNull());
    expect(screen.queryByRole("main")).toBeNull();
  });

  it("the region editor saves a good move and refuses one that drops a required part", async () => {
    const user = userEvent.setup();
    await settings({ layout: { ...structuredClone(CLASSIC_LAYOUT), frame: "custom" } });
    render(<LayoutSettings />);
    await user.selectOptions(screen.getByRole("combobox", { name: "Player controls" }), "bottom");
    await waitFor(() => expect(useSettings.getState().settings?.layout.regions.bottom).toContain("transport"));
    await user.selectOptions(screen.getByRole("combobox", { name: "Queue" }), "");
    expect(await screen.findByRole("alert")).toHaveTextContent(/needs "queue"/);
    expect(useSettings.getState().settings?.layout.regions.overlay.concat(useSettings.getState().settings!.layout.regions.right)).toContain("queue");
    // Sizes are presets only.
    await user.click(within(screen.getByRole("radiogroup", { name: "Left side" })).getByRole("radio", { name: "Wide" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.leftSize).toBe("large"));
  });

  it("travel in a look file", async () => {
    const regions = { ...DEFAULT_REGIONS, top: [], bottom: ["transport", "nowPlaying"] };
    const text = JSON.stringify({ version: 1, name: "Mine", base: "dark", layout: { frame: "custom", regions, leftSize: "small" } });
    const r = validateThemeText("Mine.platter-look", text);
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.theme.layout?.frame).toBe("custom");
      expect(r.theme.layout?.regions.bottom).toEqual(["transport", "nowPlaying"]);
      expect(r.theme.layout?.leftSize).toBe("small");
    }
  });
});

describe("the first-run step", () => {
  it("is Pick a layout and a look: Index first, every frame ready, Classic and Console under More", async () => {
    const user = userEvent.setup();
    render(<PickLookDialog />);
    act(() => looks.openPick());
    const dialog = await screen.findByRole("dialog", { name: "Pick a layout and a look" });
    const layouts = within(within(dialog).getByRole("radiogroup", { name: "Layout" }));
    expect(layouts.getAllByRole("radio").map((r) => r.getAttribute("aria-label"))).toEqual(["Index", "Deck", "Turntable", "Gatefold"]);
    await user.click(layouts.getByRole("radio", { name: "Gatefold" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.frame).toBe("gatefold"));
    await user.click(within(dialog).getByRole("button", { name: "More layouts" }));
    expect(layouts.getByRole("radio", { name: "Classic" })).toBeInTheDocument();
    expect(layouts.getByRole("radio", { name: "Console" })).not.toHaveAttribute("aria-disabled");
    await user.click(layouts.getByRole("radio", { name: "Index" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.frame).toBe("index"));
    expect(useSettings.getState().settings?.layoutOffered).toBe(true);
  });

  it("Skip means Index in Studio", async () => {
    const user = userEvent.setup();
    await settings({ theme: "midnight", accent: "sage", followSystem: false });
    render(<PickLookDialog />);
    act(() => looks.openPick());
    await user.click(await screen.findByRole("button", { name: "Skip" }));
    await waitFor(() => expect(useSettings.getState().settings).toMatchObject({ theme: "studio", accent: "brass", lookStepDone: true }));
    expect(useSettings.getState().settings?.layout.frame).toBe("index");
    expect(useLooks.getState().pick).toBe(false);
    expect(skipPatch(null, true).layout?.frame).toBe("index");
  });
});

describe("the one-time layout offer", () => {
  it("shows once to someone on Classic after the first-run step, and Keep Classic puts it away for good", async () => {
    const user = userEvent.setup();
    await settings({ layoutOffered: false, lookStepDone: true });
    expect(layoutOfferDue(useSettings.getState().settings)).toBe(true);
    const { unmount } = render(<LayoutOffer />);
    await user.click(await screen.findByRole("button", { name: "Keep Classic" }));
    await waitFor(() => expect(useSettings.getState().settings?.layoutOffered).toBe(true));
    expect(screen.queryByRole("region", { name: "Try the Index layout" })).toBeNull();
    unmount();
    render(<LayoutOffer />);
    expect(screen.queryByRole("region", { name: "Try the Index layout" })).toBeNull();
  });

  it("Try Index switches and keeps the rest of the layout", async () => {
    const user = userEvent.setup();
    await settings({ layoutOffered: false, lookStepDone: true, layout: { ...structuredClone(CLASSIC_LAYOUT), quickAccess: 6 } });
    render(<LayoutOffer />);
    await user.click(await screen.findByRole("button", { name: "Try Index" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout).toMatchObject({ frame: "index", quickAccess: 6 }));
    expect(useSettings.getState().settings?.layoutOffered).toBe(true);
  });

  it("isn't shown during the first-run step, on another frame, or once seen", () => {
    const base = { ...DEFAULT_SETTINGS, layoutOffered: false, lookStepDone: true };
    expect(layoutOfferDue({ ...base, lookStepDone: false })).toBe(false);
    expect(layoutOfferDue({ ...base, layout: index() })).toBe(false);
    expect(layoutOfferDue({ ...base, layoutOffered: true })).toBe(false);
    expect(layoutOfferDue(null)).toBe(false);
  });
});

describe("Settings > Appearance > Layout", () => {
  it("picks a frame from the cards, sets the strip's place, resets and switches back to Classic", async () => {
    const user = userEvent.setup();
    render(<LayoutSettings />);
    const cards = screen.getByRole("radiogroup", { name: "Layouts" });
    expect(within(cards).getByRole("radio", { name: "Classic" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("button", { name: "Switch to Classic" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Reset layout" })).toBeDisabled();
    await user.click(within(cards).getByRole("radio", { name: "Index" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.frame).toBe("index"));
    await user.click(within(await screen.findByRole("radiogroup", { name: "Player strip" })).getByRole("radio", { name: "Bottom" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.transport).toBe("bottom"));
    // Persisted in the core's copy too.
    expect((await mock.settingsGet()).layout.transport).toBe("bottom");
    await user.click(screen.getByRole("button", { name: "Reset layout" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout).toEqual(index()));
    await user.click(screen.getByRole("button", { name: "Switch to Classic" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.frame).toBe("classic"));
  });

  it("the frame patch keeps the other options and marks the offer seen", () => {
    const s = { ...DEFAULT_SETTINGS, layout: { ...structuredClone(CLASSIC_LAYOUT), homeGreeting: true } };
    expect(framePatch("index", s)).toMatchObject({ layout: { frame: "index", homeGreeting: true }, layoutOffered: true });
  });
});
