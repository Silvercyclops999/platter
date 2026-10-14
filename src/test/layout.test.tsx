import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  RAIL_WIDTH,
  RIGHT_MAX,
  RIGHT_MIN,
  SIDEBAR_MAX,
  SIDEBAR_MIN,
  STRIP_WIDTH,
  dragRight,
  dragSidebar,
  keyRight,
  keySidebar,
  solveLayout,
  type LayoutInput,
} from "../shell/layout";
import { AppShell } from "../shell/AppShell";
import { loadPanels, useUi } from "../stores/ui";

const WIDTHS = [800, 1024, 1280, 1440, 1600, 1920, 2560] as const;

/** Solve with the given user widths; returns [sidebar, mode, main, right]. */
function solve(windowWidth: number, rightOpen: boolean, user: Partial<LayoutInput> = {}) {
  const l = solveLayout({ windowWidth, sidebarWidth: 420, rightWidth: 420, rightOpen, sidebarCollapsed: false, ...user });
  return [l.sidebar, l.sidebarMode, l.main, l.right] as const;
}

describe("layout solver", () => {
  it("shares the window with both panels at their max (right panel open)", () => {
    // Between 1024 and 1280 the right panel shrinks first, then the sidebar rails.
    expect(solve(1200, true)).toEqual([420, "full", 400, 348]);
    expect(solve(1100, true)).toEqual([RAIL_WIDTH, "rail", 716, RIGHT_MIN]);
    expect(WIDTHS.map((w) => solve(w, true))).toEqual([
      // The right panel is at its min and the sidebar on the rail.
      [RAIL_WIDTH, "rail", 416, RIGHT_MIN],
      [RAIL_WIDTH, "rail", 640, RIGHT_MIN],
      // Just fits: main sits a little above its floor.
      [420, "full", 408, 420],
      [420, "full", 568, 420],
      [420, "full", 728, 420],
      [420, "full", 1048, 420],
      [420, "full", 1688, 420],
    ]);
  });

  it("narrows the sidebar instead of railing it when the right panel is closed", () => {
    expect(WIDTHS.map((w) => solve(w, false))).toEqual([
      [344, "narrowed", 400, STRIP_WIDTH],
      [420, "full", 548, STRIP_WIDTH],
      [420, "full", 804, STRIP_WIDTH],
      [420, "full", 964, STRIP_WIDTH],
      [420, "full", 1124, STRIP_WIDTH],
      [420, "full", 1444, STRIP_WIDTH],
      [420, "full", 2084, STRIP_WIDTH],
    ]);
  });

  it("keeps default widths at every size where they fit", () => {
    const open = WIDTHS.map((w) => solve(w, true, { sidebarWidth: 300, rightWidth: 340 }));
    expect(open[0]).toEqual([RAIL_WIDTH, "rail", 416, RIGHT_MIN]);
    expect(open[1]).toEqual([300, "full", 400, 292]);
    for (const [s, mode, main, right] of open.slice(2)) {
      expect([s, mode, right]).toEqual([300, "full", 340]);
      expect(main).toBeGreaterThan(400);
    }
    const closed = WIDTHS.map((w) => solve(w, false, { sidebarWidth: 300, rightWidth: 340 }));
    expect(closed.every(([s, mode]) => s === 300 && mode === "full")).toBe(true);
  });

  it("never lets the main column go under its floor while something can give way", () => {
    for (const w of WIDTHS)
      for (const open of [true, false]) {
        const [, , main] = solve(w, open);
        expect(main).toBeGreaterThanOrEqual(400);
      }
  });

  it("restores the user's widths when the window widens again", () => {
    const input = { sidebarWidth: 380, rightWidth: 400, rightOpen: true, sidebarCollapsed: false };
    const narrow = solveLayout({ ...input, windowWidth: 1024 });
    expect(narrow.sidebarMode).toBe("rail");
    expect(narrow.sidebarAuto).toBe(true);
    expect(narrow.right).toBe(RIGHT_MIN);
    const wide = solveLayout({ ...input, windowWidth: 1920 });
    expect(wide).toMatchObject({ sidebar: 380, sidebarMode: "full", sidebarAuto: false, right: 400 });
    // Closed panel: narrowed, then back to the user's width.
    expect(solveLayout({ ...input, rightOpen: false, windowWidth: 800 }).sidebarMode).toBe("narrowed");
    expect(solveLayout({ ...input, rightOpen: false, windowWidth: 1440 }).sidebar).toBe(380);
  });

  it("keeps a user-collapsed sidebar on the rail at any width", () => {
    const l = solveLayout({ windowWidth: 2560, sidebarWidth: 300, rightWidth: 340, rightOpen: true, sidebarCollapsed: true });
    expect(l).toMatchObject({ sidebar: RAIL_WIDTH, sidebarMode: "rail", sidebarAuto: false });
  });

  it("clamps stored widths into range", () => {
    const l = solveLayout({ windowWidth: 2560, sidebarWidth: 9000, rightWidth: 10, rightOpen: true, sidebarCollapsed: false });
    expect(l.sidebar).toBe(SIDEBAR_MAX);
    expect(l.right).toBe(RIGHT_MIN);
  });
});

