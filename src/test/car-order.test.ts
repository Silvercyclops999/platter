/** CarPlay lists playlists in the phone Library's order: pinned first, then its sort. */
import { describe, expect, it } from "vitest";
import { carPlaylistOrder } from "../phone/carOrder";
import type { LibEntry } from "../stores/library";

const pl = (id: string, title: string, extra: Partial<LibEntry> = {}): LibEntry => ({
  kind: "playlist",
  id,
  title,
  sortTitle: title,
  creator: null,
  creatorId: null,
  thumb: null,
  smart: false,
  format: null,
  addedAt: null,
  playedAt: null,
  pinned: false,
  ...extra,
});

const entries: LibEntry[] = [
  pl("liked", "Liked Songs", { liked: true, pinned: true }),
  pl("1", "The Night Drive", { playedAt: 100 }),
  pl("2", "Assorted", { playedAt: 300, addedAt: 10 }),
  pl("3", "Zed", { playedAt: 200, addedAt: 30, pinned: true }),
  pl("mix", "Saved mix", { mix: true, playedAt: 999 }),
  { ...pl("al", "An album"), kind: "album" },
];

describe("CarPlay playlist order", () => {
  it("follows Recents with pins first, and leaves out Liked Songs, mixes and albums", () => {
    expect(carPlaylistOrder(entries, "recents", {}, [])).toEqual(["3", "2", "1"]);
  });
  it("follows A to Z the way the phone files names (a leading The is ignored)", () => {
    expect(carPlaylistOrder(entries, "alpha", {}, [])).toEqual(["3", "2", "1"]);
    const unpinned = entries.map((e) => ({ ...e, pinned: e.liked ? true : false }));
    expect(carPlaylistOrder(unpinned, "alpha", {}, [])).toEqual(["2", "1", "3"]);
  });
  it("follows your own order", () => {
    const unpinned = entries.map((e) => ({ ...e, pinned: !!e.liked }));
    expect(carPlaylistOrder(unpinned, "custom", {}, ["1", "3", "2"])).toEqual(["1", "3", "2"]);
  });
});
