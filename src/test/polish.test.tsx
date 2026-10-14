import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { Heart, ListEnd, Share2 } from "lucide-react";
import { afterEach, describe, expect, it } from "vitest";
import componentsCss from "../components/components.css?raw";
import { ContextMenuHost, openContextMenu } from "../components/ContextMenu";
import { menuIconFor } from "../components/menuIcons";
import { Toaster } from "../components/Toast";
import { color } from "../design/tokens";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import homeCss from "../pages/home.css?raw";
import pagesCss from "../pages/pages.css?raw";
import searchCss from "../pages/search.css?raw";
import { geometry } from "../shell/LibraryCollection";
import shellCss from "../shell/shell.css?raw";
import { toast, useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;

afterEach(() => {
  act(() => useUi.setState({ menu: null }));
  act(() => useToasts.setState({ toasts: [] }));
});

describe("menu icons", () => {
  it("one icon per action, the same in every menu", () => {
    expect(menuIconFor("Add to queue")).toBe(ListEnd);
    expect(menuIconFor("Add popular songs to queue")).toBe(ListEnd);
    expect(menuIconFor("Add to Liked Songs")).toBe(Heart);
    expect(menuIconFor("Share")).toBe(Share2);
    expect(menuIconFor("Add to playlist")).toBe(menuIconFor("Add to other playlist"));
    expect(menuIconFor("Pin to sidebar")).toBe(menuIconFor("Pin"));
    expect(menuIconFor("Unpin from sidebar")).toBe(menuIconFor("Unpin"));
    expect(menuIconFor("Pin")).not.toBe(menuIconFor("Unpin"));
    expect(menuIconFor("Go to Some Artist")).toBe(menuIconFor("Go to artist"));
    expect(menuIconFor("Go to album")).not.toBe(menuIconFor("Go to artist"));
    expect(menuIconFor("Add lyrics")).toBe(menuIconFor("Edit lyrics"));
    expect(menuIconFor("Copy song title")).toBe(menuIconFor("Copy album title"));
    for (const label of ["Remove from Liked Songs", "Remove from queue", "Remove from this playlist", "Delete", "Edit details", "Transcribe songs without lyrics", "Play", "Pause"])
      expect(menuIconFor(label), label).not.toBeNull();
    // Sort and view options carry a check, not an icon.
    expect(menuIconFor("Recently added")).toBeNull();
  });

  it("items get a leading icon column; text lines up and the submenu arrow stays at the end", () => {
    render(<ContextMenuHost />);
    act(() =>
      openContextMenu(
        "Options",
        [
          { label: "Add to queue", onSelect: () => {} },
          { label: "Something new", onSelect: () => {} },
          { label: "Share", submenu: () => null },
        ],
        { clientX: 10, clientY: 10 },
      ),
    );
    const items = within(screen.getByRole("menu")).getAllByRole("menuitem");
    for (const item of items) expect(item.firstElementChild).toHaveClass("context-menu__icon");
    expect(items[0].querySelector(".context-menu__icon svg")).not.toBeNull();
    // No icon for this one, but it keeps the space so labels line up.
    expect(items[1].querySelector(".context-menu__icon svg")).toBeNull();
    expect(items[2].lastElementChild).toHaveClass("context-menu__chevron");
    // Keyboard is unchanged.
    expect(items[0]).toHaveFocus();
    fireEvent.keyDown(items[0], { key: "ArrowDown" });
    expect(items[1]).toHaveFocus();
  });

  it("menus of options only (sort, view) have no icon column", () => {
    render(<ContextMenuHost />);
    act(() => openContextMenu("Sort", [{ label: "Recents", checked: true }, { label: "Alphabetical", checked: false }], { clientX: 0, clientY: 0 }));
    expect(screen.getByRole("menu").querySelector(".context-menu__icon")).toBeNull();
  });
});

describe("toasts", () => {
  it("a toast about a song, album or playlist shows its cover in place of the icon", () => {
    render(<Toaster />);
    act(() => void toast.info("Added to Road Trip", { path: mock.fx.albums[0].thumb, kind: "playlist" }));
    const t = within(screen.getByRole("status", { name: "Notifications" })).getByText("Added to Road Trip").closest<HTMLElement>(".toast")!;
    expect(t).toHaveClass("toast--art");
    expect(t.querySelector(".toast__art .art")).not.toBeNull();
    expect(t.querySelector(".toast__icon")).toBeNull();
  });

  it("other toasts keep the plain style, and no art path means no cover", () => {
    render(<Toaster />);
    act(() => void toast.info("Lyrics saved"));
    act(() => void toast.info("Added to queue", { path: null }));
    for (const t of Array.from(screen.getByRole("status", { name: "Notifications" }).querySelectorAll(".toast"))) {
      expect(t).not.toHaveClass("toast--art");
      expect(t.querySelector(".toast__icon")).not.toBeNull();
    }
  });
});

describe("sidebar grid", () => {
  it("rows leave room for a 16 title and a 14 subtitle, so nothing clips", () => {
    const g = geometry("grid", "sidebar", 276);
    // art + padding + 8 margin + 22 title + 2 gap + 20 subtitle, then the gap between rows
    expect(g.rowHeight).toBe(Math.round(g.art + g.pad * 2 + 52) + 4);
    expect(shellCss).toMatch(/\.lib-cell__title \{ font-size: var\(--font-size-lg\);[^}]*line-height: 22px/);
    expect(shellCss).toMatch(/\.lib-cell__sub \{[^}]*font-size: var\(--font-size-md\);[^}]*line-height: 20px/);
  });
});

describe("playing colour", () => {
  it("is a soft coral that reads on every surface", () => {
    expect(color.playing).toBe("#F0957A");
    const lum = (hex: string) => {
      const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    const ratio = (a: string, b: string) => (Math.max(lum(a), lum(b)) + 0.05) / (Math.min(lum(a), lum(b)) + 0.05);
    for (const bg of [color.bgBase, color.bgRaised, color.bgRaised2, color.bgOverlay]) expect(ratio(color.playing, bg)).toBeGreaterThanOrEqual(4.5);
    expect(color.playing).not.toBe(color.accent);
  });

  it("marks the playing track in every list, the bars, and the playing card", () => {
    expect(componentsCss).toMatch(/\.track-row\.is-active \.track-row__title \{ color: var\(--color-playing\); \}/);
    expect(componentsCss).toMatch(/\.eq i \{[^}]*background: var\(--color-playing\)/);
    expect(componentsCss).toMatch(/\.card\.is-playing \.card__title \{ color: var\(--color-playing\); \}/);
    expect(shellCss).toMatch(/\.lib-cell__title\.is-playing \{ color: var\(--color-playing\); \}/);
    expect(shellCss).toMatch(/\.lib-row__now \{[^}]*color: var\(--color-playing\)/);
    expect(searchCss).toMatch(/\.sr-row\.is-current \.sr-row__title \{ color: var\(--color-playing\); \}/);
    expect(homeCss).toMatch(/\.shortcut\.is-playing \.shortcut__title \{ color: var\(--color-playing\); \}/);
  });
});

describe("settings text", () => {
  it("descriptions sit in the 14 band under 16 titles", () => {
    expect(pagesCss).toMatch(/\.setting-row__title \{ font-size: var\(--font-size-lg\)/);
    expect(pagesCss).toMatch(/\.setting-row__desc \{[^}]*font-size: var\(--font-size-md\)/);
  });
});
