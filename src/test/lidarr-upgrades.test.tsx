/**
 * Lidarr upgrades in the UI: "Look for a better copy" in the album menu
 * (confirming the release first), the offer card on the album page with
 * what upgrades and what is kept, the swap dialog (all at once, or song by
 * song when the album needs it), Undo, the failure messages, and the
 * Settings rows that stay hidden until a staging folder is set.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MOCK_STAGING, type UpgradeDemo } from "../lib/mock/upgrades";
import { DEFAULT_SETTINGS } from "../lib/types";
import { ContextMenuHost } from "../components/ContextMenu";
import { AlbumPage } from "../pages/AlbumPage";
import { SettingsPage } from "../pages/SettingsPage";
import { useApp } from "../stores/app";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
let album = () => mock.fx.albums[0];
/** An album whose songs are all lossy, so a better copy exists. */
const lossy = () =>
  mock.fx.albums.find((a) => {
    const ts = mock.fx.tracksByAlbum.get(a.id) ?? [];
    return a.format !== null && !a.subformats.length && ts.length > 2 && ts.every((t) => t.codec === "mp3" && t.title !== a.title);
  })!;

async function setUp(demo: UpgradeDemo | undefined, patch: Record<string, unknown> = {}) {
  mock.lidarr.reset("ready");
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: "http://10.20.30.40:8686", lidarrStagingFolder: MOCK_STAGING, ...patch });
  mock.upgrades.reset(demo);
  // The demo's album: the one with the upgrade.
  if (demo) {
    const id = mock.upgrades.records[0].albumId;
    album = () => mock.fx.albums.find((a) => a.id === id)!;
  }
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(() => {
  album = () => mock.fx.albums[0];
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
});

async function openMenu(user: ReturnType<typeof userEvent.setup>) {
  await screen.findByRole("heading", { level: 1, name: album().title });
  await user.click(screen.getByRole("button", { name: `More options for ${album().title}` }));
}

