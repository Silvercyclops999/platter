import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type DiscordStatus } from "../lib/types";
import { discordNote, imgurNote } from "../pages/DiscordSettings";
import { SettingsPage } from "../pages/SettingsPage";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
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
    expect(imgurNote(status({ uploadFailed: true }))).toMatch(/couldn't be uploaded/);
    expect(imgurNote(status({ uploadFailed: true, imgurKey: { saved: true, rejected: true } }))).toBe("Imgur rejected the client ID.");
    expect(imgurNote(null)).toBeUndefined();
  });
});

describe("Discord settings", () => {
  it("has its rows under Integrations with the defaults", async () => {
    render(<SettingsPage />);
    const group = await screen.findByRole("region", { name: "Integrations" });
    expect(within(group).getByRole("heading", { level: 3, name: "Discord" })).toBeInTheDocument();
    expect(within(group).getByRole("switch", { name: "Show what you're playing on Discord" })).toBeChecked();
    expect(within(group).getByRole("switch", { name: "Clear status when paused" })).toBeChecked();
    expect(within(group).getByRole("switch", { name: "Show unreleased songs" })).toBeChecked();
    expect(within(group).getByRole("switch", { name: "Show unreleased cover art" })).toBeChecked();
    expect(within(group).getByRole("switch", { name: "Find this song button" })).toBeChecked();
    expect(within(group).getByRole("switch", { name: "Covers from Cover Art Archive" })).toBeChecked();
    expect(await within(group).findByLabelText("Imgur client ID")).toBeInTheDocument();
  });

  it("each switch saves its setting", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(await screen.findByRole("switch", { name: "Clear status when paused" }));
    expect(useSettings.getState().settings?.discordClearOnPause).toBe(false);
    await user.click(screen.getByRole("switch", { name: "Show unreleased songs" }));
    expect(useSettings.getState().settings?.discordShowUnreleased).toBe(false);
    await user.click(screen.getByRole("switch", { name: "Show what you're playing on Discord" }));
    expect(useSettings.getState().settings?.discordStatus).toBe(false);
  });

  it("a build without a Discord app shows one row with a note and no controls", async () => {
    mock.discord.setAvailable(false);
    render(<SettingsPage />);
    expect(await screen.findByText(/built without a Discord app/)).toBeInTheDocument();
    expect(within(row("discordStatus")).queryByRole("switch")).toBeNull();
    expect(document.querySelector('[data-setting="discordUploadArt"]')).toBeNull();
    expect(document.querySelector('[data-setting="discordClearOnPause"]')).toBeNull();
  });

  it("saving a client ID turns uploads on; removing it turns them off", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.type(await screen.findByLabelText("Imgur client ID"), "my-client");
    await user.click(within(row("discordUploadArt")).getByRole("button", { name: "Save ID" }));
    expect(await within(row("discordUploadArt")).findByRole("switch", { name: "Upload other covers to Imgur" })).toBeChecked();
    expect(useSettings.getState().settings?.discordUploadArt).toBe(true);
    expect(screen.queryByDisplayValue("my-client")).toBeNull();
    await user.click(within(row("discordUploadArt")).getByRole("button", { name: "Remove ID" }));
    expect(await screen.findByLabelText("Imgur client ID")).toBeInTheDocument();
    expect(useSettings.getState().settings?.discordUploadArt).toBe(false);
  });

  it("a refused client ID shows an error on its row and stays off", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.type(await screen.findByLabelText("Imgur client ID"), "bad-client");
    await user.click(within(row("discordUploadArt")).getByRole("button", { name: "Save ID" }));
    expect(await within(row("discordUploadArt")).findByRole("alert")).toHaveTextContent("Imgur rejected the client ID.");
    expect(within(row("discordUploadArt")).getByRole("switch")).not.toBeChecked();
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
    render(<SettingsPage />);
    await screen.findByRole("switch", { name: "Upload other covers to Imgur" });
    await act(async () => mock.discord.failUpload());
    expect(within(row("discordUploadArt")).getByRole("alert")).toHaveTextContent(/couldn't be uploaded, so Discord shows the Platter icon/);
  });
});
