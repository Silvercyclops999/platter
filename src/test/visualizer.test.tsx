/**
 * The visualizer: the feed that asks the core to watch only while a view is
 * on screen, the signals the styles move with, Grooves, Blur and Bars drawn
 * from mock frames (and the CSS layers without WebGL), reduced motion and
 * music playing elsewhere, the phone's tap on the cover and swipe for the
 * style, the expanded view's cover click, arrow keys, V and Escape, and the
 * one Settings row.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setReducedMotion } from "../design/motion";
import { rgbToHsl } from "../lib/color";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockFrame } from "../lib/mock/visualizer";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { NowPlaying as PhoneNowPlaying } from "../phone/NowPlaying";
import { usePhoneUi } from "../phone/phoneUi";
import { DeckFaceplate } from "../shell/DeckFrame";
import { FullWindow } from "../shell/FullWindow";
import { FULL_WINDOW_PREFS_DEFAULT, fullWindow, useFullWindow } from "../stores/fullWindow";
import { player, usePlayer } from "../stores/player";
import { loadSettings, useSettings } from "../stores/settings";
import { ArtGL, type ArtFrame } from "../visualizer/art/ArtGL";
import { latestFrame, resetFeed, useFeed, watchFeed } from "../visualizer/feed";
import { paletteFromPixels } from "../visualizer/palette";
import { setPhoneVis, usePhoneVis } from "../visualizer/phoneVis";
import { BANDS, BAR_MAX, BarLevels, barBands, current, GAP_MS } from "../visualizer/scale";
import { LOW_BANDS, MusicSignals } from "../visualizer/signals";
import { MAX_RENDER_PIXELS, PHONE_RENDER_CAP, Stage, barColours, renderScale, stillLine } from "../visualizer/Stage";
import { DEFAULT_STYLE, normalizeStyle, stepStyle, STYLES } from "../visualizer/styles";

const mock = backend as MockBackend;

// ------------------------------------------------------------ test rig

/** A 2D context that records what was drawn. */
function fakeCtx() {
  const calls: string[] = [];
  const rec =
    (name: string) =>
    (..._args: unknown[]) =>
      void calls.push(name);
  return {
    calls,
    ctx: {
      setTransform: rec("setTransform"),
      clearRect: rec("clearRect"),
      beginPath: rec("beginPath"),
      moveTo: rec("moveTo"),
      lineTo: rec("lineTo"),
      stroke: rec("stroke"),
      fill: rec("fill"),
      arc: rec("arc"),
      rect: rec("rect"),
      clip: rec("clip"),
      save: rec("save"),
      restore: rec("restore"),
      fillRect: rec("fillRect"),
      drawImage: rec("drawImage"),
      roundRect: rec("roundRect"),
      createLinearGradient: () => ({ addColorStop: () => {} }),
      set globalAlpha(_v: number) {},
      set strokeStyle(v: string) {
        calls.push(`strokeStyle:${v}`);
      },
      set fillStyle(_v: string) {},
      set lineWidth(_v: number) {},
      set lineJoin(_v: string) {},
      set lineCap(_v: string) {},
    },
  };
}

let drawn: ReturnType<typeof fakeCtx>;
let gl: Record<string, unknown> | null = null;

/** Drive animation frames by hand. */
function frames() {
  let queue: FrameRequestCallback[] = [];
  // A clock that moves 20 ms a frame, which frames from the core read too.
  let now = 10_000;
  vi.spyOn(performance, "now").mockImplementation(() => now);
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
    queue.push(cb);
    return queue.length;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
  return (n: number) => {
    for (let i = 0; i < n; i++) {
      const run = queue;
      queue = [];
      now += 20;
      act(() => run.forEach((cb) => cb(now)));
    }
  };
}

/** A WebGL2 context that builds everything and records its draws. */
function fakeGl(drawArrays = vi.fn(), extra: Record<string, unknown> = {}) {
  const noop = () => {};
  return new Proxy(
    {
      COMPILE_STATUS: 1,
      LINK_STATUS: 2,
      TRIANGLE_STRIP: 5,
      isContextLost: () => false,
      getShaderParameter: () => true,
      getProgramParameter: () => true,
      createShader: () => ({}),
      createProgram: () => ({}),
      createBuffer: () => ({}),
      createTexture: () => ({}),
      createVertexArray: () => ({}),
      getUniformLocation: () => ({}),
      getAttribLocation: () => 0,
      drawArrays,
      ...extra,
    } as Record<string, unknown>,
    { get: (t, k: string) => (k in t ? t[k] : noop) },
  );
}

/** The last watch the mock was asked for. */
const lastCall = () => mock.visualizer.calls[mock.visualizer.calls.length - 1];

function track(): Track {
  return mock.fx.tracks[0];
}

function playing(t: Track = track()) {
  const s: PlayerState = {
    status: "playing",
    track: t,
    positionMs: 1000,
    durationMs: t.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: null, userQueue: [], nextUp: [], history: [], context: { kind: "album", id: t.albumId, title: t.albumTitle } },
  };
  act(() => usePlayer.setState({ state: s, remote: null }));
}

/** A frame from the core, as if it had just arrived. */
function frame(ms = 1000, scope = true) {
  act(() => mock.emit("player://spectrum", mockFrame(ms, scope)));
}

function setHidden(hidden: boolean) {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (hidden ? "hidden" : "visible") });
  Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
  act(() => void document.dispatchEvent(new Event("visibilitychange")));
}

