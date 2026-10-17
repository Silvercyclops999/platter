/**
 * Automix: its settings, and Mix on a playlist page: the pill on your own
 * playlists and the toggle on others', the sort and shuffle questions, the
 * locked sort, "· Mixed", each song's tempo and the transition between rows
 * with its picker.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { PlaylistDialogs } from "../components/PlaylistDialogs";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockBpm } from "../lib/mock/automix";
import type { Playlist } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { PlaylistPage } from "../pages/PlaylistPage";
import { SHUFFLE_ASK, SORT_ASK, SORT_LOCKED } from "../pages/PlaylistMix";
import { SettingsPage } from "../pages/SettingsPage";
import { connectApp, useApp } from "../stores/app";
import { useMixed } from "../stores/mixed";
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
    const sw = await screen.findByRole("switch", { name: "Automix" });
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
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Turn on mixing");
    // Not on your own playlist's menu.
    await userEvent.click(screen.getByRole("button", { name: `More options for ${pl.title}` }));
    expect(screen.queryByRole("menuitem", { name: /Turn on mix/ })).toBeNull();
    await userEvent.keyboard("{Escape}");
    await userEvent.click(pill);
    await waitFor(async () => expect(await mixedInCore(pl.id)).toBe(true));
    expect(pill).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Night drive · Mixed");
    // The sort is locked to custom order.
    const sort = screen.getByRole("button", { name: "Sort songs: Custom order" });
    expect(sort).toHaveAttribute("aria-disabled", "true");
    await userEvent.hover(sort);
    await waitFor(() => expect(screen.getAllByRole("tooltip").some((t) => t.textContent === SORT_LOCKED)).toBe(true));
    // Each gap says what it'll do; rows show their tempo.
    const gaps = await screen.findAllByRole("button", { name: /^Transition from .*\. Change$/ });
    expect(gaps).toHaveLength(3);
    const first = (await backend.playlistPage(pl.id)).tracks[0];
    const bpm = mockBpm(first.id);
    if (bpm) expect(screen.getAllByText(`${bpm} BPM`).length).toBeGreaterThan(0);
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
    expect(screen.getByRole("button", { name: "Sort songs: Custom order" })).toHaveAttribute("aria-disabled", "true");
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
    await userEvent.click(within(dialog).getByRole("button", { name: "Turn off shuffle" }));
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
