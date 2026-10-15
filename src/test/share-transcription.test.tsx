/**
 * Sharing a transcription to LRCLIB: the three Settings rows, no offer
 * before the lyrics were checked (editor or a play-through), the dialog
 * showing exactly what is sent, nothing sent without confirming, the
 * unreleased rule, and failures that change nothing.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockLyricsKind } from "../lib/mock/details";
import { mockMbCheckOn, mockPlexMissed, mockUnreleased } from "../lib/mock/fixtures";
import { lrcForLrclib } from "../lib/mock/lyricsShare";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { clearLyricsCache } from "../shell/lyrics";
import { LyricsPane } from "../shell/LyricsView";
import { SHARE_LABEL, SHARE_NOTE, SHARED_TOAST, UNRELEASED_NOTE } from "../shell/ShareTranscription";
import { openLyricsEditor, useLyricsEditor } from "../stores/lyricsEditor";
import { useLyricsShare } from "../stores/lyricsShare";
import { usePlayer } from "../stores/player";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const isUnreleased = (t: Track) => {
  const a = mock.fx.albums.find((x) => x.id === t.albumId)!;
  return mockUnreleased(a, true, mockMbCheckOn(DEFAULT_SETTINGS) && mockPlexMissed(mock.fx.albums, a));
};
const quiet = mock.fx.tracks.filter((t) => mockLyricsKind(t.id) === "none" && t.durationMs > 60_000 && !!t.albumTitle);
const released = quiet.find((t) => !isUnreleased(t))!;
const unreleased = quiet.find((t) => isUnreleased(t))!;

const LRC = "[00:01.00]<00:01.00>Hello <00:01.40>there\n[00:06.00]\n[00:08.00]<00:08.00>Second <00:08.40>line";

function playing(track: Track, status: "playing" | "paused" = "paused", positionMs = 0) {
  const state: PlayerState = {
    status,
    track,
    positionMs,
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

function transcribed(t: Track) {
  mock.own.set(t.id, { trackId: t.id, text: LRC, prefer: false, updatedAt: 1, auto: "medium" });
}

async function settings(patch: Partial<typeof DEFAULT_SETTINGS>) {
  await mock.settingsSet(patch);
  await loadSettings();
}

beforeEach(async () => {
  clearLyricsCache();
  mock.own.clear();
  mock.lyricsShare.reset();
  mock.lyricsShare.publishMs = 5;
  await mock.settingsSet({ shareTranscriptions: true, shareTranscriptionsUnreleased: false, syncTranscriptions: true, showAutoLyrics: true });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useLyricsEditor.setState({ track: null, ownIds: new Set(), version: 0 });
  useLyricsShare.setState({ version: 0 });
  useToasts.setState({ toasts: [] });
});

describe("Settings", () => {
  it("syncs transcriptions and offers sharing by default, but not for unreleased songs", async () => {
    expect(DEFAULT_SETTINGS.syncTranscriptions).toBe(true);
    expect(DEFAULT_SETTINGS.shareTranscriptions).toBe(true);
    expect(DEFAULT_SETTINGS.shareTranscriptionsUnreleased).toBe(false);
    await loadSettings();
    render(<SettingsPage />);
    const row = (k: string) => document.querySelector(`[data-setting="${k}"]`) as HTMLElement;
    await waitFor(() => expect(row("syncTranscriptions")).toBeTruthy());
    expect(within(row("syncTranscriptions")).getByRole("switch")).toBeChecked();
    expect(within(row("shareTranscriptions")).getByRole("switch")).toBeChecked();
    const unrel = within(row("shareTranscriptionsUnreleased")).getByRole("switch");
    expect(unrel).not.toBeChecked();
    await userEvent.click(unrel);
    await waitFor(async () => expect((await mock.settingsGet()).shareTranscriptionsUnreleased).toBe(true));
    await userEvent.click(within(row("syncTranscriptions")).getByRole("switch"));
    await waitFor(async () => expect((await mock.settingsGet()).syncTranscriptions).toBe(false));
  });
});

describe("the offer", () => {
  it("isn't there until the lyrics were opened in the editor", async () => {
    transcribed(released);
    playing(released);
    render(<LyricsPane />);
    await screen.findByText("Hello there");
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(screen.queryByRole("button", { name: SHARE_LABEL })).toBeNull();
    act(() => openLyricsEditor(released));
    expect(await screen.findByRole("button", { name: SHARE_LABEL })).toBeInTheDocument();
  });

  it("comes after the song plays through with the lyrics showing", async () => {
    transcribed(released);
    playing(released, "playing", 0);
    render(<LyricsPane />);
    await screen.findByText("Hello there");
    await act(() => new Promise((r) => setTimeout(r, 20)));
    // A seek to the end doesn't count.
    act(() => usePlayer.setState({ positionMs: released.durationMs - 1000 }));
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(mock.lyricsShare.reviewed.has(released.id)).toBe(false);
    act(() => usePlayer.setState({ positionMs: 0 }));
    for (let ms = 1000; ms <= released.durationMs; ms += 1000) act(() => usePlayer.setState({ positionMs: ms }));
    expect(await screen.findByRole("button", { name: SHARE_LABEL })).toBeInTheDocument();
  });

  it("is only for transcriptions, and not when the setting is off", async () => {
    mock.own.set(released.id, { trackId: released.id, text: "[00:01.00]My own words", prefer: false, updatedAt: 1 });
    mock.lyricsShare.reviewed.add(released.id);
    playing(released);
    const { unmount } = render(<LyricsPane />);
    await screen.findByText("My own words");
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(screen.queryByRole("button", { name: SHARE_LABEL })).toBeNull();
    unmount();

    clearLyricsCache();
    transcribed(released);
    await settings({ shareTranscriptions: false });
    render(<LyricsPane />);
    await screen.findByText("Hello there");
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(screen.queryByRole("button", { name: SHARE_LABEL })).toBeNull();
  });
});

describe("the dialog", () => {
  it("shows exactly what is sent and sends only after Share", async () => {
    transcribed(released);
    mock.lyricsShare.reviewed.add(released.id);
    playing(released);
    render(<LyricsPane />);
    await userEvent.click(await screen.findByRole("button", { name: SHARE_LABEL }));
    const dialog = screen.getByRole("dialog", { name: "Share lyrics publicly" });
    expect(within(dialog).getByText(SHARE_NOTE)).toBeInTheDocument();
    expect(within(dialog).getByText(released.title)).toBeInTheDocument();
    expect(within(dialog).getByText((released.trackArtist || released.artistTitle).trim())).toBeInTheDocument();
    expect(within(dialog).getByText(released.albumTitle)).toBeInTheDocument();
    const secs = Math.round(released.durationMs / 1000);
    expect(within(dialog).getByText(`${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`)).toBeInTheDocument();
    const sent = within(dialog).getByLabelText("Synced lyrics to send");
    expect(sent.textContent).toBe(lrcForLrclib(LRC).synced);
    expect(sent.textContent).toBe("[00:01.00]Hello there\n[00:06.00]\n[00:08.00]Second line");
    expect(within(dialog).queryByText(UNRELEASED_NOTE)).toBeNull();

    // Cancel sends nothing.
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(mock.lyricsShare.sent).toEqual([]);

    await userEvent.click(screen.getByRole("button", { name: SHARE_LABEL }));
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Share" }));
    await waitFor(() => expect(mock.lyricsShare.sent).toHaveLength(1));
    expect(mock.lyricsShare.sent[0].payload).toMatchObject({
      trackName: released.title,
      albumName: released.albumTitle,
      duration: secs,
      syncedLyrics: lrcForLrclib(LRC).synced,
      plainLyrics: "Hello there\n\nSecond line",
    });
    // A quiet confirmation, and no second offer.
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain(SHARED_TOAST));
    await waitFor(() => expect(screen.queryByRole("button", { name: SHARE_LABEL })).toBeNull());
  });

  it("says why when sharing fails and changes nothing", async () => {
    transcribed(released);
    mock.lyricsShare.reviewed.add(released.id);
    mock.lyricsShare.failNext = "offline";
    playing(released);
    render(<LyricsPane />);
    await userEvent.click(await screen.findByRole("button", { name: SHARE_LABEL }));
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Share" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't reach LRCLIB. Check your connection and try again. Nothing was shared.");
    expect(mock.lyricsShare.sent).toEqual([]);
    expect(mock.lyricsShare.shared.has(released.id)).toBe(false);
    // Rejected by LRCLIB: its reason, in words.
    mock.lyricsShare.failNext = "rejected";
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Share" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("LRCLIB didn't take these lyrics: Lyrics are too short"));
    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("button", { name: SHARE_LABEL })).toBeInTheDocument();
  });
});

describe("unreleased songs", () => {
  it("aren't offered unless Settings allows it, and then warn about the title", async () => {
    expect(unreleased).toBeTruthy();
    transcribed(unreleased);
    mock.lyricsShare.reviewed.add(unreleased.id);
    playing(unreleased);
    const { unmount } = render(<LyricsPane />);
    await screen.findByText("Hello there");
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(screen.queryByRole("button", { name: SHARE_LABEL })).toBeNull();
    expect((await backend.lyricsShareInfo(unreleased.id)).whyNot).toBe("unreleased");
    unmount();

    clearLyricsCache();
    await settings({ shareTranscriptionsUnreleased: true });
    render(<LyricsPane />);
    await userEvent.click(await screen.findByRole("button", { name: SHARE_LABEL }));
    expect(within(screen.getByRole("dialog")).getByText(UNRELEASED_NOTE)).toBeInTheDocument();
  });
});