beforeEach(async () => {
  resetFeed();
  mock.visualizer.stop();
  mock.visualizer.calls = [];
  localStorage.clear();
  setHidden(false);
  setReducedMotion(false);
  drawn = fakeCtx();
  gl = null;
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(((kind: string) =>
    kind === "2d" ? drawn.ctx : kind === "webgl2" ? gl : null) as unknown as HTMLCanvasElement["getContext"]);
  // Every box has room to draw in.
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({ width: 640, height: 320, x: 0, y: 0, top: 0, left: 0, right: 640, bottom: 320, toJSON: () => ({}) } as DOMRect);
  useFullWindow.setState({ open: false, mode: "artwork", osFullscreen: false, prefs: { ...FULL_WINDOW_PREFS_DEFAULT } });
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  playing();
});

afterEach(() => {
  resetFeed();
  mock.visualizer.stop();
  vi.restoreAllMocks();
  setReducedMotion(false);
  setHidden(false);
});


// ------------------------------------------------------------ styles and scales

describe("styles", () => {
  it("has Grooves, Blur and Bars, Grooves first and the default; anything older reads as Grooves", () => {
    expect(STYLES.map((s) => s.label)).toEqual(["Grooves", "Blur", "Bars"]);
    expect(DEFAULT_STYLE).toBe("grooves");
    expect(DEFAULT_SETTINGS.visualizerStyle).toBe("grooves");
    expect(DEFAULT_SETTINGS.visualizerCoverTap).toBe(true);
    for (const old of ["sleeve", "glow", "ink", "light", "analyser", "meters", "scope", "", null, undefined]) expect(normalizeStyle(old)).toBe("grooves");
    expect(normalizeStyle("bars")).toBe("bars");
  });

  it("steps through the styles both ways, wrapping round", () => {
    expect(stepStyle("grooves", 1)).toBe("blur");
    expect(stepStyle("blur", 1)).toBe("bars");
    expect(stepStyle("bars", 1)).toBe("grooves");
    expect(stepStyle("grooves", -1)).toBe("bars");
    expect(stepStyle("sleeve", 1)).toBe("blur");
  });

  it("falls to silence when frames stop instead of freezing", () => {
    expect(current({ a: 1 }, 0, GAP_MS)).toEqual({ a: 1 });
    expect(current({ a: 1 }, 0, GAP_MS + 1)).toBeNull();
  });

  it("gives every bar its own bands, rising fast, settling slowly, never past the top", () => {
    const ranges = barBands(20);
    expect(ranges).toHaveLength(20);
    for (let i = 0; i < ranges.length; i++) {
      expect(ranges[i][1]).toBeGreaterThan(ranges[i][0]);
      if (i) expect(ranges[i][0]).toBeGreaterThanOrEqual(ranges[i - 1][0]);
    }
    const bars = new BarLevels(20);
    const loud = new Array(BANDS).fill(0);
    for (let i = 0; i < 4; i++) bars.step(loud, 33);
    for (const v of bars.shown) {
      expect(v).toBeGreaterThan(0.5);
      expect(v).toBeLessThanOrEqual(BAR_MAX);
    }
    const top = bars.shown[0];
    bars.step(null, 33);
    // One frame later they have only begun to settle.
    expect(bars.shown[0]).toBeGreaterThan(top * 0.8);
    for (let i = 0; i < 60; i++) bars.step(null, 33);
    expect(Math.max(...bars.shown)).toBeLessThan(0.01);
    // A broken frame reads as silence.
    bars.step([NaN, Infinity] as unknown as number[], 33);
    for (const v of bars.shown) expect(Number.isFinite(v)).toBe(true);
  });

  it("lifts the high bars so a falling spectrum still moves the whole row", () => {
    const bars = new BarLevels(20);
    // Pink-ish: 3 dB quieter every octave.
    const pink = Array.from({ length: BANDS }, (_, k) => -20 - 3 * Math.log2((20 * Math.pow(1000, (k + 0.5) / BANDS)) / 1000));
    for (let i = 0; i < 10; i++) bars.step(pink, 33);
    const first = bars.shown[0];
    const last = bars.shown[bars.shown.length - 1];
    expect(Math.abs(first - last)).toBeLessThan(0.15);
  });

  it("colours the bars from the cover's lead, light enough to read over the dimmed cover", () => {
    const { top, bottom } = barColours([0.6, 0.1, 0.3]);
    for (const c of [top, bottom]) {
      const [r, g, b] = c.match(/\d+/g)!.map(Number);
      expect(rgbToHsl([r, g, b])[2]).toBeGreaterThan(0.55);
    }
  });

  it("caps the render scale on a phone, and the pixels on any screen", () => {
    expect(renderScale("grooves", 3, true, 430, 932)).toBeLessThanOrEqual(3 * PHONE_RENDER_CAP);
    expect(renderScale("blur", 3, true, 430, 932)).toBeLessThanOrEqual(1);
    expect(renderScale("grooves", 2, false, 800, 600)).toBe(2);
    expect(renderScale("blur", 2, false, 800, 600)).toBe(1);
    const big = renderScale("grooves", 2, false, 3840, 2160);
    expect(3840 * 2160 * big * big).toBeLessThanOrEqual(MAX_RENDER_PIXELS + 1);
  });

  it("says why the art holds still", () => {
    expect(stillLine(true, false)).toBeNull();
    expect(stillLine(false, false)).toMatch(/playing on this device/);
    expect(stillLine(true, true)).toMatch(/Reduce motion/);
    expect(stillLine(true, false, true, true)).toMatch(/Low Power Mode/);
    expect(stillLine(true, false, true, false)).toMatch(/on battery power/);
  });
});

