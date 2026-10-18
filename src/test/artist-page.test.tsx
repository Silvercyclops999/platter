import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { Album, ArtistPageSection, ReleaseSection } from "../lib/types";
import { ArtistPage } from "../pages/ArtistPage";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import {
  activeChip,
  artistSections,
  catalogueEntries,
  discographyChips,
  discographyItems,
  headerVariant,
  libraryEntries,
  missingEntries,
  playsLine,
} from "../pages/artist";
import { body } from "./rows";

const mock = backend as MockBackend;

function album(id: string, section: ReleaseSection, year: number | null, viewCount: number, addedAt = 0): Album {
  return {
    id,
    title: `Album ${id}`,
    sortTitle: `Album ${id}`,
    artistId: "1",
    artistTitle: "A",
    year,
    thumb: null,
    format: "album",
    subformats: [],
    section,
    genres: [],
    trackCount: 10,
    addedAt,
    viewCount,
    lastViewedAt: null,
    tint: null,
  };
}

const SECTIONS: ArtistPageSection[] = [
  { section: "albums", albums: [album("a1", "albums", 2015, 50), album("a2", "albums", 2019, 5), album("a3", "albums", 2012, 90)] },
  { section: "singlesEps", albums: [album("s1", "singlesEps", 2021, 1), album("s2", "singlesEps", 2018, 30)] },
  { section: "compilations", albums: [album("c1", "compilations", null, 0)] },
];

describe("artist page decisions", () => {
  it("uses the banner only when Plex has background art", () => {
    expect(headerVariant({ art: "/library/metadata/1/art/2" })).toBe("banner");
    expect(headerVariant({ art: null })).toBe("photo");
  });

  it("offers popular releases plus one chip per release type the artist has", () => {
    expect(discographyChips(libraryEntries(SECTIONS)).map((c) => c.label)).toEqual(["Popular releases", "Albums", "Singles and EPs", "Compilations"]);
  });

  it("puts the latest release first in popular releases, then the most played", () => {
    const items = discographyItems(libraryEntries(SECTIONS), "popular");
    expect(items.map((i) => i.entry.key)).toEqual(["s1", "a3", "a1", "s2", "a2", "c1"]);
    expect(items.map((i) => i.latest)).toEqual([true, false, false, false, false, false]);
    expect(discographyItems(libraryEntries(SECTIONS), "popular", 3).map((i) => i.entry.key)).toEqual(["s1", "a3", "a1"]);
  });

  it("lists a release type newest first, every release", () => {
    const entries = libraryEntries(SECTIONS);
    expect(discographyItems(entries, "albums").map((i) => i.entry.key)).toEqual(["a2", "a1", "a3"]);
    expect(discographyItems(entries, "singlesEps").every((i) => !i.latest)).toBe(true);
    // A chip the artist doesn't have falls back to popular releases.
    expect(discographyItems(entries, "live")[0].entry.key).toBe("s1");
    expect(discographyItems([], "popular")).toEqual([]);
    expect(activeChip(discographyChips(entries), "live")).toBe("popular");
    expect(activeChip(discographyChips(entries), "compilations")).toBe("compilations");
  });

  it("puts releases you can get in the same chips, after the library's in popular releases", () => {
    const cat = (n: string, kind: string, secondary: string[], year: number) => ({
      releaseGroupId: n,
      title: `Get ${n}`,
      artist: "A",
      artistMbid: null,
      kind,
      secondary,
      year,
      thumb: null,
      libraryAlbumId: null,
      request: null,
      refusal: null,
      inMissing: false,
    });
    const got = catalogueEntries([
      { kind: "albums", albums: [cat("g1", "album", [], 2024)] },
      { kind: "singlesEps", albums: [cat("g2", "single", [], 2025)] },
      { kind: "compilations", albums: [cat("g3", "album", ["compilation"], 2010)] },
    ]);
    const all = [...libraryEntries(SECTIONS.slice(0, 1)), ...got];
    expect(discographyChips(all).map((c) => c.id)).toEqual(["popular", "albums", "singlesEps", "compilations"]);
    expect(discographyItems(all, "albums").map((i) => i.entry.key)).toEqual(["rg:g1", "a2", "a1", "a3"]);
    expect(discographyItems(all, "popular").map((i) => i.entry.key)).toEqual(["rg:g2", "a3", "a1", "a2", "rg:g1", "rg:g3"]);
    // Lidarr's releases keep their section; older cores without one: albums and EPs.
    const missing = missingEntries(
      [
        { releaseGroupId: "m1", title: "M1", kind: "single", section: "singlesEps", year: 2020, request: null, thumb: null },
        { releaseGroupId: "m2", title: "M2", kind: "album", year: 2019, request: null, thumb: null },
      ],
      "A",
    );
    expect(missing.map((e) => e.section)).toEqual(["singlesEps", "albums"]);
    expect(missing[0].missing?.releaseGroupId).toBe("m1");
  });

  it("orders sections and leaves out empty ones", () => {
    const base = { popular: [mock.fx.tracks[0]], sections: SECTIONS, about: "Bio", similar: [mock.fx.artists[1]] };
    expect(artistSections(base)).toEqual(["popular", "discography", "about", "fansAlsoLike"]);
    expect(artistSections({ ...base, about: "  ", similar: [] })).toEqual(["popular", "discography"]);
    expect(artistSections({ popular: [], sections: [], about: null, similar: [] })).toEqual([]);
    expect(artistSections({ ...base, appearsOn: [album("x", "albums", 2020, 0)] })).toEqual(["popular", "discography", "about", "fansAlsoLike", "appearsOn"]);
    // A featured guest: no albums of their own, but releases to get.
    expect(artistSections({ popular: [mock.fx.tracks[0]], sections: [], about: null, similar: [], catalogue: 3, appearsOn: [album("x", "albums", 2020, 0)] })).toEqual([
      "popular",
      "discography",
      "appearsOn",
    ]);
  });

  it("writes the plays line", () => {
    expect(playsLine(1204)).toBe("1,204 plays in your library");
    expect(playsLine(1)).toBe("1 play in your library");
    expect(playsLine(0)).toBe("0 plays in your library");
  });
});

