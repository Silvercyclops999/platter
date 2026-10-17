/**
 * Sync to a device in the UI: the Devices page offers a plugged-in drive,
 * adds it as a device, shows what goes on it, its format, size limit and
 * folder, runs a sync with progress, pause, cancel and "Safe to unplug",
 * and says plainly when the device is full, write-protected, pulled out or
 * not plugged in. Send to device adds and syncs. Settings lists devices
 * (rename, forget), the default format and the remove switch.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MOCK_DRIVE } from "../lib/mock/devices";
import { DEFAULT_SETTINGS, type DeviceSyncStatus } from "../lib/types";
import { DevicesPage, spaceLine } from "../pages/DevicesPage";
import { SettingsPage } from "../pages/SettingsPage";
import { useApp } from "../stores/app";
import { loadDevices, progressLine, resetDevices, sendToDevice, startDevices, useDevices } from "../stores/devices";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const album = mock.fx.albums[2];
const albumSongs = mock.fx.tracksByAlbum.get(album.id)!.length;
const toasts = () => useToasts.getState().toasts.map((t) => t.message);

async function addDevice(name = "Car stereo") {
  const d = mock.devices.add(MOCK_DRIVE.root, name, "Music");
  mock.devices.select(d.id, "album", album.id, album.title, true);
  await loadDevices();
  return d;
}

const card = (name: string) => screen.getByRole("region", { name });

beforeEach(async () => {
  mock.devices.reset();
  mock.devices.stepMs = 20;
  resetDevices();
  useRouter.getState().reset();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
});

afterEach(() => {
  resetDevices();
});

describe("helpers", () => {
  it("say how full a device is and how a sync stands", () => {
    expect(spaceLine(null, null)).toBeNull();
    expect(spaceLine(2 * 1024 ** 3, 8 * 1024 ** 3)).toMatch(/free of/);
    const s = { phase: "copying", done: 3, total: 12 } as DeviceSyncStatus;
    expect(progressLine(s)).toBe("Copying 3 of 12 songs");
    expect(progressLine({ ...s, phase: "done", done: 1, total: 1 })).toBe("Done: 1 song copied. Safe to unplug.");
    expect(progressLine({ ...s, phase: "done", done: 0, total: 0 })).toBe("Already up to date. Safe to unplug.");
    expect(progressLine({ ...s, phase: "waiting" })).toMatch(/Waiting/);
  });
});

describe("the Devices page", () => {
  it("offers a plugged-in drive and adds it as a device", async () => {
    const user = userEvent.setup();
    render(<DevicesPage />);
    const drives = await screen.findByRole("region", { name: "Plugged in" });
    expect(within(drives).getByText("WALKMAN (E:\\)")).toBeInTheDocument();
    await user.click(within(drives).getByRole("button", { name: "Add as a device" }));
    const dialog = screen.getByRole("dialog", { name: "Add a device" });
    const name = within(dialog).getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Walkman");
    expect(within(dialog).getByLabelText("Music folder")).toHaveValue("Music");
    await user.click(within(dialog).getByRole("button", { name: "Add device" }));
    expect(await screen.findByRole("heading", { level: 2, name: "Walkman" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Plugged in" })).not.toBeInTheDocument();
    expect(within(card("Walkman")).getByText(/Plugged in \(E:\\\)/)).toBeInTheDocument();
    expect(within(card("Walkman")).getByText(/Nothing yet/)).toBeInTheDocument();
  });

  it("says to plug something in when there's nothing to show", async () => {
    mock.devices.plugged = false;
    render(<DevicesPage />);
    expect(await screen.findByText(/Plug in a USB drive, an SD card or a music player/)).toBeInTheDocument();
  });

  it("lists what goes on the device and takes things off", async () => {
    const user = userEvent.setup();
    await addDevice();
    render(<DevicesPage />);
    const c = await screen.findByRole("region", { name: "Car stereo" });
    expect(within(c).getByText(album.title)).toBeInTheDocument();
    await user.click(within(c).getByRole("button", { name: "Add Liked Songs" }));
    await waitFor(() => expect(within(c).getByText("Liked Songs")).toBeInTheDocument());
    expect(within(c).queryByRole("button", { name: "Add Liked Songs" })).not.toBeInTheDocument();
    await user.click(within(c).getByRole("button", { name: `Take ${album.title} off Car stereo` }));
    await waitFor(() => expect(within(c).queryByText(album.title)).not.toBeInTheDocument());
    expect(mock.devices.devices[0].selection.map((s) => s.kind)).toEqual(["liked"]);
  });

  it("sets the format, size limit and keeps the default in view", async () => {
    const user = userEvent.setup();
    await mock.settingsSet({ deviceFormat: "opus" });
    await loadSettings();
    await addDevice();
    render(<DevicesPage />);
    const c = await screen.findByRole("region", { name: "Car stereo" });
    const format = within(c).getByRole("combobox", { name: "Format" });
    expect(within(format).getByRole("option", { name: "Default (Opus)" })).toBeInTheDocument();
    await user.selectOptions(format, "mp3-v0");
    await waitFor(() => expect(mock.devices.devices[0].format).toBe("mp3-v0"));
    await user.selectOptions(within(c).getByRole("combobox", { name: "Size limit" }), "16");
    await waitFor(() => expect(mock.devices.devices[0].budgetGb).toBe(16));
    await user.selectOptions(within(c).getByRole("combobox", { name: "Format" }), "default");
    await waitFor(() => expect(mock.devices.devices[0].format).toBeNull());
  });

  it("syncs with progress and ends safe to unplug", async () => {
    const user = userEvent.setup();
    await addDevice();
    render(<DevicesPage />);
    const c = await screen.findByRole("region", { name: "Car stereo" });
    await user.click(within(c).getByRole("button", { name: "Sync now" }));
    expect(await within(c).findByText(`Done: ${albumSongs} songs copied. Safe to unplug.`)).toBeInTheDocument();
    expect(toasts()).toContain("Car stereo is synced. Safe to unplug.");
    expect(mock.devices.devices[0].lastSync).not.toBeNull();
  });

  it("pauses, resumes and cancels a sync", async () => {
    const user = userEvent.setup();
    mock.devices.stepMs = 200;
    await addDevice();
    render(<DevicesPage />);
    const c = await screen.findByRole("region", { name: "Car stereo" });
    await user.click(within(c).getByRole("button", { name: "Sync now" }));
    await user.click(await within(c).findByRole("button", { name: "Pause" }));
    expect(await within(c).findByText(/^Paused,/)).toBeInTheDocument();
    await user.click(within(c).getByRole("button", { name: "Resume" }));
    await user.click(await within(c).findByRole("button", { name: "Cancel" }));
    expect(await within(c).findByText("Cancelled. Safe to unplug.")).toBeInTheDocument();
    expect(within(c).getByRole("button", { name: "Sync now" })).toBeEnabled();
  });

  it.each([
    ["full", /The device is full/],
    ["writeProtected", /write-protected/],
    ["accessDenied", /isn't allowed to write/],
    ["removed", /removed before the sync finished/],
  ] as const)("says so when the device is %s", async (fail, text) => {
    const user = userEvent.setup();
    mock.devices.fail = fail;
    await addDevice();
    render(<DevicesPage />);
    const c = await screen.findByRole("region", { name: "Car stereo" });
    await user.click(within(c).getByRole("button", { name: "Sync now" }));
    const alert = await within(c).findByRole("alert");
    expect(alert).toHaveTextContent(text);
    expect(toasts().some((t) => text.test(t))).toBe(true);
  });

  it("shows what the sync wants known, such as a damaged list kept aside", async () => {
    const user = userEvent.setup();
    mock.devices.notice = "Platter's list of the songs it put on this device was damaged. A copy was kept as .platter-sync.json.bak.";
    await addDevice();
    render(<DevicesPage />);
    const c = await screen.findByRole("region", { name: "Car stereo" });
    await user.click(within(c).getByRole("button", { name: "Sync now" }));
    expect(await within(c).findByText(/kept as \.platter-sync\.json\.bak/)).toBeInTheDocument();
    expect(await within(c).findByText(/Safe to unplug/)).toBeInTheDocument();
  });

  it("can't sync a device that isn't plugged in", async () => {
    await addDevice();
    mock.devices.plugged = false;
    await loadDevices();
    render(<DevicesPage />);
    const c = await screen.findByRole("region", { name: "Car stereo" });
    expect(within(c).getByText(/^Not plugged in/)).toBeInTheDocument();
    expect(within(c).getByRole("button", { name: "Sync now" })).toBeDisabled();
  });

  it("renames and forgets a device from its menu", async () => {
    const user = userEvent.setup();
    await addDevice();
    render(
      <>
        <DevicesPage />
        <ContextMenuHost />
      </>,
    );
    const c = await screen.findByRole("region", { name: "Car stereo" });
    await user.click(within(c).getByRole("button", { name: "More options for Car stereo" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename" }));
    const rename = screen.getByRole("dialog", { name: "Rename device" });
    await user.clear(within(rename).getByLabelText("Name"));
    await user.type(within(rename).getByLabelText("Name"), "Van");
    await user.click(within(rename).getByRole("button", { name: "Save" }));
    const van = await screen.findByRole("region", { name: "Van" });
    await user.click(within(van).getByRole("button", { name: "More options for Van" }));
    await user.click(await screen.findByRole("menuitem", { name: "Forget this device" }));
    await user.click(within(screen.getByRole("dialog", { name: "Forget device" })).getByRole("button", { name: "Forget" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Van" })).not.toBeInTheDocument());
    expect(mock.devices.devices).toEqual([]);
  });
});

describe("Send to device", () => {
  it("adds the item and syncs it when the device is plugged in", async () => {
    const d = mock.devices.add(MOCK_DRIVE.root, "Car stereo", "Music");
    await startDevices();
    await sendToDevice(d.id, { kind: "playlist", id: mock.fx.playlists[0].id, title: mock.fx.playlists[0].title });
    expect(mock.devices.devices[0].selection.map((s) => s.kind)).toEqual(["playlist"]);
    expect(toasts()).toContain(`Sending ${mock.fx.playlists[0].title} to Car stereo`);
    await waitFor(() => expect(useDevices.getState().view?.sync?.phase).toBe("done"));
  });

  it("just adds the item when the device isn't plugged in", async () => {
    const d = mock.devices.add(MOCK_DRIVE.root, "Car stereo", "Music");
    mock.devices.plugged = false;
    await startDevices();
    await sendToDevice(d.id, { kind: "liked", id: "", title: "Liked Songs" });
    expect(mock.devices.devices[0].selection).toEqual([{ kind: "liked", id: "", title: "Liked Songs" }]);
    expect(toasts()).toContain("Added Liked Songs to Car stereo. Plug it in and sync to copy it.");
    expect(mock.devices.status).toBeNull();
  });
});

describe("the Devices settings", () => {
  it("has a default format and the remove switch, both saved", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const format = await screen.findByRole("combobox", { name: "Default format" });
    expect(format).toHaveValue("original");
    await user.selectOptions(format, "flac");
    await waitFor(() => expect(useSettings.getState().settings?.deviceFormat).toBe("flac"));
    const remove = screen.getByRole("switch", { name: "Delete songs removed from the selection" });
    expect(remove).not.toBeChecked();
    await user.click(remove);
    await waitFor(() => expect(useSettings.getState().settings?.deviceDeleteRemoved).toBe(true));
  });

  it("lists devices with rename and forget, and opens the Devices page", async () => {
    const user = userEvent.setup();
    await addDevice("Walkman");
    render(<SettingsPage />);
    const list = await screen.findByRole("list", { name: "Devices" });
    expect(within(list).getByText("Walkman")).toBeInTheDocument();
    expect(within(list).getByText("Plugged in")).toBeInTheDocument();
    await user.click(within(list).getByRole("button", { name: "Forget Walkman" }));
    await user.click(within(screen.getByRole("dialog", { name: "Forget device" })).getByRole("button", { name: "Forget" }));
    expect(await screen.findByText(/No devices yet/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open Devices" }));
    const e = useRouter.getState().entries;
    expect(e[e.length - 1].route).toEqual({ name: "devices" });
  });
});
