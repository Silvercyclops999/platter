import { describe, expect, it } from "vitest";
import { DEFAULTS, formatInfo, imagesOf, settingsFor } from "../src/settings.js";

describe("settings", () => {
  it("defaults as the behaviour notes have them", () => {
    expect(settingsFor("volume", {})).toMatchObject({ turn: "volume", step: 5, browser: "covers" });
    expect(settingsFor("sleep", undefined)).toMatchObject({ whileRunning: "stop", minutes: 30, endOfSong: false });
    expect(settingsFor("seekback", null)).toMatchObject({ seconds: 10 });
    expect(settingsFor("volumeup", {})).toMatchObject({ step: 5, show: "none" });
    expect(settingsFor("playpause", {})).toMatchObject({ time: "off", showTitle: false, pausedLook: "circle", group: "off" });
    expect(settingsFor("addtoplaylist", {})).toMatchObject({ playlist: "" });
    expect(settingsFor("rate", {})).toMatchObject({ stars: 5 });
    expect(settingsFor("playalbum", {})).toMatchObject({ id: "", shuffle: false, mode: "play", start: 0 });
    expect(settingsFor("listenon", {})).toMatchObject({ device: "" });
    expect(settingsFor("volumedial", {})).toMatchObject({ step: 5 });
    expect(settingsFor("next", {})).toMatchObject({ cover: false });
    expect(settingsFor("songinfo", {})).toMatchObject({ output: "none", format: "{title} - {artist}" });
    // Every action with settings has defaults that pass their own rules.
    for (const k of Object.keys(DEFAULTS) as (keyof typeof DEFAULTS)[]) {
      const { images, ...rest } = settingsFor(k, DEFAULTS[k]);
      expect(rest, k).toEqual(DEFAULTS[k]);
      expect(images).toEqual({});
    }
  });

  it("brings values back in range and ignores odd choices", () => {
    expect(settingsFor("volumeup", { step: 99 }).step).toBe(25);
    expect(settingsFor("volumeup", { step: 0 }).step).toBe(1);
    expect(settingsFor("volumeup", { step: "7" }).step).toBe(7);
    expect(settingsFor("volumeset", { level: 140 }).level).toBe(100);
    expect(settingsFor("rate", { stars: 9 }).stars).toBe(5);
    expect(settingsFor("seekforward", { seconds: "abc" }).seconds).toBe(10);
    expect(settingsFor("volume", { turn: "sideways", browser: "list" })).toMatchObject({ turn: "volume", browser: "list" });
    expect(settingsFor("playpause", { group: "manual", groupSize: 7, groupCol: 0 })).toMatchObject({ group: "manual", groupSize: 4, groupCol: 1 });
    expect(settingsFor("playplaylist", { shuffle: "true", id: 5 })).toMatchObject({ shuffle: true, id: "" });
  });

  it("keeps only data images as the user's own pictures", () => {
    expect(imagesOf({ images: { paused: "data:image/png;base64,AA", playing: "file:///x.png", on: 3 } })).toEqual({ paused: "data:image/png;base64,AA" });
  });

  it("fills a song info format", () => {
    const f = { title: "Night Drive", artist: "Rue Ellis", album: "Late", elapsed: "1:00", total: "3:20" };
    expect(formatInfo("{title} - {artist} ({elapsed}/{total}) {album} {other}", f)).toBe("Night Drive - Rue Ellis (1:00/3:20) Late {other}");
  });
});
