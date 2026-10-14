import { DEFAULT_SETTINGS } from "../lib/types";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { addedLabel, playedLabel, timeAgo } from "../lib/relativeTime";
import { LibraryPage } from "../pages/LibraryPage";
import { SettingsPage } from "../pages/SettingsPage";
import { geometry } from "../shell/LibraryCollection";
import { Sidebar } from "../shell/Sidebar";
import { mergePins, movePlaylist } from "../shell/useLibrary";
import { useApp } from "../stores/app";
import {
  chipRow,
  effectiveSort,
  libraryEntries,
  loadLibraryView,
  matchesQuery,
  pressChip,
  searchPlaceholder,
  subtitle,
  swapInOrder,
  useLibraryView,
  type LibEntry,
} from "../stores/library";
import { usePins } from "../stores/pins";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

const e = (kind: LibEntry["kind"], id: string, title: string, more: Partial<LibEntry> = {}): LibEntry => ({
  kind,
  id,
  title,
  sortTitle: title,
  creator: null,
  creatorId: null,
  thumb: null,
  smart: false,
  format: null,
  addedAt: null,
  playedAt: null,
  pinned: false,
  ...more,
});

function resetView() {
  useLibraryView.setState({ filter: "all", sub: null, sort: "recents", view: "list", wideView: "grid", customOrder: [], recents: {} });
}

beforeEach(() => {
  resetView();
  usePins.setState({ pins: [] });
  useRouter.getState().reset();
  useUi.setState({ menu: null, sidebarCollapsed: false, sidebarArt: true });
});

afterEach(() => {
  vi.restoreAllMocks();
});

// ---------------------------------------------------------------- chips

describe("library filter chips", () => {
  it("shows the types unfiltered, then clear, the type and its sub-chips", () => {
    expect(chipRow("all", null)).toEqual([
      { kind: "type", type: "playlist", selected: false },
      { kind: "type", type: "album", selected: false },
      { kind: "type", type: "artist", selected: false },
    ]);
    expect(chipRow("all", null, ["album"])).toHaveLength(1);
    expect(chipRow("playlist", null)).toEqual([
      { kind: "clear" },
      { kind: "type", type: "playlist", selected: true },
      { kind: "sub", sub: "byYou", selected: false },
      { kind: "sub", sub: "smart", selected: false },
    ]);
    // Albums and artists have no sub-chips.
    expect(chipRow("album", null)).toEqual([{ kind: "clear" }, { kind: "type", type: "album", selected: true }]);
  });

  it("allows one type and one sub-chip at a time", () => {
    // Picking a sub-chip hides its siblings; picking it again brings them back.
    expect(chipRow("playlist", "smart").map((c) => c.kind)).toEqual(["clear", "type", "sub"]);
    expect(pressChip({ kind: "sub", sub: "smart", selected: false }, "playlist", null)).toEqual({ filter: "playlist", sub: "smart" });
    expect(pressChip({ kind: "sub", sub: "smart", selected: true }, "playlist", "smart")).toEqual({ filter: "playlist", sub: null });
    // A new type replaces the old one and drops the sub-chip.
    expect(pressChip({ kind: "type", type: "album", selected: false }, "playlist", "smart")).toEqual({ filter: "album", sub: null });
    // Deselecting the type, or the clear chip, clears everything.
    expect(pressChip({ kind: "type", type: "playlist", selected: true }, "playlist", "byYou")).toEqual({ filter: "all", sub: null });
    expect(pressChip({ kind: "clear" }, "playlist", "byYou")).toEqual({ filter: "all", sub: null });
  });

  it("drops the type word from subtitles under a filter", () => {
    const album = e("album", "a", "Life", { creator: "Rue Calloway" });
    const ep = e("album", "b", "Short", { creator: "Rue Calloway", format: "ep" });
    const pl = e("playlist", "p", "Mornings", { creator: "listener" });
    const smart = e("playlist", "s", "Most Played", { creator: "listener", smart: true });
    const artist = e("artist", "r", "Rue Calloway");
    expect([album, ep, pl, smart, artist].map((x) => subtitle(x, false))).toEqual([
      "Album · Rue Calloway",
      "EP · Rue Calloway",
      "Playlist · listener",
      "Smart playlist · listener",
      "Artist",
    ]);
    expect([album, pl, artist].map((x) => subtitle(x, true))).toEqual(["Rue Calloway", "listener", ""]);
  });

  it("search placeholder follows the filter", () => {
    expect(searchPlaceholder("all")).toBe("Search in Your Library");
    expect(searchPlaceholder("album")).toBe("Search in albums");
    expect(searchPlaceholder("playlist")).toBe("Search in playlists");
  });
});

