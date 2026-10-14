import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { SearchArgs, SearchResults, Track } from "../lib/types";
import { MAX_QUERY, SearchPage, picksFor, visibleSongs } from "../pages/SearchPage";
import { MainView } from "../shell/MainView";
import { useShortcuts } from "../shell/useShortcuts";
import { useRouter } from "../stores/router";
import { useUi } from "../stores/ui";

const route = () => useRouter.getState().entries[useRouter.getState().index].route;

function Host() {
  useShortcuts();
  return <MainView />;
}

function openSearch(query: string) {
  useRouter.getState().reset({ name: "search", query });
}

const searchBox = () => screen.getByRole("searchbox", { name: "Search" });

function track(id: string, title: string): Track {
  return {
    id,
    title,
    albumId: "al1",
    albumTitle: "Album",
    artistId: "ar1",
    artistTitle: "Artist",
    trackArtist: null,
    trackNumber: 1,
    discNumber: 1,
    durationMs: 180_000,
    thumb: null,
    partKey: "/p",
    codec: null,
    container: null,
    bitrate: null,
    bitDepth: null,
    sampleRate: null,
    userRating: null,
    viewCount: 0,
    lastViewedAt: null,
  } as Track;
}

function results(songTitles: string[]): SearchResults {
  return {
    top: null,
    songs: songTitles.map((t, i) => track(`t${i}-${t}`, t)),
    artists: [],
    albums: [],
    playlists: [],
    tookMs: 1,
    source: "local",
  };
}

beforeEach(() => {
  useUi.setState({ paletteOpen: false, searchFocusTick: 0, menu: null });
});
afterEach(() => vi.restoreAllMocks());

describe("filter chips", () => {
  it("pass the filter to the core and show only that kind", async () => {
    const spy = vi.spyOn(backend, "search");
    const user = userEvent.setup();
    openSearch("Odessa");
    render(<Host />);
    await screen.findByRole("heading", { name: "Top result" });
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ query: "Odessa", filter: "all" }));

    await user.click(screen.getByRole("button", { name: "Songs" }));
    expect(screen.getByRole("button", { name: "Songs" })).toHaveAttribute("aria-pressed", "true");
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ filter: "songs" }));
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Top result" })).not.toBeInTheDocument());
    expect(screen.queryByRole("heading", { name: "Artists" })).not.toBeInTheDocument();
    expect((await screen.findAllByRole("listitem", { name: /by Odessa Wren/ })).length).toBeGreaterThan(4);

    await user.click(screen.getByRole("button", { name: "Artists" }));
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ filter: "artists" }));
    expect(await screen.findByRole("heading", { name: "Artists" })).toBeInTheDocument();
  });

  it('"Show all" on a shelf switches to that chip', async () => {
    const user = userEvent.setup();
    openSearch("o");
    render(<Host />);
    const albums = await screen.findByRole("region", { name: "Albums" });
    await user.click(within(albums).getByRole("button", { name: "Show all" }));
    expect(screen.getByRole("button", { name: "Albums" })).toHaveAttribute("aria-pressed", "true");
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Top result" })).not.toBeInTheDocument());
  });
});

