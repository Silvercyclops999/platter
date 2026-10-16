import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type PlayerState, type Settings } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { PlayerBar } from "../shell/PlayerBar";
import { inUpNextWindow, normalizeLoop, upNextShown, upNextWindowMs, volumeLabel } from "../shell/playerRules";
import { connectPlayer, resetVolumeNudge, usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
let disconnect: () => void;

beforeAll(async () => {
  disconnect = connectPlayer(mock);
  await waitFor(() => expect(usePlayer.getState().state?.track).toBeTruthy());
});
afterAll(() => disconnect());
beforeEach(async () => {
  setSettings({});
  await act(async () => {
    await mock.playPause();
    await mock.setAbLoop(null);
    await mock.setRepeat("off");
  });
});
afterEach(() => {
  vi.restoreAllMocks();
  resetVolumeNudge();
});

function setSettings(patch: Partial<Settings>) {
  useSettings.getState().setLocal({ ...DEFAULT_SETTINGS, ...patch });
}

const seekSlider = () => screen.getByRole("slider", { name: "Seek" });
const duration = () => usePlayer.getState().durationMs;
const coreLoop = async () => (await mock.playerState()).abLoop ?? null;

/** Every element is 100 px wide at x = 0, so clientX is a percentage. */
function sizeEverything() {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
    left: 0,
    width: 100,
    top: 0,
    height: 16,
    right: 100,
    bottom: 16,
    x: 0,
    y: 0,
    toJSON() {},
  } as DOMRect);
}

const rightClick = (el: HTMLElement, x: number) => fireEvent.contextMenu(el, { clientX: x });

async function seekTo(ms: number) {
  await act(async () => {
    await mock.playSeek(ms);
  });
  await waitFor(() => expect(usePlayer.getState().positionMs).toBe(ms));
}

// ---------------------------------------------------------------- rules

describe("player bar rules", () => {
  it("loop points swap, clamp and need 1 s between them", () => {
    expect(normalizeLoop(5_000, 2_000, 10_000)).toEqual({ aMs: 2_000, bMs: 5_000 });
    expect(normalizeLoop(9_000, 20_000, 10_000)).toEqual({ aMs: 9_000, bMs: 10_000 });
    expect(normalizeLoop(1_000, 1_999, 10_000)).toBeNull();
    expect(normalizeLoop(1_000, 2_000, 10_000)).toEqual({ aMs: 1_000, bMs: 2_000 });
  });

  it("up next shows in the last 15 s, or the last 30 % of short tracks", () => {
    expect(upNextWindowMs(200_000)).toBe(15_000);
    expect(upNextWindowMs(50_000)).toBe(15_000);
    expect(upNextWindowMs(40_000)).toBe(12_000);
    expect(inUpNextWindow(184_999, 200_000)).toBe(false);
    expect(inUpNextWindow(185_000, 200_000)).toBe(true);
    expect(inUpNextWindow(27_000, 40_000)).toBe(false);
    expect(inUpNextWindow(28_000, 40_000)).toBe(true);
    expect(inUpNextWindow(0, 0)).toBe(false);
  });

  it("up next is hidden with repeat one, a loop, nothing next, or the setting off", () => {
    const base = usePlayer.getState().state!;
    const s = (p: Partial<PlayerState>): PlayerState => ({ ...base, ...p });
    const dur = 200_000;
    const at = 190_000;
    const next = base.queue.nextUp[0];
    expect(next).toBeTruthy();
    expect(upNextShown(s({}), at, dur, true)).toBe(next);
    expect(upNextShown(s({}), 100_000, dur, true)).toBeNull();
    expect(upNextShown(s({}), at, dur, false)).toBeNull();
    expect(upNextShown(s({ repeat: "one" }), at, dur, true)).toBeNull();
    expect(upNextShown(s({ abLoop: { aMs: 0, bMs: 5_000 } }), at, dur, true)).toBeNull();
    expect(upNextShown(s({ queue: { ...base.queue, nextUp: [], userQueue: [] } }), at, dur, true)).toBeNull();
    // The user queue comes first.
    const queued = { ...next, itemId: "q1", source: "user" as const };
    expect(upNextShown(s({ queue: { ...base.queue, userQueue: [queued] } }), at, dur, true)).toBe(queued);
  });

  it("the readout says Muted at 0 or when muted", () => {
    expect(volumeLabel(0.42, false)).toBe("42%");
    expect(volumeLabel(0, false)).toBe("Muted");
    expect(volumeLabel(0.5, true)).toBe("Muted");
  });
});

