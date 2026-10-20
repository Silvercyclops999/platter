/**
 * How Listen on refreshes: rows show as each source answers, the skeleton
 * only while nothing is known, refreshes never overlap, and a phone that's
 * asleep stays listed.
 */
import { act, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend, type Backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { RemotePlayer } from "../lib/types";
import { ListenOn, playerSub } from "../shell/ListenOn";
import {
  loadPlayers,
  loadingPlayers,
  REMOTE_REFRESH_MS,
  resetPlayerLoads,
  useRemote,
  watchPlayers,
} from "../stores/remote";

const mock = backend as MockBackend;

const row = (id: string, name: string, extra: Partial<RemotePlayer> = {}): RemotePlayer => ({
  id,
  name,
  product: "Platter",
  kind: "computer",
  playing: null,
  online: true,
  via: "platter",
  needsPassword: false,
  unsupported: null,
  unreachable: false,
  ...extra,
});

/** A backend whose device search answers only when the test says so. */
function slowBackend() {
  const answers: ((p: RemotePlayer[]) => void)[] = [];
  let onPlayers: ((p: RemotePlayer[]) => void) | null = null;
  const b = {
    remotePlayers: vi.fn(() => new Promise<RemotePlayer[]>((res) => answers.push(res))),
    listen: vi.fn((event: string, cb: (p: RemotePlayer[]) => void) => {
      if (event === "remote://players") onPlayers = cb;
      return Promise.resolve(() => {
        onPlayers = null;
      });
    }),
  };
  return {
    b: b as unknown as Backend,
    calls: () => b.remotePlayers.mock.calls.length,
    answer: (p: RemotePlayer[]) => answers.shift()?.(p),
    partial: (p: RemotePlayer[]) => onPlayers?.(p),
  };
}

beforeEach(() => {
  resetPlayerLoads();
  useRemote.setState({ players: null, busy: undefined, password: null });
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetPlayerLoads();
});

describe("loading the players", () => {
  it("joins a load already running instead of starting another", async () => {
    const s = slowBackend();
    const one = loadPlayers(s.b);
    const two = loadPlayers(s.b);
    expect(s.calls()).toBe(1);
    expect(loadingPlayers()).toBe(true);
    s.answer([row("mac", "Studio Mac")]);
    await Promise.all([one, two]);
    expect(useRemote.getState().players?.map((p) => p.id)).toEqual(["mac"]);
    expect(loadingPlayers()).toBe(false);
  });

  it("runs once more after a load that a setting change asked to redo", async () => {
    const s = slowBackend();
    const one = loadPlayers(s.b);
    void loadPlayers(s.b, { fresh: true });
    void loadPlayers(s.b, { fresh: true });
    s.answer([]);
    await vi.waitFor(() => expect(s.calls()).toBe(2));
    s.answer([row("mac", "Studio Mac")]);
    await one;
    expect(s.calls()).toBe(2);
    expect(useRemote.getState().players?.map((p) => p.id)).toEqual(["mac"]);
  });

  it("a search that fails shows the empty list rather than loading forever", async () => {
    const b = { remotePlayers: () => Promise.reject(new Error("offline")) } as unknown as Backend;
    await loadPlayers(b);
    expect(useRemote.getState().players).toEqual([]);
  });
});

describe("watching while the picker is open", () => {
  it("shows rows as they're found, before the search finishes", async () => {
    const s = slowBackend();
    const stop = watchPlayers(s.b);
    await Promise.resolve();
    act(() => s.partial([row("mac", "Studio Mac")]));
    expect(useRemote.getState().players?.map((p) => p.id)).toEqual(["mac"]);
    act(() => s.partial([row("mac", "Studio Mac"), row("amp", "Desk", { via: "plex", product: "Plexamp" })]));
    s.answer([row("mac", "Studio Mac"), row("amp", "Desk", { via: "plex", product: "Plexamp" })]);
    await vi.waitFor(() => expect(loadingPlayers()).toBe(false));
    expect(useRemote.getState().players).toHaveLength(2);
    stop();
  });

  it("skips the timed refresh while one is still running", async () => {
    vi.useFakeTimers();
    const s = slowBackend();
    const stop = watchPlayers(s.b);
    expect(s.calls()).toBe(1);
    // The search takes longer than the refresh interval.
    await vi.advanceTimersByTimeAsync(REMOTE_REFRESH_MS * 3);
    expect(s.calls()).toBe(1);
    s.answer([]);
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(REMOTE_REFRESH_MS);
    expect(s.calls()).toBe(2);
    stop();
  });
});

describe("the Listen on section while searching", () => {
  it("shows the skeleton only until the first row arrives", async () => {
    vi.spyOn(mock, "remotePlayers").mockImplementation(() => new Promise(() => {}));
    const { container } = render(<ListenOn />);
    const section = await screen.findByRole("region", { name: "Listen on" });
    expect(container.querySelector(".skeleton, [class*='skeleton']")).not.toBeNull();
    act(() => mock.emit("remote://players", [row("mac", "Studio Mac")]));
    expect(await within(section).findByRole("button", { name: "Play on Studio Mac" })).toBeInTheDocument();
    expect(container.querySelector(".skeleton, [class*='skeleton']")).toBeNull();
    expect(within(section).queryByText(/No devices found/)).toBeNull();
  });

  it("lists a phone that's asleep, saying how to wake it", async () => {
    const phone = row("iph", "Adam's iPhone", { kind: "phone", online: false, asleep: true });
    expect(playerSub(phone)).toBe("Asleep • Open Platter on it to wake it");
    vi.spyOn(mock, "remotePlayers").mockResolvedValue([phone]);
    render(<ListenOn />);
    const section = await screen.findByRole("region", { name: "Listen on" });
    const btn = await within(section).findByRole("button", { name: "Play on Adam's iPhone (asleep, open Platter on it to wake it)" });
    expect(btn).toHaveTextContent("Asleep");
    // Asleep is not a firewall problem: no note about it.
    expect(within(section).queryByRole("note")).toBeNull();
  });
});
