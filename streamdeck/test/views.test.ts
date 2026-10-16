import { describe, expect, it } from "vitest";
import type { LibraryItem, PlatterState } from "../src/client.js";
import type { Snapshot } from "../src/live.js";
import { C, svgOf } from "../src/render.js";
import { DEFAULTS, settingsFor } from "../src/settings.js";
import { infoFields, KEY_ACTIONS, type KeyActionId, keyView, sleepLeft, type ViewCtx } from "../src/views.js";
import { idle, snapOf, state } from "./fixtures.js";

const COVERS: Record<string, string> = { k2: "data:image/png;base64,NEXT", k0: "data:image/png;base64,PREV", pc: "data:image/png;base64,PLAYLIST" };
const ITEMS: LibraryItem[] = [{ id: "p1", title: "Road Trip", subtitle: "12 songs", coverKey: "pc" }];

function ctx(snap: Snapshot, over: Partial<ViewCtx> = {}): ViewCtx {
  return { snap, now: snap.at, cover: (k) => (k ? (COVERS[k] ?? null) : null), item: (_kind, id) => ITEMS.find((i) => i.id === id), ...over };
}

/** A key's picture (as SVG text) and title. */
function show(action: KeyActionId, s: PlatterState | null, settings: Record<string, unknown> = {}, over: Partial<Snapshot> = {}, more: Partial<ViewCtx> = {}) {
  const st = action in DEFAULTS ? settingsFor(action as keyof typeof DEFAULTS, settings) : { images: settings.images };
  const v = keyView(action, st as Record<string, unknown>, ctx(snapOf(s, over), more));
  return { svg: svgOf(v.image), title: v.title };
}

const on = (svg: string) => svg.includes(C.accent);
const greyed = (svg: string) => svg.includes(C.faint);

describe("every key", () => {
  it("draws something for every state without throwing", () => {
    const states: [PlatterState | null, Partial<Snapshot>][] = [
      [state(), {}],
      [idle(), {}],
      [null, { conn: "offline" }],
      [null, { conn: "connecting" }],
      [null, { conn: "unpaired" }],
      [null, { conn: "approve" }],
      [null, { conn: "refused" }],
      [state({ private: true, title: null, trackId: null }), {}],
      [state(), { held: true }],
    ];
    for (const a of KEY_ACTIONS) {
      for (const [s, o] of states) {
        const v = show(a, s, {}, o);
        expect(v.svg.startsWith("<svg"), a).toBe(true);
        expect(v.svg).not.toContain("undefined");
        expect(v.svg).not.toContain("NaN");
      }
    }
  });

  it("Platter not running greys every key and says so", () => {
    for (const a of KEY_ACTIONS) {
      const v = show(a, null, {}, { conn: "offline" });
      expect(v.title, a).toBe("Platter\nnot running");
      expect(greyed(v.svg), a).toBe(true);
    }
  });

  it("not paired and not allowed put a badge over the usual picture", () => {
    for (const a of KEY_ACTIONS) {
      expect(show(a, null, {}, { conn: "unpaired" }).svg, a).toContain(C.warn);
      expect(show(a, null, {}, { conn: "refused" }).svg, a).toContain(C.warn);
      expect(show(a, state()).svg, a).not.toContain(C.warn);
    }
  });

  it("keys that need a song grey out when nothing plays", () => {
    for (const a of ["like", "seekback", "seekforward", "radio", "bookmark", "request", "rate", "addtoplaylist", "removefromplaylist"] as const) {
      const v = show(a, idle(), a === "removefromplaylist" ? { playlist: "p1" } : {});
      expect(v.title, a).toBe("Nothing\nplaying");
      expect(greyed(v.svg), a).toBe(true);
    }
  });

  it("uses the user's own picture for a state", () => {
    const pic = "data:image/png;base64,MINE";
    expect(show("next", state(), { images: { normal: pic } }).svg).toContain(pic);
    expect(show("shuffle", state({ shuffle: true }), { images: { on: pic } }).svg).toContain(pic);
    expect(show("shuffle", state({ shuffle: false }), { images: { on: pic } }).svg).not.toContain(pic);
    expect(show("like", state({ liked: true }), { images: { liked: pic } }).svg).toContain(pic);
    expect(show("playpause", state({ playing: false }), { images: { paused: pic } }).svg).toContain(pic);
    expect(show("playpause", state(), { images: { paused: pic } }).svg).not.toContain(pic);
  });
});

