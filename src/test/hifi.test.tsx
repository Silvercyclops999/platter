import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import { exclusiveText, formatRate } from "../lib/hifi";
import { DEFAULT_SETTINGS, type OutputState } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { HifiLine, HifiMark } from "../shell/HifiMark";
import { loadOutput, selectOutput, useOutput } from "../stores/output";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

beforeEach(async () => {
  await backend.outputSelect(null);
  await backend.settingsSet({ ...DEFAULT_SETTINGS, headphoneCorrection: [], bitPerfectOutputs: [] });
  await loadSettings();
  useOutput.setState({ state: null, switching: undefined });
  await loadOutput();
  useToasts.setState({ toasts: [] });
});

afterEach(() => vi.restoreAllMocks());

function row(key: string): HTMLElement {
  const el = document.querySelector<HTMLElement>(`[data-setting="${key}"]`);
  if (!el) throw new Error(`no row ${key}`);
  return el;
}

describe("headphone correction", () => {
  it("is off by default; turning it on searches, and a pick binds to the device in use", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sw = within(row("headphoneCorrection")).getByRole("switch");
    expect(sw).not.toBeChecked();
    expect(within(row("headphoneCorrection")).queryByRole("searchbox")).toBeNull();
    await user.click(sw);
    const search = within(row("headphoneCorrection")).getByRole("searchbox", { name: "Search for your headphones" });
    await user.type(search, "aurora 600");
    const results = await within(row("headphoneCorrection")).findByRole("list", { name: "Headphones" });
    const picks = within(results).getAllByRole("button");
    expect(picks).toHaveLength(2);
    expect(picks[0]).toHaveTextContent("Aurora Studio 600");
    expect(picks[0]).toHaveTextContent("Lab One • over-ear");
    await user.click(picks[0]);
    const bound = useSettings.getState().settings?.headphoneCorrection ?? [];
    expect(bound).toHaveLength(1);
    expect(bound[0]).toMatchObject({ deviceId: "mock:speakers", name: "Aurora Studio 600", path: "Lab One/over-ear/Aurora Studio 600" });
    expect(bound[0].preampDb).toBeLessThanOrEqual(0);
    expect(within(row("headphoneCorrection")).getByRole("switch")).toBeChecked();
    expect(row("headphoneCorrection")).toHaveTextContent("On for Speakers.");
    expect(row("headphoneCorrection")).toHaveTextContent("Profiles from the AutoEq project (MIT licence)");
    // The search closes once picked.
    expect(within(row("headphoneCorrection")).queryByRole("searchbox")).toBeNull();
  });

  it("says when nothing matches", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(within(row("headphoneCorrection")).getByRole("switch"));
    await user.type(within(row("headphoneCorrection")).getByRole("searchbox"), "zzz");
    expect(await within(row("headphoneCorrection")).findByText('No headphones match "zzz".')).toBeInTheDocument();
  });

  it("A/B compares without saving, and leaving turns the correction back on", async () => {
    const user = userEvent.setup();
    await backend.headphoneCorrectionSet("Lab One/earbud/Tern Earbud");
    await loadSettings();
    const spy = vi.spyOn(backend, "headphoneCorrectionCompare");
    const view = render(<SettingsPage />);
    const group = within(row("headphoneCorrection")).getByRole("group", { name: "Compare" });
    const corrected = within(group).getByRole("button", { name: "Corrected" });
    const original = within(group).getByRole("button", { name: "Original" });
    expect(corrected).toHaveAttribute("aria-pressed", "true");
    await user.click(original);
    expect(original).toHaveAttribute("aria-pressed", "true");
    expect(spy).toHaveBeenLastCalledWith(true);
    view.unmount();
    expect(spy).toHaveBeenLastCalledWith(false);
    expect(useSettings.getState().settings?.headphoneCorrection).toHaveLength(1);
  });

  it("each output device keeps its own", async () => {
    const user = userEvent.setup();
    await backend.headphoneCorrectionSet("Lab One/earbud/Tern Earbud");
    await loadSettings();
    render(<SettingsPage />);
    expect(within(row("headphoneCorrection")).getByRole("switch")).toBeChecked();
    await act(() => selectOutput("mock:headphones"));
    expect(within(row("headphoneCorrection")).getByRole("switch")).not.toBeChecked();
    await act(() => selectOutput(null));
    expect(within(row("headphoneCorrection")).getByRole("switch")).toBeChecked();
    // Turning it off removes it for this device only.
    await act(() => backend.outputSelect("mock:headphones").then(() => backend.headphoneCorrectionSet("Lab One/over-ear/Kestrel Open Back")));
    await act(() => backend.outputSelect(null).then(() => loadOutput()));
    await loadSettings();
    await user.click(within(row("headphoneCorrection")).getByRole("switch"));
    expect(useSettings.getState().settings?.headphoneCorrection.map((c) => c.deviceId)).toEqual(["mock:headphones"]);
  });
});

