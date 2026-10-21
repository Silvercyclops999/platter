/**
 * Keep my devices in sync on the phone: the quiet copy from the computer
 * after the first sign-in and how it fails, the status row on Devices with
 * Sync now, the two device-only rows on
 * the Sync page, and telling the core when the network changes.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import { followNetwork, meteredNow } from "../lib/deviceSyncWake";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { onPhone } from "../pages/settingsMeta";
import { usePhoneForm } from "../phone/form";
import { syncRowWords } from "../phone/pages/You";
import { PhoneShell } from "../phone/PhoneShell";
import { usePhoneUi } from "../phone/phoneUi";
import { copyInBackground } from "../phone/SyncSetup";
import { useApp } from "../stores/app";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

beforeEach(async () => {
  mock.deviceSync.reset();
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  useToasts.setState({ toasts: [] });
  await loadSettings();
  usePhoneForm.setState({ phone: true });
  usePhoneUi.setState({ tab: "home", nowPlaying: false, lyrics: false, sheets: [], crates: false, homeCard: false });
  useRouter.getState().reset({ name: "home" });
  act(() => useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } }));
});
afterEach(() => {
  usePhoneForm.setState({ phone: false });
  vi.restoreAllMocks();
});

const shell = () =>
  render(
    <>
      <PhoneShell />
      <Toaster />
    </>,
  );

/** The sync row lives on Devices, one tap from You. */
async function openYou() {
  shell();
  fireEvent.click(within(screen.getByRole("navigation", { name: "Tabs" })).getByRole("button", { name: "You" }));
  fireEvent.click(await within(await screen.findByTestId("phone-you")).findByRole("button", { name: "Devices" }));
  return within(await screen.findByTestId("you-sync", {}, { timeout: 3000 }));
}


