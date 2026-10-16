import { describe, expect, it } from "vitest";
import { dialImage, dialPress, dialTurn, type DialLists, newDial } from "../src/dials.js";
import { C, svgOf } from "../src/render.js";
import { settingsFor } from "../src/settings.js";
import { idle, snapOf, state } from "./fixtures.js";

const lists: DialLists = {
  browse: [
    { id: "p1", title: "Road Trip", subtitle: null, coverKey: "c1" },
    { id: "p2", title: "Gym", subtitle: null, coverKey: null },
  ],
  queue: [
    { itemId: "i1", title: "Shoreline", artist: "Ana Vale", coverKey: null },
    { itemId: "i2", title: "Harbour", artist: "Ana Vale", coverKey: null },
  ],
  devices: [
    { id: "local", name: "This computer", active: true, needsWindow: false },
    { id: "d1", name: "Kitchen", active: false, needsWindow: false },
  ],
};
const mm = (o = {}) => settingsFor("volume", o) as unknown as Record<string, unknown>;
const cover = () => null;

describe("multimedia dial", () => {
  it("turning is volume by the step, with the overlay", () => {
    const m = newDial();
    expect(dialTurn("volume", mm(), m, 2, lists)).toEqual({ command: "volumeStep", value: 10 });
    expect(m.overlay).toBe(true);
    expect(dialTurn("volume", mm({ step: 3 }), m, -1, lists)).toEqual({ command: "volumeStep", value: -3 });
    expect(dialTurn("volume", mm({ turn: "seek", step: 10 }), newDial(), 1, lists)).toEqual({ command: "seekBy", value: 10 });
    expect(dialTurn("volume", mm({ turn: "skip" }), newDial(), -1, lists)).toEqual({ command: "previous" });
  });

  it("press plays or pauses; double press switches to the browser and back", () => {
    const m = newDial();
    expect(dialPress("volume", mm(), m, false, lists)).toEqual({ command: "playPause" });
    expect(dialPress("volume", mm(), m, true, lists)).toBeNull();
    expect(m.view).toBe("browse");
    expect(dialTurn("volume", mm(), m, 5, lists)).toBeNull();
    expect(m.index).toBe(1);
    expect(dialPress("volume", mm({ browse: "playlists" }), m, false, lists)).toEqual({ command: "playItem", kind: "playlists", id: "p2" });
    dialPress("volume", mm(), m, true, lists);
    expect(m.view).toBe("player");
  });

  it("player view: title, artist, elapsed left and the song's whole length right", () => {
    const svg = svgOf(dialImage("volume", mm(), newDial(), snapOf(state(), { at: 0 }), 1000, lists, cover));
    expect(svg).toContain("Night Drive");
    expect(svg).toContain("Rue Ellis");
    expect(svg).toMatch(/text-anchor="start"[^>]*>1:01</);
    expect(svg).toMatch(/text-anchor="end"[^>]*>3:20</);
    expect(svg).not.toContain("-2:");
  });

  it("long titles get the smaller size and fade at the edge", () => {
    const long = svgOf(dialImage("volume", mm(), newDial(), snapOf(state({ title: "A Rather Long Title That Goes On And On" })), 0, lists, cover));
    expect(long).toMatch(/font-size="15"[^>]*fill="url/);
    expect(long).toContain("stop-opacity=\"0\"");
    const short = svgOf(dialImage("volume", mm(), newDial(), snapOf(state()), 0, lists, cover));
    expect(short).toContain('font-size="19"');
  });

  it("the overlay shows the volume or muted", () => {
    const m = { ...newDial(), overlay: true };
    expect(svgOf(dialImage("volume", mm(), m, snapOf(state({ volume: 35 })), 0, lists, cover))).toContain(">35<");
    expect(svgOf(dialImage("volume", mm(), m, snapOf(state({ muted: true })), 0, lists, cover))).toContain(">Muted<");
  });

  it("states: idle, not running, browser styles", () => {
    const i = svgOf(dialImage("volume", mm(), newDial(), snapOf(idle()), 0, lists, cover));
    expect(i).toContain("Nothing playing");
    expect(i).toContain(">-<");
    expect(svgOf(dialImage("volume", mm(), newDial(), snapOf(null, { conn: "offline" }), 0, lists, cover))).toContain("Platter not running");
    const b = { ...newDial(), view: "browse" as const };
    expect(svgOf(dialImage("volume", mm(), b, snapOf(state()), 0, lists, cover))).toContain("Road Trip");
    expect(svgOf(dialImage("volume", mm({ browser: "list" }), b, snapOf(state()), 0, lists, cover))).toContain("1 / 2");
  });
});

describe("other dials", () => {
  it("play / pause dial: seek and toggle, a centred time", () => {
    expect(dialTurn("playpausedial", {}, newDial(), -2, lists)).toEqual({ command: "seekBy", value: -10 });
    expect(dialPress("playpausedial", {}, newDial(), false, lists)).toEqual({ command: "playPause" });
    expect(svgOf(dialImage("playpausedial", {}, newDial(), snapOf(state()), 1000, lists, cover))).toMatch(/text-anchor="middle"[^>]*>1:00</);
  });

  it("volume dial: step, press mutes; a big number, or Muted", () => {
    const st = settingsFor("volumedial", { step: 4 }) as unknown as Record<string, unknown>;
    expect(dialTurn("volumedial", st, newDial(), 2, lists)).toEqual({ command: "volumeStep", value: 8 });
    expect(dialPress("volumedial", st, newDial(), false, lists)).toEqual({ command: "mute" });
    expect(svgOf(dialImage("volumedial", st, newDial(), snapOf(state({ volume: 64 })), 0, lists, cover))).toContain(">64<");
    expect(svgOf(dialImage("volumedial", st, newDial(), snapOf(state({ muted: true })), 0, lists, cover))).toContain(">Muted<");
  });

  it("queue and listen on dials turn through their lists and pick", () => {
    const q = newDial();
    dialTurn("queuedial", {}, q, 1, lists);
    expect(dialPress("queuedial", {}, q, false, lists)).toEqual({ command: "queueJump", itemId: "i2" });
    expect(svgOf(dialImage("queuedial", {}, q, snapOf(state()), 0, lists, cover))).toContain("Harbour");
    const d = newDial();
    dialTurn("listenondial", {}, d, 3, lists);
    expect(d.index).toBe(1);
    expect(dialPress("listenondial", {}, d, false, lists)).toEqual({ command: "listenOn", device: "d1" });
    expect(svgOf(dialImage("listenondial", {}, newDial(), snapOf(state()), 0, lists, cover))).toContain(C.accent);
    expect(dialPress("queuedial", {}, newDial(), false, { ...lists, queue: [] })).toBeNull();
  });
});
