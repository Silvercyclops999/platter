/**
 * Change the mood (Turn it): the sheet lists the library's moods; picking
 * one puts its songs up next, leaving out what's already queued, with Undo
 * that takes them back out; and every failure says so without touching the
 * queue.
 */
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { PlayerState, QueueItem, Track } from "../lib/types";
import { TurnSheet, useTurnChoices } from "../phone/TurnSheet";
import { usePhoneUi } from "../phone/phoneUi";
import { usePlayer } from "../stores/player";
import { useToasts } from "../stores/toasts";
import { queuedIds, turnTo, undoTurn } from "../stores/turn";

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts;
const last = () => toasts()[toasts().length - 1];

function track(id: string): Track {
  return { id, title: `Song ${id}`, albumTitle: "A", artistTitle: "B", thumb: null } as unknown as Track;
}
const item = (id: string, itemId = `q-${id}`): QueueItem => ({ itemId, track: track(id), source: "user" });

function playing(queue: Partial<{ userQueue: QueueItem[]; nextUp: QueueItem[]; history: QueueItem[] }> = {}) {
  const state = {
    status: "playing",
    track: track("now"),
    positionMs: 0,
    durationMs: 1000,
    queue: { current: item("now"), userQueue: [], nextUp: [], history: [], ...queue },
  } as unknown as PlayerState;
  act(() => usePlayer.setState({ state }));
}

beforeEach(() => {
  act(() => useToasts.setState({ toasts: [] }));
  playing();
});
afterEach(() => vi.restoreAllMocks());

describe("turning", () => {
  it("knows every song the queue holds", () => {
    playing({ userQueue: [item("u1")], nextUp: [item("n1")], history: [item("h1")] });
    expect(queuedIds(usePlayer.getState().state?.queue).sort()).toEqual(["h1", "n1", "now", "u1"]);
    expect(queuedIds(null)).toEqual([]);
  });

  it("puts the mood's songs up next, leaving out what's queued, with Undo", async () => {
    playing({ nextUp: [item("n1")] });
    const songs = vi.spyOn(backend, "turnSongs").mockResolvedValue(["a", "b", "c"]);
    const add = vi.spyOn(backend, "queueAdd").mockResolvedValue();
    expect(await turnTo("Mellow")).toBe(true);
    expect(songs).toHaveBeenCalledWith("now", "Mellow", expect.arrayContaining(["now", "n1"]));
    expect(add).toHaveBeenCalledWith(["a", "b", "c"], "next");
    const t = last()!;
    expect(t.message).toBe("Turning mellow: 3 songs up next");
    expect(t.action?.label).toBe("Undo");
  });

  it("Undo takes back only the turn's songs still waiting", () => {
    playing({ userQueue: [item("a", "q1"), item("x", "q2"), item("b", "q3")] });
    const remove = vi.spyOn(backend, "queueRemove").mockResolvedValue();
    undoTurn(["a", "b", "c"]);
    expect(remove.mock.calls.map((c) => c[0])).toEqual(["q1", "q3"]);
  });

  it("says so when every song of the mood is already queued", async () => {
    vi.spyOn(backend, "turnSongs").mockResolvedValue([]);
    const add = vi.spyOn(backend, "queueAdd");
    expect(await turnTo("Upbeat")).toBe(false);
    expect(add).not.toHaveBeenCalled();
    expect(last()?.message).toBe("Every upbeat song is already in your queue");
  });

  it("says so when the songs can't be picked, and leaves the queue alone", async () => {
    vi.spyOn(backend, "turnSongs").mockRejectedValue(new Error("database is locked"));
    const add = vi.spyOn(backend, "queueAdd");
    expect(await turnTo("Mellow")).toBe(false);
    expect(add).not.toHaveBeenCalled();
    expect(last()).toMatchObject({ kind: "error", message: "Couldn't turn the music just now. Try again in a moment." });
  });

  it("says so when the queue refuses the songs", async () => {
    vi.spyOn(backend, "turnSongs").mockResolvedValue(["a"]);
    vi.spyOn(backend, "queueAdd").mockRejectedValue(new Error("offline"));
    expect(await turnTo("Mellow")).toBe(false);
    expect(last()).toMatchObject({ kind: "error", message: "Couldn't add the songs to your queue. Try again in a moment." });
  });

  it("does nothing with nothing playing", async () => {
    act(() => usePlayer.setState({ state: null }));
    const songs = vi.spyOn(backend, "turnSongs");
    expect(await turnTo("Mellow")).toBe(false);
    expect(songs).not.toHaveBeenCalled();
  });
});

describe("the sheet", () => {
  it("lists the moods and turns toward the one picked, then closes", async () => {
    const user = userEvent.setup();
    const turn = vi.spyOn(backend, "turnSongs").mockResolvedValue(["a", "b"]);
    vi.spyOn(backend, "queueAdd").mockResolvedValue();
    act(() => usePhoneUi.setState({ sheets: [{ kind: "turn", track: track("now"), choices: [] }] }));
    render(
      <TurnSheet
        track={track("now")}
        choices={[
          { mood: "Mellow", songs: 40 },
          { mood: "Upbeat", songs: 20 },
        ]}
      />,
    );
    expect(screen.getByRole("list", { name: "Moods to turn Song now toward" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Upbeat" }));
    await waitFor(() => expect(usePhoneUi.getState().sheets).toEqual([]));
    expect(turn).toHaveBeenCalledWith("now", "Upbeat", expect.any(Array));
  });

  it("offers nothing when the moods can't be read", async () => {
    vi.spyOn(backend, "turnChoices").mockRejectedValue(new Error("offline"));
    function Probe() {
      const list = useTurnChoices("now");
      return <p>{list.length} moods</p>;
    }
    render(<Probe />);
    await waitFor(() => expect(screen.getByText("0 moods")).toBeInTheDocument());
  });

  it("the demo library offers moods for a song it has", async () => {
    const id = (await mock.albumPage((await mock.albums({ sort: "name", offset: 0, limit: 1 })).items[0].id)).tracks[0].id;
    expect((await backend.turnChoices(id)).length).toBeGreaterThan(0);
    expect(await backend.turnChoices("nope")).toEqual([]);
  });
});
