/**
 * Automix: its settings, and Mix on a playlist page: the pill on your own
 * playlists and the toggle on others', the sort and shuffle questions, the
 * locked sort, "· Mixed", each song's tempo and the transition between rows
 * with its picker.
 */
import { act, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { PlaylistDialogs } from "../components/PlaylistDialogs";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockBpm } from "../lib/mock/automix";
import type { AutomixView, MixedPlaylists, Playlist } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { PlaylistPage } from "../pages/PlaylistPage";
import { MIX_TIP, SHUFFLE_ASK, SORT_ASK, SORT_LOCKED, sortLockedLabel, useAutomixView } from "../pages/PlaylistMix";
import { SettingsPage } from "../pages/SettingsPage";
import { connectApp, useApp } from "../stores/app";
import { loadMixed, setGapStyle, setMix, useMixed } from "../stores/mixed";
import { player } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const SMART = "503";

let disconnect: () => void;
beforeAll(() => {
  disconnect = connectApp(mock);
});
afterAll(() => disconnect());
afterEach(() => vi.restoreAllMocks());

beforeEach(async () => {
  useRouter.getState().reset();
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null });
  mock.automix.reset();
  useMixed.setState({ mixed: {}, loaded: false });
  mock.resetAudio();
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  const lists = await backend.playlists();
  act(() => useApp.getState().setPlaylists(lists));
});

function page(id: string) {
  return render(
    <>
      <PlaylistPage id={id} />
      <ContextMenuHost />
      <PlaylistDialogs />
    </>,
  );
}

/** A playlist of your own with songs from different albums. */
async function ownPlaylist(): Promise<Playlist> {
  const seen = new Set<string>();
  const ids: string[] = [];
  for (const t of mock.fx.tracks) {
    if (seen.has(t.albumId)) continue;
    seen.add(t.albumId);
    ids.push(t.id);
    if (ids.length === 4) break;
  }
  return backend.playlistCreate("Night drive", ids);
}

const mixedInCore = async (id: string) => !!(await backend.automixMixedGet())[id];

describe("settings", () => {
  it("Automix is on for playlists and mixes, smooth, and each choice saves", async () => {
    render(<SettingsPage />);
    const sw = await screen.findByRole("switch", { name: "Blend songs like a DJ (Automix)" });
    expect(sw).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("combobox", { name: "Where Automix plays" })).toHaveValue("playlistsAndMixes");
    expect(screen.getByRole("combobox", { name: "Transition style" })).toHaveValue("smooth");
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Where Automix plays" }), "allButAlbums");
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Transition style" }), "energetic");
    await userEvent.click(sw);
    await waitFor(async () => {
      const s = await backend.settingsGet();
      expect([s.automix, s.automixWhere, s.automixStyle]).toEqual([false, "allButAlbums", "energetic"]);
    });
    expect(await screen.findAllByText("Turn on Automix to use this.")).toHaveLength(2);
  });
});

