import { normalize, type PlatterState } from "../src/client.js";
import type { Snapshot } from "../src/live.js";

/** A song playing: Night Drive by Rue Ellis, a minute into 3:20. */
export function state(over: Partial<PlatterState> = {}): PlatterState {
  return normalize({
    api: 1,
    playing: true,
    private: false,
    title: "Night Drive",
    artist: "Rue Ellis",
    album: "Late",
    trackId: "t1",
    liked: false,
    volume: 50,
    muted: false,
    coverKey: "k1",
    cover: "data:image/jpeg;base64,AAA",
    canRequest: true,
    positionMs: 60_000,
    durationMs: 200_000,
    next: { title: "Shoreline", artist: "Ana Vale", coverKey: "k2" },
    previous: { title: "Early", artist: "Old Band", coverKey: "k0" },
    ...over,
  });
}

/** Nothing loaded. */
export const idle = (over: Partial<PlatterState> = {}): PlatterState =>
  state({ title: null, artist: null, album: null, trackId: null, coverKey: null, cover: null, durationMs: 0, positionMs: 0, playing: false, next: null, previous: null, ...over });

export const snapOf = (s: PlatterState | null, over: Partial<Snapshot> = {}): Snapshot => ({ conn: "ok", state: s, held: false, at: 1000, ...over });
