import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockSignalPath } from "../lib/mock/signal";
import { DEFAULT_SETTINGS, type Exclusive, type SignalPath, type Track } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { PlayerBar } from "../shell/PlayerBar";
import { SignalPanel, badgeText, stages, verdictText } from "../shell/SignalPath";
import { connectPlayer, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useSignal } from "../stores/signalPath";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
let disconnect: () => void;
let unlisten: (() => void) | undefined;

beforeAll(async () => {
  disconnect = connectPlayer(mock);
  await waitFor(() => expect(usePlayer.getState().state?.track).toBeTruthy());
  // Like the app: the core's signal path events keep the store current.
  unlisten = await mock.listen("audio://signal-path", (p) => useSignal.getState().set(p));
});
afterAll(() => {
  unlisten?.();
  disconnect();
});
beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS, headphoneCorrection: [], bitPerfectOutputs: [], leveling: "off", eqEnabled: false, preampDb: 0 });
  await mock.setVolume(1);
  await mock.setMuted(false);
  await loadSettings();
  useSignal.setState({ path: null, open: false });
});
afterEach(() => vi.restoreAllMocks());

const speakers = { id: "mock:speakers", name: "Speakers", isDefault: true };

function flac(bits: number, rate: number, codec = "flac", bitrate: number | null = 2900): Track {
  return { ...usePlayer.getState().state!.track!, id: "sig-1", codec, bitDepth: bits, sampleRate: rate, bitrate };
}

function path(track: Track | null, opts: { exclusive?: Exclusive; transcode?: number | null; eq?: boolean; leveling?: boolean; volume?: number } = {}): SignalPath {
  return mockSignalPath({
    track,
    settings: { ...DEFAULT_SETTINGS, headphoneCorrection: [], leveling: opts.leveling ? "track" : "off", eqEnabled: !!opts.eq, preampDb: 0 },
    device: speakers,
    exclusive: opts.exclusive ?? { state: "off" },
    volume: opts.volume ?? 1,
    muted: false,
    transcodeKbps: opts.transcode ?? null,
  });
}

function showPanel(p: SignalPath) {
  act(() => useSignal.setState({ path: p, open: true }));
  return render(<SignalPanel />);
}

