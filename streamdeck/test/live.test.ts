import { describe, expect, it } from "vitest";
import { PlatterError } from "../src/client.js";
import { clock, elapsed, GRACE_MS, HOLD_MS, Live, progress } from "../src/live.js";
import { idle, snapOf, state } from "./fixtures.js";

const offline = new PlatterError("offline", "x");

function rig() {
  let t = 0;
  const live = new Live(() => t);
  return { live, at: (ms: number) => (t = ms) };
}

describe("Live", () => {
  it("starts connecting, then says not running after the grace period", () => {
    const { live, at } = rig();
    expect(live.snapshot.conn).toBe("connecting");
    at(GRACE_MS - 1);
    expect(live.update(null, offline).conn).toBe("connecting");
    at(GRACE_MS);
    expect(live.update(null, offline)).toMatchObject({ conn: "offline", state: null });
  });

  it("keeps the last picture through a short drop", () => {
    const { live, at } = rig();
    at(1000);
    live.update(state(), null);
    at(5000);
    const s = live.update(null, offline);
    expect(s.conn).toBe("connecting");
    expect(s.state?.title).toBe("Night Drive");
    at(1000 + GRACE_MS);
    expect(live.update(null, offline).conn).toBe("offline");
    at(1000 + GRACE_MS + 500);
    expect(live.update(state(), null).conn).toBe("ok");
  });

  it("holds the last song for a while after playback stops, then goes idle", () => {
    const { live, at } = rig();
    at(1000);
    live.update(state(), null);
    at(2000);
    const held = live.update(idle({ volume: 30 }), null);
    expect(held).toMatchObject({ conn: "ok", held: true });
    expect(held.state).toMatchObject({ title: "Night Drive", playing: false, volume: 30 });
    at(1000 + HOLD_MS);
    const after = live.update(idle(), null);
    expect(after.held).toBe(false);
    expect(after.state?.title).toBeNull();
  });

  it("not paired and not allowed are their own states; busy changes nothing", () => {
    const { live } = rig();
    expect(live.update(null, new PlatterError("unpaired", "x")).conn).toBe("unpaired");
    expect(live.update(null, new PlatterError("refused", "x")).conn).toBe("refused");
    live.update(state(), null);
    expect(live.update(null, new PlatterError("busy", "x")).conn).toBe("ok");
  });

  it("a private session counts as a song (its length shows)", () => {
    const { live, at } = rig();
    at(1);
    expect(live.update(state({ private: true, title: null, trackId: null }), null).held).toBe(false);
  });
});

describe("elapsed time", () => {
  it("runs on between polls while playing, not while paused or held", () => {
    const s = snapOf(state(), { at: 1000 });
    expect(elapsed(s, 1000)).toBe(60_000);
    expect(elapsed(s, 3500)).toBe(62_500);
    expect(elapsed(snapOf(state({ playing: false }), { at: 1000 }), 5000)).toBe(60_000);
    expect(elapsed(snapOf(state(), { at: 1000, held: true }), 5000)).toBe(60_000);
    expect(elapsed(s, 10_000_000), "never past the end").toBe(200_000);
    expect(progress(s, 1000)).toBeCloseTo(0.3);
    expect(progress(snapOf(null), 0)).toBe(0);
  });

  it("clock reads m:ss and h:mm:ss", () => {
    expect(clock(0)).toBe("0:00");
    expect(clock(61_999)).toBe("1:01");
    expect(clock(3_725_000)).toBe("1:02:05");
    expect(clock(-5)).toBe("0:00");
  });
});
