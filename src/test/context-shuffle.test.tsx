import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { AlbumPage } from "../pages/AlbumPage";
import { usePlayer } from "../stores/player";

const mock = backend as MockBackend;

afterEach(() => vi.restoreAllMocks());

describe("album page shuffle toggle", () => {
  it("shows that album's own shuffle state and sets it without playing", async () => {
    const user = userEvent.setup();
    // An album that isn't the playing context.
    const playing = (await mock.playerState()).queue.context?.id;
    const album = mock.fx.albums.find((a) => a.id !== playing)!;
    const base = await mock.playerState();
    act(() => usePlayer.getState().set({ ...base, shuffle: false, shuffledContexts: [album.id] }));

    const setCtx = vi.spyOn(backend, "setContextShuffle");
    const setGlobal = vi.spyOn(backend, "setShuffle");
    const play = vi.spyOn(backend, "playContext");
    render(<AlbumPage id={album.id} />);
    const toggle = await screen.findByRole("button", { name: "Disable shuffle" });
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    await user.click(toggle);
    expect(setCtx).toHaveBeenCalledWith(album.id, false);
    expect(setGlobal).not.toHaveBeenCalled();

    // Play doesn't pass a shuffle flag: the core applies this album's own.
    await user.click(screen.getByRole("button", { name: `Play ${album.title}` }));
    expect(play).toHaveBeenCalledWith({ context: { kind: "album", id: album.id }, startTrackId: undefined, shuffle: undefined });
  });
});
