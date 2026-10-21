/**
 * The expanded now-playing view: ways in and out (including OS full screen
 * and its Esc order), the header fading, keys, and its modes (artwork, the
 * artist's image, lyrics).
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { CONTROLS_HIDE_MS, FullWindow } from "../shell/FullWindow";
import { clearLyricsCache } from "../shell/lyrics";
import { UNSYNCED_NOTE } from "../shell/LyricsView";
import { clearNowPlayingCache } from "../shell/nowPlaying";
import { PlayerBar } from "../shell/PlayerBar";
import { VOLUME_KEY_STEP } from "../shell/useShortcuts";
import { FULL_WINDOW_PREFS_DEFAULT, fullWindow, setOsFullscreenImpl, useFullWindow } from "../stores/fullWindow";
import { player, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const byKind = (k: ReturnType<typeof mockLyricsKind>) => mock.fx.tracks.find((t) => mockLyricsKind(t.id) === k && t.durationMs > 120_000)!;
const synced = byKind("words");
const plain = byKind("plain");
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

const view = () => screen.queryByRole("dialog", { name: "Expanded now playing" });
const key = (k: string, target: Element | Window = window) => act(() => void fireEvent.keyDown(target, { key: k }));
const toggle = (name: string) => screen.getByRole("group", { name: "Show" }).querySelector<HTMLElement>(`[aria-label="${name}"]`)!;

beforeEach(() => {
  clearLyricsCache();
  clearNowPlayingCache();
  localStorage.clear();
  os.on = false;
  os.set.mockClear();
  setOsFullscreenImpl(os);
  useFullWindow.setState({ open: false, mode: "artwork", osFullscreen: false, prefs: { ...FULL_WINDOW_PREFS_DEFAULT } });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useUi.setState({ nowPlayingOpen: false, queueOpen: false, menu: null });
  useRouter.getState().reset();
  play(synced);
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("opening and closing", () => {
  it("opens from the expand button at the far right of the player bar, which closes it again", async () => {
    render(
      <>
        <PlayerBar />
        <FullWindow />
      </>,
    );
    const extras = document.querySelector(".player-bar__extras")!;
    // The More button stands in for the last ones only on a narrow window.
    const buttons = Array.from(extras.querySelectorAll("button")).filter((b) => !b.closest(".player-bar__more"));
    const expand = screen.getByRole("button", { name: "Expanded view" });
    expect(buttons[buttons.length - 1]).toBe(expand);
    fireEvent.click(expand);
    expect(view()).toBeInTheDocument();
    expect(view()).toHaveFocus();
    expect(view()).toHaveClass("fw--artwork");
    expect(expand).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(expand);
    await waitFor(() => expect(view()).toBeNull());
  });

  it("leaves the player bar in view", () => {
    render(
      <>
        <PlayerBar />
        <FullWindow />
      </>,
    );
    act(() => fullWindow.open());
    expect(view()!.contains(document.querySelector(".player-bar"))).toBe(false);
    expect(view()!.querySelector("[role=slider]")).toBeNull();
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

  it("closes with Esc, Minimize, or a double-click on the background", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    key("Escape");
    await waitFor(() => expect(view()).toBeNull());

    act(() => fullWindow.open());
    fireEvent.click(screen.getByRole("button", { name: "Minimize expanded view" }));
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
    fireEvent.click(screen.getByRole("button", { name: "Minimize expanded view" }));
    await waitFor(() => expect(view()).toBeNull());
    expect(os.on).toBe(false);
  });

  it("marks the window while open, so the bars sit over it, and clears that on close", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    expect(document.documentElement).toHaveClass("fw-open");
    key("Escape");
    await waitFor(() => expect(view()).toBeNull());
    expect(document.documentElement).not.toHaveClass("fw-open");
    expect(document.documentElement).not.toHaveClass("fw-idle");
  });

  it("the header names what's playing from, linking back to it", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    const ctx = document.querySelector(".fw__context")!;
    expect(ctx).toHaveTextContent(synced.albumTitle);
    fireEvent.click(within(ctx as HTMLElement).getByText(synced.albumTitle));
    await waitFor(() => expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "album", id: synced.albumId }));
    await waitFor(() => expect(view()).toBeNull());
  });

  it("More options opens the song's menu", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    fireEvent.click(screen.getByRole("button", { name: "More options" }));
    expect(useUi.getState().menu?.label).toBe(`Options for ${synced.title}`);
  });

  it("steps aside when a link goes to a page", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    act(() => useRouter.getState().navigate({ name: "album", id: synced.albumId }));
    await waitFor(() => expect(view()).toBeNull());
  });
});

describe("header fade", () => {
  it("after about 4 s without movement, and comes back on movement or a key", () => {
    expect(CONTROLS_HIDE_MS).toBe(4_000);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    render(<FullWindow />);
    act(() => fullWindow.open());
    const root = view()!;
    expect(root).not.toHaveClass("is-idle");
    act(() => vi.advanceTimersByTime(CONTROLS_HIDE_MS - 1));
    expect(root).not.toHaveClass("is-idle");
    act(() => vi.advanceTimersByTime(1));
    expect(root).toHaveClass("is-idle");
    // The player bar and the top bar fade with it.
    expect(document.documentElement).toHaveClass("fw-idle");
    act(() => void fireEvent.mouseMove(window));
    expect(root).not.toHaveClass("is-idle");
    expect(document.documentElement).not.toHaveClass("fw-idle");
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
    const playToggle = vi.spyOn(backend, "playToggle").mockResolvedValue();
    const seek = vi.spyOn(backend, "playSeek").mockResolvedValue();
    render(<FullWindow />);
    act(() => fullWindow.open());
    key(" ", view()!);
    expect(playToggle).toHaveBeenCalledTimes(1);
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

  it("L works like the lyrics button and F toggles OS full screen", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    key("l");
    expect(useFullWindow.getState().mode).toBe("lyrics");
    await waitFor(() => expect(document.querySelector(".fw__lyrics .lyrics")).not.toBeNull());
    key("f");
    await waitFor(() => expect(useFullWindow.getState().osFullscreen).toBe(true));
    key("F");
    await waitFor(() => expect(useFullWindow.getState().osFullscreen).toBe(false));
    key("L");
    await waitFor(() => expect(view()).toBeNull());
  });
});

describe("modes", () => {
  it("the header toggles pick artwork, the artist's image or lyrics, the active one pressed", async () => {
    render(<FullWindow />);
    act(() => fullWindow.open());
    expect(toggle("Artwork")).toHaveAttribute("aria-pressed", "true");
    expect(document.querySelector(".fw__now .turntable")).not.toBeNull();

    await waitFor(() => expect(toggle("Artist image")).not.toHaveAttribute("aria-disabled"));
    fireEvent.click(toggle("Artist image"));
    expect(view()).toHaveClass("fw--artist");
    expect(toggle("Artist image")).toHaveAttribute("aria-pressed", "true");
    expect(toggle("Artwork")).toHaveAttribute("aria-pressed", "false");
    expect(document.querySelector(".fw__artist-photo")).not.toBeNull();
    expect(screen.getByText(synced.artistTitle, { selector: ".fw__artist-name, .fw__artist-name *" })).toBeInTheDocument();

    fireEvent.click(toggle("Lyrics"));
    expect(view()).toHaveClass("fw--lyrics");
    await waitFor(() => expect(screen.getByRole("button", { current: true })).toBeInTheDocument());
  });

  it("an artist with no image: the toggle is off and the artwork shows", async () => {
    vi.spyOn(backend, "nowPlayingDetails").mockResolvedValue({ trackId: synced.id, artist: { id: synced.artistId, title: synced.artistTitle }, credits: [] });
    act(() => useFullWindow.setState({ prefs: { remember: true, mode: "artist" } }));
    render(<FullWindow />);
    act(() => fullWindow.open());
    await waitFor(() => expect(toggle("Artist image")).toHaveAttribute("aria-pressed", "true"));
    expect(view()).toHaveClass("fw--artwork");
    expect(document.querySelector(".fw__now .turntable")).not.toBeNull();
  });

  it("synced lyrics follow playback and seek on a click", async () => {
    const seek = vi.spyOn(backend, "playSeek").mockResolvedValue();
    render(<FullWindow />);
    act(() => fullWindow.open("lyrics"));
    const current = await screen.findByRole("button", { current: true });
    expect(current).toHaveClass("is-current");
    expect(screen.queryByText(UNSYNCED_NOTE)).toBeNull();
    const lines = document.querySelectorAll<HTMLElement>(".lyrics__line");
    fireEvent.click(lines[2]);
    expect(seek).toHaveBeenCalled();
  });

  it("unsynced lyrics carry the note and one tint", async () => {
    play(plain);
    render(<FullWindow />);
    act(() => fullWindow.open("lyrics"));
    expect(await screen.findByText(UNSYNCED_NOTE)).toBeInTheDocument();
    expect(document.querySelector(".lyrics--plain")).not.toBeNull();
    expect(document.querySelector(".lyrics__line.is-current, [aria-current]")).toBeNull();
  });

  it("a song without lyrics: the toggle is off, and lyrics mode offers Write your own", async () => {
    play(none);
    render(<FullWindow />);
    act(() => fullWindow.open());
    await waitFor(() => expect(toggle("Lyrics")).toHaveAttribute("aria-disabled", "true"));
    expect(screen.getByText(none.title)).toBeInTheDocument();
    act(() => fullWindow.setMode("lyrics"));
    expect(await screen.findByText("No lyrics for this song")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Write your own" })).toBeInTheDocument();
    // Live while lyrics show, so it isn't a dead end.
    expect(toggle("Lyrics")).not.toHaveAttribute("aria-disabled");
  });
});
