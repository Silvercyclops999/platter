import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { AppShell } from "../shell/AppShell";
import { connectMenuBar, MENU_ACTIONS, matchesShortcut } from "../shell/menuBar";
import { SHORTCUTS } from "../shell/shortcuts";
import { useShortcuts } from "../shell/useShortcuts";
import { player } from "../stores/player";
import { useRouter } from "../stores/router";
import { useUi } from "../stores/ui";
// The core's menu table, as text.
import menubarSource from "../../src-tauri/src/platform/menubar.rs?raw";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

/** The menu table in the core, read as text: [action, accelerator]. */
function coreItems(): [string, string | null][] {
  const src: string = menubarSource;
  const re = /(?:ui|play)\(\s*(?:"menu-[^"]+"|[A-Z_]+_ID),\s*"[^"]+",\s*(None|Some\("([^"]+)"\)),\s*"(\w+)"/g;
  return [...src.matchAll(re)].map((m) => [m[3], m[2] ?? null]);
}

/** A shortcut's Mac keys written the way the core writes accelerators. */
function asAccelerator(id: string): string {
  const def = SHORTCUTS.find((s) => s.id === id)!;
  const names: Record<string, string> = { Mod: "Cmd", "←": "Left", "→": "Right", "↑": "Up", "↓": "Down" };
  return (def.mac ?? def.keys).map((k) => names[k] ?? k).join("+");
}

const norm = (a: string) => a.toUpperCase().split("+").sort().join("+");

function Keys() {
  useShortcuts();
  return null;
}

beforeEach(() => {
  useRouter.getState().reset();
  useUi.setState({ shortcutsOpen: false, paletteOpen: false, queueOpen: false });
});
afterEach(() => vi.restoreAllMocks());

describe("menu bar", () => {
  it("has a UI action for every menu item the core sends, and no extras", () => {
    const names = coreItems().map(([a]) => a);
    expect(names.length).toBeGreaterThan(15);
    expect(new Set(names)).toEqual(new Set(Object.keys(MENU_ACTIONS)));
  });

  it("uses the app's own keys, and new keys don't clash with any shortcut", () => {
    const taken = SHORTCUTS.map((s) => norm(asAccelerator(s.id)));
    for (const [action, accel] of coreItems()) {
      const shortcut = MENU_ACTIONS[action].shortcut;
      if (shortcut) expect(accel && norm(accel), action).toBe(norm(asAccelerator(shortcut)));
      else if (accel) expect(taken, action).not.toContain(norm(accel));
    }
    // Space stays with the page.
    expect(coreItems().find(([a]) => a === "playPause")![1]).toBeNull();
  });

  it("matches key presses to shortcuts", () => {
    const def = (id: string) => SHORTCUTS.find((s) => s.id === id)!;
    const key = (k: Partial<KeyboardEvent>) => ({ altKey: false, shiftKey: false, ctrlKey: false, metaKey: false, key: "", code: "", ...k });
    expect(matchesShortcut(key({ ctrlKey: true, key: "ArrowRight" }), def("next"))).toBe(true);
    expect(matchesShortcut(key({ ctrlKey: true, shiftKey: true, key: "ArrowRight" }), def("next"))).toBe(false);
    expect(matchesShortcut(key({ ctrlKey: true, key: ",", code: "Comma" }), def("settings"))).toBe(true);
    expect(matchesShortcut(key({ altKey: true, shiftKey: true, key: "Œ", code: "KeyQ" }), def("queue"))).toBe(true);
    expect(matchesShortcut(key({ ctrlKey: true, key: "/", code: "Slash" }), def("help"))).toBe(true);
  });

  it("Settings… opens Settings", async () => {
    render(<AppShell />);
    expect(route().name).not.toBe("settings");
    await act(async () => {
      await Promise.resolve();
      mock.emit("menu://action", { action: "settings" });
    });
    expect(route().name).toBe("settings");
  });

  it("Keyboard Shortcuts opens the shortcuts window", async () => {
    const stop = connectMenuBar(mock);
    await act(async () => {
      await Promise.resolve();
      mock.emit("menu://action", { action: "shortcuts" });
    });
    expect(useUi.getState().shortcutsOpen).toBe(true);
    stop();
  });

  it("a key the page already handled doesn't act twice through the menu", async () => {
    const next = vi.spyOn(player, "next").mockImplementation(() => Promise.resolve());
    let t = 1000;
    render(<Keys />);
    const stop = connectMenuBar(mock, () => t);
    await act(async () => Promise.resolve());

    // The page takes Ctrl/⌘+→, then the menu sees the same press.
    fireEvent.keyDown(document.body, { key: "ArrowRight", code: "ArrowRight", ctrlKey: true });
    expect(next).toHaveBeenCalledTimes(1);
    act(() => mock.emit("menu://action", { action: "next" }));
    expect(next).toHaveBeenCalledTimes(1);

    // Picked with the mouse later: it acts.
    t += 2000;
    act(() => mock.emit("menu://action", { action: "next" }));
    expect(next).toHaveBeenCalledTimes(2);
    stop();
  });

  it("a key the page let through (typing in a field) still works from the menu", async () => {
    const shuffle = vi.spyOn(player, "toggleShuffle").mockImplementation(() => Promise.resolve());
    render(
      <>
        <Keys />
        <input aria-label="field" />
      </>,
    );
    const stop = connectMenuBar(mock, () => 0);
    await act(async () => Promise.resolve());
    fireEvent.keyDown(screen.getByLabelText("field"), { key: "s", code: "KeyS", ctrlKey: true });
    expect(shuffle).not.toHaveBeenCalled();
    act(() => mock.emit("menu://action", { action: "shuffle" }));
    expect(shuffle).toHaveBeenCalledTimes(1);
    stop();
  });

  it("ignores actions it doesn't know", async () => {
    const stop = connectMenuBar(mock);
    await act(async () => Promise.resolve());
    expect(() => act(() => mock.emit("menu://action", { action: "nope" }))).not.toThrow();
    stop();
  });
});
