import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GAP_MS, type Gesture, HoldRepeat, LONG_MS, Overlay, OVERLAY_MS, Presses, REPEAT_AFTER_MS, REPEAT_EVERY_MS } from "../src/timing.js";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

function presses(opts = {}) {
  const got: Gesture[] = [];
  const p = new Presses((g) => got.push(g), opts);
  const tap = (holdMs = 60) => {
    p.down();
    vi.advanceTimersByTime(holdMs);
    p.up();
  };
  return { p, got, tap };
}

describe("Presses", () => {
  it("a single press waits out the gap", () => {
    const { got, tap } = presses();
    tap();
    vi.advanceTimersByTime(GAP_MS - 1);
    expect(got).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(got).toEqual(["single"]);
  });

  it("two presses within the gap are a double", () => {
    const { got, tap } = presses();
    tap();
    vi.advanceTimersByTime(GAP_MS - 50);
    tap();
    vi.advanceTimersByTime(GAP_MS);
    expect(got).toEqual(["double"]);
  });

  it("three presses are a triple, at once", () => {
    const { got, tap } = presses();
    tap();
    vi.advanceTimersByTime(100);
    tap();
    vi.advanceTimersByTime(100);
    tap();
    expect(got).toEqual(["triple"]);
    vi.advanceTimersByTime(1000);
    expect(got).toEqual(["triple"]);
  });

  it("presses further apart than the gap are separate singles", () => {
    const { got, tap } = presses();
    tap();
    vi.advanceTimersByTime(GAP_MS + 10);
    tap();
    vi.advanceTimersByTime(GAP_MS);
    expect(got).toEqual(["single", "single"]);
  });

  it("holding fires a long press while still held, and the release does nothing more", () => {
    const { p, got } = presses();
    p.down();
    vi.advanceTimersByTime(LONG_MS - 1);
    expect(got).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(got).toEqual(["long"]);
    p.up();
    vi.advanceTimersByTime(1000);
    expect(got).toEqual(["long"]);
  });

  it("in a multi-action a press runs once, at once, with no double or long", () => {
    const { p, got } = presses();
    p.down(true);
    expect(got).toEqual(["single"]);
    vi.advanceTimersByTime(LONG_MS * 2);
    p.up();
    p.down(true);
    p.up();
    vi.advanceTimersByTime(GAP_MS * 2);
    expect(got).toEqual(["single", "single"]);
  });

  it("without doubles a press is a single on release; without long, holding is a single", () => {
    const a = presses({ double: false });
    a.tap();
    expect(a.got).toEqual(["single"]);
    const b = presses({ long: false, double: false });
    b.tap(LONG_MS * 3);
    expect(b.got).toEqual(["single"]);
  });

  it("reset forgets a press in progress", () => {
    const { p, got } = presses();
    p.down();
    p.reset();
    vi.advanceTimersByTime(LONG_MS + GAP_MS);
    expect(got).toEqual([]);
  });
});

describe("HoldRepeat", () => {
  it("fires on press, then repeats while held", () => {
    const fire = vi.fn();
    const h = new HoldRepeat(fire);
    h.down();
    expect(fire).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(REPEAT_AFTER_MS - 1);
    expect(fire).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1 + REPEAT_EVERY_MS * 3);
    expect(fire).toHaveBeenCalledTimes(4);
    h.up();
    vi.advanceTimersByTime(2000);
    expect(fire).toHaveBeenCalledTimes(4);
  });

  it("in a multi-action it fires once", () => {
    const fire = vi.fn();
    new HoldRepeat(fire).down(true);
    vi.advanceTimersByTime(2000);
    expect(fire).toHaveBeenCalledTimes(1);
  });
});

describe("Overlay", () => {
  it("shows on a turn and hides two seconds after the last one", () => {
    const seen: boolean[] = [];
    const o = new Overlay((v) => seen.push(v));
    o.poke();
    vi.advanceTimersByTime(OVERLAY_MS - 500);
    o.poke();
    vi.advanceTimersByTime(OVERLAY_MS - 1);
    expect(o.visible).toBe(true);
    vi.advanceTimersByTime(1);
    expect(o.visible).toBe(false);
    expect(seen).toEqual([true, false]);
    o.poke();
    o.hide();
    expect(seen).toEqual([true, false, true, false]);
  });
});
