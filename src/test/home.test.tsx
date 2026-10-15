import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MockHome, shortcuts } from "../lib/mock/home";
import type { MockListen } from "../lib/mock/recents";
import { useLikes } from "../stores/likes";
import { DEFAULT_SETTINGS, type Album, type Artist, type HomeData, type HomeItem, type Playlist } from "../lib/types";
import homeCss from "../pages/home.css?raw";
import { HomePage } from "../pages/HomePage";
import { HomeShelfPage } from "../pages/HomeShelfPage";
import { gridItems, greeting, showsLikedTile, visibleShelves } from "../pages/home";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";

// Wash colour per cover: "rgb(<n> 0 0)" for art path /art/<n>.
vi.mock("../lib/color", async (orig) => {
  const real = await orig<typeof import("../lib/color")>();
  return {
    ...real,
    headerColorsFromImageUrl: vi.fn(async (url: string) => {
      const n = /art(?:%2F|\/)(\d+)/.exec(decodeURIComponent(url))?.[1] ?? "0";
      return { vivid: `rgb(${n} 0 0)`, shade: "rgb(0 0 0)" };
    }),
  };
});

const mock = backend as MockBackend;

function album(n: number): Album {
  return {
    id: `al${n}`,
    title: `Album ${n}`,
    sortTitle: `Album ${n}`,
    artistId: "ar1",
    artistTitle: "Some Artist",
    year: 2000 + n,
    thumb: `/art/${n}`,
    format: "album",
    subformats: [],
    section: "albums",
    genres: [],
    trackCount: 10,
    addedAt: n,
    viewCount: n,
    lastViewedAt: null,
    tint: null,
  };
}
const artist = (n: number): Artist => ({ id: `ar${n}`, title: `Artist ${n}`, sortTitle: `Artist ${n}`, thumb: `/art/${100 + n}`, art: null, viewCount: n });
const playlist = (n: number): Playlist => ({ id: `pl${n}`, title: `Playlist ${n}`, smart: false, trackCount: 5, durationMs: 1000, thumb: `/art/${200 + n}` });
const A = (n: number): HomeItem => ({ kind: "album", album: album(n) });
const R = (n: number): HomeItem => ({ kind: "artist", artist: artist(n) });
const P = (n: number): HomeItem => ({ kind: "playlist", playlist: playlist(n) });

function data(over: Partial<HomeData> = {}): HomeData {
  return {
    shortcuts: [A(1), R(2), P(3), A(4), A(5), R(6), P(7), A(8)],
    recentlyPlayed: [A(1), R(2), P(3)],
    jumpBackIn: [album(9)],
    playlists: [playlist(3), playlist(7)],
    recentlyAdded: [album(10), album(11)],
    topArtists: [artist(2), artist(6)],
    ...over,
  };
}

const shelfTitles = () => screen.queryAllByRole("heading", { level: 2 }).map((h) => h.textContent);
const tiles = () => {
  const grid = screen.queryByRole("list", { name: "Shortcuts" });
  return grid ? within(grid).getAllByRole("listitem") : [];
};
const washOf = () => (screen.getByTestId("home") as HTMLElement).style.getPropertyValue("--home-wash");

function renderHome(d: HomeData) {
  vi.spyOn(backend, "home").mockResolvedValue(d);
  return render(
    <div className="main__scroll">
      <HomePage />
    </div>,
  );
}

beforeEach(() => {
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useRouter.getState().reset();
  Element.prototype.scrollTo = vi.fn();
  vi.spyOn(backend, "artUrl").mockImplementation((path) => `http://art.test${path}`);
});
afterEach(() => vi.restoreAllMocks());

