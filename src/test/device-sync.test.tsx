/**
 * Settings > Sync > Keep my devices in sync: the switch and its note, the
 * paired devices, adding a device (the codes to show), pairing with a code
 * (6 digits at home, the long code away) and every way it fails, unpairing,
 * Sync now, held changes and what to sync. The phone's Copy from my
 * computer is in phone-sync.test.tsx.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { usePhoneForm } from "../phone/form";
import { PhoneShell } from "../phone/PhoneShell";
import { useApp } from "../stores/app";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

const row = (title: string | RegExp) => {
  const el = screen.getByText(title, { selector: ".setting-row__title" }).closest(".setting-row");
  if (!el) throw new Error(`no row ${title}`);
  return within(el as HTMLElement);
};

beforeEach(async () => {
  mock.deviceSync.reset();
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  await loadSettings();
});
afterEach(() => vi.restoreAllMocks());

describe("Keep my devices in sync", () => {
  it("is on by default, says devices show up by themselves, and asks for no code", async () => {
    render(<SettingsPage />);
    const sw = await screen.findByRole("switch", { name: "Keep my devices in sync" });
    expect(sw).toHaveAttribute("aria-checked", "true");
    await waitFor(() => expect(row("Keep my devices in sync").getByText("Your other devices signed in to this Plex account show up here by themselves.")).toBeInTheDocument());
    expect(screen.queryByRole("button", { name: "Sync now" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Show codes" })).toBeNull();
    expect(document.querySelector('[data-setting="deviceSyncAdd"], [data-setting="deviceSyncJoin"]')).toBeNull();
  });

  it("says when the sync status can't be read, and Retry reads it again", async () => {
    const user = userEvent.setup();
    mock.deviceSync.statusFails = true;
    render(<SettingsPage />);
    await waitFor(() => expect(row("Keep my devices in sync").getByText("Couldn't read sync status.")).toBeInTheDocument());
    mock.deviceSync.statusFails = false;
    await user.click(row("Couldn't read sync status").getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(row("Keep my devices in sync").getByText("Your other devices signed in to this Plex account show up here by themselves.")).toBeInTheDocument());
    expect(screen.queryByText("Couldn't read sync status", { selector: ".setting-row__title" })).toBeNull();
  });

  it("unpairing asks first, then the device is gone", async () => {
    const user = userEvent.setup();
    mock.deviceSync.addDevice("Kitchen iPad");
    render(
      <>
        <SettingsPage />
        <Toaster />
      </>,
    );
    await user.click(await waitFor(() => row("Kitchen iPad").getByRole("button", { name: "Unpair" })));
    const dialog = await screen.findByRole("dialog", { name: "Unpair Kitchen iPad" });
    expect(dialog).toHaveTextContent("Everything already on each stays there");
    await user.click(within(dialog).getByRole("button", { name: "Unpair" }));
    await waitFor(() => expect(screen.queryByText("Kitchen iPad", { selector: ".setting-row__title" })).toBeNull());
    expect(await screen.findByText("Kitchen iPad is no longer paired")).toBeInTheDocument();
  });

  it("app states: away, an older Platter there, clocks far apart, and changes held back", async () => {
    const user = userEvent.setup();
    mock.deviceSync.addDevice("Studio PC", { words: "Studio PC is away, changes are kept and will go when it's back", lastSyncedAt: null });
    mock.deviceSync.addDevice("Kitchen iPad", { updateOther: "Update Platter on Kitchen iPad to sync everything", clock: "Kitchen iPad's clock is 3 days ahead", held: 2 });
    mock.deviceSync.hold(2, "Kitchen iPad");
    render(<SettingsPage />);
    await waitFor(() => expect(row("Studio PC").getByText(/is away, changes are kept/)).toBeInTheDocument());
    expect(row("Keep my devices in sync").getByText(/Studio PC is away/)).toBeInTheDocument();
    expect(row("Kitchen iPad").getByText(/Update Platter on Kitchen iPad to sync everything. Kitchen iPad's clock is 3 days ahead. 2 changes from it held back./)).toBeInTheDocument();
    expect(row("Changes held back").getByText("2 changes came from a device whose clock is days ahead.")).toBeInTheDocument();
    await user.click(row("Changes held back").getByRole("button", { name: "Take them anyway" }));
    await waitFor(() => expect(screen.queryByText("Changes held back", { selector: ".setting-row__title" })).toBeNull());
  });

  it("Sync now runs a round; with the switch off nothing syncs and the devices say so", async () => {
    const user = userEvent.setup();
    const now = vi.spyOn(backend, "deviceSyncNow");
    mock.deviceSync.addDevice("Studio PC", { words: "Waiting for Studio PC" });
    render(<SettingsPage />);
    await user.click(await waitFor(() => row("Sync now").getByRole("button", { name: "Sync now" })));
    expect(now).toHaveBeenCalled();
    await waitFor(() => expect(row("Studio PC").getByText(/^In sync/)).toBeInTheDocument());
    // It says how it went.
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("In sync with Studio PC"));
    await user.click(screen.getByRole("switch", { name: "Keep my devices in sync" }));
    await waitFor(async () => expect((await backend.settingsGet()).deviceSync).toBe(false));
    await waitFor(() => expect(row("Keep my devices in sync").getByText("Off. Your paired devices stay paired.")).toBeInTheDocument());
    expect(row("Studio PC").getByText("Keep my devices in sync is off")).toBeInTheDocument();
    expect(row("Sync now").getByRole("button", { name: "Sync now" })).toBeDisabled();
    await expect(mock.deviceSyncNow()).rejects.toMatchObject({ message: "Turn on Keep my devices in sync first." });
  });

  it("what to sync covers looks, downloads and imported plays, saved per device", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    for (const [name, key] of [
      ["Saved looks and themes", "syncLooks"],
      ["What each device downloaded", "syncDownloads"],
      ["Imported plays", "syncPlays"],
    ] as const) {
      const sw = screen.getByRole("switch", { name });
      expect(sw).toHaveAttribute("aria-checked", "true");
      await user.click(sw);
      await waitFor(async () => expect((await backend.settingsGet())[key]).toBe(false));
    }
    expect(row("What each device downloaded").getByText(/Nothing downloads by itself/)).toBeInTheDocument();
    expect(row("Saved looks and themes").getByText(/Each device keeps its own layout/)).toBeInTheDocument();
  });

});

describe("downloads on your other devices", () => {
  it("are offered with one tap, and Not now hides one here", async () => {
    const user = userEvent.setup();
    const set = vi.spyOn(backend, "downloadsSet");
    const album = mock.fx.albums[0];
    mock.deviceSync.suggest("album", album.id, album.title, ["Studio PC", "Kitchen iPad"]);
    mock.deviceSync.suggest("playlist", "p-none", "Road Trip");
    render(
      <>
        <SettingsPage />
        <Toaster />
      </>,
    );
    await waitFor(() => expect(row(album.title).getByText("Downloaded on Studio PC and Kitchen iPad")).toBeInTheDocument());
    // Nothing downloaded by itself.
    expect(set).not.toHaveBeenCalled();
    await user.click(row("Road Trip").getByRole("button", { name: "Not now" }));
    await waitFor(() => expect(screen.queryByText("Road Trip", { selector: ".setting-row__title" })).toBeNull());
    expect(mock.deviceSync.suggestions.some((x) => x.id === "p-none")).toBe(false);
    await user.click(row(album.title).getByRole("button", { name: "Download here" }));
    expect(set).toHaveBeenCalledWith("album", album.id, true, expect.objectContaining({ title: album.title }));
    expect((await screen.findAllByText(`Downloading ${album.title}`))[0]).toBeInTheDocument();
  });

  it("a suggestion that couldn't be hidden comes back, and a failed read keeps the list", async () => {
    const user = userEvent.setup();
    mock.deviceSync.suggest("playlist", "p-none", "Road Trip");
    render(
      <>
        <SettingsPage />
        <Toaster />
      </>,
    );
    await waitFor(() => expect(row("Road Trip").getByText("Downloaded on Studio PC")).toBeInTheDocument());
    vi.spyOn(backend, "deviceDownloadDismiss").mockRejectedValueOnce({ kind: "internal", message: "disk full" });
    vi.spyOn(backend, "deviceDownloadSuggestions").mockRejectedValue({ kind: "internal", message: "locked" });
    await user.click(row("Road Trip").getByRole("button", { name: "Not now" }));
    expect((await screen.findAllByText(/Couldn't hide that suggestion/))[0]).toBeInTheDocument();
    await waitFor(() => expect(row("Road Trip").getByText("Downloaded on Studio PC")).toBeInTheDocument());
  });

  it("aren't offered while What each device downloaded is off", async () => {
    mock.deviceSync.suggest("album", "a1", "Blue");
    await backend.settingsSet({ ...DEFAULT_SETTINGS, syncDownloads: false });
    await loadSettings();
    const ask = vi.spyOn(backend, "deviceDownloadSuggestions");
    render(<SettingsPage />);
    await screen.findByRole("switch", { name: "Keep my devices in sync" });
    expect(screen.queryByText("Blue", { selector: ".setting-row__title" })).toBeNull();
    expect(ask).not.toHaveBeenCalled();
  });

  it("show in the phone's Settings, where the old Downloads page now leads", async () => {
    usePhoneForm.setState({ phone: true });
    try {
      mock.deviceSync.suggest("playlist", "p1", "Road Trip");
      act(() => useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } }));
      useRouter.getState().reset({ name: "downloads" });
      render(<PhoneShell />);
      await waitFor(() => expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "settings", section: "Downloads and devices" }));
      const title = await screen.findByText("Road Trip", { selector: ".setting-row__title" }, { timeout: 6000 });
      const row = title.closest(".setting-row") as HTMLElement;
      expect(within(row).getByText("Downloaded on Studio PC")).toBeInTheDocument();
      expect(within(row).getByRole("button", { name: "Download here" })).toBeInTheDocument();
    } finally {
      usePhoneForm.setState({ phone: false });
    }
  });
});