// ---------------------------------------------------------------- A-B loop

describe("A-B loop", () => {
  it("right-click sets A, the second sets B, and the core loops between them", async () => {
    render(<PlayerBar />);
    sizeEverything();
    const s = seekSlider();
    rightClick(s, 60);
    // A alone shows as a pending mark; nothing is sent yet.
    expect(document.querySelector(".ab-handle--pending")).toBeInTheDocument();
    expect(await coreLoop()).toBeNull();
    // B before A swaps them.
    rightClick(s, 20);
    await waitFor(async () =>
      expect(await coreLoop()).toEqual({ aMs: Math.round(duration() * 0.2), bMs: Math.round(duration() * 0.6) }),
    );
    expect(await screen.findByRole("slider", { name: "Loop start" })).toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Loop end" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Clear A-B loop" })).toHaveTextContent("A-B");
    expect(document.querySelector(".ab-span")).toBeInTheDocument();
  });

  it("Alt+click works like right-click and doesn't seek", async () => {
    render(<PlayerBar />);
    sizeEverything();
    const seek = vi.spyOn(backend, "playSeek");
    const s = seekSlider();
    fireEvent.pointerDown(s, { button: 0, altKey: true, clientX: 10, pointerId: 1 });
    fireEvent.pointerDown(s, { button: 0, altKey: true, clientX: 30, pointerId: 1 });
    await waitFor(async () => expect((await coreLoop())?.aMs).toBe(Math.round(duration() * 0.1)));
    expect(seek).not.toHaveBeenCalled();
  });

  it("a second point under 1 s from A is ignored", async () => {
    render(<PlayerBar />);
    sizeEverything();
    const s = seekSlider();
    rightClick(s, 50);
    const near = 50 + (500 / duration()) * 100;
    rightClick(s, near);
    await new Promise((r) => setTimeout(r, 20));
    expect(await coreLoop()).toBeNull();
    expect(document.querySelector(".ab-handle--pending")).toBeInTheDocument();
  });

  it("the chip, Esc on the seek bar and a click outside the span each clear it", async () => {
    const user = userEvent.setup();
    render(<PlayerBar />);
    sizeEverything();
    const s = seekSlider();
    const d = duration();
    const set = async () => {
      await act(async () => {
        await mock.setAbLoop({ aMs: Math.round(d * 0.2), bMs: Math.round(d * 0.6) });
      });
      await screen.findByRole("button", { name: "Clear A-B loop" });
    };
    await set();
    await user.click(screen.getByRole("button", { name: "Clear A-B loop" }));
    await waitFor(async () => expect(await coreLoop()).toBeNull());
    expect(screen.queryByRole("button", { name: "Clear A-B loop" })).not.toBeInTheDocument();

    await set();
    s.focus();
    fireEvent.keyDown(s, { key: "Escape" });
    await waitFor(async () => expect(await coreLoop()).toBeNull());

    await set();
    // Inside the span: kept.
    fireEvent.pointerDown(s, { button: 0, clientX: 40, pointerId: 1 });
    fireEvent.pointerUp(s, { clientX: 40, pointerId: 1 });
    await waitFor(() => expect(usePlayer.getState().positionMs).toBe(Math.round(d * 0.4)));
    expect(await coreLoop()).not.toBeNull();
    // Outside: cleared.
    fireEvent.pointerDown(s, { button: 0, clientX: 90, pointerId: 1 });
    fireEvent.pointerUp(s, { clientX: 90, pointerId: 1 });
    await waitFor(async () => expect(await coreLoop()).toBeNull());
  });

  it("dragging a handle moves that point; dragging A past B swaps them", async () => {
    render(<PlayerBar />);
    sizeEverything();
    const d = duration();
    await act(async () => {
      await mock.setAbLoop({ aMs: Math.round(d * 0.2), bMs: Math.round(d * 0.6) });
    });
    const a = await screen.findByRole("slider", { name: "Loop start" });
    fireEvent.pointerDown(a, { button: 0, clientX: 20, pointerId: 2 });
    fireEvent.pointerMove(a, { clientX: 30, pointerId: 2 });
    fireEvent.pointerUp(a, { clientX: 30, pointerId: 2 });
    await waitFor(async () => expect(await coreLoop()).toEqual({ aMs: Math.round(d * 0.3), bMs: Math.round(d * 0.6) }));

    const a2 = screen.getByRole("slider", { name: "Loop start" });
    fireEvent.pointerDown(a2, { button: 0, clientX: 30, pointerId: 3 });
    fireEvent.pointerMove(a2, { clientX: 80, pointerId: 3 });
    fireEvent.pointerUp(a2, { clientX: 80, pointerId: 3 });
    await waitFor(async () => expect(await coreLoop()).toEqual({ aMs: Math.round(d * 0.6), bMs: Math.round(d * 0.8) }));
    // The handle press didn't seek.
    expect(usePlayer.getState().positionMs).not.toBe(Math.round(d * 0.8));
  });

  it("handles take arrow keys in 1 s steps", async () => {
    render(<PlayerBar />);
    await act(async () => {
      await mock.setAbLoop({ aMs: 10_000, bMs: 20_000 });
    });
    const b = await screen.findByRole("slider", { name: "Loop end" });
    fireEvent.keyDown(b, { key: "ArrowRight" });
    await waitFor(async () => expect(await coreLoop()).toEqual({ aMs: 10_000, bMs: 21_000 }));
  });

  it("a track change clears the loop", async () => {
    render(<PlayerBar />);
    await act(async () => {
      await mock.setAbLoop({ aMs: 10_000, bMs: 20_000 });
    });
    await screen.findByRole("button", { name: "Clear A-B loop" });
    await act(async () => {
      await mock.playNext();
      await mock.playPause();
    });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Clear A-B loop" })).not.toBeInTheDocument());
    expect(await coreLoop()).toBeNull();
    await act(async () => {
      await mock.playPrevious();
      await mock.playPause();
    });
  });

  it("with the setting off there are no handles and right-click sets nothing", async () => {
    setSettings({ abLoopHandles: false });
    render(<PlayerBar />);
    sizeEverything();
    const s = seekSlider();
    rightClick(s, 20);
    rightClick(s, 60);
    await new Promise((r) => setTimeout(r, 20));
    expect(await coreLoop()).toBeNull();
    expect(document.querySelector(".ab-layer")).not.toBeInTheDocument();
  });

  it("turning the setting off clears a set loop", async () => {
    await act(async () => {
      await mock.setAbLoop({ aMs: 10_000, bMs: 20_000 });
      await mock.settingsSet({ abLoopHandles: false });
    });
    expect(await coreLoop()).toBeNull();
    await mock.settingsSet({ abLoopHandles: true });
  });

  it("the mock core wraps playback at B back to A", async () => {
    vi.useFakeTimers();
    try {
      await mock.playSeek(10_500);
      await mock.setAbLoop({ aMs: 10_000, bMs: 11_000 });
      await mock.playResume();
      await vi.advanceTimersByTimeAsync(750);
      const p = (await mock.playerState()).positionMs;
      expect(p).toBeGreaterThanOrEqual(10_000);
      expect(p).toBeLessThan(11_000);
      await mock.playPause();
    } finally {
      vi.useRealTimers();
    }
  });
});

