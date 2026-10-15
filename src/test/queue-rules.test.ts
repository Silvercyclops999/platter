import { afterEach, describe, expect, it } from "vitest";
import { MockBackend } from "../lib/mock";
import type { PlayerState, Track } from "../lib/types";

// The mock mirrors the core's queue rules, so screens and
// Playwright see the same behaviour as the app.

let mock: MockBackend;
afterEach(() => mock?.dispose());

function fresh() {
  mock = new MockBackend({ latencyMs: 0, timelines: false });
  return mock;
}

/** Albums other than the one the mock starts with, with at least `n` tracks. */
function albumWith(m: MockBackend, n: number, skip = 0) {
  const album = m.fx.albums.slice(1).filter((a) => (m.fx.tracksByAlbum.get(a.id)?.length ?? 0) >= n)[skip];
  return { album, tracks: m.fx.tracksByAlbum.get(album.id)! };
}

const ids = (items: { track: Track }[]) => items.map((i) => i.track.id);
const cur = (s: PlayerState) => s.track!.id;

describe("Previous", () => {
  it("goes to the skipped neighbour after a jump ahead", async () => {
    const m = fresh();
    const { album, tracks } = albumWith(m, 5);
    await m.playContext({ context: { kind: "album", id: album.id }, startTrackId: tracks[0].id });
    const s = await m.playerState();
    await m.queueJump(s.queue.nextUp[2].itemId); // track 4
    await m.playPrevious();
    expect(cur(await m.playerState())).toBe(tracks[2].id);
  });

  it("skips a queued song that played between tracks 2 and 3", async () => {
    const m = fresh();
    const { album, tracks } = albumWith(m, 4);
    const other = m.fx.tracks.find((t) => t.albumId !== album.id)!;
    await m.playContext({ context: { kind: "album", id: album.id }, startTrackId: tracks[1].id });
    await m.queueAdd([other.id], "end");
    await m.playNext(); // queued song
    await m.playNext(); // track 3
    expect(cur(await m.playerState())).toBe(tracks[2].id);
    await m.playPrevious();
    expect(cur(await m.playerState())).toBe(tracks[1].id);
  });

  it("on a queued song drops it and goes back to the context track", async () => {
    const m = fresh();
    const { album, tracks } = albumWith(m, 3);
    const other = m.fx.tracks.find((t) => t.albumId !== album.id)!;
    await m.playContext({ context: { kind: "album", id: album.id }, startTrackId: tracks[0].id });
    await m.queueAdd([other.id], "next");
    await m.playNext();
    expect(cur(await m.playerState())).toBe(other.id);
    await m.playPrevious();
    const s = await m.playerState();
    expect(cur(s)).toBe(tracks[0].id);
    expect(s.queue.userQueue).toHaveLength(0);
    await m.playNext();
    expect(cur(await m.playerState())).toBe(tracks[1].id);
  });

  it("restarts the first track, and restarts at exactly 3.000 s", async () => {
    const m = fresh();
    const { album, tracks } = albumWith(m, 3);
    await m.playContext({ context: { kind: "album", id: album.id }, startTrackId: tracks[0].id });
    await m.playSeek(1_000);
    await m.playPrevious();
    let s = await m.playerState();
    expect([cur(s), s.positionMs]).toEqual([tracks[0].id, 0]);
    await m.playNext();
    await m.playSeek(3_000);
    await m.playPrevious();
    s = await m.playerState();
    expect([cur(s), s.positionMs]).toEqual([tracks[1].id, 0]);
    await m.playSeek(2_999);
    await m.playPrevious();
    expect(cur(await m.playerState())).toBe(tracks[0].id);
  });
});

describe("repeat-one", () => {
  it("drops to repeat-all on Next and on an early Previous, but not a late one", async () => {
    const m = fresh();
    const { album, tracks } = albumWith(m, 3);
    await m.playContext({ context: { kind: "album", id: album.id }, startTrackId: tracks[1].id });
    await m.setRepeat("one");
    await m.playSeek(5_000);
    await m.playPrevious();
    expect((await m.playerState()).repeat).toBe("one");
    await m.playPrevious();
    let s = await m.playerState();
    expect(s.repeat).toBe("all");
    expect(cur(s)).toBe(tracks[0].id);
    await m.setRepeat("one");
    await m.playNext();
    s = await m.playerState();
    expect(s.repeat).toBe("all");
    expect(cur(s)).toBe(tracks[1].id);
  });
});

