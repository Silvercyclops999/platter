/**
 * The visualizer: its scales and needles, the feed that asks the core to
 * watch only while a view is on screen, the art styles (their signals, beat
 * pulse, renderer and fallback) and the instruments drawing from mock
 * frames, reduced motion, the phone's full screen art, the expanded view's
 * button and V key, and its Settings rows.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setReducedMotion } from "../design/motion";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockFrame } from "../lib/mock/visualizer";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { FullWindow } from "../shell/FullWindow";
import { FULL_WINDOW_PREFS_DEFAULT, fullWindow, useFullWindow } from "../stores/fullWindow";
import { usePlayer } from "../stores/player";
import { loadSettings, useSettings } from "../stores/settings";
import { latestFrame, resetFeed, useFeed, watchFeed } from "../visualizer/feed";
import {
  AnalyserState,
  BANDS,
  FLOOR_DB,
  GAP_MS,
  Needle,
  VU_REF_DBFS,
  bandCentre,
  bandEdge,
  current,
  dbY,
  freqX,
  lowEnd,
  scopePoint,
  vuPosition,
} from "../visualizer/scale";
import { CALM_FPS, CALM_NOTE, StylePicker, VisualizerView, shownStyle, visFps } from "../visualizer/VisualizerView";
import { correlation } from "../visualizer/Scope";
import { ArtStage, MAX_RENDER_PIXELS, PHONE_RENDER_CAP, renderScale } from "../visualizer/ArtStage";
import { ArtGL, type ArtFrame } from "../visualizer/art/ArtGL";
import { StrictMode } from "react";
import { LOW_BANDS, MusicSignals } from "../visualizer/signals";
import { normalizeStyle } from "../visualizer/styles";
import { paletteFromPixels } from "../visualizer/palette";
import { rgbToHsl } from "../lib/color";
import { player } from "../stores/player";
import { PanelStrip } from "../visualizer/places";
import { DeckFaceplate } from "../shell/DeckFrame";
import { NowPlaying as PhoneNowPlaying } from "../phone/NowPlaying";
import { usePhoneUi } from "../phone/phoneUi";
import { setPhoneVis, usePhoneVis } from "../visualizer/phoneVis";

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

// ------------------------------------------------------------ scales

describe("scales", () => {
  it("runs 64 log bands from 20 Hz to 20 kHz, the same as the core", () => {
    expect(bandEdge(0)).toBeCloseTo(20);
    expect(bandEdge(BANDS)).toBeCloseTo(20_000, 0);
    expect(bandCentre(31)).toBeGreaterThan(bandEdge(31));
    expect(bandCentre(31)).toBeLessThan(bandEdge(32));
    // The middle of a 20 Hz to 20 kHz log axis is about 632 Hz.
    expect(freqX(632.5)).toBeCloseTo(0.5, 2);
    expect(dbY(0)).toBe(0);
    expect(dbY(-200)).toBe(1);
  });

  it("puts 0 VU where a real meter has it, linear in voltage with +3 at the end", () => {
    expect(vuPosition(3)).toBeCloseTo(1);
    expect(vuPosition(0)).toBeCloseTo(0.708, 3);
    expect(vuPosition(-20)).toBeCloseTo(0.0708, 3);
  });

  it("moves a needle like a VU: 99% of a step in 300 ms, falling as slowly as it rises", () => {
    const n = new Needle();
    const steady = VU_REF_DBFS; // 0 VU
    for (let t = 0; t < 300; t += 10) n.step(steady, 10);
    expect(n.pos).toBeGreaterThan(0.985);
    expect(n.pos).toBeLessThan(1.02);
    // Only part of the way after 100 ms.
    const m = new Needle();
    for (let t = 0; t < 100; t += 10) m.step(steady, 10);
    expect(m.pos).toBeLessThan(0.9);
    // Silence: back down over about the same time.
    for (let t = 0; t < 300; t += 10) n.step(FLOOR_DB, 10);
    expect(n.pos).toBeLessThan(0.02);
  });

  it("lets the analyser rise fast, fall by the smoothing setting, and hold peaks", () => {
    const loud = new Array(BANDS).fill(-10);
    const fall = (smoothing: "low" | "medium" | "high") => {
      const a = new AnalyserState();
      for (let i = 0; i < 10; i++) a.step(loud, 16, smoothing, true);
      expect(a.shown[10]).toBeGreaterThan(-11);
      for (let i = 0; i < 10; i++) a.step(null, 16, smoothing, true);
      return a;
    };
    const low = fall("low");
    const high = fall("high");
    expect(low.shown[10]).toBeLessThan(high.shown[10]);
    // The peak line stays up while the line falls.
    expect(high.peaks[10]).toBeCloseTo(-10, 0);
    // No peak line without peak hold (reduced motion).
    const calm = new AnalyserState();
    calm.step(loud, 16, "medium", false);
    expect(calm.peaks[10]).toBe(FLOOR_DB);
  });

  it("falls to silence when frames stop instead of freezing", () => {
    const f = mockFrame(0, false);
    expect(current(f, 1000, 1000 + GAP_MS)).toBe(f);
    expect(current(f, 1000, 1001 + GAP_MS)).toBeNull();
  });

  it("draws a mono pair upright on the scope and reads the low end", () => {
    const [x, y] = scopePoint(0.5, 0.5);
    expect(x).toBe(0);
    expect(y).toBeGreaterThan(0);
    const [sx] = scopePoint(0.5, -0.5);
    expect(Math.abs(sx)).toBeGreaterThan(0.5);
    expect(lowEnd(null)).toBe(0);
    expect(lowEnd(new Array(BANDS).fill(FLOOR_DB))).toBeLessThan(0.001);
    expect(lowEnd(new Array(BANDS).fill(-6))).toBeGreaterThan(0.5);
  });

  it("keeps a still Sleeve, Analyser and Meters, at 15 frames, under reduced motion; a phone runs at most 30", () => {
    for (const s of ["scope", "ink", "grooves", "light"]) expect(shownStyle(s, true)).toBe("sleeve");
    expect(shownStyle("meters", true)).toBe("meters");
    expect(shownStyle("analyser", true)).toBe("analyser");
    expect(shownStyle("sleeve", true)).toBe("sleeve");
    expect(shownStyle("ink", false)).toBe("ink");
    expect(visFps(60, false, true)).toBe(CALM_FPS);
    expect(visFps(60, true, false)).toBe(30);
    expect(visFps(15, true, false)).toBe(15);
    expect(visFps(60, false, false)).toBe(60);
  });
});

describe("styles and defaults", () => {
  it("makes a saved Glow into Sleeve, and Sleeve (Calm) the default", () => {
    expect(normalizeStyle("glow")).toBe("sleeve");
    expect(shownStyle("glow", false)).toBe("sleeve");
    expect(normalizeStyle(undefined)).toBe("sleeve");
    expect(normalizeStyle("something new")).toBe("sleeve");
    expect(normalizeStyle("grooves")).toBe("grooves");
    expect(normalizeStyle("scope")).toBe("scope");
    expect(DEFAULT_SETTINGS.visualizerStyle).toBe("sleeve");
    expect(DEFAULT_SETTINGS.visualizerIntensity).toBe("calm");
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

// ------------------------------------------------------------ the styles

describe("each style", () => {
  it("Analyser draws its line over a log frequency axis and a dB scale", async () => {
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerStyle: "analyser" } }));
    const run = frames();
    render(<VisualizerView />);
    const view = screen.getByTestId("visualizer");
    // The scale: 50, 200, 1k, 5k, 15k and the dB lines.
    for (const label of ["50", "200", "1k", "5k", "15k", "dBFS", "-12", "-60"]) expect(within(view).getByText(label)).toBeInTheDocument();
    frame();
    run(3);
    expect(view.querySelector(".vis-analyser")?.getAttribute("data-renderer")).toBe("2d");
    expect(drawn.calls.filter((c) => c === "lineTo").length).toBeGreaterThanOrEqual(BANDS - 1);
    expect(drawn.calls).toContain("stroke");
    // The core watches at the frame rate setting, no scope.
    await waitFor(() => expect(lastCall()).toEqual({ on: true, fps: 60, scope: false }));
  });

  it("Analyser uses WebGL2 when it's there, and Canvas 2D after the GPU context is lost", async () => {
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerStyle: "analyser" } }));
    const run = frames();
    const noop = () => {};
    gl = new Proxy(
      {
        COMPILE_STATUS: 1,
        LINK_STATUS: 2,
        isContextLost: () => false,
        getShaderParameter: () => true,
        getProgramParameter: () => true,
        createShader: () => ({}),
        createProgram: () => ({}),
        createBuffer: () => ({}),
        createVertexArray: () => ({}),
        getUniformLocation: () => ({}),
        getAttribLocation: () => 0,
        drawArrays: vi.fn(),
      } as Record<string, unknown>,
      { get: (t, k: string) => (k in t ? t[k] : noop) },
    );
    render(<VisualizerView />);
    const box = screen.getByTestId("visualizer").querySelector(".vis-analyser")!;
    await waitFor(() => expect(box.getAttribute("data-renderer")).toBe("webgl2"));
    frame();
    run(2);
    expect((gl!.drawArrays as ReturnType<typeof vi.fn>).mock.calls.length).toBeGreaterThan(0);
    act(() => void box.querySelector("canvas")!.dispatchEvent(new Event("webglcontextlost", { cancelable: true })));
    await waitFor(() => expect(box.getAttribute("data-renderer")).toBe("2d"));
  });

  it("Meters draw two needles with peak lamps on a VU scale", async () => {
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerStyle: "meters" } }));
    const run = frames();
    render(<VisualizerView />);
    const view = screen.getByTestId("visualizer");
    expect(within(view).getByRole("img", { name: "Level meters, left and right" })).toBeInTheDocument();
    expect(within(view).getAllByText("VU")).toHaveLength(2);
    expect(within(view).getAllByText("+3")).toHaveLength(2);
    expect(within(view).getByText(/0 VU = −10 dBFS/)).toBeInTheDocument();
    frame();
    run(3);
    // Two needles and two lamps a frame.
    expect(drawn.calls.filter((c) => c === "arc").length).toBeGreaterThanOrEqual(2);
    expect(drawn.calls.filter((c) => c === "lineTo").length).toBeGreaterThanOrEqual(2);
  });

  it("picking a style keeps it (synced setting)", async () => {
    render(<VisualizerView />);
    fireEvent.click(screen.getByRole("button", { name: "Meters" }));
    await waitFor(() => expect(useSettings.getState().settings?.visualizerStyle).toBe("meters"));
    expect((await mock.settingsGet()).visualizerStyle).toBe("meters");
  });

  it("under reduced motion offers only Sleeve, Analyser and Meters, and says why", async () => {
    setReducedMotion(true);
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerStyle: "scope" } }));
    render(<StylePicker />);
    for (const name of ["Ink", "Grooves", "Light", "Scope"]) {
      expect(screen.getByRole("button", { name })).toHaveAttribute("aria-disabled", "true");
      // The reason is on screen and read with the buttons it applies to.
      expect(screen.getByRole("button", { name })).toHaveAccessibleDescription(CALM_NOTE);
    }
    for (const name of ["Sleeve", "Analyser", "Meters"]) expect(screen.getByRole("button", { name })).not.toHaveAttribute("aria-disabled");
    // A saved Scope shows as a still Sleeve.
    expect(screen.getByRole("button", { name: "Sleeve" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText(CALM_NOTE)).toBeVisible();
    // Pressing one that's off changes nothing.
    fireEvent.click(screen.getByRole("button", { name: "Ink" }));
    expect(useSettings.getState().settings?.visualizerStyle).toBe("scope");
  });

  it("draws Analyser without the peak line at 15 frames under reduced motion", async () => {
    setReducedMotion(true);
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerStyle: "analyser" } }));
    render(<VisualizerView />);
    await waitFor(() => expect(lastCall()).toEqual({ on: true, fps: 15, scope: false }));
  });

  it("picks styles in two groups, Art then Instruments, with plain pressed buttons, each a tab stop", () => {
    render(<StylePicker />);
    const group = screen.getByRole("group", { name: "Visualizer style" });
    const art = within(group).getByRole("group", { name: "Art" });
    const instruments = within(group).getByRole("group", { name: "Instruments" });
    expect(within(art).getAllByRole("button").map((b) => b.textContent)).toEqual(["Sleeve", "Ink", "Grooves", "Light"]);
    expect(within(instruments).getAllByRole("button").map((b) => b.textContent)).toEqual(["Analyser", "Meters", "Scope"]);
    // Sleeve is the default.
    expect(within(group).getAllByRole("button").map((b) => b.getAttribute("aria-pressed"))).toEqual(["true", "false", "false", "false", "false", "false", "false"]);
    expect(screen.queryByRole("radio")).toBeNull();
  });

  it("says so and asks for nothing when playback is on another device or nothing plays", async () => {
    act(() =>
      usePlayer.setState({
        remote: { playerId: "p", playerName: "Living room", kind: "plex", status: "playing", track: track(), positionMs: 0, durationMs: 1, volume: null } as never,
      }),
    );
    const { unmount } = render(<VisualizerView />);
    expect(screen.getByRole("status")).toHaveTextContent("The visualizer shows what plays on this device.");
    unmount();
    act(() => usePlayer.setState({ remote: null, state: { ...usePlayer.getState().state!, track: null, status: "idle" } }));
    render(<VisualizerView />);
    expect(screen.getByRole("status")).toHaveTextContent("Nothing is playing on this device.");
    expect(mock.visualizer.calls).toEqual([]);
  });
});

describe("Scope", () => {
  it("Scope draws the stereo trace on its L, R, M and S frame and reads the correlation", async () => {
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerStyle: "scope" } }));
    const run = frames();
    render(<VisualizerView />);
    const scope = await screen.findByRole("group", { name: "Stereo phase scope" });
    for (const l of ["L", "R", "M", "S"]) expect(await within(scope).findByText(l)).toBeInTheDocument();
    // The scope needs the stereo pairs.
    await waitFor(() => expect(lastCall()).toEqual({ on: true, fps: 60, scope: true }));
    frame(1_000, true);
    run(2);
    expect(drawn.calls.filter((c) => c === "lineTo").length).toBeGreaterThan(200);
    expect(within(scope).getByText(/Correlation/)).toHaveTextContent(/Correlation [+−]0\.\d\d/);
  });

  it("works out the correlation: +1 mono, -1 out of phase, nothing for silence", () => {
    expect(correlation([0.5, 0.5, -0.2, -0.2])).toBeCloseTo(1);
    expect(correlation([0.5, -0.5, -0.2, 0.2])).toBeCloseTo(-1);
    expect(correlation([0, 0, 0, 0])).toBeNull();
  });

});

// ------------------------------------------------------------ the art styles

/** Low bands at `low` dB, everything else at `rest`, as a fresh frame. */
function bandsWith(low: number, rest = -50): number[] {
  const out = new Array(BANDS).fill(rest);
  for (let k = LOW_BANDS[0]; k < LOW_BANDS[1]; k++) out[k] = low;
  return out;
}