// ---------------------------------------------------------------- up next

describe("up next line", () => {
  it("near the end the artist line becomes Next, and clicking it opens the queue", async () => {
    const user = userEvent.setup();
    useUi.getState().setQueueOpen(false);
    render(<PlayerBar />);
    const next = usePlayer.getState().state!.queue.nextUp[0].track;
    expect(screen.queryByRole("button", { name: new RegExp(`^Next: ${next.title}`) })).not.toBeInTheDocument();
    await seekTo(duration() - 10_000);
    const line = await screen.findByRole("button", { name: new RegExp(`^Next: ${next.title}`) });
    expect(line).toHaveTextContent(`· ${next.trackArtist || next.artistTitle}`);
    await user.click(line);
    expect(useUi.getState().queueOpen).toBe(true);
    useUi.getState().setQueueOpen(false);
  });

  it("stays hidden with repeat one, a loop set, or the setting off", async () => {
    render(<PlayerBar />);
    await seekTo(duration() - 5_000);
    await screen.findByRole("button", { name: /^Next: / });
    await act(async () => {
      await mock.setRepeat("one");
    });
    await waitFor(() => expect(screen.queryByRole("button", { name: /^Next: / })).not.toBeInTheDocument());
    await act(async () => {
      await mock.setRepeat("off");
      await mock.setAbLoop({ aMs: duration() - 8_000, bMs: duration() - 1_000 });
    });
    await waitFor(() => expect(screen.queryByRole("button", { name: /^Next: / })).not.toBeInTheDocument());
    await act(async () => {
      await mock.setAbLoop(null);
    });
    await screen.findByRole("button", { name: /^Next: / });
    act(() => setSettings({ upNextLine: false }));
    await waitFor(() => expect(screen.queryByRole("button", { name: /^Next: / })).not.toBeInTheDocument());
  });
});

