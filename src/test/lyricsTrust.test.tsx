/**
 * Lyrics you can trust in the lyrics view: the Checked / Not checked note,
 * "Wrong lyrics?" and its picker, no lyrics for a song, the timing nudge
 * (buttons and keys), Tap to sync, what shows when lyrics are held back or
 * being checked, the speech model offer, the song menu item and Settings.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyrics, mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type Lyrics, type PlayerState, type Track, type TranscribeStatus } from "../lib/types";
import { trackMenuItems } from "../components/TrackRow";
import { LyricsPane } from "../shell/LyricsView";
import { clearLyricsCache, lyricsChanged, loadLyrics, lyricsSeen } from "../shell/lyrics";
import { CHECK_TEXT, NOTICE_TEXT, offsetText, tapOffset, WrongLyricsDialog } from "../shell/LyricsTrust";
import { SettingsPage } from "../pages/SettingsPage";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useTranscribe } from "../stores/transcribe";
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

function status(installed: boolean): TranscribeStatus {
  return {
    enabled: false,
    available: true,
    model: "small",
    models: [{ kind: "small", sizeBytes: 190 * 1024 * 1024, installed, downloading: false, doneBytes: 0 }],
    queued: 0,
    batch: { done: 0, total: 0 },
  };
}

beforeEach(async () => {
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  clearLyricsCache();
  mock.lyricsDown = false;
  mock.lyricsChoices.clear();
  mock.lyricsExtra.clear();
  mock.lyricsVerdicts.clear();
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useTranscribe.setState({ status: null, empty: new Set() });
  useWrongLyrics.setState({ track: null, tapping: null });
  useRouter.getState().reset();
});
afterEach(() => {
  vi.restoreAllMocks();
});

describe("the check note", () => {
  it("says Checked when the song matched, Not checked otherwise", async () => {
    mock.lyricsVerdicts.set(lines.id, { server: "match" });
    at(lines, 0);
    const { unmount } = render(<LyricsPane />);
    expect(await screen.findByText(CHECK_TEXT.checked)).toBeInTheDocument();
    unmount();
    clearLyricsCache();
    mock.lyricsVerdicts.clear();
    render(<LyricsPane />);
    expect(await screen.findByText(CHECK_TEXT.notChecked)).toBeInTheDocument();
    expect(screen.getByRole("toolbar", { name: "Lyrics tools" })).toBeInTheDocument();
  });

  it("makes Wrong lyrics? plain when lyrics found disagree", async () => {
    mock.lyricsExtra.set(lines.id, [{ key: "lrclib:aa", origin: "lrclibSearch", album: "Night Bridge", durationMs: lines.durationMs, lyrics: otherSong(lines) }]);
    at(lines, 0);
    render(<LyricsPane />);
    const wrong = await screen.findByRole("button", { name: "Wrong lyrics?" });
    expect(wrong).toHaveClass("is-prominent");
    expect(screen.getByText("Other lyrics found for this song say something different.")).toBeInTheDocument();
  });

  it("offers the speech model when lyrics can't be checked yet", async () => {
    // Only the request matters here; a real mock download would send status
    // updates that land after the "installed" state set below.
    const download = vi.spyOn(backend, "transcribeModelDownload").mockResolvedValue(undefined);
    useTranscribe.setState({ status: status(false) });
    at(lines, 0);
    render(<LyricsPane />);
    const offer = await screen.findByText(/Download the speech model \(about 190 MB\) so Platter can check lyrics/);
    expect(offer).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Download the speech model" }));
    expect(download).toHaveBeenCalledWith("small");
    // Installed, or checking turned off: no offer.
    act(() => useTranscribe.setState({ status: status(true) }));
    expect(screen.queryByText(/so Platter can check lyrics/)).toBeNull();
    download.mockRestore();
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
    fireEvent.click(await screen.findByRole("button", { name: "Wrong lyrics?" }));
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
    expect(await screen.findByText("Your pick")).toBeInTheDocument();
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
    fireEvent.click(await screen.findByRole("button", { name: "Wrong lyrics?" }));
    fireEvent.click(await screen.findByRole("button", { name: "No lyrics for this song" }));
    expect(await screen.findByText(NOTICE_TEXT.chosenNone)).toBeInTheDocument();
    expect(mock.lyricsChoices.get(lines.id)?.pick).toBe("none");
    fireEvent.click(screen.getByRole("button", { name: "Show lyrics again" }));
    expect(await screen.findByText(CHECK_TEXT.notChecked)).toBeInTheDocument();
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
    fireEvent.click(await screen.findByRole("button", { name: "Show lyrics 0.5 seconds later" }));
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

  it("Tap to sync lines up the clicked line with now", async () => {
    const t = mockLyrics(lines)!.timed!;
    const target = t.find((l, i) => i > 2 && l.text)!;
    at(lines, target.startMs + 2_340);
    const seek = vi.spyOn(backend, "playSeek");
    render(<LyricsPane />);
    fireEvent.click(await screen.findByRole("button", { name: "Tap to sync" }));
    expect(screen.getByRole("button", { name: "Tap to sync" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText(/Click a line as it's sung/)).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: target.text }).find((b) => b.classList.contains("lyrics__line"))!);
    await waitFor(() => expect(mock.lyricsChoices.get(lines.id)?.offsetMs).toBe(2_300));
    expect(seek).not.toHaveBeenCalled();
    expect(tapOffset({ startMs: 10_000 }, 9_000, 500)).toBe(-500);
  });

  it("has no timing nudge for lyrics without timing", async () => {
    at(plain, 0);
    render(<LyricsPane />);
    expect(await screen.findByRole("button", { name: "Wrong lyrics?" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Tap to sync" })).toBeNull();
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

  it("says when lyrics are being checked or none matched, with Wrong lyrics?", async () => {
    const spy = vi.spyOn(backend, "lyrics").mockResolvedValue({ synced: false, lines: [], notice: "checking" });
    at(lines, 0);
    render(<LyricsPane />);
    expect(await screen.findByText(NOTICE_TEXT.checking)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Wrong lyrics?" })).toBeInTheDocument();
    // The check finished: the core says so and the view asks again.
    spy.mockResolvedValue({ synced: false, lines: [], notice: "noneMatched" });
    act(() => lyricsChanged(lines.id));
    expect(await screen.findByText(NOTICE_TEXT.noneMatched)).toBeInTheDocument();
  });
});

describe("settings", () => {
  it("has the check, timing, released lyrics and choices rows", async () => {
    mock.lyricsChoices.set(lines.id, { offsetMs: 500 });
    render(<SettingsPage />);
    const row = (key: string) => document.querySelector<HTMLElement>(`[data-setting="${key}"]`)!;
    for (const k of ["lyricsCheckAudio", "lyricsFixTiming", "lyricsReleasedForVersions"]) expect(row(k)).toBeTruthy();
    expect(within(row("lyricsCheckAudio")).getByRole("switch")).toBeChecked();
    expect(within(row("lyricsFixTiming")).getByRole("switch")).toBeChecked();
    expect(within(row("lyricsReleasedForVersions")).getByRole("switch")).not.toBeChecked();
    fireEvent.click(within(row("lyricsReleasedForVersions")).getByRole("switch"));
    await waitFor(async () => expect((await mock.settingsGet()).lyricsReleasedForVersions).toBe(true));
    fireEvent.click(within(row("lyricsCheckAudio")).getByRole("switch"));
    await waitFor(async () => expect((await mock.settingsGet()).lyricsCheckAudio).toBe(false));
    expect(await within(row("lyricsChoices")).findByText("1 song")).toBeInTheDocument();
    fireEvent.click(within(row("lyricsChoices")).getByRole("button", { name: "Reset lyrics choices" }));
    await waitFor(() => expect(mock.lyricsChoices.size).toBe(0));
    expect(await within(row("lyricsChoices")).findByText("0 songs")).toBeInTheDocument();
  });
});
