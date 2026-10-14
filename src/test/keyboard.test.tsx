import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FilterChips } from "../components/Tabs";
import { TrackList, TrackRow } from "../components/TrackRow";
import { resetTypeAhead } from "../components/listKeys";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { AppShell } from "../shell/AppShell";
import { FOCUS_REGIONS, nextTabStop, regionOf } from "../shell/focusOrder";
import { SHORTCUTS, shortcutKeys, shortcutLabel, shortcutsIn } from "../shell/shortcuts";
import { ShortcutsHelp } from "../shell/ShortcutsHelp";
import { useShortcuts } from "../shell/useShortcuts";
import { SettingsPage } from "../pages/SettingsPage";
import { useRouter } from "../stores/router";
import { useUi } from "../stores/ui";
import { body } from "./rows";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

function Keys() {
  useShortcuts();
  return null;
}

beforeEach(() => {
  useRouter.getState().reset();
  useUi.setState({ shortcutsOpen: false, queueOpen: false, nowPlayingOpen: false, outputOpen: false, sidebarCollapsed: false, sidebarWidth: 300, rightWidth: 340 });
});
afterEach(() => resetTypeAhead());

describe("keyboard shortcuts window", () => {
  it("lists four sections with key caps", () => {
    act(() => useUi.setState({ shortcutsOpen: true }));
    render(<ShortcutsHelp />);
    const dialog = screen.getByRole("dialog", { name: "Keyboard shortcuts" });
    expect(within(dialog).getAllByRole("heading", { level: 3 }).map((h) => h.textContent)).toEqual(["Basic", "Playback", "Navigation", "Layout"]);
    expect(dialog).toHaveTextContent("Press Ctrl+/ to open or close this list");
    const play = dialog.querySelector('[data-shortcut="playPause"]')!;
    expect(play).toHaveTextContent("Play or pause");
    expect([...play.querySelectorAll("kbd")].map((k) => k.textContent)).toEqual(["Space"]);
    const like = dialog.querySelector('[data-shortcut="like"]')!;
    expect([...like.querySelectorAll("kbd")].map((k) => k.textContent)).toEqual(["Alt", "Shift", "B"]);
  });

  it("shows Mac key names on a Mac", () => {
    const def = (id: string) => SHORTCUTS.find((s) => s.id === id)!;
    expect(shortcutKeys(def("help"), "mac")).toEqual(["⌘", "/"]);
    expect(shortcutKeys(def("like"), "mac")).toEqual(["⌥", "⇧", "B"]);
    expect(shortcutKeys(def("back"), "mac")).toEqual(["⌘", "["]);
    expect(shortcutKeys(def("back"), "windows")).toEqual(["Alt", "←"]);
    expect(shortcutLabel("help", "mac")).toBe("⌘/");
    expect(shortcutLabel("help", "windows")).toBe("Ctrl+/");
  });

  it("has the playback, navigation and layout shortcuts the app supports", () => {
    expect(shortcutsIn("playback").map((s) => s.id)).toEqual(
      expect.arrayContaining(["playPause", "like", "shuffle", "repeat", "previous", "next", "seekBack", "seekForward", "volumeUp", "volumeDown"]),
    );
    expect(shortcutsIn("navigation").map((s) => s.id)).toEqual(
      expect.arrayContaining(["home", "search", "liked", "queue", "library", "settings", "nowPlaying"]),
    );
    expect(shortcutsIn("layout").map((s) => s.id)).toEqual(
      expect.arrayContaining(["toggleSidebar", "sidebarNarrower", "sidebarWider", "toggleRight", "rightNarrower", "rightWider"]),
    );
    // No two shortcuts share keys.
    const keys = SHORTCUTS.map((s) => s.keys.join("+"));
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("Ctrl+/ toggles it and Escape closes it", async () => {
    render(
      <>
        <Keys />
        <ShortcutsHelp />
      </>,
    );
    fireEvent.keyDown(window, { key: "/", code: "Slash", ctrlKey: true });
    expect(await screen.findByRole("dialog", { name: "Keyboard shortcuts" })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "/", code: "Slash", ctrlKey: true });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    act(() => useUi.setState({ shortcutsOpen: true }));
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(useUi.getState().shortcutsOpen).toBe(false);
  });

  it("opens from the app menu and from Settings", async () => {
    const user = userEvent.setup();
    render(<AppShell />);
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    await user.click(screen.getByRole("menuitem", { name: /Keyboard shortcuts/ }));
    expect(useUi.getState().shortcutsOpen).toBe(true);
    act(() => useUi.setState({ shortcutsOpen: false }));
    render(<SettingsPage />);
    await user.click(await screen.findByRole("button", { name: "Show shortcuts" }));
    expect(useUi.getState().shortcutsOpen).toBe(true);
  });
});

describe("navigation and layout shortcuts", () => {
  const altShift = (code: string, key = code.replace(/^(Key|Digit)/, "")) =>
    fireEvent.keyDown(window, { key, code, altKey: true, shiftKey: true });

  it("go to pages with Alt+Shift keys", () => {
    render(<Keys />);
    altShift("KeyS");
    expect(route()).toEqual({ name: "liked" });
    altShift("Digit2");
    expect(route()).toEqual({ name: "library", tab: "artists" });
    altShift("KeyH");
    expect(route()).toEqual({ name: "home" });
    altShift("KeyQ");
    expect(useUi.getState().queueOpen).toBe(true);
    altShift("KeyN");
    expect(useUi.getState()).toMatchObject({ nowPlayingOpen: true, queueOpen: false });
  });

  it("toggle and resize the sidebar and the right panel", () => {
    window.innerWidth = 1600;
    render(<Keys />);
    altShift("KeyL");
    expect(useUi.getState().sidebarCollapsed).toBe(true);
    altShift("KeyL");
    expect(useUi.getState().sidebarCollapsed).toBe(false);
    altShift("ArrowRight", "ArrowRight");
    expect(useUi.getState().sidebarWidth).toBe(332);
    altShift("ArrowLeft", "ArrowLeft");
    expect(useUi.getState().sidebarWidth).toBe(300);
    altShift("KeyR");
    expect(useUi.getState().nowPlayingOpen).toBe(true);
    altShift("BracketLeft", "[");
    expect(useUi.getState().rightWidth).toBe(372);
    altShift("BracketRight", "]");
    expect(useUi.getState().rightWidth).toBe(340);
    altShift("KeyR");
    expect(useUi.getState().nowPlayingOpen).toBe(false);
  });

  it("Ctrl+F opens and focuses the library filter", async () => {
    window.innerWidth = 1600;
    render(<AppShell />);
    fireEvent.keyDown(window, { key: "f", code: "KeyF", ctrlKey: true });
    await vi.waitFor(() => expect(document.activeElement).toHaveAttribute("type", "search"));
  });
});

describe("track lists are one Tab stop", () => {
  const [a, b, c] = mock.fx.tracks;
  function List({ onPlay = vi.fn() }: { onPlay?: () => void }) {
    return (
      <TrackList label="Tracks">
        {[a, b, c].map((t, i) => (
          <TrackRow key={t.id} track={t} index={i + 1} onPlay={onPlay} />
        ))}
      </TrackList>
    );
  }
  const rows = () => body(screen.getAllByRole("row"));

  it("only one row is tabbable, and it follows focus", async () => {
    const user = userEvent.setup();
    render(<List />);
    expect(rows().map((r) => r.getAttribute("tabindex"))).toEqual(["0", "-1", "-1"]);
    rows()[0].focus();
    await user.keyboard("{ArrowDown}");
    expect(rows().map((r) => r.getAttribute("tabindex"))).toEqual(["-1", "0", "-1"]);
  });

  it("arrows select the row they move to; Home and End jump to the ends", async () => {
    const user = userEvent.setup();
    render(<List />);
    rows()[0].focus();
    await user.keyboard("{ArrowDown}");
    expect(rows()[1]).toHaveAttribute("data-selected");
    await user.keyboard("{End}");
    expect(rows()[2]).toHaveFocus();
    expect(rows()[2]).toHaveAttribute("data-selected");
    await user.keyboard("{Home}");
    expect(rows()[0]).toHaveFocus();
  });

  it("Right steps into the row's links, Left walks back to the row", async () => {
    const user = userEvent.setup();
    render(<List />);
    rows()[0].focus();
    await user.keyboard("{ArrowRight}");
    const links = within(rows()[0]).getAllByRole("link");
    expect(links[0]).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    expect(links[1]).toHaveFocus();
    await user.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(rows()[0]).toHaveFocus();
    // Up / Down from a link go on to the next row.
    await user.keyboard("{ArrowRight}{ArrowDown}");
    expect(rows()[1]).toHaveFocus();
  });

  it("typing letters jumps to a song by title", async () => {
    const user = userEvent.setup();
    render(<List />);
    rows()[0].focus();
    await user.keyboard(c.title.charAt(0));
    const firstWithLetter = [b, c, a].find((t) => t.title.toLowerCase().startsWith(c.title.charAt(0).toLowerCase()))!;
    expect(document.activeElement).toHaveAttribute("data-row-key", firstWithLetter.id);
  });
});

describe("chip rows are one Tab stop", () => {
  it("only the selected chip is tabbable; arrows move between chips", async () => {
    const user = userEvent.setup();
    const items = [
      { id: "a", label: "Alpha" },
      { id: "b", label: "Beta" },
      { id: "c", label: "Gamma" },
    ];
    render(<FilterChips label="Kinds" items={items} value="b" onChange={() => {}} />);
    const chips = screen.getAllByRole("button");
    expect(chips.map((c) => c.getAttribute("tabindex"))).toEqual(["-1", "0", "-1"]);
    chips[1].focus();
    await user.keyboard("{ArrowRight}");
    expect(chips[2]).toHaveFocus();
    await user.keyboard("{Home}");
    expect(chips[0]).toHaveFocus();
    expect(chips.map((c) => c.getAttribute("tabindex"))).toEqual(["0", "-1", "-1"]);
  });
});

describe("Tab order through the shell", () => {
  it("goes skip link, top bar, sidebar, player bar, then the page", () => {
    window.innerWidth = 1600;
    render(<AppShell />);
    const shell = document.querySelector<HTMLElement>(".app")!;
    const skip = screen.getByRole("link", { name: "Skip to main content" });
    expect(regionOf(skip)).toBe(0);
    // Walk forward from the skip link and record the region of each stop.
    const seen: number[] = [];
    let at: HTMLElement | null = skip;
    for (let i = 0; i < 400 && at; i++) {
      seen.push(regionOf(at, shell));
      at = nextTabStop(shell, at, false);
    }
    const order = seen.filter((r, i) => i === 0 || r !== seen[i - 1]);
    expect(order.slice(0, 5)).toEqual([0, 1, 2, 3, 4]);
    expect(FOCUS_REGIONS[3]).toContain(".player-bar");
    // Shift+Tab from the page's first stop goes back into the player bar.
    const firstInPage = (() => {
      let el: HTMLElement | null = skip;
      while (el && regionOf(el, shell) !== 4) el = nextTabStop(shell, el, false);
      return el!;
    })();
    expect(regionOf(nextTabStop(shell, firstInPage, true)!, shell)).toBe(3);
  });
});