/** Artist fixtures: even index has background art, every third has no bio, the last three have no similar artists. */
const artistAt = (i: number) => mock.fx.artists[i];

function headings() {
  return screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
}

describe("artist page", () => {
  it("shows the banner header with the plays line, and sections in order", async () => {
    const a = artistAt(0);
    const { container } = render(<ArtistPage id={a.id} />);
    await screen.findByRole("heading", { level: 1, name: a.title });
    expect(container.querySelector("header[data-variant]")).toHaveAttribute("data-variant", "banner");
    const plays = mock.fx.tracks.filter((t) => t.artistId === a.id).reduce((n, t) => n + t.viewCount, 0);
    expect(screen.getAllByText(playsLine(plays)).length).toBeGreaterThan(0);
    const appears = (await mock.artistPage(a.id)).appearsOn.length ? ["Appears on"] : [];
    expect(headings()).toEqual(["Popular", "Discography", "About", "Fans also like", ...appears]);
  });

  it("shows the round photo header when there's no background art", async () => {
    const a = artistAt(1);
    const { container } = render(<ArtistPage id={a.id} />);
    await screen.findByRole("heading", { level: 1, name: a.title });
    expect(container.querySelector("header[data-variant]")).toHaveAttribute("data-variant", "photo");
    expect(container.querySelector(".detail-header__art .art--round")).not.toBeNull();
  });

  it("hides Fans also like when Plex has none, and keeps About saying there's no bio", async () => {
    const a = artistAt(11);
    render(<ArtistPage id={a.id} />);
    await screen.findByRole("heading", { level: 1, name: a.title });
    const appears = (await mock.artistPage(a.id)).appearsOn.length ? ["Appears on"] : [];
    expect(headings()).toEqual(["Popular", "Discography", "About", ...appears]);
    expect(within(screen.getByRole("button", { name: `About ${a.title}` })).getByText("No biography available.")).toBeInTheDocument();
  });

  it("shows five popular songs, ten after See more, and back", async () => {
    const user = userEvent.setup();
    render(<ArtistPage id={artistAt(0).id} />);
    await screen.findByRole("button", { name: "See more" });
    const list = screen.getByRole("grid", { name: "Popular songs" });
    expect(body(within(list).getAllByRole("row"))).toHaveLength(5);
    await user.click(screen.getByRole("button", { name: "See more" }));
    expect(body(within(list).getAllByRole("row"))).toHaveLength(10);
    await user.click(screen.getByRole("button", { name: "Show less" }));
    expect(body(within(list).getAllByRole("row"))).toHaveLength(5);
  });

  it("switches the discography row with chips that act like tabs", async () => {
    const user = userEvent.setup();
    const a = artistAt(0);
    render(<ArtistPage id={a.id} />);
    const chips = await screen.findByRole("group", { name: "Release type" });
    const popular = within(chips).getByRole("button", { name: "Popular releases" });
    expect(popular).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText(/^Latest release ·/)).toBeInTheDocument();

    const albums = within(chips).getByRole("button", { name: "Albums" });
    await user.click(albums);
    expect(albums).toHaveAttribute("aria-pressed", "true");
    expect(popular).toHaveAttribute("aria-pressed", "false");
    // Builds Plex couldn't match count as unreleased and have a section of their own.
    const own = mock.fx.albums.filter((x) => x.artistId === a.id && x.section === "albums" && x.format !== null);
    expect(within(chips).getByRole("button", { name: "Unreleased" })).toBeInTheDocument();
    const discography = screen.getByRole("heading", { name: "Discography" }).closest("section")!;
    const cards = within(discography).getAllByRole("listitem");
    expect(cards).toHaveLength(own.length);
    expect(screen.queryByText(/^Latest release ·/)).toBeNull();

    // Clicking the chip that's on keeps it on.
    await user.click(albums);
    expect(albums).toHaveAttribute("aria-pressed", "true");
  });

  it("Show all opens the discography page, at the chip's release type", async () => {
    const user = userEvent.setup();
    const a = artistAt(0);
    useRouter.getState().reset();
    render(<ArtistPage id={a.id} />);
    const chips = await screen.findByRole("group", { name: "Release type" });
    const shelf = screen.getByRole("heading", { name: "Discography" }).closest("section")!;
    await user.click(within(shelf).getByRole("button", { name: "Show all" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "discography", id: a.id });
    await user.click(within(chips).getAllByRole("button")[1]);
    await user.click(within(shelf).getByRole("button", { name: "Show all" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toMatchObject({ name: "discography", id: a.id, type: expect.any(String) });
  });

  it("shows similar artists that arrive after the page opened", async () => {
    const a = artistAt(11);
    render(<ArtistPage id={a.id} />);
    await screen.findByRole("heading", { level: 1, name: a.title });
    expect(screen.queryByRole("heading", { name: "Fans also like" })).toBeNull();

    const real = mock.artistPage.bind(mock);
    const spy = vi.spyOn(mock, "artistPage").mockImplementation(async (id) => ({ ...(await real(id)), similar: [artistAt(3)] }));
    // Another artist's update is ignored.
    act(() => mock.emit("artist://changed", { id: artistAt(5).id }));
    expect(spy).not.toHaveBeenCalled();
    act(() => mock.emit("artist://changed", { id: a.id }));
    const h = await screen.findByRole("heading", { name: "Fans also like" });
    expect(within(h.closest("section")!).getByText(artistAt(3).title)).toBeInTheDocument();
    // The rest of the page stays put (no skeleton flash).
    expect(screen.getByRole("heading", { level: 1, name: a.title })).toBeInTheDocument();
    spy.mockRestore();
  });

  it("lists similar artists from the library under Fans also like", async () => {
    const a = artistAt(0);
    render(<ArtistPage id={a.id} />);
    const h = await screen.findByRole("heading", { name: "Fans also like" });
    const shelf = h.closest("section")!;
    expect(within(shelf).getByText(artistAt(1).title)).toBeInTheDocument();
    expect(within(shelf).getByText(artistAt(2).title)).toBeInTheDocument();
  });

  it("reloads Popular when the global list arrives", async () => {
    const a = artistAt(0);
    render(<ArtistPage id={a.id} />);
    await screen.findByRole("heading", { level: 1, name: a.title });
    const list = screen.getByRole("grid", { name: "Popular songs" });
    const real = mock.artistPage.bind(mock);
    const hit = mock.fx.tracks.find((t) => t.artistId !== a.id)!;
    const spy = vi.spyOn(mock, "artistPage").mockImplementation(async (id) => ({ ...(await real(id)), popular: [{ ...hit, title: "Global Number One" }] }));
    try {
      act(() => mock.emit("artistinfo://changed", { id: a.id }));
      expect(await screen.findByRole("row", { name: /Global Number One/ })).toBeInTheDocument();
      expect(list).toBeInTheDocument();
    } finally {
      spy.mockRestore();
    }
  });

  /** Popular's section: placeholder rows are busy, the list isn't. */
  const popularSection = () => screen.getByRole("heading", { name: "Popular" }).closest("section")!;

  it("shows placeholder rows while the global list is on its way, then the list", async () => {
    const a = artistAt(0);
    const real = mock.artistPage.bind(mock);
    const hit = mock.fx.tracks.find((t) => t.artistId !== a.id)!;
    let arrived = false;
    const spy = vi.spyOn(mock, "artistPage").mockImplementation(async (id) => {
      const page = await real(id);
      return arrived ? { ...page, popular: [{ ...hit, title: "Global Number One" }], popularPending: false } : { ...page, popularPending: true };
    });
    try {
      render(<ArtistPage id={a.id} />);
      await screen.findByRole("heading", { level: 1, name: a.title });
      // Plays order never shows: placeholders until the list lands.
      expect(popularSection()).toHaveAttribute("aria-busy", "true");
      const first = (await real(a.id)).popular[0];
      expect(within(popularSection()).queryByRole("row", { name: new RegExp(first.title) })).toBeNull();
      arrived = true;
      act(() => mock.emit("artistinfo://changed", { id: a.id }));
      expect(await screen.findByRole("row", { name: /Global Number One/ })).toBeInTheDocument();
      expect(popularSection()).not.toHaveAttribute("aria-busy");
    } finally {
      spy.mockRestore();
    }
  });

  it("falls back to plays order when the global list fails", async () => {
    const a = artistAt(0);
    const real = mock.artistPage.bind(mock);
    const spy = vi.spyOn(mock, "artistPage").mockImplementation(async (id) => ({ ...(await real(id)), popularPending: true }));
    try {
      render(<ArtistPage id={a.id} />);
      await screen.findByRole("heading", { level: 1, name: a.title });
      expect(popularSection()).toHaveAttribute("aria-busy", "true");
      act(() => mock.emit("artistinfo://popular", { id: a.id }));
      await vi.waitFor(() => expect(popularSection()).not.toHaveAttribute("aria-busy"));
      const first = (await real(a.id)).popular[0];
      expect(within(popularSection()).getByRole("row", { name: new RegExp(first.title) })).toBeInTheDocument();
    } finally {
      spy.mockRestore();
    }
  });

  it("falls back to plays order when the global list is slow", async () => {
    const a = artistAt(0);
    const real = mock.artistPage.bind(mock);
    const spy = vi.spyOn(mock, "artistPage").mockImplementation(async (id) => ({ ...(await real(id)), popularPending: true }));
    try {
      render(<ArtistPage id={a.id} />);
      await screen.findByRole("heading", { level: 1, name: a.title });
      expect(popularSection()).toHaveAttribute("aria-busy", "true");
      // No word from the fetch: after about 1.5 s the plays order shows.
      await vi.waitFor(() => expect(popularSection()).not.toHaveAttribute("aria-busy"), { timeout: 3000 });
      expect(within(popularSection()).getAllByRole("row").length).toBeGreaterThan(0);
    } finally {
      spy.mockRestore();
    }
  });
});

describe("credited artist page", () => {
  const guest = () => {
    const t = mock.fx.tracks.find((x) => x.trackArtist?.includes(" feat. Juno Ashby"))!;
    return { track: t, id: "credit:juno ashby" };
  };

  it("shows the songs that credit the name and the albums they're on, without Plex-only controls", async () => {
    const user = userEvent.setup();
    const { track, id } = guest();
    render(
      <>
        <ArtistPage id={id} />
        <ContextMenuHost />
      </>,
    );
    await screen.findByRole("heading", { level: 1, name: "Juno Ashby" });
    await screen.findByRole("heading", { name: "Discography" });
    expect(headings()).toEqual(["Popular", "Discography", "Appears on"]);
    const list = screen.getByRole("grid", { name: "Popular songs" });
    expect(within(list).getAllByRole("row", { name: new RegExp(track.title) }).length).toBeGreaterThan(0);
    const appears = screen.getByRole("heading", { name: "Appears on" }).closest("section")!;
    expect(within(appears).getAllByText(track.albumTitle).length).toBeGreaterThan(0);
    await user.click(screen.getByRole("button", { name: "More options for Juno Ashby" }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: /Share/ })).toBeNull();
  });

  // A featured-only artist: no artist item on the server, only songs of
  // other artists that credit them ("feat. Juno Ashby").
  it("gives a featured-only artist a real photo and their catalogue by type, with Get", async () => {
    const user = userEvent.setup();
    const art = vi.spyOn(mock, "artUrl");
    await mock.settingsSet({ getAlbumsKinds: ["studio", "ep", "live", "compilation"], discographyChip: "popular" });
    useSettings.setState({ settings: null });
    await loadSettings();
    const { id } = guest();
    render(<ArtistPage id={id} />);
    await screen.findByRole("heading", { level: 1, name: "Juno Ashby" });
    const featured = await mock.featuredArtist(id, "Juno Ashby");
    expect(featured.photo).toBeTruthy();
    await waitFor(() => expect(art.mock.calls.map((c) => c[0])).toContain(featured.photo));
    art.mockRestore();

    const chips = await screen.findByRole("group", { name: "Release type" });
    expect(within(chips).getAllByRole("button").map((b) => b.textContent)).toEqual(["Popular releases", "Albums", "Singles and EPs", "Compilations", "Live"]);
    const shelf = screen.getByRole("heading", { name: "Discography" }).closest("section")!;
    // Popular releases: the latest release first.
    expect(within(shelf).getAllByRole("listitem")[0]).toHaveTextContent("Hollow Light");
    expect(within(shelf).getByText("Latest release · Single")).toBeInTheDocument();

    await user.click(within(chips).getByRole("button", { name: "Singles and EPs" }));
    const cards = within(shelf).getAllByRole("listitem");
    expect(cards.map((c) => c.querySelector(".card__title")?.textContent)).toEqual(["Hollow Light", "Late Ferry", "Four Walls"]);
    expect(within(cards[2]).getByText("2017 · EP")).toBeInTheDocument();
    expect(useSettings.getState().settings?.discographyChip).toBe("singlesEps");

    // Show all opens the full discography at the type.
    useRouter.getState().reset();
    await user.click(within(shelf).getByRole("button", { name: "Show all" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "discography", id, type: "singlesEps" });
    await mock.settingsSet({ getAlbumsKinds: ["studio"], discographyChip: "popular" });
  });

  it("leaves the catalogue out for a featured-only artist while offline", async () => {
    const { id } = guest();
    mock.setHealth("offline");
    try {
      expect(await mock.featuredArtist(id, "Juno Ashby")).toEqual({ shown: false, mbid: null, photo: null, photoCredit: null, sections: [] });
      expect((await mock.featuredArtistPhoto(id, "Juno Ashby")).photo).toBeNull();
    } finally {
      mock.setHealth("local");
    }
  });

  it("is not found when nothing credits the name", async () => {
    render(<ArtistPage id="credit:nobody at all" />);
    expect(await screen.findByRole("status")).toHaveTextContent("This artist isn't in your library");
  });
});
