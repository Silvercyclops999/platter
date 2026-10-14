import { describe, expect, it } from "vitest";
import { buildArtistLookup, splitCredits, trackCredits } from "../lib/credits";
import { popularTracks } from "../lib/mock/popular";
import type { Track } from "../lib/types";

const lookup = buildArtistLookup([
  { id: "1", title: "Odessa Wren" },
  { id: "2", title: "Moth & Meridian" },
  { id: "3", title: "Saltglass" },
  { id: "4", title: "Tyler, The Maker" },
  { id: "5", title: "Wren" },
]);

const names = (line: string) => splitCredits(line, lookup).map((c) => c.name);

describe("splitCredits", () => {
  it("splits a comma list and links the known names", () => {
    expect(splitCredits("Odessa Wren, Saltglass, Nobody Known", lookup)).toEqual([
      { name: "Odessa Wren", id: "1" },
      { name: "Saltglass", id: "3" },
      { name: "Nobody Known", id: null },
    ]);
  });

  it("keeps known names with '&' or ',' in them whole", () => {
    expect(names("Moth & Meridian, Saltglass")).toEqual(["Moth & Meridian", "Saltglass"]);
    expect(names("Saltglass & Moth & Meridian")).toEqual(["Saltglass", "Moth & Meridian"]);
    expect(splitCredits("tyler, the maker feat. Odessa Wren", lookup)).toEqual([
      { name: "tyler, the maker", id: "4" },
      { name: "Odessa Wren", id: "1" },
    ]);
    // Unknown to the library: split like any other line.
    expect(names("Salt & Pepper")).toEqual(["Salt", "Pepper"]);
  });

  it("understands feat. variants, with or without brackets", () => {
    for (const line of [
      "Saltglass feat. Odessa Wren",
      "Saltglass ft. Odessa Wren",
      "Saltglass Feat. Odessa Wren",
      "Saltglass featuring Odessa Wren",
      "Saltglass (feat. Odessa Wren)",
      "Saltglass [ft. Odessa Wren]",
      "Saltglass x Odessa Wren",
      "Saltglass X Odessa Wren",
      "Saltglass & Odessa Wren",
    ]) {
      expect(names(line), line).toEqual(["Saltglass", "Odessa Wren"]);
    }
    expect(names("Saltglass feat. Odessa Wren & Moth & Meridian")).toEqual(["Saltglass", "Odessa Wren", "Moth & Meridian"]);
  });

  it("doesn't split inside names", () => {
    expect(names("Maxx Feather")).toEqual(["Maxx Feather"]);
    expect(names("Rock&Roll Band")).toEqual(["Rock&Roll Band"]);
    // "Wren" is an artist, but here it is part of "Odessa Wren".
    expect(splitCredits("Odessa Wren", lookup)).toEqual([{ name: "Odessa Wren", id: "1" }]);
  });

  it("drops repeats and empty pieces", () => {
    expect(names("Saltglass, saltglass, Odessa Wren, Saltglass")).toEqual(["Saltglass", "Odessa Wren"]);
    expect(names(" , ")).toEqual([]);
    expect(names("")).toEqual([]);
  });
});

describe("trackCredits", () => {
  const base = { artistId: "3", artistTitle: "Saltglass" };

  it("uses the album artist when there are no track artists", () => {
    expect(trackCredits({ ...base, trackArtist: null }, new Map())).toEqual([{ name: "Saltglass", id: "3" }]);
  });

  it("shows the album artist once when the track line repeats it", () => {
    expect(trackCredits({ ...base, trackArtist: "Saltglass, Odessa Wren" }, lookup)).toEqual([
      { name: "Saltglass", id: "3" },
      { name: "Odessa Wren", id: "1" },
    ]);
  });

  it("links the album artist before the library names have loaded", () => {
    expect(trackCredits({ ...base, trackArtist: "Saltglass feat. Someone" }, new Map())).toEqual([
      { name: "Saltglass", id: "3" },
      { name: "Someone", id: null },
    ]);
  });
});

describe("popular rule (browser mock)", () => {
  let n = 0;
  const t = (p: Partial<Track>): Track => ({
    id: `t${++n}`,
    title: `Song ${n}`,
    albumId: "al",
    albumTitle: "LP",
    artistId: "1",
    artistTitle: "Odessa Wren",
    trackArtist: null,
    trackNumber: 1,
    discNumber: 1,
    durationMs: 200_000,
    thumb: null,
    partKey: "",
    codec: null,
    container: null,
    bitrate: null,
    bitDepth: null,
    sampleRate: null,
    userRating: null,
    viewCount: 0,
    lastViewedAt: null,
    ...p,
  });
  const me = { id: "1", title: "Odessa Wren" };

  it("ranks by listener count, then plays, and includes credited features", () => {
    const tracks = [
      t({ id: "low", ratingCount: 10 }),
      t({ id: "played", viewCount: 50 }),
      t({ id: "hit", ratingCount: 9000 }),
      t({ id: "feat", artistId: "3", artistTitle: "Saltglass", trackArtist: "Saltglass (feat. Odessa Wren)", ratingCount: 500 }),
      t({ id: "other", artistId: "3", artistTitle: "Saltglass", trackArtist: "Saltglass, Wren", ratingCount: 99999 }),
      t({ id: "unplayed" }),
    ];
    expect(popularTracks(tracks, me, lookup).map((x) => x.id)).toEqual(["hit", "feat", "low", "played", "unplayed"]);
  });

  it("collapses copies of a song and stops at ten", () => {
    const copies = [
      t({ id: "a", title: "Anthem", ratingCount: 5 }),
      t({ id: "b", title: "ANTHEM", ratingCount: 50, durationMs: 201_500 }),
      t({ id: "c", title: "Anthem", durationMs: 260_000 }),
    ];
    expect(popularTracks(copies, me, lookup).map((x) => x.id)).toEqual(["b", "c"]);
    const many = Array.from({ length: 14 }, () => t({}));
    expect(popularTracks(many, me, lookup)).toHaveLength(10);
  });
});
