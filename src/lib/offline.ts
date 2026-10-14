/**
 * Offline means Platter can't reach the Plex server (no network, server
 * down, or blocked). Music stored on this computer keeps playing; the rest
 * waits. Wording here matches the core (src-tauri/src/audio/player.rs and
 * src-tauri/src/bridge.rs).
 */
import type { Health } from "./types";

/** Play refused because none of the songs are on this computer. */
export const OFFLINE_NOTHING_PLAYS = "These songs aren't on this computer, so they can't play until Platter can reach your server.";

/** The one notice when a play leaves songs out while offline. */
export function offlineLeftOutMessage(n: number): string {
  const songs = n === 1 ? "1 song that isn't" : `${n} songs that aren't`;
  return `Skipped ${songs} on this computer. They play again when Platter can reach your server.`;
}

/** Queued songs failing one after another while offline, said once. */
export const OFFLINE_SKIPPED = "Skipped songs that aren't on this computer. They play again when Platter can reach your server.";

/** Search while offline, for results that need the server. */
export const OFFLINE_SEARCH = "You're offline. Search works again when Platter can reach your server.";

/** Signed in but the server can't be reached over any route. */
export function isOffline(h: Health | null | undefined): boolean {
  return !!h && h.status === "offline";
}
