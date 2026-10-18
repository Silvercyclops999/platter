/**
 * Home, Radio and mixes, and the library while only music on this computer
 * can play: downloaded mixes and Liked Songs stay, what can't play leaves,
 * and failures to read what is here say so instead of looking empty. Each
 * state is faked through the mock backend or the stores.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type DownloadItem, type DownloadStatus } from "../lib/types";
import { HomePage } from "../pages/HomePage";
import { HomeShelfPage } from "../pages/HomeShelfPage";
import { MixesPage } from "../pages/MixesPage";
import { offlineShelfAlbums } from "../pages/home";
import { Sidebar } from "../shell/Sidebar";
import { connectApp, isUnavailable, useApp } from "../stores/app";
import { resetDownloads, startDownloads, useDownloads } from "../stores/downloads";
import { useLibraryView } from "../stores/library";
import { useLikes } from "../stores/likes";
import { loadSettings, useSettings } from "../stores/settings";

const mock = backend as MockBackend;

const item = (over: Partial<DownloadItem>): DownloadItem => ({
  kind: "album",
  id: "x",
  title: "x",
  state: "downloaded",
  total: 2,
  done: 2,
  failed: 0,
  missing: 0,
  progress: 1,
  bytes: 0,
  error: null,
  ...over,
});
const status = (items: DownloadItem[]): DownloadStatus => ({ items, usedBytes: 0, limitBytes: 1024 ** 3, folder: "downloads", paused: null, queued: 0 });

async function goOffline() {
  act(() => mock.setHealth("offline"));
  useApp.setState({ health: await mock.health() });
}

beforeEach(async () => {
  mock.downloads.reset();
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  mock.setHealth("local");
  useApp.setState({ health: await mock.health(), cached: null });
  useLibraryView.setState({ filter: "all", sub: null, downloaded: null });
  resetDownloads();
  await startDownloads();
});
afterEach(() => {
  vi.restoreAllMocks();
  act(() => mock.setHealth("local"));
  useApp.setState({ health: null, cached: null });
});

describe("downloaded mixes", () => {
  it("a downloaded mix follows its mix when it is made again", async () => {
    const before = (await mock.mixPage("daily:1")).tracks.map((t) => t.id);
    await mock.downloadsSet("mix", "daily:1", true, { title: "Daily Mix 1", trackIds: before });
    const listed = () => mock.downloads.status().items.find((i) => i.kind === "mix")!.total;
    expect(listed()).toBe(before.length);
    await mock.mixRefresh("daily:1");
    const after = (await mock.mixPage("daily:1")).tracks.map((t) => t.id);
    expect(after).not.toEqual(before);
    // The download now lists the new songs, and only those.
    await waitFor(() => expect(listed()).toBe(after.length));
  });

  it("offline, Home and Radio and mixes keep only mixes with downloaded songs", async () => {
    mock.downloads.seedDone("mix", "daily:2", "Daily Mix 2");
    resetDownloads();
    await startDownloads();
    await goOffline();
    render(<HomePage />);
    const shelf = (await screen.findByRole("heading", { name: "Made for you" })).closest("section")!;
    expect(within(shelf as HTMLElement).getByRole("button", { name: "Daily Mix 2" })).toBeInTheDocument();
    expect(within(shelf as HTMLElement).queryByRole("button", { name: "Daily Mix 1" })).toBeNull();
  });

  it("Radio and mixes says when no mix is on this computer", async () => {
    await goOffline();
    render(<MixesPage />);
    expect(await screen.findByText("No mixes on this computer")).toBeInTheDocument();
  });

  it("Radio and mixes shows new mixes as soon as they are made", async () => {
    render(<MixesPage />);
    await screen.findByRole("button", { name: "Daily Mix 1" });
    const calls = vi.spyOn(mock, "mixesHome");
    act(() => mock.emit("mixes://changed", null));
    await waitFor(() => expect(calls).toHaveBeenCalled());
  });
});

describe("Home offline", () => {
  it("keeps the Liked Songs tile while a liked song is on this computer", async () => {
    const [here, away] = mock.fx.tracks;
    useLikes.setState({ ids: new Set([here.id, away.id]) });
    await goOffline();
    useApp.setState({ cached: new Set([here.id]) });
    const { unmount } = render(<HomePage />);
    expect(await screen.findByRole("button", { name: "Play Liked Songs" })).toBeInTheDocument();
    unmount();
    // Nothing liked here: no tile.
    useApp.setState({ cached: new Set(["nothing-liked"]) });
    render(<HomePage />);
    await screen.findByRole("region", { name: "On this computer" });
    expect(screen.queryByRole("button", { name: "Play Liked Songs" })).toBeNull();
  });

  it("couldn't read what's on this computer: says so with Try again, not 'nothing here'", async () => {
    const spy = vi.spyOn(mock, "cachedLibrary").mockRejectedValueOnce({ kind: "io", message: "disk error" });
    await goOffline();
    render(<HomePage />);
    expect(await screen.findByText("Couldn't load what's on this computer")).toBeInTheDocument();
    expect(screen.queryByText("Nothing on this computer yet")).toBeNull();
    spy.mockRestore();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("region", { name: "On this computer" })).toBeInTheDocument();
  });

  it("On this computer leaves out what Recently played shows and stays one shelf long", () => {
    const albums = mock.fx.albums.slice(0, 20);
    const lib = { albums, artistIds: [], playlistIds: [] };
    const home = { shortcuts: [], recentlyPlayed: [{ kind: "album" as const, album: albums[0] }], jumpBackIn: [], playlists: [], recentlyAdded: [], topArtists: [] };
    const shelf = offlineShelfAlbums(lib, home);
    expect(shelf).toHaveLength(12);
    expect(shelf.map((a) => a.id)).not.toContain(albums[0].id);
  });

  it("a shelf's Show all keeps to what can play", async () => {
    await goOffline();
    const cached = new Set(mock.fx.albums.slice(0, 2).map((a) => a.id));
    render(<HomeShelfPage shelf="recentlyAdded" />);
    const grid = await screen.findByRole("list", { name: "Recently added" });
    await waitFor(() => expect(within(grid).getAllByRole("listitem").length).toBeGreaterThan(0));
    const here = mock.fx.albums.filter((a) => cached.has(a.id)).map((a) => a.title);
    for (const li of within(grid).getAllByRole("listitem")) expect(here.some((t) => li.textContent?.includes(t))).toBe(true);
  });
});

describe("Home's mixes", () => {
  it("a failure to load them says so with Try again", async () => {
    const spy = vi.spyOn(mock, "mixesHome").mockRejectedValue({ kind: "internal", message: "mix task failed" });
    render(<HomePage />);
    expect(await screen.findByText("Couldn't load your mixes")).toBeInTheDocument();
    spy.mockRestore();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("heading", { name: "Made for you" })).toBeInTheDocument();
  });

  it("Made for you holds its place while the mixes load", async () => {
    let answer: (v: Awaited<ReturnType<typeof mock.mixesHome>>) => void = () => {};
    const real = await mock.mixesHome();
    vi.spyOn(mock, "mixesHome").mockReturnValue(new Promise((r) => (answer = r)));
    render(<HomePage />);
    await screen.findByRole("heading", { name: "Recently played" });
    // Its place is taken by a loading row, so the page doesn't shift.
    const shelf = screen.getByRole("region", { name: "Made for you" });
    expect(within(shelf).queryByRole("button", { name: "Daily Mix 1" })).toBeNull();
    await act(async () => answer(real));
    expect(await within(screen.getByRole("region", { name: "Made for you" })).findByRole("button", { name: "Daily Mix 1" })).toBeInTheDocument();
  });
});

describe("downloaded marks in the library", () => {
  it("only items with songs on this computer count as downloaded", async () => {
    const [a, b] = mock.fx.albums;
    act(() =>
      useDownloads.setState({
        status: status([item({ id: a.id, title: a.title, state: "queued", done: 0 }), item({ id: b.id, title: b.title, state: "error", done: 0, failed: 2 })]),
      }),
    );
    render(<Sidebar collapsed={false} />);
    await screen.findAllByRole("listitem");
    expect(screen.queryByRole("button", { name: "Downloaded" })).toBeNull();
    act(() => useDownloads.setState({ status: status([item({ id: a.id, title: a.title, state: "downloading", done: 1 })]) }));
    expect(await screen.findByRole("button", { name: "Downloaded" })).toBeInTheDocument();
  });
});

describe("the list of songs on this computer", () => {
  it("unknown (it couldn't be read) dims nothing", () => {
    const h = { ...useApp.getState().health!, cachedOnly: true };
    expect(isUnavailable("x", { health: h, cached: null })).toBe(false);
    expect(isUnavailable("x", { health: h, cached: new Set() })).toBe(true);
  });

  it("a failed read keeps nothing dimmed, and a finished download is picked up straight away", async () => {
    const spy = vi.spyOn(mock, "cachedTracks").mockRejectedValueOnce({ kind: "io", message: "busy" });
    const off = connectApp(mock);
    try {
      await goOffline();
      act(() => mock.setHealth("offline"));
      await waitFor(() => expect(spy).toHaveBeenCalled());
      expect(useApp.getState().cached).toBeNull();
      spy.mockRestore();
      const reads = vi.spyOn(mock, "cachedTracks");
      act(() => mock.emit("downloads://changed", null));
      await waitFor(() => expect(reads).toHaveBeenCalled());
      await waitFor(() => expect(useApp.getState().cached?.size).toBeGreaterThan(0));
    } finally {
      off();
    }
  });
});
