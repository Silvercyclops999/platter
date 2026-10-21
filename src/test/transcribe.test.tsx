/**
 * Lyrics written down on this computer: the Settings rows (the switch, on by
 * default, and the batch), songs written down as they play (the model
 * downloading by itself the first time), the lyrics view's progress, the
 * "Written down by Platter" line with Edit lyrics, and each failure state
 * (model download failed, checksum mismatch, out of disk, transcription
 * failed) shown in plain words.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyricsKind } from "../lib/mock/details";
import { mockMatched, mockPlexMissed } from "../lib/mock/fixtures";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { LyricsPane } from "../shell/LyricsView";
import { FIRST_TIME_TEXT, NO_WORDS_TEXT, WRITING_TEXT, writingPercent } from "../shell/TranscribeOffer";
import { lyricsFrom, OPTIONS_LABEL } from "../shell/LyricsTrust";
import { mockFollow } from "../lib/mock/transcribe";
import { useUi } from "../stores/ui";
import { SettingsPage } from "../pages/SettingsPage";
import { batchNote, modelNote, paceNote, speedNote, transcribeNote } from "../pages/TranscribeSettings";
import { clearLyricsCache } from "../shell/lyrics";
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
  await mock.settingsSet({ transcribeLyrics: false, transcribeModel: "small", transcribeGpu: "auto", showAutoLyrics: true, lyricsShowAdd: true, ...ONLINE_EXTRAS });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS } });
  useLyricsEditor.setState({ track: null, ownIds: new Set(), version: 0 });
  useToasts.setState({ toasts: [] });
  resetTranscribe();
  await startTranscribe();
});

describe("Settings", () => {
  it("is on by default on a computer, with no model or processor to pick", async () => {
    expect(DEFAULT_SETTINGS.transcribeLyrics).toBe(true);
    expect(DEFAULT_SETTINGS.showAutoLyrics).toBe(true);
    await loadSettings();
    render(<SettingsPage />);
    await waitFor(() => expect(settingRow("transcribeLyrics")).toBeTruthy());
    expect(settingRow("transcribeLyrics")).toHaveTextContent("Write down lyrics for songs without them");
    const sw = within(settingRow("transcribeLyrics")).getByRole("switch");
    expect(sw).not.toBeChecked();
    // Its own settings (the Lyrics transcription page) wait until it's on.
    expect(settingRow("transcribeUnreleased")).toBeNull();
    await userEvent.click(sw);
    await waitFor(() => expect(within(settingRow("transcribeUnreleased")).getByRole("button", { name: "Transcribe now" })).toBeInTheDocument());
    await waitFor(() => expect(within(settingRow("transcribeLyrics")).getByText(/181 MB\) downloads by itself the first time/)).toBeInTheDocument());
    for (const k of ["transcribeModel", "transcribeGpu", "transcribeProcessor"]) expect(settingRow(k)).toBeNull();
  });

  it.each([
    ["download", /download failed\. Check your connection/],
    ["checksum", /didn't match its checksum, so it was deleted/],
    ["disk", /Not enough free space\. The model needs \d+ MB free/],
  ] as const)("says when the model download fails (%s), and the next try works", async (kind, text) => {
    await turnOn();
    mock.transcribe.failNextDownload = kind;
    playing(none);
    render(
      <>
        <LyricsPane />
        <SettingsPage />
      </>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Write down the words" }));
    // Said plainly in the lyrics view and on the setting.
    await waitFor(() => expect(screen.queryAllByRole("alert").filter((a) => text.test(a.textContent ?? "")).length).toBe(2), { timeout: 3000 });
    expect(useTranscribe.getState().status?.models[0].installed).toBe(false);
    // Trying again works.
    const view = document.querySelector<HTMLElement>(".lyrics-page__empty-wrap")!;
    await userEvent.click(within(view).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(useTranscribe.getState().status?.models[0].installed).toBe(true), { timeout: 3000 });
  });

  it("runs the unreleased batch with progress and can cancel it", async () => {
    await turnOn();
    mock.transcribe.stepMs = 30;
    render(<SettingsPage />);
    await userEvent.click(await within(settingRow("transcribeUnreleased")).findByRole("button", { name: "Transcribe now" }));
    await within(settingRow("transcribeUnreleased")).findByText(/Transcribing( “.+”)?: \d+ of \d+ songs\./);
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

  it("has the busy pause and away mode, this device only", async () => {
    expect(DEFAULT_SETTINGS.transcribePauseBusy).toBe(true);
    expect(DEFAULT_SETTINGS.transcribeFasterAway).toBe(true);
    await turnOn();
    render(<SettingsPage />);
    const pause = within(await waitFor(() => settingRow("transcribePauseBusy"))).getByRole("switch", { name: "Pause while the computer is busy" });
    expect(pause).toBeChecked();
    await userEvent.click(pause);
    await waitFor(async () => expect((await mock.settingsGet()).transcribePauseBusy).toBe(false));
    const away = within(settingRow("transcribeFasterAway")).getByRole("switch", { name: "Work faster while I'm away" });
    expect(away).toBeChecked();
    for (const key of ["transcribePauseBusy", "transcribeFasterAway"]) {
      expect(settingRow(key)).toHaveTextContent("This device only.");
    }
    await mock.settingsSet({ transcribePauseBusy: true });
  });

  it("tells each model's speed from this computer's own runs when there are some", () => {
    const base = useTranscribe.getState().status!;
    const turbo = { kind: "turbo" as const, sizeBytes: 1, installed: true, downloading: false, doneBytes: 0 };
    const onCard = { ...base, gpu: "RTX 5090" };
    expect(speedNote(turbo, onCard)).toBe("a few seconds per song on your graphics card");
    expect(speedNote({ ...turbo, secsPerSong: 9 }, onCard)).toBe("about 9 seconds per song on your graphics card");
    expect(speedNote({ ...turbo, kind: "small", secsPerSong: 200 }, { ...base, gpu: undefined })).toBe("about 3 minutes per song on the processor");
    expect(speedNote({ ...turbo, secsPerSong: 1 }, onCard)).toBe("about 1 second per song on your graphics card");
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
    // A paired Bridge does the batch at night instead of this computer.
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, paused: "bridge" })).toBe(
      "2 of 9 songs. Paused: your Bridge transcribes these at night, so this computer doesn't have to.",
    );
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, secsPerSong: 170 })).toBe("Transcribing: 2 of 9 songs, about 3 minutes per song.");
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, secsPerSong: 40 })).toBe("Transcribing: 2 of 9 songs, about 40 seconds per song.");
    expect(batchNote({ ...on, batch: { done: 2, total: 9 }, secsPerSong: 20 })).toBe("Transcribing: 2 of 9 songs, about 20 seconds per song.");
    expect(modelSize(539_225_533)).toBe("514 MB");
    expect(modelSize(1_600_000_000)).toBe("1.5 GB");
  });
});

describe("Settings: batch pause, model switch and speed", () => {
  it("says the model's size and speed", () => {
    const base = useTranscribe.getState().status!;
    expect(modelNote({ ...base, gpu: undefined, model: "small" })).toBe(`${modelSize(190_098_681)}, a few minutes per song on the processor.`);
    expect(paceNote(20)).toBe("about 20 seconds per song");
    expect(paceNote(170)).toBe("about 3 minutes per song");
  });

  it("pauses and resumes the batch, keeping its progress and queue", async () => {
    await turnOn();
    mock.transcribe.install("small");
    mock.transcribe.stepMs = 20;
    render(<SettingsPage />);
    const row = await waitFor(() => settingRow("transcribeUnreleased"));
    await userEvent.click(await within(row).findByRole("button", { name: "Transcribe now" }));
    await userEvent.click(await within(row).findByRole("button", { name: "Pause" }));
    await within(row).findByText(/\d+ of \d+ songs\. Paused\./);
    const held = useTranscribe.getState().status!;
    expect(held.userPaused).toBe(true);
    expect(held.batch.total).toBeGreaterThan(1);
    // Nothing more is done while paused.
    await waitFor(() => expect(useTranscribe.getState().status?.current).toBeUndefined());
    const done = useTranscribe.getState().status!.batch.done;
    await new Promise((r) => setTimeout(r, 100));
    expect(useTranscribe.getState().status!.batch.done).toBe(done);
    expect(within(row).getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    await userEvent.click(within(row).getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(useTranscribe.getState().status!.batch.done).toBeGreaterThan(done));
    await userEvent.click(within(row).getByRole("button", { name: "Cancel" }));
    await within(row).findByRole("button", { name: "Transcribe now" });
  });

  it("says which song it's on, and when it waits for music that's playing", async () => {
    await turnOn();
    mock.transcribe.install("small");
    mock.transcribe.stepMs = 20;
    mock.transcribe.playing = true;
    render(<SettingsPage />);
    const row = await waitFor(() => settingRow("transcribeUnreleased"));
    await userEvent.click(await within(row).findByRole("button", { name: "Transcribe now" }));
    await within(row).findByText(/^Waiting: music is playing\. \d+ of \d+ songs\./);
    mock.transcribe.playing = false;
    await waitFor(() => {
      const c = useTranscribe.getState().status?.current;
      const title = mock.fx.tracks.find((t) => t.id === c?.trackId)?.title;
      expect(title).toBeTruthy();
      expect(row).toHaveTextContent(`Transcribing “${title}”:`);
    });
    await userEvent.click(within(row).getByRole("button", { name: "Cancel" }));
  });

  it("keeps going with the installed model while a new one downloads, then switches and offers a redo", async () => {
    await turnOn({ transcribeModel: "small" });
    mock.transcribe.install("small");
    mock.transcribe.stepMs = 15;
    render(<SettingsPage />);
    const row = await waitFor(() => settingRow("transcribeUnreleased"));
    await userEvent.click(await within(row).findByRole("button", { name: "Transcribe now" }));
    // A bigger model is picked and downloads; songs keep coming with Small.
    await mock.settingsSet({ transcribeModel: "medium" });
    await loadSettings();
    await mock.transcribeModelDownload("medium");
    await within(row).findByText(/Using the small model until the medium model finishes downloading, then the rest switch to it\./);
    await waitFor(() => expect(useTranscribe.getState().status?.models.find((m) => m.kind === "medium")?.installed).toBe(true), { timeout: 3000 });
    await waitFor(() => expect(useTranscribe.getState().status?.current?.model).toBe("medium"), { timeout: 3000 });
    // Songs done with Small can be done again with Medium.
    const redo = await within(row).findByRole("button", { name: /^Redo \d+ songs?$/ });
    expect(row).toHaveTextContent(/done with the small model while the medium model downloaded/);
    await userEvent.click(within(row).getByRole("button", { name: "Cancel" }));
    await userEvent.click(redo);
    await waitFor(() => expect(useTranscribe.getState().status?.batch.total).toBeGreaterThan(0));
    await waitFor(() => expect(useTranscribe.getState().status?.redo ?? []).toEqual([]), { timeout: 5000 });
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

  it("writes down a song without lyrics as it starts, downloading the model the first time", async () => {
    await turnOn();
    mockFollow.on = true;
    try {
      // The model downloads slowly enough to see the first-time line.
      mock.transcribe.stepMs = 40;
      await mock.playContext({ context: { kind: "album", id: none.albumId! }, startTrackId: none.id });
      const t = usePlayer.getState().state?.track ?? none;
      playing(t);
      page();
      await screen.findByText(FIRST_TIME_TEXT);
      expect(screen.getByRole("progressbar", { name: FIRST_TIME_TEXT })).toBeInTheDocument();
      mock.transcribe.stepMs = 5;
      await screen.findByText(lyricsFrom({ synced: true, lines: [], source: "auto" })!, undefined, { timeout: 5000 });
      expect(screen.getByText("I walk the long way back to you")).toBeInTheDocument();
      // No button asked for it.
      expect(screen.queryByRole("button", { name: /Transcribe|Write down the words/ })).toBeNull();
    } finally {
      mockFollow.on = false;
      await mock.playPause().catch(() => undefined);
    }
  });

  it("doesn't write down a studio song that has synced lyrics, or a live one showing borrowed timing", async () => {
    const studio = mock.fx.tracks.find(
      (t) => mockLyricsKind(t.id) === "lines" && !mock.fx.albums.find((a) => a.id === t.albumId)?.subformats.includes("live") && !/live/i.test(t.title),
    )!;
    await turnOn();
    playing(studio);
    page();
    await screen.findByLabelText("Lyrics", { selector: ".lyrics__scroll" });
    expect(screen.queryByText(WRITING_TEXT)).toBeNull();
    expect(screen.queryByRole("button", { name: "Write down the words" })).toBeNull();
  });

  it("offers nothing to write down while it's off, only Write your own", async () => {
    await mock.settingsSet({ transcribeLyrics: false });
    await loadSettings();
    playing(none);
    page();
    await screen.findByText("No lyrics for this song");
    expect(screen.queryByRole("button", { name: "Write down the words" })).toBeNull();
    expect(screen.getByRole("button", { name: "Write your own" })).toBeInTheDocument();
  });

  it("shows the progress, then the lyrics with where they came from and Edit lyrics", async () => {
    await turnOn();
    mock.transcribe.stepMs = 30;
    playing(none);
    page();
    await userEvent.click(await screen.findByRole("button", { name: "Write down the words" }));
    await screen.findByRole("progressbar");
    await screen.findByText("Written down by Platter", undefined, { timeout: 5000 });
    expect(screen.getByText("I walk the long way back to you")).toBeInTheDocument();
    // Edit lyrics in the options opens the editor on the transcription.
    await userEvent.click(screen.getByRole("button", { name: OPTIONS_LABEL }));
    const edit = useUi.getState().menu?.items.find((i) => i.label === "Edit lyrics");
    expect(edit).toBeTruthy();
    act(() => {
      useUi.setState({ menu: null });
      edit!.onSelect!();
    });
    await screen.findByRole("dialog", { name: /lyrics: / });
    expect(mock.own.get(none.id)?.auto).toBe("small");
  });

  it("hides transcriptions when Show auto-transcribed lyrics is off", async () => {
    await turnOn({ showAutoLyrics: false });
    mock.own.set(none.id, { trackId: none.id, text: "[00:01.00]Heard words", prefer: false, updatedAt: 1, auto: "small" });
    playing(none);
    page();
    await screen.findByText("No lyrics for this song");
  });

  it("says how far along it is: the model download's share, then the song's", () => {
    const s = useTranscribe.getState().status!;
    const c = { trackId: "1", stage: "transcribing" as const, percent: 40, background: false };
    expect(writingPercent(c, s)).toBe(40);
    expect(writingPercent({ ...c, percent: 0 }, s)).toBeNull();
    expect(writingPercent({ ...c, stage: "loading", percent: 0 }, s)).toBeNull();
    const downloading = { ...s, models: s.models.map((m) => (m.kind === s.model ? { ...m, doneBytes: m.sizeBytes / 4 } : m)) };
    expect(writingPercent({ ...c, stage: "waitingForModel", percent: 0 }, downloading)).toBe(25);
  });

  it("says a song waiting its turn is being written down", async () => {
    await turnOn();
    playing(other);
    page();
    await screen.findByRole("button", { name: "Write down the words" });
    const s = useTranscribe.getState().status!;
    act(() =>
      useTranscribe.setState({
        status: { ...s, current: { trackId: "elsewhere", stage: "transcribing", percent: 10, background: true }, queued: 1, upNext: other.id },
      }),
    );
    const note = await screen.findByRole("status");
    expect(note).toHaveTextContent(WRITING_TEXT);
  });

  it("says when transcription failed and offers to try again", async () => {
    await turnOn();
    mock.transcribe.failNextSong = true;
    playing(other);
    page();
    await userEvent.click(await screen.findByRole("button", { name: "Write down the words" }));
    const alert = await screen.findByRole("alert", undefined, { timeout: 3000 });
    expect(alert).toHaveTextContent("Transcription failed for this song.");
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByText("Written down by Platter", undefined, { timeout: 3000 });
  });

  it("says when no words were found", async () => {
    await turnOn();
    mock.transcribe.silent.add(other.id);
    playing(other);
    page();
    await userEvent.click(await screen.findByRole("button", { name: "Write down the words" }));
    await screen.findByText(NO_WORDS_TEXT, undefined, { timeout: 3000 });
    expect(screen.queryByRole("button", { name: "Write down the words" })).toBeNull();
  });
});
