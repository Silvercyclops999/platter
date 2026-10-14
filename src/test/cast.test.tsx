/**
 * Chromecast and AirPlay under Listen on: the rows (kinds, groups, password
 * lock, devices Platter can't play on), moving to them, the AirPlay password
 * prompt, and the setting that hides them.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MOCK_AIRPLAY_PASSWORD, mockLanDevices, mockPlayers } from "../lib/mock/remote";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { AppShell } from "../shell/AppShell";
import { playerSub } from "../shell/ListenOn";
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
  mock.remote.resetLan();
  // The password prompt is covered here, so receivers with a password are listed.
  await mock.settingsSet({ showPlexPlayers: true, showCastDevices: true, moveStopsOldDevice: true, showAirplayLocked: true });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS, showAirplayLocked: true } });
  useRemote.setState({ players: null, busy: undefined, password: null });
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, outputOpen: true, outputReturn: null, menu: null });
  useToasts.setState({ toasts: [] });
});

const picker = () => screen.getByRole("complementary", { name: "Connect to a device" });
const findListenOn = () => within(picker()).findByRole("region", { name: "Listen on" });

describe("Cast and AirPlay rows", () => {
  it("leave out speakers that need a password unless the setting shows them", async () => {
    await mock.settingsSet({ showAirplayLocked: false });
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS } }));
    render(<AppShell />);
    const section = await findListenOn();
    await within(section).findByRole("button", { name: "Play on Den TV" });
    expect(within(section).queryByRole("button", { name: /Patio/ })).toBeNull();
    expect(DEFAULT_SETTINGS.showAirplayLocked).toBe(false);
  });

  it("list next to Plex players with their kind and product", async () => {
    render(<AppShell />);
    const section = await findListenOn();
    expect(await within(section).findByRole("button", { name: "Play on Den TV" })).toHaveTextContent("Chromecast");
    expect(within(section).getByRole("button", { name: "Play on Downstairs" })).toHaveTextContent("Speaker group");
    expect(within(section).getByRole("button", { name: "Play on Patio (asks for its password)" })).toHaveTextContent("AirPlay");
    expect(within(section).getByRole("button", { name: "Play on Studio Desk" })).toBeInTheDocument();
  });

  it("show why a device can't be played on, and say so when picked", async () => {
    render(<AppShell />);
    const section = await findListenOn();
    const old = await within(section).findByRole("button", { name: /Old AirPort, can't play here/ });
    expect(old).toHaveTextContent("Needs AirPlay encryption");
    fireEvent.click(old);
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message).join()).toContain("Can't play on Old AirPort"));
    expect(usePlayer.getState().remote).toBeNull();
    const [, , , unsupported] = mockLanDevices();
    expect(playerSub(unsupported)).toBe("AirPlay • Needs AirPlay encryption Platter doesn't support");
  });

  it("hide with their setting while Plex players stay", async () => {
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, showCastDevices: false } }));
    await mock.settingsSet({ showCastDevices: false });
    render(<AppShell />);
    const section = await findListenOn();
    await within(section).findByRole("button", { name: "Play on Studio Desk" });
    expect(within(section).queryByRole("button", { name: "Play on Den TV" })).toBeNull();
  });
});

describe("moving to a Chromecast", () => {
  it("shows the strip and the device as current", async () => {
    await act(() => mock.playResume());
    render(<AppShell />);
    fireEvent.click(await within(await findListenOn()).findByRole("button", { name: "Play on Den TV" }));
    await screen.findByRole("button", { name: /Playing on Den TV/ });
    expect(within(picker()).getByRole("group", { name: "Listening on" })).toHaveTextContent("Den TV");
  });
});

describe("app states", () => {
  async function onDenTv() {
    await act(() => mock.playResume());
    render(<AppShell />);
    fireEvent.click(await within(await findListenOn()).findByRole("button", { name: "Play on Den TV" }));
    await screen.findByRole("button", { name: /Playing on Den TV/ });
  }

  it("a Chromecast that goes away hands back with a toast", async () => {
    await onDenTv();
    act(() => mock.remote.loseRemote());
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Couldn't reach Den TV"));
    await waitFor(() => expect(screen.queryByRole("button", { name: /Playing on Den TV/ })).toBeNull());
  });

  it("a Chromecast taken over by someone else says so", async () => {
    await onDenTv();
    act(() => mock.remote.stopElsewhere());
    expect(await screen.findByText("Stopped on Den TV")).toBeInTheDocument();
  });
});

describe("the AirPlay password prompt", () => {
  it("asks once, shows a wrong password, and connects with the right one", async () => {
    await act(() => mock.playResume());
    render(<AppShell />);
    fireEvent.click(await within(await findListenOn()).findByRole("button", { name: /Play on Patio/ }));
    const dialog = await screen.findByRole("dialog", { name: "Password for Patio" });
    const input = within(dialog).getByLabelText("AirPlay password");
    expect(input).toHaveAttribute("type", "password");
    const connect = within(dialog).getByRole("button", { name: "Connect" });
    expect(connect).toBeDisabled();

    fireEvent.change(input, { target: { value: "nope" } });
    fireEvent.click(connect);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("didn't work");
    expect(usePlayer.getState().remote).toBeNull();

    fireEvent.change(input, { target: { value: MOCK_AIRPLAY_PASSWORD } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Connect" }));
    await screen.findByRole("button", { name: /Playing on Patio/ });
    expect(screen.queryByRole("dialog", { name: "Password for Patio" })).toBeNull();

    // Kept: next time no prompt.
    await act(() => mock.remoteReturn().then((r) => usePlayer.getState().setRemote(r)));
    await waitFor(() => expect(useRemote.getState().players?.find((p) => p.id === "airplay:patio")?.needsPassword).toBe(false));
  });

  it("can be cancelled", async () => {
    render(<AppShell />);
    fireEvent.click(await within(await findListenOn()).findByRole("button", { name: /Play on Patio/ }));
    const dialog = await screen.findByRole("dialog", { name: "Password for Patio" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(usePlayer.getState().remote).toBeNull();
  });
});

describe("settings", () => {
  it("has the Chromecast and AirPlay row, on by default", async () => {
    render(<SettingsPage />);
    expect(await screen.findByText("Show Chromecast and AirPlay devices")).toBeInTheDocument();
    expect(DEFAULT_SETTINGS.showCastDevices).toBe(true);
  });
});
