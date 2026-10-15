import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { ArchiveDialogs } from "../components/ArchiveDialogs";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { ARCHIVE_ARTIST_ID, mockFoldTitle } from "../lib/mock/archive";
import type { ArtistPage as ArtistPageData, Era, Track } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { cleanKeyword } from "../pages/ArchiveSettings";
import { ArtistPage } from "../pages/ArtistPage";
import { EraPage } from "../pages/EraPage";
import { eraRange, filterByEra, formatArchiveDate, otherVersionsText, showEraCards, showVersionLink, versionLinkText } from "../pages/era";
import { releaseLabel, strengthLine } from "../pages/ReleaseStatusLabel";
import { SearchPage } from "../pages/SearchPage";
import { SettingsPage } from "../pages/SettingsPage";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";

const mock = backend as MockBackend;
const albumNamed = (title: string) => mock.fx.albums.find((a) => a.title === title && a.artistId === ARCHIVE_ARTIST_ID)!;
const trackNamed = (title: string) => mock.fx.tracks.find((t) => t.title === title && t.artistId === ARCHIVE_ARTIST_ID)!;

beforeEach(async () => {
  mock.archive.marks.clear();
  mock.archive.custom.clear();
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
  useRouter.getState().reset();
});

// ---------------------------------------------------------------- pure decisions

describe("era and version helpers", () => {
  it("formats ARCHIVE_DATE in the pinned forms", () => {
    expect(formatArchiveDate("2019")).toBe("2019");
    expect(formatArchiveDate("2019-02")).toBe("Feb 2019");
    expect(formatArchiveDate("2018-11-02")).toBe("Nov 2, 2018");
    expect(formatArchiveDate("Summer 2019")).toBe("Summer 2019");
    expect(eraRange({ start: "2018-11-02", end: "2019-02" })).toBe("Nov 2018 – Feb 2019");
    expect(eraRange({ start: "2021", end: "2021" })).toBe("2021");
    expect(eraRange({ start: null, end: null })).toBeNull();
  });

  it("era cards need two eras and the setting", () => {
    const e = { name: "A" } as Era;
    expect(showEraCards([e, e], undefined)).toBe(true);
    expect(showEraCards([e, e], false)).toBe(false);
    expect(showEraCards([e], true)).toBe(false);
    expect(showEraCards(undefined, true)).toBe(false);
  });

  it("the era filter covers Popular, every section and Appears on", () => {
    const t = (id: string) => ({ id }) as Track;
    const a = (id: string) => ({ id }) as ArtistPageData["appearsOn"][number];
    const page = {
      popular: [t("1"), t("2")],
      sections: [
        { section: "albums", albums: [a("x"), a("y")] },
        { section: "unreleased", albums: [a("z")] },
        { section: "live", albums: [a("w")] },
      ],
      appearsOn: [a("q"), a("x")],
    } as unknown as ArtistPageData;
    const era = { albumIds: ["x", "z"], trackIds: ["2"] } as Era;
    const out = filterByEra(page, era);
    expect(out.popular.map((x) => x.id)).toEqual(["2"]);
    expect(out.sections.map((s) => [s.section, s.albums.map((x) => x.id)])).toEqual([
      ["albums", ["x"]],
      ["unreleased", ["z"]],
    ]);
    expect(out.appearsOn.map((x) => x.id)).toEqual(["x"]);
    expect(filterByEra(page, null)).toBe(page);
  });

  it("+N versions shows on released songs with others, while the setting is on", () => {
    expect(showVersionLink({ otherVersions: 2, songStatus: "released" }, undefined)).toBe(true);
    expect(showVersionLink({ otherVersions: 2, songStatus: "released" }, false)).toBe(false);
    expect(showVersionLink({ otherVersions: 2, songStatus: "probablyAlternate" }, true)).toBe(false);
    expect(showVersionLink({ otherVersions: 0, songStatus: "released" }, true)).toBe(false);
    expect(versionLinkText(1)).toBe("+1 version");
    expect(versionLinkText(3)).toBe("+3 versions");
    expect(otherVersionsText(1)).toBe("1 other version");
  });

  it("labels and evidence strength", () => {
    expect(releaseLabel({ status: "probablyUnreleased", mark: null })).toBe("Probably unreleased");
    expect(releaseLabel({ status: "confirmedUnreleased", mark: "unreleased" })).toBe("Unreleased");
    expect(releaseLabel({ status: "released", mark: "released" })).toBe("Marked released");
    expect(releaseLabel({ status: "released", mark: null })).toBeNull();
    expect(releaseLabel({ status: "taggedUnreleased", mark: null })).toBeNull();
    expect(strengthLine(95)).toBe("Strong evidence");
    expect(strengthLine(40)).toBe("Some evidence");
    expect(strengthLine(0)).toBeNull();
  });

  it("the mock folds version notes like the core", () => {
    for (const t of ["Glasshouse (Demo)", "Glasshouse (v2)", "Glasshouse - Snippet", "Glasshouse (Piano Demo)", "Glasshouse (Live)"]) {
      expect(mockFoldTitle(t)).toBe("glasshouse");
    }
    expect(mockFoldTitle("Halflight (2023)")).toBe("halflight");
    expect(mockFoldTitle("Song Part II")).not.toBe(mockFoldTitle("Song Part 2"));
  });

  it("folder keywords are trimmed, unique and slash-free", () => {
    expect(cleanKeyword(" Vault ", ["Leaks"])).toBe("Vault");
    expect(cleanKeyword("leaks", ["Leaks"])).toBeNull();
    expect(cleanKeyword("a/b", [])).toBeNull();
    expect(cleanKeyword("", [])).toBeNull();
    expect(cleanKeyword("x".repeat(41), [])).toBeNull();
  });
});