// ------------------------------------------------------------ the feed

describe("the feed", () => {
  it("asks the core to watch while a view wants frames, renews it, and ends it when the view goes", async () => {
    vi.useFakeTimers();
    const stop = watchFeed({ fps: 60, scope: false });
    await vi.advanceTimersByTimeAsync(0);
    expect(lastCall()).toEqual({ on: true, fps: 60, scope: false });
    expect(useFeed.getState().running).toBe(true);
    // A second view asks for the scope: one watch, with the scope.
    const stop2 = watchFeed({ fps: 30, scope: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(lastCall()).toEqual({ on: true, fps: 60, scope: true });
    // Renewed about once a second, well inside the core's 3 s.
    const before = mock.visualizer.calls.length;
    await vi.advanceTimersByTimeAsync(2_100);
    expect(mock.visualizer.calls.length - before).toBe(2);
    stop2();
    stop();
    await vi.advanceTimersByTimeAsync(0);
    expect(lastCall()?.on).toBe(false);
    expect(mock.visualizer.watched).toBe(false);
    // Nothing more once nobody watches.
    const after = mock.visualizer.calls.length;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(mock.visualizer.calls.length).toBe(after);
  });

  it("stops while the page is hidden and starts again when it comes back", async () => {
    vi.useFakeTimers();
    const stop = watchFeed({ fps: 60, scope: false });
    await vi.advanceTimersByTimeAsync(0);
    setHidden(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(lastCall()?.on).toBe(false);
    const n = mock.visualizer.calls.length;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(mock.visualizer.calls.length).toBe(n);
    setHidden(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(lastCall()?.on).toBe(true);
    stop();
  });

  it("keeps the frames the core sends", async () => {
    const stop = watchFeed({ fps: 30, scope: false });
    frame(2_000, false);
    expect(latestFrame().frame?.bands).toHaveLength(BANDS);
    stop();
  });

  it("ends the watch even when the view goes before the first answer", async () => {
    let answer: (v: boolean) => void = () => {};
    const calls: boolean[] = [];
    vi.spyOn(backend, "visualizerWatch").mockImplementation((on) => {
      calls.push(on);
      return on ? new Promise<boolean>((r) => (answer = r)) : Promise.resolve(false);
    });
    const stop = watchFeed({ fps: 60, scope: false });
    stop();
    answer(true);
    await waitFor(() => expect(calls).toEqual([true, false]));
  });

  it("keeps one listener however quickly views come and go", async () => {
    const real = backend.listen.bind(backend);
    const live = new Set<number>();
    let n = 0;
    vi.spyOn(backend, "listen").mockImplementation(async (event, cb) => {
      // Slow to answer, as over IPC.
      await new Promise((r) => setTimeout(r, 5));
      const id = n++;
      live.add(id);
      const un = await real(event, cb);
      return () => {
        live.delete(id);
        un();
      };
    });
    // Mounted, unmounted and mounted again before the first listen answers.
    watchFeed({ fps: 60, scope: false })();
    const stop = watchFeed({ fps: 60, scope: false });
    await waitFor(() => expect(n).toBe(2));
    await new Promise((r) => setTimeout(r, 10));
    expect(live.size).toBe(1);
    stop();
    expect(live.size).toBe(0);
  });

  it("says when it rests on battery", async () => {
    vi.spyOn(backend, "visualizerWatch").mockResolvedValue(false);
    const stop = watchFeed({ fps: 60, scope: false });
    await waitFor(() => expect(useFeed.getState().resting).toBe(true));
    stop();
  });
});

// ------------------------------------------------------------ the signals

function bandsWith(low: number, rest = -50): number[] {
  const out = new Array(BANDS).fill(rest);
  for (let k = LOW_BANDS[0]; k < LOW_BANDS[1]; k++) out[k] = low;
  return out;
}

describe("the signals", () => {
  it("finds the beats in a kick every half second, and none in a steady tone", () => {
    const sig = new MusicSignals();
    let pulseAfterKick = 0;
    let pulseLater = 1;
    for (let t = 0; t < 8_000; t += 33) {
      const since = t % 500;
      sig.step(bandsWith(-44 + 34 * Math.exp(-since / 70)), {}, 33, t);
      if (t > 2_000 && since < 33) pulseAfterKick = Math.max(pulseAfterKick, sig.pulse);
      if (t > 2_000 && since >= 330 && since < 363) pulseLater = Math.min(pulseLater, sig.pulse);
    }
    expect(sig.beatCount).toBeGreaterThanOrEqual(15);
    expect(sig.beatCount).toBeLessThanOrEqual(16);
    expect(pulseAfterKick).toBeGreaterThan(0.3);
    expect(pulseLater).toBeLessThan(pulseAfterKick / 2);

    const steady = new MusicSignals();
    for (let t = 0; t < 4_000; t += 33) steady.step(bandsWith(-20), {}, 33, t);
    expect(steady.beatCount).toBe(0);
    expect(steady.pulse).toBe(0);
  });

  it("hears the kicks in the mock's song", () => {
    const sig = new MusicSignals();
    for (let t = 0; t < 8_000; t += 33) {
      const f = mockFrame(t, false);
      sig.step(f.bands, f, 33, t);
    }
    // 16 kicks and a few pickups in 8 seconds.
    expect(sig.beatCount).toBeGreaterThanOrEqual(14);
    expect(sig.beatCount).toBeLessThanOrEqual(24);
    expect(sig.low).toBeGreaterThan(0);
    expect(sig.high).toBeGreaterThan(0);
  });

  it("moves with the music and settles back when the frames stop", () => {
    const sig = new MusicSignals();
    let lowPeak = 0;
    for (let t = 0; t < 3_000; t += 33) {
      const f = mockFrame(t, false);
      sig.step(f.bands, f, 33, t);
      lowPeak = Math.max(lowPeak, sig.low);
    }
    expect(lowPeak).toBeGreaterThan(0.25);
    const flowBefore = sig.flow;
    for (let t = 3_000; t < 6_000; t += 33) sig.step(null, null, 33, t);
    expect(sig.low).toBeLessThan(0.02);
    expect(sig.pulse).toBeLessThan(0.01);
    expect(sig.flow).toBeGreaterThan(flowBefore);
  });

  it("never lets a broken frame poison the signals", () => {
    const sig = new MusicSignals();
    const broken = [NaN, undefined, Infinity, -Infinity] as unknown as number[];
    for (let t = 0; t < 1_000; t += 33) sig.step(broken, {}, 33, t);
    for (let t = 1_000; t < 2_000; t += 33) sig.step([-20, -30], {}, 33, t);
    for (const v of [sig.low, sig.mid, sig.high, sig.pulse, sig.flow, sig.drift]) expect(Number.isFinite(v)).toBe(true);
    for (let t = 2_000; t < 6_000; t += 33) sig.step(bandsWith(-44 + 34 * Math.exp(-(t % 500) / 70)), {}, 33, t);
    expect(sig.beatCount).toBeGreaterThanOrEqual(6);
  });

  it("reads four colours and a deep base from a cover, and quiet ones from a grey cover", () => {
    const px: number[] = [];
    for (let i = 0; i < 256; i++) {
      const c = i < 120 ? [200, 40, 30] : i < 200 ? [30, 60, 190] : i < 230 ? [20, 18, 16] : [230, 220, 200];
      px.push(c[0], c[1], c[2], 255);
    }
    const p = paletteFromPixels(px);
    expect(p.colours).toHaveLength(4);
    const hues = p.colours.slice(0, 2).map((c) => rgbToHsl([c[0] * 255, c[1] * 255, c[2] * 255])[0]);
    expect(Math.min(hues[0], 360 - hues[0])).toBeLessThan(25);
    expect(hues[1]).toBeGreaterThan(200);
    expect(hues[1]).toBeLessThan(250);
    const grey = paletteFromPixels(new Array(256).fill(0).flatMap((_, i) => [i % 200, i % 200, i % 200, 255]));
    for (const c of grey.colours) expect(rgbToHsl([c[0] * 255, c[1] * 255, c[2] * 255])[1]).toBeLessThanOrEqual(0.17);
  });
});

// ------------------------------------------------------------ the stage

describe("the stage", () => {
  const coverEl = () => document.querySelector<HTMLElement>(".test-cover");
  function stage(props: Partial<Parameters<typeof Stage>[0]> = {}) {
    return render(
      <>
        <div className="test-cover" />
        <Stage style="grooves" cover={coverEl} {...props} />
      </>,
    );
  }
  const kick = (ms: number) => act(() => mock.emit("player://spectrum", { t: ms, bands: bandsWith(-44 + 34 * Math.exp(-(ms % 500) / 70)), rms: [-20, -20], peak: [-6, -6] }));

  for (const style of ["grooves", "blur"] as const)
    it(`draws ${style} on the GPU from fake frames`, async () => {
      const run = frames();
      const drawArrays = vi.fn();
      gl = fakeGl(drawArrays);
      stage({ style });
      const s = screen.getByTestId("vis-stage");
      await waitFor(() => expect(s).toHaveAttribute("data-renderer", "webgl2"));
      expect(s).toHaveAttribute("data-style", style);
      for (let i = 0; i < 6; i++) {
        kick(i * 20);
        run(1);
      }
      expect(drawArrays).toHaveBeenCalledWith(5, 0, 4);
      expect(drawArrays.mock.calls.length).toBeGreaterThanOrEqual(5);
    });

  it("draws Bars on a 2D canvas, one rounded bar each, over the cover blurred", async () => {
    const run = frames();
    const rects: number[][] = [];
    (drawn.ctx as Record<string, unknown>).roundRect = (...a: number[]) => void rects.push(a);
    gl = fakeGl();
    stage({ style: "bars" });
    const s = screen.getByTestId("vis-stage");
    expect(s.querySelector(".vis-stage__backdrop")).not.toBeNull();
    expect(s.querySelector("canvas.vis-stage__bars")).not.toBeNull();
    for (let i = 0; i < 6; i++) {
      kick(i * 20);
      run(1);
    }
    expect(drawn.calls).toContain("fill");
    // 32 bars a frame on a computer.
    expect(rects.length % 32).toBe(0);
    expect(rects.length).toBeGreaterThan(0);
    const lastFrame = rects.slice(-32);
    // The bass bars stand taller than their rest (a bar is never less than its width).
    expect(Math.max(...lastFrame.map((r) => r[3]))).toBeGreaterThan(lastFrame[0][2]);
  });

  it("draws with CSS layers after the GPU context is lost, and when there is no WebGL2, still moving", async () => {
    const run = frames();
    gl = fakeGl();
    const { unmount } = stage({ style: "blur" });
    const s = screen.getByTestId("vis-stage");
    await waitFor(() => expect(s).toHaveAttribute("data-renderer", "webgl2"));
    act(() => void s.querySelector("canvas")!.dispatchEvent(new Event("webglcontextlost", { cancelable: true })));
    await waitFor(() => expect(s).toHaveAttribute("data-renderer", "css"));
    expect(s.querySelectorAll(".vis-css__field")).toHaveLength(4);
    unmount();
    gl = null;
    stage({ style: "grooves" });
    const t = screen.getByTestId("vis-stage");
    await waitFor(() => expect(t).toHaveAttribute("data-renderer", "css"));
    expect(t.querySelector(".vis-css__grooves")).not.toBeNull();
    run(1);
    const field = t.querySelector<HTMLElement>(".vis-css__field--0")!;
    const before = field.style.transform;
    for (let i = 0; i < 30; i++) {
      kick(i * 20);
      run(1);
    }
    expect(field.style.transform).not.toBe(before);
  });

  it("asks the core for frames at 60 on a computer and 30 on a phone", async () => {
    const { unmount } = stage();
    await waitFor(() => expect(lastCall()).toEqual({ on: true, fps: 60, scope: false }));
    unmount();
    stage({ phone: true });
    await waitFor(() => expect(lastCall()).toEqual({ on: true, fps: 30, scope: false }));
  });

  it("breathes the sharp cover with the beat and turns the record", async () => {
    const run = frames();
    gl = fakeGl();
    stage();
    run(1);
    const cover = coverEl()!;
    let most = 1;
    for (let i = 0; i < 60; i++) {
      kick(10_000 + i * 20);
      run(1);
      most = Math.max(most, Number(cover.style.getPropertyValue("--vis-breath")));
    }
    expect(most).toBeGreaterThan(1.005);
    expect(most).toBeLessThan(1.06);
    expect(parseFloat(cover.style.getPropertyValue("--vis-spin"))).toBeGreaterThan(0);
  });

  it("holds still under reduced motion: no breath, nothing asked of the core, and says why", async () => {
    setReducedMotion(true);
    const run = frames();
    stage({ style: "blur" });
    const s = screen.getByTestId("vis-stage");
    expect(s).toHaveAttribute("data-still", "true");
    await waitFor(() => expect(s).toHaveAttribute("data-renderer", "css"));
    run(1);
    const field = s.querySelector<HTMLElement>(".vis-css__field--1")!;
    const before = field.style.transform;
    for (let i = 0; i < 40; i++) {
      kick(i * 20);
      run(1);
    }
    expect(coverEl()!.style.getPropertyValue("--vis-breath")).toBe("1.0000");
    expect(field.style.transform).toBe(before);
    expect(mock.visualizer.calls).toEqual([]);
    expect(s).toHaveTextContent("Holding still while Reduce motion is on.");
  });

  it("holds still with one line for music playing on another device", async () => {
    act(() => usePlayer.setState({ remote: { id: "tv", name: "Living room" } as never }));
    stage();
    const s = screen.getByTestId("vis-stage");
    expect(s).toHaveAttribute("data-still", "true");
    expect(s).toHaveTextContent("The visualizer moves with music playing on this device.");
    await new Promise((r) => setTimeout(r, 20));
    expect(mock.visualizer.calls).toEqual([]);
  });

  it("holds still with one line while the core rests on battery, and keeps asking so it moves again", async () => {
    const watch = vi.spyOn(backend, "visualizerWatch").mockResolvedValue(false);
    stage();
    const s = screen.getByTestId("vis-stage");
    await waitFor(() => expect(s).toHaveAttribute("data-still", "true"));
    expect(s).toHaveTextContent("Resting while on battery power.");
    const asked = watch.mock.calls.length;
    watch.mockResolvedValue(true);
    await waitFor(() => expect(s).not.toHaveAttribute("data-still"), { timeout: 2_500 });
    expect(watch.mock.calls.length).toBeGreaterThan(asked);
    expect(s).not.toHaveTextContent("Resting");
  });

  it("gives the cover back as it was when it goes", async () => {
    const run = frames();
    gl = fakeGl();
    const { unmount } = stage();
    run(3);
    const cover = coverEl()!;
    expect(cover.style.getPropertyValue("--vis-breath")).not.toBe("");
    unmount();
    expect(cover.style.getPropertyValue("--vis-breath")).toBe("");
    expect(cover.style.getPropertyValue("--vis-spin")).toBe("");
  });

  it("draws with WebGL2 under StrictMode's second mount, on a canvas of its own", async () => {
    gl = fakeGl();
    render(
      <StrictMode>
        <Stage style="grooves" cover={() => null} />
      </StrictMode>,
    );
    const s = screen.getByTestId("vis-stage");
    await waitFor(() => expect(s).toHaveAttribute("data-renderer", "webgl2"));
    expect(s.querySelectorAll("canvas")).toHaveLength(1);
  });
});

describe("the GPU renderer", () => {
  const loseContext = vi.fn();
  const artFrame = (): ArtFrame => ({ time: 0, spin: 0, low: 0, mid: 0, high: 0, pulse: 0, drift: 0, flow: 0, label: [5, 5, 4], beatAt: new Float32Array(4), beatStrength: new Float32Array(4) });
  beforeEach(() => loseContext.mockClear());

  it("builds a program for Grooves and one for Blur, and draws each with its own", () => {
    const used: unknown[] = [];
    let n = 0;
    gl = fakeGl(vi.fn(), { createProgram: () => ({ id: n++ }), useProgram: (p: unknown) => void used.push(p) });
    const r = ArtGL.create(document.createElement("canvas"))!;
    expect(r.ready()).toBe(true);
    r.draw("grooves", artFrame(), 0, 10, 10);
    r.draw("blur", artFrame(), 0, 10, 10);
    expect(used).toHaveLength(2);
    expect(used[0]).not.toBe(used[1]);
  });

  it("gives the GPU context back when it goes, and when the art can't build", () => {
    const ext = { getExtension: (n: string) => (n === "WEBGL_lose_context" ? { loseContext } : null) };
    gl = fakeGl(vi.fn(), ext);
    const r = ArtGL.create(document.createElement("canvas"))!;
    r.dispose();
    expect(loseContext).toHaveBeenCalledTimes(1);
    gl = fakeGl(vi.fn(), { ...ext, getProgramParameter: () => false });
    expect(ArtGL.create(document.createElement("canvas"))).toBeNull();
    expect(loseContext).toHaveBeenCalledTimes(2);
  });
});

describe("the Deck faceplate", () => {
  it("has no meters of its own any more", () => {
    act(() => useFullWindow.setState({ mode: "visualizer" }));
    render(<DeckFaceplate />);
    expect(screen.getByTestId("deck-faceplate").querySelector(".deck-meters, .has-meters")).toBeNull();
  });
});

// ------------------------------------------------------------ the phone

describe("the phone", () => {
  afterEach(() => {
    setPhoneVis(false);
    act(() => usePhoneUi.setState({ nowPlaying: false }));
  });
  function open() {
    act(() => usePhoneUi.setState({ nowPlaying: true }));
    render(<PhoneNowPlaying />);
    return screen.getByRole("dialog", { name: "Now playing" });
  }
  function swipe(el: HTMLElement, dx: number) {
    let now = 1000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    fireEvent.pointerDown(el, { pointerId: 1, button: 0, clientX: 300, clientY: 400 });
    for (let i = 1; i <= 5; i++) {
      now += 160;
      fireEvent.pointerMove(el, { pointerId: 1, clientX: 300 + (dx / 5) * i, clientY: 400 });
    }
    fireEvent.pointerUp(el, { pointerId: 1, clientX: 300 + dx, clientY: 400 });
  }
  const tapOn = (el: HTMLElement) => {
    fireEvent.pointerDown(el, { pointerId: 2, button: 0, clientX: 200, clientY: 300 });
    fireEvent.pointerUp(el, { pointerId: 2, clientX: 202, clientY: 301 });
    fireEvent.click(el);
  };

  it("opens the visualizer with a tap on the cover, over the whole screen, and closes it with another", async () => {
    const np = open();
    expect(np).not.toHaveClass("is-art");
    // No button for it: the cover is the way in.
    expect(screen.queryByRole("button", { name: "Visualizer" })).toBeNull();
    const art = screen.getByRole("button", { name: "Open the visualizer" });
    tapOn(art);
    expect(np).toHaveClass("is-art", "is-art--grooves");
    const stage = within(np).getByTestId("vis-stage");
    expect(stage.querySelector(".vis-stage__scrim--top")).not.toBeNull();
    expect(np.querySelector(".ph-np__slide--cur .ph-np__cover")).toHaveClass("vis-cover", "vis-cover--grooves");
    // The style's name shows a moment; there is no row of style tabs.
    expect(within(np).getByTestId("vis-name")).toHaveTextContent("Grooves");
    expect(np.querySelector(".vis__styles, [role=group][aria-label='Visualizer style']")).toBeNull();
    // The song and the controls stay; the tap on the cover didn't hide them.
    expect(within(np).getByRole("button", { name: "Pause" })).toBeInTheDocument();
    expect(np).not.toHaveClass("is-bare");
    await waitFor(() => expect(lastCall()).toEqual({ on: true, fps: 30, scope: false }));
    expect(usePhoneVis.getState().open).toBe(true);
    tapOn(screen.getByRole("button", { name: "Close the visualizer" }));
    expect(np).not.toHaveClass("is-art");
    expect(usePhoneVis.getState().open).toBe(false);
  });

  it("opens and closes from the keyboard too", () => {
    open();
    fireEvent.keyDown(screen.getByRole("button", { name: "Open the visualizer" }), { key: "Enter" });
    expect(usePhoneVis.getState().open).toBe(true);
    fireEvent.keyDown(screen.getByRole("button", { name: "Close the visualizer" }), { key: " " });
    expect(usePhoneVis.getState().open).toBe(false);
  });

  it("changes the style with a sideways swipe over the art, and doesn't skip the song", async () => {
    const next = vi.spyOn(player, "next").mockImplementation(() => Promise.resolve());
    const previous = vi.spyOn(player, "previous").mockImplementation(() => Promise.resolve());
    setPhoneVis(true);
    const np = open();
    const art = screen.getByTestId("np-art");
    Object.defineProperty(art, "clientWidth", { value: 400, configurable: true });
    Object.defineProperty(np, "clientHeight", { value: 800, configurable: true });
    await act(async () => swipe(art, -160));
    await waitFor(() => expect(useSettings.getState().settings?.visualizerStyle).toBe("blur"));
    expect(np).toHaveClass("is-art--blur");
    expect(within(np).getByTestId("vis-name")).toHaveTextContent("Blur");
    // The cover didn't slide away.
    expect(screen.getByTestId("np-deck").style.transform).toBe("");
    await act(async () => swipe(art, 160));
    await act(async () => swipe(art, 160));
    await waitFor(() => expect(useSettings.getState().settings?.visualizerStyle).toBe("bars"));
    expect(next).not.toHaveBeenCalled();
    expect(previous).not.toHaveBeenCalled();
    // Remembered (a synced setting).
    await waitFor(async () => expect((await mock.settingsGet()).visualizerStyle).toBe("bars"));
  });

  it("still skips on a sideways swipe over the cover when the visualizer is closed", async () => {
    const next = vi.spyOn(player, "next").mockImplementation(() => Promise.resolve());
    const np = open();
    const art = screen.getByTestId("np-art");
    Object.defineProperty(art, "clientWidth", { value: 400, configurable: true });
    Object.defineProperty(np, "clientHeight", { value: 800, configurable: true });
    await act(async () => swipe(art, -160));
    expect(next).toHaveBeenCalledTimes(1);
    expect(usePhoneVis.getState().open).toBe(false);
  });

  it("doesn't skip on a quick drag that stopped before the finger lifted", async () => {
    const next = vi.spyOn(player, "next").mockImplementation(() => Promise.resolve());
    const np = open();
    const art = screen.getByTestId("np-art");
    Object.defineProperty(art, "clientWidth", { value: 400, configurable: true });
    Object.defineProperty(np, "clientHeight", { value: 800, configurable: true });
    let now = 1000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    fireEvent.pointerDown(art, { pointerId: 1, button: 0, clientX: 300, clientY: 400 });
    // 40 px left in 20 ms: a flick's speed, short of the distance...
    for (let i = 1; i <= 2; i++) {
      now += 10;
      fireEvent.pointerMove(art, { pointerId: 1, clientX: 300 - 20 * i, clientY: 400 });
    }
    // ...then held still a second before lifting.
    now += 1000;
    await act(async () => void fireEvent.pointerUp(art, { pointerId: 1, clientX: 260, clientY: 400 }));
    expect(next).not.toHaveBeenCalled();
    // The same drag lifted at once is a flick.
    now += 500;
    fireEvent.pointerDown(art, { pointerId: 1, button: 0, clientX: 300, clientY: 400 });
    for (let i = 1; i <= 2; i++) {
      now += 10;
      fireEvent.pointerMove(art, { pointerId: 1, clientX: 300 - 20 * i, clientY: 400 });
    }
    now += 10;
    await act(async () => void fireEvent.pointerUp(art, { pointerId: 1, clientX: 260, clientY: 400 }));
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("keeps the cover carrying on through other updates, and lets go once the next song plays", async () => {
    const proto = HTMLElement.prototype as unknown as Record<string, unknown>;
    const had = proto.animate;
    // The carry-on never finishes by itself here, so the row stays on its way.
    proto.animate = () => ({ finished: new Promise(() => {}), cancel: () => {} });
    try {
      vi.spyOn(player, "next").mockImplementation(() => Promise.resolve());
      const [a, b] = mock.fx.tracks;
      const was = usePlayer.getState().state!;
      const nowItem = { itemId: "q1", track: a, source: "context" as const };
      const nextItem = { itemId: "q2", track: b, source: "context" as const };
      act(() => usePlayer.setState({ state: { ...was, track: a, queue: { ...was.queue, current: nowItem, nextUp: [nextItem] } } }));
      const np = open();
      const art = screen.getByTestId("np-art");
      Object.defineProperty(art, "clientWidth", { value: 400, configurable: true });
      Object.defineProperty(np, "clientHeight", { value: 800, configurable: true });
      const row = screen.getByTestId("np-deck");
      await act(async () => swipe(art, -160));
      const on = row.style.transform;
      expect(on).toMatch(/translateX\(-/);
      const s = usePlayer.getState().state!;
      // Buffering, the volume: the row carries on.
      act(() => usePlayer.setState({ state: { ...s, volume: s.volume / 2, status: "buffering" } }));
      expect(row.style.transform).toBe(on);
      // The next song plays: the row lets go.
      act(() => usePlayer.setState({ state: { ...s, track: b, queue: { ...s.queue, current: nextItem, nextUp: [] } } }));
      await waitFor(() => expect(row.style.transform).toBe(""));
    } finally {
      if (had === undefined) delete proto.animate;
      else proto.animate = had;
    }
  });

  it("hides the controls with a tap on the art beside the cover and brings them back", () => {
    setPhoneVis(true);
    const np = open();
    fireEvent.click(screen.getByTestId("np-player"));
    expect(np).toHaveClass("is-bare");
    expect(np.querySelector(".ph-np__deck")).toHaveAttribute("inert");
    expect(np.querySelector(".ph-np__top")).toHaveAttribute("inert");
    const back = screen.getByRole("button", { name: "Show controls" });
    expect(back).toHaveFocus();
    fireEvent.click(back);
    expect(np).not.toHaveClass("is-bare");
    // Escape brings them back too, without closing now playing.
    fireEvent.click(screen.getByTestId("np-player"));
    act(() => void fireEvent.keyDown(window, { key: "Escape" }));
    expect(np).not.toHaveClass("is-bare");
    expect(usePhoneUi.getState().nowPlaying).toBe(true);
  });

  it("leaves the cover alone with the setting off", async () => {
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerCoverTap: false } }));
    setPhoneVis(true);
    const np = open();
    expect(np).not.toHaveClass("is-art");
    expect(screen.queryByRole("button", { name: "Open the visualizer" })).toBeNull();
    tapOn(screen.getByTestId("np-art"));
    expect(np).not.toHaveClass("is-art");
  });
});

// ------------------------------------------------------------ the expanded view

describe("the expanded view", () => {
  it("opens the visualizer with a click on the cover, and closes it with another", async () => {
    // The plain cover (the record player has its own handling).
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, recordMotion: "off" } }));
    render(<FullWindow />);
    act(() => fullWindow.open());
    const dialog = screen.getByRole("dialog", { name: "Expanded now playing" });
    fireEvent.click(dialog.querySelector(".fw__cover")!);
    expect(useFullWindow.getState().mode).toBe("visualizer");
    expect(dialog).toHaveClass("fw--visualizer", "fw--art", "fw--art-grooves");
    expect(within(dialog).getByTestId("vis-stage")).toBeInTheDocument();
    expect(within(dialog).getByTestId("vis-name")).toHaveTextContent("Grooves");
    fireEvent.click(dialog.querySelector(".fw__cover")!);
    expect(useFullWindow.getState().mode).toBe("artwork");
  });

  it("keeps a small Visualizer button, V, the arrow keys for the style and Escape to close", async () => {
    const seek = vi.spyOn(player, "seekBy").mockImplementation(() => Promise.resolve());
    render(<FullWindow />);
    act(() => fullWindow.open());
    fireEvent.click(within(screen.getByRole("group", { name: "Show" })).getByRole("button", { name: "Visualizer" }));
    expect(useFullWindow.getState().mode).toBe("visualizer");
    act(() => void fireEvent.keyDown(window, { key: "ArrowRight" }));
    await waitFor(() => expect(useSettings.getState().settings?.visualizerStyle).toBe("blur"));
    act(() => void fireEvent.keyDown(window, { key: "ArrowLeft" }));
    act(() => void fireEvent.keyDown(window, { key: "ArrowLeft" }));
    await waitFor(() => expect(useSettings.getState().settings?.visualizerStyle).toBe("bars"));
    expect(seek).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Expanded now playing" })).toHaveClass("fw--art-bars");
    // Escape closes the visualizer first, then the view.
    act(() => void fireEvent.keyDown(window, { key: "Escape" }));
    expect(useFullWindow.getState().mode).toBe("artwork");
    expect(useFullWindow.getState().open).toBe(true);
    // Off the visualizer the arrows seek again.
    act(() => void fireEvent.keyDown(window, { key: "ArrowRight" }));
    expect(seek).toHaveBeenCalledTimes(1);
    act(() => void fireEvent.keyDown(window, { key: "v" }));
    expect(useFullWindow.getState().mode).toBe("visualizer");
    act(() => void fireEvent.keyDown(window, { key: "V" }));
    expect(useFullWindow.getState().mode).toBe("artwork");
  });

  it("hides the controls with a click on the art, and a key brings them back (Escape only that)", () => {
    render(<FullWindow />);
    act(() => fullWindow.open("visualizer"));
    const dialog = screen.getByRole("dialog", { name: "Expanded now playing" });
    const top = dialog.querySelector(".fw__top")!;
    fireEvent.click(dialog.querySelector(".fw__stage")!);
    expect(dialog).toHaveClass("is-bare");
    expect(top).toHaveAttribute("inert");
    expect(document.documentElement).toHaveClass("fw-bare");
    act(() => void fireEvent.keyDown(window, { key: "a" }));
    expect(dialog).not.toHaveClass("is-bare");
    fireEvent.click(dialog.querySelector(".fw__stage")!);
    act(() => void fireEvent.keyDown(window, { key: "Escape" }));
    expect(dialog).not.toHaveClass("is-bare");
    expect(useFullWindow.getState().mode).toBe("visualizer");
  });

  it("leaves V and L to a field being typed in", () => {
    render(
      <>
        <input aria-label="Search" />
        <FullWindow />
      </>,
    );
    act(() => fullWindow.open());
    const field = screen.getByRole("textbox", { name: "Search" });
    act(() => void fireEvent.keyDown(field, { key: "v" }));
    act(() => void fireEvent.keyDown(field, { key: "l" }));
    expect(useFullWindow.getState().mode).toBe("artwork");
  });

  it("leaves the cover alone with the setting off", async () => {
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerCoverTap: false, recordMotion: "off" } }));
    render(<FullWindow />);
    act(() => fullWindow.open());
    fireEvent.click(screen.getByRole("dialog", { name: "Expanded now playing" }).querySelector(".fw__cover")!);
    expect(useFullWindow.getState().mode).toBe("artwork");
  });
});

// ------------------------------------------------------------ settings

describe("Settings", () => {
  it("has one visualizer row under Playback, on, and saves it", async () => {
    await mock.settingsSet({ ...DEFAULT_SETTINGS });
    await loadSettings();
    render(<SettingsPage />);
    const row = await screen.findByRole("switch", { name: "Open the visualizer when you click the cover" });
    expect(row).toBeChecked();
    const el = document.querySelector<HTMLElement>('[data-setting="visualizerCoverTap"]')!;
    expect(el.closest("section")?.querySelector("h2")?.textContent).toMatch(/Playback/);
    for (const gone of ["visualizerButton", "visualizerStyle", "visualizerIntensity", "visualizerFps", "visualizerSmoothing", "visualizerPanelStrip", "visualizerBatteryPause"])
      expect(document.querySelector(`[data-setting="${gone}"]`), gone).toBeNull();
    fireEvent.click(row);
    await waitFor(async () => expect((await mock.settingsGet()).visualizerCoverTap).toBe(false));
  });
});
