/**
 * The phone's now playing screen: the title sized to its length, the album
 * line, the neighbours in the cover row out of sight at rest, the format
 * plate for every known format (and the signal path behind it), the next
 * song at the foot, and a record with no cover lighting the screen with
 * its printed sleeve's tone.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type PlayerState, type QueueItem, type Track } from "../lib/types";
import { usePhoneForm } from "../phone/form";
import { PhoneShell } from "../phone/PhoneShell";
import { sleeveTint, titleFit } from "../phone/NowPlaying";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { signalPlateText } from "../phone/SignalSheet";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";
import { useSignal } from "../stores/signalPath";

const mock = backend as MockBackend;
const tracks = (): Track[] => mock.fx.tracks;
const last = <T,>(a: T[]): T | undefined => a[a.length - 1];
const item = (t: Track, i: number): QueueItem => ({ itemId: `q${i}`, track: t, source: "context" });

function play(t: Track, extra: Partial<PlayerState> = {}) {
  const s: PlayerState = {
    status: "playing",
    track: t,
    positionMs: 30_000,
    durationMs: t.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: item(t, 0), userQueue: [], nextUp: tracks().slice(1, 5).map((x, i) => item(x, i + 1)), history: [], context: { kind: "album", id: t.albumId, title: t.albumTitle } },
    ...extra,
  };
  act(() => usePlayer.setState({ state: s, local: s, positionMs: s.positionMs, durationMs: s.durationMs, remote: null }));
}

async function open(t: Track, extra: Partial<PlayerState> = {}) {
  play(t, extra);
  render(<PhoneShell />);
  act(() => phone.openPlayer());
  return screen.findByRole("dialog", { name: "Now playing" });
}

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  usePhoneUi.setState({ tab: "home", nowPlaying: false, lyrics: false, sheets: [] });
  useRouter.getState().reset({ name: "home" });
  useSignal.setState({ path: null, open: false });
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
  vi.restoreAllMocks();
});

describe("phone now playing", () => {
  it("sizes the title by its length", () => {
    expect(titleFit("Hum")).toBe("short");
    expect(titleFit("Sideways Into Summer Rain")).toBe("medium");
    expect(titleFit("The Long Way Round the Reservoir at Night (Alternate Take)")).toBe("long");
  });

  it("names the album under the artist, unless it is what's playing from", async () => {
    const t = tracks()[0];
    const np = await open(t, { queue: { current: item(t, 0), userQueue: [], nextUp: [], history: [], context: { kind: "playlist", id: "p1", title: "Road" } } });
    const album = np.querySelector(".ph-np__album") as HTMLElement;
    expect(album).toHaveTextContent(t.albumTitle);
    fireEvent.click(album);
    expect(last(useRouter.getState().entries)?.route).toEqual({ name: "album", id: t.albumId });
    play(t);
    act(() => phone.openPlayer());
    const again = await screen.findByRole("dialog", { name: "Now playing" });
    expect(again.querySelector(".ph-np__album")).toBeNull();
  });

  it("From links back to the album, playlist or artist it plays from", async () => {
    const t = tracks()[0];
    const np = await open(t);
    fireEvent.click(within(np).getByRole("button", { name: /^From / }));
    expect(last(useRouter.getState().entries)?.route).toEqual({ name: "album", id: t.albumId });
  });

  it("keeps the neighbouring covers out of sight until the row moves", async () => {
    await open(tracks()[0]);
    const row = screen.getByTestId("np-deck");
    expect(row).not.toHaveClass("is-moving");
    const art = screen.getByTestId("np-art");
    Object.defineProperty(art, "clientWidth", { value: 400, configurable: true });
    let now = 1000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    fireEvent.pointerDown(art, { pointerId: 1, button: 0, clientX: 300, clientY: 400 });
    for (let i = 1; i <= 4; i++) {
      now += 250;
      fireEvent.pointerMove(art, { pointerId: 1, clientX: 300 - i * 10, clientY: 400 });
    }
    expect(row).toHaveClass("is-moving");
    await act(async () => void fireEvent.pointerUp(art, { pointerId: 1, clientX: 260, clientY: 400 }));
    // A short drag springs back; at rest the neighbours are hidden again.
    await waitFor(() => expect(row).not.toHaveClass("is-moving"));
  });

  it("shows the next song at the foot, and a tap opens the queue", async () => {
    const t = tracks();
    const np = await open(t[0]);
    const next = within(np).getByRole("button", { name: new RegExp(`^Up next: ${t[1].title.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`) });
    fireEvent.click(next);
    expect(last(usePhoneUi.getState().sheets)).toEqual({ kind: "queue" });
  });

  it("the format plate shows lossy formats too, and falls back on the signal path's word", () => {
    expect(signalPlateText({ codec: "mp3", bitDepth: null, sampleRate: 44100, bitrate: 320 }, null, null)).toBe("MP3 320");
    const path = { source: { codec: "FLAC", lossless: true, bitDepth: 24, sampleRate: 96000, channels: 2, bitrate: null, transcoded: false, from: null } } as never;
    expect(signalPlateText({ codec: null, bitDepth: null, sampleRate: null, bitrate: null }, null, path)).toBe("FLAC 24/96");
    expect(signalPlateText({ codec: null, bitDepth: null, sampleRate: null, bitrate: null }, null, null)).toBeNull();
  });

  it("the plate is shown for a lossy song and opens the signal path", async () => {
    const t = { ...tracks()[0], codec: "mp3", bitDepth: null, sampleRate: 44100, bitrate: 320 };
    const np = await open(t);
    const plate = within(np).getByRole("button", { name: /^Signal path/ });
    expect(plate).toHaveTextContent("MP3 320");
    fireEvent.click(plate);
    expect(last(usePhoneUi.getState().sheets)).toEqual({ kind: "signal" });
    expect(await screen.findByRole("heading", { name: "Signal path" })).toBeInTheDocument();
  });

  it("with no cover, the screen takes the printed sleeve's tone", async () => {
    const t = { ...tracks()[0], thumb: null };
    const np = await open(t);
    expect(np.style.getPropertyValue("--ph-tint")).toBe(sleeveTint(t.albumTitle));
    expect(sleeveTint("Notes From the Long Porch")).toMatch(/^#[0-9a-f]{6}$/);
    expect(np.querySelector("[data-testid=fallback-sleeve]")).not.toBeNull();
  });
});
