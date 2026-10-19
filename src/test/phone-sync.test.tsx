/**
 * Keep my devices in sync on the phone: Copy from my computer (the setup
 * step after signing in, and from You), its progress and Stop, every way it
 * fails, the status row on You with Sync now, the two device-only rows on
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
import { closeSyncStep, openSyncStep, progressLine } from "../phone/SyncSetup";
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
  act(() => closeSyncStep());
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

async function openYou() {
  shell();
  fireEvent.click(within(screen.getByRole("navigation", { name: "Tabs" })).getByRole("button", { name: "You" }));
  return within(await screen.findByTestId("you-sync", {}, { timeout: 3000 }));
}

const step = () => within(screen.getByTestId("phone-sync-step"));

describe("Copy from my computer", () => {
  it("before pairing, asks for the computer's code, pairs, copies and says what came", async () => {
    const row = await openYou();
    fireEvent.click(await row.findByRole("button", { name: /Copy settings from my computer/ }));
    expect(await screen.findByRole("heading", { name: "Type the code from your computer" })).toBeInTheDocument();
    const input = step().getByLabelText("Code");
    const pairBtn = step().getByRole("button", { name: "Pair and copy" });
    expect(pairBtn).toBeDisabled();
    // A wrong code says so and stays.
    fireEvent.change(input, { target: { value: "111111" } });
    fireEvent.click(pairBtn);
    expect(await step().findByRole("alert")).toHaveTextContent("That code doesn't match. Check the code on the other device.");
    fireEvent.change(input, { target: { value: mock.deviceSync.otherCode } });
    fireEvent.click(step().getByRole("button", { name: "Pair and copy" }));
    expect(await screen.findByRole("heading", { name: "In sync with Studio PC" }, { timeout: 3000 })).toBeInTheDocument();
    const copied = within(step().getByRole("list", { name: "Copied" }));
    expect(copied.getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "Settings48",
      "Pins and folders12",
      "Follows and hidden items31",
      "Lyrics4",
      "Looks2",
      "Download choices9",
      "Imported plays1,840",
    ]);
    expect((await backend.settingsGet()).accent).toBe("brass");
    fireEvent.click(step().getByRole("button", { name: "Done" }));
    expect(screen.queryByTestId("phone-sync-step")).toBeNull();
    // You now says how the two are doing.
    await waitFor(() => expect(row.getByText("In sync with Studio PC")).toBeInTheDocument());
    expect(row.getByRole("button", { name: "Sync now" })).toBeInTheDocument();
  });

  it("shows where it is, and Stop keeps what came", async () => {
    mock.deviceSync.addDevice("Studio PC");
    mock.deviceSync.copyHangs = 2;
    act(() => openSyncStep(false));
    shell();
    await waitFor(() => expect(step().getByRole("status")).toHaveTextContent("Copying your pins and folders… 2 of 7"));
    expect(step().getByText("Copying from Studio PC")).toBeInTheDocument();
    expect(step().getByRole("progressbar", { name: "Copied so far" })).toHaveAttribute("aria-valuenow", "1");
    fireEvent.click(step().getByRole("button", { name: "Stop" }));
    expect(await screen.findByRole("heading", { name: "Copy stopped" })).toBeInTheDocument();
    expect(step().getByText(/What came over stays. The rest comes from Studio PC/)).toBeInTheDocument();
    const copied = within(step().getByRole("list", { name: "Copied" }));
    expect(copied.getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Settings48", "Pins and foldersNone"]);
  });

  it("says when the computer isn't on this network and the rest follows", async () => {
    mock.deviceSync.addDevice("Studio PC");
    mock.deviceSync.copyElsewhere = true;
    act(() => openSyncStep(false));
    shell();
    expect(await screen.findByRole("heading", { name: "In sync with Studio PC" }, { timeout: 3000 })).toBeInTheDocument();
    expect(step().getByText(/The rest comes over from Studio PC in the background, quickest when both are on the same Wi-Fi/)).toBeInTheDocument();
  });

  it("says what's wrong: the computer not answering, mobile data, sync off", async () => {
    mock.deviceSync.addDevice("Studio PC");
    mock.deviceSync.copyFails = true;
    act(() => openSyncStep(false));
    shell();
    expect(await screen.findByRole("heading", { name: "That didn't work" })).toBeInTheDocument();
    expect(step().getByRole("alert")).toHaveTextContent("Studio PC didn't answer. Open Platter there and try again.");
    // Fixed: Try again copies.
    mock.deviceSync.copyFails = false;
    mock.deviceSync.metered = true;
    fireEvent.click(step().getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(step().getByRole("alert")).toHaveTextContent("Copying waits for Wi-Fi. Connect to Wi-Fi, or turn off Send big things only on Wi-Fi."));
    mock.deviceSync.metered = false;
    await backend.settingsSet({ ...DEFAULT_SETTINGS, deviceSync: false });
    fireEvent.click(step().getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(step().getByRole("alert")).toHaveTextContent(/Turn on Keep my devices in sync first/));
    fireEvent.click(step().getByRole("button", { name: "Close" }));
    expect(screen.queryByTestId("phone-sync-step")).toBeNull();
  });

  it("on mobile data goes on when Send big things only on Wi-Fi is off", async () => {
    mock.deviceSync.addDevice("Studio PC");
    mock.deviceSync.metered = true;
    await backend.settingsSet({ ...DEFAULT_SETTINGS, deviceSyncWifiOnly: false });
    act(() => openSyncStep(false));
    shell();
    expect(await screen.findByRole("heading", { name: "In sync with Studio PC" }, { timeout: 3000 })).toBeInTheDocument();
  });

  it("is offered once after signing in, and Start fresh leaves it", async () => {
    act(() => openSyncStep(true));
    shell();
    expect(await screen.findByRole("heading", { name: "Bring your setup over" })).toBeInTheDocument();
    expect(step().getByText("Your sign-in and keys stay on each device.")).toBeInTheDocument();
    fireEvent.click(step().getByRole("button", { name: "Start fresh" }));
    expect(screen.queryByTestId("phone-sync-step")).toBeNull();
  });

  it("isn't offered after signing in when a computer is paired already", async () => {
    mock.deviceSync.addDevice("Studio PC");
    act(() => openSyncStep(true));
    shell();
    await screen.findByTestId("phone-shell");
    await waitFor(() => expect(screen.queryByTestId("phone-sync-step")).toBeNull());
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByTestId("phone-sync-step")).toBeNull();
  });

  it("words its progress line", () => {
    expect(progressLine({ name: "Studio PC", step: 4, of: 9, label: "Copying your pins" })).toBe("Copying your pins… 4 of 9");
  });
});

describe("the sync row on You", () => {
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