describe("home decisions", () => {
  it("keeps the shelves in a fixed order and leaves empty ones out", () => {
    expect(visibleShelves(data(), "all").map((s) => s.title)).toEqual([
      "Recently played",
      "Jump back in",
      "Your playlists",
      "Recently added",
      "Your top artists",
    ]);
    expect(visibleShelves(data({ jumpBackIn: [], playlists: [] }), "all").map((s) => s.id)).toEqual(["recentlyPlayed", "recentlyAdded", "topArtists"]);
  });

  it("narrows every shelf to the chip's type", () => {
    const artists = visibleShelves(data(), "artists");
    expect(artists.map((s) => s.id)).toEqual(["recentlyPlayed", "topArtists"]);
    expect(artists[0].items).toEqual([R(2)]);
    expect(visibleShelves(data(), "playlists").map((s) => s.id)).toEqual(["recentlyPlayed", "playlists"]);
  });

  it("shows at most 8 tiles, what there is when fewer, none with the setting off", () => {
    expect(gridItems(data({ shortcuts: [...data().shortcuts, A(20)] }), "all", true)).toHaveLength(8);
    // Liked Songs takes the first of the 8.
    expect(gridItems(data(), "all", true, true)).toHaveLength(7);
    expect(gridItems(data({ shortcuts: [A(1), R(2), P(3)] }), "all", true)).toHaveLength(3);
    expect(gridItems(data(), "albums", true).every((i) => i.kind === "album")).toBe(true);
    expect(gridItems(data(), "all", false)).toEqual([]);
  });

  it("the shortcut grid leads with what's hot, then favourites, then the rest, without repeats", () => {
    const hot = [A(1), A(2), A(3), A(4), A(5), A(6)];
    const got = shortcuts(hot, [A(1), R(1), P(1)], [A(9)]);
    expect(got.map((i) => (i.kind === "album" ? i.album.id : i.kind === "artist" ? i.artist.id : i.playlist.id))).toEqual([
      "al1",
      "al2",
      "al3",
      "al4",
      "ar1",
      "pl1",
      "al5",
      "al6",
    ]);
    expect(shortcuts([A(1)], [A(1)], [A(1), A(2)])).toHaveLength(2);
    expect(shortcuts([], [])).toEqual([]);
  });

  it("Liked Songs leads the grid on All and Playlists once something is liked", () => {
    expect(showsLikedTile("all", 3, true)).toBe(true);
    expect(showsLikedTile("playlists", 3, true)).toBe(true);
    expect(showsLikedTile("albums", 3, true)).toBe(false);
    expect(showsLikedTile("all", 0, true)).toBe(false);
    expect(showsLikedTile("all", 3, false)).toBe(false);
  });

  it("mock ranking: several plays today beat one, and older plays fade", () => {
    const m = new MockHome(mock.fx, [], []);
    const [a, b, c] = mock.fx.albums;
    const tr = (al: Album) => mock.fx.tracks.filter((t) => t.albumId === al.id);
    const now = Date.now();
    const l = (al: Album, agoMs: number, n: number): MockListen[] =>
      tr(al).slice(0, n).map((t) => ({ trackId: t.id, at: now - agoMs, end: now - agoMs + 1000, kind: "album", id: al.id }));
    const hot = new MockHome(mock.fx, [], [...l(a, 60_000, 1), ...l(b, 3_600_000, 3), ...l(c, 6 * 86_400_000, 3)]).hot(8, now);
    expect(hot.map((i) => (i.kind === "album" ? i.album.id : ""))).toEqual([b.id, a.id, c.id]);
    expect(m.hot(8, now)).toEqual([]);
  });

  it("the heading follows the time of day", () => {
    expect(greeting(new Date(2026, 0, 1, 8))).toBe("Good morning");
    expect(greeting(new Date(2026, 0, 1, 14))).toBe("Good afternoon");
    expect(greeting(new Date(2026, 0, 1, 21))).toBe("Good evening");
  });
});

