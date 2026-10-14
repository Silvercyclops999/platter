import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { Album, ArtistPageSection, ReleaseSection } from "../lib/types";
import { DiscographyPage } from "../pages/DiscographyPage";
import {
  discographyReleases,
  loadDiscographyPrefs,
  releaseLine,
  saveDiscographyPrefs,
  typeOptions,
  validType,
} from "../pages/discography";
import { body } from "./rows";

const mock = backend as MockBackend;

const album = (id: string, title: string, section: ReleaseSection, year: number | null, addedAt = 0): Album => ({
  id,
  title,
  sortTitle: title,
  artistId: "a",
  artistTitle: "Artist",
  year,
  thumb: null,
  format: section === "singlesEps" ? "single" : "album",
  subformats: section === "live" ? ["live"] : [],
  section,
  genres: [],
  trackCount: 9,
  addedAt,
  viewCount: 0,
  lastViewedAt: null,
  tint: null,
});

const sections: ArtistPageSection[] = [
  { section: "albums", albums: [album("1", "Bravo", "albums", 2019), album("2", "alpha", "albums", 2023)] },
  { section: "singlesEps", albums: [album("3", "Charlie", "singlesEps", 2024)] },
  { section: "live", albums: [album("4", "Delta", "live", null)] },
  { section: "demos", albums: [] },
];

describe("discography logic", () => {
  it("offers All and the release types the artist has", () => {
    expect(typeOptions(sections).map((o) => o.label)).toEqual(["All", "Albums", "Singles & EPs", "Live"]);
    expect(validType(sections, "demos")).toBe("all");
    expect(validType(sections, "live")).toBe("live");
  });

  it("sorts by release date newest first, or by name A to Z", () => {
    expect(discographyReleases(sections, "all", "date").map((a) => a.title)).toEqual(["Charlie", "alpha", "Bravo", "Delta"]);
    expect(discographyReleases(sections, "all", "name").map((a) => a.title)).toEqual(["alpha", "Bravo", "Charlie", "Delta"]);
    expect(discographyReleases(sections, "albums", "date").map((a) => a.title)).toEqual(["alpha", "Bravo"]);
  });

  it("describes a release as type, year and song count", () => {
    expect(releaseLine(album("x", "X", "albums", 2026))).toBe("Album · 2026 · 9 songs");
    expect(releaseLine(album("x", "X", "live", null))).toBe("Live album · 9 songs");
  });

  it("remembers sort and view on this device", () => {
    saveDiscographyPrefs({ sort: "name", view: "grid" });
    expect(loadDiscographyPrefs()).toEqual({ sort: "name", view: "grid" });
    localStorage.setItem("platter.discography.v1", "{bad");
    expect(loadDiscographyPrefs()).toEqual({ sort: "date", view: "list" });
  });
});

describe("discography page", () => {
  beforeEach(() => localStorage.removeItem("platter.discography.v1"));
  const artist = mock.fx.artists[0];
  const own = () => mock.fx.albums.filter((a) => a.artistId === artist.id);

  it("lists every release as a block with its tracklist, newest first", async () => {
    render(<DiscographyPage id={artist.id} />);
    expect(await screen.findByRole("heading", { level: 1, name: artist.title })).toBeInTheDocument();
    const blocks = await screen.findAllByRole("region");
    expect(blocks).toHaveLength(new Set(own().map((a) => a.id)).size);
    const first = blocks[0];
    const newest = [...own()].sort((a, b) => (b.year ?? -1) - (a.year ?? -1) || b.addedAt - a.addedAt)[0];
    expect(first).toHaveAccessibleName(newest.title);
    expect(within(first).getByText(releaseLine(newest))).toBeInTheDocument();
    expect(within(first).getAllByRole("button", { name: `Play ${newest.title}` })[0]).toBeInTheDocument();
    const list = await within(first).findByRole("grid", { name: `${newest.title} tracks` });
    await within(list).findAllByRole("row").then(body);
    expect(body(within(list).getAllByRole("row"))).toHaveLength(mock.fx.tracksByAlbum.get(newest.id)!.length);
  });

  it("switches to a grid from the sort and view menu, and remembers it", async () => {
    const user = userEvent.setup();
    render(
      <>
        <DiscographyPage id={artist.id} />
        <ContextMenuHost />
      </>,
    );
    await screen.findByRole("heading", { level: 1, name: artist.title });
    await user.click(screen.getByRole("button", { name: /^Sort and view/ }));
    await user.click(await screen.findByRole("menuitemradio", { name: "Grid" }));
    const grid = await screen.findByRole("list", { name: "Releases" });
    expect(grid.querySelectorAll(".card--album").length).toBeGreaterThan(0);
    expect(loadDiscographyPrefs().view).toBe("grid");
  });

  it("filters by release type", async () => {
    const user = userEvent.setup();
    const page = await mock.artistPage(artist.id);
    const kind = page.sections.find((s) => s.albums.length)!;
    render(
      <>
        <DiscographyPage id={artist.id} />
        <ContextMenuHost />
      </>,
    );
    await screen.findByRole("heading", { level: 1, name: artist.title });
    await user.click(screen.getByRole("button", { name: "Release type: All" }));
    const items = await screen.findAllByRole("menuitemradio");
    expect(items[0]).toHaveTextContent("All");
    await user.click(items[1]);
    expect(screen.getByRole("button", { name: /^Release type: / })).not.toHaveAccessibleName("Release type: All");
    expect(await screen.findAllByRole("region")).toHaveLength(kind.albums.length);
  });
});
