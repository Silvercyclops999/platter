/**
 * Settings > Lidarr away from home: no list of home sides, just a line when
 * Lidarr can't be reached away from home (the failure state), and the
 * setting that lets your other devices use Lidarr through this computer,
 * under More.
 */
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { BridgeInfo } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { LidarrAwayProblem } from "../pages/LidarrHomeSide";
import { SettingsPage } from "../pages/SettingsPage";
import { inMore, onPhone } from "../pages/settingsMeta";
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

describe("Lidarr away from home", () => {
  const away = () => document.querySelector('[data-setting="lidarrAway"]');

  it("shows nothing while a computer at home or the Bridge answers", async () => {
    mock.homeComputers = [{ name: "Studio PC", on: true }];
    mock.bridgeState = { ...mock.bridgeState, bridges: [bridge(true)] };
    render(<SettingsPage page="lidarr" />);
    await screen.findAllByRole("switch");
    expect(away()).toBeNull();
    expect(screen.queryByText("Home side")).toBeNull();
    expect(screen.queryByTestId("lidarr-away-problem")).toBeNull();
  });

  it("says plainly when nothing answers away from home", async () => {
    mock.homeComputers = [{ name: "Studio PC", on: false }];
    render(<LidarrAwayProblem sides={await mock.lidarrHomeSides()} />);
    expect(screen.getByRole("alert")).toHaveTextContent("Lidarr isn't reachable away from home. Turn on a computer at home or add the Bridge.");
  });

  it("with a Bridge that isn't answering, says so on the page instead of asking to add one", async () => {
    mock.bridgeState = { ...mock.bridgeState, bridges: [bridge(false)] };
    render(<SettingsPage page="lidarr" />);
    const alert = await screen.findByTestId("lidarr-away-problem");
    expect(alert).toHaveTextContent("Your Bridge isn't answering");
    expect(alert).toHaveTextContent("Platter is linking to it again by itself.");
    expect(alert).not.toHaveTextContent("add the Bridge");
    expect(away()).toHaveTextContent("Away from home");
  });

  it("on the computer with Lidarr set up, nothing is wrong", async () => {
    await mock.settingsSet({ lidarrUrl: "http://10.20.30.40:8686" });
    await mock.lidarrKeySet("abc123");
    render(<LidarrAwayProblem sides={await mock.lidarrHomeSides()} />);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("has its setting on by default under More, this device only, and it switches off", async () => {
    await mock.settingsSet({ lidarrUrl: "http://10.20.30.40:8686" });
    await mock.lidarrKeySet("abc123");
    await loadSettings();
    expect(inMore({ key: "homeSideLidarr", page: "lidarr" })).toBe(true);
    const user = userEvent.setup();
    render(<SettingsPage page="lidarr" />);
    const more = await screen.findByRole("button", { name: /^More in Lidarr/ });
    if (more.getAttribute("aria-expanded") !== "true") await user.click(more);
    const sw = await screen.findByRole("switch", { name: "Let my other devices use Lidarr through this computer" });
    expect(sw).toBeChecked();
    expect(document.querySelector('[data-setting="homeSideLidarr"]')).toHaveTextContent("This device only");
    await user.click(sw);
    await waitFor(async () => expect((await mock.settingsGet()).homeSideLidarr).toBe(false));
  });

  it("on a phone leaves out the computer's setting", () => {
    expect(onPhone({ key: "homeSideLidarr", page: "lidarr" })).toBe(false);
    expect(onPhone({ key: "lidarrAway", page: "lidarr" })).toBe(true);
  });
});
