/**
 * Word by word lyrics on the phone, as on the computer: where the lyrics
 * time each word (enhanced LRC, a transcription made on a computer), the
 * current line lights word by word in full screen and on the card under
 * the player; lyrics timed by line still light whole lines.
 */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyrics, mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type LyricLine, type PlayerState, type QueueItem, type Track } from "../lib/types";
import { usePhoneForm } from "../phone/form";
import { cardWindow, cardWindowAt, LyricsCard } from "../phone/LyricsScreen";
import { PhoneShell } from "../phone/PhoneShell";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { clearLyricsCache } from "../shell/lyrics";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";

const mock = backend as MockBackend;
const tracks = (): Track[] => mock.fx.tracks;
const item = (t: Track, i: number): QueueItem => ({ itemId: `q${i}`, track: t, source: "context" });
const byKind = (k: "words" | "lines") => tracks().find((t) => mockLyricsKind(t.id) === k)!;

function play(t: Track, positionMs: number) {
  const s: PlayerState = {
    status: "paused",
    track: t,
    positionMs,
    durationMs: t.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: item(t, 0), userQueue: [], nextUp: [], history: [], context: { kind: "album", id: t.albumId, title: t.albumTitle } },
  };
  act(() => usePlayer.setState({ state: s, local: s, positionMs, durationMs: s.durationMs, remote: null }));
}

/** A line with three or more timed words, and its index. */
function wordyLine(t: Track): { i: number; line: LyricLine } {
  const timed = mockLyrics(t)!.timed!;
  const i = timed.findIndex((l) => (l.words?.length ?? 0) >= 3);
  return { i, line: timed[i] };
}

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  usePhoneUi.setState({ tab: "home", nowPlaying: false, lyrics: false, sheets: [] });
  useRouter.getState().reset({ name: "home" });
  clearLyricsCache();
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
  vi.restoreAllMocks();
});

const litWords = (el: Element) => [...el.querySelectorAll(".lyrics__word.is-sung")].map((w) => w.textContent);
const allWords = (el: Element) => [...el.querySelectorAll(".lyrics__word")].map((w) => w.textContent);

describe("the card's lines", () => {
  it("carry the current line's words when they are timed, and none otherwise", () => {
    const t = byKind("words");
    const l = mockLyrics(t)!;
    const { i, line } = wordyLine(t);
    const w = cardWindowAt(l, i);
    expect(w[0]).toMatchObject({ text: line.text.trim(), sung: true, words: line.words });
    expect(w.slice(1).every((x) => !x.words && !x.sung)).toBe(true);
    // Before the first line nothing is lit and no words are given.
    expect(cardWindowAt(l, -1)[0].words).toBeUndefined();
    // Timed by line: no words.
    const lines = mockLyrics(byKind("lines"))!;
    expect(cardWindow(lines, lines.timed![1].startMs + 5)[0]).toEqual({ text: lines.timed![1].text.trim() || "♪", sung: true });
  });
});

describe("full screen lyrics", () => {
  it("light the current line word by word, and a seek moves the fill at once", async () => {
    const t = byKind("words");
    const { i, line } = wordyLine(t);
    const words = line.words!;
    play(t, words[1].startMs + 5);
    render(<PhoneShell />);
    act(() => {
      phone.openPlayer();
      phone.openLyrics();
    });
    const view = await screen.findByRole("dialog", { name: "Lyrics" });
    const at = (k: number) => view.querySelector<HTMLElement>(`[data-line="${k}"]`)!;
    await waitFor(() => expect(at(i)).toHaveClass("is-worded"));
    expect(at(i)).toHaveAttribute("aria-current", "true");
    expect(allWords(at(i)).join("")).toBe(line.text);
    expect(litWords(at(i))).toEqual(words.slice(0, 2).map((w) => w.text));
    // Lines already sung and the ones to come aren't split into words.
    expect(at(i + 1).querySelector(".lyrics__word")).toBeNull();
    // Tapping the line plays from its start: only its first word is lit.
    vi.spyOn(backend, "playSeek").mockResolvedValue(undefined as never);
    fireEvent.click(at(i));
    await waitFor(() => expect(litWords(at(i))).toEqual([words[0].text]));
  });

  it("lyrics timed by line light whole lines, with no word fill", async () => {
    const t = byKind("lines");
    const timed = mockLyrics(t)!.timed!;
    const k = timed.findIndex((l) => l.text.trim());
    play(t, timed[k].startMs + 5);
    render(<PhoneShell />);
    act(() => {
      phone.openPlayer();
      phone.openLyrics();
    });
    const view = await screen.findByRole("dialog", { name: "Lyrics" });
    const line = () => view.querySelector<HTMLElement>(`[data-line="${k}"]`)!;
    await waitFor(() => expect(line()).toHaveAttribute("aria-current", "true"));
    expect(line()).toHaveClass("is-sung");
    expect(line()).not.toHaveClass("is-worded");
    expect(line().querySelector(".lyrics__word")).toBeNull();
  });

  it("a transcription made on a computer (word timed, synced over) fills word by word on the phone", async () => {
    const t = tracks().find((x) => mockLyricsKind(x.id) === "none")!;
    // What sync puts on the phone: the computer's enhanced LRC, marked as transcribed.
    const text = "[00:01.00]<00:01.00>Heard <00:01.50>every <00:02.00>word\n[00:05.00]<00:05.00>Next <00:05.50>line";
    mock.own.set(t.id, { trackId: t.id, text, prefer: false, updatedAt: 1, auto: "small" });
    play(t, 1_700);
    render(<PhoneShell />);
    act(() => {
      phone.openPlayer();
      phone.openLyrics();
    });
    const view = await screen.findByRole("dialog", { name: "Lyrics" });
    await waitFor(() => expect(view.querySelector('[data-line="0"]')).toHaveClass("is-worded"));
    expect(litWords(view.querySelector('[data-line="0"]')!)).toEqual(["Heard ", "every "]);
    mock.own.delete(t.id);
  });
});

describe("the lyrics card under the player", () => {
  it("fills the current line word by word", async () => {
    const t = byKind("words");
    const { line } = wordyLine(t);
    const words = line.words!;
    play(t, words[2].startMs + 5);
    const { container } = render(<LyricsCard track={t} />);
    await waitFor(() => expect(container.querySelector(".ph-lyrics-card__line.is-worded")).toBeTruthy());
    const first = container.querySelector(".ph-lyrics-card__line")!;
    expect(litWords(first)).toEqual(words.slice(0, 3).map((w) => w.text));
    expect(container.querySelectorAll(".ph-lyrics-card__line .lyrics__word")).toHaveLength(words.length);
  });
});
