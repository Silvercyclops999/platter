import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { NowPlayingDetails, PlayerState, QueueItem, Track } from "../lib/types";
import { AppShell } from "../shell/AppShell";
import {
  buildCredits,
  clearNowPlayingCache,
  contextRoute,
  creditsWorthShowing,
  lyricsPreview,
  nextInQueue,
} from "../shell/nowPlaying";
import { NowPlayingSettings } from "../shell/NowPlayingSettings";
import { demoHealth } from "../lib/mock/health";
import { useApp } from "../stores/app";
import { usePlayer } from "../stores/player";
import { FULL_WINDOW_PREFS_DEFAULT, useFullWindow } from "../stores/fullWindow";
import { useRouter } from "../stores/router";
import { loadNowPlayingPrefs, NOW_PLAYING_PREFS_DEFAULT, useUi } from "../stores/ui";

const mock = backend as MockBackend;
const tracks = mock.fx.tracks;
// A track with a plain artist line (only the album artist), so credits come only from details.
const solo = tracks.find((t) => !t.trackArtist)!;
const other = tracks.find((t) => t.id !== solo.id)!;
const third = tracks.find((t) => t.id !== solo.id && t.id !== other.id)!;

const item = (t: Track, i: number, source: "user" | "context" = "context"): QueueItem => ({ itemId: `q${i}`, track: t, source });

function playerState(track: Track | null, over: Partial<PlayerState["queue"]> = {}, status: PlayerState["status"] = "playing"): PlayerState {
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
    queue: {
      current: track ? item(track, 0) : null,
      userQueue: [],
      nextUp: [item(other, 1)],
      history: [],
      context: track ? { kind: "album", id: track.albumId, title: track.albumTitle } : null,
      ...over,
    },
  };
}

const EMPTY: NowPlayingDetails = { trackId: solo.id, artist: null, credits: [], lyrics: null };
const FULL: NowPlayingDetails = {
  trackId: solo.id,
  artist: { id: solo.artistId, title: solo.artistTitle, summary: "Formed in a small harbour town.\n\nSecond paragraph.", thumb: null, art: null },
  credits: [
    { name: solo.artistTitle, roles: ["composer"] },
    { name: "M. Hale", roles: ["writer", "lyricist"] },
  ],
  lyrics: { synced: true, lines: ["", "First line", "Second line", "", "Third line", "Fourth line", "Fifth line", "Sixth line"] },
};

function details(d: NowPlayingDetails) {
  return vi.spyOn(backend, "nowPlayingDetails").mockResolvedValue(d);
}

const panel = () => screen.queryByRole("complementary", { name: "Now playing" });
const artButton = () => screen.getByRole("button", { name: "Now playing view" });

beforeEach(() => {
  localStorage.clear();
  clearNowPlayingCache();
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, nowPlayingPrefs: { ...NOW_PLAYING_PREFS_DEFAULT }, menu: null });
  usePlayer.setState({ state: playerState(solo) });
});
afterEach(() => {
  vi.restoreAllMocks();
  useUi.setState({ queueOpen: false, nowPlayingOpen: false });
});

describe("opening and the shared right slot", () => {
  it("the player-bar art toggles the panel", async () => {
    details(EMPTY);
    render(<AppShell />);
    expect(panel()).toBeNull();
    fireEvent.click(artButton());
    expect(panel()).toBeInTheDocument();
    expect(artButton()).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("separator", { name: "Resize now playing view" })).toBeInTheDocument();
    fireEvent.click(artButton());
    expect(panel()).toBeNull();
    expect(useUi.getState().nowPlayingOpen).toBe(false);
  });

  it("shares the slot with the queue, one at a time", async () => {
    details(EMPTY);
    render(<AppShell />);
    fireEvent.click(artButton());
    fireEvent.click(screen.getByRole("button", { name: "Queue" }));
    expect(screen.getByRole("complementary", { name: "Queue" })).toBeInTheDocument();
    expect(panel()).toBeNull();
    // Swapping panels rises in place instead of sliding from the edge.
    expect(document.querySelector(".app")).toHaveClass("right-swap");
    fireEvent.click(artButton());
    expect(panel()).toBeInTheDocument();
    expect(screen.queryByRole("complementary", { name: "Queue" })).toBeNull();
    expect(useUi.getState()).toMatchObject({ queueOpen: false, nowPlayingOpen: true });
  });

  it("the hide button closes it", () => {
    details(EMPTY);
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    fireEvent.click(screen.getByRole("button", { name: "Hide now playing view" }));
    expect(panel()).toBeNull();
    expect(document.querySelector(".app")).not.toHaveClass("right-swap");
  });

  it("uses the same width and limits as the queue", () => {
    details(EMPTY);
    useUi.setState({ rightWidth: 400 });
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    expect((document.querySelector(".app") as HTMLElement).style.getPropertyValue("--right-w")).toBe("400px");
    const handle = screen.getByRole("separator", { name: "Resize now playing view" });
    fireEvent.keyDown(handle, { key: "End" });
    expect(useUi.getState().rightWidth).toBe(420);
  });
});