describe("resume points", () => {
  it("a fresh album starts at track 1; one played before resumes where it was", async () => {
    const m = fresh();
    const a = albumWith(m, 4, 0);
    const b = albumWith(m, 2, 1);
    await m.playContext({ context: { kind: "album", id: a.album.id } });
    expect(cur(await m.playerState())).toBe(a.tracks[0].id);
    await m.playNext();
    await m.playNext();
    await m.playSeek(42_000);
    await m.playPause();
    await m.playContext({ context: { kind: "album", id: b.album.id } });
    await m.playContext({ context: { kind: "album", id: a.album.id } });
    const s = await m.playerState();
    expect(cur(s)).toBe(a.tracks[2].id);
    expect(s.positionMs).toBe(42_000);
  });

  it("with shuffle on starts on a random track", async () => {
    const starts = new Set<string>();
    for (let i = 0; i < 12; i++) {
      const m = fresh();
      // Burn a different number of random draws so each run differs.
      for (let k = 0; k < i; k++) await m.playContext({ context: { kind: "album", id: m.fx.albums[2].id }, shuffle: true });
      const { album } = albumWith(m, 6);
      await m.playContext({ context: { kind: "album", id: album.id }, shuffle: true });
      starts.add(cur(await m.playerState()));
      m.dispose();
    }
    expect(starts.size).toBeGreaterThan(2);
  });
});

describe("context end", () => {
  it("with Stop, rewinds to track 1, paused at 0:00, with the rest upcoming", async () => {
    const m = fresh();
    await m.settingsSet({ queueEnd: "stop" });
    const { album, tracks } = albumWith(m, 3);
    await m.playContext({ context: { kind: "album", id: album.id }, startTrackId: tracks[tracks.length - 1].id });
    await m.playNext();
    const s = await m.playerState();
    expect(s.status).toBe("paused");
    expect(s.positionMs).toBe(0);
    expect(cur(s)).toBe(tracks[0].id);
    expect(ids(s.queue.nextUp)).toEqual(tracks.slice(1).map((t) => t.id));
  });
});

describe("keep playing similar music", () => {
  it("adds similar songs when the last one ends, marked as autoplay", async () => {
    const m = fresh();
    const { album, tracks } = albumWith(m, 3);
    await m.playContext({ context: { kind: "album", id: album.id }, startTrackId: tracks[tracks.length - 1].id });
    await m.playNext();
    const s = await m.playerState();
    expect(s.status).toBe("playing");
    expect(tracks.map((t) => t.id)).not.toContain(cur(s));
    expect(s.queue.current?.source).toBe("autoplay");
  });

});

/** A playlist of one artist's songs, `n` long. */
async function playlistOf(m: MockBackend, n: number) {
  const { tracks } = albumWith(m, 4);
  const pool = m.fx.tracks.filter((t) => t.artistId === tracks[0].artistId).slice(0, n);
  const pl = await m.playlistCreate("Smart test", pool.map((t) => t.id));
  return { pl, tracks: pool };
}

/** C a playlist song, S a suggestion; the playing song first. */
const rhythm = (s: PlayerState) =>
  [s.queue.current, ...s.queue.nextUp].map((i) => (i?.source === "smart" ? "S" : "C")).join("");