describe("panel drags", () => {
  const full = { width: 300, rail: false };
  const rail = { width: RAIL_WIDTH, rail: true };

  it("follows the pointer between min and max", () => {
    expect(dragSidebar(full, 50)).toEqual({ width: 350, rail: false });
    expect(dragSidebar(full, 500)).toEqual({ width: SIDEBAR_MAX, rail: false });
    expect(dragSidebar(full, 500, 360)).toEqual({ width: 360, rail: false });
  });

  it("holds at the min, then snaps to the rail about 120 px past it", () => {
    expect(dragSidebar(full, -60)).toEqual({ width: SIDEBAR_MIN, rail: false });
    expect(dragSidebar(full, -139)).toEqual({ width: SIDEBAR_MIN, rail: false });
    expect(dragSidebar(full, -140)).toEqual({ width: RAIL_WIDTH, rail: true });
    // Dragging back in the same drag reopens it.
    expect(dragSidebar(full, -100)).toEqual({ width: SIDEBAR_MIN, rail: false });
  });

  it("snaps open from the rail at the min, then follows the pointer", () => {
    expect(dragSidebar(rail, 119)).toEqual(rail);
    expect(dragSidebar(rail, 120)).toEqual({ width: SIDEBAR_MIN, rail: false });
    expect(dragSidebar(rail, 300)).toEqual({ width: RAIL_WIDTH + 300, rail: false });
    expect(dragSidebar(rail, -50)).toEqual(rail);
  });

  it("stops the right panel at its min and max (it never collapses)", () => {
    expect(dragRight(340, 1000)).toBe(RIGHT_MIN);
    expect(dragRight(340, -40)).toBe(380);
    expect(dragRight(340, -1000)).toBe(RIGHT_MAX);
    expect(dragRight(340, -1000, 360)).toBe(360);
  });

  it("resizes from the keyboard", () => {
    expect(keySidebar(full, "ArrowRight")).toEqual({ width: 316, rail: false });
    expect(keySidebar({ width: SIDEBAR_MIN, rail: false }, "ArrowLeft")).toEqual(rail);
    expect(keySidebar(rail, "ArrowRight")).toEqual({ width: SIDEBAR_MIN, rail: false });
    expect(keySidebar(full, "End")).toEqual({ width: SIDEBAR_MAX, rail: false });
    expect(keySidebar(full, "a")).toBeNull();
    expect(keyRight(340, "ArrowLeft")).toBe(356);
    expect(keyRight(RIGHT_MIN, "ArrowRight")).toBe(RIGHT_MIN);
    expect(keyRight(340, "Home")).toBe(RIGHT_MIN);
  });
});

