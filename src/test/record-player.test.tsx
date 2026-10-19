/**
 * The record player in expanded now playing, the tonearm seek bar and the
 * vinyl and sleeve cover styles.
 */
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import identityCss from "../design/identity.css?raw";
import looksCss from "../design/looks/looks.css?raw";
import turntableCss from "../shell/turntable.css?raw";
import { DEFAULT_PARTS, applyLook, lookAttributes } from "../design/looks/apply";
import { setReducedMotion } from "../design/motion";
import { CLASSIC_LAYOUT, validateTheme } from "../design/themes";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { FullWindow } from "../shell/FullWindow";
import { PlayerBar } from "../shell/PlayerBar";
import { SPIN_DEG_PER_S, SPIN_EASE_MS, Turntable, armAngle, nextSpeed } from "../shell/Turntable";
import { Vinyl, grooveRings } from "../components/Record";
import {
  ARM_LENGTH,
  ARM_MOUNT,
  CENTRE,
  LABEL_R,
  LOCKED_R,
  MUSIC_MIN_R,
  MUSIC_START_R,
  PIVOT,
  RECORD_R,
  REST_DEG,
  SPINDLE_HOLE_R,
  SPINDLE_R,
  grooveRadius,
  innerMusicRadius,
  splitSides,
  stylusAt,
  trackBands,
} from "../shell/recordGeometry";
import { sideFor } from "../shell/useRecordSide";
import { fullWindow, useFullWindow, FULL_WINDOW_PREFS_DEFAULT } from "../stores/fullWindow";
import { connectPlayer, usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";

const mock = backend as MockBackend;

/** Drive requestAnimationFrame by hand: each `frames(n)` runs n frames 16 ms apart. */
function manualFrames() {
  let queue: FrameRequestCallback[] = [];
  let now = 1000;
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
    queue.push(cb);
    return queue.length;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
  return {
    frames(n: number) {
      for (let i = 0; i < n; i++) {
        const run = queue;
        queue = [];
        now += 16;
        act(() => run.forEach((cb) => cb(now)));
      }
    },
    pending: () => queue.length,
  };
}

const angle = (el: Element) => Number(/rotate\(([-\d.]+)deg\)/.exec((el as HTMLElement).style.transform)?.[1] ?? 0);

afterEach(() => {
  vi.restoreAllMocks();
  setReducedMotion(false);
});

describe("platter motor", () => {
  it("spins up to 33 1/3 rpm in about a second, easing, and settles exactly", () => {
    let speed = 0;
    let t = 0;
    let atQuarter = 0;
    while (speed !== SPIN_DEG_PER_S && t < 5000) {
      speed = nextSpeed(speed, SPIN_DEG_PER_S, 16);
      t += 16;
      if (t <= SPIN_EASE_MS / 4) atQuarter = speed;
    }
    expect(SPIN_DEG_PER_S).toBeCloseTo(200, 0);
    // Most of the way after a quarter of the time, there by about a second and a half.
    expect(atQuarter / SPIN_DEG_PER_S).toBeGreaterThan(0.5);
    expect(atQuarter).toBeLessThan(SPIN_DEG_PER_S);
    expect(t).toBeLessThanOrEqual(SPIN_EASE_MS * 1.6);
    // And down again to a stop.
    let down = SPIN_DEG_PER_S;
    let n = 0;
    while (down !== 0 && n < 400) {
      down = nextSpeed(down, 0, 16);
      n++;
    }
    expect(down).toBe(0);
    expect(n * 16).toBeLessThanOrEqual(SPIN_EASE_MS * 1.6);
  });
});

describe("turntable", () => {
  it("turns at 33 1/3 while playing, slows to a stop on pause, and the arm follows the play state", () => {
    const raf = manualFrames();
    const { container, rerender } = render(<Turntable thumb={null} playing />);
    const tt = container.querySelector(".turntable")!;
    const record = container.querySelector(".turntable__record")!;
    expect(tt).toHaveAttribute("data-state", "playing");
    raf.frames(30);
    const a1 = angle(record);
    raf.frames(10);
    expect(angle(record)).toBeGreaterThan(a1);

    rerender(<Turntable thumb={null} playing={false} />);
    expect(tt).toHaveAttribute("data-state", "paused");
    raf.frames(200);
    // Stopped: the loop ends and the record holds still.
    expect(raf.pending()).toBe(0);
    const held = angle(record);
    raf.frames(5);
    expect(angle(record)).toBe(held);
  });

  it("holds the record still under reduced motion, and the arm is simply in its place", () => {
    setReducedMotion(true);
    const raf = manualFrames();
    const { container } = render(<Turntable thumb={null} playing />);
    raf.frames(20);
    expect(angle(container.querySelector(".turntable__record")!)).toBe(0);
    expect(raf.pending()).toBe(0);
    expect(turntableCss).toMatch(/\[data-reduced-motion="true"\] :is\(\.turntable__arm, \.turntable__arm-shadow\) \{ transition: none; \}/);
    expect(identityCss).toMatch(/\[data-reduced-motion="true"\]\[data-identity\] \.empty-state::after/);
  });

  it("holds the record still with the record player setting off; the arm still plays", () => {
    const raf = manualFrames();
    const { container } = render(<Turntable thumb={null} playing spin={false} track={{ id: "t", albumId: "a", durationMs: 200_000 }} />);
    raf.frames(20);
    expect(angle(container.querySelector(".turntable__record")!)).toBe(0);
    expect(raf.pending()).toBe(0);
    expect(Number(container.querySelector(".turntable__arm")!.getAttribute("data-angle"))).not.toBe(REST_DEG);
  });

  it("paused, nothing animates: no frame is asked for and the arm is on its rest", () => {
    const raf = manualFrames();
    const { container } = render(<Turntable thumb={null} playing={false} track={{ id: "t", albumId: "a", durationMs: 200_000 }} />);
    expect(raf.pending()).toBe(0);
    raf.frames(5);
    expect(raf.pending()).toBe(0);
    expect(angle(container.querySelector(".turntable__record")!)).toBe(0);
    expect(Number(container.querySelector(".turntable__arm")!.getAttribute("data-angle"))).toBe(REST_DEG);
  });

  it("the drawing is decoration: hidden from assistive tech", () => {
    const { container } = render(<Turntable thumb={null} playing={false} />);
    expect(container.querySelector("svg.turntable__svg")).toHaveAttribute("aria-hidden", "true");
    expect(container.querySelector(".turntable__arm")!.closest("[aria-hidden=true]")).not.toBeNull();
  });
});

describe("the record, to scale", () => {
  it("is a 12-inch record: 302 mm across, a 100 mm label, a 7.26 mm spindle hole", () => {
    expect(RECORD_R * 2).toBe(302);
    expect(LABEL_R * 2).toBe(100);
    expect((LABEL_R * 2) / (RECORD_R * 2)).toBeCloseTo(0.331, 3);
    expect(SPINDLE_HOLE_R * 2).toBeCloseTo(7.26, 2);
    // The spindle is a hair under the hole.
    expect(SPINDLE_R).toBeLessThan(SPINDLE_HOLE_R);
    expect(SPINDLE_R * 2).toBeGreaterThan(7.1);
    // Music from 292.1 mm, never tighter than 120.6 mm; the locked groove at 106.4 mm.
    expect(MUSIC_START_R * 2).toBeCloseTo(292.1, 1);
    expect(MUSIC_MIN_R * 2).toBeCloseTo(120.6, 1);
    expect(LOCKED_R * 2).toBeCloseTo(106.4, 1);
    // Drawn that way: the vinyl, and the label cut round at its size.
    const { container } = render(<Turntable thumb={null} playing={false} />);
    expect(container.querySelector(".record__vinyl")).toHaveAttribute("r", String(RECORD_R));
    expect(container.querySelector("clipPath circle")).toHaveAttribute("r", String(LABEL_R));
    // No hard ring round the label: nothing light is stroked at its edge.
    const edge = [...container.querySelectorAll(".record__label circle")].filter((c) => c.getAttribute("stroke") === "#fff");
    expect(edge).toHaveLength(0);
  });

  it("places the gaps between songs where the songs start, from their lengths", () => {
    const bands = trackBands([60_000, 120_000, 60_000]);
    const total = 240_000;
    expect(bands).toHaveLength(3);
    expect(bands[0].outer).toBeCloseTo(MUSIC_START_R, 6);
    expect(bands[1].outer).toBeCloseTo(grooveRadius(0.25, total), 6);
    expect(bands[2].outer).toBeCloseTo(grooveRadius(0.75, total), 6);
    expect(bands[2].inner).toBeCloseTo(innerMusicRadius(total), 6);
    // Each band runs into the next.
    expect(bands[0].inner).toBeCloseTo(bands[1].outer, 6);
    // A song twice as long takes twice the width.
    expect(bands[1].outer - bands[1].inner).toBeCloseTo(2 * (bands[0].outer - bands[0].inner), 6);
    // A short side ends further out (a wider run-out); a long one is cut to the tightest radius.
    expect(innerMusicRadius(4 * 60_000)).toBeGreaterThan(innerMusicRadius(18 * 60_000));
    expect(innerMusicRadius(30 * 60_000)).toBeCloseTo(MUSIC_MIN_R, 6);

    const { container } = render(
      <svg>
        <Vinyl side={{ durationsMs: [60_000, 120_000, 60_000], current: 1 }} label={{}} seed="x" />
      </svg>,
    );
    expect(container.querySelectorAll(".record__gap")).toHaveLength(2);
    // The playing song's band is marked over its own radii.
    const now = container.querySelector(".record__now")!;
    expect(Number(now.getAttribute("r"))).toBeCloseTo((bands[1].outer + bands[1].inner) / 2, 3);
  });

  it("the grooves are uneven, not a set of equal rings", () => {
    const rings = grooveRings("a record", innerMusicRadius(20 * 60_000), "full");
    const fine = rings.filter((r) => r.w < 1).map((r) => r.r);
    const gaps = fine.slice(1).map((r, i) => r - fine[i]);
    expect(new Set(gaps.map((g) => g.toFixed(2))).size).toBeGreaterThan(gaps.length / 2);
    // And the same record draws the same way each time.
    expect(grooveRings("a record", 80, "full")).toEqual(grooveRings("a record", 80, "full"));
  });

  it("the arm follows the music from the outer groove to the inner, accurately", () => {
    const side = { durationsMs: [200_000, 300_000, 100_000], current: 0 };
    const total = 600_000;
    const at = (current: number, pos: number) => armAngle(true, { ...side, current }, pos);
    const start = at(0, 0);
    const middle = at(1, 100_000);
    const end = at(2, 100_000);
    const dist = (deg: number) => {
      const s = stylusAt(deg);
      return Math.hypot(s.x - CENTRE.x, s.y - CENTRE.y);
    };
    // The stylus lands on the first groove, half way is half way in, and the end is the last groove.
    expect(dist(start)).toBeCloseTo(MUSIC_START_R, 6);
    expect(dist(middle)).toBeCloseTo(grooveRadius(0.5, total), 6);
    expect(dist(end)).toBeCloseTo(innerMusicRadius(total), 6);
    // It swings in as the side plays, and stays off its rest.
    expect(start).toBeLessThan(middle);
    expect(middle).toBeLessThan(end);
    expect(start).toBeGreaterThan(REST_DEG);
    // A 9-inch arm over a 12-inch record: about 20 to 30 degrees across the music.
    expect(armAngle(true, { durationsMs: [22 * 60_000], current: 0 }, 22 * 60_000) - armAngle(true, { durationsMs: [22 * 60_000], current: 0 }, 0)).toBeGreaterThan(18);
    // Stopped, it's on its rest.
    expect(armAngle(false, side, 50_000)).toBe(REST_DEG);
    // The real arm's numbers: pivot 212 mm from the spindle, 228.5 mm to the stylus.
    expect(Math.hypot(PIVOT.x - CENTRE.x, PIVOT.y - CENTRE.y)).toBeCloseTo(ARM_MOUNT, 6);
    expect(ARM_LENGTH - ARM_MOUNT).toBeGreaterThan(14);
    expect(ARM_LENGTH - ARM_MOUNT).toBeLessThan(19);
  });

  it("finds the playing song's side: the album's own sides, or split at a real side's length", () => {
    const songs = [
      { id: "a", durationMs: 9 * 60_000, discNumber: 1 },
      { id: "b", durationMs: 9 * 60_000, discNumber: 1 },
      { id: "c", durationMs: 9 * 60_000, discNumber: 1 },
      { id: "d", durationMs: 9 * 60_000, discNumber: 1 },
      { id: "e", durationMs: 5 * 60_000, discNumber: 2 },
    ];
    expect(sideFor(songs, "c")).toEqual({ durationsMs: [9 * 60_000, 9 * 60_000], current: 0 });
    expect(sideFor(songs, "b")).toEqual({ durationsMs: [9 * 60_000, 9 * 60_000], current: 1 });
    expect(sideFor(songs, "e")).toEqual({ durationsMs: [5 * 60_000], current: 0 });
    expect(sideFor(songs, "b", [{ trackIds: ["a", "b", "c"] }, { trackIds: ["d"] }])).toEqual({ durationsMs: [9 * 60_000, 9 * 60_000, 9 * 60_000], current: 1 });
    expect(splitSides([10, 10, 10].map((m) => m * 60_000))).toEqual([[0, 1], [2]]);
    expect(sideFor(songs, "zzz")).toBeNull();
  });
});

describe("expanded now playing", () => {
  let disconnect: () => void;
  beforeAll(async () => {
    disconnect = connectPlayer(mock);
    await waitFor(() => expect(usePlayer.getState().state?.track).toBeTruthy());
  });
  afterAll(() => disconnect());
  beforeEach(() => {
    useFullWindow.setState({ open: false, mode: "artwork", osFullscreen: false, prefs: { ...FULL_WINDOW_PREFS_DEFAULT } });
  });

  it("shows the record player by default and the plain cover when it's off", async () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
    render(<FullWindow />);
    act(() => fullWindow.open());
    expect(document.querySelector(".fw__now .turntable")).not.toBeNull();
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, recordMotion: "off" } }));
    expect(document.querySelector(".fw__now .turntable")).toBeNull();
    expect(document.querySelector(".fw__now .fw__cover")).not.toBeNull();
  });
});

