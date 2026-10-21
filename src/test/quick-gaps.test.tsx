/**
 * Small library touches: the time after the seek bar switches between time
 * left and length, "Add to <last playlist>" in one click, back to the
 * playing song, a new smart playlist key, and Ctrl/Cmd+F finding in the
 * page you're on.
 */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { PlaylistDialogs } from "../components/PlaylistDialogs";
import { addToPlaylistItems } from "../components/PlaylistPicker";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Playlist } from "../lib/types";
import { PlaylistPage } from "../pages/PlaylistPage";
import { goToPlaying, revealRow } from "../shell/goToPlaying";
import { openPageFind, usePageReveal } from "../shell/pageFind";
import { SeekBar } from "../shell/PlayerBar";
import { useShortcuts } from "../shell/useShortcuts";
import { connectApp, useApp } from "../stores/app";
import { useCurate } from "../stores/curate";
import { usePlayer } from "../stores/player";
import { addToPlaylist, lastPlaylistId, usePlaylistUi } from "../stores/playlists";
import { nav, useRouter } from "../stores/router";
import { resetSeekEnd } from "../stores/seekTime";
import { useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

let disconnect: () => void;
beforeAll(() => {
  disconnect = connectApp(mock);
});
afterAll(() => disconnect());

beforeEach(async () => {
  localStorage.clear();
  resetSeekEnd();
  useRouter.getState().reset();
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null, paletteOpen: false, shortcutsOpen: false });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS, playlistAskDuplicates: false } });
  usePlaylistUi.setState({ duplicate: null, removing: null, deleting: null, editing: null, renameId: null });
  await mock.settingsSet({ playlistAskDuplicates: false });
  const lists = await backend.playlists();
  act(() => useApp.getState().setPlaylists(lists));
});

function Keys() {
  useShortcuts();
  return null;
}

async function playing(durationMs?: number) {
  const state = await mock.playerState();
  act(() => {
    usePlayer.getState().set(state);
    if (durationMs != null) usePlayer.setState({ durationMs });
  });
  return state;
}

describe("the time after the seek bar", () => {
  it("shows the time left, switches to the length on a click, and remembers", async () => {
    await playing(200_000);
    const { unmount } = render(<SeekBar />);
    const end = screen.getByRole("button", { name: /^-\d+:\d\d, time left\./ });
    expect(end.textContent).toMatch(/^-/);
    fireEvent.click(end);
    const length = screen.getByRole("button", { name: "3:20, song length. Show the time left" });
    expect(length.textContent).toBe("3:20");
    unmount();
    resetSeekEnd();
    render(<SeekBar />);
    expect(screen.getByRole("button", { name: /song length/ })).toBeInTheDocument();
  });

  it("a song of unknown length shows only a plain time", async () => {
    await playing(0);
    render(<SeekBar />);
    expect(screen.queryByRole("button", { name: /time left|song length/ })).toBeNull();
  });
});

describe("add to the last playlist", () => {
  const editable = () => (useApp.getState().playlists ?? []).filter((p) => !p.smart);

  it("offers the playlist last added to, first, after one add", async () => {
    const target: Playlist = editable()[0];
    expect(addToPlaylistItems(() => ["t"]).map((i) => i.label)).toEqual(["Add to playlist"]);
    await addToPlaylist(target, [mock.fx.tracks[0].id]);
    expect(lastPlaylistId()).toBe(target.id);
    const items = addToPlaylistItems(() => [mock.fx.tracks[1].id]);
    expect(items.map((i) => i.label)).toEqual([`Add to ${target.title}`, "Add to playlist"]);
    const add = vi.spyOn(backend, "playlistAdd");
    items[0].onSelect?.();
    await waitFor(() => expect(add).toHaveBeenCalledWith(target.id, [mock.fx.tracks[1].id]));
  });

  it("a deleted, smart or excluded last playlist is not offered", async () => {
    const target = editable()[0];
    await addToPlaylist(target, [mock.fx.tracks[0].id]);
    expect(addToPlaylistItems(() => ["t"], { exclude: target.id })).toHaveLength(1);
    act(() => useApp.getState().setPlaylists((useApp.getState().playlists ?? []).filter((p) => p.id !== target.id)));
    expect(addToPlaylistItems(() => ["t"])).toHaveLength(1);
    const smart = (await backend.playlists()).find((p) => p.smart)!;
    localStorage.setItem("platter.lastPlaylist.v1", smart.id);
    expect(addToPlaylistItems(() => ["t"])).toHaveLength(1);
  });
});

