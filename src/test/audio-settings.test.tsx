/**
 * Settings for Transitions, Loudness and the Equalizer: defaults, the
 * crossfade length, the EQ curve by keyboard and pointer, presets, saved
 * profiles, and a profile that follows an output device.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, EQ_PRESETS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { connectApp } from "../stores/app";
import { useOutput } from "../stores/output";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

let disconnect: () => void;
beforeAll(() => {
  disconnect = connectApp(mock);
});
afterAll(() => disconnect());

beforeEach(async () => {
  mock.resetOutputs();
  mock.resetAudio();
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  useOutput.setState({ state: null, switching: undefined });
  useToasts.setState({ toasts: [] });
  await loadSettings();
});

const core = () => backend.settingsGet();
const band = (name: string) => screen.getByRole("slider", { name });

describe("transitions and loudness", () => {
  it("shows the defaults", async () => {
    render(<SettingsPage />);
    expect(screen.getByRole("combobox", { name: "Crossfade" })).toHaveValue("smart");
    expect(screen.getByRole("switch", { name: "Never crossfade within an album" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("combobox", { name: "Loudness leveling" })).toHaveValue("smart");
    expect(screen.getByRole("slider", { name: "Preamp" })).toHaveAttribute("aria-valuenow", "0");
    expect(screen.getByRole("switch", { name: "Limiter" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("switch", { name: "Equalizer" })).toHaveAttribute("aria-checked", "false");
    // Fixed length only shows for a fixed crossfade.
    expect(screen.queryByRole("slider", { name: "Crossfade length" })).toBeNull();
  });

  it("a fixed crossfade gets a length from 1 to 12 s", async () => {
    render(<SettingsPage />);
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Crossfade" }), "fixed");
    const len = await screen.findByRole("slider", { name: "Crossfade length" });
    expect(len).toHaveAttribute("aria-valuenow", "6");
    act(() => len.focus());
    await userEvent.keyboard("{ArrowRight}{ArrowRight}");
    await waitFor(async () => expect((await core()).crossfadeSeconds).toBe(8));
    expect(screen.getByText("8 s")).toBeInTheDocument();
    expect((await core()).crossfade).toBe("fixed");
  });

  it("preamp moves in half steps and leveling and limiter save", async () => {
    render(<SettingsPage />);
    const pre = screen.getByRole("slider", { name: "Preamp" });
    act(() => pre.focus());
    await userEvent.keyboard("{ArrowRight}{ArrowRight}{ArrowRight}");
    await waitFor(async () => expect((await core()).preampDb).toBe(1.5));
    expect(screen.getByText("+1.5 dB")).toBeInTheDocument();
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Loudness leveling" }), "album");
    await userEvent.click(screen.getByRole("switch", { name: "Limiter" }));
    await waitFor(async () => {
      const s = await core();
      expect([s.leveling, s.limiter]).toEqual(["album", false]);
    });
  });
});

describe("equalizer", () => {
  it("is greyed out until it's turned on", async () => {
    render(<SettingsPage />);
    expect(band("125 Hz")).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("combobox", { name: "Preset" })).toBeDisabled();
    expect(screen.getByText("Turn on the equalizer to change the curve.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("switch", { name: "Equalizer" }));
    expect(band("125 Hz")).not.toHaveAttribute("aria-disabled");
    expect(screen.getAllByRole("slider").filter((s) => /Hz$/.test(s.getAttribute("aria-label") ?? ""))).toHaveLength(10);
  });

  it("arrows move the focused band half a dB, Shift 2 dB, and the curve turns custom", async () => {
    await backend.settingsSet({ eqEnabled: true });
    await loadSettings();
    render(<SettingsPage />);
    act(() => band("125 Hz").focus());
    await userEvent.keyboard("{ArrowUp}");
    await userEvent.keyboard("{Shift>}{ArrowUp}{/Shift}");
    expect(band("125 Hz")).toHaveAttribute("aria-valuenow", "2.5");
    expect(band("125 Hz")).toHaveAttribute("aria-valuetext", "+2.5 dB");
    // Left/right move between bands.
    await userEvent.keyboard("{ArrowRight}{ArrowDown}");
    expect(band("250 Hz")).toHaveFocus();
    await waitFor(async () => expect((await core()).eqBands.slice(2, 4)).toEqual([2.5, -0.5]));
    expect((await core()).eqPreset).toBe("custom");
    expect(screen.getByRole("combobox", { name: "Preset" })).toHaveValue("custom");
    // Values stop at +12.
    act(() => band("31 Hz").focus());
    for (let i = 0; i < 8; i++) await userEvent.keyboard("{Shift>}{ArrowUp}{/Shift}");
    expect(band("31 Hz")).toHaveAttribute("aria-valuenow", "12");
  });

  it("dragging a point sets its band", async () => {
    await backend.settingsSet({ eqEnabled: true });
    await loadSettings();
    render(<SettingsPage />);
    const svg = screen.getByRole("group", { name: "Equalizer bands" });
    // jsdom has no layout: give the graph its drawn size.
    svg.getBoundingClientRect = () => ({ left: 0, top: 0, width: 640, height: 220, right: 640, bottom: 220, x: 0, y: 0, toJSON: () => ({}) });
    svg.setPointerCapture = () => {};
    // The 1 kHz column is the sixth of ten; +6 dB is a quarter of the way down the plot.
    const x1k = 44 + (5 * (640 - 44 - 20)) / 9;
    const yPlus6 = 18 + (6 * (220 - 18 - 30)) / 24;
    fireEvent.pointerDown(svg, { button: 0, clientX: x1k + 4, clientY: yPlus6, pointerId: 1 });
    fireEvent.pointerUp(svg, { pointerId: 1 });
    await waitFor(async () => expect((await core()).eqBands[5]).toBe(6));
    await waitFor(() => expect(band("1 kHz")).toHaveAttribute("aria-valuenow", "6"));
  });

  it("presets set the whole curve", async () => {
    await backend.settingsSet({ eqEnabled: true });
    await loadSettings();
    render(<SettingsPage />);
    const options = within(screen.getByRole("combobox", { name: "Preset" }))
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(options).toEqual(EQ_PRESETS.map((p) => p.name));
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Preset" }), "bassBoost");
    expect(band("31 Hz")).toHaveAttribute("aria-valuenow", "6");
    await waitFor(async () => expect((await core()).eqBands).toEqual(EQ_PRESETS[1].bands));
    await userEvent.click(screen.getByRole("button", { name: "Reset" }));
    await waitFor(async () => expect((await core()).eqPreset).toBe("flat"));
  });

  it("saves, renames and deletes profiles", async () => {
    await backend.settingsSet({ eqEnabled: true, eqBands: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1] });
    await loadSettings();
    render(<SettingsPage />);
    await userEvent.click(screen.getByRole("button", { name: "Save as" }));
    const dialog = await screen.findByRole("dialog", { name: "Save as profile" });
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Profile name" }), "Flat");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("built-in preset");
    const name = within(dialog).getByRole("textbox", { name: "Profile name" });
    await userEvent.clear(name);
    await userEvent.type(name, "Desk");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("combobox", { name: "Preset" })).toHaveValue("profile:Desk");

    await userEvent.click(screen.getByRole("button", { name: "Rename" }));
    const rn = await screen.findByRole("dialog", { name: "Rename profile" });
    const field = within(rn).getByRole("textbox", { name: "Profile name" });
    expect(field).toHaveValue("Desk");
    await userEvent.clear(field);
    await userEvent.type(field, "Studio{Enter}");
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Preset" })).toHaveValue("profile:Studio"));

    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    const del = await screen.findByRole("dialog", { name: "Delete profile" });
    await userEvent.click(within(del).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Preset" })).toHaveValue("custom"));
    expect((await core()).eqProfiles).toEqual([]);
  });

  it("a profile bound to an output comes back when that output is used", async () => {
    await backend.settingsSet({ eqEnabled: true, eqPreset: "vocal" });
    await loadSettings();
    render(<SettingsPage />);
    await waitFor(() => expect(useOutput.getState().state).toBeTruthy());
    // Headphones get Bass boost; the speakers keep Vocal.
    await act(() => mock.outputSelect("mock:headphones"));
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Preset" }), "bassBoost");
    await waitFor(async () => expect((await core()).eqPreset).toBe("bassBoost"));
    const use = await screen.findByRole("switch", { name: "Use for this output" });
    await waitFor(() => expect(use).toBeEnabled());
    await userEvent.click(use);
    await waitFor(() => expect(use).toHaveAttribute("aria-checked", "true"));
    expect(screen.getByText(/Headphones \(USB Audio DAC\) always plays with Bass boost\./)).toBeInTheDocument();

    await act(() => mock.outputSelect(null));
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Preset" })).toHaveValue("vocal"));
    expect(screen.getByText(/Headphones \(USB Audio DAC\) uses Bass boost\./)).toBeInTheDocument();
    await act(() => mock.outputSelect("mock:headphones"));
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Preset" })).toHaveValue("bassBoost"));
  });

  it("a custom curve has to be saved before it can follow an output", async () => {
    await backend.settingsSet({ eqEnabled: true, eqBands: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0] });
    await loadSettings();
    render(<SettingsPage />);
    expect(await screen.findByRole("switch", { name: "Use for this output" })).toBeDisabled();
  });
});
