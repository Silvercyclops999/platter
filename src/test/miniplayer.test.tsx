/**
 * The miniplayer: opening and closing it from the player bar and the app
 * menu (and the window failing to open), its controls on hover, its own
 * settings panel, which window the page is, how it lays out at each window
 * size, and its queue (play, menu, keyboard, drag to reorder).
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type PlayerState, type QueueItem, type Track } from "../lib/types";
import { formatDuration } from "../lib/format";
import { advancedView } from "../pages/settingsMeta";
import { SettingsPage } from "../pages/SettingsPage";
import {
  CARD_PAD,
  Miniplayer,
  miniLayout,
  PAD,
  QUEUE_MIN_HEIGHT,
  queueMoveFor,
  SIDE_MIN_WIDTH,
  SIDE_QUEUE_MIN_HEIGHT,
  SIDE_RATIO,
  STRIP_H,
  upNext,
} from "../shell/Miniplayer";
import { PlayerBar } from "../shell/PlayerBar";
import { AppMenu } from "../shell/TopBar";
import { connectMiniplayer, isMiniplayerWindow, MINIPLAYER_FAILED, useMiniplayer } from "../stores/miniplayer";
import { player, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const track = (): Track => mock.fx.tracks[0];

function play(t: Track, extra: Partial<PlayerState> = {}) {
  const s: PlayerState = {
    status: "paused",
    track: t,
    positionMs: 61_000,
    durationMs: t.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: null, userQueue: [], nextUp: [], history: [], context: null },
    ...extra,
  };
  act(() => usePlayer.setState({ state: s, positionMs: s.positionMs, durationMs: s.durationMs }));
}

const item = (t: Track, i: number): QueueItem => ({ itemId: `q${i}`, track: t, source: "context" });

beforeEach(() => {
  mock.miniplayerOpen = false;
  mock.miniplayerFailsNext = false;
  useMiniplayer.setState({ open: false });
  useToasts.setState({ toasts: [] });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("opening and closing", () => {
  it("the player bar button opens it, shows it's open, and closes it", async () => {
    const stop = connectMiniplayer();
    play(track());
    render(<PlayerBar />);
    fireEvent.click(screen.getByRole("button", { name: "Open miniplayer" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Close miniplayer" })).toHaveAttribute("aria-pressed", "true"));
    expect(mock.miniplayerOpen).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Close miniplayer" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Open miniplayer" })).toHaveAttribute("aria-pressed", "false"));
    expect(mock.miniplayerOpen).toBe(false);
    stop();
  });

  it("closing it from its own window turns the button off", async () => {
    const stop = connectMiniplayer();
    play(track());
    render(<PlayerBar />);
    await act(async () => {});
    await act(() => mock.miniplayerSet(true));
    await waitFor(() => expect(screen.getByRole("button", { name: "Close miniplayer" })).toBeInTheDocument());
    await act(() => mock.miniplayerSet(false));
    await waitFor(() => expect(screen.getByRole("button", { name: "Open miniplayer" })).toBeInTheDocument());
    stop();
  });

  it("the app menu opens it and shows a tick while it's open", async () => {
    const stop = connectMiniplayer();
    render(<AppMenu />);
    fireEvent.click(screen.getByRole("button", { name: /^App menu/ }));
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: "Miniplayer" }));
    await waitFor(() => expect(useMiniplayer.getState().open).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: /^App menu/ }));
    expect(screen.getByRole("menuitemcheckbox", { name: "Miniplayer" })).toHaveAttribute("aria-checked", "true");
    stop();
  });

  it("a window that fails to open says so and stays off", async () => {
    mock.miniplayerFailsNext = true;
    play(track());
    render(<PlayerBar />);
    fireEvent.click(screen.getByRole("button", { name: "Open miniplayer" }));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain(MINIPLAYER_FAILED));
    expect(useMiniplayer.getState().open).toBe(false);
    expect(screen.getByRole("button", { name: "Open miniplayer" })).toHaveAttribute("aria-pressed", "false");
  });
});

describe("the miniplayer page", () => {
  it("is the page for the miniplayer window only", () => {
    expect(isMiniplayerWindow()).toBe(false);
    const spy = vi.spyOn(mock, "windowLabel").mockReturnValue("miniplayer");
    expect(isMiniplayerWindow()).toBe(true);
    spy.mockRestore();
  });

  it("at rest shows the art, title, artist and like control, with the controls hidden", () => {
    play(track());
    render(<Miniplayer />);
    expect(screen.getByText(track().title)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: `Add ${track().title} to Liked Songs` })).toBeInTheDocument();
    expect(screen.getByTestId("mini-art")).not.toHaveClass("is-active");
    expect(screen.getByRole("button", { name: "Miniplayer settings" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Close miniplayer" })).toBeInTheDocument();
  });

  it("pointing at the art brings up the controls and the progress; leaving hides them", () => {
    play(track());
    render(<Miniplayer />);
    const art = screen.getByTestId("mini-art");
    fireEvent.pointerEnter(art);
    expect(art).toHaveClass("is-active");
    const row = within(art);
    for (const name of ["Mute", "Enable shuffle", "Previous", "Play", "Next", "Enable repeat", "Copy link"]) {
      expect(row.getByRole("button", { name })).toBeInTheDocument();
    }
    expect(row.getByRole("progressbar", { name: "Song progress" })).toHaveAttribute("aria-valuetext", expect.stringMatching(/^1:01 of /));
    fireEvent.pointerLeave(art);
    expect(art).not.toHaveClass("is-active");
  });

  it("the controls drive the player", () => {
    const toggle = vi.spyOn(player, "toggle").mockResolvedValue(undefined);
    const next = vi.spyOn(player, "next").mockResolvedValue(undefined);
    play(track());
    render(<Miniplayer />);
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(toggle).toHaveBeenCalled();
    expect(next).toHaveBeenCalled();
  });

  it("pointing at mute pops up a vertical volume slider", () => {
    play(track());
    render(<Miniplayer />);
    fireEvent.pointerEnter(screen.getByTestId("mini-art"));
    expect(screen.queryByRole("slider", { name: "Volume" })).toBeNull();
    fireEvent.pointerEnter(screen.getByRole("button", { name: "Mute" }).closest(".mini__mute")!);
    const slider = screen.getByRole("slider", { name: "Volume" });
    expect(slider).toHaveAttribute("aria-orientation", "vertical");
    expect(slider).toHaveAttribute("aria-valuenow", "50");
    const nudge = vi.spyOn(player, "nudgeVolume").mockResolvedValue(undefined);
    fireEvent.keyDown(slider, { key: "ArrowUp" });
    expect(nudge).toHaveBeenCalledWith(0.1);
  });

  it("the close button closes the window", async () => {
    mock.miniplayerOpen = true;
    play(track());
    render(<Miniplayer />);
    fireEvent.click(screen.getByRole("button", { name: "Close miniplayer" }));
    await waitFor(() => expect(mock.miniplayerOpen).toBe(false));
  });

  it("nothing playing and nothing played yet shows the mark and says so", async () => {
    vi.spyOn(mock, "recentTracks").mockResolvedValue([]);
    act(() => usePlayer.setState({ state: null, positionMs: 0, durationMs: 0 }));
    render(<Miniplayer />);
    expect(await screen.findByText("Nothing playing")).toBeInTheDocument();
    expect(screen.queryByTestId("mini-art")).toBeNull();
  });

  it("nothing playing shows the last song played, dimmed, and Play starts it again", async () => {
    const last = mock.fx.tracks[2];
    vi.spyOn(mock, "recentTracks").mockResolvedValue([last]);
    const playAlbum = vi.spyOn(player, "playAlbum").mockResolvedValue(undefined);
    act(() => usePlayer.setState({ state: null, positionMs: 0, durationMs: 0 }));
    render(<Miniplayer />);
    expect(await screen.findByTestId("mini-idle")).toHaveClass("is-idle");
    fireEvent.click(screen.getByRole("button", { name: `Play ${last.title}` }));
    expect(playAlbum).toHaveBeenCalledWith(last.albumId, last.id);
  });

  it("a long title stays on one line with the whole of it in a tooltip", () => {
    const long = { ...track(), title: "A very long song title that would never fit in a small window at all (extended mix)" };
    play(long);
    render(<Miniplayer />);
    const title = screen.getByText(long.title);
    expect(title).toHaveClass("truncate");
    expect(title).toHaveAttribute("title", long.title);
  });

  it("a thin line on the art's bottom edge shows how far through the song it is", () => {
    const t = { ...track(), durationMs: 200_000 };
    play(t, { positionMs: 50_000, durationMs: 200_000 });
    render(<Miniplayer />);
    expect(screen.getByTestId("mini-edge-line")).toHaveStyle({ width: "25%" });
    act(() => usePlayer.getState().setPosition(150_000, 200_000));
    expect(screen.getByTestId("mini-edge-line")).toHaveStyle({ width: "75%" });
  });

  it("Space plays and pauses, but not when a button has focus", () => {
    const toggle = vi.spyOn(player, "toggle").mockResolvedValue(undefined);
    play(track());
    render(<Miniplayer />);
    fireEvent.keyDown(document.body, { key: " " });
    expect(toggle).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(screen.getByRole("button", { name: "Close miniplayer" }), { key: " " });
    expect(toggle).toHaveBeenCalledTimes(1);
  });
});

describe("its settings panel", () => {
  it("swaps in for the content, saves the switches, and Done goes back", async () => {
    play(track());
    render(<Miniplayer />);
    fireEvent.click(screen.getByRole("button", { name: "Miniplayer settings" }));
    expect(screen.getByRole("heading", { name: "Miniplayer settings" })).toBeInTheDocument();
    expect(screen.queryByTestId("mini-art")).toBeNull();
    const tint = screen.getByRole("switch", { name: "Background colour" });
    expect(tint).toHaveAttribute("aria-checked", "true");
    const side = screen.getByRole("switch", { name: "Side by side when wide" });
    expect(side).toHaveAttribute("aria-checked", "true");
    fireEvent.click(tint);
    fireEvent.click(screen.getByRole("switch", { name: "Show the queue" }));
    fireEvent.click(side);
    await waitFor(async () => {
      const s = await mock.settingsGet();
      expect(s.miniplayerTint).toBe(false);
      expect(s.miniplayerQueue).toBe(true);
      expect(s.miniplayerSideBySide).toBe(false);
    });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.getByTestId("mini-art")).toBeInTheDocument();
    await mock.settingsSet({ miniplayerTint: true, miniplayerQueue: false, miniplayerSideBySide: true });
  });

  it("background colour on fills the window; off keeps the card", () => {
    play(track());
    const { container, unmount } = render(<Miniplayer />);
    expect(container.querySelector(".mini")).toHaveClass("is-tinted");
    unmount();
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, miniplayerTint: false } }));
    const off = render(<Miniplayer />);
    expect(off.container.querySelector(".mini")).not.toHaveClass("is-tinted");
  });

  it("the Settings page has the side by side switch, on by default", async () => {
    advancedView.startOpen = true;
    try {
      await mock.settingsSet({ ...DEFAULT_SETTINGS });
      await loadSettings();
      expect(DEFAULT_SETTINGS.miniplayerSideBySide).toBe(true);
      render(<SettingsPage />);
      const row = document.querySelector<HTMLElement>('[data-setting="miniplayerSideBySide"]');
      expect(row).not.toBeNull();
      const sw = within(row!).getByRole("switch");
      expect(sw).toHaveAttribute("aria-checked", "true");
      fireEvent.click(sw);
      await waitFor(async () => expect((await mock.settingsGet()).miniplayerSideBySide).toBe(false));
    } finally {
      advancedView.startOpen = false;
      await mock.settingsSet({ miniplayerSideBySide: true });
    }
  });
});

describe("the up-next order", () => {
  it("puts your queue before the context", () => {
    const [a, b, c] = mock.fx.tracks;
    const q = upNext({ userQueue: [item(a, 1)], nextUp: [item(b, 2), item(c, 3)] }, 2);
    expect(q.map((x) => x.track.id)).toEqual([a.id, b.id]);
    expect(upNext(null)).toEqual([]);
  });

  it("a move lands within, or at the end of, your queue", () => {
    // Two of yours, then three from the context.
    expect(queueMoveFor(5, 2, 0, 2)).toBe(1); // yours, down one
    expect(queueMoveFor(5, 2, 1, 0)).toBe(0); // yours, to the top
    expect(queueMoveFor(5, 2, 0, 1)).toBeNull(); // dropped where it was
    expect(queueMoveFor(5, 2, 0, 5)).toBe(1); // below your queue: the end of it
    expect(queueMoveFor(5, 2, 3, 0)).toBe(0); // from the context into yours
    expect(queueMoveFor(5, 2, 4, 4)).toBe(2); // context songs only join the end of yours
    expect(queueMoveFor(5, 2, 9, 0)).toBeNull();
  });
});

/**
 * jsdom has no layout: elements get a size from `measure` while on the
 * page and 0 once removed. Returns a function that tells every observer
 * about every element it watches (detached ones too, as browsers do).
 */
