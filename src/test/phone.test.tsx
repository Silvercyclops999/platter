/**
 * The phone layout: when it's used, its tabs, the mini player opening now
 * playing, the artwork's swipe and pull (distances, flicks and snapping
 * back), the queue sheet with the sleep timer stacked on it, lyrics that
 * seek on a tap, and the phone's settings.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Profiler } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../App";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyrics, mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type PlayerState, type QueueItem, type Track } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { phoneFromAgent, phoneFromUrl, usePhoneForm } from "../phone/form";
import { CLOSE_FRACTION, SKIP_FRACTION, follow, lockAxis, release } from "../phone/gestures";
import { anchorTarget, PHONE_ANCHOR } from "../phone/lyricsFollow";
import { cardWindow } from "../phone/LyricsScreen";
import { inLibraryLine, topByPlays } from "../phone/pages/Artist";
import { PhonePlaylist, topGenres } from "../phone/pages/Playlist";
import { dockInset } from "../phone/dockInset";
import { PhoneShell } from "../phone/PhoneShell";
import { CLOSE_MS } from "../phone/NowPlaying";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { SIRI_PHRASES } from "../phone/phoneSettings";
import { dropIndex } from "../phone/Sheets";
import { cancelSleep, due, sleepLabel, startSleep, useSleep } from "../phone/sleepTimer";
import { plateText, remaining } from "../phone/bits";
import { useDownloads } from "../stores/downloads";
import { useFileTags } from "../stores/fileTags";
import { player, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";

const mock = backend as MockBackend;
const tracks = (): Track[] => mock.fx.tracks;
const item = (t: Track, i: number): QueueItem => ({ itemId: `q${i}`, track: t, source: "context" });

function play(t: Track, extra: Partial<PlayerState> = {}) {
  const s: PlayerState = {
    status: "paused",
    track: t,
    positionMs: 30_000,
    durationMs: t.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: item(t, 0), userQueue: [], nextUp: tracks().slice(1, 5).map((x, i) => item(x, i + 1)), history: [], context: { kind: "album", id: t.albumId, title: t.albumTitle } },
    ...extra,
  };
  act(() => usePlayer.setState({ state: s, local: s, positionMs: s.positionMs, durationMs: s.durationMs, remote: null }));
}

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  usePhoneUi.setState({ tab: "home", nowPlaying: false, lyrics: false, sheets: [] });
  useRouter.getState().reset({ name: "home" });
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

afterEach(() => {
  cancelSleep();
  usePhoneForm.setState({ phone: false });
  vi.restoreAllMocks();
});

describe("when the phone layout is used", () => {
  it("takes ?phone=1 in the browser and a phone's web view in the app", () => {
    expect(phoneFromUrl("?phone=1")).toBe(true);
    expect(phoneFromUrl("?phone=0&x=1")).toBe(false);
    expect(phoneFromAgent("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)")).toBe(true);
    expect(phoneFromAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe(false);
  });

  it("the app draws the phone shell on a phone", async () => {
    render(<App />);
    expect(await screen.findByTestId("phone-shell")).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Tabs" })).toBeInTheDocument();
  });
});

describe("tabs", () => {
  it("switch between Home, Search, Library and You", async () => {
    render(<PhoneShell />);
    const tabs = screen.getByRole("navigation", { name: "Tabs" });
    expect(within(tabs).getByRole("button", { name: "Home" })).toHaveAttribute("aria-current", "page");
    fireEvent.click(within(tabs).getByRole("button", { name: "Search" }));
    expect(await screen.findByRole("searchbox", { name: "Search your library" })).toBeInTheDocument();
    expect(within(tabs).getByRole("button", { name: "Search" })).toHaveAttribute("aria-current", "page");
    fireEvent.click(within(tabs).getByRole("button", { name: "Library" }));
    expect(await screen.findByRole("heading", { name: "Library" })).toBeInTheDocument();
    fireEvent.click(within(tabs).getByRole("button", { name: "You" }));
    expect(await screen.findByTestId("phone-you")).toBeInTheDocument();
    expect(within(tabs).getAllByRole("button")).toHaveLength(4);
    expect(within(tabs).queryByRole("button", { name: "Crates" })).not.toBeInTheDocument();
    expect(useRouter.getState().entries).toHaveLength(1);
  });

  it("a search pill from an earlier search with none of its kind now shows everything", async () => {
    render(<PhoneShell />);
    fireEvent.click(within(screen.getByRole("navigation", { name: "Tabs" })).getByRole("button", { name: "Search" }));
    const box = await screen.findByRole("searchbox", { name: "Search your library" });
    fireEvent.change(box, { target: { value: "Copperline Choir" } });
    fireEvent.click(await screen.findByRole("button", { name: "Artists" }));
    fireEvent.change(box, { target: { value: "Late Night Drive" } });
    expect(await screen.findByText("Late Night Drive")).toBeInTheDocument();
    expect(screen.queryByText(/Nothing in your library matches/)).not.toBeInTheDocument();
  });

  it("a page opened from a tab keeps that tab lit, and the tab again goes back to its top", async () => {
    render(<PhoneShell />);
    const tabs = screen.getByRole("navigation", { name: "Tabs" });
    fireEvent.click(within(tabs).getByRole("button", { name: "Library" }));
    const album = mock.fx.albums[0];
    act(() => useRouter.getState().navigate({ name: "album", id: album.id }));
    expect(await screen.findByRole("heading", { name: album.title })).toBeInTheDocument();
    expect(within(tabs).getByRole("button", { name: "Library" })).toHaveAttribute("aria-current", "page");
    fireEvent.click(within(tabs).getByRole("button", { name: "Library" }));
    expect(await screen.findByRole("heading", { name: "Library" })).toBeInTheDocument();
  });
});

describe("mini player and now playing", () => {
  it("shows the song with its format plate and opens now playing", async () => {
    const t = tracks()[0];
    play(t);
    render(<PhoneShell />);
    const mini = screen.getByTestId("mini-player");
    expect(within(mini).getByText(t.title)).toBeInTheDocument();
    fireEvent.click(within(mini).getByRole("button", { name: /Open the player/ }));
    const np = await screen.findByRole("dialog", { name: "Now playing" });
    expect(within(np).getByText("From")).toBeInTheDocument();
    expect(within(np).getByText(t.albumTitle)).toBeInTheDocument();
    expect(within(np).getByRole("slider", { name: "Seek" })).toBeInTheDocument();
    expect(within(np).getByText(remaining(30_000, t.durationMs))).toBeInTheDocument();
    fireEvent.click(within(np).getByRole("button", { name: "Close now playing" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Now playing" })).not.toBeInTheDocument());
  });

  it("pages pad their foot by what the dock covers, with the mini player and without", async () => {
    // A dock 90 px high with only the tabs (and the home indicator), 72 more with the mini player.
    const real = HTMLElement.prototype.getBoundingClientRect;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (!this.classList.contains("ph-dock")) return real.call(this);
      const h = 90 + (this.querySelector(".ph-mini") ? 72 : 0);
      return { x: 0, y: 844 - h, left: 0, top: 844 - h, width: 390, height: h, right: 390, bottom: 844, toJSON: () => ({}) } as DOMRect;
    });
    act(() => usePlayer.setState({ state: null }));
    render(<PhoneShell />);
    const shell = screen.getByTestId("phone-shell");
    expect(shell.style.getPropertyValue("--ph-dock-h")).toBe("90px");
    play(tracks()[0]);
    await waitFor(() => expect(shell.style.getPropertyValue("--ph-dock-h")).toBe("162px"));
    act(() => usePlayer.setState({ state: null }));
    await waitFor(() => expect(shell.style.getPropertyValue("--ph-dock-h")).toBe("90px"));
    expect(dockInset(0)).toBeNull();
    expect(dockInset(161.2)).toBe("162px");
  });

  it("now playing marks an explicit, downloaded song with the row glyphs before the artist, not words", async () => {
    const t = tracks()[0];
    play(t);
    render(<PhoneShell />);
    act(() => phone.openPlayer());
    const np = await screen.findByRole("dialog", { name: "Now playing" });
    // After the shell's own first load of the downloaded songs.
    await waitFor(() => expect(useDownloads.getState().status).not.toBeNull());
    act(() => {
      useFileTags.setState({ index: { tracks: { [t.id]: { explicit: true } }, albums: {}, groups: {} } });
      useDownloads.setState({ ids: new Set([t.id]) });
    });
    const line = np.querySelector(".ph-np__artist-line") as HTMLElement;
    expect(within(line).getAllByRole("img").map((g) => g.getAttribute("aria-label"))).toEqual(["Explicit", "Downloaded"]);
    expect(within(np).queryByText("Downloaded")).toBeNull();
    act(() => {
      useFileTags.setState({ index: { tracks: {}, albums: {}, groups: {} } });
      useDownloads.setState({ ids: new Set() });
    });
  });

  it("the plate says the format, or nothing when it is not known", () => {
    const t = { codec: "flac", bitDepth: 24, sampleRate: 96000, bitrate: 2300 };
    expect(plateText(t)).toBe("FLAC 24/96");
    expect(plateText({ codec: null, bitDepth: null, sampleRate: null, bitrate: null })).toBeNull();
  });
});

describe("artwork gestures", () => {
  it("pick a direction, skip past 30% of the width or on a flick, close past 25% of the height", () => {
    const base = { vx: 0, vy: 0, width: 400, height: 800 };
    expect(lockAxis(4, 3)).toBeNull();
    expect(lockAxis(30, 5)).toBe("x");
    expect(lockAxis(4, 30)).toBe("y");
    expect(release({ ...base, axis: "x", dx: -400 * SKIP_FRACTION, dy: 0 })).toBe("next");
    expect(release({ ...base, axis: "x", dx: 400 * SKIP_FRACTION, dy: 0 })).toBe("previous");
    expect(release({ ...base, axis: "x", dx: -60, dy: 0 })).toBe("snap");
    expect(release({ ...base, axis: "x", dx: -60, dy: 0, vx: -1 })).toBe("next");
    // A flick back the other way doesn't skip.
    expect(release({ ...base, axis: "x", dx: -60, dy: 0, vx: 1 })).toBe("snap");
    expect(release({ ...base, axis: "y", dx: 0, dy: 800 * CLOSE_FRACTION })).toBe("close");
    expect(release({ ...base, axis: "y", dx: 0, dy: 80 })).toBe("snap");
    expect(release({ ...base, axis: "y", dx: 0, dy: 80, vy: 1.2 })).toBe("close");
    // Up does nothing (it only gives a little).
    expect(release({ ...base, axis: "y", dx: 0, dy: -400, vy: -3 })).toBe("snap");
    expect(follow("y", 0, -300).y).toBeGreaterThanOrEqual(-24);
    expect(follow("x", 50, 40)).toEqual({ x: 50, y: 0 });
  });

  function drag(el: HTMLElement, from: [number, number], to: [number, number], ms: number) {
    let now = 1000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    fireEvent.pointerDown(el, { pointerId: 1, button: 0, clientX: from[0], clientY: from[1] });
    const steps = 5;
    for (let i = 1; i <= steps; i++) {
      now += ms / steps;
      fireEvent.pointerMove(el, { pointerId: 1, clientX: from[0] + ((to[0] - from[0]) * i) / steps, clientY: from[1] + ((to[1] - from[1]) * i) / steps });
    }
    return {
      offset: () => el.style.transform,
      up: () => fireEvent.pointerUp(el, { pointerId: 1, clientX: to[0], clientY: to[1] }),
    };
  }

  async function openPlayer() {
    play(tracks()[0], { status: "playing" });
    render(<PhoneShell />);
    act(() => phone.openPlayer());
    const art = await screen.findByTestId("np-art");
    Object.defineProperty(art, "clientWidth", { value: 400, configurable: true });
    const np = screen.getByRole("dialog", { name: "Now playing" });
    Object.defineProperty(np, "clientHeight", { value: 800, configurable: true });
    return art;
  }

  const row = () => screen.getByTestId("np-deck");
  const covers = () => [...document.querySelectorAll<HTMLElement>(".ph-np__cover")];
  const after = (t: Track[], at: number, status: PlayerState["status"]) =>
    play(t[at], { status, queue: { current: item(t[at], at), userQueue: [], nextUp: t.slice(at + 1, at + 4).map((x, i) => item(x, at + 1 + i)), history: [item(t[at - 1], at - 1)], context: null } });

  it("a slow swipe past the distance skips; the covers follow the finger and settle on the next one", async () => {
    const next = vi.spyOn(player, "next").mockImplementation(() => Promise.resolve());
    const prev = vi.spyOn(player, "previous").mockImplementation(() => Promise.resolve());
    const art = await openPlayer();
    const g = drag(art, [300, 400], [140, 400], 800);
    expect(row().style.transform).toBe("translateX(-160px)");
    // The art box itself never moves: the row inside it does.
    expect(art.style.transform).toBe("");
    await act(async () => g.up());
    expect(next).toHaveBeenCalledTimes(1);
    const back = drag(art, [100, 400], [250, 400], 800);
    await act(async () => back.up());
    expect(prev).toHaveBeenCalledTimes(1);
  });

  it("a short slow swipe snaps back without skipping; a short fast flick skips", async () => {
    const next = vi.spyOn(player, "next").mockImplementation(() => Promise.resolve());
    const art = await openPlayer();
    const slow = drag(art, [300, 400], [240, 400], 1000);
    await act(async () => slow.up());
    expect(next).not.toHaveBeenCalled();
    expect(row().style.transform).toBe("");
    const flick = drag(art, [300, 400], [240, 400], 50);
    await act(async () => flick.up());
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("the previous, this and the next cover sit side by side at one size, and stay that size through a skip", async () => {
    vi.spyOn(player, "next").mockImplementation(() => Promise.resolve());
    const art = await openPlayer();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const t = tracks();
      // This song's cover, the next one's, and this one's again on the left (Previous restarts it 30 s in).
      expect(covers()).toHaveLength(3);
      expect(document.querySelector(".ph-np__slide--next")).toHaveAttribute("aria-hidden", "true");
      const sizes = () => covers().map((c) => c.className);
      const before = sizes();
      expect(new Set(before).size).toBe(1);
      const g = drag(art, [300, 400], [100, 400], 800);
      // While the finger is down a pause flickers by (as between two songs): nothing changes size.
      play(t[0], { status: "paused" });
      act(() => void vi.advanceTimersByTime(100));
      play(t[0], { status: "playing" });
      expect(sizes()).toEqual(before);
      await act(async () => g.up());
      // The row carried on to the next cover, which is the one now playing once the player answers.
      after(t, 1, "loading");
      act(() => void vi.advanceTimersByTime(100));
      after(t, 1, "playing");
      act(() => void vi.advanceTimersByTime(1000));
      expect(new Set(sizes()).size).toBe(1);
      expect(sizes()).toEqual(before);
      for (const c of covers()) expect(c.style.transform).toBe("");
      expect(row().style.transform).toBe("");
    } finally {
      vi.useRealTimers();
    }
  });

  it("a pause that lasts lets the cover settle back a little, all three together", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      play(tracks()[0], { status: "playing" });
      render(<PhoneShell />);
      act(() => phone.openPlayer());
      expect(covers()).toHaveLength(3);
      expect(covers().every((c) => c.classList.contains("is-playing"))).toBe(true);
      play(tracks()[0], { status: "paused" });
      expect(covers().every((c) => c.classList.contains("is-playing"))).toBe(true);
      act(() => void vi.advanceTimersByTime(400));
      expect(covers().some((c) => c.classList.contains("is-playing"))).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("pulling down far enough closes the player; a short pull snaps back", async () => {
    const art = await openPlayer();
    const np = screen.getByRole("dialog", { name: "Now playing" });
    const short = drag(art, [200, 200], [200, 300], 1000);
    expect(np.style.transform).toBe("translateY(100px)");
    short.up();
    expect(usePhoneUi.getState().nowPlaying).toBe(true);
    expect(np.style.transform).toBe("");
    const long = drag(art, [200, 200], [200, 450], 1000);
    long.up();
    expect(usePhoneUi.getState().nowPlaying).toBe(false);
  });

  it("a drag draws nothing new while the finger moves: only the transform changes", async () => {
    const commits: string[] = [];
    play(tracks()[0], { status: "playing" });
    render(
      <Profiler id="phone" onRender={(_id, phase) => commits.push(phase)}>
        <PhoneShell />
      </Profiler>,
    );
    act(() => phone.openPlayer());
    const art = await screen.findByTestId("np-art");
    Object.defineProperty(art, "clientWidth", { value: 400, configurable: true });
    const np = screen.getByRole("dialog", { name: "Now playing" });
    Object.defineProperty(np, "clientHeight", { value: 800, configurable: true });
    await act(async () => {});
    commits.length = 0;
    const pull = drag(art, [200, 200], [200, 300], 1000);
    expect(np.style.transform).toBe("translateY(100px)");
    expect(np).toHaveClass("is-dragging");
    const swipe = drag(art, [300, 400], [250, 400], 1000);
    expect(commits).toEqual([]);
    swipe.up();
    pull.up();
  });

  it("closing leaves the pages inert until now playing has slid off them", async () => {
    play(tracks()[0], { status: "playing" });
    render(<PhoneShell />);
    act(() => phone.openPlayer());
    await screen.findByTestId("np-art");
    const main = document.querySelector(".ph-main")!;
    expect(main).toHaveAttribute("inert");
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      act(() => phone.closePlayer());
      // Back in reach for assistive tech at once, but restyling the pages waits for the slide.
      expect(main).not.toHaveAttribute("aria-hidden");
      expect(main).toHaveAttribute("inert");
      expect(screen.getByRole("dialog", { name: "Now playing", hidden: true })).toHaveClass("is-closing");
      act(() => void vi.advanceTimersByTime(CLOSE_MS));
      expect(main).not.toHaveAttribute("inert");
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("queue and sleep timer", () => {
  it("the queue pins the playing song and the sleep timer stacks on it, closing back to the queue", async () => {
    const t = tracks()[0];
    play(t);
    render(<PhoneShell />);
    act(() => {
      phone.openPlayer();
      phone.push({ kind: "queue" });
    });
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    expect(within(queue).getByText(t.title)).toHaveClass("is-playing");
    // Under Now playing; a tap on the row plays or pauses it.
    expect(within(within(queue).getByRole("region", { name: "Now playing" })).getByRole("button", { name: `Play ${t.title}` })).toBeInTheDocument();
    expect(within(queue).getAllByRole("button", { name: /^Move / })).toHaveLength(4);
    fireEvent.click(within(queue).getByRole("button", { name: "Sleep timer" }));
    const sleep = await screen.findByRole("dialog", { name: "Sleep timer" });
    for (const label of ["5 minutes", "10 minutes", "15 minutes", "30 minutes", "45 minutes", "1 hour", "End of this song", "End of this album", "Custom length"])
      expect(within(sleep).getByRole("button", { name: label })).toBeInTheDocument();
    fireEvent.click(within(sleep).getByRole("button", { name: "15 minutes" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Sleep timer" })).not.toBeInTheDocument());
    expect(screen.getByRole("dialog", { name: "Queue" })).toBeInTheDocument();
    expect(useSleep.getState().timer?.kind).toBe("minutes");
    expect(within(screen.getByRole("dialog", { name: "Queue" })).getByText("15 min")).toBeInTheDocument();
  });

  it("a custom length starts the timer; closing the timer sheet goes back to the queue", async () => {
    play(tracks()[0]);
    render(<PhoneShell />);
    act(() => {
      phone.push({ kind: "queue" });
      phone.push({ kind: "sleep" });
    });
    const sleep = await screen.findByRole("dialog", { name: "Sleep timer" });
    fireEvent.click(within(sleep).getByRole("button", { name: "Custom length" }));
    fireEvent.change(within(sleep).getByRole("spinbutton", { name: "Minutes" }), { target: { value: "25" } });
    fireEvent.click(within(sleep).getByRole("button", { name: "Start" }));
    await waitFor(() => expect(useSleep.getState().timer).toMatchObject({ kind: "minutes", minutes: 25 }));
    act(() => phone.push({ kind: "sleep" }));
    fireEvent.click(screen.getAllByTestId("sheet-dim").slice(-1)[0]);
    expect(usePhoneUi.getState().sheets.map((s) => s.kind)).toEqual(["queue"]);
  });

  it("Escape closes only the sheet on top", async () => {
    play(tracks()[0]);
    render(<PhoneShell />);
    act(() => {
      phone.push({ kind: "queue" });
      phone.push({ kind: "sleep" });
    });
    await screen.findByRole("dialog", { name: "Sleep timer" });
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(usePhoneUi.getState().sheets.map((s) => s.kind)).toEqual(["queue"]));
    await new Promise((r) => setTimeout(r, 100));
    expect(usePhoneUi.getState().sheets.map((s) => s.kind)).toEqual(["queue"]);
  });

  it("edit mode moves picked songs up and removes them", async () => {
    play(tracks()[0]);
    const move = vi.spyOn(player, "queueMove").mockImplementation(() => Promise.resolve());
    const remove = vi.spyOn(player, "queueRemove").mockImplementation(() => Promise.resolve());
    render(<PhoneShell />);
    act(() => phone.push({ kind: "queue" }));
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    fireEvent.click(within(queue).getByRole("button", { name: "Edit" }));
    expect(within(queue).getByRole("button", { name: "Move up" })).toBeDisabled();
    fireEvent.click(within(queue).getByRole("checkbox", { name: `Select ${tracks()[3].title}` }));
    fireEvent.click(within(queue).getByRole("button", { name: "Move up" }));
    await waitFor(() => expect(move).toHaveBeenCalledWith("q3", 0));
    fireEvent.click(within(queue).getByRole("checkbox", { name: `Select ${tracks()[2].title}` }));
    fireEvent.click(within(queue).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(remove).toHaveBeenCalledWith("q2"));
  });

  it("dragging a handle moves the row by whole rows", async () => {
    expect(dropIndex(0, 130, 4)).toBe(2);
    expect(dropIndex(3, -500, 4)).toBe(0);
    expect(dropIndex(1, 20, 4)).toBe(1);
    play(tracks()[0]);
    const move = vi.spyOn(player, "queueMove").mockImplementation(() => Promise.resolve());
    render(<PhoneShell />);
    act(() => phone.push({ kind: "queue" }));
    const handle = await screen.findByTestId("handle-q1");
    fireEvent.pointerDown(handle, { pointerId: 2, clientY: 100 });
    fireEvent.pointerMove(handle, { pointerId: 2, clientY: 230 });
    fireEvent.pointerUp(handle, { pointerId: 2, clientY: 230 });
    expect(move).toHaveBeenCalledWith("q1", 2);
  });

  it("the timer is due at its time, at a song's end, or at the album's last song's end", () => {
    const [a, b] = tracks();
    play(a, { positionMs: 10_000 });
    expect(due({ kind: "minutes", minutes: 5, endsAt: 100_000 }, 0)).toBe(false);
    expect(due({ kind: "minutes", minutes: 5, endsAt: 100_000 }, 99_000)).toBe(true);
    expect(due({ kind: "track", trackId: a.id }, 0)).toBe(false);
    play(a, { positionMs: a.durationMs - 1000 });
    expect(due({ kind: "track", trackId: a.id }, 0)).toBe(true);
    // Not the album's last song: plays on.
    expect(due({ kind: "album", albumId: a.albumId }, 0)).toBe(b.albumId !== a.albumId);
    expect(sleepLabel({ kind: "minutes", minutes: 90, endsAt: 90 * 60_000 }, 0)).toBe("1 h 30 min");
    expect(sleepLabel(null)).toBe("Sleep timer");
  });

  it("the sleep timer fades out and pauses, then puts the volume back", async () => {
    vi.useFakeTimers();
    play(tracks()[0], { status: "playing" });
    const pause = vi.spyOn(player, "pause").mockImplementation(() => Promise.resolve());
    const vol = vi.spyOn(player, "setVolume").mockImplementation(() => Promise.resolve());
    startSleep({ kind: "minutes", minutes: 5 }, Date.now());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5 * 60_000 + 1000);
    });
    expect(pause).toHaveBeenCalledTimes(1);
    expect(vol.mock.calls.slice(-1)[0]?.[0]).toBe(0.5);
    expect(useSleep.getState().timer).toBeNull();
  });

  it("turned off mid-fade, the music plays on at its own level", async () => {
    vi.useFakeTimers();
    play(tracks()[0], { status: "playing" });
    const pause = vi.spyOn(player, "pause").mockImplementation(() => Promise.resolve());
    const vol = vi.spyOn(player, "setVolume").mockImplementation(() => Promise.resolve());
    startSleep({ kind: "minutes", minutes: 5 }, Date.now());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5 * 60_000 - 3000);
    });
    expect(useSleep.getState().fading).toBe(true);
    expect(vol.mock.calls.slice(-1)[0]?.[0]).toBeLessThan(0.5);
    act(() => cancelSleep());
    expect(vol.mock.calls.slice(-1)[0]?.[0]).toBe(0.5);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(pause).not.toHaveBeenCalled();
    expect(vol.mock.calls.slice(-1)[0]?.[0]).toBe(0.5);
  });
});

describe("lyrics", () => {
  const timedTrack = () => tracks().find((t) => mockLyricsKind(t.id) === "lines" || mockLyricsKind(t.id) === "words")!;

  it("keeps the current line about 45% down, never above the top", () => {
    expect(PHONE_ANCHOR).toBe(0.45);
    expect(anchorTarget(100, 30, 800, 3000)).toBe(0);
    expect(anchorTarget(1000, 30, 800, 3000)).toBe(1000 + 15 - 360);
    expect(anchorTarget(2900, 30, 800, 3000)).toBe(2200);
  });

  it("the card shows the current line lit and the next ones dim", () => {
    const t = timedTrack();
    const l = mockLyrics(t)!;
    const at = l.timed![2].startMs + 10;
    const w = cardWindow(l, at);
    expect(w[0]).toEqual({ text: l.timed![2].text.trim() || "♪", sung: true });
    expect(w.slice(1).every((x) => !x.sung)).toBe(true);
  });

  it("full screen: tapping a line seeks there, keeps pause, and the lines update at once", async () => {
    const t = timedTrack();
    const l = mockLyrics(t)!;
    play(t, { status: "paused", positionMs: 0 });
    const seek = vi.spyOn(backend, "playSeek");
    const toggle = vi.spyOn(player, "toggle");
    render(<PhoneShell />);
    act(() => {
      phone.openPlayer();
      phone.openLyrics();
    });
    const view = await screen.findByRole("dialog", { name: "Lyrics" });
    const lineAt = (i: number) => view.querySelector<HTMLElement>(`[data-line="${i}"]`)!;
    await waitFor(() => expect(lineAt(3)).toBeTruthy());
    expect(lineAt(3)).not.toHaveClass("is-sung");
    fireEvent.click(lineAt(3));
    expect(seek).toHaveBeenCalledWith(l.timed![3].startMs);
    expect(toggle).not.toHaveBeenCalled();
    expect(usePlayer.getState().state?.status).toBe("paused");
    await waitFor(() => expect(lineAt(3)).toHaveClass("is-sung"));
    expect(lineAt(3)).toHaveAttribute("aria-current", "true");
    expect(lineAt(4)).not.toHaveClass("is-sung");
  });
});

describe("pages", () => {
  it("top tracks rank by your plays and the library line counts what you have", () => {
    const [a, b, c] = tracks();
    expect(topByPlays([{ ...a, viewCount: 2 }, { ...b, viewCount: 9 }, { ...c, viewCount: 2 }]).map((t) => t.id)).toEqual([b.id, a.id, c.id]);
    expect(inLibraryLine(1, 12)).toBe("In your library: 1 album, 12 songs");
    expect(topGenres([{ ...a, genres: ["Jazz", "Soul"] }, { ...b, genres: ["Soul"] }])).toEqual(["Soul", "Jazz"]);
  });

  it("a song's sheet offers Remove from this playlist only on that playlist's page", async () => {
    const t = tracks()[0];
    render(<PhoneShell />);
    act(() => phone.push({ kind: "track", track: t }));
    let sheet = await screen.findByRole("dialog", { name: t.title });
    expect(within(sheet).queryByRole("button", { name: "Remove from this playlist" })).not.toBeInTheDocument();
    act(() => phone.closeSheets());
    const pl = mock.fx.playlists[0];
    act(() => phone.push({ kind: "track", track: t, inPlaylist: { playlist: pl, itemId: "i1", order: ["i1"] } }));
    sheet = await screen.findByRole("dialog", { name: t.title });
    expect(within(sheet).getByRole("button", { name: "Remove from this playlist" })).toBeInTheDocument();
  });
});

describe("playlist genre pills", () => {
  it("the play button pauses while the playlist plays, with or without a genre picked", async () => {
    const [a, b] = tracks();
    const pl = mock.fx.playlists[0];
    vi.spyOn(backend, "playlistPage").mockResolvedValue({
      playlist: pl,
      tracks: [{ ...a, genres: ["Jazz"] }, { ...b, genres: ["Soul"] }],
      itemIds: ["i1", "i2"],
    } as Awaited<ReturnType<typeof backend.playlistPage>>);
    const toggle = vi.spyOn(player, "toggle").mockResolvedValue(undefined as never);
    const playPlaylist = vi.spyOn(player, "playPlaylist").mockResolvedValue(undefined as never);
    play(a, { status: "playing" });
    const s = usePlayer.getState().state!;
    act(() => usePlayer.setState({ state: { ...s, queue: { ...s.queue, context: { kind: "playlist", id: pl.id, title: pl.title } } } }));
    render(<PhonePlaylist id={pl.id} />);
    fireEvent.click(await screen.findByRole("button", { name: "Soul" }));
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(toggle).toHaveBeenCalledTimes(1);
    expect(playPlaylist).not.toHaveBeenCalled();
    // Paused, a picked genre plays just its songs.
    act(() => usePlayer.setState({ state: { ...usePlayer.getState().state!, status: "paused" } }));
    fireEvent.click(screen.getByRole("button", { name: `Play ${pl.title}` }));
    expect(playPlaylist).toHaveBeenCalledWith(pl.id, undefined, undefined, [b.id]);
    expect(toggle).toHaveBeenCalledTimes(1);
  });
});

describe("phone settings", () => {
  it("show on the phone only, and persist", async () => {
    const { unmount } = render(<SettingsPage section="Downloads and devices" />);
    expect(await screen.findByText("Haptics on controls")).toBeInTheDocument();
    const row = (key: string) => document.querySelector(`[data-setting="${key}"]`) as HTMLElement;
    fireEvent.click(within(row("phoneHaptics")).getByRole("switch"));
    fireEvent.click(within(row("phoneMobileDownloads")).getByRole("switch"));
    await waitFor(async () => {
      const s = await backend.settingsGet();
      expect(s.phoneHaptics).toBe(false);
      expect(s.phoneMobileDownloads).toBe(true);
    });
    expect(useSettings.getState().settings?.phoneKeepRecent).toBe(50);
    expect(useSettings.getState().settings?.phoneMobileQuality).toBe("auto");
    unmount();
    usePhoneForm.setState({ phone: false });
    render(<SettingsPage />);
    await screen.findAllByText("Download location");
    expect(screen.queryByText("Haptics on controls")).not.toBeInTheDocument();
  });

  it("CarPlay's two rows start on and persist", async () => {
    render(<SettingsPage section="Downloads and devices" />);
    expect(await screen.findByText("Resume when the car connects")).toBeInTheDocument();
    expect(screen.getByText("Show downloads only when offline")).toBeInTheDocument();
    const row = (key: string) => document.querySelector(`[data-setting="${key}"]`) as HTMLElement;
    for (const key of ["carplayResume", "carplayOfflineDownloads"]) {
      const sw = within(row(key)).getByRole("switch");
      expect(sw).toBeChecked();
      fireEvent.click(sw);
    }
    await waitFor(async () => {
      const s = await backend.settingsGet();
      expect(s.carplayResume).toBe(false);
      expect(s.carplayOfflineDownloads).toBe(false);
    });
  });

  it("Siri has a row of things to say and a dislike choice that hides by default", async () => {
    render(<SettingsPage section="Downloads and devices" />);
    expect(await screen.findByText("Ask Siri")).toBeInTheDocument();
    for (const said of ["on Platter", "on shuffle", "play more like this", "I like this song", "Yesterday next"]) {
      expect(SIRI_PHRASES).toContain(said);
    }
    const row = document.querySelector('[data-setting="siriDislikeHides"]') as HTMLElement;
    expect(within(row).getByText("When you tell Siri you don't like a song")).toBeInTheDocument();
    expect(within(row).getByText("Hide it and skip")).toBeInTheDocument();
    await backend.settingsSet({ siriDislikeHides: false });
    expect((await backend.settingsGet()).siriDislikeHides).toBe(false);
  });

  it("keeps only the offered counts of recent songs", async () => {
    await expect(backend.settingsSet({ phoneKeepRecent: 7 })).rejects.toMatchObject({ kind: "invalid" });
    await backend.settingsSet({ phoneKeepRecent: 250, phoneMobileQuality: "192" });
    const s = await backend.settingsGet();
    expect([s.phoneKeepRecent, s.phoneMobileQuality]).toEqual([250, "192"]);
  });
});