describe("resize handles in the shell", () => {
  const initial = useUi.getState();
  beforeEach(() => {
    localStorage.clear();
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
    useUi.setState({ sidebarWidth: 300, sidebarCollapsed: false, rightWidth: 340, queueOpen: false });
  });
  afterEach(() => {
    useUi.setState({ sidebarWidth: initial.sidebarWidth, sidebarCollapsed: initial.sidebarCollapsed, rightWidth: initial.rightWidth, queueOpen: false });
    localStorage.clear();
  });

  const shellVar = (name: string) => (document.querySelector(".app") as HTMLElement).style.getPropertyValue(name);

  it("drags the sidebar to the rail and back open, saving on release", () => {
    render(<AppShell />);
    const handle = screen.getByRole("separator", { name: "Resize sidebar" });
    expect(shellVar("--sidebar-w")).toBe("300px");

    fireEvent.pointerDown(handle, { button: 0, pointerId: 1, clientX: 300 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 340 });
    // Mid-drag: the CSS variable moves, the store doesn't.
    expect(shellVar("--sidebar-w")).toBe("340px");
    expect(useUi.getState().sidebarWidth).toBe(300);
    fireEvent.pointerUp(handle, { pointerId: 1, clientX: 340 });
    expect(useUi.getState().sidebarWidth).toBe(340);
    expect(loadPanels().sidebarWidth).toBe(340);

    // Well past the min: snaps to the rail.
    fireEvent.pointerDown(handle, { button: 0, pointerId: 2, clientX: 340 });
    fireEvent.pointerMove(handle, { pointerId: 2, clientX: 100 });
    expect(useUi.getState().sidebarCollapsed).toBe(true);
    fireEvent.pointerUp(handle, { pointerId: 2, clientX: 100 });
    expect(document.querySelector(".sidebar")).toHaveClass("is-collapsed");
    expect(shellVar("--sidebar-w")).toBe(`${RAIL_WIDTH}px`);
    expect(loadPanels().sidebarCollapsed).toBe(true);

    // Out from the rail: snaps open at the min.
    const h2 = screen.getByRole("separator", { name: "Resize sidebar" });
    fireEvent.pointerDown(h2, { button: 0, pointerId: 3, clientX: 72 });
    fireEvent.pointerMove(h2, { pointerId: 3, clientX: 200 });
    fireEvent.pointerUp(h2, { pointerId: 3, clientX: 200 });
    expect(useUi.getState()).toMatchObject({ sidebarCollapsed: false, sidebarWidth: SIDEBAR_MIN });
    expect(document.querySelector(".sidebar")).not.toHaveClass("is-collapsed");
  });

  it("resizes the sidebar with arrow keys", () => {
    render(<AppShell />);
    const handle = screen.getByRole("separator", { name: "Resize sidebar" });
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(useUi.getState().sidebarWidth).toBe(316);
    expect(handle).toHaveAttribute("aria-valuenow", "316");
  });

  it("stops the right panel at its min when dragged narrower", () => {
    useUi.setState({ queueOpen: true });
    render(<AppShell />);
    const handle = screen.getByRole("separator", { name: "Resize queue" });
    fireEvent.pointerDown(handle, { button: 0, pointerId: 1, clientX: 1000 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 1400 });
    expect(shellVar("--right-w")).toBe(`${RIGHT_MIN}px`);
    fireEvent.pointerUp(handle, { pointerId: 1, clientX: 1400 });
    expect(useUi.getState().rightWidth).toBe(RIGHT_MIN);
    expect(useUi.getState().queueOpen).toBe(true);
  });

  it("auto-collapses the sidebar for a narrow window and hides its handle", () => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 800 });
    useUi.setState({ queueOpen: true });
    render(<AppShell />);
    expect(document.querySelector(".sidebar")).toHaveClass("is-collapsed");
    expect(screen.queryByRole("separator", { name: "Resize sidebar" })).toBeNull();
    // Widening restores the user's width; nothing was written to the store.
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(document.querySelector(".sidebar")).not.toHaveClass("is-collapsed");
    expect(shellVar("--sidebar-w")).toBe("300px");
    expect(useUi.getState().sidebarCollapsed).toBe(false);
  });

  it("leaves a strip that reopens the closed right panel", () => {
    render(<AppShell />);
    expect(shellVar("--right-w")).toBe(`${STRIP_WIDTH}px`);
    fireEvent.click(screen.getByRole("button", { name: "Open queue" }));
    expect(useUi.getState().queueOpen).toBe(true);
    expect(screen.getByRole("complementary", { name: "Queue" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Close queue" }));
    expect(useUi.getState().queueOpen).toBe(false);
    expect(screen.getByRole("button", { name: "Open queue" })).toBeInTheDocument();
  });
});
