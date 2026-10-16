/**
 * Your other Platter computers under Listen on: listed first, moving to one
 * and back, the one-time firewall note when one can't be reached, and the
 * settings that announce this computer and name it.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockPlatterDevices, mockPlayers } from "../lib/mock/remote";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { AppShell } from "../shell/AppShell";
import { FIREWALL_NOTE_KEY, playerSub } from "../shell/ListenOn";
import { connectApp } from "../stores/app";
import { connectPlayer, usePlayer } from "../stores/player";
import { useRemote } from "../stores/remote";
import { useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;

let disconnect: () => void;
beforeAll(async () => {
  const a = connectApp(mock);
  const p = connectPlayer(mock);
  disconnect = () => {
    a();
    p();
  };
  await waitFor(() => expect(usePlayer.getState().state).toBeTruthy());
});
afterAll(() => disconnect());

beforeEach(async () => {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
  localStorage.clear();
  if (usePlayer.getState().remote) await act(() => mock.remoteReturn().then((r) => usePlayer.getState().setRemote(r)));
  act(() => usePlayer.getState().setRemote({ session: null, stoppedOn: null }));
  mock.remote.setPlayers(mockPlayers(mock.fx.tracks[7]));
  mock.remote.setPlatterDevices(mockPlatterDevices());
  mock.remote.resetLan();
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useRemote.setState({ players: null, busy: undefined, password: null });
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, outputOpen: true, outputReturn: null, menu: null });
  useToasts.setState({ toasts: [] });
});

const picker = () => screen.getByRole("complementary", { name: "Connect to a device" });
const findListenOn = () => within(picker()).findByRole("region", { name: "Listen on" });

describe("Platter computers under Listen on", () => {
  it("come first, as computers", async () => {
    render(<AppShell />);
    const section = await findListenOn();
    const mac = await within(section).findByRole("button", { name: "Play on Studio MacBook" });
    expect(mac).toHaveTextContent("Platter");
    const rows = within(section).getAllByRole("button");
    expect(rows[0]).toBe(mac);
    expect(screen.queryByRole("note", { name: "Can't reach a computer" })).toBeNull();
  });

  it("still show with Plex players and Cast devices turned off", async () => {
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, showPlexPlayers: false, showCastDevices: false, showDlnaDevices: false } }));
    await mock.settingsSet({ showPlexPlayers: false, showCastDevices: false, showDlnaDevices: false });
    render(<AppShell />);
    const section = await findListenOn();
    expect(await within(section).findByRole("button", { name: "Play on Studio MacBook" })).toBeInTheDocument();
    expect(within(section).queryByRole("button", { name: "Play on Studio Desk" })).toBeNull();
  });

  it("move playback there and back", async () => {
    await act(() => mock.playResume());
    render(<AppShell />);
    fireEvent.click(await within(picker()).findByRole("button", { name: "Play on Studio MacBook" }));
    expect(await screen.findByRole("button", { name: /Playing on Studio MacBook/ })).toBeInTheDocument();
    expect(usePlayer.getState().local?.status).toBe("paused");
    fireEvent.click(within(await findListenOn()).getByRole("button", { name: "Play on this computer" }));
    await waitFor(() => expect(usePlayer.getState().remote).toBeNull());
  });
});

describe("a Platter computer that can't be reached", () => {
  beforeEach(() => mock.remote.setPlatterDevices(mockPlatterDevices(true)));

  it("says so on its row and explains the firewall once", async () => {
    const { unmount } = render(<AppShell />);
    const section = await findListenOn();
    const row = await within(section).findByRole("button", { name: "Play on Studio MacBook (not reachable on this network)" });
    expect(row).toHaveTextContent("Can't reach it on this network");
    const note = within(section).getByRole("note", { name: "Can't reach a computer" });
    expect(note).toHaveTextContent("allow Platter through the firewall on Studio MacBook");
    fireEvent.click(within(note).getByRole("button", { name: "Got it" }));
    expect(within(section).queryByRole("note")).toBeNull();
    expect(localStorage.getItem(FIREWALL_NOTE_KEY)).toBe("1");
    unmount();
    render(<AppShell />);
    await within(await findListenOn()).findByRole("button", { name: /Studio MacBook/ });
    expect(screen.queryByRole("note", { name: "Can't reach a computer" })).toBeNull();
  });

  it("keeps playing here with a toast when picked", async () => {
    await act(() => mock.playResume());
    render(<AppShell />);
    fireEvent.click(await within(picker()).findByRole("button", { name: /Studio MacBook/ }));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Couldn't reach Studio MacBook"));
    expect(usePlayer.getState().remote).toBeNull();
    expect(usePlayer.getState().local?.status).toBe("playing");
  });

  it("grey line", () => {
    expect(playerSub(mockPlatterDevices(true)[0])).toBe("Platter • Can't reach it on this network");
    expect(playerSub(mockPlatterDevices()[0])).toBe("Platter");
  });
});

describe("settings", () => {
  it("has the device rows under Playback with their defaults", async () => {
    render(<SettingsPage />);
    for (const title of ["Show this computer to my other devices", "This computer's name", "Show AirPlay devices that need a password"]) {
      expect(await screen.findByText(title)).toBeInTheDocument();
    }
    // It opens a network port, so it starts off.
    expect(DEFAULT_SETTINGS.showToMyDevices).toBe(false);
    expect(DEFAULT_SETTINGS.computerName).toBe("");
    expect(DEFAULT_SETTINGS.showAirplayLocked).toBe(false);
  });

  it("saves the computer's name on Enter, trimmed", async () => {
    render(<SettingsPage />);
    const field = await screen.findByRole("textbox", { name: "This computer's name" });
    fireEvent.change(field, { target: { value: "  Den PC  " } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(useSettings.getState().settings?.computerName).toBe("Den PC"));
    expect((await mock.settingsGet()).computerName).toBe("Den PC");
  });

  it("turning showing this computer on and off saves it", async () => {
    render(<SettingsPage />);
    const toggle = await screen.findByRole("switch", { name: "Show this computer to my other devices" });
    const before = !!useSettings.getState().settings?.showToMyDevices;
    fireEvent.click(toggle);
    await waitFor(() => expect(useSettings.getState().settings?.showToMyDevices).toBe(!before));
  });
});
