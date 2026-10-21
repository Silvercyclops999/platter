/**
 * Settings > Lidarr > Home side: the setting that lets your other devices
 * use Lidarr through this computer, the list of home sides (a computer at
 * home, the Bridge) with the one that answers, and the failure state when
 * none is on.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { BridgeInfo } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { LidarrHomeSides } from "../pages/LidarrHomeSide";
import { SettingsPage } from "../pages/SettingsPage";
import { onPhone } from "../pages/settingsMeta";
import { useApp } from "../stores/app";
import { loadSettings, useSettings } from "../stores/settings";

const mock = backend as MockBackend;

const bridge = (online: boolean): BridgeInfo => ({
  id: "b1",
  name: "Shed Bridge",
  version: "0.4.0",
  pairedAt: 1,
  lastSeen: null,
  online,
  status: null,
  music: "none",
});

beforeEach(async () => {
  mock.lidarr.reset();
  mock.homeComputers = [];
  mock.bridgeState = { bridges: [], pairing: null, queued: [] };
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
});

describe("Lidarr home side", () => {
  it("lists a computer at home first, then the Bridge, and says which answers", async () => {
    mock.homeComputers = [{ name: "Studio PC", on: true }];
    mock.bridgeState = { ...mock.bridgeState, bridges: [bridge(true)] };
    render(<LidarrHomeSides />);
    const box = await screen.findByTestId("lidarr-home-sides");
    const items = within(box).getAllByRole("listitem");
    expect(items.map((i) => i.textContent)).toEqual([
      "Studio PCYour computer at home, on now. It answers for this device.",
      "Shed BridgePlatter Bridge, on all the time.",
    ]);
    expect(within(box).queryByRole("alert")).toBeNull();
  });

  it("the Bridge answers while the computer is off", async () => {
    mock.homeComputers = [{ name: "Studio PC", on: false }];
    mock.bridgeState = { ...mock.bridgeState, bridges: [bridge(true)] };
    render(<LidarrHomeSides />);
    const items = await screen.findAllByRole("listitem");
    expect(items[0]).toHaveTextContent("Your computer at home, off or away now.");
    expect(items[0]).not.toHaveTextContent("It answers");
    expect(items[1]).toHaveTextContent("Platter Bridge, on all the time. It answers for this device.");
  });

  it("says plainly when nothing answers away from home", async () => {
    mock.homeComputers = [{ name: "Studio PC", on: false }];
    render(<LidarrHomeSides />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Lidarr isn't reachable away from home. Turn on a computer at home or add the Bridge.");
  });

  it("with a Bridge that isn't answering, says so instead of asking to add one", async () => {
    mock.bridgeState = { ...mock.bridgeState, bridges: [bridge(false)] };
    render(<LidarrHomeSides />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Your Bridge isn't answering");
    expect(alert).toHaveTextContent("Platter is linking to it again by itself.");
    expect(alert).not.toHaveTextContent("add the Bridge");
  });

  it("on the computer with Lidarr set up, says it answers for your other devices", async () => {
    await mock.settingsSet({ lidarrUrl: "http://10.20.30.40:8686" });
    await mock.lidarrKeySet("abc123");
    render(<LidarrHomeSides />);
    expect(await screen.findByText("Your other devices use Lidarr through this computer while Platter is open here.")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("has its setting on by default, this device only, and it switches off", async () => {
    await mock.settingsSet({ lidarrUrl: "http://10.20.30.40:8686" });
    await mock.lidarrKeySet("abc123");
    await loadSettings();
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sw = await screen.findByRole("switch", { name: "Let my other devices use Lidarr through this computer" });
    expect(sw).toBeChecked();
    expect(document.querySelector('[data-setting="homeSideLidarr"]')).toHaveTextContent("This device only");
    expect(document.querySelector('[data-setting="lidarrHomeSides"]')).not.toBeNull();
    await user.click(sw);
    await waitFor(async () => expect((await mock.settingsGet()).homeSideLidarr).toBe(false));
    expect(await within(document.querySelector<HTMLElement>('[data-setting="lidarrHomeSides"]')!).findByText(/None yet/)).toBeTruthy();
  });

  it("on a phone shows the home sides but not the computer's setting", () => {
    expect(onPhone({ key: "homeSideLidarr", page: "lidarr" })).toBe(false);
    expect(onPhone({ key: "lidarrHomeSides", page: "lidarr" })).toBe(true);
  });
});