describe("the art signals", () => {
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
    // Sixteen kicks; the very first frame has nothing to compare with.
    expect(sig.beatCount).toBeGreaterThanOrEqual(15);
    expect(sig.beatCount).toBeLessThanOrEqual(16);
    expect(pulseAfterKick).toBeGreaterThan(0.3);
    expect(pulseLater).toBeLessThan(pulseAfterKick / 2);

    const steady = new MusicSignals();
    for (let t = 0; t < 4_000; t += 33) steady.step(bandsWith(-20), {}, 33, t);
    expect(steady.beatCount).toBe(0);
    expect(steady.pulse).toBe(0);
  });

  it("hears the kicks in the mock's song, and a lively intensity pulses harder than a calm one", () => {
    const calm = new MusicSignals();
    const lively = new MusicSignals();
    let calmMax = 0;
    let livelyMax = 0;
    for (let t = 0; t < 8_000; t += 33) {
      const f = mockFrame(t, false);
      calm.step(f.bands, f, 33, t, "calm");
      lively.step(f.bands, f, 33, t, "lively");
      calmMax = Math.max(calmMax, calm.pulse);
      livelyMax = Math.max(livelyMax, lively.pulse);
    }
    // 16 kicks and a few pickups in 8 seconds.
    expect(calm.beatCount).toBeGreaterThanOrEqual(14);
    expect(calm.beatCount).toBeLessThanOrEqual(24);
    expect(livelyMax).toBeGreaterThan(calmMax);
    expect(calm.low).toBeGreaterThan(0);
    expect(calm.high).toBeGreaterThan(0);
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
    expect(sig.mid).toBeLessThan(0.02);
    expect(sig.high).toBeLessThan(0.02);
    expect(sig.pulse).toBeLessThan(0.01);
    // The art still drifts, slowly, in a quiet moment.
    expect(sig.flow).toBeGreaterThan(flowBefore);
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
    // The lead is the cover's red, then its blue.
    expect(Math.min(hues[0], 360 - hues[0])).toBeLessThan(25);
    expect(hues[1]).toBeGreaterThan(200);
    expect(hues[1]).toBeLessThan(250);
    expect(p.base[0] + p.base[1] + p.base[2]).toBeLessThan(0.35);
    const grey = paletteFromPixels(new Array(256).fill(0).flatMap((_, i) => [i % 200, i % 200, i % 200, 255]));
    for (const c of grey.colours) expect(rgbToHsl([c[0] * 255, c[1] * 255, c[2] * 255])[1]).toBeLessThanOrEqual(0.17);
  });

  it("caps the render scale on a phone, and the pixels on any screen", () => {
    expect(renderScale("grooves", 3, true, 430, 932)).toBeLessThanOrEqual(3 * PHONE_RENDER_CAP);
    expect(renderScale("sleeve", 3, true, 430, 932)).toBeLessThanOrEqual(1.5);
    expect(renderScale("grooves", 2, false, 800, 600)).toBe(2);
    expect(renderScale("sleeve", 2, false, 800, 600)).toBe(1);
    const big = renderScale("grooves", 2, false, 3840, 2160);
    expect(3840 * 2160 * big * big).toBeLessThanOrEqual(MAX_RENDER_PIXELS + 1);
  });
});

