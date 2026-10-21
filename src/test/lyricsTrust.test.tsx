/**
 * The quiet parts of the lyrics view: the line under the lyrics and its
 * options menu, "Wrong lyrics?" and its picker, no lyrics for a song, the
 * timing nudge (buttons and keys) and Tap to sync behind Timing is off, the
 * check running without a word, what shows when lyrics are held back, the
 * song menu item and Settings.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyrics, mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type Lyrics, type PlayerState, type Track, type TranscribeStatus } from "../lib/types";
import { trackMenuItems } from "../components/TrackRow";
import { LyricsPane } from "../shell/LyricsView";
import { actionableNotice, clearLyricsCache, lyricsChanged, loadLyrics, loadLyricsInfo, lyricsSeen, setCheckStatus, useLyricsTrust } from "../shell/lyrics";
import {
  NOTICE_TEXT,
  offsetText,
  OPTIONS_LABEL,
  resetCheckClocks,
  tapOffset,
  TIMING_ITEM,
  UNAVAILABLE_NOTE,
  WrongLyricsDialog,
} from "../shell/LyricsTrust";
import { SettingsPage } from "../pages/SettingsPage";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useTranscribe } from "../stores/transcribe";
import { useUi } from "../stores/ui";
import { useWrongLyrics } from "../stores/wrongLyrics";

const mock = backend as MockBackend;
const byKind = (k: ReturnType<typeof mockLyricsKind>) => mock.fx.tracks.find((t) => mockLyricsKind(t.id) === k && t.durationMs > 120_000)!;
const lines = byKind("lines");
const plain = byKind("plain");

function state(track: Track | null): PlayerState {
  return {
    status: "paused",
    track,
    positionMs: 0,
    durationMs: track?.durationMs ?? 0,
    volume: 0.7,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: null, userQueue: [], nextUp: [], history: [], context: null },
  };
}

function at(track: Track, positionMs: number) {
  act(() => usePlayer.setState({ state: state(track), positionMs, durationMs: track.durationMs }));
}

/** Another song's words, timed like the server's, found on LRCLIB. */
function otherSong(track: Track): Lyrics {
  const l = mockLyrics(track)!;
  const words = ["Midnight on the bridge", "Counting every headlight", "Write my name in silver", "Selling every story"];
  return {
    ...l,
    source: "lrclib",
    credit: "Lyrics from LRCLIB, matched by title",
    lines: l.lines.map((_, i) => words[i % words.length]),
    timed: l.timed?.map((t, i) => ({ startMs: t.startMs, text: t.text ? words[i % words.length] : "" })),
  };
}

/** The options menu's items, opened from the button under the lyrics. */
async function menuLabels(): Promise<string[]> {
  fireEvent.click(await screen.findByRole("button", { name: OPTIONS_LABEL }));
  const labels = useUi.getState().menu?.items.map((i) => i.label) ?? [];
  act(() => useUi.setState({ menu: null }));
  return labels;
}

/** Picks `label` in the options menu under the lyrics. */
async function option(label: string) {
  fireEvent.click(await screen.findByRole("button", { name: OPTIONS_LABEL }));
  const item = useUi.getState().menu?.items.find((i) => i.label === label);
  if (!item) throw new Error(`no "${label}" in the lyrics options`);
  act(() => {
    useUi.setState({ menu: null });
    item.onSelect?.();
  });
}

/** The first line with words, on screen. */
const firstLine = (track: Track) => mockLyrics(track)!.lines.find((l) => l.trim())!;

function status(installed: boolean): TranscribeStatus {
  return {
    enabled: false,
    available: true,
    model: "small",
    modelPicked: false,
    models: [{ kind: "small", sizeBytes: 190 * 1024 * 1024, installed, downloading: false, doneBytes: 0 }],
    queued: 0,
    batch: { done: 0, total: 0 },
  };
}

beforeEach(async () => {
  mock.transcribe.reset();
  mock.own.clear();
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  clearLyricsCache();
  resetCheckClocks();
  mock.lyricsDown = false;
  mock.lyricsChoices.clear();
  mock.lyricsExtra.clear();
  mock.lyricsVerdicts.clear();
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useTranscribe.setState({ status: null, empty: new Set() });
  useWrongLyrics.setState({ track: null, tapping: null, offsets: {} });
  useRouter.getState().reset();
  mock.checkStatuses.clear();
  useLyricsTrust.setState({ checks: {} });
});
afterEach(() => {
  vi.restoreAllMocks();
});

