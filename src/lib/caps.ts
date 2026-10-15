/** Server capability names and how an item's server is found from its id. */
import type { BackendCaps, CapName, Caps, ServerCaps } from "./types";

export const CAP_NAMES: readonly CapName[] = [
  "liveUpdates",
  "incrementalSync",
  "likeDates",
  "starRatings",
  "playlistMove",
  "playlistPoster",
  "playlistSummary",
  "serverSmartPlaylists",
  "transcode",
  "lyrics",
  "syncedLyrics",
  "serverRadio",
  "sonic",
  "playQueueHandoff",
  "resumeQueue",
  "remotePlayers",
  "friendActivity",
  "friendHistory",
  "metadataEdit",
  "homeUsers",
  "realFilePaths",
  "checkFiles",
  "loudness",
  "releaseTypes",
  "serverHistory",
  "backdatedScrobble",
  "trackDetails",
];

/** Every capability on: what the UI assumes until the core says otherwise. */
export const ALL_CAPS: Caps = Object.fromEntries(CAP_NAMES.map((c) => [c, true])) as Caps;

/** The server tag an id or image path starts with (`0a1b2c3d:123`), if any. */
export function tagOf(id: string): string | null {
  const m = /^([0-9a-f]{8}):/.exec(id);
  return m ? m[1] : null;
}

/** The server an id belongs to: the one its tag names, else the primary. */
export function serverFor(all: BackendCaps, id: string | null | undefined): ServerCaps | null {
  const tag = id ? tagOf(id) : null;
  const named = tag ? all.servers.find((s) => s.tag === tag) : undefined;
  return named ?? all.servers.find((s) => s.primary) ?? null;
}

/**
 * Whether the server with this item can do `cap`. An id with a tag no
 * server claims belongs to the primary (a Plex account covers all its
 * servers). With no servers listed, what any server can do.
 */
export function capFor(all: BackendCaps, id: string | null | undefined, cap: CapName): boolean {
  const s = serverFor(all, id);
  return s ? s.caps[cap] : all.caps[cap];
}
