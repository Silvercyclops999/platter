/**
 * Volume on another device: players have no mute, so dragging the slider
 * to the bottom leaves it silent and unmuting brings back the old level.
 */
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { RemoteCommand, RemoteSession } from "../lib/types";
import { connectPlayer, player, usePlayer } from "../stores/player";

const mock = backend as MockBackend;

let disconnect: () => void;
beforeAll(async () => {
  disconnect = connectPlayer(mock);
  await waitFor(() => expect(usePlayer.getState().state).toBeTruthy());
});
afterAll(() => {
  usePlayer.getState().setRemote({ session: null, stoppedOn: null });
  disconnect();
});

describe("volume on another device", () => {
  it("dragged to 0 stays at 0, and unmuting brings back the level from before", async () => {
    let session: RemoteSession = {
      playerId: "den",
      playerName: "Den TV",
      kind: "tv",
      status: "playing",
      track: mock.fx.tracks[0],
      positionMs: 0,
      durationMs: 200_000,
      volume: 0.6,
      shuffle: false,
      repeat: "off",
    };
    const sent: number[] = [];
    vi.spyOn(backend, "remoteCommand").mockImplementation(async (c: RemoteCommand) => {
      if (c.kind === "volume") {
        sent.push(c.volume);
        session = { ...session, volume: c.volume };
      }
      return { session, stoppedOn: null };
    });
    usePlayer.getState().setRemote({ session, stoppedOn: null });
    await player.volumeInput(0.2);
    await player.volumeInput(0);
    await player.volumeCommit(0);
    expect(sent[sent.length - 1]).toBe(0);
    expect(session.volume).toBe(0);
    await player.toggleMute();
    expect(sent[sent.length - 1]).toBe(0.6);
  });
});
