/**
 * Lyrics transcribed on this computer: the Settings rows (switch, model with
 * its size, download, delete, the batch), the lyrics view's offer and its
 * progress, the auto-transcribed note with Edit, and each failure state
 * (model download failed, checksum mismatch, out of disk, transcription
 * failed) shown in plain words.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { LyricsPage } from "../pages/LyricsPage";
import { SettingsPage } from "../pages/SettingsPage";
import { batchNote, transcribeNote } from "../pages/TranscribeSettings";
import { clearLyricsCache, lyricsSourceLabel } from "../shell/lyrics";
import { LyricsEditorHost } from "../shell/LyricsEditor";
import { useLyricsEditor } from "../stores/lyricsEditor";
import { usePlayer } from "../stores/player";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { modelSize, resetTranscribe, startTranscribe, useTranscribe } from "../stores/transcribe";

const mock = backend as MockBackend;
const none = mock.fx.tracks.find((t) => mockLyricsKind(t.id) === "none" && t.durationMs > 120_000)!;
const other = mock.fx.tracks.filter((t) => mockLyricsKind(t.id) === "none" && t.durationMs > 120_000)[1]!;

function playing(track: Track) {
  const state: PlayerState = {
    status: "paused",
    track,
    positionMs: 0,
    durationMs: track.durationMs,
    volume: 0.7,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: null, userQueue: [], nextUp: [], history: [], context: null },
  };
  act(() => usePlayer.setState({ state, positionMs: 0, durationMs: track.durationMs }));
}

async function turnOn(extra = {}) {
  await mock.settingsSet({ transcribeLyrics: true, ...extra });
  await loadSettings();
}

function settingRow(key: string) {
  return document.querySelector(`[data-setting="${key}"]`) as HTMLElement;
}

beforeEach(async () => {
  clearLyricsCache();
  mock.own.clear();
  mock.transcribe.reset();
  mock.transcribe.stepMs = 5;
  await mock.settingsSet({ transcribeLyrics: false, transcribeModel: "small", showAutoLyrics: true, lyricsShowAdd: true });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useLyricsEditor.setState({ track: null, ownIds: new Set(), version: 0 });
  useToasts.setState({ toasts: [] });
  resetTranscribe();
  await startTranscribe();
});

describe("Settings", () => {
  it("is off by default, shows model sizes before downloading, downloads and deletes", async () => {
    expect(DEFAULT_SETTINGS.transcribeLyrics).toBe(false);
    expect(DEFAULT_SETTINGS.showAutoLyrics).toBe(true);
    render(<SettingsPage />);
    await waitFor(() => expect(settingRow("transcribeLyrics")).toBeTruthy());
    const sw = within(settingRow("transcribeLyrics")).getByRole("switch");
    expect(sw).not.toBeChecked();
    expect(within(settingRow("transcribeUnreleased")).getByRole("button", { name: "Transcribe now" })).toBeDisabled();
    await userEvent.click(sw);
    await waitFor(() => expect(within(settingRow("transcribeLyrics")).getByText(/181 MB\) downloads the first time/)).toBeInTheDocument());
    const download = await within(settingRow("transcribeModel")).findByRole("button", { name: "Download 181 MB" });
    await userEvent.click(download);
    await within(settingRow("transcribeModel")).findByRole("button", { name: "Delete model" });
    expect(useTranscribe.getState().status?.models[0].installed).toBe(true);
    await userEvent.click(within(settingRow("transcribeModel")).getByRole("button", { name: "Delete model" }));
    await within(settingRow("transcribeModel")).findByRole("button", { name: "Download 181 MB" });
  });

  it.each([
    ["download", /download failed\. Check your connection/],
    ["checksum", /didn't match its checksum, so it was deleted/],
    ["disk", /Not enough free space\. The model needs \d+ MB free/],
  ] as const)("says when the model download fails (%s)", async (kind, text) => {
    await turnOn();
    mock.transcribe.failNextDownload = kind;
    render(<SettingsPage />);
    await userEvent.click(await within(settingRow("transcribeModel")).findByRole("button", { name: /^Download/ }));
    const alert = await within(settingRow("transcribeLyrics")).findByRole("alert");
    expect(alert).toHaveTextContent(text);
    expect(useTranscribe.getState().status?.models[0].installed).toBe(false);
    // Trying again works.
    await userEvent.click(within(settingRow("transcribeModel")).getByRole("button", { name: /^Download/ }));
    await within(settingRow("transcribeModel")).findByRole("button", { name: "Delete model" });
  });

  it("runs the unreleased batch with progress and can cancel it", async () => {
    await turnOn();
    mock.transcribe.stepMs = 30;
    render(<SettingsPage />);
    await userEvent.click(await within(settingRow("transcribeUnreleased")).findByRole("button", { name: "Transcribe now" }));
    await within(settingRow("transcribeUnreleased")).findByText(/Transcribing: \d+ of \d+ songs\./);
    await userEvent.click(within(settingRow("transcribeUnreleased")).getByRole("button", { name: "Cancel" }));
    await within(settingRow("transcribeUnreleased")).findByRole("button", { name: "Transcribe now" });
  });

  it("lists whisper.cpp and the model source in the notices", async () => {
    const { NOTICES, SOURCE_NOTICES } = await import("../pages/SettingsPage");
    expect(NOTICES.some((n) => n.name.startsWith("whisper.cpp") && n.licence === "MIT")).toBe(true);
    expect(SOURCE_NOTICES.some((n) => n.name.startsWith("Hugging Face"))).toBe(true);
  });

  it("explains pauses and an unavailable computer in plain words", () => {
    const base = useTranscribe.getState().status!;
    expect(transcribeNote({ ...base, available: false, unavailableReason: undefined })).toMatch(/isn't available/);
    const on = { ...base, enabled: true };
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, paused: "battery" })).toBe("2 of 9 songs. Paused while on battery power.");
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, paused: "slowConnection" })).toMatch(/slow connection/);
    expect(modelSize(539_225_533)).toBe("514 MB");
    expect(modelSize(1_600_000_000)).toBe("1.5 GB");
  });
});

describe("Lyrics view", () => {
  function page() {
    return render(
      <>
        <LyricsPage />
        <LyricsEditorHost />
      </>,
    );
  }

  it("offers nothing while transcription is off", async () => {
    playing(none);
    page();
    await screen.findByText("No lyrics for this song");
    expect(screen.queryByRole("button", { name: "Transcribe this song" })).toBeNull();
  });

  it("transcribes a song, downloading the model first, and marks the result", async () => {
    await turnOn();
    playing(none);
    page();
    await screen.findByText(/Downloads the 181 MB speech model first/);
    await userEvent.click(screen.getByRole("button", { name: "Transcribe this song" }));
    await screen.findByText(/Downloading the speech model|Transcribing…/);
    await screen.findByText("Auto-transcribed, may be wrong.", undefined, { timeout: 3000 });
    expect(screen.getByText("I walk the long way back to you")).toBeInTheDocument();
    // Edit opens the editor on the transcription.
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    await screen.findByRole("dialog", { name: /lyrics: / });
    expect(mock.own.get(none.id)?.auto).toBe("small");
  });

  it("hides transcriptions when Show auto-transcribed lyrics is off", async () => {
    await turnOn({ showAutoLyrics: false });
    mock.own.set(none.id, { trackId: none.id, text: "[00:01.00]Heard words", prefer: false, updatedAt: 1, auto: "small" });
    playing(none);
    page();
    await screen.findByText("No lyrics for this song");
    expect(lyricsSourceLabel({ synced: true, lines: [], source: "auto" })).toBe("Auto-transcribed, may be wrong.");
  });

  it("says when transcription failed and offers to try again", async () => {
    await turnOn();
    mock.transcribe.failNextSong = true;
    playing(other);
    page();
    await userEvent.click(await screen.findByRole("button", { name: "Transcribe this song" }));
    const alert = await screen.findByRole("alert", undefined, { timeout: 3000 });
    expect(alert).toHaveTextContent("Transcription failed for this song.");
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByText("Auto-transcribed, may be wrong.", undefined, { timeout: 3000 });
  });

  it("says when no words were found", async () => {
    await turnOn();
    mock.transcribe.silent.add(other.id);
    playing(other);
    page();
    await userEvent.click(await screen.findByRole("button", { name: "Transcribe this song" }));
    await screen.findByText("No words were found when this song was transcribed.", undefined, { timeout: 3000 });
    expect(screen.queryByRole("button", { name: "Transcribe this song" })).toBeNull();
  });

  it("cancels a song being transcribed", async () => {
    await turnOn();
    mock.transcribe.stepMs = 40;
    playing(other);
    page();
    await userEvent.click(await screen.findByRole("button", { name: "Transcribe this song" }));
    await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    await screen.findByRole("button", { name: "Transcribe this song" });
    expect(mock.own.has(other.id)).toBe(false);
  });
});
