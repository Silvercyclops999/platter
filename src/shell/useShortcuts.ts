import { useEffect } from "react";
import { isMod, platform } from "../lib/platform";
import { toggleLiked } from "../stores/likes";
import { useSettings } from "../stores/settings";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { createPlaylist } from "../stores/playlists";
import { bookmarkNowPlaying } from "../stores/curate";
import { useUi } from "../stores/ui";
import { keyRight, keySidebar, rightRoom, sidebarRoom, solveLayout } from "./layout";

function isTyping(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  return el.isContentEditable || el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT";
}

/** Volume change per Ctrl/⌘+↑/↓: 1/16 of the range. */
export const VOLUME_KEY_STEP = 1 / 16;
/** Seek per Shift+←/→. */
export const SEEK_SHORTCUT_MS = 5_000;

/** Like or unlike the playing song: Alt+Shift+B (Option+Shift+B on a Mac). */
export const LIKE_SHORTCUT = platform === "mac" ? "⌥⇧B" : "Alt+Shift+B";

/** The like shortcut's keys (by key position, so Option's special letters on a Mac still count). */
export function isLikeShortcut(e: Pick<KeyboardEvent, "altKey" | "shiftKey" | "ctrlKey" | "metaKey" | "key" | "code">): boolean {
  return e.altKey && e.shiftKey && !e.ctrlKey && !e.metaKey && (e.code === "KeyB" || e.key.toLowerCase() === "b");
}

/** Steps of the resize handles' keyboard step per layout shortcut press. */
const RESIZE_STEPS = 2;

/** The column widths in use right now (the shell derives the same). */
function currentLayout() {
  const ui = useUi.getState();
  const rightOpen = ui.queueOpen || ui.nowPlayingOpen || ui.outputOpen;
  const layout = solveLayout({
    windowWidth: window.innerWidth,
    sidebarWidth: ui.sidebarWidth,
    rightWidth: ui.rightWidth,
    rightOpen,
    sidebarCollapsed: ui.sidebarCollapsed,
  });
  return { ui, layout, rightOpen };
}

/** Collapse Your Library to the rail, or open it again (to the wide library when there's no room). */
export function toggleSidebar() {
  const { ui, layout } = currentLayout();
  if (layout.sidebarAuto) return nav.go({ name: "library" });
  ui.setSidebar({ collapsed: !ui.sidebarCollapsed });
}

/** Step the sidebar narrower (Left) or wider (Right), as its handle does. */
export function resizeSidebar(key: "ArrowLeft" | "ArrowRight") {
  const { ui, layout } = currentLayout();
  if (layout.sidebarAuto) return;
  let cur = { width: layout.sidebar, rail: layout.sidebarMode === "rail" };
  const room = sidebarRoom(window.innerWidth, layout.right);
  for (let i = 0; i < RESIZE_STEPS; i++) cur = keySidebar(cur, key, room) ?? cur;
  ui.setSidebar(cur.rail ? { collapsed: true } : { collapsed: false, width: cur.width });
}

/** Step the open right panel narrower or wider. */
export function resizeRight(narrower: boolean) {
  const { ui, layout, rightOpen } = currentLayout();
  if (!rightOpen) return;
  let w = layout.right;
  const room = rightRoom(window.innerWidth, layout.sidebar);
  for (let i = 0; i < RESIZE_STEPS; i++) w = keyRight(w, narrower ? "ArrowRight" : "ArrowLeft", room) ?? w;
  ui.setRightWidth(w);
}

/** Close whatever is in the right slot, or open the now-playing view. */
export function toggleRightPanel() {
  const ui = useUi.getState();
  if (ui.queueOpen || ui.nowPlayingOpen || ui.outputOpen) {
    useUi.setState({ queueOpen: false, nowPlayingOpen: false, outputOpen: false, outputReturn: null });
    return;
  }
  ui.setNowPlayingOpen(true);
}

/** Open the library filter: in the sidebar, or on the wide library page when the sidebar is a rail. */
export function filterLibrary() {
  const { ui, layout } = currentLayout();
  if (layout.sidebarMode === "rail") {
    if (layout.sidebarAuto) nav.go({ name: "library" });
    else ui.setSidebar({ collapsed: false });
  }
  // After the field has rendered.
  requestAnimationFrame(() => useUi.getState().focusLibraryFilter());
}

/**
 * Alt+Shift (Option+Shift on a Mac) + a key: navigation and layout. Matched
 * by key position, so Option's special characters on a Mac still count.
 */
const ALT_SHIFT: Record<string, () => void> = {
  KeyH: () => nav.go({ name: "home" }),
  KeyS: () => nav.go({ name: "liked" }),
  Digit0: () => nav.go({ name: "library" }),
  Digit1: () => nav.go({ name: "library", tab: "playlists" }),
  Digit2: () => nav.go({ name: "library", tab: "artists" }),
  Digit3: () => nav.go({ name: "library", tab: "albums" }),
  KeyQ: () => useUi.getState().toggleQueue(),
  KeyN: () => useUi.getState().toggleNowPlaying(),
  KeyD: () => useUi.getState().toggleOutput(),
  KeyP: () => void createPlaylist(),
  KeyM: bookmarkNowPlaying,
  KeyL: toggleSidebar,
  KeyR: toggleRightPanel,
  ArrowLeft: () => resizeSidebar("ArrowLeft"),
  ArrowRight: () => resizeSidebar("ArrowRight"),
  BracketRight: () => resizeRight(true),
  BracketLeft: () => resizeRight(false),
};

