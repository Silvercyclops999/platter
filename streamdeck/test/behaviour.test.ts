import { describe, expect, it } from "vitest";
import { GESTURES, pressCommands, REPEATS } from "../src/behaviour.js";
import { DEFAULTS, settingsFor } from "../src/settings.js";
import type { Gesture } from "../src/timing.js";
import { KEY_ACTIONS, type KeyActionId } from "../src/views.js";
import { snapOf, state } from "./fixtures.js";

const press = (a: KeyActionId, raw: Record<string, unknown> = {}, g: Gesture = "single", s = state()) =>
  pressCommands(a, g, (a in DEFAULTS ? settingsFor(a as keyof typeof DEFAULTS, raw) : {}) as Record<string, unknown>, snapOf(s, { at: Date.now() }));

describe("what a press sends", () => {
  it("play / pause: single plays or pauses, double next, triple or long previous", () => {
    expect(press("playpause")).toEqual([{ command: "playPause" }]);
    expect(press("playpause", {}, "double")).toEqual([{ command: "next" }]);
    expect(press("playpause", {}, "triple")).toEqual([{ command: "previous" }]);
    expect(press("playpause", {}, "long")).toEqual([{ command: "previous" }]);
    expect(GESTURES.playpause).toEqual({ double: true, long: true });
  });

  it("every key sends something by default, unless it needs setting up", () => {
    const unset = ["removefromplaylist", "playplaylist", "playalbum", "playartist", "playmix", "playlink", "playsearch"];
    for (const a of KEY_ACTIONS) expect(press(a).length > 0, a).toBe(!unset.includes(a));
  });

  it("transport, seek and modes", () => {
    expect(press("previous")).toEqual([{ command: "previous" }]);
    expect(press("next")).toEqual([{ command: "next" }]);
    expect(press("seekback")).toEqual([{ command: "seekBy", value: -10 }]);
    expect(press("seekforward", { seconds: 30 })).toEqual([{ command: "seekBy", value: 30 }]);
    expect(press("songinfo")).toEqual([{ command: "nowPlaying" }]);
    expect(press("shuffle")).toEqual([{ command: "shuffle" }]);
    expect(press("shuffle", {}, "long")).toEqual([{ command: "shuffle", smart: true }]);
    expect(press("repeat")).toEqual([{ command: "repeat" }]);
    expect(press("crossfade")).toEqual([{ command: "crossfade" }]);
    expect(press("private")).toEqual([{ command: "privateSession" }]);
  });

  it("volume", () => {
    expect(press("volumeup")).toEqual([{ command: "volumeStep", value: 5 }]);
    expect(press("volumedown", { step: 12 })).toEqual([{ command: "volumeStep", value: -12 }]);
    expect(press("mute")).toEqual([{ command: "mute" }]);
    expect(press("volumeset", { level: 30 })).toEqual([{ command: "volumeSet", value: 30 }]);
    expect(REPEATS).toEqual(expect.arrayContaining(["volumeup", "volumedown"]));
  });

  it("library keys: add toggles out again, ask without a playlist", () => {
    expect(press("like")).toEqual([{ command: "like" }]);
    expect(press("addtoplaylist")).toEqual([{ command: "addToPlaylist" }]);
    expect(press("addtoplaylist", { playlist: "p1" })).toEqual([{ command: "addToPlaylist", playlist: "p1" }]);
    expect(press("addtoplaylist", { playlist: "p1" }, "single", state({ playlists: ["p1"] }))).toEqual([{ command: "removeFromPlaylist", playlist: "p1" }]);
    expect(press("removefromplaylist", { playlist: "p1" })).toEqual([{ command: "removeFromPlaylist", playlist: "p1" }]);
    expect(press("rate", { stars: 3 })).toEqual([{ command: "rate", stars: 3 }]);
  });

  it("play keys: play now (shuffled, from a time) or queue", () => {
    expect(press("playplaylist", { id: "p1", shuffle: true })).toEqual([{ command: "playItem", kind: "playlists", id: "p1", shuffle: true }]);
    expect(press("playalbum", { id: "a1", start: 30 })).toEqual([{ command: "playItem", kind: "albums", id: "a1", shuffle: false, start: 30 }]);
    expect(press("playartist", { id: "r1", mode: "queue" })).toEqual([{ command: "playItem", kind: "artists", id: "r1", queue: true }]);
    expect(press("playmix", { id: "m1" })).toEqual([{ command: "playItem", kind: "mixes", id: "m1", shuffle: false }]);
    expect(press("playlink", { link: " platter://album/5 " })).toEqual([{ command: "playLink", link: "platter://album/5" }]);
    expect(press("playsearch", { query: "night" })).toEqual([{ command: "playSearch", query: "night" }]);
    expect(press("radio")).toEqual([{ command: "radio" }]);
    expect(press("daylist")).toEqual([{ command: "playShortcut", name: "daylist" }]);
    expect(press("discover")).toEqual([{ command: "playShortcut", name: "discover" }]);
    expect(press("likedsongs")).toEqual([{ command: "playShortcut", name: "liked" }]);
  });

  it("queue, devices and Platter", () => {
    expect(press("queue")).toEqual([{ command: "queue" }]);
    expect(press("listenon")).toEqual([{ command: "listenOn" }]);
    expect(press("listenon", { device: "d1" })).toEqual([{ command: "listenOn", device: "d1" }]);
    expect(press("lyrics")).toEqual([{ command: "lyrics" }]);
    expect(press("miniplayer")).toEqual([{ command: "miniplayer" }]);
    expect(press("window")).toEqual([{ command: "window" }]);
    expect(press("bookmark")).toEqual([{ command: "bookmark" }]);
    expect(press("request")).toEqual([{ command: "requestAlbum" }]);
    expect(press("listen")).toEqual([{ command: "listenAlong" }]);
  });

  it("sleep: starts, stops while running (or adds time), long press starts again", () => {
    expect(press("sleep")).toEqual([{ command: "sleep", minutes: 30 }]);
    expect(press("sleep", { endOfSong: true })).toEqual([{ command: "sleep", endOfSong: true }]);
    const running = state({ sleep: { remainingMs: 600_000, endOfSong: false } });
    expect(press("sleep", {}, "single", running)).toEqual([{ command: "sleepCancel" }]);
    expect(press("sleep", { whileRunning: "add", addMinutes: 15 }, "single", running)).toEqual([{ command: "sleep", minutes: 25 }]);
    expect(press("sleep", { minutes: 20 }, "long", running)).toEqual([{ command: "sleep", minutes: 20 }]);
  });
});
