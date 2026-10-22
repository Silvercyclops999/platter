import { act, render, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import { bindingFor } from "../lib/hifi";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type SignalPath } from "../lib/types";
import { onPhone } from "../pages/settingsMeta";
import { SettingsPage } from "../pages/SettingsPage";
import { stages } from "../shell/SignalPath";
import { connectApp, tunedToast } from "../stores/app";
import { loadOutput, useOutput } from "../stores/output";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const AURORA = { id: "mock:aurora", name: "Adam's Aurora Studio 600", isDefault: false };

beforeEach(async () => {
  mock.resetOutputs();
  await backend.outputSelect(null);
  await backend.settingsSet({ ...DEFAULT_SETTINGS, headphoneCorrection: [], headphoneDeclined: [], bitPerfectOutputs: [], tuneHeadphones: true });
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

async function playOn(id: string) {
  await backend.outputSelect(id);
  await loadOutput();
  await loadSettings();
}

describe("tune for my headphones", () => {
  it("is on by default, under Sound's More, and on the phone too", async () => {
    expect(DEFAULT_SETTINGS.tuneHeadphones).toBe(true);
    render(<SettingsPage />);
    const sw = within(row("tuneHeadphones")).getByRole("switch");
    expect(sw).toBeChecked();
    expect(row("tuneHeadphones")).toHaveTextContent("Tune for my headphones");
    expect(onPhone({ key: "tuneHeadphones" })).toBe(true);
  });

  it("known headphones get their correction and say so with an Undo", async () => {
    mock.plugOutput(AURORA);
    await playOn(AURORA.id);
    const s = useSettings.getState().settings!;
    expect(s.headphoneCorrection).toEqual([expect.objectContaining({ deviceId: AURORA.id, name: "Aurora Studio 600", auto: true })]);
    // The toast, as the app shows it.
    tunedToast({ deviceId: AURORA.id, name: "Aurora Studio 600" }, backend);
    const t = lastToast();
    expect(t.message).toBe("Tuned for Aurora Studio 600");
    expect(t.action?.label).toBe("Undo");
  });

  it("Undo turns it off for those headphones for good", async () => {
    mock.plugOutput(AURORA);
    await playOn(AURORA.id);
    tunedToast({ deviceId: AURORA.id, name: "Aurora Studio 600" }, backend);
    await act(async () => lastToast().action!.run());
    await vi.waitFor(() => expect(useSettings.getState().settings?.headphoneCorrection).toEqual([]));
    expect(useSettings.getState().settings?.headphoneDeclined).toContain(AURORA.id);
    // Away and back: not picked again.
    await playOn("mock:speakers");
    await playOn(AURORA.id);
    expect(useSettings.getState().settings?.headphoneCorrection).toEqual([]);
  });

  it("Undo after the sound moved on leaves the other output alone", async () => {
    mock.plugOutput(AURORA);
    await playOn(AURORA.id);
    tunedToast({ deviceId: AURORA.id, name: "Aurora Studio 600" }, backend);
    await playOn("mock:speakers");
    await act(async () => lastToast().action!.run());
    await vi.waitFor(() => expect(lastToast().message).toBe("Couldn't undo that: Those headphones aren't playing any more."));
    expect(useSettings.getState().settings?.headphoneCorrection).toEqual([expect.objectContaining({ deviceId: AURORA.id })]);
    expect(useSettings.getState().settings?.headphoneDeclined ?? []).not.toContain(AURORA.id);
  });

  it("speakers and names that aren't one model are left alone", async () => {
    await playOn("mock:headphones");
    expect(useSettings.getState().settings?.headphoneCorrection).toEqual([]);
    mock.plugOutput({ id: "route:speaker:Speaker", name: "Aurora Studio 600", isDefault: false });
    await playOn("route:speaker:Speaker");
    expect(useSettings.getState().settings?.headphoneCorrection).toEqual([]);
    mock.plugOutput({ id: "mock:kestrel", name: "Kestrel", isDefault: false });
    await playOn("mock:kestrel");
    expect(useSettings.getState().settings?.headphoneCorrection).toEqual([]);
  });

  it("switched off: nothing is picked, and picked ones stop playing until it's on again", async () => {
    mock.plugOutput(AURORA);
    await playOn(AURORA.id);
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(within(row("tuneHeadphones")).getByRole("switch"));
    await vi.waitFor(() => expect(useSettings.getState().settings?.tuneHeadphones).toBe(false));
    const s = useSettings.getState().settings!;
    expect(bindingFor(s, AURORA)).toBeUndefined();
    expect(within(row("headphoneCorrection")).getByRole("switch")).not.toBeChecked();
    await user.click(within(row("tuneHeadphones")).getByRole("switch"));
    await vi.waitFor(() => expect(bindingFor(useSettings.getState().settings, AURORA)?.name).toBe("Aurora Studio 600"));
  });

  it("a picked correction is shown as picked, and offers the likely headphones when changed", async () => {
    mock.plugOutput(AURORA);
    await playOn(AURORA.id);
    const user = userEvent.setup();
    render(<SettingsPage />);
    expect(row("headphoneCorrection")).toHaveTextContent("Picked for");
    await user.click(within(row("headphoneCorrection")).getByRole("button", { name: "Change" }));
    expect(await within(row("headphoneCorrection")).findByText("These look like your headphones.")).toBeInTheDocument();
    const list = within(row("headphoneCorrection")).getByRole("list", { name: "Headphones" });
    expect(within(list).getAllByRole("button")[0]).toHaveTextContent("Aurora Studio 600");
  });

  it("the signal path names the headphones it's tuned for", () => {
    const p = {
      eq: { eq: false, correction: true, correctionPreampDb: -3, preampDb: 0, limiter: false },
    } as unknown as SignalPath;
    const eq = (tuned: string | null) => stages({ ...emptyPath(), ...p }, null, tuned).find((s) => s.name === "EQ")!.value;
    expect(eq("Aurora Studio 600")).toBe("Tuned for Aurora Studio 600");
    expect(eq(null)).toBe("Headphone correction");
  });

  it("names are cut down to the model like the core does, maker in front or not", async () => {
    mock.plugOutput({ id: "mock:studio", name: "Headphones (2- Studio 600 Stereo)", isDefault: false });
    await playOn("mock:studio");
    expect(useSettings.getState().settings?.headphoneCorrection).toEqual([expect.objectContaining({ name: "Aurora Studio 600", auto: true })]);
  });

  it("bit-perfect headphones are left alone", async () => {
    mock.plugOutput(AURORA);
    await backend.outputSelect(AURORA.id);
    await backend.headphoneCorrectionSet(null);
    await backend.settingsSet({ headphoneDeclined: [] });
    await backend.bitPerfectSet(true);
    await playOn("mock:speakers");
    await playOn(AURORA.id);
    expect(useSettings.getState().settings?.headphoneCorrection).toEqual([]);
  });

  it("the phone has no bit-perfect row (the system mixes every app's sound)", () => {
    expect(onPhone({ key: "bitPerfectOutputs" })).toBe(false);
  });

  it("the app shows the core's tuned message", async () => {
    const stop = connectApp(mock);
    act(() => mock.emit("audio://tuned", { deviceId: "x", name: "Aurora Studio 600" }));
    await vi.waitFor(() => expect(useToasts.getState().toasts.some((t) => t.message === "Tuned for Aurora Studio 600")).toBe(true));
    stop();
  });
});

function emptyPath(): SignalPath {
  return {
    trackId: "t",
    verdict: { kind: "lossless" },
    source: null,
    decode: null,
    loudness: { on: false, mode: "off", gainDb: null },
    eq: { eq: false, correction: false, correctionPreampDb: null, preampDb: 0, limiter: false },
    resample: null,
    blend: { automix: false, crossfade: "off", crossfadeMs: 0 },
    output: { mode: "shared", volumeBypassed: false, device: null, rate: null, bits: null },
    why: null,
  } as unknown as SignalPath;
}

function lastToast() {
  const list = useToasts.getState().toasts;
  return list[list.length - 1];
}