describe("bringing the setup over after the first sign-in", () => {
  const toasts = () => useToasts.getState().toasts.map((t) => t.message);

  it("copies from the computer by itself, with no screen, and says what came", async () => {
    mock.deviceSync.addDevice("Studio PC");
    shell();
    copyInBackground();
    await waitFor(() => expect(toasts()).toContain("Brought your setup over from Studio PC"), { timeout: 3000 });
    expect((await backend.settingsGet()).accent).toBe("brass");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("waits for a computer that isn't linked yet, then copies", async () => {
    const copy = vi.spyOn(backend, "pairCopySettings");
    copyInBackground();
    await new Promise((r) => setTimeout(r, 30));
    expect(copy).not.toHaveBeenCalled();
    act(() => mock.deviceSync.addDevice("Studio PC"));
    await waitFor(() => expect(copy).toHaveBeenCalledTimes(1), { timeout: 3000 });
    await waitFor(() => expect(toasts()).toContain("Brought your setup over from Studio PC"));
  });

  it("stops waiting after a while; everyday sync brings things over later", async () => {
    const copy = vi.spyOn(backend, "pairCopySettings");
    copyInBackground(20);
    await new Promise((r) => setTimeout(r, 60));
    act(() => mock.deviceSync.addDevice("Studio PC"));
    await new Promise((r) => setTimeout(r, 60));
    expect(copy).not.toHaveBeenCalled();
  });

  it("stays quiet when the computer doesn't answer or sync is off", async () => {
    mock.deviceSync.addDevice("Studio PC");
    mock.deviceSync.copyFails = true;
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    copyInBackground();
    await waitFor(() => expect(warn).toHaveBeenCalledWith("first copy", expect.anything()), { timeout: 3000 });
    expect(toasts()).toEqual([]);
    mock.deviceSync.copyFails = false;
    await backend.settingsSet({ ...DEFAULT_SETTINGS, deviceSync: false });
    const copy = vi.spyOn(backend, "pairCopySettings");
    copyInBackground();
    await new Promise((r) => setTimeout(r, 30));
    expect(copy).not.toHaveBeenCalled();
    expect(toasts()).toEqual([]);
  });
});

describe("Devices on the phone", () => {
  it("says other devices show up by themselves, with no button to press", async () => {
    const row = await openYou();
    expect(await row.findByText("No other devices yet")).toBeInTheDocument();
    expect(row.getByText(/Open Platter on your computer with the same Plex account. It shows up here by itself/)).toBeInTheDocument();
    expect(row.queryByRole("button")).toBeNull();
    expect(screen.queryByText(/USB drive/)).toBeNull();
    expect(screen.getByRole("button", { name: "Downloads on this phone" })).toBeInTheDocument();
  });
});

describe("the sync row on Devices", () => {
  it("says the devices are in step, and Sync now syncs", async () => {
    mock.deviceSync.addDevice("Studio PC");
    const row = await openYou();
    expect(await row.findByText("In sync with Studio PC")).toBeInTheDocument();
    expect(row.getByText(/minute ago/i)).toBeInTheDocument();
    const now = vi.spyOn(backend, "deviceSyncNow");
    fireEvent.click(row.getByRole("button", { name: "Sync now" }));
    await waitFor(() => expect(now).toHaveBeenCalled());
    expect((await screen.findAllByText("In sync with Studio PC")).length).toBeGreaterThan(1);
  });

  it("names how a device that isn't in step is doing", async () => {
    mock.deviceSync.addDevice("Studio PC", { words: "Studio PC is away, changes are kept and will go when it's back", lastSyncedAt: Date.now() - 3_600_000 });
    mock.deviceSync.addDevice("Kitchen iPad", { lastSyncedAt: Date.now() - 7_200_000 });
    const row = await openYou();
    expect(await row.findByText("Studio PC is away, changes are kept and will go when it's back")).toBeInTheDocument();
    expect(row.getByText(/^Last in step .+ ago, and 1 more device$/i)).toHaveClass("is-warn");
  });

  it("says so when the status can't be read, and Retry reads it", async () => {
    mock.deviceSync.statusFails = true;
    const row = await openYou();
    expect(await row.findByText("Couldn't read sync status")).toBeInTheDocument();
    mock.deviceSync.statusFails = false;
    mock.deviceSync.addDevice("Studio PC");
    fireEvent.click(row.getByRole("button", { name: "Retry" }));
    expect(await row.findByText("In sync with Studio PC")).toBeInTheDocument();
  });

  it("each failure state reads as one plain line", () => {
    const d = { id: "1", name: "Studio PC", lastSyncedAt: null, updateOther: null, clock: null, held: 0 };
    for (const words of [
      "Waiting for Studio PC",
      "Studio PC is away, changes are kept and will go when it's back",
      "The relay is busy, trying again later",
      "Waiting for Studio PC on your home network",
      "Imported plays wait for Wi-Fi",
      "Catching up after a long time away",
      "This device is signed in to a different account. Sync paused",
      "Couldn't save changes from Studio PC",
      "Keep my devices in sync is off",
    ]) {
      expect(syncRowWords({ ...d, words }, 0)).toEqual({ title: words, sub: "Not in step yet", warn: true });
    }
    expect(syncRowWords({ ...d, words: "In sync" }, 2)).toEqual({ title: "In sync with Studio PC", sub: "Not in step yet, and 2 more devices", warn: false });
  });
});

describe("Sync settings on the phone", () => {
  it("has Sync when I'm away from home and Send big things only on Wi-Fi, both on, this device only", async () => {
    usePhoneForm.setState({ phone: false });
    render(<SettingsPage />);
    const away = await screen.findByRole("switch", { name: "Sync when I'm away from home" });
    const wifi = screen.getByRole("switch", { name: "Send big things only on Wi-Fi" });
    expect(away).toHaveAttribute("aria-checked", "true");
    expect(wifi).toHaveAttribute("aria-checked", "true");
    fireEvent.click(away);
    fireEvent.click(wifi);
    await waitFor(async () => expect(await backend.settingsGet()).toMatchObject({ deviceSyncAway: false, deviceSyncWifiOnly: false }));
    // Both show on the phone too.
    for (const key of ["deviceSync", "deviceSyncAway", "deviceSyncWifiOnly", "syncSettings", "syncPlays"]) expect(onPhone({ key, page: "sync" }), key).toBe(true);
  });
});

describe("the network changing", () => {
  it("tells the core, with mobile data when the web view can tell", async () => {
    const tell = vi.spyOn(backend, "deviceSyncNetwork");
    const stop = followNetwork();
    window.dispatchEvent(new Event("online"));
    expect(tell).toHaveBeenCalledWith(null);
    stop();
    window.dispatchEvent(new Event("online"));
    expect(tell).toHaveBeenCalledTimes(1);
    expect(meteredNow({ type: "cellular" })).toBe(true);
    expect(meteredNow({ type: "wifi" })).toBe(false);
    expect(meteredNow({ type: "unknown" })).toBeNull();
    expect(meteredNow(undefined)).toBeNull();
  });
});