describe("smart shuffle", () => {
  it("is for playlists and Liked Songs, not albums", async () => {
    const m = fresh();
    const { album } = albumWith(m, 6);
    await m.playContext({ context: { kind: "album", id: album.id }, shuffle: true });
    await expect(m.setSmartShuffle(true)).rejects.toBeTruthy();
    expect((await m.playerState()).smartShuffle).toBe(false);
    await m.playContext({ context: { kind: "liked", id: "liked" }, shuffle: true });
    await m.setSmartShuffle(true);
    const s = await m.playerState();
    expect(s.smartShuffle).toBe(true);
    expect(s.smartContexts).toContain("liked");
  });

  it("puts one suggestion after every two playlist songs, once, and doesn't refill", async () => {
    const m = fresh();
    const { pl, tracks } = await playlistOf(m, 7);
    expect(tracks).toHaveLength(7);
    await m.setContextSmartShuffle(pl.id, true);
    await m.playContext({ context: { kind: "playlist", id: pl.id } });
    let s = await m.playerState();
    expect(s.shuffle && s.smartShuffle).toBe(true);
    expect(s.smartContexts).toContain(pl.id);
    expect(rhythm(s)).toBe("CCSCCSCCSC");
    const smart = s.queue.nextUp.filter((i) => i.source === "smart");
    expect(smart.every((i) => i.track.artistId !== tracks[0].artistId)).toBe(true);
    expect(smart.every((i) => !tracks.some((t) => t.id === i.track.id))).toBe(true);
    await m.playNext();
    await m.playNext();
    await m.playNext();
    s = await m.playerState();
    expect(s.queue.nextUp.filter((i) => i.source === "smart")).toHaveLength(2);
  });

  it("removing the playing suggestion skips it and picks new ones", async () => {
    const m = fresh();
    const { pl } = await playlistOf(m, 7);
    await m.setContextSmartShuffle(pl.id, true);
    await m.playContext({ context: { kind: "playlist", id: pl.id } });
    await expect(m.smartSuggestionRemove()).rejects.toBeTruthy();
    await m.playNext();
    await m.playNext();
    let s = await m.playerState();
    expect(s.queue.current?.source).toBe("smart");
    await m.smartSuggestionRemove();
    s = await m.playerState();
    expect(s.queue.current?.source).toBe("context");
    expect(rhythm(s)).toBe("CCSCCSC");
  });

  it("adding the playing suggestion keeps it playing without its badge", async () => {
    const m = fresh();
    const { pl } = await playlistOf(m, 7);
    await m.setContextSmartShuffle(pl.id, true);
    await m.playContext({ context: { kind: "playlist", id: pl.id } });
    await m.playNext();
    await m.playNext();
    const cur = (await m.playerState()).queue.current!;
    await m.smartSuggestionKeep(cur.itemId);
    const s = await m.playerState();
    expect(s.queue.current?.itemId).toBe(cur.itemId);
    expect(s.queue.current?.source).toBe("context");
    expect(s.status).toBe("playing");
  });

  it("off mid-play keeps the song and the shuffled order, without suggestions", async () => {
    const m = fresh();
    const { pl } = await playlistOf(m, 7);
    await m.setContextSmartShuffle(pl.id, true);
    await m.playContext({ context: { kind: "playlist", id: pl.id } });
    await m.playNext();
    await m.playNext();
    const before = await m.playerState();
    const order = ids(before.queue.nextUp.filter((i) => i.source !== "smart"));
    await m.setSmartShuffle(false);
    const s = await m.playerState();
    expect(s.shuffle || s.smartShuffle).toBe(false);
    expect(s.queue.current?.itemId).toBe(before.queue.current?.itemId);
    expect(s.queue.current?.source).toBe("smart");
    expect(ids(s.queue.nextUp)).toEqual(order);
  });

  it("is remembered per playlist, and the setting takes it out everywhere", async () => {
    const m = fresh();
    const { pl } = await playlistOf(m, 5);
    await m.setContextSmartShuffle(pl.id, true);
    await m.setContextSmartShuffle("liked", true);
    await m.setContextSmartShuffle("liked", false);
    let s = await m.playerState();
    expect(s.smartContexts).toEqual([pl.id]);
    expect(s.shuffledContexts).not.toContain("liked");
    await m.playContext({ context: { kind: "playlist", id: pl.id } });
    await m.settingsSet({ smartShuffle: false });
    await new Promise((r) => setTimeout(r, 5));
    s = await m.playerState();
    expect(s.smartShuffle).toBe(false);
    expect(s.shuffle).toBe(true);
    expect(s.queue.nextUp.some((i) => i.source === "smart")).toBe(false);
  });

  it("hands over to keep playing at the end, without badges", async () => {
    const m = fresh();
    const { pl } = await playlistOf(m, 3);
    await m.setContextSmartShuffle(pl.id, true);
    await m.playContext({ context: { kind: "playlist", id: pl.id } });
    expect(rhythm(await m.playerState())).toBe("CCSC");
    await m.playNext();
    await m.playNext();
    await m.playNext();
    await m.playNext();
    const s = await m.playerState();
    expect(s.queue.current?.source).toBe("autoplay");
    expect(s.queue.nextUp.every((i) => i.source !== "smart")).toBe(true);
  });
});