describe("tonearm seek bar", () => {
  let disconnect: () => void;
  beforeAll(async () => {
    disconnect = connectPlayer(mock);
    await waitFor(() => expect(usePlayer.getState().state?.track).toBeTruthy());
  });
  afterAll(() => disconnect());

  it("is still a slider with a name, value and keys", async () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, tonearmSeek: true } });
    const user = userEvent.setup();
    const seek = vi.spyOn(backend, "playSeek").mockResolvedValue(undefined);
    render(<PlayerBar />);
    act(() => usePlayer.getState().setPosition(60_000, 200_000));
    const s = screen.getByRole("slider", { name: "Seek" });
    expect(s).toHaveClass("slider--tonearm");
    expect(s).toHaveAttribute("aria-valuenow", "60000");
    expect(s).toHaveAttribute("aria-valuetext", "1:00 of 3:20");
    expect(s).toHaveAttribute("tabindex", "0");
    s.focus();
    await user.keyboard("{ArrowRight}");
    expect(seek).toHaveBeenLastCalledWith(65_000);
  });

  it("off, it's the plain bar", () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, tonearmSeek: false } });
    render(<PlayerBar />);
    expect(screen.getByRole("slider", { name: "Seek" })).not.toHaveClass("slider--tonearm");
  });
});

