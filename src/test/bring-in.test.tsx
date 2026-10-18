/**
 * "Bring in my listening" in the UI: the progress card on the Missing page
 * (wave, counts, why it's paused, Needs you), pausing, and its settings
 * inside Library import. The waves themselves run in the core.
 */
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type BringInStatus } from "../lib/types";
import { BringInCard, BringInSettings, parseWaves, stateLine, waveLine, whereLine } from "../pages/BringIn";
import { ImportPage } from "../pages/ImportPage";
import { resetImportListeners } from "../pages/importShared";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

async function settings(patch: Record<string, unknown> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, importSources: ["streaming"], lidarrUrl: "http://192.168.x.x:8686", ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(() => {
  mock.importMock.reset();
  resetImportListeners();
  mock.bringIn.userPaused = false;
  mock.bringIn.paused = null;
  mock.bringIn.bridge = null;
  mock.bringIn.needsYou = [{ key: "album:a\u001fb", artist: "Mira Vale", album: "Second Bloom", plays: 9, reason: "Only a bootleg on MusicBrainz.", at: 0 }];
  useToasts.setState({ toasts: [] });
});

const base: BringInStatus = {
  on: true,
  practice: false,
  ready: true,
  waves: [100, 50, 20],
  wave: 0,
  threshold: 100,
  done: 12,
  remaining: 339,
  today: 7,
  perDay: 30,
  paused: null,
  userPaused: false,
  finished: false,
  requestedTotal: 11,
  needsYou: [],
  practiced: [],
};

describe("wording", () => {
  it("says which wave and what it's doing", () => {
    expect(waveLine(base)).toBe("Wave 1 of 3 · albums with 100+ plays");
    expect(waveLine({ ...base, finished: true, threshold: null })).toBe("Every wave is done");
    expect(stateLine(base)).toBe("Running while Platter is open.");
    expect(stateLine({ ...base, paused: "Lidarr's queue has 25 items." })).toBe("Paused: Lidarr's queue has 25 items.");
    expect(stateLine({ ...base, userPaused: true })).toMatch(/^Paused\. Nothing/);
    expect(stateLine({ ...base, today: 30 })).toBe("Today's 30 are done. More tomorrow.");
    expect(stateLine({ ...base, practice: true })).toMatch(/^Practice mode/);
    expect(stateLine({ ...base, ready: false })).toMatch(/^Waiting for Lidarr/);
    expect(whereLine(base)).toBeNull();
    expect(stateLine({ ...base, bridge: "Shed" })).toBe("Asking Lidarr for a few albums a day.");
    expect(whereLine({ ...base, bridge: "Shed" })).toBe("Running on your Bridge (Shed), even while this computer is off.");
  });

  it("reads waves typed as a list", () => {
    expect(parseWaves("100, 50, 20")).toEqual([100, 50, 20]);
    expect(parseWaves("20 100 50")).toEqual([100, 50, 20]);
    expect(parseWaves("50, 50")).toBeNull();
    expect(parseWaves("0")).toBeNull();
    expect(parseWaves("1,2,3,4,5,6")).toBeNull();
    expect(parseWaves("lots")).toBeNull();
  });
});

describe("the progress card", () => {
  it("shows on Played a lot only when the setting is on", async () => {
    await settings({ bringIn: false });
    mock.importMock.run("streaming", "Downloads/my_extended_history.zip");
    const { unmount } = render(<ImportPage view="played" />);
    await screen.findByRole("tab", { name: "Played a lot", selected: true });
    expect(screen.queryByRole("region", { name: "Bring in my listening" })).toBeNull();
    unmount();
    await settings({ bringIn: true });
    render(<ImportPage view="played" />);
    const card = await screen.findByRole("region", { name: "Bring in my listening" });
    expect(card).toHaveTextContent("Wave 1 of 3 · albums with 100+ plays");
    expect(card).toHaveTextContent("12 done · 339 left in this wave · 7 of 30 today");
  });

  it("pauses and carries on", async () => {
    await settings({ bringIn: true });
    render(<BringInCard />);
    const card = await screen.findByRole("region", { name: "Bring in my listening" });
    fireEvent.click(within(card).getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(within(card).getByRole("status")).toHaveTextContent("Paused. Nothing is asked for"));
    expect(mock.bringIn.userPaused).toBe(true);
    fireEvent.click(within(card).getByRole("button", { name: "Carry on" }));
    await waitFor(() => expect(mock.bringIn.userPaused).toBe(false));
  });

  it("says when the Bridge runs it", async () => {
    await settings({ bringIn: true });
    mock.bringIn.bridge = "Shed Bridge";
    mock.bringIn.paused = "Lidarr's queue has 25 items. It carries on once it's down to 20.";
    render(<BringInCard />);
    const card = await screen.findByRole("region", { name: "Bring in my listening" });
    expect(card).toHaveTextContent("Running on your Bridge (Shed Bridge)");
    expect(within(card).getByRole("status")).toHaveTextContent("Paused: Lidarr's queue has 25 items.");
  });

  it("says why a guard paused it", async () => {
    await settings({ bringIn: true });
    mock.bringIn.paused = "Only 12 GB is free where Lidarr downloads. It carries on once there's 50 GB.";
    render(<BringInCard />);
    expect(await screen.findByText(/^Paused: Only 12 GB is free/)).toBeInTheDocument();
  });

  it("lists what needs you, to try again or leave", async () => {
    await settings({ bringIn: true });
    render(<BringInCard />);
    fireEvent.click(await screen.findByRole("button", { name: "Needs you (1)" }));
    const list = screen.getByRole("list", { name: "Albums that need you" });
    expect(list).toHaveTextContent("Second Bloom by Mira Vale · 9 plays");
    expect(list).toHaveTextContent("Only a bootleg on MusicBrainz.");
    fireEvent.click(within(list).getByRole("button", { name: "Leave Second Bloom" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: /Needs you/ })).toBeNull());
  });
});

