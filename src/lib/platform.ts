import { isTauri } from "./backend";

export type Platform = "windows" | "mac" | "other";

export function detectPlatform(ua: string = typeof navigator !== "undefined" ? navigator.userAgent : ""): Platform {
  if (/Windows/i.test(ua)) return "windows";
  if (/Mac OS X|Macintosh/i.test(ua)) return "mac";
  return "other";
}

export const platform: Platform = detectPlatform();

/**
 * Window chrome mode:
 *  - "windows": undecorated window (tauri.windows.conf.json) → draw our own title bar
 *  - "mac": overlay title bar → leave room for the traffic lights
 *  - "none": plain browser → no chrome
 */
export const chrome: "windows" | "mac" | "none" = !isTauri() ? "none" : platform === "mac" ? "mac" : platform === "windows" ? "windows" : "none";

export const modKey = platform === "mac" ? "⌘" : "Ctrl";

/** Ctrl on Windows/Linux, ⌘ on macOS. */
export function isMod(e: { ctrlKey: boolean; metaKey: boolean }): boolean {
  return platform === "mac" ? e.metaKey : e.ctrlKey;
}
