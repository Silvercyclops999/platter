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
import { SPIN_DEG_PER_S, SPIN_EASE_MS, Turntable, nextSpeed } from "../shell/Turntable";
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
  it("turns while playing, slows to a stop on pause, and the arm follows the play state", () => {
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

  it("holds the record still under reduced motion", () => {
    setReducedMotion(true);
    const raf = manualFrames();
    const { container } = render(<Turntable thumb={null} playing />);
    raf.frames(20);
    expect(angle(container.querySelector(".turntable__record")!)).toBe(0);
    // The arm swing, the sleeve slide and the needle drop become fades.
    expect(turntableCss).toMatch(/\[data-reduced-motion="true"\] \.turntable__arm \{ animation: tt-fade/);
    expect(identityCss).toMatch(/\[data-reduced-motion="true"\]\[data-identity\] \.empty-state::after/);
    expect(identityCss).toMatch(/\[data-reduced-motion="true"\]\[data-identity\] \.sleeve-disc \{ animation: identity-fade/);
  });

  it("the record and arm are decoration: hidden from assistive tech", () => {
    const { container } = render(<Turntable thumb={null} playing={false} />);
    expect(container.querySelector(".turntable__arm")).toHaveAttribute("aria-hidden", "true");
    expect(container.querySelector(".turntable__platter")).toHaveAttribute("aria-hidden", "true");
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