describe("panel content", () => {
  it("shows the context link, art, title and artist", async () => {
    details(EMPTY);
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    const p = panel()!;
    fireEvent.click(within(p).getByRole("link", { name: solo.albumTitle }));
    const r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "album", id: solo.albumId });
    expect(within(p).getByRole("link", { name: solo.title })).toBeInTheDocument();
    expect(within(p).getByRole("button", { name: `Go to album ${solo.albumTitle}` })).toBeInTheDocument();
    expect(within(p).getByRole("button", { name: `More options for ${solo.title}` })).toBeInTheDocument();
    // The like control follows the title.
    expect(within(p).getByRole("button", { name: `Add ${solo.title} to Liked Songs` })).toBeInTheDocument();
  });

  it("hides lyrics, about and credits when there's nothing to show", async () => {
    const spy = details(EMPTY);
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    await waitFor(() => expect(spy).toHaveBeenCalledWith(solo.id));
    await screen.findByText("Next in queue");
    expect(within(panel()!).queryByText("Lyrics")).toBeNull();
    expect(screen.queryByText("About the artist")).toBeNull();
    expect(screen.queryByText("Credits")).toBeNull();
  });

  it("shows lyrics, about and credits when Plex has them", async () => {
    details(FULL);
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    expect(await screen.findByText("First line")).toBeInTheDocument();
    // The preview is the first five lines with words.
    expect(screen.getByText("Fifth line")).toBeInTheDocument();
    expect(screen.queryByText("Sixth line")).toBeNull();
    expect(screen.getByText("About the artist")).toBeInTheDocument();
    expect(screen.getByText(/Formed in a small harbour town/)).toBeInTheDocument();
    expect(screen.getByText("Credits")).toBeInTheDocument();
    expect(screen.getByText("Main artist • Composer")).toBeInTheDocument();
    expect(screen.getByText("Writer • Lyricist")).toBeInTheDocument();
  });

  it("offline: sections that need the server step aside, and come back on reconnect", async () => {
    details(FULL);
    const server = { name: "Basement Server", version: "1.42.2" };
    act(() => useApp.setState({ health: demoHealth("offline", server) }));
    try {
      useUi.getState().setNowPlayingOpen(true);
      render(<AppShell />);
      expect(await screen.findByText("First line")).toBeInTheDocument();
      expect(screen.queryByText("About the artist")).toBeNull();
      expect(screen.queryByText("Writer • Lyricist")).toBeNull();
      act(() => useApp.setState({ health: demoHealth("local", server) }));
      expect(await screen.findByText("About the artist")).toBeInTheDocument();
      expect(screen.getByText("Writer • Lyricist")).toBeInTheDocument();
    } finally {
      act(() => useApp.setState({ health: null }));
    }
  });

  it("opens full lyrics and the full bio in the panel, with a way back", async () => {
    details(FULL);
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    fireEvent.click(await screen.findByRole("button", { name: "Show lyrics" }));
    expect(screen.getByText("Sixth line")).toBeInTheDocument();
    const back = screen.getByRole("button", { name: "Back to now playing" });
    expect(back).toHaveFocus();
    fireEvent.click(back);
    fireEvent.click(screen.getByRole("button", { name: `About ${solo.artistTitle}` }));
    expect(screen.getByText("Second paragraph.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Back to now playing" }));
    expect(screen.getByText("Next in queue")).toBeInTheDocument();
  });

  it("the lyrics card setting hides the card", async () => {
    details(FULL);
    useUi.getState().setNowPlayingPrefs({ lyricsCard: false });
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    await screen.findByText("About the artist");
    expect(screen.queryByText("First line")).toBeNull();
  });

  it("next in queue prefers the user's queue and opens the queue", async () => {
    details(EMPTY);
    usePlayer.setState({ state: playerState(solo, { userQueue: [item(third, 5, "user")] }) });
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    const card = (await screen.findByText("Next in queue")).closest("section")!;
    expect(within(card).getByText(third.title)).toBeInTheDocument();
    const jump = vi.spyOn(backend, "queueJump").mockResolvedValue();
    fireEvent.click(within(card).getByRole("button", { name: `Play ${third.title}` }));
    expect(jump).toHaveBeenCalledWith("q5");
    fireEvent.click(within(card).getByRole("button", { name: "Open queue" }));
    expect(screen.getByRole("complementary", { name: "Queue" })).toBeInTheDocument();
    expect(panel()).toBeNull();
  });

  it("says when nothing else is queued", async () => {
    details(EMPTY);
    usePlayer.setState({ state: playerState(solo, { nextUp: [] }) });
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    expect(await screen.findByText("Nothing else queued.")).toBeInTheDocument();
  });

  it("follows the track as it changes", async () => {
    const spy = vi.spyOn(backend, "nowPlayingDetails").mockImplementation(async (id) => ({ ...FULL, trackId: id, lyrics: id === solo.id ? FULL.lyrics : null }));
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    await screen.findByText("First line");
    act(() => usePlayer.setState({ state: playerState(other) }));
    expect(within(panel()!).getByRole("link", { name: other.title })).toBeInTheDocument();
    // The old track's lyrics go at once, not after the fetch.
    expect(screen.queryByText("First line")).toBeNull();
    await waitFor(() => expect(spy).toHaveBeenCalledWith(other.id));
    // Back to the first track: from the cache, no new request.
    act(() => usePlayer.setState({ state: playerState(solo) }));
    expect(screen.getByText("First line")).toBeInTheDocument();
    expect(spy.mock.calls.filter((c) => c[0] === solo.id)).toHaveLength(1);
  });

  it("says when nothing is playing", () => {
    usePlayer.setState({ state: playerState(null) });
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    expect(within(panel()!).getByText(/Nothing playing/)).toBeInTheDocument();
  });
});

