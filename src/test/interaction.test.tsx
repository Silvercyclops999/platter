import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { Slider } from "../components/Slider";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { PlayerState } from "../lib/types";
import { navigateWithArt, ART_TRANSITION } from "../lib/viewTransition";
import { AlbumCard } from "../pages/cards";
import { pickFitSize } from "../pages/detail";
import { MainView, restoreScroll, stickyThreshold } from "../shell/MainView";
import { PlayerBar } from "../shell/PlayerBar";
import { QueuePanel, moveIndex } from "../shell/QueuePanel";
import { Sidebar } from "../shell/Sidebar";
import { useShortcuts } from "../shell/useShortcuts";
import { libraryEntries, useLibraryView, type LibEntry } from "../stores/library";
import { usePins } from "../stores/pins";
import { resetVolumeNudge, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { useUi } from "../stores/ui";
import { useApp } from "../stores/app";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

async function setPlayer(patch: Partial<PlayerState>) {
  const base = await mock.playerState();
  act(() => usePlayer.getState().set({ ...base, ...patch }));
}

afterEach(() => {
  vi.restoreAllMocks();
  act(() => useUi.setState({ menu: null, pageBar: null, paletteOpen: false }));
  resetVolumeNudge();
});

// ---------------------------------------------------------------- track rows

function List3({ onPlay = vi.fn() }: { onPlay?: () => void }) {
  const [a, b, c] = mock.fx.tracks;
  return (
    <>
      <button type="button">outside</button>
      <TrackList label="Tracks">
        <TrackRow track={a} index={1} onPlay={onPlay} />
        <TrackRow track={b} index={2} onPlay={vi.fn()} />
        <TrackRow track={c} index={3} onPlay={vi.fn()} />
      </TrackList>
      <ContextMenuHost />
    </>
  );
}

const rows = () => screen.getAllByRole("listitem");
const selectedTitles = () => rows().filter((r) => r.dataset.selected === "true").map((r) => r.dataset.trackId);

describe("track row selection", () => {
  it("single click selects one row; double-click plays", async () => {
    const user = userEvent.setup();
    const onPlay = vi.fn();
    render(<List3 onPlay={onPlay} />);
    await user.click(rows()[1]);
    expect(selectedTitles()).toEqual([mock.fx.tracks[1].id]);
    await user.click(rows()[0]);
    expect(selectedTitles()).toEqual([mock.fx.tracks[0].id]);
    expect(onPlay).not.toHaveBeenCalled();
    await user.dblClick(rows()[0]);
    expect(onPlay).toHaveBeenCalledTimes(1);
  });

  it("Ctrl+click toggles, Shift+click selects a range, clicking outside clears", async () => {
    const user = userEvent.setup();
    render(<List3 />);
    const [a, b, c] = mock.fx.tracks;
    await user.click(rows()[0]);
    await user.keyboard("{Control>}");
    await user.click(rows()[2]);
    await user.keyboard("{/Control}");
    expect(selectedTitles()).toEqual([a.id, c.id]);
    await user.click(rows()[0]);
    await user.keyboard("{Shift>}");
    await user.click(rows()[2]);
    await user.keyboard("{/Shift}");
    expect(selectedTitles()).toEqual([a.id, b.id, c.id]);
    await user.click(screen.getByRole("button", { name: "outside" }));
    expect(selectedTitles()).toEqual([]);
  });

  it("right-click on a multi-selection opens one menu that acts on every selected song", async () => {
    const user = userEvent.setup();
    const spy = vi.spyOn(backend, "queueAdd");
    render(<List3 />);
    const [a, , c] = mock.fx.tracks;
    await user.click(rows()[0]);
    await user.keyboard("{Control>}");
    await user.click(rows()[2]);
    await user.keyboard("{/Control}");
    fireEvent.contextMenu(rows()[2], { clientX: 20, clientY: 20 });
    const menu = await screen.findByRole("menu", { name: "Options for 2 songs" });
    expect(within(menu).getAllByRole("menuitem").map((i) => i.textContent)).toEqual(["Add to queue"]);
    await user.click(within(menu).getByRole("menuitem", { name: "Add to queue" }));
    expect(spy).toHaveBeenCalledWith([a.id, c.id], "end");
    // The selection survives the menu.
    expect(selectedTitles()).toEqual([a.id, c.id]);
  });

  it("right-click on an unselected row selects just that row and shows its own menu", async () => {
    const user = userEvent.setup();
    render(<List3 />);
    await user.click(rows()[0]);
    await user.keyboard("{Control>}");
    await user.click(rows()[1]);
    await user.keyboard("{/Control}");
    fireEvent.contextMenu(rows()[2], { clientX: 20, clientY: 20 });
    await screen.findByRole("menu", { name: `Options for ${mock.fx.tracks[2].title}` });
    expect(selectedTitles()).toEqual([mock.fx.tracks[2].id]);
  });

  it("keyboard: arrows move the selection, Shift+arrow extends, Ctrl+A selects all, Escape clears, Enter plays", async () => {
    const user = userEvent.setup();
    const onPlay = vi.fn();
    render(<List3 onPlay={onPlay} />);
    const [a, b, c] = mock.fx.tracks;
    await user.click(rows()[0]);
    await user.keyboard("{Shift>}{ArrowDown}{/Shift}");
    expect(selectedTitles()).toEqual([a.id, b.id]);
    await user.keyboard("{ArrowDown}");
    expect(selectedTitles()).toEqual([c.id]);
    expect(rows()[2]).toHaveFocus();
    await user.keyboard("{Control>}a{/Control}");
    expect(selectedTitles()).toEqual([a.id, b.id, c.id]);
    await user.keyboard("{Escape}");
    expect(selectedTitles()).toEqual([]);
    rows()[0].focus();
    await user.keyboard("{Enter}");
    expect(onPlay).toHaveBeenCalledTimes(1);
  });

  it("the playing row shows bars and its hover button pauses instead of restarting", async () => {
    const user = userEvent.setup();
    const toggle = vi.spyOn(backend, "playToggle");
    const onPlay = vi.fn();
    const t = mock.fx.tracks[0];
    render(
      <TrackList label="Tracks">
        <TrackRow track={t} index={1} active playing onPlay={onPlay} />
      </TrackList>,
    );
    const row = rows()[0];
    expect(row).toHaveClass("is-active");
    expect(row.querySelector(".eq")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: `Pause ${t.title}` }));
    expect(toggle).toHaveBeenCalled();
    expect(onPlay).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------- cards

describe("cards", () => {
  beforeEach(() => useRouter.getState().reset());

  it("the play button plays without opening the album", async () => {
    const user = userEvent.setup();
    const spy = vi.spyOn(backend, "playContext");
    const album = mock.fx.albums[0];
    render(<AlbumCard album={album} />);
    await user.click(screen.getByRole("button", { name: `Play ${album.title}` }));
    expect(spy).toHaveBeenCalledWith({ context: { kind: "album", id: album.id }, startTrackId: undefined, shuffle: undefined });
    expect(route()).toEqual({ name: "home" });
    await user.click(screen.getByRole("button", { name: album.title }));
    expect(route()).toEqual({ name: "album", id: album.id });
  });

  it("the playing album's card shows Pause, and its menu has Add to queue", async () => {
    const user = userEvent.setup();
    const album = mock.fx.albums[0];
    const base = await mock.playerState();
    await setPlayer({ status: "playing", queue: { ...base.queue, context: { kind: "album", id: album.id, title: album.title } } });
    const add = vi.spyOn(backend, "queueAdd");
    render(
      <>
        <AlbumCard album={album} />
        <ContextMenuHost />
      </>,
    );
    expect(screen.getByRole("button", { name: `Pause ${album.title}` })).toBeInTheDocument();
    fireEvent.contextMenu(screen.getByRole("button", { name: album.title }), { clientX: 5, clientY: 5 });
    const menu = await screen.findByRole("menu");
    expect(within(menu).getAllByRole("menuitem").map((i) => i.textContent)).toEqual(["Pause", "Add to queue", "Go to artist", "Pin to sidebar"]);
    await user.click(within(menu).getByRole("menuitem", { name: "Add to queue" }));
    await waitFor(() => expect(add).toHaveBeenCalledWith(mock.fx.tracksByAlbum.get(album.id)!.map((t) => t.id), "end"));
  });
});

// ---------------------------------------------------------------- shortcuts

function ShortcutHost() {
  useShortcuts();
  return null;
}

describe("playback shortcuts", () => {
  beforeEach(async () => {
    await setPlayer({ volume: 0.5, muted: false, shuffle: false, repeat: "off" });
  });

  it("Ctrl+S toggles shuffle", () => {
    const spy = vi.spyOn(backend, "setShuffle");
    render(<ShortcutHost />);
    const ev = fireEvent.keyDown(window, { key: "s", ctrlKey: true });
    expect(ev).toBe(false); // default prevented
    expect(spy).toHaveBeenCalledWith(true);
  });

  it("Ctrl+R cycles repeat off, all, one, off and never reloads", async () => {
    const spy = vi.spyOn(backend, "setRepeat");
    render(<ShortcutHost />);
    expect(fireEvent.keyDown(window, { key: "r", ctrlKey: true })).toBe(false);
    expect(spy).toHaveBeenLastCalledWith("all");
    await setPlayer({ repeat: "all" });
    fireEvent.keyDown(window, { key: "r", ctrlKey: true });
    expect(spy).toHaveBeenLastCalledWith("one");
    await setPlayer({ repeat: "one" });
    fireEvent.keyDown(window, { key: "r", ctrlKey: true });
    expect(spy).toHaveBeenLastCalledWith("off");
    // Still blocks the reload while typing.
    const input = document.createElement("input");
    document.body.appendChild(input);
    expect(fireEvent.keyDown(input, { key: "r", ctrlKey: true })).toBe(false);
    expect(spy).toHaveBeenCalledTimes(3);
    input.remove();
  });

  it("Ctrl+Up / Ctrl+Down change the volume in steps", async () => {
    const spy = vi.spyOn(backend, "setVolume");
    render(<ShortcutHost />);
    fireEvent.keyDown(window, { key: "ArrowUp", ctrlKey: true });
    expect(spy).toHaveBeenLastCalledWith(0.6);
    fireEvent.keyDown(window, { key: "ArrowUp", ctrlKey: true });
    expect(spy).toHaveBeenLastCalledWith(0.7); // builds on the last request
    resetVolumeNudge();
    fireEvent.keyDown(window, { key: "ArrowDown", ctrlKey: true });
    expect(spy).toHaveBeenLastCalledWith(0.4);
  });

  it("Space plays/pauses even with a track row focused, but not while typing", () => {
    const spy = vi.spyOn(backend, "playToggle");
    render(
      <>
        <ShortcutHost />
        <List3 />
        <input aria-label="field" />
      </>,
    );
    rows()[0].focus();
    fireEvent.keyDown(rows()[0], { key: " " });
    expect(spy).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(screen.getByRole("textbox", { name: "field" }), { key: " " });
    expect(spy).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------- player bar

describe("player bar", () => {
  it("the mouse wheel over the volume control nudges it", async () => {
    await setPlayer({ volume: 0.5, muted: false });
    const spy = vi.spyOn(backend, "setVolume");
    render(<PlayerBar />);
    const vol = screen.getByRole("slider", { name: "Volume" }).closest(".volume")!;
    expect(fireEvent.wheel(vol, { deltaY: -100 })).toBe(false);
    expect(spy).toHaveBeenLastCalledWith(0.55);
    resetVolumeNudge();
    fireEvent.wheel(vol, { deltaY: 100 });
    expect(spy).toHaveBeenLastCalledWith(0.45);
  });

  it("raising the volume while muted unmutes back to the remembered level", async () => {
    await setPlayer({ volume: 0.62, muted: true });
    const mute = vi.spyOn(backend, "setMuted");
    const vol = vi.spyOn(backend, "setVolume");
    render(<PlayerBar />);
    expect(screen.getByRole("slider", { name: "Volume" })).toHaveAttribute("aria-valuenow", "0");
    fireEvent.wheel(screen.getByRole("slider", { name: "Volume" }), { deltaY: -100 });
    expect(mute).toHaveBeenCalledWith(false);
    expect(vol).not.toHaveBeenCalled();
  });

  it("the repeat button cycles off, all, one with the one-glyph", async () => {
    const user = userEvent.setup();
    await setPlayer({ repeat: "off" });
    const spy = vi.spyOn(backend, "setRepeat");
    render(<PlayerBar />);
    await user.click(screen.getByRole("button", { name: "Enable repeat" }));
    expect(spy).toHaveBeenLastCalledWith("all");
    await setPlayer({ repeat: "one" });
    const btn = screen.getByRole("button", { name: "Disable repeat" });
    expect(btn).toHaveAttribute("aria-pressed", "true");
    expect(btn.querySelector(".lucide-repeat-1")).not.toBeNull();
  });
});

describe("slider hover", () => {
  it("shows the value under the pointer and hides on leave", () => {
    render(<Slider label="Seek" value={0} max={200_000} hoverText={(v) => `${Math.round(v / 1000)}s`} />);
    const s = screen.getByRole("slider", { name: "Seek" });
    const track = s.querySelector(".slider__track") as HTMLElement;
    track.getBoundingClientRect = () => ({ left: 0, width: 100, top: 0, height: 4, right: 100, bottom: 4, x: 0, y: 0, toJSON() {} });
    fireEvent.pointerMove(s, { clientX: 25 });
    expect(s.querySelector(".slider__hover")).toHaveTextContent("50s");
    fireEvent.pointerLeave(s);
    expect(s.querySelector(".slider__hover")).toBeNull();
  });
});

// ---------------------------------------------------------------- sticky header + scroll

describe("sticky page header", () => {
  it("appears with the title and a play button once the page header scrolls away", async () => {
    const album = mock.fx.albums.find((a) => a.trackCount >= 3)!;
    const base = await mock.playerState();
    await setPlayer({ status: "paused", queue: { ...base.queue, context: null } });
    useRouter.getState().reset({ name: "album", id: album.id });
    const { container } = render(<MainView />);
    await screen.findByRole("heading", { level: 1, name: album.title });
    await waitFor(() => expect(useUi.getState().pageBar?.title).toBe(album.title));
    const scroller = container.querySelector(".main__scroll") as HTMLElement;
    expect(screen.queryByTestId("sticky-bar")).toBeNull();

    scroller.scrollTop = 400;
    fireEvent.scroll(scroller);
    const bar = await screen.findByTestId("sticky-bar");
    expect(bar).toHaveTextContent(album.title);
    const play = vi.spyOn(backend, "playContext");
    fireEvent.click(within(bar).getByRole("button", { name: `Play ${album.title}` }));
    expect(play).toHaveBeenCalled();

    scroller.scrollTop = 0;
    fireEvent.scroll(scroller);
    expect(screen.queryByTestId("sticky-bar")).toBeNull();
  });

  it("threshold follows the action row", () => {
    const sc = document.createElement("div");
    expect(stickyThreshold(sc)).toBe(280);
    const row = document.createElement("div");
    row.className = "action-row";
    Object.defineProperty(row, "offsetTop", { value: 400 });
    Object.defineProperty(row, "offsetHeight", { value: 56 });
    sc.appendChild(row);
    expect(stickyThreshold(sc)).toBe(392);
  });
});

describe("scroll restoration", () => {
  it("keeps trying until the page is tall enough", async () => {
    let height = 300;
    let top = 0;
    const el = document.createElement("div");
    Object.defineProperty(el, "scrollTop", {
      get: () => top,
      set: (v: number) => (top = Math.min(v, height)),
    });
    const done = vi.fn();
    restoreScroll(el, 900, done);
    expect(top).toBe(300);
    expect(done).not.toHaveBeenCalled();
    height = 2000; // content arrived
    await waitFor(() => expect(done).toHaveBeenCalled());
    expect(top).toBe(900);
  });

  it("stops when the user scrolls", () => {
    const el = document.createElement("div");
    let top = 0;
    Object.defineProperty(el, "scrollTop", { get: () => top, set: (v: number) => (top = Math.min(v, 100)) });
    const done = vi.fn();
    restoreScroll(el, 900, done);
    el.dispatchEvent(new Event("wheel"));
    expect(done).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------- queue

describe("queue panel", () => {
  it("rows in the queue have a hover remove button", async () => {
    const user = userEvent.setup();
    const album = mock.fx.albums.find((a) => a.trackCount >= 3)!;
    const other = mock.fx.tracks.find((t) => t.albumId !== album.id)!;
    await act(() => mock.queueClear());
    await act(() => mock.playContext({ context: { kind: "album", id: album.id } }));
    await act(() => mock.queueAdd([other.id], "end"));
    await act(async () => usePlayer.getState().set(await mock.playerState()));
    const spy = vi.spyOn(backend, "queueRemove");
    render(<QueuePanel />);
    const region = screen.getByRole("region", { name: "Next in queue" });
    await user.click(within(region).getByRole("button", { name: `Remove ${other.title} from queue` }));
    expect(spy).toHaveBeenCalledWith(usePlayer.getState().state!.queue.userQueue.find((i) => i.track.id === other.id)!.itemId);
    // Now-playing row has no remove button.
    expect(within(screen.getByRole("region", { name: "Now playing" })).queryByRole("button", { name: /^Remove/ })).toBeNull();
    await act(() => mock.queueClear());
    await act(() => mock.playPause());
  });

  it("drop index accounts for the dragged row leaving its slot", () => {
    expect(moveIndex(0, 3)).toBe(2);
    expect(moveIndex(3, 0)).toBe(0);
    expect(moveIndex(2, 2)).toBe(2);
  });
});

// ---------------------------------------------------------------- sidebar library

const e = (kind: LibEntry["kind"], id: string, title: string, pinned = false): LibEntry => ({ kind, id, title, subtitle: kind, thumb: null, pinned });

describe("sidebar library", () => {
  const items = [e("album", "a1", "Zebra", true), e("playlist", "p1", "Mornings"), e("playlist", "p2", "Afternoon"), e("artist", "r1", "Bell", true)];

  it("keeps pinned first, then sorts the rest", () => {
    const base = { filter: "all" as const, query: "", recents: {} };
    expect(libraryEntries(items, { ...base, sort: "alpha" }).map((x) => x.title)).toEqual(["Zebra", "Bell", "Afternoon", "Mornings"]);
    expect(libraryEntries(items, { ...base, sort: "recents", recents: { "playlist:p2": 5 } }).map((x) => x.title)).toEqual([
      "Zebra",
      "Bell",
      "Afternoon",
      "Mornings",
    ]);
    expect(libraryEntries(items, { ...base, sort: "recents", recents: { "playlist:p1": 9, "playlist:p2": 5 } }).map((x) => x.title)).toEqual([
      "Zebra",
      "Bell",
      "Mornings",
      "Afternoon",
    ]);
  });

  it("filters by type and by search text", () => {
    expect(libraryEntries(items, { filter: "playlist", sort: "alpha", query: "", recents: {} }).map((x) => x.id)).toEqual(["p2", "p1"]);
    expect(libraryEntries(items, { filter: "all", sort: "alpha", query: "morn", recents: {} }).map((x) => x.id)).toEqual(["p1"]);
  });

  it("chips filter the list, search narrows it, and the view menu switches to compact", async () => {
    const user = userEvent.setup();
    act(() => {
      useApp.setState({ playlists: mock.fx.playlists.slice(0, 2) });
      usePins.setState({ pins: [{ kind: "album", id: mock.fx.albums[0].id, title: mock.fx.albums[0].title, subtitle: "Album", thumb: null }] });
      useLibraryView.setState({ filter: "all", sort: "recents", view: "list" });
    });
    useRouter.getState().reset();
    render(
      <>
        <Sidebar collapsed={false} />
        <ContextMenuHost />
      </>,
    );
    const list = screen.getByRole("list", { name: "Your Library items" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(3);
    await user.click(screen.getByRole("button", { name: "Albums" }));
    expect(within(list).getAllByRole("listitem")).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: "Albums" })); // again clears
    expect(within(list).getAllByRole("listitem")).toHaveLength(3);

    await user.click(screen.getByRole("button", { name: "Search in Your Library" }));
    await user.type(screen.getByRole("searchbox", { name: "Search in Your Library" }), mock.fx.playlists[1].title);
    expect(within(list).getAllByRole("listitem")).toHaveLength(1);
    await user.keyboard("{Escape}");
    expect(within(list).getAllByRole("listitem")).toHaveLength(3);

    await user.click(screen.getByRole("button", { name: /^Sort and view/ }));
    await user.click(await screen.findByRole("menuitemradio", { name: "Compact" }));
    expect(useLibraryView.getState().view).toBe("compact");
    expect(list).toHaveClass("is-compact");
    act(() => {
      usePins.setState({ pins: [] });
      useLibraryView.setState({ view: "list" });
    });
  });
});

// ---------------------------------------------------------------- headers + art transition

describe("page title fit", () => {
  it("steps down the scale until it fits", () => {
    expect(pickFitSize([48, 32, 24], (s) => s <= 32)).toBe(32);
    expect(pickFitSize([48, 32, 24], () => false)).toBe(24);
    expect(pickFitSize([48, 32, 24], () => true)).toBe(48);
  });
});

describe("card to page art transition", () => {
  it("falls back to plain navigation without the View Transitions API", () => {
    const go = vi.fn();
    expect(navigateWithArt(document.createElement("span"), go)).toBe(false);
    expect(go).toHaveBeenCalled();
  });

  it("names the clicked art and runs the navigation inside a view transition", async () => {
    const art = document.createElement("span");
    document.body.appendChild(art);
    let named = "";
    const start = vi.fn((update: () => Promise<void>) => {
      named = art.style.viewTransitionName;
      const finished = update();
      return { finished };
    });
    (document as unknown as { startViewTransition: unknown }).startViewTransition = start;
    const go = vi.fn();
    try {
      expect(navigateWithArt(art, go)).toBe(true);
      expect(named).toBe(ART_TRANSITION);
      expect(go).toHaveBeenCalled();
      expect(document.documentElement.dataset.artTransition).toBe("true");
    } finally {
      delete (document as unknown as { startViewTransition?: unknown }).startViewTransition;
      art.remove();
    }
  });

  it("skips the transition under reduced motion", () => {
    const start = vi.fn();
    (document as unknown as { startViewTransition: unknown }).startViewTransition = start;
    document.documentElement.dataset.reducedMotion = "true";
    const go = vi.fn();
    try {
      expect(navigateWithArt(document.createElement("span"), go)).toBe(false);
      expect(start).not.toHaveBeenCalled();
      expect(go).toHaveBeenCalled();
    } finally {
      delete (document as unknown as { startViewTransition?: unknown }).startViewTransition;
      document.documentElement.dataset.reducedMotion = "false";
    }
  });
});
