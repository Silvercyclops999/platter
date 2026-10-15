/**
 * Downloads: the toggle on album, playlist and Liked Songs pages (outline,
 * progress ring with percent, filled), "Download complete", the remove
 * confirm, the downloaded mark on rows and library items, the Downloaded
 * chip (on by itself offline), Settings storage rows, Go offline, keeping
 * downloaded items in sync with the library, and the app states: storage
 * full, download failed, file missing on disk.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { DownloadDialogs, DownloadsBanner } from "../components/DownloadButton";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type DownloadItem, type DownloadStatus } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { LikedPage } from "../pages/LikedPage";
import { PlaylistPage } from "../pages/PlaylistPage";
import { SettingsPage } from "../pages/SettingsPage";
import { offlineCallout } from "../shell/bannerView";
import { Sidebar } from "../shell/Sidebar";
import { AppMenu } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { catchingUpMessage, findItem, resetDownloads, setDownload, startDownloads, useDownloads } from "../stores/downloads";
import { useLibraryView } from "../stores/library";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const album = mock.fx.albums[3];
const albumTracks = mock.fx.tracksByAlbum.get(album.id)!;

const button = () => screen.getByTestId("download-button");
const toasts = () => useToasts.getState().toasts.map((t) => t.message);
/**
 * Waits for a download to finish: the button turns to the filled disc, then
 * names "Remove download of …". A role query re-run on every progress tick of
 * a long download takes seconds on its own, so the wait watches the button's
 * class and the accessible name is checked once at the end.
 */
async function downloaded(title: string): Promise<HTMLElement> {
  await waitFor(() => expect(button()).toHaveClass("dl-btn--downloaded"));
  return screen.getByRole("button", { name: `Remove download of ${title}` });
}

beforeEach(async () => {
  mock.downloads.reset();
  mock.downloads.stepMs = 5;
  mock.downloads.steps = 3;
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  mock.setHealth("local");
  useApp.setState({ health: await mock.health() });
  useSettings.setState({ settings: null });
  await loadSettings();
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null });
  useLibraryView.setState({ filter: "all", sub: null, downloaded: null });
  resetDownloads();
  await startDownloads();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("album download", () => {
  it("goes from the outline to a ring with percent to the filled disc, then says it's complete", async () => {
    mock.downloads.stepMs = 25;
    mock.downloads.steps = 2;
    render(<AlbumPage id={album.id} />);
    const dl = await screen.findByRole("button", { name: `Download ${album.title}` });
    expect(dl).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(dl);
    // Downloading: a ring and the percent beside it.
    await waitFor(() => expect(button()).toHaveClass("dl-btn--downloading"));
    expect(within(button()).getByText(/^\d+%$/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Downloading, \d+%\. Cancel the download/ })).toBeInTheDocument();
    const done = await downloaded(album.title);
    expect(done).toHaveAttribute("aria-pressed", "true");
    expect(toasts()).toContain("Download complete");
    // Every row has the downloaded mark.
    await waitFor(() => expect(screen.getAllByRole("img", { name: "Downloaded" })).toHaveLength(albumTracks.length));
    expect(screen.getAllByRole("row", { name: /, downloaded/ }).length).toBe(albumTracks.length);
  });

  it("asks before removing, and Cancel keeps it", async () => {
    mock.downloads.seedDone("album", album.id, album.title);
    resetDownloads();
    await startDownloads();
    render(
      <>
        <AlbumPage id={album.id} />
        <DownloadDialogs />
      </>,
    );
    const remove = await screen.findByRole("button", { name: `Remove download of ${album.title}` });
    await userEvent.click(remove);
    const dialog = await screen.findByRole("dialog", { name: "Remove from downloads?" });
    expect(dialog).toHaveTextContent("You won't be able to play this offline.");
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(mock.downloads.isOn("album", album.id)).toBe(true);
    await userEvent.click(remove);
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Remove" }));
    await screen.findByRole("button", { name: `Download ${album.title}` });
    expect(mock.downloads.isOn("album", album.id)).toBe(false);
    await waitFor(() => expect(screen.queryAllByRole("img", { name: "Downloaded" })).toHaveLength(0));
  });

  it("is in the album menu", async () => {
    render(
      <>
        <AlbumPage id={album.id} />
        <ContextMenuHost />
      </>,
    );
    await userEvent.click(await screen.findByRole("button", { name: `More options for ${album.title}` }));
    await userEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Download" }));
    await waitFor(() => expect(mock.downloads.isOn("album", album.id)).toBe(true));
  });
});

