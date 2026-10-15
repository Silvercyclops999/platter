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
import { Miniplayer, upNext } from "../shell/Miniplayer";
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
