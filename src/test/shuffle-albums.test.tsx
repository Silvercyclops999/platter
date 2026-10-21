/**
 * Shuffle albums: whole albums in a random order, each start to finish,
 * from a crate, the artist page's menu, the menu bar and the palette. A
 * press with nothing to play says so instead of playing nothing.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { CratePage } from "../pages/CratesPage";
import { player } from "../stores/player";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { MENU_ACTIONS } from "../shell/menuBar";
import { ONLINE_EXTRAS } from "./onlineExtras";

const mock = backend as MockBackend;

/** The albums in play order, one entry per run of songs. */
function runs(ids: string[]): string[] {
  const out: string[] = [];
  for (const id of ids) {
    const album = mock.fx.tracks.find((t) => t.id === id)!.albumId;
    if (out[out.length - 1] !== album) out.push(album);
  }
  return out;
}

beforeEach(async () => {
  useToasts.setState({ toasts: [] });
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS });
  useSettings.setState({ settings: null });
  await loadSettings();
});
afterEach(() => vi.restoreAllMocks());

describe("the songs for a press", () => {
  it("are whole albums, each once and in track order", async () => {
    const albums = mock.fx.albums.slice(0, 6).map((a) => a.id);
    const ids = await backend.albumShuffleTracks(albums);
    const order = runs(ids);
    expect([...order].sort()).toEqual([...albums].filter((a) => (mock.fx.tracksByAlbum.get(a) ?? []).length).sort());
    for (const a of order) {
      expect(ids.filter((id) => (mock.fx.tracksByAlbum.get(a) ?? []).some((t) => t.id === id))).toEqual((mock.fx.tracksByAlbum.get(a) ?? []).map((t) => t.id));
    }
  });

  it("leave hidden songs out, and fail plainly when nothing is left", async () => {
    const album = mock.fx.albums.find((a) => (mock.fx.tracksByAlbum.get(a.id) ?? []).length > 1)!;
    const songs = mock.fx.tracksByAlbum.get(album.id)!;
    await backend.hiddenSet({ kind: "track", id: songs[0].id, title: songs[0].title }, true);
    const ids = await backend.albumShuffleTracks([album.id]);
    expect(ids).toEqual(songs.slice(1).map((t) => t.id));
    await expect(backend.albumShuffleTracks(["no-such-album"])).rejects.toMatchObject({ kind: "invalid" });
    await backend.hiddenSet({ kind: "track", id: songs[0].id, title: songs[0].title }, false);
  });
});

describe("playing them", () => {
  it("plays one list with shuffle off, named for the queue", async () => {
    const play = vi.spyOn(backend, "playContext");
    const albums = mock.fx.albums.slice(0, 3).map((a) => a.id);
    await player.shuffleAlbums(albums);
    const args = play.mock.calls[0][0];
    expect(args.context.kind).toBe("tracks");
    expect(args.context.id).toBe("albums");
    expect(args.shuffle).toBe(false);
    expect(args.startTrackId).toBe(args.context.trackIds![0]);
    expect((await mock.playerState()).queue.context?.title).toBe("Albums on shuffle");
  });

  it("says so when there's nothing to play, and plays nothing", async () => {
    const play = vi.spyOn(backend, "playContext");
    vi.spyOn(backend, "albumShuffleTracks").mockRejectedValue({ kind: "invalid", message: "There are no albums to play here." });
    await player.shuffleAlbums(["x"]);
    expect(play).not.toHaveBeenCalled();
    expect(useToasts.getState().toasts.slice(-1)[0]).toMatchObject({ kind: "error", message: "There are no albums to play here." });
  });

  it("a lookup that fails shows its error", async () => {
    vi.spyOn(backend, "albumShuffleTracks").mockRejectedValue({ kind: "internal", message: "database is locked" });
    await player.shuffleAlbums();
    expect(useToasts.getState().toasts.slice(-1)[0]?.message).toBe("database is locked");
  });

  it("the menu bar and an empty press take the whole library", async () => {
    const ask = vi.spyOn(backend, "albumShuffleTracks");
    vi.spyOn(backend, "playContext").mockResolvedValue(undefined);
    MENU_ACTIONS.shuffleAlbums.run();
    expect(ask).toHaveBeenCalledWith([]);
  });
});

describe("a crate's page", () => {
  it("shuffles the crate's albums", async () => {
    const user = userEvent.setup();
    const shuffle = vi.spyOn(player, "shuffleAlbums").mockResolvedValue(undefined);
    const crate = (await backend.crates("label")).crates.find((c) => c.count > 1)!;
    const page = (await backend.cratePage("label", crate.key))!;
    render(<CratePage kind="label" crateKey={crate.key} />);
    await user.click(await screen.findByRole("button", { name: "Shuffle albums" }));
    expect(shuffle).toHaveBeenCalledWith(page.albums.map((a) => a.album.id));
  });
});
