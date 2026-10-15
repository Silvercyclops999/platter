import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextShuffleButton } from "../components/ShuffleButton";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { nextShuffleMode, shuffleLabel } from "../lib/shuffleMode";
import { DEFAULT_SETTINGS } from "../lib/types";
import { PlayControls, PlayerBar } from "../shell/PlayerBar";
import { QueuePanel } from "../shell/QueuePanel";
import { connectPlayer, usePlayer } from "../stores/player";
import { loadSettings, useSettings } from "../stores/settings";

const mock = backend as MockBackend;
let disconnect: () => void;

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  disconnect = connectPlayer(mock);
});
afterEach(async () => {
  disconnect();
  vi.restoreAllMocks();
  await act(() => mock.playPause().catch(() => undefined));
});

/** A playlist of one artist's songs, `n` long. */
async function playlistOf(n: number, title = "Road Trip") {
  const artist = mock.fx.artists.find((a) => mock.fx.tracks.filter((t) => t.artistId === a.id).length >= n)!;
  const songs = mock.fx.tracks.filter((t) => t.artistId === artist.id).slice(0, n);
  return mock.playlistCreate(title, songs.map((t) => t.id));
}

async function playSmart(n = 7) {
  const pl = await playlistOf(n);
  await act(async () => {
    await mock.setContextSmartShuffle(pl.id, true);
    await mock.playContext({ context: { kind: "playlist", id: pl.id } });
  });
  await waitFor(() => expect(usePlayer.getState().state?.smartShuffle).toBe(true));
  return pl;
}

/** Next, until a suggestion plays. */
async function toSuggestion() {
  for (let i = 0; i < 3 && usePlayer.getState().state?.queue.current?.source !== "smart"; i++) {
    await act(() => mock.playNext());
  }
  await waitFor(() => expect(usePlayer.getState().state?.queue.current?.source).toBe("smart"));
}

describe("shuffle labels", () => {
  it("say what the next click does, naming the playlist", () => {
    expect(nextShuffleMode("off", true)).toBe("shuffle");
    expect(nextShuffleMode("shuffle", true)).toBe("smart");
    expect(nextShuffleMode("smart", true)).toBe("off");
    expect(nextShuffleMode("shuffle", false)).toBe("off");
    expect(shuffleLabel("off", true, "Road Trip")).toBe("Turn on shuffle for Road Trip");
    expect(shuffleLabel("shuffle", true, "Road Trip")).toBe("Turn on smart shuffle for Road Trip");
    expect(shuffleLabel("smart", true, "Road Trip")).toBe("Turn off smart shuffle for Road Trip");
    expect(shuffleLabel("shuffle", false, "Road Trip")).toBe("Disable shuffle");
  });
});

describe("the player bar's shuffle button on a playlist", () => {
  it("cycles off, shuffle, smart shuffle, off", async () => {
    const user = userEvent.setup();
    const pl = await playlistOf(7);
    await act(() => mock.playContext({ context: { kind: "playlist", id: pl.id }, shuffle: false }));
    render(<PlayControls />);
    await user.click(await screen.findByRole("button", { name: "Turn on shuffle for Road Trip" }));
    await user.click(await screen.findByRole("button", { name: "Turn on smart shuffle for Road Trip" }));
    const off = await screen.findByRole("button", { name: "Turn off smart shuffle for Road Trip" });
    expect(off.querySelector(".smart-shuffle-icon")).not.toBeNull();
    await waitFor(() => expect(usePlayer.getState().state?.queue.nextUp.some((i) => i.source === "smart")).toBe(true));
    await user.click(off);
    await screen.findByRole("button", { name: "Turn on shuffle for Road Trip" });
    const s = usePlayer.getState().state!;
    expect(s.shuffle || s.smartShuffle).toBe(false);
    expect(s.queue.nextUp.some((i) => i.source === "smart")).toBe(false);
  });

  it("is plain on and off while the setting is off", async () => {
    const user = userEvent.setup();
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, smartShuffle: false } });
    const pl = await playlistOf(5);
    await act(() => mock.playContext({ context: { kind: "playlist", id: pl.id }, shuffle: false }));
    render(<PlayControls />);
    await user.click(await screen.findByRole("button", { name: "Enable shuffle" }));
    await user.click(await screen.findByRole("button", { name: "Disable shuffle" }));
    await screen.findByRole("button", { name: "Enable shuffle" });
  });
});