describe("back to the playing song", () => {
  it("opens the playing list and pulses the playing row", async () => {
    const state = await playing();
    const track = state.track!;
    act(() =>
      usePlayer.getState().set({ ...state, queue: { ...state.queue, context: { kind: "playlist", id: "p9", title: "Road" } } }),
    );
    render(
      <div className="main__scroll">
        <div className="track-row" tabIndex={-1} data-track-id={track.id} />
      </div>,
    );
    goToPlaying();
    expect(route()).toEqual({ name: "playlist", id: "p9" });
    const row = document.querySelector(".track-row")!;
    expect(row).toHaveClass("track-row--found");
    expect(row).toHaveFocus();
  });

  it("a list without a page of its own goes to the song's album", async () => {
    const state = await playing();
    act(() => usePlayer.getState().set({ ...state, queue: { ...state.queue, context: { kind: "tracks", id: "albums", title: "Albums on shuffle" } } }));
    goToPlaying();
    expect(route()).toEqual({ name: "album", id: state.track!.albumId });
  });

  it("waits for the page's rows, asks the page to bring the row in, and says so when it never shows", async () => {
    vi.useFakeTimers();
    const host = document.createElement("div");
    host.className = "main__scroll";
    document.body.append(host);
    const asked: string[] = [];
    function Page() {
      usePageReveal(reveal);
      return null;
    }
    const reveal = (id: string) => void asked.push(id);
    const { unmount } = render(<Page />);
    try {
      revealRow("late");
      vi.advanceTimersByTime(300);
      expect(asked.length).toBeGreaterThan(0);
      host.innerHTML = `<div class="track-row" data-track-id="late"></div>`;
      vi.advanceTimersByTime(100);
      expect(host.querySelector(".track-row")).toHaveClass("track-row--found");
      revealRow("never");
      vi.advanceTimersByTime(6_000);
      expect(useToasts.getState().toasts.slice(-1)[0]?.message).toBe("The playing song isn't on this page.");
      const n = asked.length;
      vi.advanceTimersByTime(1_000);
      expect(asked.length).toBe(n);
    } finally {
      unmount();
      host.remove();
      vi.useRealTimers();
    }
  });

  it("leaving the page stops looking", () => {
    vi.useFakeTimers();
    try {
      revealRow("gone");
      act(() => nav.go({ name: "home" }));
      act(() => nav.go({ name: "liked" }));
      const host = document.createElement("div");
      host.className = "main__scroll";
      host.innerHTML = `<div class="track-row" data-track-id="gone"></div>`;
      document.body.append(host);
      vi.advanceTimersByTime(500);
      expect(host.querySelector(".track-row")).not.toHaveClass("track-row--found");
      vi.advanceTimersByTime(6_000);
      expect(useToasts.getState().toasts).toHaveLength(0);
      host.remove();
    } finally {
      vi.useRealTimers();
    }
  });

  it("an artist's list goes to the song's album, where the song is", async () => {
    const state = await playing();
    act(() => usePlayer.getState().set({ ...state, queue: { ...state.queue, context: { kind: "artist", id: state.track!.artistId, title: "x" } } }));
    goToPlaying();
    expect(route()).toEqual({ name: "album", id: state.track!.albumId });
  });

  it("says so when nothing is playing, and Ctrl/Cmd+Shift+J runs it", async () => {
    const state = await playing();
    act(() => usePlayer.getState().set({ ...state, track: null }));
    render(<Keys />);
    fireEvent.keyDown(window, { key: "J", code: "KeyJ", ctrlKey: true, metaKey: true, shiftKey: true });
    expect(useToasts.getState().toasts.slice(-1)[0]?.message).toBe("Nothing is playing.");
  });
});