// ---------------------------------------------------------------- settings

describe("settings rows", () => {
  it("each feature has its switch with the right default, and changes save", async () => {
    const user = userEvent.setup();
    const save = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    const sw = (name: string) => screen.getByRole("switch", { name });
    expect(sw("Loop part of a song from the seek bar")).toBeChecked();
    expect(sw("Show what's next in the player bar")).toBeChecked();
    expect(sw("Show volume percentage")).toBeChecked();
    expect(sw("Notify on track change")).not.toBeChecked();
    await user.click(sw("Notify on track change"));
    expect(save).toHaveBeenCalledWith({ notifyTrackChange: true });
    await user.click(sw("Loop part of a song from the seek bar"));
    expect(save).toHaveBeenCalledWith({ abLoopHandles: false });
    await user.click(sw("Show what's next in the player bar"));
    expect(save).toHaveBeenCalledWith({ upNextLine: false });
    await user.click(sw("Show volume percentage"));
    expect(save).toHaveBeenCalledWith({ volumePercent: false });
    // Sections: loop under Playback, the two looks under Appearance, notify under App.
    const section = (name: string) => screen.getByRole("switch", { name }).closest("section")!.querySelector("h2")!.textContent;
    expect(section("Loop part of a song from the seek bar")).toBe("Playback");
    expect(section("Show volume percentage")).toBe("Appearance");
    expect(section("Notify on track change")).toBe("App");
    await mock.settingsSet({ ...DEFAULT_SETTINGS });
  });
});

// ---------------------------------------------------------------- volume readout

describe("volume readout", () => {
  const readout = () => document.querySelector(".volume-readout") as HTMLElement;
  const volumeSlider = () => screen.getByRole("slider", { name: "Volume" });

  beforeEach(async () => {
    await act(async () => {
      await mock.setVolume(0.5);
      await mock.setMuted(false);
    });
    await waitFor(() => expect(usePlayer.getState().state?.volume).toBe(0.5));
  });

  it("is hidden until the level changes", () => {
    render(<PlayerBar />);
    expect(readout()).not.toHaveClass("is-shown");
  });

  it("shows while dragging and fades about a second after letting go", async () => {
    render(<PlayerBar />);
    sizeEverything();
    const s = volumeSlider();
    fireEvent.pointerDown(s, { button: 0, clientX: 30, pointerId: 1 });
    fireEvent.pointerMove(s, { clientX: 35, pointerId: 1 });
    expect(readout()).toHaveClass("is-shown");
    expect(readout()).toHaveTextContent("35%");
    // Held while the button is down, however long.
    await new Promise((r) => setTimeout(r, 1_200));
    expect(readout()).toHaveClass("is-shown");
    fireEvent.pointerUp(s, { clientX: 35, pointerId: 1 });
    await waitFor(() => expect(readout()).not.toHaveClass("is-shown"), { timeout: 2_000 });
  });

  it("shows on the wheel and on keys, then fades", async () => {
    render(<PlayerBar />);
    fireEvent.wheel(volumeSlider(), { deltaY: -100 });
    await waitFor(() => expect(readout()).toHaveClass("is-shown"));
    expect(readout()).toHaveTextContent("60%");
    await waitFor(() => expect(readout()).not.toHaveClass("is-shown"), { timeout: 2_000 });
    fireEvent.keyDown(volumeSlider(), { key: "ArrowDown" });
    await waitFor(() => expect(readout()).toHaveClass("is-shown"));
    expect(readout()).toHaveTextContent("50%");
  });

  it("says Muted when muted", async () => {
    const user = userEvent.setup();
    render(<PlayerBar />);
    await user.click(screen.getByRole("button", { name: "Mute" }));
    await waitFor(() => expect(readout()).toHaveClass("is-shown"));
    expect(readout()).toHaveTextContent("Muted");
  });

  it("is not drawn with the setting off", async () => {
    setSettings({ volumePercent: false });
    render(<PlayerBar />);
    fireEvent.wheel(volumeSlider(), { deltaY: -100 });
    await new Promise((r) => setTimeout(r, 20));
    expect(readout()).toBeNull();
  });
});
