/**
 * Right-panel views over the expanded now-playing view: the device picker,
 * the queue and friend activity open beside it (the view narrows and stays),
 * the now-playing panel is its alternative, and Esc, Minimize and going to a
 * page leave the view with the panel still open.
 */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { AppShell } from "../shell/AppShell";
import { clearLyricsCache } from "../shell/lyrics";
import { clearNowPlayingCache } from "../shell/nowPlaying";
import { FULL_WINDOW_PREFS_DEFAULT, fullWindow, setOsFullscreenImpl, useFullWindow } from "../stores/fullWindow";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { NOW_PLAYING_PREFS_DEFAULT, useUi } from "../stores/ui";

const mock = backend as MockBackend;
const synced = mock.fx.tracks.find((t) => mockLyricsKind(t.id) === "words" && t.durationMs > 120_000)!;

function play(track: Track) {
  const s: PlayerState = {
    status: "paused",
    track,
    positionMs: 60_000,
    durationMs: track.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: null, userQueue: [], nextUp: [], history: [], context: { kind: "album", id: track.albumId, title: track.albumTitle } },
  };
  act(() => usePlayer.setState({ state: s, positionMs: 60_000, durationMs: track.durationMs }));
}

const os = { on: false, set: async (v: boolean) => void (os.on = v), get: async () => os.on };

const view = () => screen.queryByRole("dialog", { name: "Expanded now playing" });
const app = () => document.querySelector(".app")!;
const beside = () => app().classList.contains("fw-beside");
const devices = () => screen.queryByRole("complementary", { name: "Connect to a device" });
const queue = () => screen.queryByRole("complementary", { name: "Queue" });
const activity = () => screen.queryByRole("complementary", { name: "Listening activity" });
const nowPlaying = () => screen.queryByRole("complementary", { name: "Now playing" });
const bar = () => document.querySelector<HTMLElement>(".player-bar")!;
const button = (name: string) => bar().querySelector<HTMLElement>(`button[aria-label="${name}"]`)!;
const deviceButton = () => button("Connect to a device");
const queueButton = () => button("Queue");
const coverButton = () => button("Now playing view");
const lyricsButton = () => button("Lyrics");
const esc = () => act(() => void fireEvent.keyDown(window, { key: "Escape" }));

function openLyrics() {
  act(() => fullWindow.open("lyrics"));
  expect(view()).toHaveClass("fw--lyrics");
}

beforeEach(() => {
  clearLyricsCache();
  clearNowPlayingCache();
  localStorage.clear();
  os.on = false;
  setOsFullscreenImpl(os);
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1400 });
  useFullWindow.setState({ open: false, mode: "artwork", osFullscreen: false, prefs: { ...FULL_WINDOW_PREFS_DEFAULT } });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, outputOpen: false, outputReturn: null, activityOpen: false, nowPlayingPrefs: { ...NOW_PLAYING_PREFS_DEFAULT }, menu: null });
  useRouter.getState().reset();
  play(synced);
});
afterEach(() => {
  vi.restoreAllMocks();
  act(() => void useFullWindow.setState({ open: false }));
});

