import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Album, type ArtistPageSection, type CatAlbum, type ReleaseSection } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { ArtistPage } from "../pages/ArtistPage";
import { useRouter } from "../stores/router";
import { catalogueEntries, libraryEntries } from "../pages/artist";
import { DiscographyPage } from "../pages/DiscographyPage";
import { discographyGroups, discographyPrefs, prefsPatch, releaseLine, typeOptions, validType } from "../pages/discography";
import { body } from "./rows";
import { loadSettings, useSettings } from "../stores/settings";

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
const lib = libraryEntries(sections);

const cat = (n: string, title: string, kind: string, secondary: string[], year: number): CatAlbum => ({
  releaseGroupId: n,
  title,
  artist: "Artist",
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
const toGet = catalogueEntries([
  { kind: "albums", albums: [cat("g1", "Echo", "album", [], 2021)] },
  { kind: "compilations", albums: [cat("g2", "Foxtrot Hits", "album", ["compilation"], 2025)] },
]);

const titles = (gs: { entries: { title: string }[] }[]) => gs.map((g) => g.entries.map((e) => e.title));

describe("discography logic", () => {
  it("offers All and the release types the artist has, in the library or to get", () => {
    expect(typeOptions(lib).map((o) => o.label)).toEqual(["All", "Albums", "Singles and EPs", "Live"]);
    expect(typeOptions([...lib, ...toGet]).map((o) => o.label)).toEqual(["All", "Albums", "Singles and EPs", "Compilations", "Live"]);
    expect(validType(lib, "demos")).toBe("all");
    expect(validType(lib, "live")).toBe("live");
    expect(validType([...lib, ...toGet], "compilations")).toBe("compilations");
  });

  it("sections by type, sorted by release date newest first or by name A to Z, either way round", () => {
    const all = [...lib, ...toGet];
    expect(discographyGroups(all, "all", "date").map((g) => g.label)).toEqual(["Albums", "Singles and EPs", "Compilations", "Live"]);
    expect(titles(discographyGroups(all, "all", "date"))).toEqual([["alpha", "Echo", "Bravo"], ["Charlie"], ["Foxtrot Hits"], ["Delta"]]);
    expect(titles(discographyGroups(all, "albums", "name"))).toEqual([["alpha", "Bravo", "Echo"]]);
    expect(titles(discographyGroups(all, "albums", "date", true))).toEqual([["Bravo", "Echo", "alpha"]]);
    expect(titles(discographyGroups(all, "albums", "name", true))).toEqual([["Echo", "Bravo", "alpha"]]);
  });

  it("describes a release as type, year and song count (type and year for one to get)", () => {
    const [x] = libraryEntries([{ section: "albums", albums: [album("x", "X", "albums", 2026)] }]);
    const [y] = libraryEntries([{ section: "live", albums: [album("y", "Y", "live", null)] }]);
    expect(releaseLine(x)).toBe("Album · 2026 · 9 songs");
    expect(releaseLine(y)).toBe("Live album · 9 songs");
    expect(releaseLine(toGet[1])).toBe("Compilation album · 2025");
  });

  it("reads sort and view from Settings, and writes only what changed", () => {
    expect(discographyPrefs(null)).toEqual({ sort: "date", reverse: false, view: "list" });
    expect(discographyPrefs({ discographySort: "name", discographyReverse: true, discographyView: "grid" })).toEqual({ sort: "name", reverse: true, view: "grid" });
    expect(prefsPatch({ view: "grid" })).toEqual({ discographyView: "grid" });
    expect(prefsPatch({ reverse: false })).toEqual({ discographyReverse: false });
  });
});

describe("discography page", () => {
  beforeEach(async () => {
    await mock.settingsSet({ discographySort: "date", discographyReverse: false, discographyView: "list" });
    useSettings.setState({ settings: null });
    await loadSettings();
  });
  const artist = mock.fx.artists[0];
  const own = () => mock.fx.albums.filter((a) => a.artistId === artist.id);

  it("lists every release as a block with its tracklist, sectioned by type, newest first", async () => {
    const groups = discographyGroups(libraryEntries((await mock.artistPage(artist.id)).sections), "all", "date");
    render(<DiscographyPage id={artist.id} />);
    expect(await screen.findByRole("heading", { level: 1, name: artist.title })).toBeInTheDocument();
    const blocks = await screen.findAllByRole("region");
    expect(blocks).toHaveLength(new Set(own().map((a) => a.id)).size);
    if (groups.length > 1) expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual(groups.map((g) => g.label));
    const first = blocks[0];
    const newest = groups[0].entries[0];
    expect(first).toHaveAccessibleName(newest.title);
    expect(within(first).getByText(releaseLine(newest))).toBeInTheDocument();
    expect(within(first).getAllByRole("button", { name: `Play ${newest.title}` })[0]).toBeInTheDocument();
    const list = await within(first).findByRole("grid", { name: `${newest.title} songs` });
    await within(list).findAllByRole("row").then(body);
    expect(body(within(list).getAllByRole("row"))).toHaveLength(mock.fx.tracksByAlbum.get(newest.key)!.length);
    // Save puts the release's songs in Liked Songs.
    expect(within(first).getByRole("button", { name: `Save ${newest.title} to Liked Songs` })).toBeEnabled();
  });

  it("switches to a grid from the sort and view menu, and remembers it (and the order) on this device", async () => {
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
    expect((await screen.findAllByRole("list")).some((l) => l.querySelectorAll(".card--album").length > 0)).toBe(true);
    await waitFor(() => expect(useSettings.getState().settings?.discographyView).toBe("grid"));
    await user.click(screen.getByRole("button", { name: /^Sort and view/ }));
    await user.click(await screen.findByRole("menuitemradio", { name: "Reverse order" }));
    expect(screen.getByRole("button", { name: /^Sort and view: Release date, reversed, grid view/ })).toBeInTheDocument();
    expect((await mock.settingsGet()).discographyReverse).toBe(true);
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

describe("discography settings", () => {
  beforeEach(async () => {
    await mock.settingsSet({ ...DEFAULT_SETTINGS });
    useSettings.setState({ settings: null });
    await loadSettings();
  });

  it("starts on Popular releases, newest first, as a list, not reversed", () => {
    expect([DEFAULT_SETTINGS.discographyChip, DEFAULT_SETTINGS.discographySort, DEFAULT_SETTINGS.discographyReverse, DEFAULT_SETTINGS.discographyView]).toEqual([
      "popular",
      "date",
      false,
      "list",
    ]);
  });

  it("has a row for each, which changes what the pages remember", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const chip = await screen.findByRole("combobox", { name: "Discography opens on" });
    expect(chip).toHaveValue("popular");
    await user.selectOptions(chip, "compilations");
    await waitFor(() => expect(useSettings.getState().settings?.discographyChip).toBe("compilations"));
    await user.selectOptions(screen.getByRole("combobox", { name: "Full discography order" }), "name");
    await user.selectOptions(screen.getByRole("combobox", { name: "Full discography view" }), "grid");
    await user.click(screen.getByRole("switch", { name: "Reverse the full discography's order" }));
    await waitFor(() => expect(useSettings.getState().settings).toMatchObject({ discographySort: "name", discographyView: "grid", discographyReverse: true }));
    expect(await mock.settingsGet()).toMatchObject({ discographyChip: "compilations", discographySort: "name", discographyView: "grid", discographyReverse: true });
  });

  it("names a type picked on an artist page that the list doesn't offer", async () => {
    await mock.settingsSet({ discographyChip: "live" });
    useSettings.setState({ settings: null });
    await loadSettings();
    render(<SettingsPage />);
    const chip = await screen.findByRole("combobox", { name: "Discography opens on" });
    expect(chip).toHaveValue("live");
    expect(within(chip).getByRole("option", { name: "Live" })).toBeInTheDocument();
  });
});

describe("discography of an artist outside the library", () => {
  const COLD = "c0ffee00-0000-4000-8000-000000000001";
  beforeEach(async () => {
    await mock.settingsSet({ ...DEFAULT_SETTINGS, getAlbumsKinds: ["studio", "ep", "live", "compilation"] });
    useSettings.setState({ settings: null });
    await loadSettings();
  });

  it("shows every release the catalogue finds, sectioned by type, each with Get", async () => {
    render(<DiscographyPage id={COLD} catalogue />);
    expect(await screen.findByRole("heading", { level: 1, name: "Cold Coast" })).toBeInTheDocument();
    await screen.findAllByRole("region");
    expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual(["Albums", "Singles and EPs", "Live"]);
    expect(screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent)).toEqual(["Low Tide Radio", "Harbour Static", "First Frost", "Weatherglass", "Live at the Lido"]);
    const ep = screen.getByRole("region", { name: "Weatherglass" });
    expect(within(ep).getByText("EP · 2019")).toBeInTheDocument();
    expect(within(ep).getByRole("button", { name: "More options for Weatherglass" })).toBeInTheDocument();
  });

  it("follows the Get albums kinds", async () => {
    await mock.settingsSet({ getAlbumsKinds: ["studio"] });
    useSettings.setState({ settings: null });
    await loadSettings();
    render(<DiscographyPage id={COLD} catalogue />);
    await screen.findByRole("heading", { level: 1, name: "Cold Coast" });
    const blocks = await screen.findAllByRole("region");
    expect(blocks.map((b) => b.getAttribute("aria-label"))).toEqual(["Low Tide Radio", "Harbour Static"]);
    expect(screen.queryByRole("heading", { level: 2, name: "Albums" })).toBeNull();
  });

  it("opens from the catalogue artist page's Show all, at the chip's type", async () => {
    const user = userEvent.setup();
    useRouter.getState().reset();
    render(<ArtistPage id={COLD} catalogue />);
    const chips = await screen.findByRole("group", { name: "Release type" });
    expect(within(chips).getAllByRole("button").map((b) => b.textContent)).toEqual(["Popular releases", "Albums", "Singles and EPs", "Live"]);
    await user.click(within(chips).getByRole("button", { name: "Live" }));
    const shelf = screen.getByRole("heading", { name: "Discography" }).closest("section")!;
    await user.click(within(shelf).getByRole("button", { name: "Show all" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "discography", id: COLD, type: "live", catalogue: true });
  });
});
