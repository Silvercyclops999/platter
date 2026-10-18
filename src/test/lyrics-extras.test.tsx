/**
 * Pronunciation and translation lines under lyrics, and their settings.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyrics, mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { clearLyricsCache } from "../shell/lyrics";
import { LyricsPane } from "../shell/LyricsView";
import { clearTranslations } from "../shell/lyricExtras";
import { usePlayer } from "../stores/player";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const byKind = (k: ReturnType<typeof mockLyricsKind>) => mock.fx.tracks.find((t) => mockLyricsKind(t.id) === k && t.durationMs > 120_000)!;
const words = byKind("words");
const lines = byKind("lines");
const plain = byKind("plain");

function at(track: Track, positionMs: number) {
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
  act(() => usePlayer.setState({ state, positionMs, durationMs: track.durationMs }));
}

async function withSettings(more: Partial<typeof DEFAULT_SETTINGS>) {
  await backend.settingsSet({ ...DEFAULT_SETTINGS, ...more });
  await loadSettings();
}

const view = () => screen.getByLabelText("Lyrics", { selector: ".lyrics__scroll" });

beforeEach(async () => {
  clearLyricsCache();
  clearTranslations();
  mock.translateFails = false;
  useToasts.setState({ toasts: [] });
  await withSettings({});
});
afterEach(() => vi.restoreAllMocks());

describe("pronunciation", () => {
  it("off by default: nothing is romanised", async () => {
    const pron = vi.spyOn(backend, "lyricsPronounce");
    at(lines, 40_000);
    render(<LyricsPane />);
    await screen.findByRole("button", { current: true });
    expect(pron).not.toHaveBeenCalled();
    expect(document.querySelector(".lyrics__extra")).toBeNull();
  });

  it("puts a romanised line under synced lines, word timing kept", async () => {
    await withSettings({ showPronunciation: true });
    const pron = vi.spyOn(backend, "lyricsPronounce").mockImplementation(async (ls) => ls.map((l, i) => (i % 2 === 0 && l ? `roman ${i}` : null)));
    const t = mockLyrics(words)!.timed!;
    const line = t.findIndex((l, i) => i > 0 && i % 2 === 0 && (l.words?.length ?? 0) > 3);
    at(words, t[line].words![1].startMs + 10);
    render(<LyricsPane />);
    const current = await screen.findByRole("button", { current: true });
    await waitFor(() => expect(within(current).getByText(`roman ${line}`)).toHaveClass("lyrics__extra--pron"));
    expect(pron).toHaveBeenCalledWith(t.map((l) => l.text));
    expect(current.querySelectorAll(".lyrics__word.is-sung")).toHaveLength(2);
    // The line's name is the lyric, then its pronunciation.
    expect(current).toHaveAccessibleName(`${t[line].text}, roman ${line}`);
  });

  it("works on plain lyrics too", async () => {
    await withSettings({ showPronunciation: true });
    vi.spyOn(backend, "lyricsPronounce").mockImplementation(async (ls) => ls.map((l) => (l ? `say ${l}` : null)));
    at(plain, 40_000);
    render(<LyricsPane />);
    await screen.findByLabelText("Lyrics", { selector: ".lyrics__scroll" });
    await waitFor(() => expect(view().querySelectorAll("p.lyrics__line .lyrics__extra--pron").length).toBeGreaterThan(5));
  });
});

describe("translation", () => {
  it("no service set: no Translate button and nothing sent", async () => {
    const tr = vi.spyOn(backend, "lyricsTranslate");
    at(plain, 40_000);
    render(<LyricsPane />);
    await screen.findByLabelText("Lyrics", { selector: ".lyrics__scroll" });
    expect(screen.queryByRole("button", { name: "Translate" })).toBeNull();
    expect(tr).not.toHaveBeenCalled();
  });

  it("Translate shows the translation under each line and hides it again", async () => {
    const user = userEvent.setup();
    await withSettings({ translationService: "https://tr.example.com", translateTo: "fr" });
    at(plain, 40_000);
    render(<LyricsPane />);
    await user.click(await screen.findByRole("button", { name: "Translate" }));
    await waitFor(() => expect(view().querySelectorAll(".lyrics__extra--trans").length).toBeGreaterThan(5));
    expect(view().querySelector(".lyrics__extra--trans")!.textContent).toMatch(/^\[fr\] /);
    await user.click(screen.getByRole("button", { name: "Hide translation" }));
    expect(view().querySelector(".lyrics__extra--trans")).toBeNull();
  });

  it("keeps a song's translation: Hide then Translate, or coming back, doesn't ask again", async () => {
    const user = userEvent.setup();
    await withSettings({ translationService: "https://tr.example.com", translateTo: "fr" });
    const tr = vi.spyOn(backend, "lyricsTranslate");
    at(plain, 40_000);
    const { unmount } = render(<LyricsPane />);
    await user.click(await screen.findByRole("button", { name: "Translate" }));
    await waitFor(() => expect(view().querySelector(".lyrics__extra--trans")).not.toBeNull());
    unmount();
    render(<LyricsPane />);
    await user.click(await screen.findByRole("button", { name: "Translate" }));
    await waitFor(() => expect(view().querySelector(".lyrics__extra--trans")).not.toBeNull());
    expect(tr).toHaveBeenCalledTimes(1);
    // Another language is another translation.
    await withSettings({ translationService: "https://tr.example.com", translateTo: "de" });
    await user.click(await screen.findByRole("button", { name: "Translate" }));
    await waitFor(() => expect(view().querySelector(".lyrics__extra--trans")!.textContent).toMatch(/^\[de\] /));
    expect(tr).toHaveBeenCalledTimes(2);
  });

  it("a service that can't be reached says so and leaves the lyrics as they are", async () => {
    const user = userEvent.setup();
    await withSettings({ translationService: "https://tr.example.com" });
    mock.translateFails = true;
    at(plain, 40_000);
    render(<LyricsPane />);
    await user.click(await screen.findByRole("button", { name: "Translate" }));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Couldn't reach the translation service."));
    expect(document.querySelector(".lyrics__extra--trans")).toBeNull();
    expect(screen.getByRole("button", { name: "Translate" })).toHaveAttribute("aria-pressed", "false");
  });
});

describe("settings", () => {
  it("Show pronunciation is off and no translation service is set", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sw = await screen.findByRole("switch", { name: "Show pronunciation" });
    expect(sw).not.toBeChecked();
    await user.click(sw);
    await waitFor(() => expect(useSettings.getState().settings?.showPronunciation).toBe(true));
    const field = screen.getByRole("textbox", { name: "Translation service address" });
    expect(field).toHaveValue("");
    await user.type(field, "https://tr.example.com{Enter}");
    await waitFor(() => expect(useSettings.getState().settings?.translationService).toBe("https://tr.example.com"));
    expect(screen.getByText("Translate lyrics into")).toBeInTheDocument();
  });
});
