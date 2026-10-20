import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { compareSemver } from "../lib/version";
import { BRIDGE_UPDATE_DOCS, BridgeSettings, bridgeMatches, bridgeVersionLine, envList } from "../pages/BridgeSettings";
import { LidarrWorkStatus } from "../pages/LidarrModes";

const mock = backend as MockBackend;

beforeEach(() => {
  mock.bridgeState = { bridges: [], pairing: null, queued: [] };
  mock.bridgePairAfterMs = 20;
});

describe("Settings > Bridge", () => {
  it("is found by the words people use for it", () => {
    for (const q of ["bridge", "unraid", "docker", "server lidarr", ""]) expect(bridgeMatches(q)).toBe(true);
    expect(bridgeMatches("equalizer")).toBe(false);
  });

  it("says it's optional, shows a pair code, then the paired Bridge and what it's doing", async () => {
    // Long enough that a busy machine still sees the code before the Bridge pairs.
    mock.bridgePairAfterMs = 1500;
    const user = userEvent.setup();
    render(<BridgeSettings />);
    expect(screen.getByTestId("bridge-settings")).toHaveTextContent("Platter works fully without it");
    await user.click(await screen.findByRole("button", { name: "Pair a Bridge" }));
    const code = await screen.findByTestId("bridge-code");
    expect(code).toHaveTextContent(/PB1-\S+/);
    expect(code).toHaveTextContent("BRIDGE_PAIR_CODE");
    const row = await screen.findByTestId("bridge-row", {}, { timeout: 5000 });
    expect(row).toHaveTextContent("Platter Bridge");
    expect(row).toHaveTextContent("Answering");
    expect(row).toHaveTextContent("read-write");
    expect(row).toHaveTextContent("Last catch-up");
    expect(row).toHaveTextContent("Second Wind");
    expect(screen.queryByTestId("bridge-code")).toBeNull();
    // Unpair asks once more.
    await user.click(within(row).getByRole("button", { name: "Unpair" }));
    await user.click(within(row).getByRole("button", { name: "Unpair Platter Bridge" }));
    expect(await screen.findByRole("button", { name: "Pair a Bridge" })).toBeInTheDocument();
    expect(screen.queryByTestId("bridge-row")).toBeNull();
  });

  it("adds a Bridge from the code it shows, then says which settings are its own and what it still needs", async () => {
    const user = userEvent.setup();
    render(<BridgeSettings />);
    const add = await screen.findByTestId("bridge-add");
    expect(add).toHaveTextContent("starting with PBB-");
    const field = within(add).getByRole("textbox", { name: "Add the Bridge" });
    // A code of the other kind is turned away with words.
    await user.type(field, "PB1-q3Vx8bN2kLm9TzR4wYc7HdE1fGj5sAo6pUi0eWn3vXb8yZaQk");
    await user.click(within(add).getByRole("button", { name: "Add the Bridge" }));
    expect(await within(add).findByRole("alert")).toHaveTextContent("starts with PBB-");
    await user.clear(field);
    await user.type(field, "PBB-q3Vx8bN2kLm9TzR4wYc7HdE1fGj5sAo6pUi0eWn3vXb8yZaQk{Enter}");
    const row = await screen.findByTestId("bridge-row");
    expect(row).toHaveTextContent("Shed Bridge");
    expect(within(row).getByTestId("bridge-env")).toHaveTextContent("Set on the Bridge itself: Lidarr's address and Lidarr's key. It uses those, not what this computer sends.");
    expect(within(row).getByTestId("bridge-plex-link")).toHaveTextContent("open plex.tv/link and enter K3QZ");
    expect(within(row).getByTestId("bridge-music-path")).toHaveTextContent("MUSIC_PLEX_PATH to /data/media/Music");
    expect(field).toHaveValue("");
  });

  it("names the Bridge's own settings plainly", () => {
    expect(envList(["lidarrUrl"])).toBe("Lidarr's address");
    expect(envList(["lidarrUrl", "lidarrKey", "plexToken"])).toBe("Lidarr's address, Lidarr's key and the Plex token");
    expect(envList([])).toBe("");
  });

  it("shows Lidarr settings changes a Bridge queued, with exactly what it would send", async () => {
    const user = userEvent.setup();
    mock.bridgeState.queued = [
      { id: "q1", what: "Change Lidarr's file naming settings", method: "PUT", path: "/config/naming/1", body: '{"renameTracks":false}', at: 1, device: "Platter Bridge" },
    ];
    render(<BridgeSettings />);
    const q = await screen.findByTestId("bridge-queued");
    expect(q).toHaveTextContent("Waiting for you: Change Lidarr's file naming settings");
    expect(q).toHaveTextContent("never does that on its own");
    expect(q).toHaveTextContent("PUT /config/naming/1");
    await user.click(within(q).getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByTestId("bridge-queued")).toBeNull();
  });

  describe("when the Bridge and this app aren't the same version", () => {
    const paired = (version: string, statusVersion?: string) => {
      mock.bridgeState.bridges = [
        {
          id: "b1",
          name: "Shed Bridge",
          version,
          pairedAt: 1,
          lastSeen: 1,
          online: true,
          status:
            statusVersion === undefined
              ? null
              : {
                  name: "Shed Bridge",
                  version: statusVersion,
                  at: 1,
                  doing: "Waiting for Lidarr",
                  lastCatchUp: null,
                  music: "none",
                  plex: true,
                  lidarr: true,
                  problem: null,
                  fromEnv: [],
                  plexLink: null,
                  musicPlexPath: null,
                },
          music: "none",
        },
      ];
    };
    afterEach(() => {
      mock.appVersion = "0.1.0";
    });

    it("says a Bridge behind this app needs updating, with a link to how", async () => {
      mock.appVersion = "0.2.31";
      // What the Bridge said last wins over the version it paired with.
      paired("0.2.20", "0.2.29");
      mock.helpLinksOpened.length = 0;
      const user = userEvent.setup();
      render(<BridgeSettings />);
      const line = await screen.findByTestId("bridge-version");
      expect(line).toHaveTextContent(
        "Platter Bridge 0.2.29 is behind this app (0.2.31). Update it on your server: on Unraid the Docker tab shows the update, or the Auto Update Applications plugin does it for you.",
      );
      await user.click(within(line).getByRole("button", { name: "How to update" }));
      await waitFor(() => expect(mock.helpLinksOpened).toEqual([BRIDGE_UPDATE_DOCS]));
      expect(BRIDGE_UPDATE_DOCS).toBe("https://github.com/Silvercyclops999/platter/blob/main/docs/BRIDGE.md#updates");
    });

    it("says to update Platter when the Bridge is newer", async () => {
      mock.appVersion = "0.2.29";
      paired("0.2.31");
      render(<BridgeSettings />);
      const line = await screen.findByTestId("bridge-version");
      expect(line).toHaveTextContent("Platter Bridge is newer than this app; update Platter.");
      expect(within(line).queryByRole("button")).toBeNull();
    });

    it("says nothing when they match, or when the Bridge's version isn't known", async () => {
      mock.appVersion = "0.2.31";
      paired("0.2.31", "0.2.31");
      const { unmount } = render(<BridgeSettings />);
      await screen.findByTestId("bridge-row");
      expect(screen.queryByTestId("bridge-version")).toBeNull();
      unmount();
      paired("", "");
      render(<BridgeSettings />);
      await screen.findByTestId("bridge-row");
      expect(screen.queryByTestId("bridge-version")).toBeNull();
    });

    it("ignores pre-release tags", async () => {
      mock.appVersion = "0.2.31-beta.2";
      paired("0.2.31");
      render(<BridgeSettings />);
      await screen.findByTestId("bridge-row");
      expect(screen.queryByTestId("bridge-version")).toBeNull();
    });

    it("compares versions as numbers, not text", () => {
      expect(compareSemver("0.2.29", "0.2.31")).toBe(-1);
      expect(compareSemver("0.2.10", "0.2.9")).toBe(1);
      expect(compareSemver("1.0.0", "0.99.99")).toBe(1);
      expect(compareSemver("0.2.31", "0.2.31")).toBe(0);
      expect(compareSemver("v0.2.31", "0.2.31")).toBe(0);
      expect(compareSemver("0.2.31-rc.1", "0.2.31")).toBe(0);
      expect(compareSemver("0.2.30-beta", "0.2.31+build.5")).toBe(-1);
      expect(compareSemver("0.3", "0.3.0")).toBe(0);
      expect(compareSemver("", "0.2.31")).toBeNull();
      expect(compareSemver("unknown", "0.2.31")).toBeNull();
      expect(compareSemver("0.2.31", null)).toBeNull();
      expect(bridgeVersionLine("0.2.31", null)).toBeNull();
      expect(bridgeVersionLine("0.2.29", "0.2.31")?.behind).toBe(true);
    });
  });

  it("the Lidarr work line names the Bridge when one is paired", async () => {
    mock.bridgeState.bridges = [
      { id: "b1", name: "Shed Bridge", version: "0.2.20", pairedAt: 1, lastSeen: 1, online: true, status: null, music: "none" },
    ];
    render(<LidarrWorkStatus />);
    expect(await screen.findByTestId("lidarr-bridge-line")).toHaveTextContent("Shed Bridge does this work while no computer has Platter open.");
  });
});