function fakeLayout(measure: (el: HTMLElement) => { w: number; h: number }) {
  const observers: { cb: ResizeObserverCallback; els: Set<Element> }[] = [];
  class FakeRO {
    rec: { cb: ResizeObserverCallback; els: Set<Element> };
    constructor(cb: ResizeObserverCallback) {
      this.rec = { cb, els: new Set() };
      observers.push(this.rec);
    }
    observe(el: Element) {
      this.rec.els.add(el);
    }
    unobserve(el: Element) {
      this.rec.els.delete(el);
    }
    disconnect() {
      this.rec.els.clear();
    }
  }
  vi.stubGlobal("ResizeObserver", FakeRO);
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) {
    return this.isConnected ? measure(this).w : 0;
  });
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(function (this: HTMLElement) {
    return this.isConnected ? measure(this).h : 0;
  });
  return () =>
    act(() => {
      for (const o of observers) if (o.els.size) o.cb([], {} as ResizeObserver);
    });
}

/** A window of w x h; the art area gets `art` square. */
const windowOf = (w: number, h: number, art = 200) => (el: HTMLElement) =>
  el.classList.contains("mini") ? { w, h } : el.classList.contains("mini__art-area") ? { w: art, h: art } : { w: 0, h: 0 };

describe("at any window size", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("the art is square and as big as fits its area", () => {
    fakeLayout((el) => (el.classList.contains("mini__art-area") ? { w: 500, h: 300 } : { w: 0, h: 0 }));
    play(track());
    render(<Miniplayer />);
    expect(screen.getByTestId("mini-art")).toHaveStyle({ width: "300px", height: "300px" });
  });

  it("the art comes back after the settings panel (it used to stay at 0)", () => {
    const resize = fakeLayout((el) => (el.classList.contains("mini__art-area") ? { w: 400, h: 400 } : { w: 0, h: 0 }));
    play(track());
    render(<Miniplayer />);
    fireEvent.click(screen.getByRole("button", { name: "Miniplayer settings" }));
    resize();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    resize();
    const art = screen.getByTestId("mini-art");
    expect(art).toHaveStyle({ width: "400px", height: "400px" });
    // The controls are still there to point at.
    fireEvent.pointerEnter(art);
    expect(within(art).getByRole("button", { name: "Play" })).toBeInTheDocument();
  });

  const on = { queue: true, sideBySide: true };

  it("stacked, the queue shows from about 500 tall and hides below", () => {
    expect(miniLayout(340, 365, on)).toEqual({ side: false, queue: false, card: null });
    expect(miniLayout(340, QUEUE_MIN_HEIGHT - 1, on).queue).toBe(false);
    expect(miniLayout(340, QUEUE_MIN_HEIGHT, on).queue).toBe(true);
    expect(miniLayout(340, 900, { queue: false, sideBySide: true })).toEqual({ side: false, queue: false, card: null });
  });

  it("with the queue, the art is medium in a small window and grows in a big one, leaving room for rows", () => {
    const small = miniLayout(340, 520, on);
    expect(small.card).toBe(200 + 2 * CARD_PAD);
    const big = miniLayout(700, 1000, on);
    expect(big.card!).toBeGreaterThan(small.card!);
    // At least as much height is left for queue rows as the art takes.
    expect(1000 - STRIP_H - big.card!).toBeGreaterThan(big.card! - 2 * CARD_PAD);
    // Never wider than the window.
    expect(miniLayout(300, 1000, on).card!).toBeLessThanOrEqual(300 - 2 * PAD);
  });

  it("wide windows go side by side, with the art at full height", () => {
    const wide = miniLayout(900, 520, on);
    expect(wide.side).toBe(true);
    expect(wide.queue).toBe(true);
    expect(wide.card).toBe(520 - STRIP_H - 2 * PAD);
    // Not quite wide enough: stacked.
    expect(miniLayout(700, 520, on).side).toBe(false);
    // Wide but tiny: stays stacked so the column beside the art has room.
    expect(miniLayout(SIDE_MIN_WIDTH - 1, 260, on).side).toBe(false);
    // Short and wide: side by side without the queue.
    expect(miniLayout(600, SIDE_QUEUE_MIN_HEIGHT - 1, on)).toMatchObject({ side: true, queue: false });
    // The setting turns it off.
    expect(miniLayout(900, 520, { queue: true, sideBySide: false }).side).toBe(false);
    expect(900 / 520).toBeGreaterThan(SIDE_RATIO);
  });

  function withQueue(userCount = 2, nextCount = 3) {
    const ts = mock.fx.tracks;
    const user = ts.slice(1, 1 + userCount).map((t, i) => ({ ...item(t, i), source: "user" as const }));
    const next = ts.slice(1 + userCount, 1 + userCount + nextCount).map((t, i) => item(t, 10 + i));
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, miniplayerQueue: true } }));
    play(ts[0], { queue: { current: null, userQueue: user, nextUp: next, history: [], context: null } });
    return [...user, ...next];
  }
  const rows = () => within(screen.getByRole("listbox", { name: "Queue" })).getAllByRole("option");

  it("a small window hides the queue even with it on", () => {
    fakeLayout(windowOf(340, 365));
    withQueue();
    const { container } = render(<Miniplayer />);
    expect(screen.queryByRole("listbox", { name: "Queue" })).toBeNull();
    expect(container.querySelector(".mini__body")).toHaveAttribute("data-queue", "off");
  });

  it("a tall window lists the queue under the title: title, artists and length on one line", () => {
    fakeLayout(windowOf(340, 520));
    const items = withQueue();
    const { container } = render(<Miniplayer />);
    expect(screen.getByRole("heading", { name: "Queue" })).toBeInTheDocument();
    expect(rows()).toHaveLength(items.length);
    const first = rows()[0];
    expect(first).toHaveTextContent(items[0].track.title);
    expect(first).toHaveTextContent(formatDuration(items[0].track.durationMs));
    expect(container.querySelector(".mini__body")).toHaveAttribute("data-layout", "stacked");
    // A fixed-size art card, so the rest goes to rows.
    expect(container.querySelector<HTMLElement>(".mini__card")!.style.height).toBe(`${200 + 2 * CARD_PAD}px`);
  });

  it("a wide window puts the title, a progress bar and the queue beside the art", () => {
    fakeLayout(windowOf(900, 520));
    const items = withQueue();
    const { container } = render(<Miniplayer />);
    const side = container.querySelector<HTMLElement>(".mini__side")!;
    expect(side).not.toBeNull();
    expect(within(side).getByText(track().title)).toBeInTheDocument();
    expect(within(side).getByRole("progressbar", { name: "Song progress" })).toBeInTheDocument();
    expect(within(side).getAllByRole("option")).toHaveLength(items.length);
    expect(container.querySelector(".mini__body")).toHaveAttribute("data-layout", "side");
  });

  it("with side by side off, a wide window stays stacked", () => {
    fakeLayout(windowOf(900, 520));
    withQueue();
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, miniplayerQueue: true, miniplayerSideBySide: false } }));
    const { container } = render(<Miniplayer />);
    expect(container.querySelector(".mini__side")).toBeNull();
    expect(container.querySelector(".mini__body")).toHaveAttribute("data-layout", "stacked");
  });

  it("clicking a row plays it", () => {
    fakeLayout(windowOf(340, 520));
    const items = withQueue();
    const jump = vi.spyOn(player, "queueJump").mockResolvedValue(undefined);
    render(<Miniplayer />);
    fireEvent.click(rows()[2]);
    expect(jump).toHaveBeenCalledWith(items[2].itemId);
  });

  it("a row's menu: Play next, Remove from queue, Go to album", async () => {
    fakeLayout(windowOf(340, 520));
    const items = withQueue();
    const move = vi.spyOn(player, "queueMove").mockResolvedValue(undefined);
    const remove = vi.spyOn(player, "queueRemove").mockResolvedValue(undefined);
    render(<Miniplayer />);
    const menuOf = (i: number) => {
      fireEvent.contextMenu(rows()[i]);
      return screen.getByRole("menu");
    };
    fireEvent.click(within(menuOf(3)).getByRole("menuitem", { name: "Play next" }));
    expect(move).toHaveBeenCalledWith(items[3].itemId, 0);
    fireEvent.click(within(menuOf(1)).getByRole("menuitem", { name: "Remove from queue" }));
    expect(remove).toHaveBeenCalledWith(items[1].itemId);
    mock.miniplayerWent = [];
    fireEvent.click(within(menuOf(0)).getByRole("menuitem", { name: "Go to album" }));
    await waitFor(() => expect(mock.miniplayerWent).toEqual([{ kind: "album", id: items[0].track.albumId }]));
  });

  it("the keyboard: arrows move, Enter plays, Delete removes, Alt+arrows reorder", () => {
    fakeLayout(windowOf(340, 520));
    const items = withQueue();
    const jump = vi.spyOn(player, "queueJump").mockResolvedValue(undefined);
    const remove = vi.spyOn(player, "queueRemove").mockResolvedValue(undefined);
    const move = vi.spyOn(player, "queueMove").mockResolvedValue(undefined);
    render(<Miniplayer />);
    // One tab stop: the first row.
    expect(rows().map((r) => r.tabIndex)).toEqual([0, -1, -1, -1, -1]);
    rows()[0].focus();
    fireEvent.keyDown(rows()[0], { key: "ArrowDown" });
    expect(document.activeElement).toBe(rows()[1]);
    expect(rows()[1].tabIndex).toBe(0);
    fireEvent.keyDown(rows()[1], { key: "Enter" });
    expect(jump).toHaveBeenCalledWith(items[1].itemId);
    fireEvent.keyDown(rows()[1], { key: "Delete" });
    expect(remove).toHaveBeenCalledWith(items[1].itemId);
    fireEvent.keyDown(rows()[1], { key: "ArrowUp", altKey: true });
    expect(move).toHaveBeenCalledWith(items[1].itemId, 0);
  });

  it("rows drag to reorder, with a line where the song will land", () => {
    // jsdom has no DragEvent; without one the pointer position never reaches handlers.
    if (!("DragEvent" in window)) (window as unknown as { DragEvent: typeof MouseEvent }).DragEvent = class DragEvent extends MouseEvent {};
    fakeLayout(windowOf(340, 520));
    const items = withQueue();
    const move = vi.spyOn(player, "queueMove").mockResolvedValue(undefined);
    render(<Miniplayer />);
    const data = new Map<string, string>();
    const dataTransfer = {
      setData: (k: string, v: string) => data.set(k, v),
      getData: (k: string) => data.get(k) ?? "",
      get types() {
        return [...data.keys()];
      },
      effectAllowed: "",
      dropEffect: "",
    };
    // Your first song dragged to below your second (rows have no height in
    // jsdom, so any point past the top counts as the lower half).
    fireEvent.dragStart(rows()[0], { dataTransfer });
    fireEvent.dragOver(rows()[1], { dataTransfer, clientY: 5 });
    expect(rows()[1]).not.toHaveClass("is-drop-before");
    expect(rows()[2]).toHaveClass("is-drop-before");
    fireEvent.drop(rows()[1], { dataTransfer, clientY: 5 });
    expect(move).toHaveBeenCalledWith(items[0].itemId, 1);
    expect(rows().some((r) => r.className.includes("is-drop"))).toBe(false);
    // A context song dragged to the top joins your queue there.
    data.clear();
    fireEvent.dragStart(rows()[4], { dataTransfer });
    fireEvent.drop(rows()[0], { dataTransfer, clientY: 0 });
    expect(move).toHaveBeenLastCalledWith(items[4].itemId, 0);
  });

  it("an empty queue says so", () => {
    fakeLayout(windowOf(340, 520));
    withQueue(0, 0);
    render(<Miniplayer />);
    expect(screen.getByText("Nothing else is queued.")).toBeInTheDocument();
  });
});

describe("the main window", () => {
  it("opens the album the miniplayer asks for", async () => {
    const stop = connectMiniplayer();
    await act(async () => {});
    const albumId = mock.fx.tracks[0].albumId;
    await act(() => mock.miniplayerGo("album", albumId));
    await waitFor(() => {
      const r = useRouter.getState();
      expect(r.entries[r.index].route).toEqual({ name: "album", id: albumId });
    });
    stop();
  });
});