describe("the line under the lyrics", () => {
  it("says where they came from and holds everything else in one options menu", async () => {
    at(lines, 0);
    render(<LyricsPane />);
    expect(await screen.findByRole("button", { name: OPTIONS_LABEL })).toBeInTheDocument();
    // Synced lyrics: timing is in the menu, not on screen.
    expect(screen.queryByRole("group", { name: "Lyrics timing" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Tap to sync" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Wrong lyrics?" })).toBeNull();
    expect(await menuLabels()).toEqual([TIMING_ITEM, "Wrong lyrics?", "Write your own", "Share lyrics"]);
  });

  it("never shows the check or asks to download the speech model", async () => {
    const download = vi.spyOn(backend, "transcribeModelDownload").mockResolvedValue(undefined);
    useTranscribe.setState({ status: status(false) });
    mock.lyricsVerdicts.set(lines.id, { server: "match" });
    at(lines, 0);
    render(<LyricsPane />);
    await screen.findAllByRole("button", { name: firstLine(lines) });
    for (const t of [/Not checked/, /^Checked/, /Checking/, /speech model/, /Other lyrics found/]) expect(screen.queryByText(t)).toBeNull();
    expect(download).not.toHaveBeenCalled();
  });

  it("stays quiet while the check runs, and the lyrics show at once", async () => {
    const real = mock.lyrics.bind(mock);
    vi.spyOn(mock, "lyrics").mockImplementation(async (id) => {
      const l = await real(id);
      return l && { ...l, check: "checking" };
    });
    mock.checkStatuses.set(lines.id, { trackId: lines.id, state: "listening" });
    at(lines, 0);
    render(<LyricsPane />);
    await screen.findAllByRole("button", { name: firstLine(lines) });
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByText(/check/i)).toBeNull();
  });
});