describe("the art stage", () => {
  const coverEl = () => document.querySelector<HTMLElement>(".test-cover");
  function stage(props: Partial<Parameters<typeof ArtStage>[0]> = {}) {
    return render(
      <>
        <div className="test-cover" />
        <ArtStage style="sleeve" fps={30} reduced={false} intensity="calm" cover={coverEl} {...props} />
      </>,
    );
  }
  const kick = (ms: number) => act(() => mock.emit("player://spectrum", { t: ms, bands: bandsWith(-44 + 34 * Math.exp(-(ms % 500) / 70)), rms: [-20, -20], peak: [-6, -6] }));

  it("draws each art style with WebGL2 on one quad, and Sleeve in layers after the GPU context is lost", async () => {
    const run = frames();
    const drawArrays = vi.fn();
    gl = fakeGl(drawArrays);
    stage({ style: "grooves" });
    const s = screen.getByTestId("art-stage");
    await waitFor(() => expect(s).toHaveAttribute("data-renderer", "webgl2"));
    expect(s).toHaveAttribute("data-style", "grooves");
    frame();
    run(2);
    expect(drawArrays).toHaveBeenCalledWith(5, 0, 4);
    act(() => void s.querySelector("canvas")!.dispatchEvent(new Event("webglcontextlost", { cancelable: true })));
    await waitFor(() => expect(s).toHaveAttribute("data-renderer", "css"));
    expect(s).toHaveAttribute("data-style", "sleeve");
    expect(s.querySelectorAll(".art-css__field")).toHaveLength(4);
  });

  it("falls back to Sleeve drawn with layers when there is no WebGL2, and still moves with the music", async () => {
    const run = frames();
    stage({ style: "ink" });
    const s = screen.getByTestId("art-stage");
    await waitFor(() => expect(s).toHaveAttribute("data-renderer", "css"));
    expect(s).toHaveAttribute("data-style", "sleeve");
    run(1);
    const field = s.querySelector<HTMLElement>(".art-css__field--0")!;
    const before = field.style.transform;
    for (let i = 0; i < 30; i++) {
      kick(i * 20);
      run(1);
    }
    expect(field.style.transform).not.toBe(before);
    // The core is asked for frames at the stage's rate.
    await waitFor(() => expect(lastCall()).toEqual({ on: true, fps: 30, scope: false }));
  });

  it("breathes the sharp cover with the beat", async () => {
    const run = frames();
    stage();
    run(1);
    const cover = coverEl()!;
    let most = 1;
    for (let i = 0; i < 60; i++) {
      kick(10_000 + i * 20);
      run(1);
      most = Math.max(most, Number(cover.style.getPropertyValue("--art-breath")));
    }
    expect(most).toBeGreaterThan(1.005);
    // Only a few percent.
    expect(most).toBeLessThan(1.06);
  });

  it("stays still under reduced motion: no pulse, no drift, nothing asked of the core", async () => {
    const run = frames();
    stage({ reduced: true, fps: CALM_FPS });
    const s = screen.getByTestId("art-stage");
    expect(s).toHaveAttribute("data-reduced", "true");
    await waitFor(() => expect(s).toHaveAttribute("data-renderer", "css"));
    run(1);
    const field = s.querySelector<HTMLElement>(".art-css__field--1")!;
    const before = field.style.transform;
    for (let i = 0; i < 40; i++) {
      kick(i * 20);
      run(1);
    }
    expect(coverEl()!.style.getPropertyValue("--art-breath")).toBe("1.0000");
    expect(field.style.transform).toBe(before);
    expect(mock.visualizer.calls).toEqual([]);
  });

  it("gives the cover back as it was when it goes", async () => {
    const run = frames();
    const { unmount } = stage({ style: "light" });
    run(3);
    expect(coverEl()!.style.getPropertyValue("--art-breath")).not.toBe("");
    const cover = coverEl()!;
    unmount();
    expect(cover.style.getPropertyValue("--art-breath")).toBe("");
    expect(cover.style.getPropertyValue("--art-leak-a")).toBe("");
  });
});


