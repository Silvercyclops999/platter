import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { Album, ArtistPageSection, ReleaseSection } from "../lib/types";
import { ArtistPage } from "../pages/ArtistPage";
import { useRouter } from "../stores/router";
import { artistSections, discographyChips, discographyItems, headerVariant, playsLine } from "../pages/artist";

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
    expect(discographyChips(SECTIONS).map((c) => c.label)).toEqual(["Popular releases", "Albums", "Singles & EPs", "Compilations"]);
  });

  it("puts the latest release first in popular releases, then the most played", () => {
    const items = discographyItems(SECTIONS, "popular");
    expect(items.map((i) => i.album.id)).toEqual(["s1", "a3", "a1", "s2", "a2", "c1"]);
    expect(items.map((i) => i.latest)).toEqual([true, false, false, false, false, false]);
    expect(discographyItems(SECTIONS, "popular", 3).map((i) => i.album.id)).toEqual(["s1", "a3", "a1"]);
  });

  it("lists a release type newest first, every release", () => {
    expect(discographyItems(SECTIONS, "albums").map((i) => i.album.id)).toEqual(["a2", "a1", "a3"]);
    expect(discographyItems(SECTIONS, "singlesEps").every((i) => !i.latest)).toBe(true);
    // A chip the artist doesn't have falls back to popular releases.
    expect(discographyItems(SECTIONS, "live")[0].album.id).toBe("s1");
    expect(discographyItems([], "popular")).toEqual([]);
  });

  it("orders sections and leaves out empty ones", () => {
    const base = { popular: [mock.fx.tracks[0]], sections: SECTIONS, about: "Bio", similar: [mock.fx.artists[1]] };
    expect(artistSections(base)).toEqual(["popular", "discography", "about", "fansAlsoLike"]);
    expect(artistSections({ ...base, about: "  ", similar: [] })).toEqual(["popular", "discography"]);
    expect(artistSections({ popular: [], sections: [], about: null, similar: [] })).toEqual([]);
    expect(artistSections({ ...base, appearsOn: [album("x", "albums", 2020, 0)] })).toEqual(["popular", "discography", "about", "fansAlsoLike", "appearsOn"]);
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
    const list = screen.getByRole("list", { name: "Popular tracks" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(5);
    await user.click(screen.getByRole("button", { name: "See more" }));
    expect(within(list).getAllByRole("listitem")).toHaveLength(10);
    await user.click(screen.getByRole("button", { name: "Show less" }));
    expect(within(list).getAllByRole("listitem")).toHaveLength(5);
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
    const own = mock.fx.albums.filter((x) => x.artistId === a.id && x.section === "albums");
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
    const list = screen.getByRole("list", { name: "Popular tracks" });
    const real = mock.artistPage.bind(mock);
    const hit = mock.fx.tracks.find((t) => t.artistId !== a.id)!;
    const spy = vi.spyOn(mock, "artistPage").mockImplementation(async (id) => ({ ...(await real(id)), popular: [{ ...hit, title: "Global Number One" }] }));
    try {
      act(() => mock.emit("artistinfo://changed", { id: a.id }));
      expect(await screen.findByRole("listitem", { name: /Global Number One/ })).toBeInTheDocument();
      expect(list).toBeInTheDocument();
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
    expect(headings()).toEqual(["Popular", "Appears on"]);
    const list = screen.getByRole("list", { name: "Popular tracks" });
    expect(within(list).getAllByRole("listitem", { name: new RegExp(track.title) }).length).toBeGreaterThan(0);
    const appears = screen.getByRole("heading", { name: "Appears on" }).closest("section")!;
    expect(within(appears).getAllByText(track.albumTitle).length).toBeGreaterThan(0);
    await user.click(screen.getByRole("button", { name: "More options for Juno Ashby" }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: /Share/ })).toBeNull();
  });

  it("is not found when nothing credits the name", async () => {
    render(<ArtistPage id="credit:nobody at all" />);
    expect(await screen.findByText(/Couldn't load this artist/)).toBeInTheDocument();
  });
});
