/**
 * The full-window now-playing view: ways in and out (including OS full
 * screen and its Esc order), the controls fading, keys, and the lyrics side.
 */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { CONTROLS_HIDE_MS, FullWindow } from "../shell/FullWindow";
import { clearLyricsCache } from "../shell/lyrics";
import { PlayerBar } from "../shell/PlayerBar";
import { VOLUME_KEY_STEP } from "../shell/useShortcuts";
import { fullWindow, setOsFullscreenImpl, useFullWindow } from "../stores/fullWindow";
import { player, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const byKind = (k: ReturnType<typeof mockLyricsKind>) => mock.fx.tracks.find((t) => mockLyricsKind(t.id) === k && t.durationMs > 120_000)!;
const synced = byKind("words");
const none = byKind("none");

function play(track: Track, positionMs = 60_000) {
  const s: PlayerState = {
    status: "paused",
    track,
    positionMs,
    durationMs: track.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: null, userQueue: [], nextUp: [], history: [], context: { kind: "album", id: track.albumId, title: track.albumTitle } },
  };
  act(() => usePlayer.setState({ state: s, positionMs, durationMs: track.durationMs }));
}

/** A fake window full screen. */
const os = { on: false, set: vi.fn(async (v: boolean) => void (os.on = v)), get: async () => os.on };

const view = () => screen.queryByRole("dialog", { name: "Now playing, full window" });
const key = (k: string, target: Element | Window = window) => act(() => void fireEvent.keyDown(target, { key: k }));

beforeEach(() => {
  clearLyricsCache();
  os.on = false;
  os.set.mockClear();
  setOsFullscreenImpl(os);
  useFullWindow.setState({ open: false, lyrics: true, osFullscreen: false });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useUi.setState({ nowPlayingOpen: false, nowPlayingExpanded: false, queueOpen: false, menu: null });
  useRouter.getState().reset();
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
  play(synced);
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("opening and closing", () => {
  it("opens from the expand button at the far right of the player bar", () => {
    render(
      <>
        <PlayerBar />
        <FullWindow />
      </>,
    );
    const extras = document.querySelector(".player-bar__extras")!;
    const buttons = extras.querySelectorAll("button");
    expect(buttons[buttons.length - 1]).toHaveAccessibleName("Full-window view");
    fireEvent.click(screen.getByRole("button", { name: "Full-window view" }));
    expect(view()).toBeInTheDocument();
    expect(view()).toHaveFocus();
  });

  it("opens on a double-click on the player bar cover, leaving the now-playing panel as it was", () => {
    render(
      <>
        <PlayerBar />
        <FullWindow />
      </>,
    );
    const cover = screen.getByRole("button", { name: "Now playing view" });
    fireEvent.click(cover, { detail: 1 });
    fireEvent.click(cover, { detail: 2 });
    fireEvent.doubleClick(cover);
    expect(view()).toBeInTheDocument();
    expect(useUi.getState().nowPlayingOpen).toBe(false);
  });

  it("closes with Esc, the close button, or a double-click on the background", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    key("Escape");
    await waitFor(() => expect(view()).toBeNull());

    act(() => fullWindow.open());
    fireEvent.click(screen.getByRole("button", { name: "Close full-window view" }));
    await waitFor(() => expect(view()).toBeNull());

    act(() => fullWindow.open());
    // A double-click on a control doesn't close it; on the background it does.
    fireEvent.doubleClick(screen.getByRole("button", { name: "Enter full screen" }));
    expect(view()).toBeInTheDocument();
    fireEvent.doubleClick(document.querySelector(".fw__stage")!);
    await waitFor(() => expect(view()).toBeNull());
  });

  it("in OS full screen, the first Esc leaves full screen and the second closes", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    fireEvent.click(screen.getByRole("button", { name: "Enter full screen" }));
    await waitFor(() => expect(useFullWindow.getState().osFullscreen).toBe(true));
    expect(os.set).toHaveBeenLastCalledWith(true);
    expect(screen.getByRole("button", { name: "Exit full screen" })).toBeInTheDocument();
    key("Escape");
    await waitFor(() => expect(useFullWindow.getState().osFullscreen).toBe(false));
    expect(view()).toBeInTheDocument();
    key("Escape");
    await waitFor(() => expect(view()).toBeNull());
  });

  it("closing from OS full screen leaves full screen too", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    key("f");
    await waitFor(() => expect(os.on).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: "Close full-window view" }));
    await waitFor(() => expect(view()).toBeNull());
    expect(os.on).toBe(false);
  });

  it("steps aside when a link goes to a page", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    act(() => useRouter.getState().navigate({ name: "album", id: synced.albumId }));
    await waitFor(() => expect(view()).toBeNull());
  });
});