describe("the art renderer", () => {
  const loseContext = vi.fn();
  /** A context that names its uniforms and records what they are set to. */
  function recordingGl(over: Record<string, unknown> = {}) {
    const set: { name: string; v: number }[] = [];
    const textures: unknown[] = [];
    const ctx = fakeGl(vi.fn(), {
      getUniformLocation: (_p: unknown, name: string) => ({ name }),
      uniform1f: (loc: { name: string }, v: number) => void set.push({ name: loc.name, v }),
      createTexture: () => ({ id: textures.length }),
      getExtension: (n: string) => (n === "WEBGL_lose_context" ? { loseContext } : null),
      ...over,
    });
    return { ctx, set };
  }
  const canvas = () => document.createElement("canvas");
  const artFrame = (): ArtFrame => ({
    style: "sleeve",
    time: 0,
    spin: 0,
    low: 0,
    mid: 0,
    high: 0,
    pulse: 0,
    flow: 0,
    drift: 0,
    anchor: [0, 0, 10, 0],
    motion: 1,
    beatAt: new Float32Array(4),
    beatStrength: new Float32Array(4),
  });
  const mixAt = (r: ArtGL, set: { name: string; v: number }[], now: number) => {
    set.length = 0;
    r.draw(artFrame(), now, 10, 10);
    return set.find((u) => u.name === "uMix")!.v;
  };
  beforeEach(() => loseContext.mockClear());

  it("gives the GPU context back when it goes, and when the art can't build", () => {
    gl = recordingGl().ctx;
    const r = ArtGL.create(canvas())!;
    expect(r).not.toBeNull();
    r.dispose();
    expect(loseContext).toHaveBeenCalledTimes(1);
    gl = recordingGl({ getProgramParameter: () => false }).ctx;
    expect(ArtGL.create(canvas())).toBeNull();
    expect(loseContext).toHaveBeenCalledTimes(2);
  });

  it("keeps what shows when a cover can't be a texture, and a second cover mid-fade carries the same fade on", () => {
    const { ctx, set } = recordingGl();
    gl = ctx;
    const r = ArtGL.create(canvas())!;
    const img = {} as TexImageSource;
    r.setCover(img, 0, 1000);
    expect(mixAt(r, set, 500)).toBeCloseTo(0.5);
    // Another cover while the first is still mostly fading in: the fade goes on from where it was.
    r.setCover(img, 300, 1000);
    expect(mixAt(r, set, 500)).toBeCloseTo(0.5);
    // Once the cover mostly shows, a new one starts its own fade.
    r.setCover(img, 800, 1000);
    expect(mixAt(r, set, 800)).toBe(0);
    expect(mixAt(r, set, 2000)).toBe(1);
    // A cover that won't upload changes nothing.
    (ctx as unknown as Record<string, unknown>).texImage2D = () => {
      throw new Error("tainted");
    };
    expect(() => r.setCover(img, 3000, 1000)).toThrow();
    expect(mixAt(r, set, 3000)).toBe(1);
  });

  it("fades to a plain texture with no cover", () => {
    const uploads: unknown[][] = [];
    const { ctx, set } = recordingGl({ texImage2D: (...a: unknown[]) => void uploads.push(a) });
    gl = ctx;
    const r = ArtGL.create(canvas())!;
    uploads.length = 0;
    r.setCover(null, 0, 1000);
    // One dark pixel, not the last cover.
    expect(uploads).toHaveLength(1);
    expect(uploads[0][3]).toBe(1);
    expect(mixAt(r, set, 500)).toBeCloseTo(0.5);
  });

  it("draws with WebGL2 under StrictMode's second mount, on a canvas of its own", async () => {
    gl = recordingGl().ctx;
    render(
      <StrictMode>
        <ArtStage style="sleeve" fps={30} reduced={false} intensity="calm" cover={() => null} />
      </StrictMode>,
    );
    const s = screen.getByTestId("art-stage");
    await waitFor(() => expect(s).toHaveAttribute("data-renderer", "webgl2"));
    expect(s.querySelectorAll("canvas")).toHaveLength(1);
  });

  it("never lets a broken frame poison the signals", () => {
    const sig = new MusicSignals();
    const broken = [NaN, undefined, Infinity, -Infinity] as unknown as number[];
    for (let t = 0; t < 1_000; t += 33) sig.step(broken, {}, 33, t);
    for (let t = 1_000; t < 2_000; t += 33) sig.step([-20, -30], {}, 33, t);
    for (const v of [sig.low, sig.mid, sig.high, sig.pulse, sig.flow, sig.drift]) expect(Number.isFinite(v)).toBe(true);
    // Real frames after that still find their beats.
    for (let t = 2_000; t < 6_000; t += 33) sig.step(bandsWith(-44 + 34 * Math.exp(-(t % 500) / 70)), {}, 33, t);
    expect(sig.beatCount).toBeGreaterThanOrEqual(6);
  });
});