// ---------------------------------------------------------------- persistence

describe("library view persistence", () => {
  it("remembers filter, sort and both views across launches", () => {
    act(() => {
      useLibraryView.getState().setFilter("playlist", "smart");
      useLibraryView.getState().setSort("alpha");
      useLibraryView.getState().setView("compactGrid");
      useLibraryView.getState().setWideView("compactList");
    });
    const saved = loadLibraryView(localStorage.getItem("platter.library-view.v1"));
    expect(saved).toMatchObject({ filter: "playlist", sub: "smart", sort: "alpha", view: "compactGrid", wideView: "compactList" });
  });

  it("reads older and broken saves safely", () => {
    expect(loadLibraryView(JSON.stringify({ filter: "album", view: "compact", sort: "alpha" }))).toMatchObject({
      filter: "album",
      sub: null,
      view: "compactList",
      wideView: "grid",
      sort: "alpha",
    });
    // A sub-chip only belongs to its type; unknown values fall back.
    expect(loadLibraryView(JSON.stringify({ filter: "album", sub: "smart", sort: "nope", view: 3 }))).toMatchObject({
      filter: "album",
      sub: null,
      sort: "recents",
      view: "list",
    });
    expect(loadLibraryView("{oops")).toMatchObject({ filter: "all", view: "list" });
  });
});

// ---------------------------------------------------------------- search

describe("library search matching", () => {
  const album = e("album", "a", "The Life of Pablo", { creator: "Kanyé West" });
  it("matches word prefixes in any order, ignoring case and accents", () => {
    expect(matchesQuery(album, "life of")).toBe(true);
    expect(matchesQuery(album, "PABLO life")).toBe(true);
    expect(matchesQuery(album, "kanye")).toBe(true);
    expect(matchesQuery(album, "wést")).toBe(true);
  });
  it("doesn't match inside words or with typos", () => {
    expect(matchesQuery(album, "ablo")).toBe(false);
    expect(matchesQuery(album, "pablp")).toBe(false);
  });
});

// ---------------------------------------------------------------- sort

