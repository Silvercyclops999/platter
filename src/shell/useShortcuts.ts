import { useEffect } from "react";
import { isMod } from "../lib/platform";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { useUi } from "../stores/ui";

function isTyping(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  return el.isContentEditable || el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT";
}

/** Volume change per Ctrl/⌘+↑/↓. */
export const VOLUME_KEY_STEP = 0.1;

/**
 * Global keyboard + mouse shortcuts:
 *   Ctrl/⌘+K command palette · Ctrl/⌘+L search · Ctrl/⌘+, settings ·
 *   Alt+←/→ back/forward · mouse back/forward buttons ·
 *   Space play/pause · Ctrl/⌘+←/→ prev/next · Ctrl/⌘+↑/↓ volume ·
 *   Ctrl/⌘+S shuffle · Ctrl/⌘+R repeat.
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
      if (isMod(e) && !e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
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