describe("the panel strip and the Deck's meters", () => {
  it("puts a slim Analyser under the panel's cover only with its setting on, and not under the expanded view", async () => {
    const { rerender } = render(<PanelStrip />);
    expect(screen.queryByTestId("panel-visualizer")).toBeNull();
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerPanelStrip: true } }));
    rerender(<PanelStrip />);
    const strip = screen.getByTestId("panel-visualizer");
    expect(within(strip).getByRole("img", { name: "Spectrum analyser" })).toHaveClass("vis-analyser--slim");
    // No labels in the strip.
    expect(within(strip).queryByText("1k")).toBeNull();
    await waitFor(() => expect(lastCall()?.on).toBe(true));
    act(() => fullWindow.open("artwork"));
    expect(screen.queryByTestId("panel-visualizer")).toBeNull();
  });

  it("shows needle meters on the Deck faceplate only while the visualizer is on", async () => {
    render(<DeckFaceplate />);
    const face = screen.getByTestId("deck-faceplate");
    expect(within(face).queryByTestId("deck-meters")).toBeNull();
    expect(face.querySelector(".deck-face__body")).not.toHaveClass("has-meters");
    act(() => fullWindow.open("visualizer"));
    expect(within(face).getByTestId("deck-meters")).toBeInTheDocument();
    expect(face.querySelector(".deck-face__body")).toHaveClass("has-meters");
    // Still on with the view closed: it opens in the visualizer next time.
    await act(async () => void (await fullWindow.close()));
    expect(within(face).getByTestId("deck-meters")).toBeInTheDocument();
    // Another mode turns them off.
    act(() => fullWindow.open("lyrics"));
    expect(within(face).queryByTestId("deck-meters")).toBeNull();
    // So does the Visualizer button setting.
    act(() => fullWindow.setMode("visualizer"));
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerButton: false } }));
    expect(within(face).queryByTestId("deck-meters")).toBeNull();
  });
});