describe("the panel", () => {
  it("draws bit-perfect as plates with the verdict in words and no fix to suggest", () => {
    showPanel(path(flac(24, 96000), { exclusive: { state: "on", rate: 96000, bits: 24 } }));
    const dialog = screen.getByRole("dialog", { name: "Signal path" });
    expect(within(dialog).getByText("Bit-perfect")).toHaveClass("signal-verdict--bitPerfect");
    const chain = within(dialog).getByRole("list", { name: "From the file to your speakers" });
    const names = [...chain.querySelectorAll(".signal-plate__name")].map((n) => n.textContent);
    expect(names).toEqual(["Source", "Decode", "Loudness", "EQ", "Resample", "Blend", "Output"]);
    expect(chain).toHaveTextContent("FLAC · 24-bit · 96 kHz · stereo");
    expect(chain).toHaveTextContent("Exclusive, 24-bit at 96 kHz, volume bypassed");
    expect(within(dialog).queryByText("Why isn't this bit-perfect?")).toBeNull();
  });

  it("names a resample and points at bit-perfect output", () => {
    showPanel(path(flac(24, 96000)));
    const dialog = screen.getByRole("dialog", { name: "Signal path" });
    expect(dialog).toHaveTextContent("Lossless, resampled to 48 kHz");
    expect(dialog).toHaveTextContent("96 kHz → 48 kHz");
    expect(within(dialog).getByText("Why isn't this bit-perfect?")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Open the setting" })).toBeInTheDocument();
  });

  it("says when volume and EQ are applied, with the leveling gain", () => {
    showPanel(path(flac(16, 48000), { eq: true, leveling: true }));
    const dialog = screen.getByRole("dialog", { name: "Signal path" });
    expect(dialog).toHaveTextContent("Lossless, volume and EQ applied");
    expect(dialog).toHaveTextContent("Track gain");
    expect(dialog).toHaveTextContent("Equalizer");
  });

  it("calls a lossy file lossy, with nothing to change", () => {
    showPanel(path(flac(0, 44100, "mp3", 320)));
    const dialog = screen.getByRole("dialog", { name: "Signal path" });
    expect(within(dialog).getByText("Lossy source (MP3 320)")).toHaveClass("signal-verdict--lossy");
    expect(dialog).toHaveTextContent("The file itself is lossy");
    expect(within(dialog).queryByRole("button", { name: "Open the setting" })).toBeNull();
  });

  it("fills in a transcode's bitrate from the streaming quality", () => {
    const p = path(flac(16, 44100), { transcode: 320 });
    expect(p.verdict.text).toBe("Lossy stream (MP3)");
    expect(verdictText(p, "MP3 320")).toBe("Lossy stream (MP3 320)");
    expect(verdictText(p, "FLAC")).toBe("Lossy stream (MP3)");
    expect(stages(p, "MP3 320")[0]).toMatchObject({ name: "Source", value: "MP3 320 · 44.1 kHz · stereo", detail: "Converted by your server" });
  });

  it("says nothing is playing when idle", () => {
    showPanel(path(null));
    expect(screen.getByRole("dialog", { name: "Signal path" })).toHaveTextContent("Nothing playing.");
    expect(badgeText(path(null), null)).toBeNull();
  });

  it("opens the setting that would change it", async () => {
    const user = userEvent.setup();
    const go = vi.spyOn(nav, "go").mockImplementation(() => {});
    showPanel(path(flac(24, 96000)));
    await user.click(screen.getByRole("button", { name: "Open the setting" }));
    expect(go).toHaveBeenCalledWith({ name: "settings", row: "bitPerfectOutputs" });
    expect(useSignal.getState().open).toBe(false);
  });
});

describe("the badge", () => {
  it("opens the panel, takes focus into it, and Escape closes it back to the badge", async () => {
    const user = userEvent.setup();
    render(<PlayerBar />);
    const badge = await screen.findByRole("button", { name: /^Signal path: / });
    expect(badge).toHaveAttribute("aria-expanded", "false");
    await user.click(badge);
    const dialog = await screen.findByRole("dialog", { name: "Signal path" });
    await waitFor(() => expect(dialog).toHaveFocus());
    expect(badge).toHaveAttribute("aria-expanded", "true");
    await waitFor(() => expect(dialog.querySelectorAll(".signal-plate").length).toBeGreaterThan(5));
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Signal path" })).toBeNull();
    expect(badge).toHaveFocus();
  });

  it("works from the keyboard and closes when focus leaves", async () => {
    const user = userEvent.setup();
    render(
      <>
        <PlayerBar />
        <button type="button">Elsewhere</button>
      </>,
    );
    const badge = await screen.findByRole("button", { name: /^Signal path: / });
    badge.focus();
    await user.keyboard("{Enter}");
    const dialog = await screen.findByRole("dialog", { name: "Signal path" });
    await waitFor(() => expect(dialog).toHaveFocus());
    act(() => screen.getByRole("button", { name: "Elsewhere" }).focus());
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Signal path" })).toBeNull());
  });

  it("stays live while open: a sound setting changes the verdict", async () => {
    const user = userEvent.setup();
    render(<PlayerBar />);
    await user.click(await screen.findByRole("button", { name: /^Signal path: / }));
    const dialog = await screen.findByRole("dialog", { name: "Signal path" });
    await waitFor(() => expect(dialog).not.toHaveTextContent("EQ applied"));
    await act(() => backend.settingsSet({ eqEnabled: true }));
    await waitFor(() => expect(dialog).toHaveTextContent("EQ applied"));
  });

  it("opens from More player options too", async () => {
    const user = userEvent.setup();
    render(
      <>
        <PlayerBar />
        <ContextMenuHost />
      </>,
    );
    await user.click(screen.getByRole("button", { name: "More player options" }));
    const menu = await screen.findByRole("menu", { name: "More player options" });
    await user.click(within(menu).getByRole("menuitem", { name: "Signal path" }));
    expect(await screen.findByRole("dialog", { name: "Signal path" })).toBeInTheDocument();
    act(() => useUi.setState({ menu: null }));
  });
});

describe("settings", () => {
  it("Show the signal path badge is on in Sound and turning it off hides the badge", async () => {
    const user = userEvent.setup();
    expect(DEFAULT_SETTINGS.signalPathBadge).toBe(true);
    render(<SettingsPage />);
    const row = document.querySelector<HTMLElement>('[data-setting="signalPathBadge"]')!;
    expect(row).toHaveTextContent("Show the signal path badge");
    expect(row.closest("section")?.querySelector("h2")?.textContent).toMatch(/Sound/);
    const sw = within(row).getByRole("switch");
    expect(sw).toBeChecked();
    await user.click(sw);
    await waitFor(() => expect(useSettings.getState().settings?.signalPathBadge).toBe(false));
    render(<PlayerBar />);
    expect(screen.queryByRole("button", { name: /^Signal path: / })).toBeNull();
  });
});
