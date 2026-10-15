/** The mock's one server: a Plex server, with what Plex can do. */
import { ALL_CAPS } from "../caps";
import type { BackendCaps, Caps } from "../types";

/** Plex has no server-side saved queue, dated plays or radio Platter uses. */
export function plexCaps(): Caps {
  return { ...ALL_CAPS, resumeQueue: false, backdatedScrobble: false, serverRadio: false };
}

export function mockBackendCaps(name: string | null): BackendCaps {
  const caps = plexCaps();
  return { caps, servers: [{ tag: "0a1b2c3d", kind: "plex", name, primary: true, caps }] };
}