describe("the phone", () => {
  afterEach(() => {
    setPhoneVis(false);
    act(() => usePhoneUi.setState({ nowPlaying: false }));
  });

  it("fills now playing with the art behind the song and the controls, at no more than 30 frames", async () => {
    act(() => usePhoneUi.setState({ nowPlaying: true }));
    render(<PhoneNowPlaying />);
    const np = screen.getByRole("dialog", { name: "Now playing" });
    expect(np).not.toHaveClass("is-art");
    const button = screen.getByRole("button", { name: "Visualizer" });
    expect(button).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(button);
    expect(screen.getByRole("button", { name: "Visualizer" })).toHaveAttribute("aria-pressed", "true");
    // Sleeve, the default, over the whole screen, with a dark scrim at the top and the bottom.
    expect(np).toHaveClass("is-art", "is-art--sleeve");
    const art = within(np).getByTestId("art-stage");
    expect(art.querySelector(".art-stage__scrim--top")).not.toBeNull();
    expect(art.querySelector(".art-stage__scrim--bottom")).not.toBeNull();
    // The cover deck stays, its cover the sharp one over the art; the song and controls stay too.
    expect(screen.getByTestId("np-art")).toBeInTheDocument();
    expect(np.querySelector(".ph-np__slide--cur .ph-np__cover")).toHaveClass("art-cover", "art-cover--sleeve");
    expect(within(np).getByRole("button", { name: "Pause" })).toBeInTheDocument();
    // The picker, in its groups.
    const picker = screen.getByTestId("np-art-styles");
    expect(within(picker).getByRole("group", { name: "Art" })).toBeInTheDocument();
    expect(within(picker).getByRole("group", { name: "Instruments" })).toBeInTheDocument();
    // The desktop's 60 is capped at 30 on the phone.
    await waitFor(() => expect(lastCall()).toEqual({ on: true, fps: 30, scope: false }));
    // Remembered on this device.
    expect(usePhoneVis.getState().open).toBe(true);
  });

  it("hides the controls with a tap on the art and brings them back with another", () => {
    setPhoneVis(true);
    act(() => usePhoneUi.setState({ nowPlaying: true }));
    render(<PhoneNowPlaying />);
    const np = screen.getByRole("dialog", { name: "Now playing" });
    fireEvent.click(screen.getByTestId("np-art"));
    expect(np).toHaveClass("is-bare");
    expect(np.querySelector(".ph-np__deck")).toHaveAttribute("inert");
    expect(np.querySelector(".ph-np__top")).toHaveAttribute("inert");
    expect(screen.getByTestId("np-art-styles")).toHaveAttribute("inert");
    // The cover stays.
    expect(np.querySelector(".ph-np__slide--cur .ph-np__cover")).not.toBeNull();
    fireEvent.click(screen.getByTestId("np-player"));
    expect(np).not.toHaveClass("is-bare");
    expect(np.querySelector(".ph-np__deck")).not.toHaveAttribute("inert");
    // A press on a control is the control's own.
    fireEvent.click(within(np).getByRole("button", { name: "Pause" }));
    expect(np).not.toHaveClass("is-bare");
  });

  it("keeps a way back with the controls hidden: a focused Show controls button, and Escape", () => {
    setPhoneVis(true);
    act(() => usePhoneUi.setState({ nowPlaying: true }));
    render(<PhoneNowPlaying />);
    const np = screen.getByRole("dialog", { name: "Now playing" });
    expect(screen.queryByRole("button", { name: "Show controls" })).toBeNull();
    fireEvent.click(screen.getByTestId("np-art"));
    const back = screen.getByRole("button", { name: "Show controls" });
    // Outside the hidden part, and holding focus so the dialog always has something to press.
    expect(back.closest("[inert]")).toBeNull();
    expect(back).toHaveFocus();
    fireEvent.click(back);
    expect(np).not.toHaveClass("is-bare");
    expect(screen.queryByRole("button", { name: "Show controls" })).toBeNull();
    // Escape brings them back too, without closing now playing.
    fireEvent.click(screen.getByTestId("np-art"));
    expect(np).toHaveClass("is-bare");
    act(() => void fireEvent.keyDown(window, { key: "Escape" }));
    expect(np).not.toHaveClass("is-bare");
    expect(usePhoneUi.getState().nowPlaying).toBe(true);
  });

  it("still skips on a sideways swipe over the art", async () => {
    const next = vi.spyOn(player, "next").mockImplementation(() => Promise.resolve());
    setPhoneVis(true);
    act(() => usePhoneUi.setState({ nowPlaying: true }));
    render(<PhoneNowPlaying />);
    const np = screen.getByRole("dialog", { name: "Now playing" });
    expect(np).toHaveClass("is-art");
    const art = screen.getByTestId("np-art");
    Object.defineProperty(art, "clientWidth", { value: 400, configurable: true });
    Object.defineProperty(np, "clientHeight", { value: 800, configurable: true });
    let now = 1000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    fireEvent.pointerDown(art, { pointerId: 1, button: 0, clientX: 300, clientY: 400 });
    for (let i = 1; i <= 5; i++) {
      now += 160;
      fireEvent.pointerMove(art, { pointerId: 1, clientX: 300 - 32 * i, clientY: 400 });
    }
    expect(screen.getByTestId("np-deck").style.transform).toBe("translateX(-160px)");
    await act(async () => void fireEvent.pointerUp(art, { pointerId: 1, clientX: 140, clientY: 400 }));
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("shows an instrument in the cover's place, as before", async () => {
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerStyle: "meters" } }));
    setPhoneVis(true);
    act(() => usePhoneUi.setState({ nowPlaying: true }));
    render(<PhoneNowPlaying />);
    expect(screen.getByRole("dialog", { name: "Now playing" })).not.toHaveClass("is-art");
    expect(screen.queryByTestId("np-art")).toBeNull();
    const vis = within(screen.getByTestId("np-visualizer")).getByTestId("visualizer");
    expect(vis).toHaveClass("vis--phone", "vis--meters");
    expect(screen.queryByTestId("art-stage")).toBeNull();
  });

  it("hides the button with the setting off", async () => {
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerButton: false } }));
    setPhoneVis(true);
    act(() => usePhoneUi.setState({ nowPlaying: true }));
    render(<PhoneNowPlaying />);
    expect(screen.queryByRole("button", { name: "Visualizer" })).toBeNull();
    expect(screen.getByTestId("np-art")).toBeInTheDocument();
  });
});

