/**
 * Settings > Sync > Keep my devices in sync: the switch and its note, the
 * paired devices, adding a device (the codes to show), pairing with a code
 * (6 digits at home, the long code away) and every way it fails, unpairing,
 * Sync now, held changes and what to sync. The phone's Copy from my
 * computer is in phone-sync.test.tsx.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { pairCodeReady } from "../pages/KeepInSyncSettings";
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
  it("is on by default and says when nothing is paired yet", async () => {
    render(<SettingsPage />);
    const sw = await screen.findByRole("switch", { name: "Keep my devices in sync" });
    expect(sw).toHaveAttribute("aria-checked", "true");
    await waitFor(() => expect(row("Keep my devices in sync").getByText("No other device paired yet. Add one below.")).toBeInTheDocument());
    expect(screen.queryByRole("button", { name: "Sync now" })).toBeNull();
  });

  it("adding a device shows the 6-digit code, the long code and its QR code, then the device", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(row("Add a device").getByRole("button", { name: "Show codes" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a device" });
    const code = await within(dialog).findByLabelText(/^Code \d \d \d \d \d \d$/);
    expect(code.textContent).toMatch(/^\d{3} \d{3}$/);
    expect(within(dialog).getByText("On the same network")).toBeInTheDocument();
    expect(within(dialog).getByText("Somewhere else")).toBeInTheDocument();
    expect(within(dialog).getByRole("img", { name: "QR code for the long code" })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Copy the long code" })).toBeInTheDocument();
    expect(within(dialog).getByText(/Works for (10|9):\d\d/)).toBeInTheDocument();
    act(() => mock.deviceSync.pairArrives("Studio PC"));
    await within(dialog).findByText(/Paired with Studio PC. The two keep in sync from now on/);
    await user.click(within(dialog).getByRole("button", { name: "Done" }));
    await waitFor(() => expect(row("Studio PC").getByText(/^In sync, last just now$/)).toBeInTheDocument());
    expect(row("Keep my devices in sync").getByText("In sync with Studio PC.")).toBeInTheDocument();
  });

  it("a device that used the long code is paired only after a yes here", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(row("Add a device").getByRole("button", { name: "Show codes" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a device" });
    await within(dialog).findByLabelText(/^Code /);
    act(() => mock.deviceSync.pairAsks("Kitchen iPad"));
    await within(dialog).findByText(/Pair with Kitchen iPad\? It used the long code/);
    expect(mock.deviceSync.devices).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Pair" }));
    await within(dialog).findByText(/Paired with Kitchen iPad/);
    expect(mock.deviceSync.devices.map((d) => d.name)).toEqual(["Kitchen iPad"]);
  });

  it("saying no to a device that used the long code pairs nothing", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(row("Add a device").getByRole("button", { name: "Show codes" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a device" });
    await within(dialog).findByLabelText(/^Code /);
    act(() => mock.deviceSync.pairAsks("Someone's Laptop"));
    await user.click(await within(dialog).findByRole("button", { name: "Don't pair" }));
    await within(dialog).findByText(/Not paired. Get new codes to try again/);
    expect(mock.deviceSync.devices).toHaveLength(0);
    expect(within(dialog).getByRole("button", { name: "New codes" })).toBeInTheDocument();
  });

  it("with the relay down the long code is hidden and the 6-digit code still shows", async () => {
    const user = userEvent.setup();
    mock.deviceSync.relayDown = true;
    render(<SettingsPage />);
    await user.click(row("Add a device").getByRole("button", { name: "Show codes" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a device" });
    await within(dialog).findByLabelText(/^Code \d/);
    expect(within(dialog).getByText("The relay can't be reached. The 6-digit code still works on this network.")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "Copy the long code" })).toBeNull();
    expect(within(dialog).queryByRole("img", { name: "QR code for the long code" })).toBeNull();
  });

  it("closing the dialog stops the codes, even while they're being made", async () => {
    const user = userEvent.setup();
    const cancel = vi.spyOn(backend, "devicePairCancel");
    const { unmount } = render(<SettingsPage />);
    await user.click(row("Add a device").getByRole("button", { name: "Show codes" }));
    await screen.findByRole("dialog", { name: "Add a device" });
    unmount();
    expect(cancel).toHaveBeenCalled();
  });

  it("says when the sync status can't be read, and Retry reads it again", async () => {
    const user = userEvent.setup();
    mock.deviceSync.statusFails = true;
    render(<SettingsPage />);
    await waitFor(() => expect(row("Keep my devices in sync").getByText("Couldn't read sync status.")).toBeInTheDocument());
    mock.deviceSync.statusFails = false;
    await user.click(row("Couldn't read sync status").getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(row("Keep my devices in sync").getByText("No other device paired yet. Add one below.")).toBeInTheDocument());
    expect(screen.queryByText("Couldn't read sync status", { selector: ".setting-row__title" })).toBeNull();
  });

  it("codes that ran out offer new ones; cancelling stops showing them", async () => {
    const user = userEvent.setup();
    const cancel = vi.spyOn(backend, "devicePairCancel");
    render(<SettingsPage />);
    await user.click(row("Add a device").getByRole("button", { name: "Show codes" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a device" });
    await within(dialog).findByLabelText(/^Code /);
    act(() => mock.deviceSync.expirePair());
    await within(dialog).findByText(/Those codes ran out/);
    await user.click(within(dialog).getByRole("button", { name: "New codes" }));
    await within(dialog).findByLabelText(/^Code /);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(cancel).toHaveBeenCalled();
  });

  it("pairs with the right 6-digit code and explains a wrong one", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(row("Pair with a code").getByRole("button", { name: "Enter a code" }));
    const dialog = await screen.findByRole("dialog", { name: "Pair with a code" });
    const field = within(dialog).getByRole("textbox", { name: "Code" });
    const pair = within(dialog).getByRole("button", { name: "Pair" });
    expect(pair).toBeDisabled();
    await user.type(field, "111 111");
    await user.click(pair);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("That code doesn't match");
    await user.clear(field);
    await user.type(field, "135790");
    await user.click(pair);
    await within(dialog).findByText("Paired with Studio PC. The two keep in sync from now on.");
    await user.click(within(dialog).getByRole("button", { name: "Done" }));
    await waitFor(() => expect(row("Studio PC").getByRole("button", { name: "Unpair" })).toBeInTheDocument());
  });

  it("failure states: away from home, another account, the relay down, a code that isn't one", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(row("Pair with a code").getByRole("button", { name: "Enter a code" }));
    const dialog = await screen.findByRole("dialog", { name: "Pair with a code" });
    const field = within(dialog).getByRole("textbox", { name: "Code" });
    const pair = within(dialog).getByRole("button", { name: "Pair" });
    const tryCode = async (code: string, said: RegExp) => {
      await user.clear(field);
      await user.type(field, code);
      await user.click(pair);
      await waitFor(() => expect(within(dialog).getByRole("alert")).toHaveTextContent(said));
    };
    mock.deviceSync.away = true;
    await tryCode("135790", /No device on this network is showing a code. Away from home, use the long code/);
    mock.deviceSync.away = false;
    mock.deviceSync.otherAccount = true;
    await tryCode("135790", /signed in to a different Plex account/);
    await tryCode(mock.deviceSync.otherLong, /didn't answer/);
    mock.deviceSync.otherAccount = false;
    mock.deviceSync.relayDown = true;
    await tryCode(mock.deviceSync.otherLong, /The relay can't be reached/);
    // Not a code at all: Pair stays off.
    await user.clear(field);
    await user.type(field, "hello");
    expect(pair).toBeDisabled();
    expect(screen.queryByText("Studio PC", { selector: ".setting-row__title" })).toBeNull();
  });

  it("the long code from away pairs, pasted or as the QR code's link", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(row("Pair with a code").getByRole("button", { name: "Enter a code" }));
    const dialog = await screen.findByRole("dialog", { name: "Pair with a code" });
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Code" }), { target: { value: `platter://pair-device/${mock.deviceSync.otherLong}` } });
    await user.click(within(dialog).getByRole("button", { name: "Pair" }));
    await within(dialog).findByText(/Paired with Studio PC/);
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

  it("reads a code as ready only when it's 6 digits or a long code", () => {
    expect(pairCodeReady("135 790")).toBe(true);
    expect(pairCodeReady("13579")).toBe(false);
    expect(pairCodeReady(mock.deviceSync.otherLong)).toBe(true);
    expect(pairCodeReady(`platter://pair-device/${mock.deviceSync.otherLong}`)).toBe(true);
    expect(pairCodeReady("PB1-short")).toBe(false);
    expect(pairCodeReady("platter://copy-keys/135790")).toBe(false);
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

  it("show on the phone's Downloads page", async () => {
    usePhoneForm.setState({ phone: true });
    try {
      mock.deviceSync.suggest("playlist", "p1", "Road Trip");
      act(() => useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } }));
      useRouter.getState().reset({ name: "downloads" });
      render(<PhoneShell />);
      const list = await screen.findByRole("list", { name: "On your other devices" });
      expect(within(list).getByText("Downloaded on Studio PC")).toBeInTheDocument();
      expect(within(list).getByRole("button", { name: "Download here" })).toBeInTheDocument();
    } finally {
      usePhoneForm.setState({ phone: false });
    }
  });
});