describe("playlists and Liked Songs", () => {
  it("downloads a playlist from its page", async () => {
    mock.downloads.steps = 1;
    mock.downloads.stepMs = 1;
    const pl = mock.fx.playlists.find((p) => (mock.fx.playlistTracks.get(p.id) ?? []).length > 0 && !p.smart)!;
    render(<PlaylistPage id={pl.id} />);
    await userEvent.click(await screen.findByRole("button", { name: `Download ${pl.title}` }));
    await downloaded(pl.title);
  });

  it("Liked Songs' toggle is the Download Liked Songs setting", async () => {
    render(<LikedPage />);
    await userEvent.click(await screen.findByRole("button", { name: "Download Liked Songs" }));
    await waitFor(() => expect(mock.downloads.isOn("liked", "")).toBe(true));
    expect((await mock.settingsGet()).downloadLiked).toBe(true);
  });
});

describe("keeping downloads in sync", () => {
  const pl = () => mock.fx.playlists.find((p) => (mock.fx.playlistTracks.get(p.id) ?? []).length > 1 && !p.smart)!;
  /** Songs on no downloaded item yet. */
  const fresh = (n: number) => {
    const have = new Set([...(mock.fx.playlistTracks.get(pl().id) ?? []), ...mock.likes.ids()]);
    return mock.fx.tracks.filter((t) => !have.has(t.id)).slice(0, n).map((t) => t.id);
  };
  const seed = async (kind: "playlist" | "liked" | "album", id: string, title: string) => {
    mock.downloads.seedDone(kind, id, title);
    resetDownloads();
    await startDownloads();
  };

  it("songs added to a downloaded playlist download by themselves", async () => {
    const p = pl();
    await seed("playlist", p.id, p.title);
    mock.downloads.stepMs = 30;
    mock.downloads.steps = 2;
    render(<PlaylistPage id={p.id} />);
    await screen.findByRole("button", { name: `Remove download of ${p.title}` });
    const added = fresh(3);
    await mock.playlistAdd(p.id, added);
    // The button shows it's catching up, with a toast saying how many.
    await waitFor(() => expect(button()).toHaveClass("dl-btn--downloading"));
    expect(toasts()).toContain(catchingUpMessage(3));
    expect(useDownloads.getState().ids.has(added[2])).toBe(false);
    // Row badges fill in one by one, then it's downloaded again.
    await waitFor(() => expect(useDownloads.getState().ids.has(added[0])).toBe(true));
    await downloaded(p.title);
    for (const id of added) expect(useDownloads.getState().ids.has(id)).toBe(true);
    expect(toasts().filter((t) => t === "Download complete")).toHaveLength(1);
  });

  it("songs removed from a downloaded playlist lose their files quietly", async () => {
    const p = pl();
    await seed("playlist", p.id, p.title);
    const ids = mock.fx.playlistTracks.get(p.id)!;
    const gone = ids[ids.length - 1];
    const page = await mock.playlistPage(p.id);
    const item = page.itemIds[page.tracks.findIndex((t) => t.id === gone)];
    await mock.playlistRemove(p.id, [item]);
    await waitFor(() => expect(useDownloads.getState().ids.has(gone)).toBe(false));
    expect(mock.downloads.downloadedIds()).not.toContain(gone);
    const st = useDownloads.getState().status!.items.find((i) => i.kind === "playlist")!;
    expect(st.state).toBe("downloaded");
    expect(toasts()).toEqual([]);
  });

  it("a new like downloads when Liked Songs is downloaded", async () => {
    await seed("liked", "", "Liked Songs");
    const [song] = fresh(1);
    await mock.likesSet([song], true);
    await waitFor(() => expect(toasts()).toContain(catchingUpMessage(1)));
    await waitFor(() => expect(useDownloads.getState().ids.has(song)).toBe(true));
    await waitFor(() => expect(toasts()).toContain("Download complete"));
    // Unliking takes the file away again.
    await mock.likesSet([song], false);
    await waitFor(() => expect(useDownloads.getState().ids.has(song)).toBe(false));
  });

  it("an album that gains a song (a live library update) fetches it", async () => {
    await seed("album", album.id, album.title);
    const [song] = fresh(1);
    const list = mock.fx.tracksByAlbum.get(album.id)!;
    list.push(mock.fx.tracks.find((t) => t.id === song)!);
    try {
      act(() => mock.downloads.resync());
      await waitFor(() => expect(toasts()).toContain(catchingUpMessage(1)));
      await waitFor(() => expect(useDownloads.getState().ids.has(song)).toBe(true));
    } finally {
      list.pop();
    }
  });

  it("Go offline holds the new songs, then they catch up", async () => {
    const p = pl();
    await seed("playlist", p.id, p.title);
    await mock.settingsSet({ goOffline: true });
    await waitFor(async () => expect((await mock.downloadsStatus()).paused).toBe("offline"));
    const [song] = fresh(1);
    await mock.playlistAdd(p.id, [song]);
    await waitFor(() => expect(useDownloads.getState().status!.items[0].total).toBe(mock.fx.playlistTracks.get(p.id)!.length));
    const it = useDownloads.getState().status!.items[0];
    expect(it.state).toBe("queued");
    expect(useDownloads.getState().ids.has(song)).toBe(false);
    expect(toasts()).not.toContain(catchingUpMessage(1));
    await mock.settingsSet({ goOffline: false });
    await waitFor(() => expect(useDownloads.getState().ids.has(song)).toBe(true));
  });
});

