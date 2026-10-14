import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { Album, ArtistPageSection, ReleaseSection } from "../lib/types";
import { ArtistPage } from "../pages/ArtistPage";
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
    expect(headings()).toEqual(["Popular", "Discography", "About", "Fans also like"]);
  });

  it("shows the round photo header when there's no background art", async () => {
    const a = artistAt(1);
    const { container } = render(<ArtistPage id={a.id} />);
    await screen.findByRole("heading", { level: 1, name: a.title });
    expect(container.querySelector("header[data-variant]")).toHaveAttribute("data-variant", "photo");
    expect(container.querySelector(".detail-header__art .art--round")).not.toBeNull();
  });

  it("hides About and Fans also like when Plex has neither", async () => {
    const a = artistAt(11);
    render(<ArtistPage id={a.id} />);
    await screen.findByRole("heading", { level: 1, name: a.title });
    expect(headings()).toEqual(["Popular", "Discography"]);
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

  it("expands the discography into a grid with Show all, and back", async () => {
    const user = userEvent.setup();
    const a = artistAt(0);
    render(<ArtistPage id={a.id} />);
    await screen.findByRole("group", { name: "Release type" });
    await user.click(screen.getByRole("button", { name: "Show all" }));
    const grid = document.querySelector(".discography__grid")!;
    expect(grid.querySelectorAll('[role="listitem"]')).toHaveLength(mock.fx.albums.filter((x) => x.artistId === a.id).length);
    await user.click(screen.getByRole("button", { name: "Show less" }));
    expect(document.querySelector(".discography__grid")).toBeNull();
  });

  it("lists similar artists from the library under Fans also like", async () => {
    const a = artistAt(0);
    render(<ArtistPage id={a.id} />);
    const h = await screen.findByRole("heading", { name: "Fans also like" });
    const shelf = h.closest("section")!;
    expect(within(shelf).getByText(artistAt(1).title)).toBeInTheDocument();
    expect(within(shelf).getByText(artistAt(2).title)).toBeInTheDocument();
  });
});