describe("Look for a better copy", () => {
  it("confirms the release with its tracklist, then asks Lidarr into staging", async () => {
    const user = userEvent.setup();
    album = lossy;
    await setUp(undefined);
    render(
      <>
        <AlbumPage id={album().id} />
        <ContextMenuHost />
      </>,
    );
    await openMenu(user);
    await user.click(await screen.findByRole("menuitem", { name: "Look for a better copy" }));
    const dialog = await screen.findByRole("dialog", { name: `Look for a better copy of ${album().title}` });
    expect(within(dialog).getByRole("heading", { level: 2 })).toHaveTextContent(`Which release is ${album().title}?`);
    // The album's songs beside the official tracklist; the deluxe edition has one more.
    const first = mock.fx.tracksByAlbum.get(album().id)![0].title;
    const n = within(dialog).getAllByText(first).length;
    expect(n >= 2 && n % 2 === 0).toBe(true);
    await user.click(within(dialog).getByRole("radio", { name: new RegExp(`${album().title} \\(Deluxe Edition\\)`) }));
    expect(await within(dialog).findByText("Bonus Track")).toBeInTheDocument();
    await user.click(within(dialog).getAllByRole("radio")[0]);
    await user.click(within(dialog).getByRole("button", { name: "This is the album" }));
    // Then the request, into the staging folder.
    expect(await within(dialog).findByText(MOCK_STAGING)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Look for it" }));
    await waitFor(() => expect(mock.upgrades.records).toHaveLength(1));
    expect(mock.upgrades.records[0].destination).toContain(MOCK_STAGING);
    expect(useToasts.getState().toasts.map((t) => t.message)).toContain(`Lidarr is looking for a better copy of ${album().title}`);
    // The card says it's on its way.
    expect(await screen.findByText("Looking for a better copy")).toBeInTheDocument();
  });

  it("isn't offered without a staging folder", async () => {
    const user = userEvent.setup();
    album = lossy;
    await setUp(undefined, { lidarrStagingFolder: "" });
    render(
      <>
        <AlbumPage id={album().id} />
        <ContextMenuHost />
      </>,
    );
    await openMenu(user);
    await screen.findByRole("menuitem", { name: "Add to queue" });
    expect(screen.queryByRole("menuitem", { name: "Look for a better copy" })).toBeNull();
  });

  it("can't be asked for before the release is confirmed", async () => {
    await setUp(undefined);
    await expect(mock.lidarrUpgradeRequest(album().id)).rejects.toMatchObject({ message: "Confirm which MusicBrainz release this album is first." });
  });
});

describe("the offer and the swap", () => {
  it("shows what upgrades and what is kept, swaps with the dialog, and undoes from the toast", async () => {
    const user = userEvent.setup();
    await setUp("ready");
    render(
      <>
        <AlbumPage id={album().id} />
        <ContextMenuHost />
      </>,
    );
    const card = await screen.findByRole("region", { name: "Better copy" });
    expect(await within(card).findByText(/^Upgrade \d+ of \d+ songs \((MP3|AAC) \d+ to FLAC\)$/)).toBeInTheDocument();
    expect(within(card).getByRole("heading", { name: "Upgrades" })).toBeInTheDocument();
    expect(within(card).getByRole("heading", { name: "Kept as they are" })).toBeInTheDocument();
    expect(within(card).getByText("The new copy isn't better")).toBeInTheDocument();
    expect(within(card).getByText(/Old files go to a _to_delete folder/)).toBeInTheDocument();
    await user.click(within(card).getByRole("button", { name: "Swap files…" }));
    const dialog = await screen.findByRole("dialog", { name: /Swap in the better copy/ });
    const boxes = within(dialog).getAllByRole("checkbox");
    expect(boxes.every((b) => (b as HTMLInputElement).checked)).toBe(true);
    await user.click(within(dialog).getByRole("button", { name: `Swap ${boxes.length} songs` }));
    await within(card).findByText(/^Upgraded \d+ songs/);
    expect(within(card).getByText(/Old files are in .*_to_delete/)).toBeInTheDocument();
    const toast = useToasts.getState().toasts.find((t) => t.action?.label === "Undo");
    expect(toast?.message).toMatch(/^Upgraded \d+ songs/);
    // Undo from the card.
    await user.click(within(card).getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(mock.upgrades.records[0].swaps[0].undone).toBe(true));
  });

  it("asks song by song when the folder holds anything kept for a reason", async () => {
    const user = userEvent.setup();
    await setUp("mixed");
    render(
      <>
        <AlbumPage id={album().id} />
        <ContextMenuHost />
      </>,
    );
    const card = await screen.findByRole("region", { name: "Better copy" });
    expect(await within(card).findByText("Unreleased")).toBeInTheDocument();
    expect(within(card).getByText("Not one of this album's songs in Plex")).toBeInTheDocument();
    await user.click(within(card).getByRole("button", { name: "Swap files…" }));
    const dialog = await screen.findByRole("dialog", { name: /Swap in the better copy/ });
    expect(within(dialog).getByText(/asks about each song/)).toBeInTheDocument();
    const boxes = within(dialog).getAllByRole("checkbox");
    expect(boxes.every((b) => !(b as HTMLInputElement).checked)).toBe(true);
    expect(within(dialog).getByRole("button", { name: "Swap 0 songs" })).toBeDisabled();
    await user.click(boxes[0]);
    await user.click(within(dialog).getByRole("button", { name: "Swap 1 song" }));
    await waitFor(() => expect(mock.upgrades.records[0].swaps).toHaveLength(1));
    expect(mock.upgrades.records[0].swaps[0].files).toHaveLength(1);
  });

  it("shows a failed swap plainly and keeps the offer", async () => {
    const user = userEvent.setup();
    await setUp("locked");
    render(
      <>
        <AlbumPage id={album().id} />
        <ContextMenuHost />
      </>,
    );
    const card = await screen.findByRole("region", { name: "Better copy" });
    await user.click(await within(card).findByRole("button", { name: "Swap files…" }));
    const dialog = await screen.findByRole("dialog", { name: /Swap in the better copy/ });
    await user.click(within(dialog).getByRole("button", { name: /^Swap \d+ songs$/ }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("is in use by another program. Close it, then try again. Nothing was changed.");
    expect(mock.upgrades.records[0].swaps).toHaveLength(0);
  });

  it("points to Settings when the share mapping isn't set", async () => {
    await setUp("noMap");
    render(
      <>
        <AlbumPage id={album().id} />
        <ContextMenuHost />
      </>,
    );
    const card = await screen.findByRole("region", { name: "Better copy" });
    expect(await within(card).findByText(/Set where this computer reaches your music share/)).toBeInTheDocument();
    expect(within(card).getByRole("button", { name: "Open Settings" })).toBeInTheDocument();
  });
});

describe("Settings > Lidarr upgrades", () => {
  it("hides the upgrade rows until a staging folder is set, and saves it", async () => {
    const user = userEvent.setup();
    await setUp(undefined, { lidarrStagingFolder: "" });
    render(<SettingsPage />);
    const field = await screen.findByRole("textbox", { name: "Lidarr staging folder for upgrades" });
    expect(document.querySelector('[data-setting="upgradesAsk"]')).toBeNull();
    expect(document.querySelector('[data-setting="musicShare"]')).toBeNull();
    await user.type(field, "/downloads/upgrades/{Enter}");
    await waitFor(() => expect(useSettings.getState().settings?.lidarrStagingFolder).toBe("/downloads/upgrades"));
    await waitFor(() => expect(document.querySelector('[data-setting="upgradesAsk"]')).not.toBeNull());
    expect(document.querySelector('[data-setting="upgradeOfficialPrefix"]')).not.toBeNull();
    expect(document.querySelector('[data-setting="lidarrUpgrades"]')).not.toBeNull();
  });

  it("keeps the share mapping on this computer and refuses a backup folder inside the music", async () => {
    const user = userEvent.setup();
    await setUp(undefined);
    render(<SettingsPage />);
    const local = await screen.findByRole("textbox", { name: "On this computer" });
    expect(screen.getByRole("textbox", { name: "Plex's music folder" })).toHaveValue("/data/media/music");
    await user.type(local, "M:\\Music");
    await user.type(screen.getByRole("textbox", { name: "Backup folder for replaced files" }), "M:\\Music\\Old");
    await user.click(screen.getByRole("button", { name: "Save folders" }));
    expect(await screen.findByText(/Pick a backup folder outside your music folder/)).toBeInTheDocument();
    await user.clear(screen.getByRole("textbox", { name: "Backup folder for replaced files" }));
    await user.click(screen.getByRole("button", { name: "Save folders" }));
    expect(await screen.findByText("Saved on this computer.")).toBeInTheDocument();
    expect(mock.upgrades.share.music).toEqual([{ server: "/data/media/music", local: "M:\\Music" }]);
  });

  it("lists upgrades and looks for better copies of confirmed albums in bulk", async () => {
    const user = userEvent.setup();
    await setUp("ready");
    render(<SettingsPage />);
    const list = await screen.findByRole("list", { name: "Upgrades" });
    expect(within(list).getByText("Ready to swap")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Look for better copies" }));
    const dialog = await screen.findByRole("dialog", { name: "Look for better copies" });
    await user.click(within(dialog).getByRole("button", { name: "Look for them" }));
    await waitFor(() => expect(useToasts.getState().toasts.length).toBeGreaterThan(0));
  });
});