// ------------------------------------------------------------ the expanded view

describe("the expanded view", () => {
  it("has a Visualizer button and V that fill the view with the art round the cover", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    const button = within(screen.getByRole("group", { name: "Show" })).getByRole("button", { name: "Visualizer" });
    fireEvent.click(button);
    expect(useFullWindow.getState().mode).toBe("visualizer");
    const dialog = screen.getByRole("dialog", { name: "Expanded now playing" });
    expect(dialog).toHaveClass("fw--visualizer", "fw--art", "fw--art-sleeve");
    const art = within(dialog).getByTestId("art-stage");
    expect(art.querySelector(".art-stage__scrim--top")).not.toBeNull();
    expect(dialog.querySelector(".fw__now--art .fw__cover.art-cover")).not.toBeNull();
    expect(within(dialog).getByRole("group", { name: "Visualizer style" })).toBeInTheDocument();
    // A click on the art hides the controls (and the bars), another brings them back.
    fireEvent.click(dialog.querySelector(".fw__stage")!);
    expect(dialog).toHaveClass("is-bare");
    expect(document.documentElement).toHaveClass("fw-idle");
    fireEvent.click(dialog.querySelector(".fw__stage")!);
    expect(dialog).not.toHaveClass("is-bare");
    // V goes back to the artwork, and again to the visualizer.
    act(() => void fireEvent.keyDown(window, { key: "v" }));
    expect(useFullWindow.getState().mode).toBe("artwork");
    act(() => void fireEvent.keyDown(window, { key: "V" }));
    expect(useFullWindow.getState().mode).toBe("visualizer");
  });

  it("takes hidden controls out of reach, and a key brings them back (Escape only that)", () => {
    render(<FullWindow />);
    act(() => fullWindow.open("visualizer"));
    const dialog = screen.getByRole("dialog", { name: "Expanded now playing" });
    const top = dialog.querySelector(".fw__top")!;
    fireEvent.click(dialog.querySelector(".fw__stage")!);
    expect(dialog).toHaveClass("is-bare");
    expect(top).toHaveAttribute("inert");
    expect(document.documentElement).toHaveClass("fw-bare");
    for (const c of dialog.querySelectorAll(".fw__chrome")) expect(c).toHaveAttribute("inert");
    // Any key shows them.
    act(() => void fireEvent.keyDown(window, { key: "a" }));
    expect(dialog).not.toHaveClass("is-bare");
    expect(top).not.toHaveAttribute("inert");
    expect(document.documentElement).not.toHaveClass("fw-bare");
    // Escape shows them and leaves the view open.
    fireEvent.click(dialog.querySelector(".fw__stage")!);
    act(() => void fireEvent.keyDown(window, { key: "Escape" }));
    expect(dialog).not.toHaveClass("is-bare");
    expect(useFullWindow.getState().open).toBe(true);
  });

  it("keeps the instruments in the stage with the cover in the corner", async () => {
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerStyle: "analyser" } }));
    render(<FullWindow />);
    act(() => fullWindow.open("visualizer"));
    const dialog = screen.getByRole("dialog", { name: "Expanded now playing" });
    expect(dialog).toHaveClass("fw--visualizer");
    expect(dialog).not.toHaveClass("fw--art");
    expect(within(dialog).getByTestId("visualizer")).toBeInTheDocument();
    expect(dialog.querySelector(".fw__now--row .fw__cover")).not.toBeNull();
    expect(screen.queryByTestId("art-stage")).toBeNull();
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
    act(() => void fireEvent.keyDown(window, { key: "v" }));
    expect(useFullWindow.getState().mode).toBe("visualizer");
  });

  it("hides the button, and V does nothing, with the setting off", async () => {
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerButton: false } }));
    render(<FullWindow />);
    act(() => fullWindow.open());
    expect(within(screen.getByRole("group", { name: "Show" })).queryByRole("button", { name: "Visualizer" })).toBeNull();
    act(() => void fireEvent.keyDown(window, { key: "v" }));
    expect(useFullWindow.getState().mode).toBe("artwork");
    // A remembered visualizer mode shows the artwork while the button is off.
    act(() => useFullWindow.setState({ mode: "visualizer" }));
    expect(screen.getByRole("dialog", { name: "Expanded now playing" })).toHaveClass("fw--artwork");
    expect(screen.queryByTestId("visualizer")).toBeNull();
    expect(screen.queryByTestId("art-stage")).toBeNull();
  });
});

