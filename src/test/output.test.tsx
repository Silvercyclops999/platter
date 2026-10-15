/**
 * Output picker: the speaker toggle, the right-slot panel, switching devices
 * through the core, the Settings row, and the fallback when the chosen device
 * is unplugged (app state: output device disappeared).
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { AppShell } from "../shell/AppShell";
import { OutputDeviceSelect } from "../shell/OutputSetting";
import { RELAY_MAX_KBPS, streamQuality } from "../shell/streamQuality";
import { connectApp } from "../stores/app";
import { outputChoices, outputLostMessage, useOutput } from "../stores/output";
import { connectPlayer, usePlayer } from "../stores/player";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const SPEAKERS = { id: "mock:speakers", name: "Speakers (Built-in Audio)", isDefault: true };
const HEADPHONES = { id: "mock:headphones", name: "Headphones (USB Audio DAC)", isDefault: false };

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
  mock.resetOutputs();
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, outputOpen: false, outputReturn: null, menu: null });
  useOutput.setState({ state: null, switching: undefined });
  useToasts.setState({ toasts: [] });
});

const picker = () => screen.queryByRole("complementary", { name: "Connect to a device" });
const toggle = () => screen.getByRole("button", { name: "Connect to a device" });

describe("opening and closing", () => {
  it("the speaker toggle opens the picker in the right slot and shows it's active", async () => {
    render(<AppShell />);
    expect(picker()).toBeNull();
    fireEvent.click(toggle());
    expect(picker()).toBeInTheDocument();
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
    expect(within(picker()!).getByRole("heading", { name: "Connect" })).toBeInTheDocument();
    expect(screen.getByRole("separator", { name: "Resize device picker" })).toBeInTheDocument();
    fireEvent.click(toggle());
    expect(picker()).toBeNull();
    expect(toggle()).toHaveAttribute("aria-pressed", "false");
  });

  it("closing returns the slot to what it showed before", async () => {
    render(<AppShell />);
    fireEvent.click(screen.getByRole("button", { name: "Queue" }));
    fireEvent.click(toggle());
    expect(screen.queryByRole("complementary", { name: "Queue" })).toBeNull();
    fireEvent.click(within(picker()!).getByRole("button", { name: "Close" }));
    expect(screen.getByRole("complementary", { name: "Queue" })).toBeInTheDocument();

    act(() => useUi.getState().setNowPlayingOpen(true));
    fireEvent.click(toggle());
    expect(useUi.getState().nowPlayingOpen).toBe(false);
    fireEvent.click(toggle());
    expect(useUi.getState().nowPlayingOpen).toBe(true);
    expect(picker()).toBeNull();
  });

  it("opening the queue or now playing takes the slot back from the picker", () => {
    act(() => useUi.getState().setOutputOpen(true));
    act(() => useUi.getState().setQueueOpen(true));
    expect(useUi.getState()).toMatchObject({ outputOpen: false, queueOpen: true });
    act(() => useUi.getState().setOutputOpen(true));
    act(() => useUi.getState().setNowPlayingOpen(true));
    expect(useUi.getState()).toMatchObject({ outputOpen: false, queueOpen: false, nowPlayingOpen: true });
  });
});

describe("devices from the core", () => {
  it("shows the current device card and the other outputs", async () => {
    render(<AppShell />);
    fireEvent.click(toggle());
    fireEvent.click(await within(picker()!).findByRole("button", { name: /^Sound output/ }));
    const p = picker()!;
    expect((await within(p).findAllByTitle(SPEAKERS.name))[0]).toBeInTheDocument();
    expect(within(p).getByText("Current device, system default")).toBeInTheDocument();
    expect(within(p).getAllByText(/^This computer/).length).toBeGreaterThan(0);
    // System default first and marked, then every device, the current one included.
    const rows = within(p.querySelector("#output-others-list") as HTMLElement).getAllByRole("button");
    expect(rows.map((r) => r.getAttribute("aria-label"))).toEqual([
      `Play on System default (${SPEAKERS.name})`,
      `Play on ${SPEAKERS.name}`,
      `Play on ${HEADPHONES.name}`,
      "Play on Living Room Receiver (HDMI)",
    ]);
    expect(rows[0]).toHaveAttribute("aria-current", "true");
    expect(rows.slice(1).every((r) => !r.hasAttribute("aria-current"))).toBe(true);
    // No network tips in this phase.
    expect(within(p).queryByText(/wi-?fi|network/i)).toBeNull();
  });

  it("switching keeps the position and play state", async () => {
    const user = userEvent.setup();
    const album = mock.fx.albums.find((a) => a.trackCount >= 2)!;
    await act(() => mock.playContext({ context: { kind: "album", id: album.id } }));
    await act(() => mock.playSeek(42_000));
    await waitFor(() => expect(usePlayer.getState().state?.status).toBe("playing"));
    const before = (await mock.playerState()).positionMs;

    render(<AppShell />);
    fireEvent.click(toggle());
    fireEvent.click(await within(picker()!).findByRole("button", { name: /^Sound output/ }));
    await user.click(await within(picker()!).findByRole("button", { name: `Play on ${HEADPHONES.name}` }));
    await waitFor(() => expect(useOutput.getState().state?.selected).toBe(HEADPHONES.id));
    const p = picker()!;
    expect(within(p).getAllByTitle(HEADPHONES.name)[0]).toBeInTheDocument();
    expect(within(p).getByRole("button", { name: `Play on ${HEADPHONES.name}` })).toHaveAttribute("aria-current", "true");
    expect(within(p).getByRole("button", { name: `Play on System default (${SPEAKERS.name})` })).not.toHaveAttribute("aria-current");
    const after = await mock.playerState();
    expect(after.status).toBe("playing");
    expect(after.positionMs).toBeGreaterThanOrEqual(before);
    expect(after.positionMs).toBeLessThan(before + 2_000);
    expect((await mock.settingsGet()).outputDevice).toBe(HEADPHONES.id);

    // And back to following the system default.
    await user.click(within(p).getByRole("button", { name: `Play on System default (${SPEAKERS.name})` }));
    await waitFor(() => expect(useOutput.getState().state?.selected).toBeNull());
    await act(() => mock.playPause());
  });

  it("the chosen device going away pauses, falls back to the default and says so", async () => {
    await act(() => mock.playContext({ context: { kind: "album", id: mock.fx.albums[0].id } }));
    await act(async () => {
      await mock.outputSelect(HEADPHONES.id);
    });
    await waitFor(() => expect(usePlayer.getState().state?.status).toBe("playing"));
    render(<AppShell />);
    fireEvent.click(toggle());
    fireEvent.click(await within(picker()!).findByRole("button", { name: /^Sound output/ }));
    expect((await within(picker()!).findAllByTitle(HEADPHONES.name))[0]).toBeInTheDocument();

    act(() => mock.unplugOutput(HEADPHONES.id));
    await waitFor(() =>
      expect(useToasts.getState().toasts.map((t) => t.message)).toContain(outputLostMessage(HEADPHONES.name)),
    );
    expect(outputLostMessage(HEADPHONES.name)).toBe("Headphones (USB Audio DAC) disconnected. Switched to the system default.");
    await waitFor(() => expect(usePlayer.getState().state?.status).toBe("paused"));
    const p = picker()!;
    await waitFor(() => expect(within(p).getAllByTitle(SPEAKERS.name)[0]).toBeInTheDocument());
    expect(within(p).queryByText(HEADPHONES.name)).toBeNull();
    expect(useOutput.getState().state?.selected).toBeNull();
  });

  it("an unplugged device that wasn't chosen just leaves the list", async () => {
    render(<AppShell />);
    fireEvent.click(toggle());
    fireEvent.click(await within(picker()!).findByRole("button", { name: /^Sound output/ }));
    await within(picker()!).findByRole("button", { name: `Play on ${HEADPHONES.name}` });
    act(() => mock.unplugOutput(HEADPHONES.id));
    await waitFor(() => expect(within(picker()!).queryByRole("button", { name: `Play on ${HEADPHONES.name}` })).toBeNull());
    expect(useToasts.getState().toasts).toHaveLength(0);
  });
});

describe("Mac-style device list", () => {
  const MAC = [
    { id: "BuiltInSpeakerDevice", name: "MacBook Pro Speakers", isDefault: true },
    { id: "MSLoopbackDriverDevice_UID", name: "Microsoft Teams Audio", isDefault: false },
    { id: "BlackHole2ch_UID", name: "BlackHole 2ch", isDefault: false },
  ];

  it("the panel lists the current speakers and virtual devices, not just the others", async () => {
    mock.setOutputs(MAC);
    render(<AppShell />);
    fireEvent.click(toggle());
    fireEvent.click(await within(picker()!).findByRole("button", { name: /^Sound output: MacBook Pro Speakers/ }));
    const rows = within(picker()!.querySelector("#output-others-list") as HTMLElement).getAllByRole("button");
    expect(rows.map((r) => r.getAttribute("aria-label"))).toEqual([
      "Play on System default (MacBook Pro Speakers)",
      "Play on MacBook Pro Speakers",
      "Play on Microsoft Teams Audio",
      "Play on BlackHole 2ch",
    ]);
    expect(rows[0]).toHaveAttribute("aria-current", "true");
  });

  it("Settings shows the same list", async () => {
    mock.setOutputs(MAC);
    render(
      <>
        <span id="t">Output device</span>
        <span id="d">Where music plays</span>
        <OutputDeviceSelect ids={{ title: "t", desc: "d" }} />
      </>,
    );
    const select = await screen.findByRole("combobox", { name: "Output device" });
    expect(within(select).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "System default (MacBook Pro Speakers)",
      "MacBook Pro Speakers",
      "Microsoft Teams Audio",
      "BlackHole 2ch",
    ]);
  });
});

describe("outputChoices", () => {
  const spk = { id: "spk", name: "Speakers", isDefault: true };
  const teams = { id: "teams", name: "Microsoft Teams Audio", isDefault: false };

  it("keeps the device in use even when the list left it out", () => {
    const c = outputChoices({ devices: [teams], selected: null, current: spk });
    expect(c.map((x) => x.label)).toEqual(["System default (Speakers)", "Speakers", "Microsoft Teams Audio"]);
    expect(c.filter((x) => x.selected).map((x) => x.id)).toEqual([null]);
  });

  it("marks a chosen device, and a chosen one that's unplugged", () => {
    expect(outputChoices({ devices: [spk, teams], selected: "teams", current: teams }).filter((x) => x.selected).map((x) => x.id)).toEqual(["teams"]);
    const gone = outputChoices({ devices: [spk], selected: "usb", current: spk });
    expect(gone.map((x) => x.label)).toEqual(["System default (Speakers)", "Speakers", "Disconnected device"]);
    expect(gone.filter((x) => x.selected).map((x) => x.id)).toEqual(["usb"]);
  });

  it("lists a device once", () => {
    expect(outputChoices({ devices: [spk, spk, teams], selected: null, current: spk })).toHaveLength(3);
  });
});

describe("Settings: Playback → Output device", () => {
  const ids = { title: "t", desc: "d" };

  it("lists System default and every device, and picks one", async () => {
    const user = userEvent.setup();
    render(
      <>
        <span id="t">Output device</span>
        <span id="d">Where music plays</span>
        <OutputDeviceSelect ids={ids} />
      </>,
    );
    const select = await screen.findByRole("combobox", { name: "Output device" });
    expect(select).toHaveValue("__system-default__");
    const labels = within(select).getAllByRole("option").map((o) => o.textContent);
    expect(labels).toEqual([`System default (${SPEAKERS.name})`, SPEAKERS.name, HEADPHONES.name, "Living Room Receiver (HDMI)"]);
    await user.selectOptions(select, HEADPHONES.id);
    await waitFor(() => expect(useOutput.getState().state?.selected).toBe(HEADPHONES.id));
    expect(select).toHaveValue(HEADPHONES.id);
    await user.selectOptions(select, "__system-default__");
    await waitFor(() => expect(useOutput.getState().state?.selected).toBeNull());
  });
});

describe("stream quality on the device card", () => {
  const flac = { codec: "flac", bitDepth: 24, sampleRate: 96000, bitrate: 2800 };
  const mp3 = { codec: "mp3", bitDepth: null, sampleRate: 44100, bitrate: 192 };
  const s = { qualityHome: "original", qualityAway: "320" } as const;

  it("shows the file at home on Original, the transcode when capped", () => {
    expect(streamQuality(flac, "local", s)).toBe("FLAC 24/96");
    expect(streamQuality(flac, "remote", s)).toBe("MP3 320");
    expect(streamQuality(flac, "relay", s)).toBe(`MP3 ${RELAY_MAX_KBPS}`);
    expect(streamQuality(mp3, "remote", s)).toBe("MP3 192");
    expect(streamQuality(flac, "local", { qualityHome: "256", qualityAway: "320" })).toBe("MP3 256");
    expect(streamQuality(flac, "relay", { qualityHome: "original", qualityAway: "original" })).toBe(`MP3 ${RELAY_MAX_KBPS}`);
  });
});
