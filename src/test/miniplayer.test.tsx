/**
 * The miniplayer: opening and closing it from the player bar and the app
 * menu (and the window failing to open), its controls on hover, its own
 * settings panel, and which window the page is.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type PlayerState, type QueueItem, type Track } from "../lib/types";
import { Miniplayer, miniLayout, QUEUE_ROWS, upNext } from "../shell/Miniplayer";
import { PlayerBar } from "../shell/PlayerBar";
import { AppMenu } from "../shell/TopBar";
import { connectMiniplayer, isMiniplayerWindow, MINIPLAYER_FAILED, useMiniplayer } from "../stores/miniplayer";
import { player, usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
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

  it("nothing playing says so", () => {
    act(() => usePlayer.setState({ state: null, positionMs: 0, durationMs: 0 }));
    render(<Miniplayer />);
    expect(screen.getByText("Nothing playing")).toBeInTheDocument();
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
    fireEvent.click(tint);
    fireEvent.click(screen.getByRole("switch", { name: "Show the queue" }));
    await waitFor(async () => {
      const s = await mock.settingsGet();
      expect(s.miniplayerTint).toBe(false);
      expect(s.miniplayerQueue).toBe(true);
    });
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.getByTestId("mini-art")).toBeInTheDocument();
    await mock.settingsSet({ miniplayerTint: true, miniplayerQueue: false });
  });

  it("background colour off leaves the card untinted", () => {
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, miniplayerTint: false } }));
    play(track());
    const { container } = render(<Miniplayer />);
    expect(container.querySelector(".mini")).not.toHaveClass("is-tinted");
  });

  it("the up-next list puts your queue before the context", () => {
    const [a, b, c] = mock.fx.tracks;
    const q = upNext({ userQueue: [item(a, 1)], nextUp: [item(b, 2), item(c, 3)] }, 2);
    expect(q.map((x) => x.track.id)).toEqual([a.id, b.id]);
    expect(upNext(null)).toEqual([]);
  });
});

describe("at any window size", () => {
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
  const artArea = (w: number, h: number) => (el: HTMLElement) => (el.classList.contains("mini__art-area") ? { w, h } : { w: 0, h: 0 });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("the art is square and as big as fits its area", () => {
    fakeLayout(artArea(500, 300));
    play(track());
    render(<Miniplayer />);
    expect(screen.getByTestId("mini-art")).toHaveStyle({ width: "300px", height: "300px" });
  });

  it("the art comes back after the settings panel (it used to stay at 0)", () => {
    const resize = fakeLayout(artArea(400, 400));
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

  it("small and tall windows stack; wide ones go side by side", () => {
    expect(miniLayout(340, 365, false)).toEqual({ wide: false, queueRows: 0 });
    expect(miniLayout(600, 400, false).wide).toBe(true);
    expect(miniLayout(900, 900, false).wide).toBe(false);
    // Wider than tall but too small to split stays stacked.
    expect(miniLayout(500, 300, false).wide).toBe(false);
  });

  it("the queue uses spare height, more rows in bigger windows", () => {
    expect(miniLayout(340, 365, true).queueRows).toBe(0);
    expect(miniLayout(340, 480, true).queueRows).toBe(QUEUE_ROWS);
    expect(miniLayout(340, 900, true).queueRows).toBe(8);
    const wide = miniLayout(900, 600, true);
    expect(wide.wide).toBe(true);
    expect(wide.queueRows).toBeGreaterThan(QUEUE_ROWS);
  });

  it("a wide window puts the title and up next beside the art", () => {
    fakeLayout((el) => (el.classList.contains("mini") ? { w: 900, h: 600 } : { w: 300, h: 300 }));
    const [a, b] = mock.fx.tracks;
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, miniplayerQueue: true } }));
    play(a, { queue: { current: null, userQueue: [item(b, 1)], nextUp: [], history: [], context: null } });
    const { container } = render(<Miniplayer />);
    const side = container.querySelector<HTMLElement>(".mini__side");
    expect(side).not.toBeNull();
    expect(within(side!).getByText(a.title)).toBeInTheDocument();
    expect(within(side!).getByRole("region", { name: "Up next" })).toBeInTheDocument();
  });
});