describe("Play / Pause", () => {
  it("the cover fills the key with no text while playing", () => {
    const v = show("playpause", state());
    expect(v.svg).toContain("data:image/jpeg;base64,AAA");
    expect(v.svg).not.toContain("<text");
    expect(v.svg).not.toContain("<circle");
    expect(v.title).toBe("");
  });

  it("paused: a see-through circle with a play symbol, or a small label", () => {
    const v = show("playpause", state({ playing: false }));
    expect(v.svg).toMatch(/<circle[^>]*fill-opacity="0.5"/);
    expect(v.svg).not.toContain("<text");
    expect(show("playpause", state({ playing: false }), { pausedLook: "label" }).svg).toContain(">Paused<");
  });

  it("the time band: elapsed, remaining or off", () => {
    expect(show("playpause", state(), { time: "elapsed" }).svg).toContain(">1:00<");
    expect(show("playpause", state(), { time: "remaining" }).svg).toContain(">-2:20<");
    expect(show("playpause", state(), { time: "elapsed" }, {}, { now: 4000 }).svg, "ticks on").toContain(">1:03<");
    expect(show("playpause", state(), { showTitle: true }).svg).toContain("Night Drive");
  });

  it("idle shows a play symbol; the last song stays (paused) while held", () => {
    expect(show("playpause", idle()).svg).not.toContain("base64");
    const held = show("playpause", state({ playing: false }), {}, { held: true });
    expect(held.svg).toContain("data:image/jpeg;base64,AAA");
  });

  it("a group key shows its part of the big cover", () => {
    const v = show("playpause", state(), {}, {}, { slice: { size: 2, col: 1, row: 0 } });
    expect(v.svg).toMatch(/<image x="-144" y="0" width="288" height="288"/);
    // The paused circle sits once, on the corner the four keys share.
    const p = show("playpause", state({ playing: false }), {}, {}, { slice: { size: 2, col: 0, row: 0 } });
    expect(p.svg).toContain('cx="144" cy="144"');
    expect(show("playpause", state({ playing: false }), {}, {}, { slice: { size: 2, col: 1, row: 1 } }).svg).not.toContain("<circle");
  });
});

describe("transport and modes", () => {
  it("previous and next can show the other song's cover", () => {
    expect(show("next", state()).svg).not.toContain("NEXT");
    expect(show("next", state(), { cover: true }).svg).toContain("NEXT");
    expect(show("previous", state(), { cover: true }).svg).toContain("PREV");
  });

  it("seek shows its seconds", () => {
    expect(show("seekback", state(), { seconds: 15 }).svg).toContain(">15 s<");
  });

  it("song info: title and artist over the dimmed cover, time and bar; idle says nothing playing", () => {
    const v = show("songinfo", state());
    expect(v.svg).toContain("Night Drive");
    expect(v.svg).toContain("Rue Ellis");
    expect(v.svg).toContain("1:00 / 3:20");
    expect(v.svg).toContain("AAA");
    const i = show("songinfo", idle());
    expect(i.svg).toContain("Nothing playing");
    expect(i.svg).toContain(">-<");
  });

  it("shuffle, repeat, crossfade and private show when they're on", () => {
    expect(on(show("shuffle", state({ shuffle: true })).svg)).toBe(true);
    expect(on(show("shuffle", state()).svg)).toBe(false);
    expect(show("shuffle", state({ smartShuffle: true, shuffle: true })).svg).toContain(">Smart<");
    expect(on(show("repeat", state()).svg)).toBe(false);
    expect(show("repeat", state({ repeat: "all" })).svg).toContain(">All<");
    expect(show("repeat", state({ repeat: "one" })).svg).toContain(">One<");
    expect(on(show("crossfade", state({ crossfade: true })).svg)).toBe(true);
    expect(on(show("private", state({ private: true })).svg)).toBe(true);
  });
});

describe("volume", () => {
  it("up and down can show the level as a bar or a number", () => {
    expect(show("volumeup", state({ volume: 40 })).svg).not.toContain(">40<");
    expect(show("volumeup", state({ volume: 40 }), { show: "number" }).svg).toContain(">40<");
    expect(show("volumedown", state({ volume: 40 }), { show: "bar" }).svg).toContain(`fill="${C.accent}"`);
  });

  it("mute shows muted; volume set lights up at its level", () => {
    expect(on(show("mute", state({ muted: true })).svg)).toBe(true);
    expect(on(show("mute", state()).svg)).toBe(false);
    expect(on(show("volumeset", state({ volume: 50 }), { level: 50 }).svg)).toBe(true);
    expect(show("volumeset", state({ volume: 20 }), { level: 50 }).svg).toContain(">50%<");
  });
});