describe("expand button", () => {
  beforeEach(() => {
    useFullWindow.setState({ open: false, mode: "lyrics", osFullscreen: false, prefs: { ...FULL_WINDOW_PREFS_DEFAULT, mode: "lyrics" } });
  });
  afterEach(() => {
    useFullWindow.setState({ open: false, mode: "artwork", osFullscreen: false, prefs: { ...FULL_WINDOW_PREFS_DEFAULT } });
  });

  it("opens the expanded view in artwork mode, even when another mode was used last", () => {
    details(EMPTY);
    useUi.getState().setNowPlayingOpen(true);
    render(<AppShell />);
    fireEvent.click(screen.getByRole("button", { name: "Expand now playing view" }));
    expect(useFullWindow.getState()).toMatchObject({ open: true, mode: "artwork" });
    expect(screen.getByRole("dialog", { name: "Expanded now playing" })).toBeInTheDocument();
    // One expanded view: the panel has no mode of its own.
    expect(screen.queryByRole("region", { name: "Now playing, expanded" })).toBeNull();
  });
});

describe("open when a song starts", () => {
  it("opens for a new track that starts playing, when the setting is on", () => {
    details(EMPTY);
    useUi.getState().setNowPlayingPrefs({ openOnStart: true });
    render(<AppShell />);
    expect(panel()).toBeNull();
    act(() => usePlayer.setState({ state: playerState(other) }));
    expect(panel()).toBeInTheDocument();
  });

  it("leaves an open queue alone, and does nothing when off", () => {
    details(EMPTY);
    render(<AppShell />);
    act(() => usePlayer.setState({ state: playerState(other) }));
    expect(panel()).toBeNull();
    useUi.getState().setNowPlayingPrefs({ openOnStart: true });
    act(() => useUi.getState().setQueueOpen(true));
    act(() => usePlayer.setState({ state: playerState(third) }));
    expect(useUi.getState()).toMatchObject({ queueOpen: true, nowPlayingOpen: false });
  });

  it("ignores the session restored at launch and paused tracks", () => {
    details(EMPTY);
    usePlayer.setState({ state: null });
    useUi.getState().setNowPlayingPrefs({ openOnStart: true });
    render(<AppShell />);
    act(() => usePlayer.setState({ state: playerState(solo, {}, "paused") }));
    act(() => usePlayer.setState({ state: playerState(other, {}, "paused") }));
    expect(useUi.getState().nowPlayingOpen).toBe(false);
  });
});

