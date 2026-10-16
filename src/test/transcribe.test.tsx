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
import { mockMatched, mockPlexMissed } from "../lib/mock/fixtures";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { LyricsPane } from "../shell/LyricsView";
import { stageText } from "../shell/TranscribeOffer";
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

  it("the unreleased batch takes albums Plex couldn't match unless that setting is off", async () => {
    await turnOn();
    const tracksOf = (match: (a: (typeof mock.fx.albums)[number]) => boolean) =>
      mock.fx.albums.filter(match).reduce((n, a) => n + (mock.fx.tracksByAlbum.get(a.id)?.length ?? 0), 0);
    const demos = tracksOf((a) => a.subformats.includes("demo"));
    // Albums Plex couldn't match, less the one MusicBrainz says Plex missed.
    const builds = tracksOf((a) => !mockMatched(a) && !mockPlexMissed(mock.fx.albums, a));
    expect(builds).toBeGreaterThan(0);
    expect(await mock.transcribeUnreleased()).toBe(demos + builds);
    mock.transcribe.reset();
    await mock.settingsSet({ countUnmatchedUnreleased: false });
    expect(await mock.transcribeUnreleased()).toBe(demos);
    await mock.settingsSet({ countUnmatchedUnreleased: true });
  });

  it("lists whisper.cpp and the model source in the notices", async () => {
    const { NOTICES, SOURCE_NOTICES } = await import("../pages/SettingsPage");
    expect(NOTICES.some((n) => n.name.startsWith("whisper.cpp") && n.licence === "MIT")).toBe(true);
    expect(SOURCE_NOTICES.some((n) => n.name.startsWith("Hugging Face"))).toBe(true);
  });

  it("has processor use, the busy pause and away mode, all this device only", async () => {
    expect(DEFAULT_SETTINGS.transcribeProcessor).toBe("light");
    expect(DEFAULT_SETTINGS.transcribePauseBusy).toBe(true);
    expect(DEFAULT_SETTINGS.transcribeFasterAway).toBe(true);
    await turnOn();
    render(<SettingsPage />);
    const use = await within(await waitFor(() => settingRow("transcribeProcessor"))).findByRole("combobox", { name: "Processor use" });
    expect(use).toHaveValue("light");
    expect(within(use).getAllByRole("option").map((o) => o.textContent)).toEqual(["Light", "Balanced", "Fast"]);
    await userEvent.selectOptions(use, "fast");
    await waitFor(async () => expect((await mock.settingsGet()).transcribeProcessor).toBe("fast"));
    const pause = within(settingRow("transcribePauseBusy")).getByRole("switch", { name: "Pause while the computer is busy" });
    expect(pause).toBeChecked();
    await userEvent.click(pause);
    await waitFor(async () => expect((await mock.settingsGet()).transcribePauseBusy).toBe(false));
    const away = within(settingRow("transcribeFasterAway")).getByRole("switch", { name: "Work faster while I'm away" });
    expect(away).toBeChecked();
    for (const key of ["transcribeProcessor", "transcribePauseBusy", "transcribeFasterAway"]) {
      expect(settingRow(key)).toHaveTextContent("This device only.");
    }
    // The model picker says Medium is slower and harder on the processor; Small stays the default.
    expect(DEFAULT_SETTINGS.transcribeModel).toBe("small");
    expect(settingRow("transcribeModel")).toHaveTextContent(/Medium is .*much slower and works the processor much harder/);
    expect(within(settingRow("transcribeModel")).getByRole("option", { name: /Medium \(slower, much harder on the processor\)/ })).toBeInTheDocument();
    await mock.settingsSet({ transcribeProcessor: "light", transcribePauseBusy: true });
  });

  it("shows a busy pause and the pace of a long run", async () => {
    await turnOn();
    mock.transcribe.stepMs = 30;
    mock.transcribe.busy = true;
    render(<SettingsPage />);
    await userEvent.click(await within(settingRow("transcribeUnreleased")).findByRole("button", { name: "Transcribe now" }));
    await within(settingRow("transcribeUnreleased")).findByText(/\d+ of \d+ songs\. Paused while your computer is busy\./);
    await userEvent.click(within(settingRow("transcribeUnreleased")).getByRole("button", { name: "Cancel" }));
  });

  it("explains pauses and an unavailable computer in plain words", () => {
    const base = useTranscribe.getState().status!;
    expect(transcribeNote({ ...base, available: false, unavailableReason: undefined })).toMatch(/isn't available/);
    const on = { ...base, enabled: true };
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, paused: "battery" })).toBe("2 of 9 songs. Paused while on battery power.");
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, paused: "offline" })).toMatch(/can't be reached/);
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, paused: "busy" })).toBe("2 of 9 songs. Paused while your computer is busy.");
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, secsPerSong: 170 })).toBe("Transcribing: 2 of 9 songs, about 3 minutes per song.");
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, secsPerSong: 40 })).toBe("Transcribing: 2 of 9 songs, about 1 minute per song.");
    expect(modelSize(539_225_533)).toBe("514 MB");
    expect(modelSize(1_600_000_000)).toBe("1.5 GB");
  });
});

