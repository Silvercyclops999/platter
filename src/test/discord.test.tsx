import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type DiscordStatus } from "../lib/types";
import { coversNote, discordNote, imgurNote } from "../pages/DiscordSettings";
import { SettingsPage } from "../pages/SettingsPage";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS });
  await backend.discordImgurKeySet(null);
  mock.discord.setAvailable(true);
  mock.discord.setConnected(true);
  await loadSettings();
  useToasts.setState({ toasts: [] });
});

function row(key: string): HTMLElement {
  const el = document.querySelector<HTMLElement>(`[data-setting="${key}"]`);
  if (!el) throw new Error(`no row ${key}`);
  return el;
}

/** A setting shown inside another row (the Imgur client ID under Cover art source). */
function part(key: string): HTMLElement {
  const el = document.querySelector<HTMLElement>(`[data-setting-part="${key}"]`);
  if (!el) throw new Error(`no part ${key}`);
  return el;
}

const status = (p: Partial<DiscordStatus>): DiscordStatus => ({
  available: true,
  connected: true,
  imgurKey: { saved: false, rejected: false },
  uploadFailed: false,
  ...p,
});

describe("Discord notes", () => {
  it("says when Discord isn't open, only while the status is on", () => {
    expect(discordNote(status({ connected: false }), true)).toBe("Discord isn't open. Platter shows your status once it is.");
    expect(discordNote(status({ connected: false }), false)).toBeUndefined();
    expect(discordNote(status({}), true)).toBeUndefined();
    expect(discordNote(status({ available: false }), true)).toMatch(/built without a Discord app/);
  });

  it("names a refused client ID before a failed upload", () => {
    expect(imgurNote(status({ uploadFailed: true, imgurKey: { saved: true, rejected: false } }))).toMatch(/couldn't be uploaded/);
    expect(imgurNote(status({ uploadFailed: true, imgurKey: { saved: true, rejected: true } }))).toBe("Imgur rejected the client ID.");
    expect(imgurNote(null)).toBeUndefined();
  });
});

describe("Discord settings", () => {
  it("has its rows on the Discord page with the defaults; unreleased choices sit under Sharing and friends", async () => {
    render(<SettingsPage />);
    const group = await screen.findByRole("region", { name: "Integrations" });
    expect(within(group).getAllByRole("heading", { level: 3, name: "Discord" })[0]).toBeInTheDocument();
    expect(within(group).getByRole("switch", { name: "Show what you're playing on Discord" })).toBeChecked();
    expect(within(group).getByRole("switch", { name: "Clear status when paused" })).toBeChecked();
    expect(within(group).getByRole("switch", { name: "Find this song button" })).toBeChecked();
    // The four cover rows are one choice: Cover Art Archive on, uploads off.
    expect(within(group).getByRole("combobox", { name: "Cover art source" })).toHaveValue("archive");
    expect(within(row("discordCoverSource")).getByText(/Anyone with the link can see/)).toBeInTheDocument();
    expect(within(row("discordCoverSource")).getByText(/Litterbox keeps a cover 3 days/)).toBeInTheDocument();
    expect(within(group).queryByLabelText("Imgur client ID")).toBeNull();
    expect(screen.getByRole("combobox", { name: "Discord status shows" })).toHaveValue("app");
    const sharing = screen.getByRole("region", { name: "Sharing and friends" });
    expect(within(sharing).getByRole("switch", { name: "Show unreleased music on Discord" })).toBeChecked();
    expect(within(sharing).getByRole("switch", { name: "Show unreleased cover art on Discord" })).toBeChecked();
  });

  it("the cover art source, its host and the status line save their settings", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.selectOptions(await screen.findByRole("combobox", { name: "Cover art source" }), "archiveThenUpload");
    expect(useSettings.getState().settings).toMatchObject({ discordCoverArchive: true, discordUploadCovers: true });
    await user.selectOptions(await screen.findByRole("combobox", { name: "Image host" }), "x0");
    expect(useSettings.getState().settings?.discordCoverHost).toBe("x0");
    await user.selectOptions(screen.getByRole("combobox", { name: "Cover art source" }), "upload");
    expect(useSettings.getState().settings).toMatchObject({ discordCoverArchive: false, discordUploadCovers: true });
    await user.selectOptions(screen.getByRole("combobox", { name: "Cover art source" }), "none");
    expect(useSettings.getState().settings).toMatchObject({ discordCoverArchive: false, discordUploadCovers: false, discordUploadArt: false });
    expect(screen.queryByRole("combobox", { name: "Image host" })).toBeNull();
    await user.selectOptions(screen.getByRole("combobox", { name: "Discord status shows" }), "artist");
    expect(useSettings.getState().settings?.discordStatusLine).toBe("artist");
  });

  it("a failed upload is noted on the host that was used", () => {
    const failed = status({ uploadFailed: true });
    expect(coversNote(failed, true)).toMatch(/couldn't be uploaded/);
    expect(coversNote(failed, false)).toBeUndefined();
    expect(imgurNote(failed)).toBeUndefined();
    const imgur = status({ uploadFailed: true, imgurKey: { saved: true, rejected: false } });
    expect(coversNote(imgur, true)).toBeUndefined();
    expect(imgurNote(imgur)).toMatch(/couldn't be uploaded/);
  });

  it("each switch saves its setting", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(await screen.findByRole("switch", { name: "Clear status when paused" }));
    expect(useSettings.getState().settings?.discordClearOnPause).toBe(false);
    await user.click(screen.getByRole("switch", { name: "Show unreleased music on Discord" }));
    expect(useSettings.getState().settings?.discordShowUnreleased).toBe(false);
    await user.click(screen.getByRole("switch", { name: "Show what you're playing on Discord" }));
    expect(useSettings.getState().settings?.discordStatus).toBe(false);
    // Its other rows wait until the status is on again.
    await waitFor(() => expect(document.querySelector('[data-setting="discordClearOnPause"]')).toBeNull());
  });

  it("a build without a Discord app shows one row with a note and no controls", async () => {
    mock.discord.setAvailable(false);
    render(<SettingsPage />);
    expect(await screen.findByText(/built without a Discord app/)).toBeInTheDocument();
    expect(within(row("discordStatus")).queryByRole("switch")).toBeNull();
    expect(document.querySelector('[data-setting="discordCoverSource"]')).toBeNull();
    expect(document.querySelector('[data-setting="discordClearOnPause"]')).toBeNull();
  });

  it("saving a client ID turns uploads on; removing it turns them off", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.selectOptions(await screen.findByRole("combobox", { name: "Cover art source" }), "archiveThenUpload");
    await user.type(await screen.findByLabelText("Imgur client ID"), "my-client");
    await user.click(within(part("discordUploadArt")).getByRole("button", { name: "Save ID" }));
    expect(await within(part("discordUploadArt")).findByRole("switch", { name: "Upload to Imgur first" })).toBeChecked();
    expect(useSettings.getState().settings?.discordUploadArt).toBe(true);
    expect(screen.queryByDisplayValue("my-client")).toBeNull();
    await user.click(within(part("discordUploadArt")).getByRole("button", { name: "Remove ID" }));
    expect(await screen.findByLabelText("Imgur client ID")).toBeInTheDocument();
    expect(useSettings.getState().settings?.discordUploadArt).toBe(false);
  });

  it("a refused client ID shows an error on its row and stays off", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.selectOptions(await screen.findByRole("combobox", { name: "Cover art source" }), "archiveThenUpload");
    await user.type(await screen.findByLabelText("Imgur client ID"), "bad-client");
    await user.click(within(part("discordUploadArt")).getByRole("button", { name: "Save ID" }));
    expect(await within(part("discordUploadArt")).findByRole("alert")).toHaveTextContent("Imgur rejected the client ID.");
    expect(within(part("discordUploadArt")).getByRole("switch")).not.toBeChecked();
    expect(useSettings.getState().settings?.discordUploadArt).toBe(false);
  });

  it("the Find this song row points at public links when they're off", async () => {
    await backend.settingsSet({ sharePublicLinks: false });
    await loadSettings();
    render(<SettingsPage />);
    expect(await within(row("discordFindSong")).findByText("Turn on Public links for released music to use this.")).toBeInTheDocument();
  });
});

describe("Discord app states", () => {
  it("Discord not running: the status row says so and updates when Discord opens", async () => {
    mock.discord.setConnected(false);
    render(<SettingsPage />);
    expect(await within(await screen.findByRole("region", { name: "Integrations" })).findByText(/Discord isn't open/)).toBeInTheDocument();
    await act(async () => mock.discord.setConnected(true));
    expect(screen.queryByText(/Discord isn't open/)).toBeNull();
  });

  it("image upload failed: the Imgur row says Discord shows the Platter icon", async () => {
    await backend.discordImgurKeySet("my-client");
    await backend.settingsSet({ discordUploadArt: true });
    await loadSettings();
    render(<SettingsPage />);
    await screen.findByRole("switch", { name: "Upload to Imgur first" });
    await act(async () => mock.discord.failUpload());
    expect(await within(part("discordUploadArt")).findByRole("alert")).toHaveTextContent(/couldn't be uploaded, so Discord shows the Platter icon/);
  });
});