describe("library sort", () => {
  const pin = e("album", "pin", "Zzz", { pinned: true });
  const items = [
    pin,
    e("playlist", "p1", "Mornings", { creator: "listener", playedAt: 50 }),
    e("album", "a1", "Bell", { creator: "Odessa", addedAt: 300 }),
    e("album", "a2", "Afternoon", { creator: "Cara", addedAt: 100, playedAt: 10 }),
    e("artist", "r1", "Abel", { sortTitle: "Abel" }),
  ];
  const base = { filter: "all" as const, query: "", recents: {} };
  const titles = (opts: Parameters<typeof libraryEntries>[1]) => libraryEntries(items, opts).map((x) => x.title);

  it("keeps pins first whatever the sort", () => {
    for (const sort of ["recents", "added", "alpha", "creator"] as const) expect(titles({ ...base, sort })[0]).toBe("Zzz");
  });

  it("sorts by recents (opened here or played anywhere), added, alphabetical and creator", () => {
    expect(titles({ ...base, sort: "recents" })).toEqual(["Zzz", "Mornings", "Afternoon", "Bell", "Abel"]);
    // Opening Bell moves it to the top, under the pins.
    expect(titles({ ...base, sort: "recents", recents: { "album:a1": 999_999 } })).toEqual(["Zzz", "Bell", "Mornings", "Afternoon", "Abel"]);
    expect(titles({ ...base, sort: "added" })).toEqual(["Zzz", "Bell", "Afternoon", "Abel", "Mornings"]);
    expect(titles({ ...base, sort: "alpha" })).toEqual(["Zzz", "Abel", "Afternoon", "Bell", "Mornings"]);
    expect(titles({ ...base, sort: "creator" })).toEqual(["Zzz", "Abel", "Afternoon", "Mornings", "Bell"]);
  });

  it("offers custom order only under Playlists", () => {
    expect(effectiveSort("custom", "all")).toBe("recents");
    expect(effectiveSort("custom", "playlist")).toBe("custom");
    const lists = [e("playlist", "x", "X"), e("playlist", "y", "Y"), e("playlist", "z", "Z")];
    const order = (customOrder: string[]) =>
      libraryEntries(lists, { filter: "playlist", sort: "custom", query: "", recents: {}, customOrder }).map((x) => x.id);
    expect(order(["z", "x"])).toEqual(["z", "x", "y"]);
    expect(swapInOrder(["z", "x", "y"], "x", "y")).toEqual(["z", "y", "x"]);
  });

  it("moves a playlist among those shown", () => {
    const lists = [e("playlist", "x", "X"), e("playlist", "y", "Y", { smart: true }), e("playlist", "z", "Z")];
    act(() => useLibraryView.setState({ customOrder: ["x", "y", "z"] }));
    // Only regular playlists shown: moving z up swaps it with x, y stays put.
    const shown = libraryEntries(lists, { filter: "playlist", sub: "byYou", sort: "custom", query: "", recents: {}, customOrder: ["x", "y", "z"] });
    movePlaylist(lists, shown, "z", -1);
    expect(useLibraryView.getState().customOrder).toEqual(["z", "y", "x"]);
  });

  it("filters by the Smart and By you sub-chips", () => {
    const lists = [e("playlist", "x", "X"), e("playlist", "y", "Y", { smart: true })];
    const ids = (sub: "smart" | "byYou") => libraryEntries(lists, { filter: "playlist", sub, sort: "alpha", query: "", recents: {} }).map((x) => x.id);
    expect(ids("smart")).toEqual(["y"]);
    expect(ids("byYou")).toEqual(["x"]);
  });

  it("keeps pins that left the library, marked unavailable", () => {
    const all = mergePins([e("album", "a1", "Bell")], [
      { kind: "album", id: "a1", title: "Old title", subtitle: "", thumb: null },
      { kind: "playlist", id: "gone", title: "Gone", subtitle: "", thumb: null },
    ]);
    expect(all.map((x) => [x.title, x.pinned, !!x.unavailable])).toEqual([
      ["Bell", true, false],
      ["Gone", true, true],
    ]);
    expect(subtitle(all[1], false)).toBe("Not available");
  });
});

// ---------------------------------------------------------------- views and geometry

describe("library views", () => {
  it("sizes rows and grids for each place", () => {
    expect(geometry("list", "sidebar", 276).rowHeight).toBe(64);
    expect(geometry("list", "sidebar", 276, false).rowHeight).toBe(48);
    expect(geometry("compactList", "sidebar", 276).rowHeight).toBe(32);
    expect(geometry("grid", "sidebar", 276).cols).toBe(2);
    expect(geometry("grid", "sidebar", 396).cols).toBe(3);
    expect(geometry("compactGrid", "sidebar", 276).cols).toBe(3);
    expect(geometry("list", "rail", 56).rowHeight).toBe(56);
    // Wide grids keep their card size: more columns as the page widens.
    expect(geometry("grid", "wide", 1140).cols).toBeLessThan(geometry("grid", "wide", 2100).cols);
    expect(geometry("compactGrid", "wide", 2100).cols).toBeGreaterThan(geometry("grid", "wide", 2100).cols);
  });

  it("formats the wide table's dates", () => {
    const now = 1_800_000_000;
    expect(timeAgo(now - 7 * 60, now)).toBe("7 minutes ago");
    expect(timeAgo(now - 14 * 3600, now)).toBe("14 hours ago");
    expect(timeAgo(now - 3600, now)).toBe("1 hour ago");
    expect(addedLabel(now - 14 * 3600, now)).toBe("14 hours ago");
    expect(addedLabel(now - 40 * 86400, now)).toMatch(/\d{4}/);
    expect(playedLabel(null, now)).toBe("");
  });
});

// ---------------------------------------------------------------- rendered

async function renderSidebar(collapsed = false, auto = false) {
  act(() => useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } }));
  await act(() => useLibraryView.getState().loadItems());
  return render(
    <>
      <Sidebar collapsed={collapsed} auto={auto} />
      <ContextMenuHost />
    </>,
  );
}

const list = () => screen.getByRole("list", { name: "Your Library items" });
const rows = () => within(list()).getAllByRole("listitem");