describe("a right-panel view over the expanded view", () => {
  it("the device button opens the picker beside lyrics, pressed, and the view stays in lyrics", () => {
    render(<AppShell />);
    openLyrics();
    fireEvent.click(deviceButton());
    expect(devices()).toBeInTheDocument();
    expect(deviceButton()).toHaveAttribute("aria-pressed", "true");
    expect(view()).toHaveClass("fw--lyrics");
    expect(beside()).toBe(true);
    // The view's own controls still work.
    fireEvent.click(screen.getByRole("group", { name: "Show" }).querySelector<HTMLElement>('[aria-label="Artwork"]')!);
    expect(useFullWindow.getState().mode).toBe("artwork");
    expect(devices()).toBeInTheDocument();
  });

  it("the queue button opens the queue beside the view, in any mode", () => {
    render(<AppShell />);
    act(() => fullWindow.open("artwork"));
    fireEvent.click(queueButton());
    expect(queue()).toBeInTheDocument();
    expect(queueButton()).toHaveAttribute("aria-pressed", "true");
    expect(view()).toBeInTheDocument();
    expect(beside()).toBe(true);
  });

  it("friend activity opens beside the view", () => {
    render(<AppShell />);
    openLyrics();
    act(() => useUi.getState().toggleActivity());
    expect(activity()).toBeInTheDocument();
    expect(view()).toBeInTheDocument();
    expect(beside()).toBe(true);
  });

  it("the same button closes its panel and the view takes the full width back, not the now-playing panel", () => {
    render(<AppShell />);
    // The now-playing panel was open before the view and the picker.
    act(() => useUi.getState().setNowPlayingOpen(true));
    openLyrics();
    fireEvent.click(deviceButton());
    fireEvent.click(deviceButton());
    expect(devices()).toBeNull();
    expect(deviceButton()).toHaveAttribute("aria-pressed", "false");
    expect(view()).toHaveClass("fw--lyrics");
    expect(beside()).toBe(false);
    expect(useUi.getState().nowPlayingOpen).toBe(false);

    fireEvent.click(queueButton());
    fireEvent.click(queueButton());
    expect(queue()).toBeNull();
    expect(view()).toBeInTheDocument();
    expect(beside()).toBe(false);
  });

  it("one panel at a time: another replaces it", () => {
    render(<AppShell />);
    openLyrics();
    fireEvent.click(deviceButton());
    fireEvent.click(queueButton());
    expect(devices()).toBeNull();
    expect(queue()).toBeInTheDocument();
    expect(deviceButton()).toHaveAttribute("aria-pressed", "false");
    act(() => useUi.getState().toggleActivity());
    expect(queue()).toBeNull();
    expect(activity()).toBeInTheDocument();
    fireEvent.click(deviceButton());
    expect(activity()).toBeNull();
    expect(devices()).toBeInTheDocument();
    expect(beside()).toBe(true);
  });

  it("the cover button closes the view and shows the now-playing panel instead", async () => {
    render(<AppShell />);
    openLyrics();
    fireEvent.click(deviceButton());
    expect(coverButton()).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(coverButton(), { detail: 1 });
    await waitFor(() => expect(view()).toBeNull());
    expect(nowPlaying()).toBeInTheDocument();
    expect(devices()).toBeNull();
    expect(coverButton()).toHaveAttribute("aria-pressed", "true");
  });

  it("the now-playing panel left under the view isn't pressed, and the cover button still leaves the view for it", async () => {
    render(<AppShell />);
    act(() => useUi.getState().setNowPlayingOpen(true));
    openLyrics();
    expect(beside()).toBe(false);
    expect(coverButton()).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(coverButton(), { detail: 1 });
    await waitFor(() => expect(view()).toBeNull());
    expect(nowPlaying()).toBeInTheDocument();
  });

  it("the lyrics button switches another mode to lyrics, and closes the view from lyrics, the panel kept", async () => {
    render(<AppShell />);
    act(() => fullWindow.open("artwork"));
    fireEvent.click(deviceButton());
    fireEvent.click(lyricsButton());
    expect(view()).toHaveClass("fw--lyrics");
    expect(devices()).toBeInTheDocument();
    fireEvent.click(lyricsButton());
    await waitFor(() => expect(view()).toBeNull());
    expect(devices()).toBeInTheDocument();
  });
});

describe("leaving the view", () => {
  it("Esc closes the view and keeps the panel; a second Esc does nothing more", async () => {
    render(<AppShell />);
    openLyrics();
    fireEvent.click(deviceButton());
    esc();
    await waitFor(() => expect(view()).toBeNull());
    expect(devices()).toBeInTheDocument();
    expect(beside()).toBe(false);
    esc();
    expect(view()).toBeNull();
    expect(devices()).toBeInTheDocument();
    expect(useFullWindow.getState().open).toBe(false);
  });

  it("Esc from inside the panel leaves the view too; other keys stay the panel's", async () => {
    render(<AppShell />);
    openLyrics();
    fireEvent.click(queueButton());
    const vol = usePlayer.getState().state!.volume;
    act(() => void fireEvent.keyDown(queue()!, { key: "ArrowDown" }));
    expect(usePlayer.getState().state!.volume).toBe(vol);
    act(() => void fireEvent.keyDown(queue()!, { key: "Escape" }));
    await waitFor(() => expect(view()).toBeNull());
    expect(queue()).toBeInTheDocument();
  });

  it("Minimize closes the view and keeps the panel", async () => {
    render(<AppShell />);
    openLyrics();
    act(() => useUi.getState().toggleActivity());
    fireEvent.click(screen.getByRole("button", { name: "Minimize expanded view" }));
    await waitFor(() => expect(view()).toBeNull());
    expect(activity()).toBeInTheDocument();
  });

  it("reopening lyrics with a panel open puts the view back beside it", () => {
    render(<AppShell />);
    fireEvent.click(deviceButton());
    expect(beside()).toBe(false);
    fireEvent.click(lyricsButton());
    expect(view()).toHaveClass("fw--lyrics");
    expect(devices()).toBeInTheDocument();
    expect(beside()).toBe(true);
  });

  it("going to a page, home again, or back closes the view and keeps the panel", async () => {
    render(<AppShell />);
    openLyrics();
    fireEvent.click(queueButton());
    act(() => useRouter.getState().navigate({ name: "album", id: synced.albumId }));
    await waitFor(() => expect(view()).toBeNull());
    expect(queue()).toBeInTheDocument();

    openLyrics();
    act(() => useRouter.getState().back());
    await waitFor(() => expect(view()).toBeNull());

    // Already home: the page doesn't change, the view still steps aside.
    openLyrics();
    act(() => useRouter.getState().navigate({ name: "home" }));
    await waitFor(() => expect(view()).toBeNull());
    expect(queue()).toBeInTheDocument();
  });

  it("the faded controls leave the panel alone", () => {
    vi.useFakeTimers();
    try {
      render(<AppShell />);
      openLyrics();
      fireEvent.click(deviceButton());
      act(() => void vi.advanceTimersByTime(5_000));
      expect(view()).toHaveClass("is-idle");
      expect(devices()).toBeInTheDocument();
      expect(devices()!.closest(".fw")).toBeNull();
      expect(beside()).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });
});