// ------------------------------------------------------------ settings

describe("Settings", () => {
  it("has every visualizer row under Playback with the defaults", async () => {
    await mock.settingsSet({ ...DEFAULT_SETTINGS });
    await loadSettings();
    render(<SettingsPage />);
    const button = await screen.findByRole("switch", { name: "Show the Visualizer button" });
    expect(button).toBeChecked();
    expect(screen.getByRole("switch", { name: "Analyser under the cover in the now playing panel" })).not.toBeChecked();
    expect(screen.getByRole("switch", { name: "Rest the visualizer on battery" })).toBeChecked();
    for (const key of ["visualizerButton", "visualizerStyle", "visualizerIntensity", "visualizerFps", "visualizerSmoothing", "visualizerPanelStrip", "visualizerBatteryPause"]) {
      const row = document.querySelector<HTMLElement>(`[data-setting="${key}"]`);
      expect(row, key).not.toBeNull();
      expect(row!.closest("section")?.querySelector("h2")?.textContent).toMatch(/Playback/);
    }
    const style = document.querySelector<HTMLElement>('[data-setting="visualizerStyle"]')!;
    const select = style.querySelector("select")!;
    expect(select.value).toBe("sleeve");
    // The styles in their groups, Art first.
    const groups = [...select.querySelectorAll("optgroup")];
    expect(groups.map((g) => g.label)).toEqual(["Art", "Instruments"]);
    expect([...groups[0].querySelectorAll("option")].map((o) => o.textContent)).toEqual(["Sleeve", "Ink", "Grooves", "Light"]);
    expect([...groups[1].querySelectorAll("option")].map((o) => o.textContent)).toEqual(["Analyser", "Meters", "Scope"]);
    const intensity = document.querySelector<HTMLElement>('[data-setting="visualizerIntensity"]')!;
    expect(intensity.querySelector("select")!.value).toBe("calm");
    expect([...intensity.querySelectorAll("option")].map((o) => o.textContent)).toEqual(["Calm", "Lively"]);
    expect(document.querySelector('[data-setting="visualizerFps"]')).toHaveTextContent("60 frames a second");
    expect(document.querySelector('[data-setting="visualizerSmoothing"]')).toHaveTextContent("Medium");
    // The device-only ones say so.
    for (const key of ["visualizerFps", "visualizerPanelStrip", "visualizerBatteryPause"])
      expect(document.querySelector(`[data-setting="${key}"]`)).toHaveTextContent("This device only");
    // Each is saved.
    fireEvent.change(intensity.querySelector("select")!, { target: { value: "lively" } });
    await waitFor(async () => expect((await mock.settingsGet()).visualizerIntensity).toBe("lively"));
    fireEvent.change(select, { target: { value: "grooves" } });
    await waitFor(async () => expect((await mock.settingsGet()).visualizerStyle).toBe("grooves"));
    // Turning the button off saves it.
    fireEvent.click(button);
    await waitFor(async () => expect((await mock.settingsGet()).visualizerButton).toBe(false));
  });
});