describe("Lyrics view", () => {
  function page() {
    return render(
      <>
        <LyricsPane />
        <LyricsEditorHost />
      </>,
    );
  }

  it("offers to transcribe a live version showing borrowed timing, and the result replaces it", async () => {
    const live = mock.fx.tracks.find(
      (t) => mockLyricsKind(t.id) === "lines" && !!mock.fx.albums.find((a) => a.id === t.albumId)?.subformats.includes("live"),
    )!;
    await turnOn();
    playing(live);
    page();
    await screen.findByText("Timing is from another version");
    await userEvent.click(await screen.findByRole("button", { name: "Transcribe this song" }));
    await screen.findByText("Auto-transcribed, may be wrong.", undefined, { timeout: 3000 });
    expect(screen.queryByText("Timing is from another version")).toBeNull();
  });

  it("doesn't offer it for a studio song with synced lyrics", async () => {
    const studio = mock.fx.tracks.find(
      (t) => mockLyricsKind(t.id) === "lines" && !mock.fx.albums.find((a) => a.id === t.albumId)?.subformats.includes("live") && !/live/i.test(t.title),
    )!;
    await turnOn();
    playing(studio);
    page();
    await screen.findByLabelText("Lyrics", { selector: ".lyrics__scroll" });
    expect(screen.queryByRole("button", { name: "Transcribe this song" })).toBeNull();
  });

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
    await screen.findByText(/Downloading the speech model|Transcribing…|Reading the song…/);
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

  it("says what the speech helper is doing, and when nothing moves for a while", () => {
    const s = useTranscribe.getState().status!;
    const c = { trackId: "1", stage: "transcribing" as const, percent: 40, background: false };
    expect(stageText(c, s)).toBe("Transcribing… 40%");
    expect(stageText({ ...c, percent: 0 }, s)).toBe("Transcribing…");
    expect(stageText({ ...c, stage: "loading", percent: 0 }, s)).toBe("Loading the speech model…");
    expect(stageText({ ...c, quietSecs: 150 }, s)).toBe("Transcribing… 40%. No progress for 2 minutes. Your computer may be busy; it keeps going.");
    expect(stageText({ ...c, quietSecs: 70 }, s)).toMatch(/No progress for a minute\./);
    for (const t of [stageText(c, s), stageText({ ...c, quietSecs: 400 }, s)]) {
      expect(t).not.toMatch(/Starting|\u2014/);
    }
  });

  it("cancels the song being transcribed and offers it again", async () => {
    await turnOn();
    mock.transcribe.stepMs = 5000;
    playing(other);
    page();
    await userEvent.click(await screen.findByRole("button", { name: "Transcribe this song" }));
    const note = await screen.findByRole("status");
    await userEvent.click(within(note).getByRole("button", { name: "Cancel" }));
    await screen.findByRole("button", { name: "Transcribe this song" });
    expect(useTranscribe.getState().status?.current).toBeUndefined();
  });

  it("says a song asked for waits for the background song to pause", async () => {
    await turnOn();
    playing(other);
    page();
    await screen.findByRole("button", { name: "Transcribe this song" });
    const s = useTranscribe.getState().status!;
    act(() =>
      useTranscribe.setState({
        status: { ...s, current: { trackId: "elsewhere", stage: "transcribing", percent: 10, background: true }, queued: 1, upNext: other.id },
      }),
    );
    const note = await screen.findByRole("status");
    expect(note).toHaveTextContent("Pausing the background song first…");
    expect(within(note).getByRole("button", { name: "Cancel" })).toBeInTheDocument();
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
