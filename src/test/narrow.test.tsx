import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { FilterChips } from "../components/Tabs";
import { solveLayout, RIGHT_MIN, RAIL_WIDTH, MAIN_MIN } from "../shell/layout";
import { TopBar } from "../shell/TopBar";
import { nav, useRouter } from "../stores/router";

const startWidth = window.innerWidth;
function setWidth(w: number) {
  act(() => {
    window.innerWidth = w;
    window.dispatchEvent(new Event("resize"));
  });
}
afterEach(() => setWidth(startWidth));

describe("narrow windows", () => {
  it("give way in order: sidebar to the rail, then the right panel, then main bottoms out", () => {
    const user = { sidebarWidth: 380, rightWidth: 400, rightOpen: true, sidebarCollapsed: false };
    // Wide: everything at the user's widths.
    expect(solveLayout({ ...user, windowWidth: 1600 })).toMatchObject({ sidebar: 380, right: 400 });
    // Narrower: the sidebar rails while the right panel keeps its width.
    expect(solveLayout({ ...user, windowWidth: 1100 })).toMatchObject({ sidebar: RAIL_WIDTH, right: 400 });
    // The narrowest: the right panel at its minimum and main near its floor.
    const tight = solveLayout({ ...user, windowWidth: 800 });
    expect(tight).toMatchObject({ sidebar: RAIL_WIDTH, right: RIGHT_MIN });
    expect(Math.abs(tight.main - MAIN_MIN)).toBeLessThan(8);
    // Room again: the right panel's own width comes back (it isn't left narrow).
    expect(solveLayout({ ...user, windowWidth: 1100 }).right).toBe(400);
  });

  it("keep the search field open while it holds a query", () => {
    useRouter.getState().reset();
    setWidth(800);
    render(<TopBar />);
    expect(screen.queryByRole("combobox", { name: "Search" })).not.toBeInTheDocument();
    act(() => nav.go({ name: "search", query: "glass" }));
    expect(screen.getByRole("combobox", { name: "Search" })).toHaveValue("glass");
  });

  it("chip rows can scroll sideways in one line", () => {
    const items = [
      { id: "a", label: "All" },
      { id: "b", label: "Songs" },
    ];
    render(<FilterChips label="Filter results" items={items} value="a" onChange={() => {}} scroll />);
    const row = screen.getByRole("group", { name: "Filter results" });
    expect(row).toHaveClass("chips--scroll");
    expect(row.parentElement).toHaveClass("chips-scroll");
  });
});
