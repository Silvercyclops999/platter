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
import { actionableNotice, clearLyricsCache, lyricsChanged, loadLyrics, loadLyricsInfo, lyricsSeen, setCheckStatus, useLyricsTrust } from "../shell/lyrics";
import {
  CHECK_STUCK_MS,
  CHECK_TEXT,
  CHECK_WAIT_MS,
  checkProgress,
  NOTICE_TEXT,
  offsetText,
  resetCheckClocks,
  tapOffset,
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

describe("a check that takes long", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  /** The server's lyrics, said to be being checked (with a reason, when given). */
  function checking(checkWaiting?: string) {
    const real = mock.lyrics.bind(mock);
    return vi.spyOn(mock, "lyrics").mockImplementation(async (id) => {
      const l = await real(id);
      return l && { ...l, check: "checking", ...(checkWaiting ? { checkWaiting } : {}) };
    });
  }

  /** Let `ms` pass, running what it lets finish (the lyrics fetch, the check timers). */
  const pass = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms));

  it("spins, then says it is waiting, then offers Try again", async () => {
    const spy = checking();
    at(lines, 0);
    render(<LyricsPane />);
    await pass(100);
    expect(screen.getByText(CHECK_TEXT.checking)).toBeInTheDocument();
    await pass(CHECK_WAIT_MS - 1000);
    expect(screen.getByText(CHECK_TEXT.checking)).toBeInTheDocument();
    await pass(1000);
    expect(screen.getByText(CHECK_TEXT.waiting)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    await pass(CHECK_STUCK_MS - CHECK_WAIT_MS);
    expect(screen.getByText(CHECK_TEXT.stuck)).toBeInTheDocument();
    const calls = spy.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await pass(100);
    // Asked again, and the clock starts over.
    expect(spy.mock.calls.length).toBeGreaterThan(calls);
    expect(screen.getByText(CHECK_TEXT.checking)).toBeInTheDocument();
  });

  it("says where the core says the check is, and stops spinning after a while", async () => {
    checking();
    mock.checkStatuses.set(lines.id, { trackId: lines.id, state: "queued" });
    at(lines, 0);
    const { container } = render(<LyricsPane />);
    await pass(100);
    const status = () => screen.getByRole("status");
    expect(status()).toHaveTextContent("Waiting to check against the song…");
    expect(container.querySelector(".lyrics-trust__check .lyrics-trust__spin")).not.toBeNull();
    act(() => setCheckStatus({ trackId: lines.id, state: "listening" }));
    expect(status()).toHaveTextContent(CHECK_TEXT.checking);
    await pass(CHECK_WAIT_MS);
    // Same words, no spinner; the core's own status, not the fallback.
    expect(status()).toHaveTextContent(CHECK_TEXT.checking);
    expect(container.querySelector(".lyrics-trust__check .lyrics-trust__spin")).toBeNull();
    await pass(CHECK_STUCK_MS);
    expect(screen.queryByText(CHECK_TEXT.stuck)).toBeNull();
    act(() => setCheckStatus({ trackId: lines.id, state: "failed", reason: "stuck" }));
    expect(status()).toHaveTextContent("Couldn't check yet");
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("shows the result once the check ends", async () => {
    const spy = checking();
    at(lines, 0);
    render(<LyricsPane />);
    await pass(100);
    await pass(CHECK_WAIT_MS);
    expect(screen.getByText(CHECK_TEXT.waiting)).toBeInTheDocument();
    spy.mockRestore();
    mock.lyricsVerdicts.set(lines.id, { server: "match" });
    act(() => lyricsChanged(lines.id));
    await pass(100);
    expect(screen.getByText(CHECK_TEXT.checked)).toBeInTheDocument();
    await pass(CHECK_STUCK_MS);
    expect(screen.queryByText(CHECK_TEXT.stuck)).toBeNull();
  });
});

describe("the toolbar", () => {
  it("has one timing control showing the offset, and More holds what the row has no room for", async () => {
    at(lines, 0);
    render(<LyricsPane />);
    const timing = await screen.findByRole("group", { name: "Lyrics timing" });
    expect(within(timing).getByText("0 s")).toBeInTheDocument();
    // Hidden at this width (no layout here), so found by its label.
    fireEvent.click(screen.getByLabelText("More lyrics tools"));
    // jsdom lays nothing out, so every action that can move counts as moved.
    expect(useUi.getState().menu?.items.map((i) => i.label)).toEqual(["Tap to sync", "Wrong lyrics?"]);
    act(() => useUi.getState().menu?.items[0].onSelect?.());
    expect(screen.getByRole("button", { name: "Tap to sync" })).toHaveAttribute("aria-pressed", "true");
    act(() => useUi.setState({ menu: null }));
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
    fireEvent.click(await screen.findByRole("button", { name: "Tap to sync" }));
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
    fireEvent.click(await screen.findByRole("button", { name: "Tap to sync" }));
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
    expect(await screen.findByRole("button", { name: "Show lyrics 0.5 seconds later" })).toBeDisabled();
    act(() => setCheckStatus({ trackId: lines.id, state: "failed", reason: "stuck" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Show lyrics 0.5 seconds later" })).toBeEnabled());
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
      expect(await screen.findByText(CHECK_TEXT.notChecked)).toBeInTheDocument();
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
      expect(await screen.findByText(CHECK_TEXT.notChecked)).toBeInTheDocument();
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

describe("check progress", () => {
  it("says what the check is doing in words, never just a spinner", () => {
    const s = (state: string, reason?: string) => checkProgress({ trackId: "1", state, reason } as never);
    expect(s("queued")?.text).toBe("Waiting to check against the song…");
    expect(s("reading")?.text).toBe("Reading the song to check the lyrics…");
    expect(s("listening")).toEqual({ text: "Checking against the song…", spin: true, retry: false });
    expect(s("waiting", "busy")).toEqual({ text: "Waiting: your computer is busy", spin: false, retry: false });
    expect(s("failed", "noModel")).toEqual({ text: "Can't check without the speech model", spin: false, retry: false });
    expect(s("failed", "stuck")).toEqual({ text: "Couldn't check yet", spin: false, retry: true });
    expect(s("done")).toBeNull();
    expect(checkProgress(undefined)).toBeNull();
  });

  it("follows the check's status and offers Try again when it couldn't check", async () => {
    const spy = vi.spyOn(backend, "lyrics").mockResolvedValue({ synced: false, lines: [], notice: "checking" });
    mock.checkStatuses.set(lines.id, { trackId: lines.id, state: "waiting", reason: "busy" });
    at(lines, 0);
    render(<LyricsPane />);
    expect(await screen.findByText(/Waiting: your computer is busy/)).toBeInTheDocument();
    act(() => setCheckStatus({ trackId: lines.id, state: "failed", reason: "stuck" }));
    expect(await screen.findByText(/Couldn't check yet/)).toBeInTheDocument();
    const calls = spy.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(spy.mock.calls.length).toBeGreaterThan(calls));
    expect(screen.queryByText(/Couldn't check yet/)).toBeNull();
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