describe("bit-perfect output", () => {
  it("is off by default and explains what it bypasses", () => {
    render(<SettingsPage />);
    const r = row("bitPerfectOutputs");
    expect(within(r).getByRole("switch")).not.toBeChecked();
    expect(r).toHaveTextContent("volume, the equalizer, headphone correction, loudness leveling and crossfades are bypassed");
  });

  it("turns on for the device in use and says how it plays", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(within(row("bitPerfectOutputs")).getByRole("switch"));
    expect(useSettings.getState().settings?.bitPerfectOutputs).toEqual([{ deviceId: "mock:speakers", deviceName: "Speakers (Built-in Audio)" }]);
    expect(within(row("bitPerfectOutputs")).getByRole("switch")).toBeChecked();
    expect(await within(row("bitPerfectOutputs")).findByText(/^Bit-perfect at .* kHz, \d+-bit\. Volume and sound settings are bypassed\.$/)).toBeInTheDocument();
    // Another device isn't affected.
    await act(() => selectOutput("mock:headphones"));
    expect(within(row("bitPerfectOutputs")).getByRole("switch")).not.toBeChecked();
  });

  it("words for each state", () => {
    expect(formatRate(96000)).toBe("96 kHz");
    expect(formatRate(44100)).toBe("44.1 kHz");
    expect(exclusiveText({ state: "off" })).toBeNull();
    expect(exclusiveText({ state: "on", rate: 96000, bits: 24 })).toBe("Bit-perfect at 96 kHz, 24-bit. Volume and sound settings are bypassed.");
    expect(exclusiveText({ state: "fallback", reason: "Another app is using the device on its own." })).toBe(
      "Playing shared for now: Another app is using the device on its own.",
    );
  });
});

describe("indicators", () => {
  const speakers = { id: "mock:speakers", name: "Speakers (Built-in Audio)", isDefault: true };
  const out = (exclusive: OutputState["exclusive"]): OutputState => ({ devices: [speakers], selected: null, current: speakers, exclusive });

  it("show nothing while neither is on", () => {
    useOutput.setState({ state: out({ state: "off" }) });
    const { container } = render(
      <>
        <HifiMark />
        <HifiLine />
      </>,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("bit-perfect shows its rate in the player bar and its state in the picker", () => {
    const s = useSettings.getState().settings!;
    useSettings.getState().setLocal({ ...s, bitPerfectOutputs: [{ deviceId: speakers.id, deviceName: speakers.name }] });
    useOutput.setState({ state: out({ state: "on", rate: 96000, bits: 24 }) });
    render(
      <>
        <HifiMark />
        <HifiLine />
      </>,
    );
    expect(screen.getByRole("button", { name: "Bit-perfect output, 96 kHz. Open Connect." })).toHaveTextContent("96 kHz");
    expect(screen.getByText("Bit-perfect at 96 kHz, 24-bit. Volume and sound settings are bypassed.")).toBeInTheDocument();
  });

  it("a fallback says why, and correction shows its headphone", () => {
    const s = useSettings.getState().settings!;
    useSettings.getState().setLocal({
      ...s,
      bitPerfectOutputs: [{ deviceId: speakers.id, deviceName: speakers.name }],
      headphoneCorrection: [
        { deviceId: speakers.id, deviceName: speakers.name, name: "Tern Earbud", source: "Lab One", form: "earbud", path: "x/Tern Earbud", preampDb: -3, filters: [] },
      ],
    });
    useOutput.setState({ state: out({ state: "fallback", reason: "The device can't play 96 kHz directly." }) });
    render(
      <>
        <HifiMark />
        <HifiLine />
      </>,
    );
    expect(screen.getByRole("button", { name: "Bit-perfect output is playing shared for now. Open Connect." })).toBeInTheDocument();
    expect(screen.getByText("Playing shared for now: The device can't play 96 kHz directly.")).toBeInTheDocument();
    expect(screen.getByText("Headphone correction: Tern Earbud")).toBeInTheDocument();
  });
});
