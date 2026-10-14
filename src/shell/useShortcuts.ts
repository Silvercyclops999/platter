import { useEffect } from "react";
import { isMod, platform } from "../lib/platform";
import { toggleLiked } from "../stores/likes";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useUi } from "../stores/ui";

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

/**
 * Global keyboard + mouse shortcuts:
 *   Ctrl/⌘+K command palette · Ctrl/⌘+L search · Ctrl/⌘+, settings ·
 *   Alt+←/→ back/forward · mouse back/forward buttons ·
 *   Space play/pause · Ctrl/⌘+←/→ prev/next · Shift+←/→ seek 5 s ·
 *   Ctrl/⌘+↑/↓ volume (1/16 steps) ·
 *   Ctrl/⌘+S shuffle · Ctrl/⌘+R repeat ·
 *   Alt+Shift+B like / unlike the playing song.
 */
export function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const ui = useUi.getState();
      const key = e.key.toLowerCase();
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
      if (e.altKey && !isMod(e) && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
        e.preventDefault();
        if (e.key === "ArrowLeft") nav.back();
        else nav.forward();
        return;
      }
      // Ctrl/⌘+R would reload the window; it toggles repeat instead.
      if (isMod(e) && !e.shiftKey && !e.altKey && key === "r") e.preventDefault();
      if (ui.paletteOpen || ui.menu) return;
      if (isTyping(e.target)) return;
      if (isMod(e) && !e.shiftKey && !e.altKey && key === "s") {
        e.preventDefault();
        player.toggleShuffle();
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