describe("new smart playlist key", () => {
  it("Alt+Shift+O opens a new smart playlist", () => {
    useCurate.setState({ editor: null });
    render(<Keys />);
    fireEvent.keyDown(window, { key: "O", code: "KeyO", altKey: true, shiftKey: true });
    expect(useCurate.getState().editor).toMatchObject({ id: null, title: "" });
    useCurate.setState({ editor: null });
  });
});

describe("find in the page you're on", () => {
  async function playlistPage() {
    const p = await backend.playlistCreate("Find me", mock.fx.tracks.slice(0, 6).map((t) => t.id));
    act(() => useApp.getState().setPlaylists([...(useApp.getState().playlists ?? []), p]));
    render(
      <>
        <PlaylistPage id={p.id} />
        <ContextMenuHost />
        <PlaylistDialogs />
        <Keys />
      </>,
    );
    await screen.findAllByRole("row");
    return p;
  }
  const titles = () => Array.from(document.querySelectorAll(".track-row .track-row__title")).map((el) => el.textContent);

  it("Ctrl/Cmd+F opens the playlist's finder, which narrows and clears", async () => {
    const user = userEvent.setup();
    const p = await playlistPage();
    const all = titles();
    expect(all.length).toBe(6);
    fireEvent.keyDown(window, { key: "f", code: "KeyF", ctrlKey: true, metaKey: true });
    const field = await screen.findByRole("searchbox", { name: `Find in ${p.title}` });
    await waitFor(() => expect(field).toHaveFocus());
    const want = mock.fx.tracks[2].title;
    await user.type(field, want);
    await waitFor(() => expect(titles()).toEqual(all.filter((t) => t?.toLowerCase().includes(want.toLowerCase()))));
    await user.keyboard("{Escape}");
    expect(titles()).toEqual(all);
  });

  it("while narrowed, Alt+Shift+arrows don't move songs, and back to the playing song clears the finder", async () => {
    const user = userEvent.setup();
    const p = await playlistPage();
    const all = titles();
    const move = vi.spyOn(backend, "playlistMove");
    openPageFind();
    const want = mock.fx.tracks[2].title;
    await user.type(await screen.findByRole("searchbox", { name: `Find in ${p.title}` }), want);
    await waitFor(() => expect(titles().length).toBeLessThan(all.length));
    const row = document.querySelector<HTMLElement>(".track-row")!;
    row.focus();
    fireEvent.keyDown(row, { key: "ArrowDown", altKey: true, shiftKey: true });
    expect(move).not.toHaveBeenCalled();
    const hidden = mock.fx.tracks.slice(0, 6).find((t) => !titles().includes(t.title))!;
    revealRow(hidden.id);
    await waitFor(() => expect(titles()).toEqual(all));
  });

  it("says when nothing matches", async () => {
    const user = userEvent.setup();
    await playlistPage();
    expect(openPageFind()).toBe(true);
    await user.type(await screen.findByRole("searchbox"), "zzzz no such song");
    expect(await screen.findByText('Nothing in this playlist matches "zzzz no such song"')).toBeInTheDocument();
  });

  it("elsewhere there is no page finder, so Ctrl/Cmd+F filters Your Library", () => {
    expect(openPageFind()).toBe(false);
    const focus = vi.spyOn(useUi.getState(), "focusLibraryFilter");
    render(<Keys />);
    fireEvent.keyDown(window, { key: "f", code: "KeyF", ctrlKey: true, metaKey: true });
    return waitFor(() => expect(focus).toHaveBeenCalled());
  });
});
