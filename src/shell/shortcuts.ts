/**
 * Every keyboard shortcut Platter has, for the shortcuts window and menu
 * hints. The handler lives in useShortcuts.ts; tests check the two agree.
 *
 * Keys are written once with "Mod" (Ctrl on Windows, ⌘ on a Mac), "Alt"
 * (Option on a Mac) and "Shift", and shown with the right names per platform.
 */
import { platform as currentPlatform, type Platform } from "../lib/platform";

export type ShortcutSection = "basic" | "playback" | "navigation" | "layout" | "power";

export const SECTION_TITLES: Record<ShortcutSection, string> = {
  basic: "Basic",
  playback: "Playback",
  navigation: "Navigation",
  layout: "Layout",
  power: "Keyboard power mode",
};

export interface ShortcutDef {
  id: string;
  section: ShortcutSection;
  label: string;
  /** Keys pressed together, e.g. ["Mod", "K"]. */
  keys: string[];
  /** Different keys on a Mac, when they differ beyond the key names. */
  mac?: string[];
}

export const SHORTCUTS: ShortcutDef[] = [
  // Basic
  { id: "help", section: "basic", label: "Show keyboard shortcuts", keys: ["Mod", "/"] },
  { id: "palette", section: "basic", label: "Quick search and commands", keys: ["Mod", "K"] },
  { id: "createPlaylist", section: "basic", label: "Create playlist", keys: ["Alt", "Shift", "P"] },
  { id: "createSmartPlaylist", section: "basic", label: "Create smart playlist", keys: ["Alt", "Shift", "O"] },
  { id: "filterLibrary", section: "basic", label: "Find in this list, or filter Your Library", keys: ["Mod", "F"] },
  { id: "selectAll", section: "basic", label: "Select all songs in a list", keys: ["Mod", "A"] },
  { id: "menu", section: "basic", label: "Open the menu for the focused item", keys: ["Shift", "F10"] },
  { id: "playFocused", section: "basic", label: "Play the focused song", keys: ["Enter"] },
  { id: "queueFromSearch", section: "basic", label: "Add the highlighted song in search to the queue", keys: ["Shift", "Enter"] },
  { id: "close", section: "basic", label: "Close a menu, dialog or view", keys: ["Esc"] },
  // Playback
  { id: "playPause", section: "playback", label: "Play or pause", keys: ["Space"] },
  { id: "like", section: "playback", label: "Like or unlike the current song", keys: ["Alt", "Shift", "B"] },
  { id: "bookmark", section: "playback", label: "Bookmark the current song at this moment", keys: ["Alt", "Shift", "M"] },
  { id: "shuffle", section: "playback", label: "Shuffle", keys: ["Mod", "S"] },
  { id: "repeat", section: "playback", label: "Repeat", keys: ["Mod", "R"] },
  { id: "previous", section: "playback", label: "Previous", keys: ["Mod", "←"] },
  { id: "next", section: "playback", label: "Next", keys: ["Mod", "→"] },
  { id: "seekBack", section: "playback", label: "Seek back 5 seconds", keys: ["Shift", "←"] },
  { id: "seekForward", section: "playback", label: "Seek forward 5 seconds", keys: ["Shift", "→"] },
  { id: "volumeUp", section: "playback", label: "Volume up", keys: ["Mod", "↑"] },
  { id: "volumeDown", section: "playback", label: "Volume down", keys: ["Mod", "↓"] },
  // Navigation
  { id: "back", section: "navigation", label: "Go back", keys: ["Alt", "←"], mac: ["Mod", "["] },
  { id: "forward", section: "navigation", label: "Go forward", keys: ["Alt", "→"], mac: ["Mod", "]"] },
  { id: "home", section: "navigation", label: "Home", keys: ["Alt", "Shift", "H"] },
  { id: "search", section: "navigation", label: "Search", keys: ["Mod", "L"] },
  { id: "liked", section: "navigation", label: "Liked Songs", keys: ["Alt", "Shift", "S"] },
  { id: "library", section: "navigation", label: "Your Library", keys: ["Alt", "Shift", "0"] },
  { id: "playlists", section: "navigation", label: "Playlists", keys: ["Alt", "Shift", "1"] },
  { id: "artists", section: "navigation", label: "Artists", keys: ["Alt", "Shift", "2"] },
  { id: "albums", section: "navigation", label: "Albums", keys: ["Alt", "Shift", "3"] },
  { id: "queue", section: "navigation", label: "Queue", keys: ["Alt", "Shift", "Q"] },
  { id: "nowPlaying", section: "navigation", label: "Now playing view", keys: ["Alt", "Shift", "N"] },
  { id: "devices", section: "navigation", label: "Connect to a device", keys: ["Alt", "Shift", "D"] },
  { id: "goToPlaying", section: "navigation", label: "Go to the playing song", keys: ["Mod", "Shift", "J"] },
  { id: "settings", section: "navigation", label: "Settings", keys: ["Mod", ","] },
  // Layout
  { id: "toggleSidebar", section: "layout", label: "Collapse or expand Your Library", keys: ["Alt", "Shift", "L"] },
  { id: "sidebarNarrower", section: "layout", label: "Make Your Library narrower", keys: ["Alt", "Shift", "←"] },
  { id: "sidebarWider", section: "layout", label: "Make Your Library wider", keys: ["Alt", "Shift", "→"] },
  { id: "toggleRight", section: "layout", label: "Show or hide the right panel", keys: ["Alt", "Shift", "R"] },
  { id: "rightNarrower", section: "layout", label: "Make the right panel narrower", keys: ["Alt", "Shift", "]"] },
  { id: "rightWider", section: "layout", label: "Make the right panel wider", keys: ["Alt", "Shift", "["] },
  // Keyboard power mode (Settings, off by default). The hint key is the one chosen there.
  { id: "powerHints", section: "power", label: "Show jump hints, then type a hint's letters", keys: ["F"] },
  { id: "powerDown", section: "power", label: "Select the next song in the list", keys: ["J"] },
  { id: "powerUp", section: "power", label: "Select the previous song in the list", keys: ["K"] },
  { id: "powerTop", section: "power", label: "Jump to the top of the list", keys: ["G", "G"] },
  { id: "powerBottom", section: "power", label: "Jump to the bottom of the list", keys: ["Shift", "G"] },
  { id: "powerSearch", section: "power", label: "Search", keys: ["/"] },
];

