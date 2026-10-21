/**
 * The macOS menu bar's items, from the UI's side. The core builds the menus
 * and sends `menu://action` with an action name; this runs the same code the
 * app's own buttons and shortcuts run.
 *
 * Items with keys share them with the app's shortcuts. When the page has
 * already handled the key press, the menu's copy of it is skipped, so a key
 * never acts twice whichever of the two sees it first.
 */
import { backend, type Backend } from "../lib/backend";
import { isMod } from "../lib/platform";
import { player } from "../stores/player";
import { createPlaylist } from "../stores/playlists";
import { folders } from "../stores/folders";
import { fullWindow } from "../stores/fullWindow";
import { miniplayer } from "../stores/miniplayer";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { SHORTCUTS, type ShortcutDef } from "./shortcuts";
import { checkForUpdates } from "./TopBar";
import { filterLibrary, toggleSidebar, VOLUME_KEY_STEP } from "./useShortcuts";
import { openPageFind } from "./pageFind";

export interface MenuAction {
  run: () => void;
  /** The app shortcut with the same keys, if any. */
  shortcut?: string;
}

export const MENU_ACTIONS: Record<string, MenuAction> = {
  checkUpdates: { run: checkForUpdates },
  settings: {
    shortcut: "settings",
    run: () => {
      useUi.getState().setPaletteOpen(false);
      nav.go({ name: "settings" });
    },
  },
  newPlaylist: { run: () => void createPlaylist() },
  newFolder: { run: () => void folders.create() },
  /** The same switch as Settings' (from the menu bar or the tray); the core moves the check. */
  goOffline: { run: () => void updateSettings({ goOffline: !useSettings.getState().settings?.goOffline }) },
  signOut: {
    run: () => {
      useUi.getState().setPaletteOpen(false);
      useUi.getState().setSignOutOpen(true);
    },
  },
  search: {
    shortcut: "search",
    run: () => {
      useUi.getState().setPaletteOpen(false);
      useUi.getState().focusSearch();
    },
  },
  filterLibrary: {
    shortcut: "filterLibrary",
    run: () => {
      useUi.getState().setPaletteOpen(false);
      if (!openPageFind()) filterLibrary();
    },
  },
  playPause: { run: () => player.toggle() },
  next: { shortcut: "next", run: () => player.next() },
  previous: { shortcut: "previous", run: () => player.previous() },
  volumeUp: { shortcut: "volumeUp", run: () => void player.nudgeVolume(VOLUME_KEY_STEP) },
  volumeDown: { shortcut: "volumeDown", run: () => void player.nudgeVolume(-VOLUME_KEY_STEP) },
  shuffle: { shortcut: "shuffle", run: () => player.toggleShuffle(!!useSettings.getState().settings?.smartShuffle) },
  repeat: { shortcut: "repeat", run: () => player.cycleRepeatNow() },
  shuffleAlbums: { run: () => void player.shuffleAlbums() },
  miniplayer: { run: () => void miniplayer.toggle() },
  fullWindow: { run: () => fullWindow.toggle() },
  lyrics: { run: () => fullWindow.toggleLyrics() },
  queue: { shortcut: "queue", run: () => useUi.getState().toggleQueue() },
  toggleLibrary: { shortcut: "toggleSidebar", run: toggleSidebar },
  shortcuts: {
    shortcut: "help",
    run: () => {
      useUi.getState().setPaletteOpen(false);
      useUi.getState().setShortcutsOpen(true);
    },
  },
};

type KeyLike = Pick<KeyboardEvent, "altKey" | "shiftKey" | "ctrlKey" | "metaKey" | "key" | "code">;

const ARROWS: Record<string, string> = { "←": "ArrowLeft", "→": "ArrowRight", "↑": "ArrowUp", "↓": "ArrowDown" };
const CODES: Record<string, string> = { "/": "Slash", ",": "Comma", "[": "BracketLeft", "]": "BracketRight" };

/** Whether a key press is a shortcut's keys (the Mac ones where they differ). */
export function matchesShortcut(e: KeyLike, def: ShortcutDef, mac = true): boolean {
  const keys = mac && def.mac ? def.mac : def.keys;
  const mods = new Set<string>(keys.filter((k) => k === "Mod" || k === "Alt" || k === "Shift"));
  const main = keys.filter((k) => !mods.has(k));
  if (main.length !== 1) return false;
  if (isMod(e) !== mods.has("Mod") || e.altKey !== mods.has("Alt") || e.shiftKey !== mods.has("Shift")) return false;
  const k = main[0];
  if (ARROWS[k]) return e.key === ARROWS[k];
  if (CODES[k]) return e.code === CODES[k] || e.key === k;
  if (/^[A-Z0-9]$/.test(k)) return e.code === (/\d/.test(k) ? `Digit${k}` : `Key${k}`) || e.key.toUpperCase() === k;
  return e.key === k;
}

/** How close the page's key press must be to count as the same press. */
const SAME_PRESS_MS = 500;

/**
 * Listen for menu bar picks. Returns a cleanup. `now` is for tests.
 */
export function connectMenuBar(b: Backend = backend, now: () => number = () => performance.now()): () => void {
  let alive = true;
  let un: (() => void) | null = null;
  let last: { e: KeyboardEvent; at: number } | null = null;
  const onKey = (e: KeyboardEvent) => {
    last = { e, at: now() };
  };
  // Capture, so it's seen before any handler stops it.
  window.addEventListener("keydown", onKey, true);

  const handledByPage = (a: MenuAction): boolean => {
    if (!a.shortcut || !last || now() - last.at > SAME_PRESS_MS) return false;
    const def = SHORTCUTS.find((s) => s.id === a.shortcut);
    return !!def && last.e.defaultPrevented && matchesShortcut(last.e, def);
  };

  b.listen("menu://action", ({ action }) => {
    const a = MENU_ACTIONS[action];
    if (!a || handledByPage(a)) return;
    last = null;
    a.run();
  }).then((u) => (alive ? (un = u) : u()));

  return () => {
    alive = false;
    window.removeEventListener("keydown", onKey, true);
    un?.();
  };
}