describe("cover styles", () => {
  it("vinyl and sleeve are layout choices a look can carry, and are drawn", () => {
    for (const shape of ["square", "rounded", "circle", "vinyl", "sleeve"] as const) {
      const r = validateTheme("x.platter-look", { version: 1, name: "X", base: "dark", layout: { coverShape: shape } });
      expect(r.ok, shape).toBe(true);
      expect(lookAttributes({ ...DEFAULT_PARTS, layout: { ...CLASSIC_LAYOUT, coverShape: shape } }).data.coverShape).toBe(shape);
    }
    expect(validateTheme("x.platter-look", { version: 1, name: "X", base: "dark", layout: { coverShape: "star" } }).ok).toBe(false);
    const el = document.createElement("div");
    applyLook(el, { ...DEFAULT_PARTS, layout: { ...CLASSIC_LAYOUT, coverShape: "vinyl" } });
    expect(el.dataset.lookCoverShape).toBe("vinyl");
    // Each has its own rules: the spindle hole, and the record's edge beside the sleeve.
    expect(looksCss).toMatch(/data-look-cover-shape="vinyl"\] \.art:not\(\.art--round\)::after/);
    expect(looksCss).toMatch(/data-look-cover-shape="sleeve"\] \.card \.art:not\(\.art--round\)::before/);
  });
});
