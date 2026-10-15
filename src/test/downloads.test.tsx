/**
 * Downloads: the toggle on album, playlist and Liked Songs pages (outline,
 * progress ring with percent, filled), "Download complete", the remove
 * confirm, the downloaded mark on rows and library items, the Downloaded
 * chip (on by itself offline), Settings storage rows, Go offline, and the
 * app states: storage full, download failed, file missing on disk.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { DownloadDialogs, DownloadsBanner } from "../components/DownloadButton";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { LikedPage } from "../pages/LikedPage";
import { PlaylistPage } from "../pages/PlaylistPage";
import { SettingsPage } from "../pages/SettingsPage";
import { offlineMessage } from "../shell/bannerView";
import { Sidebar } from "../shell/Sidebar";
import { AppMenu } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { resetDownloads, startDownloads, useDownloads } from "../stores/downloads";
import { useLibraryView } from "../stores/library";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const album = mock.fx.albums[3];
const albumTracks = mock.fx.tracksByAlbum.get(album.id)!;

const button = () => screen.getByTestId("download-button");
const toasts = () => useToasts.getState().toasts.map((t) => t.message);

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

describe("album download", () => {
  it("goes from the outline to a ring with percent to the filled disc, then says it's complete", { timeout: 20_000 }, async () => {
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
    const done = await screen.findByRole("button", { name: `Remove download of ${album.title}` }, { timeout: 15_000 });
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
  it("downloads a playlist from its page", { timeout: 20_000 }, async () => {
    mock.downloads.steps = 1;
    mock.downloads.stepMs = 1;
    const pl = mock.fx.playlists.find((p) => (mock.fx.playlistTracks.get(p.id) ?? []).length > 0 && !p.smart)!;
    render(<PlaylistPage id={pl.id} />);
    await userEvent.click(await screen.findByRole("button", { name: `Download ${pl.title}` }));
    await screen.findByRole("button", { name: `Remove download of ${pl.title}` }, { timeout: 15_000 });
  });

  it("Liked Songs' toggle is the Download Liked Songs setting", async () => {
    render(<LikedPage />);
    await userEvent.click(await screen.findByRole("button", { name: "Download Liked Songs" }));
    await waitFor(() => expect(mock.downloads.isOn("liked", "")).toBe(true));
    expect((await mock.settingsGet()).downloadLiked).toBe(true);
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
    await waitFor(() => expect(button()).toHaveClass("dl-btn--error"), { timeout: 5000 });
    const retry = screen.getByRole("button", { name: new RegExp(`couldn't download\\. Download ${one.title} again`) });
    await userEvent.click(retry);
    await screen.findByRole("button", { name: `Remove download of ${one.title}` }, { timeout: 5000 });
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
    await screen.findByRole("button", { name: `Remove download of ${album.title}` }, { timeout: 5000 });
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
    expect(offlineMessage(h)).toMatch(/Turn off Go offline in the app menu/);
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
