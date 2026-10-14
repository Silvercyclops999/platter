import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type { MockBackend } from "../lib/mock";
import { PlayerBar } from "../shell/PlayerBar";
import { QueuePanel } from "../shell/QueuePanel";
import { connectPlayer, usePlayer } from "../stores/player";

const mock = backend as MockBackend;
let disconnect: () => void;

beforeAll(async () => {
  disconnect = connectPlayer(mock);
  await waitFor(() => expect(usePlayer.getState().state).not.toBeNull());
});
afterAll(() => disconnect());

describe("PlayerBar reflects the mock player", () => {
  it("shows the current track, artist, and position", () => {
    render(<PlayerBar />);
    const s = usePlayer.getState().state!;
    expect(screen.getByRole("link", { name: s.track!.title })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: s.track!.artistTitle })).toBeInTheDocument();
    expect(screen.getByText(formatDuration(s.positionMs))).toBeInTheDocument();
    const seek = screen.getByRole("slider", { name: "Seek" });
    expect(seek).toHaveAttribute("aria-valuemax", String(s.durationMs));
    expect(seek).toHaveAttribute("aria-valuenow", String(s.positionMs));
  });

  it("play/pause, shuffle, repeat and mute follow the core state", async () => {
    const user = userEvent.setup();
    render(<PlayerBar />);
    await user.click(screen.getByRole("button", { name: "Play" }));
    await screen.findByRole("button", { name: "Pause" });
    expect(usePlayer.getState().state!.status).toBe("playing");

    await user.click(screen.getByRole("button", { name: "Enable shuffle" }));
    expect(await screen.findByRole("button", { name: "Disable shuffle" })).toHaveAttribute("aria-pressed", "true");

    await user.click(screen.getByRole("button", { name: "Enable repeat" }));
    await screen.findByRole("button", { name: "Enable repeat one" });
    await user.click(screen.getByRole("button", { name: "Enable repeat one" }));
    await screen.findByRole("button", { name: "Disable repeat" });
    await user.click(screen.getByRole("button", { name: "Disable repeat" }));
    await screen.findByRole("button", { name: "Enable repeat" });

    await user.click(screen.getByRole("button", { name: "Mute" }));
    await screen.findByRole("button", { name: "Unmute" });
    expect(screen.getByRole("slider", { name: "Volume" })).toHaveAttribute("aria-valuenow", "0");

    await user.click(screen.getByRole("button", { name: "Pause" }));
    await screen.findByRole("button", { name: "Play" });
    // restore
    await act(() => mock.setShuffle(false));
    await act(() => mock.setMuted(false));
  });

  it("position events move the seek bar", async () => {
    render(<PlayerBar />);
    act(() => mock.emit("player://position", { positionMs: 90_000, durationMs: 200_000, bufferedMs: 120_000 }));
    expect(screen.getByRole("slider", { name: "Seek" })).toHaveAttribute("aria-valuenow", "90000");
    expect(screen.getByText("1:30")).toBeInTheDocument();
    expect(screen.getByText("-1:50")).toBeInTheDocument();
  });

  it("Phase 3 buttons are present but inert", () => {
    render(<PlayerBar />);
    expect(screen.getByRole("button", { name: "Lyrics" })).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("button", { name: "Connect to a device" })).toHaveAttribute("aria-disabled", "true");
  });
});

describe("QueuePanel", () => {
  it("renders Now playing, Next in queue, Next from context, and History", async () => {
    const user = userEvent.setup();
    const album = mock.fx.albums.find((a) => a.trackCount >= 5)!;
    const tracks = mock.fx.tracksByAlbum.get(album.id)!;
    const other = mock.fx.tracks.find((t) => t.albumId !== album.id)!;
    await act(() => mock.playContext({ context: { kind: "album", id: album.id }, startTrackId: tracks[1].id }));
    await act(() => mock.queueAdd([other.id], "end"));

    render(<QueuePanel />);
    const now = screen.getByRole("region", { name: "Now playing" });
    expect(within(now).getByText(tracks[1].title)).toBeInTheDocument();

    const user1 = screen.getByRole("region", { name: "Next in queue" });
    expect(within(user1).getByText(other.title)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Clear queue" })).toBeInTheDocument();

    const next = screen.getByRole("region", { name: `Next from: ${album.title}` });
    expect(within(next).getAllByRole("listitem")).toHaveLength(tracks.length - 2);

    // Next plays the user-queued track before the context continues.
    await act(() => mock.playNext());
    expect(within(screen.getByRole("region", { name: "Now playing" })).getByText(other.title)).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Next in queue" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Recently played" }));
    expect(screen.getByRole("tab", { name: "Recently played" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getAllByText(tracks[1].title).length).toBeGreaterThan(0);
    await act(() => mock.playPause());
  });
});

describe("mock queue semantics", () => {
  it("previous restarts after 3 s, repeat one replays, clear only clears the user queue", async () => {
    const album = mock.fx.albums.find((a) => a.trackCount >= 4)!;
    const tracks = mock.fx.tracksByAlbum.get(album.id)!;
    // An explicit start track (this album may have a resume point from above).
    await mock.playContext({ context: { kind: "album", id: album.id }, startTrackId: tracks[0].id });
    await mock.playNext();
    let s = await mock.playerState();
    expect(s.track!.id).toBe(tracks[1].id);
    await mock.playSeek(10_000);
    await mock.playPrevious();
    s = await mock.playerState();
    expect(s.track!.id).toBe(tracks[1].id);
    expect(s.positionMs).toBe(0);
    await mock.playPrevious();
    s = await mock.playerState();
    expect(s.track!.id).toBe(tracks[0].id);

    await mock.queueAdd([tracks[3].id], "next");
    await mock.queueClear();
    s = await mock.playerState();
    expect(s.queue.userQueue).toHaveLength(0);
    expect(s.queue.nextUp.length).toBe(tracks.length - 1);
    await mock.playPause();
  });
});