// ---------------------------------------------------------------- release status with marks

describe("Why? with marks", () => {
  it("lists every piece of evidence, hints marked as adding weight only", async () => {
    const user = userEvent.setup();
    const leaks = albumNamed("Paper Kingdom");
    render(<AlbumPage id={leaks.id} />);
    await user.click(await screen.findByRole("button", { name: "Why probably unreleased?" }));
    const pop = screen.getByRole("dialog", { name: "Probably unreleased" });
    expect(within(pop).getByText("Plex couldn't match it to a known release.")).toBeInTheDocument();
    expect(within(pop).getByText(/Folder name: Leaks/)).toBeInTheDocument();
    expect(within(pop).getByText(/Most of its songs are under a minute long/)).toBeInTheDocument();
    expect(within(pop).getAllByText("Adds weight only")).toHaveLength(2);
    expect(within(pop).getByText("Strong evidence")).toBeInTheDocument();
  });

  it("Confirm marks it unreleased, and the mark can be cleared", async () => {
    const user = userEvent.setup();
    const leaks = albumNamed("Paper Kingdom");
    render(<AlbumPage id={leaks.id} />);
    await user.click(await screen.findByRole("button", { name: "Why probably unreleased?" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Confirm" }));
    expect(await screen.findByRole("button", { name: "Why unreleased?" })).toBeInTheDocument();
    expect((await backend.albumReleaseStatus(leaks.id)).status).toBe("confirmedUnreleased");
    // The popover stays open on the new answer.
    const pop = screen.getByRole("dialog", { name: "Unreleased" });
    expect(within(pop).getByText("You confirmed it's unreleased.")).toBeInTheDocument();
    await user.click(within(pop).getByRole("button", { name: "Clear my mark" }));
    expect(await screen.findByRole("button", { name: "Why probably unreleased?" })).toBeInTheDocument();
  });

  it("Not unreleased marks it released, and everything follows", async () => {
    const user = userEvent.setup();
    const leaks = albumNamed("Paper Kingdom");
    expect((await backend.shareInfo("album", leaks.id)).public).toBe("unreleased");
    render(<AlbumPage id={leaks.id} />);
    await user.click(await screen.findByRole("button", { name: "Why probably unreleased?" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Not unreleased" }));
    expect(await screen.findByRole("button", { name: "Why marked released?" })).toBeInTheDocument();
    // The artist page moves it out of Unreleased.
    const page = await backend.artistPage(ARCHIVE_ARTIST_ID);
    const unreleased = page.sections.find((s) => s.section === "unreleased")!;
    expect(unreleased.albums.some((a) => a.id === leaks.id)).toBe(false);
  });

  it("a file tag beats the mark, and says so", async () => {
    const sessions = albumNamed("Night Ferry Sessions");
    await backend.releaseMarkSet("album", sessions.id, "released");
    const s = await backend.albumReleaseStatus(sessions.id);
    expect(s.status).toBe("taggedUnreleased");
    expect(s.evidence?.some((e) => e.kind === "markOverridden")).toBe(true);
  });
});

// ---------------------------------------------------------------- other versions

describe("Other versions", () => {
  it("released songs show +N versions, which opens the panel", async () => {
    const user = userEvent.setup();
    const lp = albumNamed("Glass Season");
    render(
      <>
        <AlbumPage id={lp.id} />
        <ArchiveDialogs />
      </>,
    );
    const link = await screen.findByRole("button", { name: "+6 versions of Glasshouse" });
    await user.click(link);
    const dialog = await screen.findByRole("dialog", { name: "Versions of Glasshouse" });
    await within(dialog).findByText("6 other versions in your library");
    const rows = within(dialog).getAllByRole("listitem");
    expect(rows).toHaveLength(7);
    // Released versions first, newest first: the remaster (2025), the live one (2022), then the album (2019).
    expect(within(rows[0]).getByText("Glasshouse (2025 Remaster)")).toBeInTheDocument();
    expect(within(rows[1]).getByText("Glasshouse (Live)")).toBeInTheDocument();
    expect(within(rows[2]).getByText("Glasshouse")).toBeInTheDocument();
    expect(rows[2]).toHaveAttribute("aria-current", "true");
    expect(within(rows[2]).getByText("This one")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Play Glasshouse (Demo)" })).toBeInTheDocument();
    expect(within(dialog).getAllByText("Demo").length).toBeGreaterThan(0);
  });

  it("the song menu says how many, and the setting hides the row link", async () => {
    const user = userEvent.setup();
    const lp = albumNamed("Glass Season");
    const { unmount } = render(
      <>
        <AlbumPage id={lp.id} />
        <ContextMenuHost />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: /^More options for Ferryman by/ }));
    expect(await screen.findByRole("menuitem", { name: "Other versions (2)" })).toBeInTheDocument();
    unmount();
    await backend.settingsSet({ showVersionCounts: false });
    await loadSettings();
    render(<AlbumPage id={lp.id} />);
    await screen.findByRole("heading", { name: "Glass Season" });
    await new Promise((r) => setTimeout(r, 30));
    expect(screen.queryByRole("button", { name: /versions of Glasshouse/ })).toBeNull();
  });

  it("unreleased versions don't get the link, but know their status", async () => {
    const page = await backend.albumPage(albumNamed("Glass Season (Build v1)").id);
    const demo = page.tracks.find((t) => t.title === "Glasshouse (Demo)")!;
    expect(demo.songStatus).toBe("unreleased");
    expect(showVersionLink(demo, true)).toBe(false);
    const halflight = (await backend.albumPage(albumNamed("Paper Kingdom").id)).tracks.find((t) => t.title === "Halflight (2023)")!;
    expect(halflight.songStatus).toBe("probablyAlternate");
    const crown = (await backend.albumPage(albumNamed("Paper Kingdom").id)).tracks.find((t) => t.title === "Crown of Paper")!;
    expect(crown.songStatus).toBe("probablyUnreleased");
  });
});

// ---------------------------------------------------------------- search

describe("search groups versions", () => {
  it("shows the released version first, with its other versions under it", async () => {
    const res = await backend.search({ query: "glasshouse", filter: "songs", limit: 30 });
    expect(res.songs.filter((t) => t.artistId === ARCHIVE_ARTIST_ID).map((t) => t.title)).toEqual(["Glasshouse"]);
    expect(res.versions?.[trackNamed("Glasshouse").id]).toHaveLength(6);
  });

  it("the released one leads even when a demo is the better hit", async () => {
    const res = await backend.search({ query: "glasshouse (demo)", filter: "all", limit: 30 });
    const first = res.songs.find((t) => t.artistId === ARCHIVE_ARTIST_ID);
    expect(first?.title).toBe("Glasshouse");
    if (res.top?.kind === "track") expect(res.top.track?.songStatus).toBe("released");
  });

  it("N other versions expands in place", async () => {
    const user = userEvent.setup();
    render(<SearchPage query="ferryman" />);
    await user.click(await screen.findByRole("button", { name: "Songs" }));
    const toggle = await screen.findByRole("button", { name: /2 other versions/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(await screen.findByRole("row", { name: /^Ferryman \(Session\)/ })).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------- artist page

describe("artist page sections and eras", () => {
  it("sections follow release type and status", async () => {
    render(<ArtistPage id={ARCHIVE_ARTIST_ID} />);
    const chips = await screen.findByRole("group", { name: "Release type" });
    const names = within(chips)
      .getAllByRole("button")
      .map((b) => b.textContent);
    expect(names).toEqual(["Popular releases", "Albums", "Singles & EPs", "Mixtapes", "Unreleased", "Live", "Demos"]);
    const page = await backend.artistPage(ARCHIVE_ARTIST_ID);
    const ids = (s: string) => page.sections.find((x) => x.section === s)!.albums.map((a) => a.title).sort();
    expect(ids("demos")).toEqual(["Basement Takes"]);
    expect(ids("unreleased")).toEqual(["Glass Season (Build v1)", "Glass Season (Build v2)", "Loose Files", "Night Ferry Sessions", "Paper Kingdom"]);
  });

  it("an album moved to a section of your own gets its chip", async () => {
    const lp = albumNamed("Live at the Lantern");
    await backend.albumSectionSet(lp.id, ARCHIVE_ARTIST_ID, "Road Tapes");
    expect(await backend.artistSections(ARCHIVE_ARTIST_ID)).toEqual(["Road Tapes"]);
    render(<ArtistPage id={ARCHIVE_ARTIST_ID} />);
    const chips = await screen.findByRole("group", { name: "Release type" });
    expect(within(chips).getByRole("button", { name: "Road Tapes" })).toBeInTheDocument();
    expect(within(chips).queryByRole("button", { name: "Live" })).toBeNull();
    await backend.albumSectionSet(lp.id, ARCHIVE_ARTIST_ID, null);
    expect(await backend.artistSections(ARCHIVE_ARTIST_ID)).toEqual([]);
  });

  it("era cards sit under Popular, oldest first, and filter the page", async () => {
    const user = userEvent.setup();
    const { rerender } = render(<ArtistPage id={ARCHIVE_ARTIST_ID} />);
    const shelf = (await screen.findByRole("heading", { name: "Eras" })).closest("section")!;
    const titles = within(shelf)
      .getAllByRole("button", { name: /^(Glass Season|Night Ferry|Paper Kingdom)$/ })
      .map((b) => b.textContent);
    expect(titles).toEqual(["Glass Season", "Night Ferry", "Paper Kingdom"]);
    expect(within(shelf).getByText(/Sep 2018 – Feb 2019/)).toBeInTheDocument();
    // Popular comes first.
    const headings = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    expect(headings.indexOf("Popular")).toBeLessThan(headings.indexOf("Eras"));
    await user.click(within(shelf).getByRole("button", { name: "Night Ferry" }));
    const route = useRouter.getState().entries[useRouter.getState().index].route;
    expect(route).toEqual({ name: "artist", id: ARCHIVE_ARTIST_ID, era: "Night Ferry" });
    rerender(<ArtistPage id={ARCHIVE_ARTIST_ID} era="Night Ferry" />);
    expect(await screen.findByRole("button", { name: "Era: Night Ferry. Show everything" })).toBeInTheDocument();
    const chips = screen.getByRole("group", { name: "Release type" });
    expect(within(chips).queryByRole("button", { name: "Live" })).toBeNull();
    expect(within(chips).getByRole("button", { name: "Mixtapes" })).toBeInTheDocument();
    // Back clears it.
    useRouter.getState().back();
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "home" });
  });

  it("the setting hides era cards", async () => {
    await backend.settingsSet({ showEraCards: false });
    await loadSettings();
    render(<ArtistPage id={ARCHIVE_ARTIST_ID} />);
    await screen.findByRole("group", { name: "Release type" });
    expect(screen.queryByRole("heading", { name: "Eras" })).toBeNull();
  });

  it("the era page lists builds newest first", async () => {
    render(<EraPage artistId={ARCHIVE_ARTIST_ID} name="Glass Season" />);
    expect(await screen.findByRole("heading", { level: 1, name: "Glass Season" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getAllByRole("heading", { level: 2 }).length).toBe(3));
    const groups = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    expect(groups).toEqual(["Glass Season (Build v2)", "Glass Season", "Glass Season (Build v1)"]);
    expect(screen.getByRole("button", { name: "Play Glass Season" })).toBeInTheDocument();
  });

  it("an unreleased build gets its era banner", async () => {
    render(<AlbumPage id={albumNamed("Glass Season (Build v2)").id} />);
    const banner = await screen.findByRole("complementary", { name: "From the archive" });
    expect(within(banner).getByRole("button", { name: "Glass Season era" })).toBeInTheDocument();
    expect(within(banner).getByText("Feb 2019")).toBeInTheDocument();
    expect(within(banner).getByText("Build v2")).toBeInTheDocument();
    expect(within(banner).getByText(/Source: Late build/)).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------- settings

describe("archive settings", () => {
  it("rows sit under Library, Archive, and change the settings", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const eras = await screen.findByRole("switch", { name: "Show era cards on artist pages" });
    const versions = screen.getByRole("switch", { name: "Show other versions on songs" });
    expect(eras).toBeChecked();
    expect(versions).toBeChecked();
    await user.click(eras);
    expect((await backend.settingsGet()).showEraCards).toBe(false);
    await user.click(versions);
    expect((await backend.settingsGet()).showVersionCounts).toBe(false);
  });

  it("folder names can be added and removed", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const list = await screen.findByRole("list", { name: "Folder names" });
    expect(within(list).getAllByRole("listitem").map((l) => l.textContent)).toEqual(["Unreleased", "Leaks", "Demos", "Sessions", "Snippets"]);
    await user.type(screen.getByPlaceholderText("A folder name, like Vault"), "Vault{Enter}");
    await waitFor(async () => expect((await backend.settingsGet()).archiveFolderKeywords).toContain("Vault"));
    await user.click(await screen.findByRole("button", { name: "Remove Leaks" }));
    await waitFor(async () => expect((await backend.settingsGet()).archiveFolderKeywords).not.toContain("Leaks"));
    // Without the keyword, the leaks album loses its folder hint.
    const s = await backend.albumReleaseStatus(albumNamed("Paper Kingdom").id);
    expect(s.evidence?.some((e) => e.kind === "folder")).toBe(false);
  });
});