describe("library sidebar", () => {
  it("lists playlists, albums and artists; chips and sub-chips filter it", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    expect(rows().length).toBeGreaterThan(3);
    await user.click(screen.getByRole("button", { name: "Playlists" }));
    // Liked Songs is pinned first among the playlists.
    expect(rows()).toHaveLength(mock.fx.playlists.length + 1);
    // Type word gone: just the owner.
    expect(within(rows()[0]).getByText("listener")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Search in playlists" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Smart" }));
    expect(rows()).toHaveLength(mock.fx.playlists.filter((p) => p.smart).length);
    expect(screen.queryByRole("button", { name: "By you" })).toBeNull();
    expect(useLibraryView.getState()).toMatchObject({ filter: "playlist", sub: "smart" });
    await user.click(screen.getByRole("button", { name: "Clear filter" }));
    expect(screen.getByRole("button", { name: "Albums" })).toBeInTheDocument();
    expect(useLibraryView.getState()).toMatchObject({ filter: "all", sub: null });
  });

  it("searches live, shows a no-match message, and Escape clears but keeps focus", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    await user.click(screen.getByRole("button", { name: "Albums" }));
    await user.click(screen.getByRole("button", { name: "Search in albums" }));
    const box = screen.getByRole("searchbox", { name: "Search in albums" });
    const artist = mock.fx.albums[0].artistTitle;
    await user.type(box, artist.split(" ")[0].slice(0, 4));
    const n = mock.fx.albums.filter((a) => matchesQuery({ title: a.title, creator: a.artistTitle }, artist.split(" ")[0].slice(0, 4))).length;
    expect(rows()).toHaveLength(n);
    await user.clear(box);
    await user.type(box, "zzqx");
    expect(screen.getByText(`Couldn't find "zzqx"`)).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(box).toHaveValue("");
    expect(box).toHaveFocus();
    expect(rows().length).toBeGreaterThan(0);
  });

  it("renders each view from the sort and view menu", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    const pick = async (name: string) => {
      await user.click(screen.getByRole("button", { name: /^Sort and view/ }));
      await user.click(await screen.findByRole("menuitemradio", { name }));
    };
    await pick("Compact list");
    expect(list()).toHaveClass("lib-items--compactList");
    expect(list().querySelector(".lib-row--compact img, .lib-row--compact .art")).toBeNull();
    await pick("Compact grid");
    expect(list()).toHaveClass("lib-items--compactGrid");
    expect(list().querySelector(".lib-cell--compact")).not.toBeNull();
    await pick("Grid");
    expect(list().querySelector(".lib-cell__title")).not.toBeNull();
    await pick("List");
    expect(list().querySelector(".lib-row .lib-row__art")).not.toBeNull();
    expect(screen.getByRole("button", { name: "Sort and view: Recents, list view" })).toBeInTheDocument();
    // Custom order is only offered under Playlists.
    await user.click(screen.getByRole("button", { name: /^Sort and view/ }));
    expect(screen.queryByRole("menuitemradio", { name: "Custom order" })).toBeNull();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "Playlists" }));
    await pick("Custom order");
    expect(useLibraryView.getState().sort).toBe("custom");
  });

  it("hides list art when the setting is off", async () => {
    act(() => useUi.getState().setSidebarArt(false));
    await renderSidebar();
    expect(list().querySelector(".lib-row__art")).toBeNull();
    act(() => useUi.getState().setSidebarArt(true));
    expect(list().querySelector(".lib-row__art")).not.toBeNull();
  });

  it("opens on click, plays on double-click, and has a menu on right-click", async () => {
    const user = userEvent.setup();
    const play = vi.spyOn(backend, "playContext");
    await renderSidebar();
    await user.click(screen.getByRole("button", { name: "Albums" }));
    act(() => useLibraryView.getState().setSort("alpha"));
    const first = rows()[0];
    const open = first.querySelector<HTMLButtonElement>(".lib-row__open")!;
    await user.click(open);
    expect(route().name).toBe("album");
    expect(first).toHaveClass("is-selected");
    expect(play).not.toHaveBeenCalled();
    await user.dblClick(open);
    expect(play).toHaveBeenCalledWith(expect.objectContaining({ context: expect.objectContaining({ kind: "album" }) }));
    fireEvent.contextMenu(rows()[0]);
    const menu = await screen.findByRole("menu");
    expect(within(menu).getAllByRole("menuitem").map((m) => m.textContent)).toEqual(["Play", "Add to queue", "Add to playlist", "Go to artist", "Pin"]);
    await user.click(within(menu).getByRole("menuitem", { name: "Pin" }));
    expect(usePins.getState().pins).toHaveLength(1);
    // The pin glyph shows, and the pin stays first.
    expect(within(rows()[0]).getByLabelText("Pinned")).toBeInTheDocument();
  });

  it("colours the playing context's title, also when paused", async () => {
    const album = mock.fx.albums[0];
    await renderSidebar();
    const base = await mock.playerState();
    act(() => usePlayer.getState().set({ ...base, status: "paused", queue: { ...base.queue, context: { kind: "album", id: album.id, title: album.title } } }));
    const title = await screen.findByText(album.title, { selector: ".lib-row__title" });
    expect(title).toHaveClass("is-playing");
  });

  it("moves focus with the arrow keys; Enter opens the item", async () => {
    const user = userEvent.setup();
    await renderSidebar();
    const buttons = () => list().querySelectorAll<HTMLButtonElement>(".lib-row__open");
    expect(buttons()[0]).toHaveAttribute("tabindex", "0");
    expect(buttons()[1]).toHaveAttribute("tabindex", "-1");
    buttons()[0].focus();
    await user.keyboard("{ArrowDown}");
    await waitFor(() => expect(buttons()[1]).toHaveFocus());
    await user.keyboard("{ArrowDown}{ArrowUp}");
    await waitFor(() => expect(buttons()[1]).toHaveFocus());
    await user.keyboard("{Enter}");
    expect(route().name).not.toBe("home");
  });

  it("collapses to the rail and opens again", async () => {
    const user = userEvent.setup();
    const { unmount } = await renderSidebar();
    await user.click(screen.getByRole("button", { name: "Collapse Your Library" }));
    expect(useUi.getState().sidebarCollapsed).toBe(true);
    unmount();
    await renderSidebar(true);
    // The rail: no chips or search, art tiles labelled with title and subtitle.
    expect(screen.queryByRole("group", { name: "Show in library" })).toBeNull();
    expect(screen.queryByRole("searchbox")).toBeNull();
    const first = rows()[0].querySelector("button")!;
    expect(first.getAttribute("aria-label")).toMatch(/ · |, /);
    await user.click(screen.getByRole("button", { name: "Open Your Library" }));
    expect(useUi.getState().sidebarCollapsed).toBe(false);
  });

  it("an auto-collapsed rail opens the wide library instead", async () => {
    const user = userEvent.setup();
    await renderSidebar(true, true);
    await user.click(screen.getByRole("button", { name: "Open Your Library" }));
    expect(route()).toEqual({ name: "library" });
  });
});