export type ShortcutId = (typeof SHORTCUTS)[number]["id"];

const NAMES: Record<"windows" | "mac", Record<string, string>> = {
  windows: { Mod: "Ctrl", Alt: "Alt", Shift: "Shift" },
  mac: { Mod: "⌘", Alt: "⌥", Shift: "⇧", Enter: "Return" },
};

/** The key caps for a shortcut on a platform ("other" shows the Windows names). */
export function shortcutKeys(def: ShortcutDef, p: Platform = currentPlatform): string[] {
  const mac = p === "mac";
  const names = NAMES[mac ? "mac" : "windows"];
  return (mac && def.mac ? def.mac : def.keys).map((k) => names[k] ?? k);
}

/** One line for menus and tooltips: "Ctrl+/" on Windows, "⌘/" on a Mac. */
export function shortcutLabel(id: string, p: Platform = currentPlatform): string {
  const def = SHORTCUTS.find((s) => s.id === id);
  if (!def) return "";
  return shortcutKeys(def, p).join(p === "mac" ? "" : "+");
}

/** The hint key as a key cap ("F", ";"). */
export function hintKeyCap(key: string | undefined): string {
  return (key || "f").toUpperCase();
}

export function shortcutsIn(section: ShortcutSection): ShortcutDef[] {
  return SHORTCUTS.filter((s) => s.section === section);
}