describe("library sidebar", () => {
  it("marks downloaded items and offers a Downloaded chip, on by itself offline", async () => {
    mock.downloads.seedDone("album", mock.fx.albums[0].id, mock.fx.albums[0].title);
    resetDownloads();
    await startDownloads();
    render(<Sidebar collapsed={false} />);
    const chip = await screen.findByRole("button", { name: "Downloaded" });
    expect(chip).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(chip);
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(1));
    expect(within(screen.getByRole("listitem")).getByRole("img", { name: "Downloaded" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Downloaded" }));
    await waitFor(() => expect(screen.getAllByRole("listitem").length).toBeGreaterThan(1));
    // Offline: the chip turns itself on; back online it goes off again.
    act(() => mock.setHealth("offline"));
    act(() => useApp.setState({ health: { ...useApp.getState().health!, cachedOnly: true, status: "offline" } }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Downloaded" })).toHaveAttribute("aria-pressed", "true"));
    act(() => useApp.setState({ health: { ...useApp.getState().health!, cachedOnly: false, status: "local" } }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Downloaded" })).toHaveAttribute("aria-pressed", "false"));
  });
});

describe("app states", () => {
  it("storage full: the queue stops with a banner that opens Settings", async () => {
    await mock.settingsSet({ downloadLimitGb: 1 });
    mock.downloads.usedOffset = 1024 ** 3;
    render(<DownloadsBanner />);
    await mock.downloadsSet("album", mock.fx.albums[0].id, true);
    const banner = await screen.findByTestId("downloads-banner");
    expect(banner).toHaveTextContent("Downloads stopped: they reached the storage limit you set.");
    expect(within(banner).getByRole("button", { name: "Open Settings" })).toBeInTheDocument();
    expect(useDownloads.getState().status?.paused).toBe("full");
  });

  it("download failed: the button shows the error and a click tries again", async () => {
    mock.downloads.failNext = true;
    const one = mock.fx.albums.find((a) => (mock.fx.tracksByAlbum.get(a.id) ?? []).length === 1) ?? album;
    render(<AlbumPage id={one.id} />);
    await mock.downloadsSet("mix", "m", true, { title: "Mix", trackIds: [mock.fx.tracksByAlbum.get(one.id)![0].id] });
    await userEvent.click(await screen.findByRole("button", { name: `Download ${one.title}` }));
    await waitFor(() => expect(button()).toHaveClass("dl-btn--error"));
    const retry = screen.getByRole("button", { name: new RegExp(`couldn't download\\. Download ${one.title} again`) });
    await userEvent.click(retry);
    await downloaded(one.title);
  });

  it("file missing on disk: shown as an error and downloaded again", async () => {
    mock.downloads.seedDone("album", album.id, album.title);
    resetDownloads();
    await startDownloads();
    render(<AlbumPage id={album.id} />);
    await screen.findByRole("button", { name: `Remove download of ${album.title}` });
    act(() => mock.downloads.loseFile(albumTracks[0].id));
    const again = await screen.findByRole("button", { name: /missing from this computer\. Download .* again/ });
    await userEvent.click(again);
    await downloaded(album.title);
  });
});

describe("Go offline", () => {
  it("the app menu switch goes offline and back", async () => {
    render(<AppMenu />);
    await userEvent.click(screen.getByRole("button", { name: /App menu/ }));
    const item = screen.getByRole("menuitemcheckbox", { name: "Go offline" });
    expect(item).toHaveAttribute("aria-checked", "false");
    await userEvent.click(item);
    await waitFor(() => expect(useSettings.getState().settings?.goOffline).toBe(true));
    const h = await waitFor(async () => {
      const h = await mock.health();
      expect(h.banner?.kind).toBe("goneOffline");
      return h;
    });
    expect(h.cachedOnly).toBe(true);
    expect(offlineCallout(h)?.title).toBe("You chose Go offline.");
    expect(useDownloads.getState().status?.paused ?? (await mock.downloadsStatus()).paused).toBe("offline");
    await userEvent.click(screen.getByRole("button", { name: /App menu/ }));
    await userEvent.click(screen.getByRole("menuitemcheckbox", { name: "Go offline" }));
    await waitFor(async () => expect((await mock.health()).status).toBe("local"));
  });

  it("waits for the home network when downloads are home only", async () => {
    mock.downloads.connection = "remote";
    const st = await mock.downloadsSet("album", album.id, true);
    expect(st.paused).toBe("notHome");
    await mock.settingsSet({ downloadHomeOnly: false });
    await waitFor(async () => expect((await mock.downloadsStatus()).paused).toBe(null));
  });
});

describe("Settings", () => {
  it("shows storage, the limit, Remove all and the location", async () => {
    mock.downloads.seedDone("album", album.id, album.title);
    resetDownloads();
    await startDownloads();
    render(<SettingsPage />);
    const row = (key: string) => within(document.querySelector(`[data-setting="${key}"]`) as HTMLElement);
    await waitFor(() => expect(row("downloadsStorage").getByText(/^Downloads: \d/)).toBeInTheDocument());
    expect(row("downloadsStorage").getByRole("meter", { name: "Download storage used" })).toBeInTheDocument();
    expect(row("downloadFolder").getByText(/downloads$/)).toBeInTheDocument();
    await userEvent.click(row("downloadFolder").getByRole("button", { name: "Change location" }));
    await waitFor(() => expect(row("downloadFolder").getByText(/Platter downloads$/)).toBeInTheDocument());
    await userEvent.click(row("downloadsStorage").getByRole("button", { name: "Remove all downloads" }));
    const dialog = await screen.findByRole("dialog", { name: "Remove all downloads?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(mock.downloads.downloadedIds()).toEqual([]));
    await waitFor(() => expect(row("downloadsStorage").getByRole("button", { name: "Remove all downloads" })).toBeDisabled());
  });
});

describe("answers that arrive late", () => {
  // In the app a command's answer and the core's events travel separately, so
  // an answer can land after an event that is newer than it. These force
  // that order with answers the test hands over when it chooses.
  function later<T>() {
    let resolve!: (v: T) => void;
    const promise = new Promise<T>((r) => (resolve = r));
    return { promise, resolve };
  }
  const item = (state: DownloadItem["state"], done: number): DownloadItem => ({
    kind: "album",
    id: album.id,
    title: album.title,
    state,
    total: 1,
    done,
    failed: 0,
    missing: 0,
    progress: done,
    bytes: 0,
    error: null,
  });
  const status = (it: DownloadItem): DownloadStatus => ({ items: [it], usedBytes: 0, limitBytes: 1024 ** 3, folder: "downloads", paused: null, queued: 0 });
  const shown = () => findItem(useDownloads.getState().status, "album", album.id)?.state;

  it("Download's answer doesn't undo a finish the core sent before it", async () => {
    const answer = later<DownloadStatus>();
    vi.spyOn(backend, "downloadsSet").mockReturnValue(answer.promise);
    const asked = setDownload({ kind: "album", id: album.id, title: album.title }, true);
    act(() => mock.emit("downloads://status", status(item("downloaded", 1))));
    answer.resolve(status(item("downloading", 0)));
    await act(() => asked);
    expect(shown()).toBe("downloaded");
  });

  it("the status asked for at startup doesn't undo an event that came first", async () => {
    resetDownloads();
    const answer = later<DownloadStatus>();
    const ask = vi.spyOn(backend, "downloadsStatus").mockReturnValue(answer.promise);
    const starting = startDownloads();
    await waitFor(() => expect(ask).toHaveBeenCalled());
    act(() => mock.emit("downloads://status", status(item("downloaded", 1))));
    answer.resolve(status(item("downloading", 0)));
    await act(() => starting);
    expect(shown()).toBe("downloaded");
  });

  it("the downloaded marks keep the newest list when two reloads cross", async () => {
    const [a, b] = albumTracks.map((t) => t.id);
    const first = later<string[]>();
    const second = later<string[]>();
    const ask = vi.spyOn(backend, "downloadedTracks").mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    act(() => mock.emit("downloads://changed", null));
    act(() => mock.emit("downloads://changed", null));
    expect(ask).toHaveBeenCalledTimes(2);
    await act(async () => second.resolve([a, b]));
    await act(async () => first.resolve([a]));
    expect([...useDownloads.getState().ids].sort()).toEqual([a, b].sort());
  });
});
