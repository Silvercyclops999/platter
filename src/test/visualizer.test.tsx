/**
 * The visualizer: its scales and needles, the feed that asks the core to
 * watch only while a view is on screen, each style drawing from mock frames,
 * reduced motion, the expanded view's button and V key, and its Settings rows.
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
import { CALM_FPS, VisualizerView, shownStyle, visFps } from "../visualizer/VisualizerView";

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
  let now = 0;
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
    queue.push(cb);
    return queue.length;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
  return (n: number) => {
    for (let i = 0; i < n; i++) {
      const run = queue;
      queue = [];
      now = performance.now();
      act(() => run.forEach((cb) => cb(now)));
    }
  };
}

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

  it("draws a mono pair upright on the scope and reads the low end for Glow", () => {
    const [x, y] = scopePoint(0.5, 0.5);
    expect(x).toBe(0);
    expect(y).toBeGreaterThan(0);
    const [sx] = scopePoint(0.5, -0.5);
    expect(Math.abs(sx)).toBeGreaterThan(0.5);
    expect(lowEnd(null)).toBe(0);
    expect(lowEnd(new Array(BANDS).fill(FLOOR_DB))).toBeLessThan(0.001);
    expect(lowEnd(new Array(BANDS).fill(-6))).toBeGreaterThan(0.5);
  });

  it("keeps only Analyser and Meters, at 15 frames, under reduced motion; a phone runs at most 30", () => {
    expect(shownStyle("scope", true)).toBe("analyser");
    expect(shownStyle("glow", true)).toBe("analyser");
    expect(shownStyle("meters", true)).toBe("meters");
    expect(shownStyle("glow", false)).toBe("glow");
    expect(visFps(60, false, true)).toBe(CALM_FPS);
    expect(visFps(60, true, false)).toBe(30);
    expect(visFps(15, true, false)).toBe(15);
    expect(visFps(60, false, false)).toBe(60);
  });
});

// ------------------------------------------------------------ the feed

describe("the feed", () => {
  it("asks the core to watch while a view wants frames, renews it, and ends it when the view goes", async () => {
    vi.useFakeTimers();
    const stop = watchFeed({ fps: 60, scope: false });
    await vi.advanceTimersByTimeAsync(0);
    expect(mock.visualizer.calls.at(-1)).toEqual({ on: true, fps: 60, scope: false });
    expect(useFeed.getState().running).toBe(true);
    // A second view asks for the scope: one watch, with the scope.
    const stop2 = watchFeed({ fps: 30, scope: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(mock.visualizer.calls.at(-1)).toEqual({ on: true, fps: 60, scope: true });
    // Renewed about once a second, well inside the core's 3 s.
    const before = mock.visualizer.calls.length;
    await vi.advanceTimersByTimeAsync(2_100);
    expect(mock.visualizer.calls.length - before).toBe(2);
    stop2();
    stop();
    await vi.advanceTimersByTimeAsync(0);
    expect(mock.visualizer.calls.at(-1)?.on).toBe(false);
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
    expect(mock.visualizer.calls.at(-1)?.on).toBe(false);
    const n = mock.visualizer.calls.length;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(mock.visualizer.calls.length).toBe(n);
    setHidden(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(mock.visualizer.calls.at(-1)?.on).toBe(true);
    stop();
  });

  it("keeps the frames the core sends", async () => {
    const stop = watchFeed({ fps: 30, scope: false });
    frame(2_000, false);
    expect(latestFrame().frame?.bands).toHaveLength(BANDS);
    stop();
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
    await waitFor(() => expect(mock.visualizer.calls.at(-1)).toEqual({ on: true, fps: 60, scope: false }));
  });

  it("Analyser uses WebGL2 when it's there, and Canvas 2D after the GPU context is lost", async () => {
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
    expect(within(view).getByText(/0 VU = −14 dBFS/)).toBeInTheDocument();
    frame();
    run(3);
    // Two needles and two lamps a frame.
    expect(drawn.calls.filter((c) => c === "arc").length).toBeGreaterThanOrEqual(2);
    expect(drawn.calls.filter((c) => c === "lineTo").length).toBeGreaterThanOrEqual(2);
  });

  it("picking a style keeps it (synced setting)", async () => {
    render(<VisualizerView />);
    fireEvent.click(screen.getByRole("radio", { name: "Meters" }));
    await waitFor(() => expect(useSettings.getState().settings?.visualizerStyle).toBe("meters"));
    expect((await mock.settingsGet()).visualizerStyle).toBe("meters");
  });

  it("under reduced motion offers only Analyser and Meters, without the peak line, at 15 frames", async () => {
    setReducedMotion(true);
    await act(async () => void useSettings.setState({ settings: { ...DEFAULT_SETTINGS, visualizerStyle: "scope" } }));
    render(<VisualizerView />);
    expect(screen.getByRole("radio", { name: "Scope" })).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("radio", { name: "Glow" })).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("radio", { name: "Analyser" })).toHaveAttribute("aria-checked", "true");
    await waitFor(() => expect(mock.visualizer.calls.at(-1)).toEqual({ on: true, fps: 15, scope: false }));
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

// ------------------------------------------------------------ the expanded view

describe("the expanded view", () => {
  it("has a Visualizer button and V that swap the cover for the visualizer, the cover moving to the corner", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    const button = within(screen.getByRole("group", { name: "Show" })).getByRole("button", { name: "Visualizer" });
    fireEvent.click(button);
    expect(useFullWindow.getState().mode).toBe("visualizer");
    const dialog = screen.getByRole("dialog", { name: "Expanded now playing" });
    expect(dialog).toHaveClass("fw--visualizer");
    expect(within(dialog).getByTestId("visualizer")).toBeInTheDocument();
    expect(dialog.querySelector(".fw__now--row .fw__cover")).not.toBeNull();
    // V goes back to the artwork, and again to the visualizer.
    act(() => void fireEvent.keyDown(window, { key: "v" }));
    expect(useFullWindow.getState().mode).toBe("artwork");
    act(() => void fireEvent.keyDown(window, { key: "V" }));
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
    for (const key of ["visualizerButton", "visualizerStyle", "visualizerFps", "visualizerSmoothing", "visualizerPanelStrip", "visualizerBatteryPause"]) {
      const row = document.querySelector<HTMLElement>(`[data-setting="${key}"]`);
      expect(row, key).not.toBeNull();
      expect(row!.closest("section")?.querySelector("h2")?.textContent).toMatch(/Playback/);
    }
    const style = document.querySelector<HTMLElement>('[data-setting="visualizerStyle"]')!;
    expect(style).toHaveTextContent("Analyser");
    expect(document.querySelector('[data-setting="visualizerFps"]')).toHaveTextContent("60 frames a second");
    expect(document.querySelector('[data-setting="visualizerSmoothing"]')).toHaveTextContent("Medium");
    // The device-only ones say so.
    for (const key of ["visualizerFps", "visualizerPanelStrip", "visualizerBatteryPause"])
      expect(document.querySelector(`[data-setting="${key}"]`)).toHaveTextContent("This device only");
    // Turning the button off saves it.
    fireEvent.click(button);
    await waitFor(async () => expect((await mock.settingsGet()).visualizerButton).toBe(false));
  });
});