describe("home page", () => {
  it("has a screen-reader heading and no visible title", async () => {
    renderHome(data());
    const h1 = screen.getByRole("heading", { level: 1 });
    expect(h1).toHaveClass("sr-only");
    expect(h1.textContent).toBe(greeting());
    await waitFor(() => expect(tiles()).toHaveLength(8));
  });

  it("chips filter the grid and shelves and scroll back to the top", async () => {
    const user = userEvent.setup();
    renderHome(data());
    await waitFor(() => expect(shelfTitles()).toContain("Your top artists"));
    expect(screen.getByRole("button", { name: "All" })).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByRole("button", { name: "Artists" }));
    expect(shelfTitles()).toEqual(["Recently played", "Your top artists"]);
    expect(tiles()).toHaveLength(2);
    expect(Element.prototype.scrollTo).toHaveBeenCalledWith({ top: 0 });
    await user.click(screen.getByRole("button", { name: "Albums" }));
    expect(shelfTitles()).toEqual(["Recently played", "Jump back in", "Recently added"]);
    await user.click(screen.getByRole("button", { name: "All" }));
    // Five shelves, the three mix shelves, the release radar and New to you (those show on All only).
    await waitFor(() => expect(shelfTitles()).toHaveLength(10));
    expect(shelfTitles()).toContain("New to you");
    expect(shelfTitles()).toContain("Release radar");
  });

  it("a chip with nothing to show says so", async () => {
    const user = userEvent.setup();
    renderHome(data({ shortcuts: [A(1)], recentlyPlayed: [A(1)], playlists: [], topArtists: [] }));
    await waitFor(() => expect(tiles()).toHaveLength(1));
    await user.click(screen.getByRole("button", { name: "Playlists" }));
    expect(screen.getByRole("status")).toHaveTextContent("No playlists to show yet.");
  });

  it("shows fewer tiles when fewer are known, and hides the grid at none", async () => {
    const { unmount } = renderHome(data({ shortcuts: [A(1), P(3), R(2)] }));
    await waitFor(() => expect(tiles()).toHaveLength(3));
    unmount();
    renderHome(data({ shortcuts: [] }));
    await waitFor(() => expect(shelfTitles()).toContain("Recently played"));
    expect(screen.queryByRole("list", { name: "Shortcuts" })).toBeNull();
  });

  it("hides empty shelves, and shows the empty state when there's nothing at all", async () => {
    const { unmount } = renderHome(data({ jumpBackIn: [], topArtists: [] }));
    await waitFor(() =>
      expect(shelfTitles()).toEqual(["Made for you", "Recently played", "Your mixes by mood", "Your genre mixes", "Your playlists", "Recently added"]),
    );
    unmount();
    renderHome({ shortcuts: [], recentlyPlayed: [], jumpBackIn: [], playlists: [], recentlyAdded: [], topArtists: [] });
    expect(await screen.findByText("Nothing here yet")).toBeInTheDocument();
  });

  it("the shortcut grid setting hides the grid", async () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, homeShortcuts: false } });
    renderHome(data());
    await waitFor(() => expect(shelfTitles()).toContain("Recently played"));
    expect(screen.queryByRole("list", { name: "Shortcuts" })).toBeNull();
  });

  it("tiles open their item and play from the button", async () => {
    const user = userEvent.setup();
    const play = vi.spyOn(backend, "playContext").mockResolvedValue();
    renderHome(data());
    await waitFor(() => expect(tiles()).toHaveLength(8));
    await user.click(within(tiles()[1]).getByRole("button", { name: "Play Artist 2" }));
    expect(play).toHaveBeenCalledWith(expect.objectContaining({ context: expect.objectContaining({ kind: "artist", id: "ar2" }) }));
    await user.click(within(tiles()[2]).getByRole("button", { name: "Playlist 3" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "playlist", id: "pl3" });
  });

  it("Show all opens the shelf's grid page", async () => {
    const user = userEvent.setup();
    renderHome(data());
    await waitFor(() => expect(shelfTitles()).toContain("Your top artists"));
    const shelf = screen.getByRole("heading", { name: "Your top artists" }).closest("section")!;
    await user.click(within(shelf as HTMLElement).getByRole("button", { name: "Show all" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "homeShelf", shelf: "topArtists" });
  });

  it("Liked Songs is the first tile once something is liked, and plays or opens Liked Songs", async () => {
    const user = userEvent.setup();
    const play = vi.spyOn(backend, "playContext").mockResolvedValue();
    useLikes.setState({ ids: new Set(["t1"]), loaded: true });
    renderHome(data());
    await waitFor(() => expect(tiles()).toHaveLength(8));
    expect(within(tiles()[0]).getByRole("button", { name: "Liked Songs" })).toBeInTheDocument();
    expect(within(tiles()[7]).getByRole("button", { name: "Playlist 7" })).toBeInTheDocument();
    await user.click(within(tiles()[0]).getByRole("button", { name: "Play Liked Songs" }));
    expect(play).toHaveBeenCalledWith(expect.objectContaining({ context: expect.objectContaining({ kind: "liked" }) }));
    await user.click(within(tiles()[0]).getByRole("button", { name: "Liked Songs" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "liked" });
    // Not on the Albums chip.
    await user.click(screen.getByRole("button", { name: "Albums" }));
    expect(screen.queryByRole("button", { name: "Play Liked Songs" })).toBeNull();
    useLikes.setState({ ids: new Set(), loaded: true });
  });

  it("Recently played's Show all opens Recents", async () => {
    const user = userEvent.setup();
    renderHome(data());
    await waitFor(() => expect(shelfTitles()).toContain("Recently played"));
    const shelf = screen.getByRole("heading", { name: "Recently played" }).closest("section")!;
    await user.click(within(shelf as HTMLElement).getByRole("button", { name: "Show all" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "recents" });
  });
});

describe("colour wash", () => {
  it("takes the first tile's colour at rest and follows the hovered tile", async () => {
    renderHome(data());
    await waitFor(() => expect(washOf()).toBe("rgb(1 0 0)"));
    expect(screen.getByTestId("home")).toHaveClass("has-wash");
    fireEvent.pointerEnter(tiles()[1]);
    await waitFor(() => expect(washOf()).toBe("rgb(102 0 0)"));
    fireEvent.pointerEnter(tiles()[2]);
    await waitFor(() => expect(washOf()).toBe("rgb(203 0 0)"));
    fireEvent.pointerLeave(screen.getByRole("list", { name: "Shortcuts" }));
    await waitFor(() => expect(washOf()).toBe("rgb(1 0 0)"));
  });

  it("keyboard focus on a tile moves the wash too", async () => {
    renderHome(data());
    await waitFor(() => expect(tiles()).toHaveLength(8));
    act(() => within(tiles()[3]).getByRole("button", { name: "Album 4" }).focus());
    await waitFor(() => expect(washOf()).toBe("rgb(4 0 0)"));
  });

  it("is off with Artwork colours off, or with its own setting off", async () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, artworkColours: false } });
    const { unmount } = renderHome(data());
    await waitFor(() => expect(tiles()).toHaveLength(8));
    fireEvent.pointerEnter(tiles()[1]);
    await act(async () => {});
    expect(washOf()).toBe("");
    expect(screen.getByTestId("home")).not.toHaveClass("has-wash");
    unmount();
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, homeColourWash: false } });
    renderHome(data());
    await waitFor(() => expect(tiles()).toHaveLength(8));
    await act(async () => {});
    expect(washOf()).toBe("");
  });

  it("cross-fades on the motion bands and swaps at once under reduced motion", () => {
    expect(homeCss).toMatch(/@property --home-wash \{\s*syntax: "<color>";/);
    expect(homeCss).toMatch(/transition: --home-wash var\(--motion-slow\)/);
    const reduced = homeCss.slice(homeCss.indexOf(':root[data-reduced-motion="true"] .page-pad.home'));
    expect(reduced.slice(0, reduced.indexOf("}"))).toMatch(/\.home__wash \{\s*transition-duration: 0s;/);
  });
});