/** Held down, these repeat; the rest fire once per press. */
const REPEATS = new Set(["ArrowLeft", "ArrowRight", "BracketLeft", "BracketRight"]);

/** The Alt+Shift action's key for an event, if any (by code, falling back to the key for arrows). */
export function altShiftCode(e: Pick<KeyboardEvent, "altKey" | "shiftKey" | "ctrlKey" | "metaKey" | "key" | "code">): string | null {
  if (!e.altKey || !e.shiftKey || e.ctrlKey || e.metaKey) return null;
  if (ALT_SHIFT[e.code]) return e.code;
  if (e.key === "ArrowLeft" || e.key === "ArrowRight") return e.key;
  return null;
}

/**
 * Global keyboard + mouse shortcuts. The full list, with the keys shown per
 * platform, is in shortcuts.ts (the shortcuts window reads it).
 */
export function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const ui = useUi.getState();
      const key = e.key.toLowerCase();
      if (isMod(e) && !e.altKey && (e.code === "Slash" || e.key === "/")) {
        e.preventDefault();
        ui.setPaletteOpen(false);
        ui.setShortcutsOpen(!ui.shortcutsOpen);
        return;
      }
      if (isMod(e) && !e.shiftKey && !e.altKey && key === "k") {
        e.preventDefault();
        ui.setPaletteOpen(!ui.paletteOpen);
        return;
      }
      if (isMod(e) && !e.shiftKey && !e.altKey && key === "l") {
        e.preventDefault();
        ui.setPaletteOpen(false);
        ui.focusSearch();
        return;
      }
      if (isMod(e) && !e.shiftKey && !e.altKey && e.key === ",") {
        e.preventDefault();
        ui.setPaletteOpen(false);
        nav.go({ name: "settings" });
        return;
      }
      if (e.altKey && !e.shiftKey && !isMod(e) && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
        e.preventDefault();
        if (e.key === "ArrowLeft") nav.back();
        else nav.forward();
        return;
      }
      // ⌘[ and ⌘] on a Mac, as in other Mac apps.
      if (platform === "mac" && e.metaKey && !e.shiftKey && !e.altKey && (e.key === "[" || e.key === "]")) {
        e.preventDefault();
        if (e.key === "[") nav.back();
        else nav.forward();
        return;
      }
      if (isMod(e) && !e.shiftKey && !e.altKey && key === "f") {
        e.preventDefault();
        ui.setPaletteOpen(false);
        filterLibrary();
        return;
      }
      // Ctrl/⌘+R would reload the window; it toggles repeat instead.
      if (isMod(e) && !e.shiftKey && !e.altKey && key === "r") e.preventDefault();
      if (ui.paletteOpen || ui.menu) return;
      if (isTyping(e.target)) return;
      if (isMod(e) && !e.shiftKey && !e.altKey && key === "s") {
        e.preventDefault();
        player.toggleShuffle(!!useSettings.getState().settings?.smartShuffle);
        return;
      }
      if (isMod(e) && !e.shiftKey && !e.altKey && key === "r") {
        if (!e.repeat) player.cycleRepeatNow();
        return;
      }
      if (isMod(e) && !e.shiftKey && !e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
        e.preventDefault();
        player.nudgeVolume(e.key === "ArrowUp" ? VOLUME_KEY_STEP : -VOLUME_KEY_STEP);
        return;
      }
      if (isMod(e) && e.key === "ArrowRight") {
        e.preventDefault();
        player.next();
        return;
      }
      if (isMod(e) && e.key === "ArrowLeft") {
        e.preventDefault();
        player.previous();
        return;
      }
      const altShift = isLikeShortcut(e) ? null : altShiftCode(e);
      if (altShift) {
        e.preventDefault();
        if (!e.repeat || REPEATS.has(altShift)) ALT_SHIFT[altShift]();
        return;
      }
      if (isLikeShortcut(e)) {
        e.preventDefault();
        const track = usePlayer.getState().state?.track;
        if (track && !e.repeat) void toggleLiked([track]);
        return;
      }
      if (e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
        e.preventDefault();
        player.seekBy(e.key === "ArrowRight" ? SEEK_SHORTCUT_MS : -SEEK_SHORTCUT_MS);
        return;
      }
      if (e.key === " " && !e.repeat) {
        // Don't steal Space from buttons/sliders/rows that use it.
        const t = e.target as HTMLElement | null;
        // Track rows don't use Space, so it still plays/pauses with a row selected.
        if (t && t !== document.body && t.closest("button, a, [role=slider], [role=tab], [role=option], [role=checkbox], [role=switch]")) return;
        e.preventDefault();
        player.toggle();
      }
    };
    const onMouse = (e: MouseEvent) => {
      if (e.button === 3) {
        e.preventDefault();
        nav.back();
      } else if (e.button === 4) {
        e.preventDefault();
        nav.forward();
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mouseup", onMouse);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onMouse);
    };
  }, []);
}