describe("wide library", () => {
  it("keeps its own view, shows the table in list views, and minimizes back", async () => {
    const user = userEvent.setup();
    act(() => useLibraryView.setState({ view: "list", wideView: "list" }));
    useRouter.getState().navigate({ name: "album", id: mock.fx.albums[0].id });
    useRouter.getState().navigate({ name: "library", tab: "artists" });
    await act(() => useLibraryView.getState().loadItems());
    render(
      <>
        <LibraryPage tab="artists" />
        <ContextMenuHost />
      </>,
    );
    // Arriving with a type filters to it.
    expect(useLibraryView.getState().filter).toBe("artist");
    expect(screen.getByRole("searchbox", { name: "Search in artists" })).toBeInTheDocument();
    expect(screen.getByText("Date added")).toBeInTheDocument();
    expect(screen.getByText("Played", { selector: ".lib-thead__played" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^Sort and view/ }));
    await user.click(await screen.findByRole("menuitemradio", { name: "Grid" }));
    expect(useLibraryView.getState()).toMatchObject({ wideView: "grid", view: "list" });
    expect(screen.queryByText("Date added")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Minimize Your Library" }));
    expect(route().name).toBe("album");
  });
});

describe("library art setting", () => {
  it("is a Settings row, saved per device", async () => {
    const user = userEvent.setup();
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS } }));
    render(<SettingsPage />);
    const sw = screen.getByRole("switch", { name: "Show the library sidebar's art" });
    expect(sw).toBeChecked();
    await user.click(sw);
    expect(useUi.getState().sidebarArt).toBe(false);
    expect(localStorage.getItem("platter.sidebar-art.v1")).toBe("off");
  });
});