describe("show all page", () => {
  it("lists every item of the shelf from the core", async () => {
    const shelf = vi.spyOn(backend, "homeShelf");
    render(<HomeShelfPage shelf="recentlyAdded" />);
    expect(screen.getByRole("heading", { level: 1, name: "Recently added" })).toBeInTheDocument();
    const grid = screen.getByRole("list", { name: "Recently added" });
    await waitFor(() => expect(within(grid).getAllByRole("listitem").length).toBeGreaterThan(12));
    expect(shelf).toHaveBeenCalledWith("recentlyAdded");
  });
});

describe("mock history", () => {
  it("fills the grid and every shelf", async () => {
    const d = await mock.home();
    expect(d.shortcuts).toHaveLength(8);
    expect(new Set(d.shortcuts.map((i) => i.kind))).toEqual(new Set(["album", "artist", "playlist"]));
    for (const k of ["recentlyPlayed", "jumpBackIn", "playlists", "recentlyAdded", "topArtists"] as const) expect(d[k].length, k).toBeGreaterThan(0);
  });

  it("a played playlist moves to the front of recently played", async () => {
    const pl = mock.fx.playlists[5];
    await mock.playContext({ context: { kind: "playlist", id: pl.id } });
    const d = await mock.home();
    expect(d.recentlyPlayed[0]).toEqual({ kind: "playlist", playlist: pl });
    expect(d.playlists[0].id).toBe(pl.id);
    await mock.playPause();
  });
});