describe("mix on your own playlist", () => {
  it("the pill turns mixing on: marked Mixed, sort locked, tempos and transitions shown", async () => {
    const pl = await ownPlaylist();
    page(pl.id);
    const pill = await screen.findByRole("button", { name: "Mix" });
    expect(pill).toHaveAttribute("aria-pressed", "false");
    await userEvent.hover(pill);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(MIX_TIP);
    // Not on your own playlist's menu.
    await userEvent.click(screen.getByRole("button", { name: `More options for ${pl.title}` }));
    expect(screen.queryByRole("menuitem", { name: /Turn on mix/ })).toBeNull();
    await userEvent.keyboard("{Escape}");
    await userEvent.click(pill);
    await waitFor(async () => expect(await mixedInCore(pl.id)).toBe(true));
    expect(pill).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Night drive · Mixed");
    // The sort is locked to custom order.
    const sort = screen.getByRole("button", { name: sortLockedLabel("Custom order") });
    expect(sort).toHaveAttribute("aria-disabled", "true");
    await userEvent.hover(sort);
    await waitFor(() => expect(screen.getAllByRole("tooltip").some((t) => t.textContent === SORT_LOCKED)).toBe(true));
    // Each gap says what it'll do; rows show their tempo.
    const gaps = await screen.findAllByRole("button", { name: /^Transition from .*\. Change$/ });
    expect(gaps).toHaveLength(3);
    const first = (await backend.playlistPage(pl.id)).tracks[0];
    const bpm = mockBpm(first.id);
    if (bpm) expect(screen.getAllByText(`${bpm} BPM`).length).toBeGreaterThan(0);
    // Each transition's one cell spans the list's columns, and Tab reaches its picker.
    const grid = screen.getByRole("grid");
    const cell = gaps[0].closest('[role="gridcell"]');
    expect(cell).toHaveAttribute("aria-colspan", grid.getAttribute("aria-colcount"));
    expect(gaps[0].tabIndex).toBe(0);
    // Pick a transition for the first gap.
    await userEvent.click(gaps[0]);
    await userEvent.click(await screen.findByRole("menuitemradio", { name: /Echo out/ }));
    await waitFor(async () => {
      const m = await backend.automixMixedGet();
      expect(Object.values(m[pl.id].styles)).toEqual(["echo"]);
    });
    expect(await screen.findByRole("button", { name: /: Echo out\. Change$/ })).toBeInTheDocument();
    // Off again.
    await userEvent.click(pill);
    await waitFor(async () => expect(await mixedInCore(pl.id)).toBe(false));
    expect(screen.queryByText(/· Mixed/)).toBeNull();
    expect(screen.queryAllByRole("button", { name: /^Transition from/ })).toHaveLength(0);
  });

  it("turning mix on while sorted asks first, and Continue goes back to custom order", async () => {
    const pl = await ownPlaylist();
    page(pl.id);
    await userEvent.click(await screen.findByRole("button", { name: "Sort songs: Custom order" }));
    await userEvent.click(await screen.findByRole("menuitemradio", { name: "Title" }));
    await userEvent.click(screen.getByRole("button", { name: "Mix" }));
    let dialog = await screen.findByRole("dialog", { name: "Turn on mixing?" });
    expect(dialog).toHaveTextContent(SORT_ASK);
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(await mixedInCore(pl.id)).toBe(false);
    expect(screen.getByRole("button", { name: "Sort songs: Title" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Mix" }));
    dialog = await screen.findByRole("dialog", { name: "Turn on mixing?" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Continue" }));
    await waitFor(async () => expect(await mixedInCore(pl.id)).toBe(true));
    expect(screen.getByRole("button", { name: sortLockedLabel("Custom order") })).toHaveAttribute("aria-disabled", "true");
  });

  it("shuffle on a mixed playlist asks first", async () => {
    const pl = await ownPlaylist();
    await backend.automixMixSet(pl.id, true);
    page(pl.id);
    await screen.findByText(/· Mixed/);
    const cycle = vi.spyOn(player, "cycleContextShuffle").mockImplementation(async () => {});
    const shuffle = () => screen.getByRole("button", { name: /shuffle/i });
    await userEvent.click(shuffle());
    let dialog = await screen.findByRole("dialog", { name: "Shuffle a mixed playlist?" });
    expect(dialog).toHaveTextContent(SHUFFLE_ASK);
    await userEvent.click(within(dialog).getByRole("button", { name: "Don't shuffle" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(cycle).not.toHaveBeenCalled();
    await userEvent.click(shuffle());
    dialog = await screen.findByRole("dialog", { name: "Shuffle a mixed playlist?" });
    await userEvent.click(within(dialog).getByRole("button", { name: "Shuffle anyway" }));
    expect(cycle).toHaveBeenCalledTimes(1);
    expect(cycle.mock.calls[0].slice(0, 2)).toEqual([pl.id, "playlist"]);
  });

  it("an unmixed playlist shuffles without asking", async () => {
    const pl = await ownPlaylist();
    page(pl.id);
    const cycle = vi.spyOn(player, "cycleContextShuffle").mockImplementation(async () => {});
    const shuffle = await screen.findByRole("button", { name: /shuffle/i });
    await userEvent.click(shuffle);
    expect(cycle).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("mix on someone else's playlist", () => {
  it("a toggle in the action row and an entry in the more menu", async () => {
    const { playlist } = await backend.playlistPage(SMART);
    page(SMART);
    const toggle = await screen.findByRole("button", { name: /^Turn on mix for / });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("button", { name: "Mix" })).toBeNull();
    await userEvent.click(toggle);
    await waitFor(async () => expect(await mixedInCore(SMART)).toBe(true));
    expect(await screen.findByRole("button", { name: /^Turn off mix for / })).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: `More options for ${playlist.title}` }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Turn off mix" }));
    await waitFor(async () => expect(await mixedInCore(SMART)).toBe(false));
  });
});

/** A promise settled by hand. */
function later<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const viewOf = (bpm: number): AutomixView => ({ songs: [{ id: "a", bpm, key: null }], gaps: [] });

describe("transitions on a mixed playlist's rows", () => {
  it("only the newest answer is shown, and analysis news settles before reading again", async () => {
    const answers: ReturnType<typeof later<AutomixView>>[] = [];
    const ask = vi.spyOn(backend, "automixView").mockImplementation(() => {
      const a = later<AutomixView>();
      answers.push(a);
      return a.promise;
    });
    const ids = ["a", "b"];
    const { result } = renderHook(() => useAutomixView("pl", true, ids));
    await waitFor(() => expect(ask).toHaveBeenCalledTimes(1));
    // A burst of news about these songs: read once, after it settles.
    act(() => {
      mock.emit("analysis://changed", ["a"]);
      mock.emit("analysis://changed", ["b"]);
      mock.emit("analysis://changed", ["a", "b"]);
      mock.emit("analysis://changed", ["elsewhere"]);
    });
    await waitFor(() => expect(ask).toHaveBeenCalledTimes(2));
    // The first answer comes back last: it's dropped.
    await act(async () => answers[1].resolve(viewOf(120)));
    await act(async () => answers[0].resolve(viewOf(90)));
    expect(result.current?.songs[0].bpm).toBe(120);
    expect(ask).toHaveBeenCalledTimes(2);
  });
});

describe("mixed playlists store", () => {
  it("a change that fails puts back only its own playlist", async () => {
    const answer = later<MixedPlaylists>();
    vi.spyOn(backend, "automixMixSet").mockReturnValue(answer.promise);
    useMixed.setState({ mixed: { b: { styles: {} } }, loaded: true });
    const going = setMix("a", true);
    expect(useMixed.getState().mixed.a).toBeDefined();
    // Something else changes meanwhile.
    act(() => useMixed.setState((s) => ({ mixed: { ...s.mixed, c: { styles: {} } } })));
    answer.reject(new Error("no"));
    await going;
    expect(Object.keys(useMixed.getState().mixed).sort()).toEqual(["b", "c"]);
    expect(useToasts.getState().toasts.some((t) => t.message === "Couldn't turn on mixing")).toBe(true);
  });

  it("a gap's pick keeps the rest of the playlist's entry and goes back alone", async () => {
    vi.spyOn(backend, "automixStyleSet").mockRejectedValue(new Error("no"));
    useMixed.setState({ mixed: { a: { styles: { "x>y": "lift" } }, b: { styles: {} } }, loaded: true });
    const going = setGapStyle("a", "y", "z", "echo");
    expect(useMixed.getState().mixed.a.styles).toEqual({ "x>y": "lift", "y>z": "echo" });
    act(() => useMixed.setState((s) => ({ mixed: { ...s.mixed, b: { styles: { "p>q": "none" } } } })));
    await going;
    expect(useMixed.getState().mixed).toEqual({ a: { styles: { "x>y": "lift" } }, b: { styles: { "p>q": "none" } } });
  });

  it("changes to one playlist reach the core in order", async () => {
    const order: boolean[] = [];
    const first = later<MixedPlaylists>();
    vi.spyOn(backend, "automixMixSet").mockImplementation(async (id, on) => {
      order.push(on);
      if (order.length === 1) return first.promise;
      return on ? { [id]: { styles: {} } } : {};
    });
    useMixed.setState({ mixed: {}, loaded: true });
    const on = setMix("a", true);
    const off = setMix("a", false);
    await waitFor(() => expect(order).toEqual([true]));
    // The second waits for the first.
    await new Promise((r) => setTimeout(r, 20));
    expect(order).toEqual([true]);
    first.resolve({ a: { styles: {} } });
    await on;
    await off;
    expect(order).toEqual([true, false]);
    expect(useMixed.getState().mixed.a).toBeUndefined();
  });

  it("a list that can't be read is logged and counts as loaded", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.spyOn(backend, "automixMixedGet").mockRejectedValue(new Error("db"));
    useMixed.setState({ mixed: { a: { styles: {} } }, loaded: false });
    await loadMixed();
    expect(useMixed.getState()).toMatchObject({ loaded: true, mixed: { a: { styles: {} } } });
    expect(warn).toHaveBeenCalled();
  });

  it("the app reads the list again when another device changes it", async () => {
    useMixed.setState({ mixed: {}, loaded: true });
    await backend.automixMixSet("from-elsewhere", true);
    act(() => mock.emit("syncstore://changed", { namespaces: ["mixedPlaylists"], remote: true }));
    await waitFor(() => expect(useMixed.getState().mixed["from-elsewhere"]).toBeDefined());
  });
});
