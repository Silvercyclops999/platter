/**
 * Long songs (20 minutes or more) pick up where they were left: the core
 * says where it resumed, the window says so once with Start over, and Start
 * over goes back to the top.
 */
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { connectPlayer, usePlayer, withRemote } from "../stores/player";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
let disconnect: () => void;

beforeAll(async () => {
  disconnect = connectPlayer(mock);
  await waitFor(() => expect(usePlayer.getState().state).not.toBeNull());
});
afterAll(() => disconnect());
afterEach(() => {
  vi.restoreAllMocks();
  useToasts.setState({ toasts: [] });
});

const set = { ...mock.fx.tracks[0], id: "long-set", durationMs: 75 * 60 * 1000 };

function resumeAt(ms: number | null, track = set) {
  const s = usePlayer.getState().state!;
  act(() => usePlayer.getState().set({ ...s, track, durationMs: track.durationMs, positionMs: ms ?? 0, resumedAtMs: ms }));
}

describe("long songs pick up where they were left", () => {
  it("says where it resumed, once, and Start over goes back to the top", async () => {
    const seek = vi.spyOn(backend, "playSeek");
    const user = userEvent.setup();
    render(<Toaster />);
    resumeAt(41 * 60 * 1000 + 12_000);
    expect(await screen.findByText("Resumed at 41:12")).toBeInTheDocument();
    // The same state again doesn't say it twice.
    resumeAt(41 * 60 * 1000 + 12_000);
    expect(useToasts.getState().toasts).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: "Start over" }));
    expect(seek).toHaveBeenCalledWith(0);
  });

  it("Start over only restarts the song it was said for", async () => {
    const seek = vi.spyOn(backend, "playSeek");
    const user = userEvent.setup();
    render(<Toaster />);
    resumeAt(41 * 60 * 1000);
    await screen.findByText("Resumed at 41:00");
    // Next was pressed while the note still shows.
    resumeAt(null, { ...set, id: "other" });
    await user.click(screen.getByRole("button", { name: "Start over" }));
    expect(seek).not.toHaveBeenCalled();
  });

  it("is this device's own: playing elsewhere says nothing", () => {
    const s = usePlayer.getState().state!;
    expect(withRemote({ ...s, resumedAtMs: 5_000_000 }, { status: "playing", track: set, positionMs: 0, durationMs: set.durationMs, shuffle: false, repeat: "off" } as never)?.resumedAtMs).toBeNull();
  });

  it("says nothing for a song started from the top", () => {
    resumeAt(null);
    expect(useToasts.getState().toasts).toHaveLength(0);
  });
});
