import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { BridgeSettings, bridgeMatches } from "../pages/BridgeSettings";
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
    const user = userEvent.setup();
    render(<BridgeSettings />);
    expect(screen.getByTestId("bridge-settings")).toHaveTextContent("Platter works fully without it");
    await user.click(await screen.findByRole("button", { name: "Pair a Bridge" }));
    const code = await screen.findByTestId("bridge-code");
    expect(code).toHaveTextContent(/PB1-\S+/);
    expect(code).toHaveTextContent("BRIDGE_PAIR_CODE");
    const row = await screen.findByTestId("bridge-row");
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

  it("the Lidarr work line names the Bridge when one is paired", async () => {
    mock.bridgeState.bridges = [
      { id: "b1", name: "Shed Bridge", version: "0.2.20", pairedAt: 1, lastSeen: 1, online: true, status: null, music: "none" },
    ];
    render(<LidarrWorkStatus />);
    expect(await screen.findByTestId("lidarr-bridge-line")).toHaveTextContent("Shed Bridge does this work while no computer has Platter open.");
  });
});