describe("per-context shuffle", () => {
  it("is remembered per album, and can be set for one that isn't playing", async () => {
    const m = fresh();
    const a = albumWith(m, 4, 0);
    const b = albumWith(m, 4, 1);
    await m.playContext({ context: { kind: "album", id: a.album.id } });
    await m.setShuffle(true);
    let s = await m.playerState();
    expect(s.shuffle).toBe(true);
    expect(s.shuffledContexts).toContain(a.album.id);

    await m.playContext({ context: { kind: "album", id: b.album.id } });
    expect((await m.playerState()).shuffle).toBe(false);

    await m.playContext({ context: { kind: "album", id: a.album.id } });
    expect((await m.playerState()).shuffle).toBe(true);

    await m.setContextShuffle(b.album.id, true);
    s = await m.playerState();
    expect(s.shuffle).toBe(true);
    expect(s.shuffledContexts).toEqual([a.album.id, b.album.id]);

    await m.setContextShuffle(a.album.id, false);
    s = await m.playerState();
    expect(s.shuffle).toBe(false);
    expect(s.shuffledContexts).toEqual([b.album.id]);
  });

  it("turning shuffle on deals earlier tracks too; off continues after the current one", async () => {
    const m = fresh();
    const { album, tracks } = albumWith(m, 6);
    await m.playContext({ context: { kind: "album", id: album.id }, startTrackId: tracks[3].id });
    await m.setShuffle(true);
    let s = await m.playerState();
    expect(s.queue.nextUp).toHaveLength(tracks.length - 1);
    expect(ids(s.queue.nextUp)).toContain(tracks[0].id);
    await m.setShuffle(false);
    s = await m.playerState();
    expect(ids(s.queue.nextUp)).toEqual(tracks.slice(4).map((t) => t.id));
  });
});

describe("shuffle style", () => {
  function playlistOrder(m: MockBackend) {
    return m.playerState().then((s) => [s.track!, ...s.queue.nextUp.map((i) => i.track)]);
  }
  const adjacentSameArtist = (ts: Track[]) =>
    ts.slice(1).filter((t, i) => (t.trackArtist ?? t.artistTitle) === (ts[i].trackArtist ?? ts[i].artistTitle)).length;

  it("spread keeps artists apart better than standard", async () => {
    let spread = 0;
    let standard = 0;
    for (let run = 0; run < 20; run++) {
      const m = fresh();
      // Up to 8 tracks from each of several artists, like a mixed playlist.
      const byArtist = new Map<string, string[]>();
      for (const t of m.fx.tracks) {
        if (t.trackArtist) continue;
        const k = t.artistTitle;
        const list = byArtist.get(k) ?? [];
        if (list.length < 8) byArtist.set(k, [...list, t.id]);
      }
      const trackIds = [...byArtist.values()].slice(0, 5).flat();
      await m.playContext({ context: { kind: "tracks", trackIds }, shuffle: true });
      spread += adjacentSameArtist(await playlistOrder(m));
      await m.settingsSet({ shuffleStyle: "standard" });
      await m.playContext({ context: { kind: "tracks", trackIds }, shuffle: true });
      standard += adjacentSameArtist(await playlistOrder(m));
      m.dispose();
    }
    expect(spread * 3).toBeLessThan(standard);
  });

  it("defaults to spread", async () => {
    const m = fresh();
    expect((await m.settingsGet()).shuffleStyle).toBe("spread");
  });
});