describe("settings", () => {
  it("off by default, then waves, pace, space and queue", async () => {
    await settings();
    render(<BringInSettings />);
    const sw = await screen.findByRole("switch", { name: "Bring in my listening" });
    expect(sw).toHaveAttribute("aria-checked", "false");
    expect(screen.queryByRole("textbox", { name: "Waves, fewest plays" })).toBeNull();
    fireEvent.click(sw);
    await waitFor(() => expect(useSettings.getState().settings?.bringIn).toBe(true));
    const waves = await screen.findByRole("textbox", { name: "Waves, fewest plays" });
    expect(waves).toHaveValue("100, 50, 20");
    fireEvent.change(waves, { target: { value: "200 100" } });
    fireEvent.blur(waves);
    await waitFor(() => expect(useSettings.getState().settings?.bringInWaves).toEqual([200, 100]));
    fireEvent.change(waves, { target: { value: "nope" } });
    fireEvent.blur(waves);
    await waitFor(() => expect(useToasts.getState().toasts.some((t) => t.message.startsWith("Waves are 1 to 5"))).toBe(true));
    expect(waves).toHaveValue("200, 100");
    fireEvent.change(screen.getByRole("combobox", { name: "Albums a day" }), { target: { value: "10" } });
    fireEvent.change(screen.getByRole("combobox", { name: "Pause below free space" }), { target: { value: "100" } });
    fireEvent.change(screen.getByRole("combobox", { name: "Pause above Lidarr's queue" }), { target: { value: "5" } });
    await waitFor(() => {
      const s = useSettings.getState().settings!;
      expect([s.bringInPerDay, s.bringInMinFreeGb, s.bringInMaxQueue]).toEqual([10, 100, 5]);
    });
    expect((await mock.settingsGet()).bringInPerDay).toBe(10);
  });
});