describe("Wrong lyrics?", () => {
  it("lists every set found with source, album, length, sync and check, and picks one", async () => {
    mock.lyricsExtra.set(lines.id, [{ key: "lrclib:aa", origin: "lrclibSearch", album: "Night Bridge", durationMs: 204_000, lyrics: otherSong(lines) }]);
    mock.lyricsVerdicts.set(lines.id, { server: "match", "lrclib:aa": "noMatch" });
    at(lines, 0);
    render(
      <>
        <LyricsPane />
        <WrongLyricsDialog />
      </>,
    );
    await option("Wrong lyrics?");
    const dialog = await screen.findByRole("dialog", { name: "Wrong lyrics?" });
    const rows = await within(dialog).findAllByRole("radio");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveAttribute("aria-checked", "true");
    expect(rows[0]).toHaveTextContent("From your server");
    expect(rows[0]).toHaveTextContent("Matches the song");
    expect(rows[0]).toHaveTextContent("Synced");
    expect(rows[1]).toHaveTextContent("LRCLIB, found by title · Night Bridge · 3:24 · Synced");
    expect(rows[1]).toHaveTextContent("Doesn't match the song");
    // The user always wins: the other one, even though it didn't match.
    fireEvent.click(rows[1]);
    await waitFor(() => expect(mock.lyricsChoices.get(lines.id)).toEqual({ pick: "lrclib:aa", offsetMs: 0 }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect((await screen.findAllByRole("button", { name: "Midnight on the bridge" })).length).toBeGreaterThan(0);
  });

  it("No lyrics for this song, then Show lyrics again", async () => {
    at(lines, 0);
    render(
      <>
        <LyricsPane />
        <WrongLyricsDialog />
      </>,
    );
    await option("Wrong lyrics?");
    fireEvent.click(await screen.findByRole("button", { name: "No lyrics for this song" }));
    expect(await screen.findByText(NOTICE_TEXT.chosenNone)).toBeInTheDocument();
    expect(mock.lyricsChoices.get(lines.id)?.pick).toBe("none");
    fireEvent.click(screen.getByRole("button", { name: "Show lyrics again" }));
    expect((await screen.findAllByRole("button", { name: firstLine(lines) })).length).toBeGreaterThan(0);
    expect(mock.lyricsChoices.has(lines.id)).toBe(false);
  });

  it("offers Transcribe this song and Write or paste lyrics", async () => {
    const track = vi.spyOn(backend, "transcribeTrack");
    at(lines, 0);
    useWrongLyrics.setState({ track: lines });
    render(<WrongLyricsDialog />);
    const dialog = await screen.findByRole("dialog", { name: "Wrong lyrics?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Transcribe this song" }));
    await waitFor(() => expect(track).toHaveBeenCalledWith(lines.id));
    // Transcription was off: asking for it here turns it on.
    expect((await mock.settingsGet()).transcribeLyrics).toBe(true);
    await waitFor(() => expect(mock.lyricsChoices.get(lines.id)?.pick).toBe("auto"));
  });

  it("is in the song menu once the song's lyrics were looked up", async () => {
    const label = () => trackMenuItems(lines).filter((i) => !i.when || i.when()).map((i) => i.label);
    expect(label()).not.toContain("Wrong lyrics?");
    await loadLyrics(lines.id);
    expect(lyricsSeen(lines.id)).toBe(true);
    expect(label()).toContain("Wrong lyrics?");
    trackMenuItems(lines).find((i) => i.label === "Wrong lyrics?")!.onSelect!();
    expect(useWrongLyrics.getState().track?.id).toBe(lines.id);
  });
});

describe("timing", () => {
  it("nudges by half a second with the buttons and the - and + keys", async () => {
    at(lines, 0);
    render(<LyricsPane />);
    const first = mockLyrics(lines)!.timed!.find((t) => t.text)!;
    await option(TIMING_ITEM);
    const later = await screen.findByRole("button", { name: "Show lyrics 0.5 seconds later" });
    await waitFor(() => expect(later).toBeEnabled());
    fireEvent.click(later);
    await waitFor(() => expect(mock.lyricsChoices.get(lines.id)?.offsetMs).toBe(500));
    expect(await screen.findByRole("button", { name: `Timing moved ${offsetText(500)}. Reset` })).toBeInTheDocument();
    expect((await loadLyrics(lines.id))!.timed!.find((t) => t.text)!.startMs).toBe(first.startMs + 500);
    fireEvent.keyDown(window, { key: "+" });
    await waitFor(() => expect(mock.lyricsChoices.get(lines.id)?.offsetMs).toBe(1000));
    await screen.findByRole("button", { name: `Timing moved ${offsetText(1000)}. Reset` });
    fireEvent.keyDown(window, { key: "-" });
    await waitFor(() => expect(mock.lyricsChoices.get(lines.id)?.offsetMs).toBe(500));
    await screen.findByRole("button", { name: `Timing moved ${offsetText(500)}. Reset` });
    fireEvent.click(screen.getByRole("button", { name: "Show lyrics 0.5 seconds earlier" }));
    await waitFor(() => expect(mock.lyricsChoices.has(lines.id)).toBe(false));
    expect(screen.queryByRole("button", { name: /Timing moved/ })).toBeNull();
  });

  it("keeps the lines on screen while the song's lyrics are asked for again", async () => {
    at(lines, 0);
    render(<LyricsPane />);
    const first = mockLyrics(lines)!.timed!.find((t) => t.text)!;
    const [line] = await screen.findAllByRole("button", { name: first.text });
    // Each line follows its own text direction (right-to-left lyrics align right).
    expect(line.getAttribute("dir")).toBe("auto");
    // A check landing or a nudge asks again: no skeleton, no remount.
    act(() => lyricsChanged(lines.id));
    expect(screen.getAllByRole("button", { name: first.text })[0]).toBe(line);
    await waitFor(() => expect(useLyricsTrust.getState().version).toBeGreaterThan(0));
    await act(() => loadLyrics(lines.id));
    expect(screen.getAllByRole("button", { name: first.text })[0]).toBe(line);
  });

  it("never lets an answer from before a change replace the newer one", async () => {
    const old = mockLyrics(lines)!;
    const fresh = { ...old, offsetMs: 500 };
    const answers: ((l: Lyrics) => void)[] = [];
    vi.spyOn(mock, "lyrics").mockImplementation(() => new Promise<Lyrics | null>((r) => answers.push(r)));
    const slow = loadLyrics(lines.id);
    lyricsChanged(lines.id);
    const quick = loadLyrics(lines.id);
    answers[1](fresh);
    await quick;
    answers[0](old);
    await slow;
    vi.restoreAllMocks();
    expect((await loadLyrics(lines.id))!.offsetMs).toBe(500);
  });

  it("Tap to sync lines up the clicked line with now", async () => {
    const t = mockLyrics(lines)!.timed!;
    const target = t.find((l, i) => i > 2 && l.text)!;
    at(lines, target.startMs + 2_340);
    const seek = vi.spyOn(backend, "playSeek");
    render(<LyricsPane />);
    // Wait for the lines themselves, so a late lyrics load can't reset tap mode after the click.
    await screen.findAllByRole("button", { name: target.text });
    await option(TIMING_ITEM);
    const tap = await screen.findByRole("button", { name: "Tap to sync" });
    await waitFor(() => expect(tap).toBeEnabled());
    fireEvent.click(tap);
    await waitFor(() => expect(screen.getByRole("button", { name: "Tap to sync" })).toHaveAttribute("aria-pressed", "true"));
    expect(screen.getByText(/Click a line as it's sung/)).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: target.text }).find((b) => b.classList.contains("lyrics__line"))!);
    await waitFor(() => expect(mock.lyricsChoices.get(lines.id)?.offsetMs).toBe(2_300));
    expect(seek).not.toHaveBeenCalled();
    expect(tapOffset({ startMs: 10_000 }, 9_000, 500)).toBe(-500);
  });

  it("counts every quick press: shown at once, written one at a time, none lost", async () => {
    let inFlight = 0;
    let most = 0;
    const real = mock.lyricsOffset.bind(mock);
    const write = vi.spyOn(backend, "lyricsOffset").mockImplementation(async (id, ms) => {
      most = Math.max(most, ++inFlight);
      try {
        return await real(id, ms);
      } finally {
        inFlight--;
      }
    });
    at(lines, 0);
    render(<LyricsPane />);
    await option(TIMING_ITEM);
    const later = await screen.findByRole("button", { name: "Show lyrics 0.5 seconds later" });
    // The nudge waits while the check runs; on a slow machine it may not have landed yet.
    await waitFor(() => expect(later).toBeEnabled());
    fireEvent.click(later);
    fireEvent.click(later);
    fireEvent.keyDown(window, { key: "+" });
    // All three show before any answer comes back.
    expect(screen.getByRole("button", { name: `Timing moved ${offsetText(1500)}. Reset` })).toBeInTheDocument();
    await waitFor(() => expect(mock.lyricsChoices.get(lines.id)?.offsetMs).toBe(1500));
    expect(most).toBe(1);
    expect(write.mock.calls[write.mock.calls.length - 1]?.[1]).toBe(1500);
    await waitFor(() => expect(useWrongLyrics.getState().offsets[lines.id]).toBeUndefined());
    expect(screen.getByRole("button", { name: `Timing moved ${offsetText(1500)}. Reset` })).toBeInTheDocument();
  });

  it("Tap to sync uses where playback is this instant, not the last tick", async () => {
    let now = 1_000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    const t = mockLyrics(lines)!.timed!;
    const target = t.find((l, i) => i > 2 && l.text)!;
    at(lines, 0);
    act(() => usePlayer.setState({ state: { ...state(lines), status: "playing" }, positionMs: target.startMs + 2_340 }));
    render(<LyricsPane />);
    await screen.findAllByRole("button", { name: target.text });
    await option(TIMING_ITEM);
    const tap = await screen.findByRole("button", { name: "Tap to sync" });
    await waitFor(() => expect(tap).toBeEnabled());
    fireEvent.click(tap);
    await waitFor(() => expect(screen.getByRole("button", { name: "Tap to sync" })).toHaveAttribute("aria-pressed", "true"));
    // 200 ms after the last position tick.
    now = 1_200;
    fireEvent.click(screen.getAllByRole("button", { name: target.text }).find((b) => b.classList.contains("lyrics__line"))!);
    await waitFor(() => expect(mock.lyricsChoices.get(lines.id)?.offsetMs).toBe(2_500));
  });

  it("holds the nudge while the check may still move the timing itself", async () => {
    const synced = mockLyrics(lines)!;
    const spy = vi.spyOn(backend, "lyrics").mockResolvedValue({ ...synced, check: "checking" });
    const write = vi.spyOn(backend, "lyricsOffset");
    at(lines, 0);
    render(<LyricsPane />);
    await option(TIMING_ITEM);
    const later = await screen.findByRole("button", { name: "Show lyrics 0.5 seconds later" });
    expect(later).toBeDisabled();
    expect(screen.getByRole("button", { name: "Tap to sync" })).toBeDisabled();
    fireEvent.keyDown(window, { key: "+" });
    fireEvent.click(later);
    expect(write).not.toHaveBeenCalled();
    // The check landed (and fixed the timing itself): the nudge is back.
    spy.mockResolvedValue({ ...synced, check: "checked", autoOffsetMs: 1500 });
    act(() => lyricsChanged(lines.id));
    await waitFor(() => expect(screen.getByRole("button", { name: "Show lyrics 0.5 seconds later" })).toBeEnabled());
    fireEvent.keyDown(window, { key: "+" });
    await waitFor(() => expect(write).toHaveBeenCalledWith(lines.id, 500));
  });

  it("lets the nudge back if the check stalls", async () => {
    vi.spyOn(backend, "lyrics").mockResolvedValue({ ...mockLyrics(lines)!, check: "checking" });
    at(lines, 0);
    render(<LyricsPane />);
    await option(TIMING_ITEM);
    expect(await screen.findByRole("button", { name: "Show lyrics 0.5 seconds later" })).toBeDisabled();
    act(() => setCheckStatus({ trackId: lines.id, state: "failed", reason: "stuck" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Show lyrics 0.5 seconds later" })).toBeEnabled());
  });

  it("has no timing nudge for lyrics without timing", async () => {
    at(plain, 0);
    render(<LyricsPane />);
    const labels = await menuLabels();
    expect(labels).toContain("Wrong lyrics?");
    expect(labels).not.toContain(TIMING_ITEM);
  });

  it("closes the timing bar with Done, and the - and + keys work without it", async () => {
    at(lines, 0);
    render(<LyricsPane />);
    await option(TIMING_ITEM);
    const bar = await screen.findByRole("toolbar", { name: "Lyrics timing tools" });
    await waitFor(() => expect(within(bar).getByRole("button", { name: "Show lyrics 0.5 seconds later" })).toBeEnabled());
    fireEvent.click(within(bar).getByRole("button", { name: "Done" }));
    expect(screen.queryByRole("toolbar", { name: "Lyrics timing tools" })).toBeNull();
    fireEvent.keyDown(window, { key: "+" });
    await waitFor(() => expect(mock.lyricsChoices.get(lines.id)?.offsetMs).toBe(500));
  });
});

describe("when nothing is shown", () => {
  it("holds released lyrics back from another version until Use anyway", async () => {
    const choose = vi.spyOn(backend, "lyricsChoose");
    vi.spyOn(backend, "lyrics").mockResolvedValue({ synced: false, lines: [], notice: "versionHeld", pick: "lrclib:ab12" });
    at(lines, 0);
    render(<LyricsPane />);
    expect(await screen.findByText(NOTICE_TEXT.versionHeld)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Use anyway" }));
    await waitFor(() => expect(choose).toHaveBeenCalledWith(lines.id, "lrclib:ab12"));
  });

  it("shows the loading lines while lyrics are held for the check, then none matched as no lyrics", async () => {
    const spy = vi.spyOn(backend, "lyrics").mockResolvedValue({ synced: false, lines: [], notice: "checking" });
    at(lines, 0);
    render(<LyricsPane />);
    expect(await screen.findByLabelText("Loading lyrics")).toBeInTheDocument();
    expect(screen.queryByText(NOTICE_TEXT.checking)).toBeNull();
    // The check finished and nothing matched: the song is a song without lyrics.
    spy.mockResolvedValue({ synced: false, lines: [], notice: "noneMatched" });
    act(() => lyricsChanged(lines.id));
    expect(await screen.findByText("No lyrics for this song")).toBeInTheDocument();
  });
});

describe("couldn't ask, instrumentals and Look again", () => {
  it("a source that couldn't be reached isn't 'no lyrics': Retry, Wrong lyrics?, the button stays on", async () => {
    mock.lyricsUnreachable = true;
    try {
      at(lines, 0);
      render(<LyricsPane />);
      expect(await screen.findByText(NOTICE_TEXT.unavailable)).toBeInTheDocument();
      expect(screen.getByText(UNAVAILABLE_NOTE)).toBeInTheDocument();
      expect(screen.queryByText("No lyrics for this song")).toBeNull();
      expect(screen.queryByRole("button", { name: "Add lyrics" })).toBeNull();
      // The song's menu offers Wrong lyrics? for it.
      expect(lyricsSeen(lines.id)).toBe(true);
      expect(actionableNotice({ synced: false, lines: [], notice: "unavailable" })).toBe(true);
      // Back: Retry asks again and the lyrics show.
      mock.lyricsUnreachable = false;
      fireEvent.click(screen.getByRole("button", { name: "Retry" }));
      expect((await screen.findAllByRole("button", { name: firstLine(lines) })).length).toBeGreaterThan(0);
    } finally {
      mock.lyricsUnreachable = false;
    }
  });

  it("isn't cached, and coming back online asks again", async () => {
    mock.lyricsUnreachable = true;
    const ask = vi.spyOn(backend, "lyrics");
    try {
      at(lines, 0);
      render(<LyricsPane />);
      expect(await screen.findByText(NOTICE_TEXT.unavailable)).toBeInTheDocument();
      expect((await loadLyricsInfo(lines.id))?.notice).toBe("unavailable");
      expect(ask).toHaveBeenCalledTimes(2);
      mock.lyricsUnreachable = false;
      act(() => {
        window.dispatchEvent(new Event("online"));
      });
      expect((await screen.findAllByRole("button", { name: firstLine(lines) })).length).toBeGreaterThan(0);
    } finally {
      mock.lyricsUnreachable = false;
    }
  });

  it("says when a song is instrumental, without Add lyrics, and dims the button", async () => {
    mock.lyricsDown = true;
    mock.lyricsInstrumental.add(lines.id);
    try {
      at(lines, 0);
      render(<LyricsPane />);
      expect(await screen.findByText(NOTICE_TEXT.instrumental)).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Add lyrics" })).toBeNull();
      expect(actionableNotice({ synced: false, lines: [], notice: "instrumental" })).toBe(false);
    } finally {
      mock.lyricsDown = false;
      mock.lyricsInstrumental.clear();
    }
  });

  it("Look again in Wrong lyrics? forgets the song's misses and asks again", async () => {
    at(lines, 0);
    useWrongLyrics.setState({ track: lines });
    const list = vi.spyOn(backend, "lyricsCandidates");
    render(<WrongLyricsDialog />);
    const dialog = await screen.findByRole("dialog", { name: "Wrong lyrics?" });
    const look = await within(dialog).findByRole("button", { name: "Look again" });
    const before = list.mock.calls.length;
    fireEvent.click(look);
    await waitFor(() => expect(mock.lookedAgain).toContain(lines.id));
    await waitFor(() => expect(list.mock.calls.length).toBeGreaterThan(before));
    expect(await within(dialog).findByRole("button", { name: "Look again" })).toBeInTheDocument();
  });
});

describe("settings", () => {
  it("has the released lyrics and choices rows; the check and timing fixes just run", async () => {
    mock.lyricsChoices.set(lines.id, { offsetMs: 500 });
    render(<SettingsPage />);
    const row = (key: string) => document.querySelector<HTMLElement>(`[data-setting="${key}"]`)!;
    for (const k of ["lyricsCheckAudio", "lyricsFixTiming", "lyricsOfficialFirst", "lyricsShowAdd"]) expect(row(k)).toBeNull();
    expect(within(row("lyricsReleasedForVersions")).getByRole("switch")).not.toBeChecked();
    fireEvent.click(within(row("lyricsReleasedForVersions")).getByRole("switch"));
    await waitFor(async () => expect((await mock.settingsGet()).lyricsReleasedForVersions).toBe(true));
    expect(await within(row("lyricsChoices")).findByText("1 song")).toBeInTheDocument();
    fireEvent.click(within(row("lyricsChoices")).getByRole("button", { name: "Reset lyrics choices" }));
    await waitFor(() => expect(mock.lyricsChoices.size).toBe(0));
    expect(await within(row("lyricsChoices")).findByText("0 songs")).toBeInTheDocument();
  });
});