describe("the page's shuffle toggle", () => {
  it("is the same control, and sets how a playlist that isn't playing plays next", async () => {
    const user = userEvent.setup();
    const pl = await playlistOf(5, "Later");
    render(<ContextShuffleButton id={pl.id} kind="playlist" name={pl.title} />);
    await user.click(screen.getByRole("button", { name: "Turn on shuffle for Later" }));
    await user.click(await screen.findByRole("button", { name: "Turn on smart shuffle for Later" }));
    await screen.findByRole("button", { name: "Turn off smart shuffle for Later" });
    expect(usePlayer.getState().state?.smartContexts).toContain(pl.id);
    await act(() => mock.playContext({ context: { kind: "playlist", id: pl.id } }));
    await waitFor(() => expect(usePlayer.getState().state?.smartShuffle).toBe(true));
  });

  it("is on and off for albums", async () => {
    const user = userEvent.setup();
    const album = mock.fx.albums[2];
    render(<ContextShuffleButton id={album.id} kind="album" name={album.title} />);
    await user.click(screen.getByRole("button", { name: "Enable shuffle" }));
    await user.click(await screen.findByRole("button", { name: "Disable shuffle" }));
    await screen.findByRole("button", { name: "Enable shuffle" });
    expect(usePlayer.getState().state?.smartContexts ?? []).not.toContain(album.id);
  });
});

describe("suggestions", () => {
  it("carry a sparkle before the artist in the queue, under the playlist's heading", async () => {
    await playSmart();
    render(<QueuePanel />);
    const next = await screen.findByRole("region", { name: "Next from: Road Trip" });
    const rows = within(next).getAllByRole("row", { name: /smart shuffle suggestion/ });
    expect(rows).toHaveLength(3);
    expect(rows[0].querySelector(".track-row__sub .queue-smart")).not.toBeNull();
  });

  it("show Remove suggestion while one plays, which skips it and picks new ones", async () => {
    const user = userEvent.setup();
    await playSmart();
    render(<PlayerBar />);
    expect(screen.queryByRole("button", { name: "Remove suggestion" })).toBeNull();
    await toSuggestion();
    const remove = vi.spyOn(backend, "smartSuggestionRemove");
    await user.click(await screen.findByRole("button", { name: "Remove suggestion" }));
    expect(remove).toHaveBeenCalled();
    await waitFor(() => expect(usePlayer.getState().state?.queue.current?.source).toBe("context"));
    expect(screen.queryByRole("button", { name: "Remove suggestion" })).toBeNull();
  });

  it("Add to <playlist> adds it to the playlist and drops the badge", async () => {
    const user = userEvent.setup();
    const pl = await playSmart();
    render(<PlayerBar />);
    await toSuggestion();
    const song = usePlayer.getState().state!.queue.current!;
    const add = vi.spyOn(backend, "playlistAdd");
    await user.click(await screen.findByRole("button", { name: "Add to Road Trip" }));
    expect(add).toHaveBeenCalledWith(pl.id, [song.track.id]);
    await waitFor(() => expect(usePlayer.getState().state?.queue.current?.source).toBe("context"));
    expect(usePlayer.getState().state?.queue.current?.itemId).toBe(song.itemId);
    expect(screen.queryByRole("button", { name: "Add to Road Trip" })).toBeNull();
  });

  it("Add to Liked Songs likes it", async () => {
    const user = userEvent.setup();
    await act(async () => {
      await mock.setContextSmartShuffle("liked", true);
      await mock.playContext({ context: { kind: "liked", id: "liked" } });
    });
    await waitFor(() => expect(usePlayer.getState().state?.smartShuffle).toBe(true));
    render(<PlayerBar />);
    await toSuggestion();
    const likes = vi.spyOn(backend, "likesSet");
    await user.click(await screen.findByRole("button", { name: "Add to Liked Songs" }));
    const song = usePlayer.getState().state!.queue.current!;
    expect(likes).toHaveBeenCalledWith([song.track.id], true);
    await waitFor(() => expect(usePlayer.getState().state?.queue.current?.source).toBe("context"));
  });
});