describe("as-you-type results", () => {
  it("never shows results for an older query", async () => {
    const pending = new Map<string, (r: SearchResults) => void>();
    vi.spyOn(backend, "search").mockImplementation(
      (args: SearchArgs) => new Promise<SearchResults>((resolve) => pending.set(args.query, resolve)),
    );
    const { rerender } = render(<SearchPage query="gl" />);
    rerender(<SearchPage query="glo" />);
    rerender(<SearchPage query="glow" />);
    // Newest answers first, then the stale ones arrive late.
    await act(async () => pending.get("glow")!(results(["Glow"])));
    expect(await screen.findByRole("listitem", { name: /^Glow by/ })).toBeInTheDocument();
    await act(async () => {
      pending.get("gl")!(results(["Glass Stale"]));
      pending.get("glo")!(results(["Gloom Stale"]));
    });
    expect(screen.queryByRole("listitem", { name: /Stale/ })).not.toBeInTheDocument();
    expect(screen.getByRole("listitem", { name: /^Glow by/ })).toBeInTheDocument();
  });

  it("keeps the previous results on screen while the next keystroke loads", async () => {
    const pending = new Map<string, (r: SearchResults) => void>();
    vi.spyOn(backend, "search").mockImplementation(
      (args: SearchArgs) => new Promise<SearchResults>((resolve) => pending.set(args.query, resolve)),
    );
    const { rerender } = render(<SearchPage query="ri" />);
    await act(async () => pending.get("ri")!(results(["Ribbon"])));
    rerender(<SearchPage query="rib" />);
    expect(screen.getByRole("listitem", { name: /^Ribbon by/ })).toBeInTheDocument();
  });

  it("clips very long queries and treats blank ones as empty", async () => {
    const spy = vi.spyOn(backend, "search");
    const { rerender } = render(<SearchPage query={"x".repeat(10_000)} />);
    await waitFor(() => expect(spy).toHaveBeenCalled());
    expect(spy.mock.calls[0][0].query.length).toBe(MAX_QUERY);
    expect(await screen.findByRole("heading", { name: /No results for/ })).toBeInTheDocument();
    spy.mockClear();
    rerender(<SearchPage query="    " />);
    expect(spy).not.toHaveBeenCalled();
  });

  it("shows a calm message when search fails", async () => {
    vi.spyOn(backend, "search").mockRejectedValue({ kind: "internal", message: "boom" });
    render(<SearchPage query="anything" />);
    expect(await screen.findByRole("heading", { name: "Search isn't available right now" })).toBeInTheDocument();
  });
});