describe("library keys", () => {
  it("like follows the song", () => {
    expect(on(show("like", state({ liked: true })).svg)).toBe(true);
    expect(on(show("like", state()).svg)).toBe(false);
  });

  it("add to playlist shows the playlist's cover and name, and whether the song is in it", () => {
    const v = show("addtoplaylist", state(), { playlist: "p1" });
    expect(v.svg).toContain("PLAYLIST");
    expect(v.svg).toContain("Road Trip");
    expect(on(v.svg)).toBe(false);
    expect(on(show("addtoplaylist", state({ playlists: ["p1"] }), { playlist: "p1" }).svg)).toBe(true);
    expect(show("addtoplaylist", state()).svg).toContain(">Ask<");
  });

  it("remove from playlist is greyed when the song isn't in it", () => {
    expect(show("removefromplaylist", state(), { playlist: "p1" }).title).toBe("Not in\nplaylist");
    expect(show("removefromplaylist", state({ playlists: ["p1"] }), { playlist: "p1" }).title).toBe("");
    expect(show("removefromplaylist", state()).title).toBe("Pick a\nplaylist");
  });

  it("rate shows its stars, lights up at the song's rating, greys without ratings", () => {
    expect(show("rate", state({ canRate: true }), { stars: 4 }).svg).toContain(">4 stars<");
    expect(on(show("rate", state({ canRate: true, stars: 4 }), { stars: 4 }).svg)).toBe(true);
    expect(show("rate", state({ canRate: false }), { stars: 4 }).title).toBe("No star\nratings");
  });
});

describe("play keys", () => {
  it("show the collection's cover, framed while it plays", () => {
    const v = show("playplaylist", state(), { id: "p1" });
    expect(v.svg).toContain("PLAYLIST");
    expect(on(v.svg)).toBe(false);
    expect(on(show("playplaylist", state({ context: { kind: "playlist", id: "p1" } }), { id: "p1" }).svg)).toBe(true);
    expect(show("playalbum", state()).title).toBe("Pick\none");
  });

  it("link and search ask to be set", () => {
    expect(show("playlink", state()).title).toBe("Set a\nlink");
    expect(show("playlink", state(), { link: "platter://album/1" }).title).toBe("");
    expect(show("playsearch", state(), { query: "night" }).svg).toContain(">night<");
    expect(show("playsearch", state()).title).toBe("Type a\nsearch");
  });

  it("liked songs lights up while it plays", () => {
    expect(on(show("likedsongs", state({ context: { kind: "liked", id: "liked" } })).svg)).toBe(true);
  });
});

describe("queue, devices and Platter", () => {
  it("queue shows the next cover and its title", () => {
    const v = show("queue", state());
    expect(v.svg).toContain("NEXT");
    expect(v.svg).toContain("Next: Shoreline");
    expect(show("queue", state({ next: null })).svg).toContain("Nothing next");
  });

  it("listen on shows the device and lights up when playing there", () => {
    const st = { device: "d1", deviceName: "Kitchen" };
    expect(show("listenon", state(), st).svg).toContain(">Kitchen<");
    expect(on(show("listenon", state({ device: "Kitchen" }), st).svg)).toBe(true);
    expect(on(show("listenon", state(), st).svg)).toBe(false);
    expect(on(show("listenon", state(), { device: "local", deviceName: "This computer" }).svg)).toBe(true);
  });

  it("lyrics, miniplayer and window follow Platter", () => {
    expect(on(show("lyrics", state({ lyrics: true })).svg)).toBe(true);
    expect(on(show("miniplayer", state({ miniplayer: true })).svg)).toBe(true);
    expect(on(show("window", state({ window: true })).svg)).toBe(true);
    expect(on(show("window", state()).svg)).toBe(false);
  });

  it("sleep shows its time, then the countdown while it runs", () => {
    expect(show("sleep", state(), { minutes: 45 }).svg).toContain(">45 min<");
    expect(show("sleep", state(), { endOfSong: true }).svg).toContain(">Song end<");
    const running = state({ sleep: { remainingMs: 125_000, endOfSong: false } });
    expect(show("sleep", running).svg).toContain(">2:05<");
    expect(show("sleep", running, {}, {}, { now: 6000 }).svg, "counts down between polls").toContain(">2:00<");
    expect(sleepLeft(snapOf(state({ sleep: { remainingMs: null, endOfSong: true } })), 0)).toBe("Song end");
  });

  it("request needs Lidarr", () => {
    expect(show("request", state({ canRequest: false })).title).toBe("Set up\nLidarr");
    expect(show("request", state()).title).toBe("");
  });

  it("song info text for overlays", () => {
    expect(infoFields(snapOf(state()), 1000)).toEqual({ title: "Night Drive", artist: "Rue Ellis", album: "Late", elapsed: "1:00", total: "3:20" });
    expect(infoFields(snapOf(idle()), 0)).toMatchObject({ title: "Nothing playing", artist: "-" });
  });
});