describe("settings", () => {
  it("both rows switch and are remembered on this device", () => {
    render(<NowPlayingSettings />);
    const open = screen.getByRole("switch", { name: "Open now playing when a song starts" });
    const lyrics = screen.getByRole("switch", { name: "Lyrics card in now playing" });
    expect(open).toHaveAttribute("aria-checked", "false");
    expect(lyrics).toHaveAttribute("aria-checked", "true");
    fireEvent.click(open);
    fireEvent.click(lyrics);
    expect(open).toHaveAttribute("aria-checked", "true");
    expect(lyrics).toHaveAttribute("aria-checked", "false");
    expect(loadNowPlayingPrefs()).toEqual({ openOnStart: true, lyricsCard: false });
  });

  it("falls back to defaults for missing or broken stored values", () => {
    localStorage.setItem("platter.nowPlaying.v1", "{nope");
    expect(loadNowPlayingPrefs()).toEqual(NOW_PLAYING_PREFS_DEFAULT);
    localStorage.setItem("platter.nowPlaying.v1", JSON.stringify({ openOnStart: "yes", lyricsCard: false }));
    expect(loadNowPlayingPrefs()).toEqual({ openOnStart: false, lyricsCard: false });
  });
});

describe("rules", () => {
  const lookup = new Map([["m. hale", "a9"]]);

  it("credits merge roles by name and look up library artists", () => {
    const rows = buildCredits(
      [
        { name: "Odessa Wren", id: "a1" },
        { name: "Guest", id: null },
      ],
      [
        { name: "odessa wren", roles: ["composer"] },
        { name: "M. Hale", roles: ["writer", "writer", "producer"] },
      ],
      lookup,
    );
    expect(rows).toEqual([
      { name: "Odessa Wren", id: "a1", roles: ["Main artist", "Composer"] },
      { name: "Guest", id: null, roles: ["Artist"] },
      { name: "M. Hale", id: "a9", roles: ["Writer", "Producer"] },
    ]);
    expect(creditsWorthShowing(rows)).toBe(true);
  });

  it("credits with only the main artist aren't worth a card", () => {
    const only = buildCredits([{ name: "Odessa Wren", id: "a1" }], [], lookup);
    expect(creditsWorthShowing(only)).toBe(false);
    expect(creditsWorthShowing(buildCredits([{ name: "Odessa Wren", id: "a1" }], [{ name: "Odessa Wren", roles: ["composer"] }], lookup))).toBe(true);
    expect(creditsWorthShowing([])).toBe(false);
  });

  it("next in queue and the context link", () => {
    const a = item(solo, 1, "user");
    const b = item(other, 2);
    expect(nextInQueue({ userQueue: [a], nextUp: [b], history: [] })).toBe(a);
    expect(nextInQueue({ userQueue: [], nextUp: [b], history: [] })).toBe(b);
    expect(nextInQueue({ userQueue: [], nextUp: [], history: [] })).toBeNull();
    expect(nextInQueue(null)).toBeNull();
    expect(contextRoute({ kind: "playlist", id: "p1", title: "Mix" })).toEqual({ name: "playlist", id: "p1" });
    expect(contextRoute({ kind: "artist", id: "a1" })).toEqual({ name: "artist", id: "a1" });
    expect(contextRoute({ kind: "search", id: "q" })).toBeNull();
    expect(contextRoute({ kind: "album" })).toBeNull();
  });

  it("lyrics preview skips breaks", () => {
    expect(lyricsPreview(["", "a", "", "b", "c"], 2)).toEqual(["a", "b"]);
  });
});
