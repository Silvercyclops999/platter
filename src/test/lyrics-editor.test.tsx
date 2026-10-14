/**
 * Writing your own lyrics: the LRC rules, the sync rules (stamping, stepping
 * back, nudging, dragging a mark), the editor's flow from the lyrics view,
 * the full-window view and the track menu, save and cancel, export, which
 * source wins, and where the lyrics came from.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { clockTenths, isSynced, linesFromText, lrcTag, lyricsFromText, parseLrc, textFromLines, toLrc, unsyncedCount, type EditLine } from "../lib/lrc";
import type { MockBackend } from "../lib/mock";
import { mockLyricsKind } from "../lib/mock/details";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { LyricsPage } from "../pages/LyricsPage";
import { SettingsPage } from "../pages/SettingsPage";
import { FullWindow } from "../shell/FullWindow";
import { clearLyricsCache, lyricsSourceLabel } from "../shell/lyrics";
import { LyricsEditorHost } from "../shell/LyricsEditor";
import { clearTimes, moveTo, nextStampable, nudge, prevStampable, reconcile, stamp, timelineWindow } from "../shell/lyricsSync";
import { fullWindow } from "../stores/fullWindow";
import { closeLyricsEditor, loadOwnLyricsIds, lyricsEditLabel, useLyricsEditor } from "../stores/lyricsEditor";
import { player, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const byKind = (k: ReturnType<typeof mockLyricsKind>) => mock.fx.tracks.find((t) => mockLyricsKind(t.id) === k && t.durationMs > 120_000)!;
const none = byKind("none");
const fromPlex = byKind("lines");
const fromFile = byKind("plain");

function state(track: Track | null, status: PlayerState["status"] = "paused"): PlayerState {
  return {
    status,
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

function at(track: Track, positionMs: number, status: PlayerState["status"] = "paused") {
  act(() => usePlayer.setState({ state: state(track, status), positionMs, durationMs: track.durationMs }));
}

const toastText = () => useToasts.getState().toasts.map((t) => t.message);
const editor = () => screen.queryByRole("dialog", { name: /lyrics: / });
const L = (text: string, startMs: number | null = null): EditLine => ({ text, startMs });

function page() {
  return render(
    <>
      <LyricsPage />
      <LyricsEditorHost />
    </>,
  );
}

beforeEach(async () => {
  clearLyricsCache();
  mock.lyricsDown = false;
  mock.own.clear();
  mock.exported.length = 0;
  mock.exportCancels = false;
  await mock.settingsSet({ lyricsPreferOwn: false, lyricsShowAdd: true });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useLyricsEditor.setState({ track: null, ownIds: new Set(), version: 0 });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  useUi.setState({ nowPlayingOpen: false, nowPlayingExpanded: false, queueOpen: false, menu: null });
});
afterEach(() => {
  act(() => closeLyricsEditor());
  vi.restoreAllMocks();
});

// ---------------------------------------------------------------- LRC rules

describe("LRC text", () => {
  it("reads plain text as lines, one break between verses", () => {
    expect(linesFromText("\n\n  One\r\nTwo  \n\n\n\nThree\n\n")).toEqual([L("One"), L("Two"), L(""), L("Three")]);
  });

  it("reads LRC back, skipping header tags and word timing", () => {
    const lines = parseLrc("﻿[ar:Someone]\n[ti:Song]\n[00:12.30]First <00:12.80>line\n[01:05.5][02:00.00]Chorus\n\n[00:20]Last");
    expect(lines).toEqual([L("First line", 12_300), L("Chorus", 65_500), L(""), L("Last", 20_000)]);
    expect(textFromLines(lines)).toBe("First line\nChorus\n\nLast");
  });

  it("writes LRC only when every line has a time", () => {
    expect(lrcTag(0)).toBe("[00:00.00]");
    expect(lrcTag(65_432)).toBe("[01:05.43]");
    expect(lrcTag(600_005)).toBe("[10:00.01]");
    const synced = [L("One", 1_000), L(""), L("Two", 61_250)];
    expect(isSynced(synced)).toBe(true);
    expect(toLrc(synced)).toBe("[00:01.00]One\n\n[01:01.25]Two");
    const partial = [L("One", 1_000), L("Two")];
    expect(isSynced(partial)).toBe(false);
    expect(unsyncedCount(partial)).toBe(1);
    expect(unsyncedCount([L("One"), L("Two")])).toBe(0);
    expect(toLrc([L("One"), L(""), L("Two")])).toBe("One\n\nTwo");
    // What's written reads back the same.
    expect(parseLrc(toLrc(synced))).toEqual(synced);
  });

  it("turns saved text into lyrics marked as yours", () => {
    const l = lyricsFromText("[00:02.00]Two\n[00:01.00]One")!;
    expect(l.source).toBe("user");
    expect(l.synced).toBe(true);
    expect(l.timed!.map((t) => t.text)).toEqual(["One", "Two"]);
    expect(lyricsFromText("Plain")).toMatchObject({ synced: false, lines: ["Plain"] });
    expect(lyricsFromText("  \n[ar:x]")).toBeNull();
    expect(clockTenths(62_340)).toBe("1:02.3");
  });
});

// ---------------------------------------------------------------- sync rules

describe("sync rules", () => {
  const base = [L("A"), L(""), L("B"), L("C")];

  it("steps over breaks between verses", () => {
    expect(nextStampable(base, 0)).toBe(2);
    expect(nextStampable(base, 3)).toBe(-1);
    expect(prevStampable(base, 2)).toBe(0);
    expect(prevStampable(base, 0)).toBe(-1);
    expect(prevStampable(base, base.length)).toBe(3);
  });

  it("stamps a line and drops later times that would be out of order", () => {
    let ls = stamp(base, 0, 1_000.4);
    ls = stamp(ls, 2, 5_000);
    ls = stamp(ls, 3, 9_000);
    expect(ls.map((l) => l.startMs)).toEqual([1_000, null, 5_000, 9_000]);
    // Back to B and stamped later than C: C has to be stamped again.
    ls = stamp(ls, 2, 9_500);
    expect(ls.map((l) => l.startMs)).toEqual([1_000, null, 9_500, null]);
    expect(stamp(ls, 1, 2_000)).toBe(ls);
  });

  it("nudges by 0.1 s, kept between its neighbours and inside the song", () => {
    const ls = [L("A", 1_000), L("B", 1_050), L("C", 3_000)];
    expect(nudge(ls, 1, 100)[1].startMs).toBe(1_150);
    expect(nudge(ls, 1, -100)[1].startMs).toBe(1_000);
    expect(nudge(ls, 0, -2_000)[0].startMs).toBe(0);
    expect(nudge(ls, 2, 100, 3_050)[2].startMs).toBe(3_050);
    expect(nudge([L("A")], 0, 100)).toEqual([L("A")]);
  });

  it("moves a dragged mark, clamped to its neighbours", () => {
    const ls = [L("A", 1_000), L("B", 2_000), L("C", 3_000)];
    expect(moveTo(ls, 1, 2_500)[1].startMs).toBe(2_500);
    expect(moveTo(ls, 1, 9_000)[1].startMs).toBe(3_000);
    expect(moveTo(ls, 1, 0)[1].startMs).toBe(1_000);
    expect(clearTimes(ls).every((l) => l.startMs === null)).toBe(true);
  });

  it("keeps times for lines whose words didn't change", () => {
    const old = [L("A", 1_000), L("B", 2_000), L("C", 3_000)];
    expect(reconcile(old, [L("A"), L(""), L("B changed"), L("C")])).toEqual([L("A", 1_000), L(""), L("B changed"), L("C", 3_000)]);
  });
});

// ---------------------------------------------------------------- the editor

describe("lyrics editor", () => {
  it("opens from the empty lyrics view, saves plain lyrics, and the view shows them as yours", async () => {
    const user = userEvent.setup();
    at(none, 0);
    page();
    await screen.findByText("No lyrics for this song");
    await user.click(screen.getByRole("button", { name: "Add lyrics" }));
    const dlg = await screen.findByRole("dialog", { name: `Add lyrics: ${none.title}` });
    const box = within(dlg).getByRole("textbox", { name: "Lyrics" });
    await waitFor(() => expect(box).toHaveFocus());
    const save = within(dlg).getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();
    await user.type(box, "First line{Enter}Second line");
    expect(within(dlg).getByText("2 lines.")).toBeInTheDocument();
    await user.click(save);
    await waitFor(() => expect(editor()).toBeNull());
    expect(mock.own.get(none.id)).toMatchObject({ text: "First line\nSecond line", prefer: false });
    expect(toastText()).toContain("Lyrics saved");
    expect(lyricsEditLabel(none.id)).toBe("Edit lyrics");
    expect(await screen.findByText("Second line")).toBeInTheDocument();
    expect(screen.getByText("Your lyrics")).toBeInTheDocument();
    // Edit opens them again.
    await user.click(screen.getByRole("button", { name: "Edit" }));
    const again = await screen.findByRole("dialog", { name: `Edit lyrics: ${none.title}` });
    await waitFor(() => expect(within(again).getByRole("textbox", { name: "Lyrics" })).toHaveValue("First line\nSecond line"));
  });

  it("hides Add lyrics in the empty view when its setting is off", async () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, lyricsShowAdd: false } });
    at(none, 0);
    page();
    await screen.findByText("No lyrics for this song");
    expect(screen.queryByRole("button", { name: "Add lyrics" })).toBeNull();
  });

  it("Cancel closes at once with nothing changed, and asks first when there is", async () => {
    const user = userEvent.setup();
    render(<LyricsEditorHost />);
    act(() => useLyricsEditor.setState({ track: none }));
    const dlg = await screen.findByRole("dialog", { name: /Add lyrics/ });
    await user.click(within(dlg).getByRole("button", { name: "Cancel" }));
    expect(editor()).toBeNull();

    act(() => useLyricsEditor.setState({ track: none }));
    const box = within(await screen.findByRole("dialog", { name: /Add lyrics/ })).getByRole("textbox", { name: "Lyrics" });
    await user.type(box, "Words");
    await user.keyboard("{Escape}");
    expect(screen.getByRole("alert")).toHaveTextContent("Discard your changes?");
    await user.click(screen.getByRole("button", { name: "Keep editing" }));
    expect(box).toHaveValue("Words");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByRole("button", { name: "Discard" }));
    expect(editor()).toBeNull();
    expect(mock.own.size).toBe(0);
  });

  it("syncs: the song plays from the start, Space stamps, Backspace steps back and rewinds, Alt+arrows nudge, Enter plays from a line", async () => {
    const user = userEvent.setup();
    const playTracks = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    const seek = vi.spyOn(player, "seek").mockResolvedValue(undefined);
    const toggle = vi.spyOn(player, "toggle").mockResolvedValue(undefined);
    at(fromPlex, 30_000);
    render(<LyricsEditorHost />);
    act(() => useLyricsEditor.setState({ track: none }));
    const dlg = await screen.findByRole("dialog", { name: /Add lyrics/ });
    await user.type(within(dlg).getByRole("textbox", { name: "Lyrics" }), "One{Enter}{Enter}Two{Enter}Three");
    await user.click(within(dlg).getByRole("button", { name: "Sync to the song" }));
    // Another song was playing: this one starts from the top.
    expect(playTracks).toHaveBeenCalledWith([none.id], none.id);
    at(none, 0);
    const list = within(dlg).getByRole("listbox", { name: "Lines" });
    expect(list).toHaveFocus();
    const opt = (name: string) => within(list).getByRole("option", { name: new RegExp(name) });
    expect(opt("One")).toHaveClass("is-next");

    at(none, 4_000);
    fireEvent.keyDown(list, { key: " " });
    expect(opt("One")).toHaveTextContent("0:04.0");
    expect(opt("Two")).toHaveClass("is-next");
    at(none, 9_000);
    fireEvent.keyDown(list, { key: " " });
    expect(opt("Two")).toHaveTextContent("0:09.0");
    // Partly synced can't be saved yet.
    expect(within(dlg).getByRole("button", { name: "Save" })).toBeDisabled();
    expect(within(dlg).getByText("1 line still need a time")).toBeInTheDocument();

    // Backspace: back to Two, and the song rewinds 3 s.
    at(none, 11_000);
    fireEvent.keyDown(list, { key: "Backspace" });
    expect(seek).toHaveBeenLastCalledWith(8_000);
    expect(opt("Two")).toHaveClass("is-next");
    expect(opt("Two")).toHaveAttribute("aria-selected", "true");
    at(none, 9_500);
    fireEvent.keyDown(list, { key: " " });
    expect(opt("Two")).toHaveTextContent("0:09.5");
    at(none, 14_000);
    fireEvent.keyDown(list, { key: " " });
    expect(opt("Three")).toHaveTextContent("0:14.0");

    // Alt+Left nudges the selected line (Three) back 0.1 s.
    fireEvent.keyDown(list, { key: "ArrowLeft", altKey: true });
    fireEvent.keyDown(list, { key: "ArrowLeft", altKey: true });
    expect(opt("Three")).toHaveTextContent("0:13.8");
    // Up selects Two; Alt+Right nudges it.
    fireEvent.keyDown(list, { key: "ArrowUp" });
    fireEvent.keyDown(list, { key: "ArrowRight", altKey: true });
    expect(opt("Two")).toHaveTextContent("0:09.6");
    // Enter plays from it.
    fireEvent.keyDown(list, { key: "Enter" });
    expect(seek).toHaveBeenLastCalledWith(9_600);
    expect(toggle).toHaveBeenCalled();

    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(editor()).toBeNull());
    expect(mock.own.get(none.id)!.text).toBe("[00:04.00]One\n\n[00:09.60]Two\n[00:13.80]Three");
  });

  it("drags a mark on the timeline, kept between its neighbours", async () => {
    vi.spyOn(player, "seek").mockResolvedValue(undefined);
    mock.own.set(none.id, { trackId: none.id, text: "[00:10.00]One\n[00:20.00]Two\n[00:30.00]Three", prefer: false, updatedAt: 1 });
    const user = userEvent.setup();
    at(none, 0);
    render(<LyricsEditorHost />);
    act(() => useLyricsEditor.setState({ track: none }));
    const dlg = await screen.findByRole("dialog", { name: /Edit lyrics/ });
    await waitFor(() => expect(within(dlg).getByRole("textbox", { name: "Lyrics" })).toHaveValue("One\nTwo\nThree"));
    await user.click(within(dlg).getByRole("button", { name: "Sync to the song" }));
    // The timeline shows 20 s around the playhead.
    at(none, 20_000);
    const bar = screen.getByTestId("lyrics-timeline");
    expect(bar.dataset.span).toBe("20000");
    await waitFor(() => expect(bar.dataset.start).toBe(String(20_000 - Math.round(20_000 / 3))));
    // One, at 0:10, is outside it.
    expect(screen.queryByRole("slider", { name: 'Start of "One"' })).toBeNull();
    vi.spyOn(bar, "getBoundingClientRect").mockReturnValue({ left: 0, width: 1000, top: 0, height: 36, right: 1000, bottom: 36, x: 0, y: 0, toJSON: () => ({}) });
    const mark = screen.getByRole("slider", { name: 'Start of "Two"' });
    const px = (ms: number) => ((ms - Number(bar.dataset.start)) / Number(bar.dataset.span)) * 1000;
    fireEvent.pointerDown(mark, { clientX: px(20_000), pointerId: 1 });
    fireEvent.pointerMove(mark, { clientX: px(25_000), pointerId: 1 });
    fireEvent.pointerUp(mark, { clientX: px(25_000), pointerId: 1 });
    expect(mark).toHaveAttribute("aria-valuenow", "25000");
    expect(within(dlg).getByRole("option", { name: /Two/ })).toHaveAttribute("aria-selected", "true");
    expect(within(dlg).getByRole("option", { name: /Two/ })).toHaveTextContent("0:25.0");
    // Past Three: stops at Three's start.
    fireEvent.pointerDown(mark, { clientX: px(25_000), pointerId: 1 });
    fireEvent.pointerMove(mark, { clientX: px(60_000), pointerId: 1 });
    fireEvent.pointerUp(mark, { pointerId: 1 });
    expect(mark).toHaveAttribute("aria-valuenow", "30000");
  });

  it("the timeline window follows the line and stays inside the song", () => {
    expect(timelineWindow(30_000, 200_000)).toEqual({ start: 23_333, span: 20_000 });
    expect(timelineWindow(1_000, 200_000)).toEqual({ start: 0, span: 20_000 });
    expect(timelineWindow(199_000, 200_000)).toEqual({ start: 180_000, span: 20_000 });
    expect(timelineWindow(5_000, 12_000)).toEqual({ start: 0, span: 12_000 });
  });

  it("exports an .lrc through the save dialog, and says nothing when cancelled", async () => {
    const user = userEvent.setup();
    mock.own.set(none.id, { trackId: none.id, text: "[00:10.00]One", prefer: false, updatedAt: 1 });
    render(<LyricsEditorHost />);
    act(() => useLyricsEditor.setState({ track: none }));
    const dlg = await screen.findByRole("dialog", { name: /Edit lyrics/ });
    await waitFor(() => expect(within(dlg).getByRole("textbox", { name: "Lyrics" })).toHaveValue("One"));
    await user.click(within(dlg).getByRole("button", { name: "Export .lrc" }));
    await waitFor(() => expect(mock.exported).toHaveLength(1));
    expect(mock.exported[0].text).toBe("[00:10.00]One");
    expect(mock.exported[0].fileName).toContain(none.title.replace(/[<>:"/\\|?*]/g, "_"));
    await waitFor(() => expect(toastText().some((t) => t.startsWith("Saved ") && t.endsWith(".lrc"))).toBe(true));
    mock.exportCancels = true;
    useToasts.setState({ toasts: [] });
    await user.click(within(dlg).getByRole("button", { name: "Export .lrc" }));
    await act(async () => {});
    expect(toastText()).toEqual([]);
  });

  it("a failed save or export keeps the editor open with a toast", async () => {
    const user = userEvent.setup();
    vi.spyOn(backend, "ownLyricsSave").mockRejectedValue({ kind: "internal", message: "db: disk I/O error" });
    vi.spyOn(backend, "lyricsExport").mockRejectedValue({ kind: "io", message: "Access is denied. (os error 5)" });
    render(<LyricsEditorHost />);
    act(() => useLyricsEditor.setState({ track: none }));
    const dlg = await screen.findByRole("dialog", { name: /Add lyrics/ });
    const box = within(dlg).getByRole("textbox", { name: "Lyrics" });
    await user.type(box, "Keep me");
    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(toastText()).toContain("Couldn't save the lyrics"));
    expect(editor()).not.toBeNull();
    expect(box).toHaveValue("Keep me");
    expect(within(dlg).getByRole("button", { name: "Save" })).toBeEnabled();
    await user.click(within(dlg).getByRole("button", { name: "Export .lrc" }));
    await waitFor(() => expect(toastText()).toContain("Couldn't export the lyrics"));
    expect(editor()).not.toBeNull();
  });

  it("removes your lyrics after asking", async () => {
    const user = userEvent.setup();
    mock.own.set(none.id, { trackId: none.id, text: "Mine", prefer: false, updatedAt: 1 });
    await loadOwnLyricsIds();
    render(<LyricsEditorHost />);
    act(() => useLyricsEditor.setState({ track: none }));
    const dlg = await screen.findByRole("dialog", { name: /Edit lyrics/ });
    await user.click(await within(dlg).findByRole("button", { name: "Remove" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Remove your lyrics for this song?");
    await user.click(within(screen.getByRole("alert")).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(editor()).toBeNull());
    expect(mock.own.has(none.id)).toBe(false);
    expect(lyricsEditLabel(none.id)).toBe("Add lyrics");
  });

  it("opens from the track menu with Add or Edit", async () => {
    const item = () => trackMenuItems(none).find((i) => /lyrics/.test(i.label))!;
    expect(item().label).toBe("Add lyrics");
    act(() => item().onSelect!());
    expect(useLyricsEditor.getState().track?.id).toBe(none.id);
    mock.own.set(none.id, { trackId: none.id, text: "Mine", prefer: false, updatedAt: 1 });
    await loadOwnLyricsIds();
    expect(item().label).toBe("Edit lyrics");
  });

  it("opens from the full-window view", async () => {
    const user = userEvent.setup();
    at(none, 0);
    render(
      <>
        <FullWindow />
        <LyricsEditorHost />
      </>,
    );
    act(() => void fullWindow.open());
    await user.click(await screen.findByRole("button", { name: "Add lyrics" }));
    expect(await screen.findByRole("dialog", { name: `Add lyrics: ${none.title}` })).toBeInTheDocument();
    // Space in the editor isn't play/pause for the full-window view.
    const toggle = vi.spyOn(player, "toggle").mockResolvedValue(undefined);
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Lyrics" }), { key: " " });
    expect(toggle).not.toHaveBeenCalled();
    act(() => void fullWindow.close());
  });
});

// ---------------------------------------------------------------- sources

describe("which lyrics show", () => {
  it("Plex's first, then yours; yours first for the song or with the setting", async () => {
    mock.own.set(fromPlex.id, { trackId: fromPlex.id, text: "[00:01.00]Mine", prefer: false, updatedAt: 1 });
    expect((await backend.lyrics(fromPlex.id))!.source).toBe("plex");
    mock.own.set(none.id, { trackId: none.id, text: "Mine", prefer: false, updatedAt: 1 });
    expect((await backend.lyrics(none.id))!.source).toBe("user");
    mock.own.set(fromPlex.id, { trackId: fromPlex.id, text: "[00:01.00]Mine", prefer: true, updatedAt: 1 });
    expect((await backend.lyrics(fromPlex.id))!.source).toBe("user");
    mock.own.set(fromPlex.id, { trackId: fromPlex.id, text: "[00:01.00]Mine", prefer: false, updatedAt: 1 });
    await mock.settingsSet({ lyricsPreferOwn: true });
    expect((await backend.lyrics(fromPlex.id))!.source).toBe("user");
  });

  it("the per-song switch saves with the lyrics", async () => {
    const user = userEvent.setup();
    render(<LyricsEditorHost />);
    act(() => useLyricsEditor.setState({ track: fromPlex }));
    const dlg = await screen.findByRole("dialog", { name: /Add lyrics/ });
    await user.type(within(dlg).getByRole("textbox", { name: "Lyrics" }), "Mine");
    await user.click(within(dlg).getByRole("switch", { name: "Show these instead of Plex's lyrics" }));
    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mock.own.get(fromPlex.id)?.prefer).toBe(true));
    expect((await backend.lyrics(fromPlex.id))!.source).toBe("user");
  });

  it("says where lyrics came from, including a file next to the song", async () => {
    expect(lyricsSourceLabel({ synced: false, lines: ["x"], source: "file" })).toBe("From the lyrics file next to this song");
    expect(lyricsSourceLabel({ synced: false, lines: ["x"], source: "user" })).toBe("Your lyrics");
    expect(lyricsSourceLabel({ synced: false, lines: ["x"], source: "plex" })).toBe("Lyrics from Plex");
    expect(lyricsSourceLabel({ synced: false, lines: ["x"], source: "plex", credit: "Lyrics licensed" })).toBeNull();
    expect(lyricsSourceLabel({ synced: false, lines: ["x"], source: "lrclib", credit: "Lyrics from LRCLIB" })).toBeNull();
    at(fromFile, 0);
    page();
    expect(await screen.findByText("From the lyrics file next to this song")).toBeInTheDocument();
  });

  it("Settings has both lyrics options, in the Appearance section", async () => {
    const user = userEvent.setup();
    await loadSettings();
    render(<SettingsPage />);
    const prefer = await screen.findByRole("switch", { name: "Prefer my own lyrics over Plex's" });
    const add = screen.getByRole("switch", { name: "Show Add lyrics on songs without lyrics" });
    expect(prefer).toHaveAttribute("aria-checked", "false");
    expect(add).toHaveAttribute("aria-checked", "true");
    await user.click(prefer);
    await waitFor(() => expect(useSettings.getState().settings?.lyricsPreferOwn).toBe(true));
    await user.click(add);
    await waitFor(() => expect(useSettings.getState().settings?.lyricsShowAdd).toBe(false));
  });
});