describe("controls fade", () => {
  it("after 3 s without movement, and come back on movement or a key", () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    render(<FullWindow />);
    act(() => fullWindow.open());
    const root = view()!;
    expect(root).not.toHaveClass("is-idle");
    act(() => vi.advanceTimersByTime(CONTROLS_HIDE_MS));
    expect(root).toHaveClass("is-idle");
    act(() => void fireEvent.mouseMove(window));
    expect(root).not.toHaveClass("is-idle");
    act(() => vi.advanceTimersByTime(CONTROLS_HIDE_MS));
    expect(root).toHaveClass("is-idle");
    key("Shift");
    expect(root).not.toHaveClass("is-idle");
  });

  it("never with the setting off", () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, fullWindowHideControls: false } });
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    render(<FullWindow />);
    act(() => fullWindow.open());
    act(() => vi.advanceTimersByTime(CONTROLS_HIDE_MS * 3));
    expect(view()).not.toHaveClass("is-idle");
  });
});

describe("keys", () => {
  it("Space plays or pauses, arrows seek and change volume", () => {
    const toggle = vi.spyOn(backend, "playToggle").mockResolvedValue();
    const seek = vi.spyOn(backend, "playSeek").mockResolvedValue();
    render(<FullWindow />);
    act(() => fullWindow.open());
    key(" ", view()!);
    expect(toggle).toHaveBeenCalledTimes(1);
    key("ArrowRight", view()!);
    expect(seek).toHaveBeenLastCalledWith(65_000);
    key("ArrowLeft", view()!);
    expect(seek).toHaveBeenLastCalledWith(60_000);
    const nudge = vi.spyOn(player, "nudgeVolume").mockResolvedValue();
    key("ArrowUp", view()!);
    expect(nudge).toHaveBeenLastCalledWith(VOLUME_KEY_STEP);
    key("ArrowDown", view()!);
    expect(nudge).toHaveBeenLastCalledWith(-VOLUME_KEY_STEP);
  });

  it("L toggles the lyrics side and F toggles OS full screen", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    await waitFor(() => expect(document.querySelector(".fw__lyrics")).not.toBeNull());
    key("l");
    expect(document.querySelector(".fw__lyrics")).toBeNull();
    key("L");
    expect(document.querySelector(".fw__lyrics")).not.toBeNull();
    key("f");
    await waitFor(() => expect(useFullWindow.getState().osFullscreen).toBe(true));
    key("F");
    await waitFor(() => expect(useFullWindow.getState().osFullscreen).toBe(false));
  });
});

describe("lyrics side", () => {
  it("sits beside the cover on wide windows", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    await waitFor(() => expect(document.querySelector(".fw__lyrics")).not.toBeNull());
    expect(view()).toHaveClass("is-wide", "has-lyrics");
    expect(view()).not.toHaveClass("is-compact");
    expect(screen.getByRole("button", { name: "Lyrics" })).toHaveAttribute("aria-pressed", "true");
  });

  it("starts hidden on narrow windows and swaps in for the cover", async () => {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 800 });
    render(<FullWindow />);
    act(() => fullWindow.open());
    await act(() => new Promise((r) => setTimeout(r, 0)));
    expect(document.querySelector(".fw__lyrics")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Lyrics" }));
    await waitFor(() => expect(view()).toHaveClass("is-compact"));
  });

  it("isn't offered for a song without lyrics", async () => {
    play(none);
    render(<FullWindow />);
    act(() => fullWindow.open());
    const btn = screen.getByRole("button", { name: "Lyrics" });
    await waitFor(() => expect(btn).toHaveAttribute("aria-disabled", "true"));
    expect(document.querySelector(".fw__lyrics")).toBeNull();
    expect(screen.getByText(none.title)).toBeInTheDocument();
  });
});
