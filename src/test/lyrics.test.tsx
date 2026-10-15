/**
 * The lyrics view: line and word tracking, seeking from a line, the
 * auto-follow pause, plain lyrics, no lyrics, the player bar button and
 * auto-show.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyrics, mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type Lyrics, type PlayerState, type Track } from "../lib/types";
import { BORROWED_NOTE, LyricsPane, UNSYNCED_NOTE } from "../shell/LyricsView";
import { SettingsPage, SOURCE_NOTICES } from "../pages/SettingsPage";
import {
  clearLyricsCache,
  currentLineIndex,
  currentWordIndex,
  followTarget,
  interpolate,
  lineState,
  loadLyrics,
  timedLines,
  useLyricsAutoShow,
} from "../shell/lyrics";
import { PlayerBar } from "../shell/PlayerBar";
import { FULL_WINDOW_PREFS_DEFAULT, fullWindow, loadFullWindowPrefs, useFullWindow } from "../stores/fullWindow";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const byKind = (k: ReturnType<typeof mockLyricsKind>) => mock.fx.tracks.find((t) => mockLyricsKind(t.id) === k && t.durationMs > 120_000)!;
const words = byKind("words");
const lines = byKind("lines");
const plain = byKind("plain");
const none = byKind("none");

function state(track: Track | null, status: PlayerState["status"] = "paused"): PlayerState {
  return {
    status,
    track,
    positionMs: 0,
    durationMs: track?.durationMs ?? 0,
    volume: 0.7,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: null, userQueue: [], nextUp: [], history: [], context: null },
  };
}

function at(track: Track, positionMs: number, status: PlayerState["status"] = "paused") {
  act(() => usePlayer.setState({ state: state(track, status), positionMs, durationMs: track.durationMs }));
}

const route = () => {
  const r = useRouter.getState();
  return r.entries[r.index].route;
};

beforeEach(() => {
  clearLyricsCache();
  mock.lyricsDown = false;
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useRouter.getState().reset();
  useUi.setState({ nowPlayingOpen: false, queueOpen: false });
  useFullWindow.setState({ open: false, mode: "artwork", osFullscreen: false, prefs: { ...FULL_WINDOW_PREFS_DEFAULT } });
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

// ---------------------------------------------------------------- rules

describe("timing rules", () => {
  const ls = [{ startMs: 1000 }, { startMs: 2000 }, { startMs: 2000 }, { startMs: 5000 }];

  it("finds the line playing: the last one started", () => {
    expect(currentLineIndex(ls, 0)).toBe(-1);
    expect(currentLineIndex(ls, 1000)).toBe(0);
    expect(currentLineIndex(ls, 1999)).toBe(0);
    expect(currentLineIndex(ls, 2000)).toBe(2);
    expect(currentLineIndex(ls, 99_000)).toBe(3);
    expect(currentLineIndex([], 5)).toBe(-1);
  });

  it("finds the word playing", () => {
    const w = [
      { startMs: 100, text: "a " },
      { startMs: 300, text: "b" },
    ];
    expect(currentWordIndex(w, 50)).toBe(-1);
    expect(currentWordIndex(w, 200)).toBe(0);
    expect(currentWordIndex(w, 300)).toBe(1);
    expect(currentWordIndex(undefined, 300)).toBe(-1);
  });

  it("names past, current and future lines", () => {
    expect([0, 1, 2].map((i) => lineState(i, 1))).toEqual(["past", "current", "future"]);
    expect(lineState(0, -1)).toBe("future");
  });

  it("puts the current line a little above the middle, inside the scroll range", () => {
    expect(followTarget(1000, 60, 900, 5000)).toBeCloseTo(1000 + 30 - 900 * 0.42);
    expect(followTarget(100, 60, 900, 5000)).toBe(0);
    expect(followTarget(4900, 60, 900, 5000)).toBe(4100);
  });

  it("moves the position on between updates only while playing", () => {
    const base = { positionMs: 10_000, at: 1_000 };
    expect(interpolate(base, 1_200, true, 60_000)).toBe(10_200);
    expect(interpolate(base, 1_200, false, 60_000)).toBe(10_000);
    expect(interpolate(base, 99_000, true, 60_000)).toBe(60_000);
  });

  it("uses timed lines only for synced lyrics", () => {
    const l: Lyrics = { synced: true, lines: ["a"], timed: [{ startMs: 0, text: "a" }] };
    expect(timedLines(l)).toHaveLength(1);
    expect(timedLines({ synced: false, lines: ["a"] })).toBeNull();
    expect(timedLines(null)).toBeNull();
  });
});

describe("mock lyrics", () => {
  it("has a song with word timing, one timed by line, one plain and one with none", () => {
    const w = mockLyrics(words)!;
    expect(w.synced && w.timed!.some((l) => l.words && l.words.length > 1)).toBe(true);
    expect(w.timed!.every((l) => !l.words || l.words.map((x) => x.text).join("") === l.text)).toBe(true);
    const l = mockLyrics(lines)!;
    expect(l.synced && l.timed!.every((x) => !x.words)).toBe(true);
    expect(mockLyrics(plain)!.synced).toBe(false);
    expect(mockLyrics(none)).toBeNull();
  });
});

// ---------------------------------------------------------------- view

describe("synced lyrics", () => {
  const timed = () => mockLyrics(words)!.timed!;

  it("marks the current line and the sung words, and follows playback", async () => {
    const t = timed();
    const line = t.findIndex((l, i) => i > 0 && (l.words?.length ?? 0) > 3);
    at(words, t[line].words![1].startMs + 10);
    render(<LyricsPane />);
    const current = await screen.findByRole("button", { current: true });
    expect(current).toHaveAccessibleName(t[line].text);
    const sung = current.querySelectorAll(".lyrics__word.is-sung");
    expect(sung).toHaveLength(2);
    expect(screen.getAllByRole("button", { name: t[line - 1].text || "Instrumental" })[0]).toHaveClass(/is-(past|current)/);
    // Playback moves on: the next line takes over.
    at(words, t[line + 1].startMs + 5);
    await waitFor(() => expect(screen.getByRole("button", { current: true })).toHaveClass("lyrics__line"));
    expect(screen.getByRole("button", { current: true })).not.toBe(current);
    expect(current).toHaveClass("is-past");
  });

  it("seeks to a line when it's clicked", async () => {
    const t = timed();
    at(words, 0);
    const seek = vi.spyOn(backend, "playSeek");
    render(<LyricsPane />);
    const target = t.findIndex((l, i) => i > 3 && l.text);
    await screen.findAllByRole("button", { name: t[target].text });
    const btn = document.querySelectorAll<HTMLElement>(".lyrics__line")[target];
    expect(btn).toHaveAccessibleName(t[target].text);
    fireEvent.click(btn);
    expect(seek).toHaveBeenCalledWith(t[target].startMs);
  });

  it("stops following on a hand scroll and stays put until Sync", async () => {
    at(words, 30_000);
    render(<LyricsPane />);
    const scroller = await screen.findByLabelText("Lyrics", { selector: ".lyrics__scroll" });
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    fireEvent.wheel(scroller, { deltaY: 400 });
    const sync = screen.getByRole("button", { name: /^Sync/ });
    expect(sync).toHaveTextContent("Sync");
    expect(sync).toHaveClass("lyrics__sync");
    // No timer brings following back: the pill stays.
    act(() => vi.advanceTimersByTime(60_000));
    expect(screen.getByRole("button", { name: /^Sync/ })).toBeInTheDocument();
    // The highlight keeps moving while scrolled away.
    act(() => usePlayer.setState({ positionMs: 90_000 }));
    // Sync resumes at once.
    fireEvent.click(screen.getByRole("button", { name: /^Sync/ }));
    expect(screen.queryByRole("button", { name: /^Sync/ })).toBeNull();
  });

  it("shows the credit line under the lyrics", async () => {
    at(words, 0);
    render(<LyricsPane />);
    expect(await screen.findByText(/Lyrics licensed and provided by LyricFind/)).toBeInTheDocument();
  });
});

describe("plain and missing lyrics", () => {
  it("shows unsynced lyrics as text in one tint, with a note, no highlighting and no seeking", async () => {
    at(plain, 40_000);
    const seek = vi.spyOn(backend, "playSeek");
    render(<LyricsPane />);
    const view = await screen.findByLabelText("Lyrics", { selector: ".lyrics__scroll" });
    const note = within(view).getByText(UNSYNCED_NOTE);
    expect(note).toBe(view.querySelector(".lyrics__lines")!.firstElementChild);
    // The only button is Share lyrics (lines don't seek).
    expect(within(view).queryAllByRole("button").map((b) => b.textContent?.trim())).toEqual(["Share lyrics"]);
    const ls = view.querySelectorAll("p.lyrics__line");
    expect(ls.length).toBeGreaterThan(5);
    expect([...ls].every((l) => !/is-(past|current|future)/.test(l.className))).toBe(true);
    expect(view.querySelector("[aria-current]")).toBeNull();
    fireEvent.click(ls[3]);
    expect(seek).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /^Sync/ })).toBeNull();
  });

  it("synced lyrics have no unsynced note", async () => {
    at(words, 0);
    render(<LyricsPane />);
    await screen.findByLabelText("Lyrics", { selector: ".lyrics__scroll" });
    expect(screen.queryByText(UNSYNCED_NOTE)).toBeNull();
  });

  const liveTrack = mock.fx.tracks.find(
    (t) => mockLyricsKind(t.id) === "words" && t.durationMs > 120_000 && !!mock.fx.albums.find((a) => a.id === t.albumId)?.subformats.includes("live"),
  )!;

  it("a live version with timing from another version shows it unsynced, with a note", async () => {
    at(liveTrack, 40_000);
    const seek = vi.spyOn(backend, "playSeek");
    render(<LyricsPane />);
    const view = await screen.findByLabelText("Lyrics", { selector: ".lyrics__scroll" });
    const note = within(view).getByText(BORROWED_NOTE);
    expect(note).toBe(view.querySelector(".lyrics__lines")!.firstElementChild);
    expect(within(view).queryByText(UNSYNCED_NOTE)).toBeNull();
    expect(view.querySelector("[aria-current]")).toBeNull();
    fireEvent.click(view.querySelectorAll("p.lyrics__line")[2]);
    expect(seek).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /^Sync/ })).toBeNull();
  });

  it("with borrowed timing turned on, a live version follows along and still says where the timing is from", async () => {
    await mock.settingsSet({ lyricsLiveTiming: true });
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, lyricsLiveTiming: true } }));
    at(liveTrack, 40_000);
    render(<LyricsPane />);
    const view = await screen.findByLabelText("Lyrics", { selector: ".lyrics__scroll" });
    await waitFor(() => expect(view.querySelector("[aria-current]")).not.toBeNull());
    expect(within(view).getByText(BORROWED_NOTE)).toHaveClass("lyrics__borrowed");
    await mock.settingsSet({ lyricsLiveTiming: false });
  });

  it("says so when the song has none", async () => {
    at(none, 0);
    render(<LyricsPane />);
    expect(await screen.findByText("No lyrics for this song")).toBeInTheDocument();
  });

  it("lyrics source down: silent, the view says there are none", async () => {
    mock.lyricsDown = true;
    at(words, 0);
    render(<LyricsPane />);
    expect(await screen.findByText("No lyrics for this song")).toBeInTheDocument();
  });

  it("a failed fetch is none, and isn't remembered", async () => {
    const spy = vi.spyOn(backend, "lyrics").mockRejectedValueOnce({ kind: "network", message: "down" });
    expect(await loadLyrics(words.id)).toBeNull();
    expect(await loadLyrics(words.id)).not.toBeNull();
    expect(spy).toHaveBeenCalledTimes(2);
  });
});

// ---------------------------------------------------------------- button and auto-show

describe("player bar lyrics button", () => {
  it("opens the expanded view in lyrics mode and leaves it", async () => {
    useRouter.getState().navigate({ name: "liked" });
    at(words, 0);
    render(<PlayerBar />);
    const btn = screen.getByRole("button", { name: "Lyrics" });
    await waitFor(() => expect(btn).not.toHaveAttribute("aria-disabled"));
    fireEvent.click(btn);
    expect(useFullWindow.getState()).toMatchObject({ open: true, mode: "lyrics" });
    expect(route()).toEqual({ name: "liked" });
    expect(btn).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(btn);
    await waitFor(() => expect(useFullWindow.getState().open).toBe(false));
    expect(btn).toHaveAttribute("aria-pressed", "false");
  });

  it("switches an open view to lyrics instead of closing it", async () => {
    at(words, 0);
    render(<PlayerBar />);
    act(() => fullWindow.open("artwork"));
    const btn = screen.getByRole("button", { name: "Lyrics" });
    expect(btn).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(btn);
    expect(useFullWindow.getState()).toMatchObject({ open: true, mode: "lyrics" });
  });

  it("is off for a song without lyrics, but stays live while lyrics show", async () => {
    at(none, 0);
    render(<PlayerBar />);
    const btn = screen.getByRole("button", { name: "Lyrics" });
    await waitFor(() => expect(btn).toHaveAttribute("aria-disabled", "true"));
    fireEvent.click(btn);
    expect(useFullWindow.getState().open).toBe(false);
    act(() => fullWindow.open("lyrics"));
    expect(btn).not.toHaveAttribute("aria-disabled");
  });
});

describe("the old lyrics route", () => {
  it("opens the expanded view in lyrics mode instead of a page", () => {
    useRouter.getState().navigate({ name: "liked" });
    act(() => useRouter.getState().navigate({ name: "lyrics" }));
    expect(useFullWindow.getState()).toMatchObject({ open: true, mode: "lyrics" });
    expect(route()).toEqual({ name: "liked" });
    expect(useRouter.getState().entries).toHaveLength(2);
    act(() => void fullWindow.close());
    act(() => useRouter.getState().reset({ name: "lyrics" }));
    expect(useFullWindow.getState()).toMatchObject({ open: true, mode: "lyrics" });
    expect(route()).toEqual({ name: "home" });
  });
});

describe("auto-show", () => {
  function Host() {
    useLyricsAutoShow();
    return null;
  }

  async function startSong(track: Track) {
    at(none, 0, "playing");
    render(<Host />);
    at(track, 0, "playing");
    // Let the lyrics fetch settle.
    await act(() => loadLyrics(track.id));
    await act(() => new Promise((r) => setTimeout(r, 0)));
  }

  const fw = () => useFullWindow.getState();

  it("opens the expanded view in lyrics mode for a synced song while now playing is open", async () => {
    useUi.getState().setNowPlayingOpen(true);
    await startSong(words);
    expect(fw()).toMatchObject({ open: true, mode: "lyrics" });
    expect(route()).toEqual({ name: "home" });
  });

  it("leaves an open view on the mode the user picked", async () => {
    useUi.getState().setNowPlayingOpen(true);
    act(() => fullWindow.open("artist"));
    await startSong(words);
    expect(fw()).toMatchObject({ open: true, mode: "artist" });
  });

  it("not for plain lyrics", async () => {
    useUi.getState().setNowPlayingOpen(true);
    await startSong(plain);
    expect(fw().open).toBe(false);
  });

  it("not with now playing closed", async () => {
    await startSong(lines);
    expect(fw().open).toBe(false);
  });

  it("never when the setting is Off", async () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, lyricsMode: "off" } });
    useUi.getState().setNowPlayingOpen(true);
    await startSong(words);
    expect(fw().open).toBe(false);
  });
});

// ---------------------------------------------------------------- settings

describe("lyrics settings", () => {
  it("list Lyrics, LRCLIB and the expanded view's fade with their defaults, and save changes", async () => {
    const user = userEvent.setup();
    await backend.settingsSet({ ...DEFAULT_SETTINGS });
    useSettings.setState({ settings: null });
    await loadSettings();
    const set = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    const mode = screen.getByRole("combobox", { name: "Lyrics" });
    expect(mode).toHaveValue("autoShow");
    const lrclib = screen.getByRole("switch", { name: "Lyrics from LRCLIB when Plex has none" });
    expect(lrclib).not.toBeChecked();
    const fade = screen.getByRole("switch", { name: "Expanded view hides controls" });
    expect(fade).toBeChecked();
    await user.selectOptions(mode, "off");
    expect(set).toHaveBeenCalledWith({ lyricsMode: "off" });
    await user.click(lrclib);
    expect(set).toHaveBeenCalledWith({ lyricsLrclib: true });
    await user.click(fade);
    expect(set).toHaveBeenCalledWith({ fullWindowHideControls: false });
    expect(SOURCE_NOTICES.some((n) => n.name.startsWith("LRCLIB"))).toBe(true);
    await backend.settingsSet({ ...DEFAULT_SETTINGS });
  });

  it("Remember the expanded view's mode: on by default, after the fade, this device only", async () => {
    const user = userEvent.setup();
    localStorage.clear();
    await loadSettings();
    const set = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    const remember = screen.getByRole("switch", { name: "Remember the expanded view's mode" });
    expect(remember).toBeChecked();
    const rows = [...document.querySelectorAll("[data-setting]")].map((r) => r.getAttribute("data-setting"));
    expect(rows.indexOf("fullWindowRememberMode")).toBe(rows.indexOf("fullWindowHideControls") + 1);
    await user.click(remember);
    expect(remember).not.toBeChecked();
    expect(useFullWindow.getState().prefs.remember).toBe(false);
    expect(JSON.parse(localStorage.getItem("platter.expandedView.v1")!)).toMatchObject({ remember: false });
    expect(set).not.toHaveBeenCalled();
    localStorage.clear();
  });
});

describe("expanded view mode", () => {
  it("the expand button reopens the mode last used, artwork the first time", () => {
    localStorage.clear();
    fullWindow.toggle();
    expect(useFullWindow.getState()).toMatchObject({ open: true, mode: "artwork" });
    fullWindow.setMode("artist");
    act(() => void fullWindow.close());
    fullWindow.toggle();
    expect(useFullWindow.getState().mode).toBe("artist");
    fullWindow.toggleLyrics();
    act(() => void fullWindow.close());
    fullWindow.toggle();
    expect(useFullWindow.getState().mode).toBe("lyrics");
    // Kept across launches on this device.
    expect(loadFullWindowPrefs()).toEqual({ remember: true, mode: "lyrics" });
    localStorage.clear();
  });

  it("with Remember off, the expand button always starts on the artwork", () => {
    localStorage.clear();
    fullWindow.setRemember(false);
    fullWindow.open("lyrics");
    act(() => void fullWindow.close());
    fullWindow.toggle();
    expect(useFullWindow.getState().mode).toBe("artwork");
    expect(loadFullWindowPrefs().remember).toBe(false);
    localStorage.clear();
  });

  it("Show borrowed timing on live versions: off by default, saved when turned on", async () => {
    const user = userEvent.setup();
    await backend.settingsSet({ ...DEFAULT_SETTINGS });
    useSettings.setState({ settings: null });
    await loadSettings();
    const set = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    const sw = screen.getByRole("switch", { name: "Show borrowed timing on live versions" });
    expect(sw).not.toBeChecked();
    await user.click(sw);
    expect(set).toHaveBeenCalledWith({ lyricsLiveTiming: true });
    await backend.settingsSet({ ...DEFAULT_SETTINGS });
  });

  it("turning borrowed timing on asks for lyrics again", async () => {
    const spy = vi.spyOn(backend, "lyrics");
    await loadLyrics(words.id);
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, lyricsLiveTiming: true } }));
    await loadLyrics(words.id);
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it("turning LRCLIB on asks for lyrics again", async () => {
    const spy = vi.spyOn(backend, "lyrics");
    await loadLyrics(none.id);
    await loadLyrics(none.id);
    expect(spy).toHaveBeenCalledTimes(1);
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, lyricsLrclib: true } }));
    await loadLyrics(none.id);
    expect(spy).toHaveBeenCalledTimes(2);
  });
});