describe("keyboard", () => {
  it("Ctrl+L focuses search from anywhere", async () => {
    useRouter.getState().reset({ name: "album", id: "nope" });
    render(<Host />);
    fireEvent.keyDown(window, { key: "l", ctrlKey: true });
    await waitFor(() => expect(route().name).toBe("search"));
    await waitFor(() => expect(searchBox()).toHaveFocus());
  });

  it("arrows walk top result, songs, then shelves; Enter plays in a search context", async () => {
    const play = vi.spyOn(backend, "playContext");
    const recent = vi.spyOn(backend, "recentSearchAdd");
    openSearch("Odessa");
    render(<Host />);
    await screen.findByRole("heading", { name: "Top result" });
    const res = await backend.search({ query: "Odessa", filter: "all", limit: 10 });
    const picks = picksFor(res, "all");
    const songs = visibleSongs(res, "all");
    expect(picks[0].kind).toBe("top");
    expect(picks.slice(1, 1 + songs.length).every((p) => p.kind === "song")).toBe(true);
    expect(picks[1 + songs.length].kind).toBe("artist");

    const box = searchBox();
    box.focus();
    fireEvent.keyDown(box, { key: "ArrowDown" }); // top result
    expect(document.querySelector(".top-result.is-selected")).not.toBeNull();
    fireEvent.keyDown(box, { key: "ArrowDown" }); // first song
    fireEvent.keyDown(box, { key: "ArrowDown" }); // second song
    const selectedRow = document.querySelector(".track-row.is-selected");
    expect(selectedRow).toHaveAccessibleName(new RegExp(`^${songs[1].title} by`));

    fireEvent.keyDown(box, { key: "Enter" });
    expect(play).toHaveBeenCalledWith({
      context: { kind: "search", trackIds: songs.map((s) => s.id) },
      startTrackId: songs[1].id,
    });
    expect(recent).toHaveBeenCalledWith("Odessa");

    // Up past the top clears the selection; Enter then just submits the query.
    fireEvent.keyDown(box, { key: "ArrowUp" });
    fireEvent.keyDown(box, { key: "ArrowUp" });
    fireEvent.keyDown(box, { key: "ArrowUp" });
    expect(document.querySelector(".search-pick.is-selected, .track-row.is-selected")).toBeNull();
    play.mockClear();
    fireEvent.keyDown(box, { key: "Enter" });
    expect(play).not.toHaveBeenCalled();
  });

  it("Enter on an artist, album or playlist plays that context", async () => {
    const play = vi.spyOn(backend, "playContext");
    openSearch("s");
    render(<Host />);
    await screen.findByRole("heading", { name: "Top result" });
    const res = await backend.search({ query: "s", filter: "all", limit: 10 });
    expect(res.artists.length && res.albums.length && res.playlists.length).toBeTruthy();
    const picks = picksFor(res, "all");
    const box = searchBox();
    box.focus();
    const go = (i: number) => {
      for (let k = 0; k < picks.length + 2; k++) fireEvent.keyDown(box, { key: "ArrowUp" });
      for (let k = 0; k <= i; k++) fireEvent.keyDown(box, { key: "ArrowDown" });
      fireEvent.keyDown(box, { key: "Enter" });
    };
    const artistAt = picks.findIndex((p) => p.kind === "artist");
    const albumAt = picks.findIndex((p) => p.kind === "album");
    const playlistAt = picks.findIndex((p) => p.kind === "playlist");
    go(artistAt);
    expect(play).toHaveBeenLastCalledWith(expect.objectContaining({ context: { kind: "artist", id: res.artists[0].id } }));
    go(albumAt);
    expect(play).toHaveBeenLastCalledWith(expect.objectContaining({ context: { kind: "album", id: res.albums[0].id } }));
    go(playlistAt);
    expect(play).toHaveBeenLastCalledWith(expect.objectContaining({ context: { kind: "playlist", id: res.playlists[0].id } }));
    go(0);
    const top = res.top!;
    const want =
      top.kind === "track"
        ? { kind: "search", trackIds: visibleSongs(res, "all").map((t) => t.id) }
        : { kind: top.kind, id: (top.artist ?? top.album ?? top.playlist)!.id };
    expect(play).toHaveBeenLastCalledWith(expect.objectContaining({ context: want }));
  });

  it("Shift+Enter adds the selection to the queue instead of playing", async () => {
    const play = vi.spyOn(backend, "playContext");
    const queue = vi.spyOn(backend, "queueAdd");
    openSearch("s");
    render(<Host />);
    await screen.findByRole("heading", { name: "Top result" });
    const res = await backend.search({ query: "s", filter: "all", limit: 10 });
    const songs = visibleSongs(res, "all");
    const box = searchBox();
    box.focus();
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "Enter", shiftKey: true });
    await waitFor(() => expect(queue).toHaveBeenCalledWith([songs[0].id], "end"));

    // An album queues all its tracks.
    const albumAt = picksFor(res, "all").findIndex((p) => p.kind === "album");
    for (let k = 1; k < albumAt; k++) fireEvent.keyDown(box, { key: "ArrowDown" });
    fireEvent.keyDown(box, { key: "Enter", shiftKey: true });
    const page = await backend.albumPage(res.albums[0].id);
    await waitFor(() => expect(queue).toHaveBeenLastCalledWith(page.tracks.map((t) => t.id), "end"));
    expect(play).not.toHaveBeenCalled();
  });
});

describe("recent searches", () => {
  it("lists recent searches, opens one, and clears them", async () => {
    const user = userEvent.setup();
    await backend.recentSearchAdd("saltglass");
    openSearch("");
    render(<Host />);
    const item = await screen.findByRole("button", { name: "saltglass" });
    await user.click(item);
    expect(route()).toEqual({ name: "search", query: "saltglass" });

    useRouter.getState().replace({ name: "search", query: "" });
    await user.click(await screen.findByRole("button", { name: "Clear recent searches" }));
    expect(await screen.findByRole("heading", { name: "Search your library" })).toBeInTheDocument();
    expect(await backend.recentSearches()).toEqual([]);
  });
});
