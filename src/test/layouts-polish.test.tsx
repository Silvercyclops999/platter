/**
 * The polish pass over the layouts: the library's filter chips and whether
 * they're remembered, the typographic sleeve for records without a cover,
 * empty states that say what to do, Turntable's up-next sleeves, Gatefold
 * remembering lyrics or credits, Console letting go of a picked song, and
 * Index reaching Radio and mixes and the archive.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Art, SLEEVE_ARTIST_FROM, sleeveHue } from "../components/Art";
import { ContextMenuHost } from "../components/ContextMenu";
import { CLASSIC_LAYOUT, type Layout } from "../design/themes/validate";
import { backend } from "../lib/backend";
import {
  CHIP_PREFS_KEY,
  NO_CHIPS,
  RECENT_DAYS,
  albumFacts,
  anyMatches,
  chipOptions,
  matchesChips,
  parseChipPrefs,
  pressFacet,
  releaseType,
  trackQuality,
  useLibraryChips,
  type ChipFacts,
} from "../lib/libraryChips";
import { albumQuality, type MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type LibraryItem, type QueueItem, type QueueView, type Settings, type Track } from "../lib/types";
import { LibraryPage } from "../pages/LibraryPage";
import { MixesPage } from "../pages/MixesPage";
import { SettingsPage } from "../pages/SettingsPage";
import { AppShell } from "../shell/AppShell";
import { resetConsole, useConsole } from "../shell/ConsoleFrame";
import { facetEntries, facetSongs, useAlbumFacts } from "../shell/FacetChips";
import { LEAF_TAB_KEY, loadLeafTab } from "../shell/GatefoldFrame";
import { upNextSleeves } from "../shell/TurntableFrame";
import { useLibraryView, type LibEntry } from "../stores/library";
import { player, usePlayer } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { renderHook } from "@testing-library/react";
import gatefoldCss from "../shell/gatefold.css?raw";
import framesCss from "../shell/frames.css?raw";
import { FRAME_RAIL, GATEFOLD_SPINE } from "../shell/frames";

const mock = backend as MockBackend;
const NOW = 2_000_000_000;
const DAY = 86_400;

const facts = (over: Partial<ChipFacts> = {}): ChipFacts => ({ type: "albums", quality: "lossless", year: 1994, addedAt: NOW - 400 * DAY, ...over });

async function settings(p: Partial<Settings> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...p });
  await loadSettings();
}

const frame = (f: Layout["frame"]): Layout => ({ ...structuredClone(CLASSIC_LAYOUT), frame: f });

function resetChips() {
  localStorage.removeItem(CHIP_PREFS_KEY);
  useLibraryChips.getState().reload();
}

beforeEach(() => {
  resetChips();
  useRouter.getState().reset();
});
afterEach(() => {
  resetChips();
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, menu: null });
});

describe("filter chips: the rules", () => {
  it("each group narrows on its own and they combine", () => {
    const hi = facts({ quality: "hiRes", year: 2003 });
    const lossy = facts({ quality: "lossy" });
    expect(matchesChips(hi, NO_CHIPS, NOW)).toBe(true);
    // Lossless takes hi-res files too; hi-res takes only those.
    expect(matchesChips(hi, { ...NO_CHIPS, format: "lossless" }, NOW)).toBe(true);
    expect(matchesChips(facts(), { ...NO_CHIPS, format: "hiRes" }, NOW)).toBe(false);
    expect(matchesChips(lossy, { ...NO_CHIPS, format: "lossless" }, NOW)).toBe(false);
    expect(matchesChips(hi, { ...NO_CHIPS, decade: 2000 }, NOW)).toBe(true);
    expect(matchesChips(hi, { ...NO_CHIPS, decade: 1990 }, NOW)).toBe(false);
    expect(matchesChips(facts({ year: null }), { ...NO_CHIPS, decade: 1990 }, NOW)).toBe(false);
    expect(matchesChips(facts({ type: "live" }), { ...NO_CHIPS, type: "albums" }, NOW)).toBe(false);
    // Added recently: within the last RECENT_DAYS days.
    expect(matchesChips(facts({ addedAt: NOW - (RECENT_DAYS - 1) * DAY }), { ...NO_CHIPS, recent: true }, NOW)).toBe(true);
    expect(matchesChips(facts({ addedAt: NOW - (RECENT_DAYS + 1) * DAY }), { ...NO_CHIPS, recent: true }, NOW)).toBe(false);
    expect(matchesChips(hi, { type: "albums", format: "hiRes", decade: 2000, recent: false }, NOW)).toBe(true);
    expect(matchesChips(hi, { type: "albums", format: "hiRes", decade: 1990, recent: false }, NOW)).toBe(false);
  });

  it("an artist matches when one album matches every chip", () => {
    const albums = [facts({ quality: "hiRes", year: 1985 }), facts({ quality: "lossy", year: 2001 })];
    expect(anyMatches(albums, { ...NO_CHIPS, format: "hiRes" }, NOW)).toBe(true);
    // Both chips on one album, not one chip each on different albums.
    expect(anyMatches(albums, { ...NO_CHIPS, format: "hiRes", decade: 2000 }, NOW)).toBe(false);
    expect(anyMatches([], NO_CHIPS, NOW)).toBe(true);
  });

  it("offers only chips that leave something to see", () => {
    const o = chipOptions([facts({ year: 1994 }), facts({ year: 2012, quality: "lossy", addedAt: NOW - DAY })], NOW);
    expect(o.formats).toEqual(["lossless"]);
    expect(o.decades).toEqual([2010, 1990]);
    expect(o.recent).toBe(true);
    // One kind of release: no type chip.
    expect(o.types).toEqual([]);
    expect(chipOptions([facts(), facts({ type: "archive" })], NOW).types).toEqual(["albums", "archive"]);
    // One decade only: no decade chip.
    expect(chipOptions([facts(), facts()], NOW).decades).toEqual([]);
  });

  it("pressing the chosen chip clears its group", () => {
    const on = pressFacet(NO_CHIPS, "format", "hiRes");
    expect(on.format).toBe("hiRes");
    expect(pressFacet(on, "format", "hiRes").format).toBeNull();
    expect(pressFacet(on, "format", "lossless").format).toBe("lossless");
    expect(pressFacet(NO_CHIPS, "recent", true).recent).toBe(true);
    expect(pressFacet({ ...NO_CHIPS, recent: true }, "recent", true).recent).toBe(false);
  });

  it("reads the kind of release and the files", () => {
    expect(releaseType("singlesEps")).toBe("singlesEps");
    expect(releaseType("soundtracks")).toBe("compilations");
    expect(releaseType("demos")).toBe("archive");
    expect(releaseType("mixtapes")).toBe("archive");
    expect(releaseType(null, "ep")).toBe("singlesEps");
    expect(releaseType(undefined, null)).toBe("albums");
    expect(trackQuality({ codec: "FLAC", bitDepth: 16, sampleRate: 44_100 })).toBe("lossless");
    expect(trackQuality({ codec: "alac", bitDepth: 24, sampleRate: 48_000 })).toBe("hiRes");
    expect(trackQuality({ codec: "flac", bitDepth: 16, sampleRate: 96_000 })).toBe("hiRes");
    expect(trackQuality({ codec: "mp3", bitDepth: null, sampleRate: 44_100 })).toBe("lossy");
    expect(trackQuality({ codec: null, bitDepth: null, sampleRate: null })).toBeNull();
    // The mock works an album out like the core: any lossy song makes it lossy.
    expect(albumQuality([{ codec: "flac", bitDepth: 24, sampleRate: 96_000 }, { codec: "aac", bitDepth: null, sampleRate: 44_100 }])).toBe("lossy");
    expect(albumQuality([{ codec: "flac", bitDepth: 16, sampleRate: 44_100 }])).toBe("lossless");
    expect(albumQuality([])).toBeNull();
    const item = { section: "live", format: "album", quality: "hiRes", year: 1977, addedAt: 5 } as LibraryItem;
    expect(albumFacts(item)).toEqual({ type: "live", quality: "hiRes", year: 1977, addedAt: 5 });
  });

  it("narrows library rows and songs", () => {
    const items = [
      { kind: "album", id: "a1", creatorId: "r1", quality: "hiRes", year: 2015, section: "albums", addedAt: 1, title: "A1", sortTitle: "A1", smart: false },
      { kind: "album", id: "a2", creatorId: "r2", quality: "lossy", year: 1999, section: "albums", addedAt: 1, title: "A2", sortTitle: "A2", smart: false },
    ] as LibraryItem[];
    const { result } = renderHook(() => useAlbumFacts(items));
    const row = (kind: LibEntry["kind"], id: string) => ({ kind, id }) as LibEntry;
    const rows = [row("album", "a1"), row("album", "a2"), row("artist", "r1"), row("artist", "r2"), row("playlist", "p")];
    expect(facetEntries(rows, NO_CHIPS, result.current)).toHaveLength(5);
    expect(facetEntries(rows, { ...NO_CHIPS, format: "hiRes" }, result.current).map((r) => r.id)).toEqual(["a1", "r1"]);
    const songs = [
      { albumId: "a1", codec: "flac", bitDepth: 24, sampleRate: 96_000, id: "s1" },
      { albumId: "a2", codec: "mp3", bitDepth: null, sampleRate: 44_100, id: "s2" },
    ];
    expect(facetSongs(songs, { ...NO_CHIPS, decade: 1990 }, result.current.byAlbum).map((s) => s.id)).toEqual(["s2"]);
    expect(facetSongs(songs, { ...NO_CHIPS, format: "lossless" }, result.current.byAlbum).map((s) => s.id)).toEqual(["s1"]);
  });
});

describe("remembering the chips (this device)", () => {
  it("keeps the last chips per view across a restart, and forgets them when off", () => {
    useLibraryChips.getState().set("albums", { ...NO_CHIPS, format: "hiRes" });
    useLibraryChips.getState().set("songs", { ...NO_CHIPS, decade: 1990 });
    // A restart reads them back.
    useLibraryChips.setState({ chips: parseChipPrefs(null).chips });
    useLibraryChips.getState().reload();
    expect(useLibraryChips.getState().chips.albums.format).toBe("hiRes");
    expect(useLibraryChips.getState().chips.songs.decade).toBe(1990);
    expect(useLibraryChips.getState().chips.artists).toEqual(NO_CHIPS);

    // Off: this session keeps them, the next starts clean.
    useLibraryChips.getState().setRemember(false);
    expect(useLibraryChips.getState().chips.albums.format).toBe("hiRes");
    useLibraryChips.getState().reload();
    expect(useLibraryChips.getState().remember).toBe(false);
    expect(useLibraryChips.getState().chips.albums).toEqual(NO_CHIPS);
  });

  it("reads bad stored chips as none", () => {
    const p = parseChipPrefs(JSON.stringify({ remember: true, chips: { albums: { type: "boxset", format: "dsd", decade: 1995, recent: "yes" } } }));
    expect(p.chips.albums).toEqual(NO_CHIPS);
    expect(parseChipPrefs("{nope").remember).toBe(true);
    expect(parseChipPrefs(JSON.stringify({ remember: false, chips: { albums: { format: "hiRes" } } })).chips.albums).toEqual(NO_CHIPS);
  });

  it("is a switch on the Settings page, on by default", async () => {
    await settings();
    render(<SettingsPage />);
    const sw = await screen.findByRole("switch", { name: "Remember library filters" });
    expect(sw).toBeChecked();
    await userEvent.setup().click(sw);
    expect(useLibraryChips.getState().remember).toBe(false);
    expect(JSON.parse(localStorage.getItem(CHIP_PREFS_KEY)!)).toEqual({ remember: false });
  });
});

async function renderLibrary(archive = false) {
  act(() => {
    useLibraryView.setState({ filter: "album", sub: null, sort: "alpha", view: "list", wideView: "list", customOrder: [], recents: {} });
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  });
  await act(() => useLibraryView.getState().loadItems());
  return render(
    <>
      <LibraryPage tab="albums" archive={archive} />
      <ContextMenuHost />
    </>,
  );
}

const shownTitles = () => [...document.querySelectorAll(".lib-row__title")].map((x) => x.textContent ?? "");

describe("filter chips on the library", () => {
  it("narrows albums to hi-res files and says so when nothing matches", async () => {
    const user = userEvent.setup();
    await renderLibrary();
    const group = await screen.findByRole("group", { name: "Filter albums" });
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(0));
    const items = useLibraryView.getState().items!;
    const hiRes = new Set(items.filter((i) => i.kind === "album" && i.quality === "hiRes").map((i) => i.title));
    await user.click(within(group).getByRole("button", { name: "Hi-res" }));
    expect(within(group).getByRole("button", { name: "Hi-res" })).toHaveAttribute("aria-pressed", "true");
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(0));
    for (const t of shownTitles()) expect(hiRes.has(t)).toBe(true);
    expect(useLibraryChips.getState().chips.albums.format).toBe("hiRes");

    // Combined with a decade nothing has: a plain sentence and a way out.
    act(() => useLibraryChips.getState().set("albums", { ...NO_CHIPS, format: "hiRes", decade: 1900 }));
    expect(await screen.findByText("No albums match these filters. Clear one to see more.")).toBeInTheDocument();
    const clears = screen.getAllByRole("button", { name: "Clear filters" });
    await user.click(clears[clears.length - 1]);
    expect(useLibraryChips.getState().chips.albums).toEqual(NO_CHIPS);
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(0));
  });

  it("picks a decade from its menu", async () => {
    const user = userEvent.setup();
    await renderLibrary();
    const group = await screen.findByRole("group", { name: "Filter albums" });
    await user.click(within(group).getByRole("button", { name: "Decade" }));
    const menu = await screen.findByRole("menu");
    const items = within(menu).getAllByRole("menuitemradio").map((m) => m.textContent ?? "");
    expect(items[0]).toContain("Any decade");
    const pick = within(menu).getAllByRole("menuitemradio")[1];
    const label = pick.textContent!.replace(/[^0-9s]/g, "");
    await user.click(pick);
    expect(within(group).getByRole("button", { name: `Decade: ${label}` })).toHaveClass("is-on");
  });

  it("the archive shows demos and unreleased albums with chips of its own", async () => {
    await renderLibrary(true);
    expect(await screen.findByRole("heading", { name: "Archive" })).toBeInTheDocument();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(0));
    const items = useLibraryView.getState().items!;
    const archive = new Set(items.filter((i) => i.kind === "album" && ["demos", "mixtapes", "unreleased"].includes(i.section ?? "")).map((i) => i.title));
    for (const t of shownTitles()) expect(archive.has(t)).toBe(true);
    // It leaves the albums' own chips alone, and has no type chip of its own.
    expect(useLibraryChips.getState().chips.albums).toEqual(NO_CHIPS);
    expect(within(screen.getByRole("group", { name: "Filter albums" })).queryByRole("button", { name: "Type" })).toBeNull();
  });
});

describe("the typographic sleeve for missing art", () => {
  it("prints the title and artist on a flat tone, not a grey square", () => {
    render(<Art path={null} size={SLEEVE_ARTIST_FROM} sleeve={{ title: "Harbor Tapes", artist: "Vesper Lowe" }} />);
    const s = screen.getByTestId("fallback-sleeve");
    expect(s).toHaveTextContent("Harbor Tapes");
    expect(s).toHaveTextContent("Vesper Lowe");
    expect(s.style.getPropertyValue("--sleeve-h")).toBe(String(sleeveHue("Harbor Tapes")));
    expect(s).toHaveAttribute("aria-hidden", "true");
  });

  it("drops the artist when small and prints one letter when tiny", () => {
    const { rerender } = render(<Art path={null} size={56} sleeve={{ title: "Harbor Tapes", artist: "Vesper Lowe" }} />);
    expect(screen.getByTestId("fallback-sleeve")).not.toHaveTextContent("Vesper Lowe");
    rerender(<Art path={null} size={24} sleeve={{ title: "éclat", artist: "X" }} />);
    expect(screen.getByTestId("fallback-sleeve").textContent).toBe("É");
  });

  it("keeps the old fallback for artists, round art and art without a sleeve", () => {
    const { rerender } = render(<Art path={null} size={120} kind="artist" sleeve={{ title: "A" }} />);
    expect(screen.queryByTestId("fallback-sleeve")).toBeNull();
    rerender(<Art path={null} size={120} />);
    expect(screen.queryByTestId("fallback-sleeve")).toBeNull();
    rerender(<Art path={null} size={120} round sleeve={{ title: "A" }} />);
    expect(screen.queryByTestId("fallback-sleeve")).toBeNull();
  });

  it("hues differ between records and hold for one", () => {
    expect(sleeveHue("Ivory")).toBe(sleeveHue("Ivory"));
    expect(sleeveHue("Ivory")).not.toBe(sleeveHue("Afterglow"));
    expect(sleeveHue("Ivory")).toBeGreaterThanOrEqual(0);
    expect(sleeveHue("Ivory")).toBeLessThan(360);
  });
});

describe("Radio and mixes", () => {
  it("lists the mixes by kind with where radio starts", async () => {
    await settings();
    render(<MixesPage />);
    expect(screen.getByRole("heading", { name: "Radio and mixes" })).toBeInTheDocument();
    expect(screen.getByText(/Start radio from any song, album or artist/)).toBeInTheDocument();
    expect(await screen.findByRole("region", { name: "Made for you" })).toBeInTheDocument();
    // It leads with what there is to play, and each mix is a row with its name and Play.
    expect(screen.getByTestId("mixes-lead")).toHaveTextContent(/^\d+ mixes made from what you play/);
    const made = screen.getByRole("list", { name: "Made for you" });
    expect(within(made).getByRole("button", { name: "Daily Mix 1" })).toBeInTheDocument();
    expect(within(made).getByRole("button", { name: "Play Daily Mix 1" })).toBeInTheDocument();
    // Anywhere on the row opens the mix, not just its title.
    await userEvent.setup().click(within(screen.getByTestId("mix-card-daily:1")).getByText(/ and more$/));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "mix", id: "daily:1" });
  });

  it("says what to do when mixes are off", async () => {
    await settings({ homeMixes: false });
    render(<MixesPage />);
    expect(screen.getByText("Turn on mixes in Settings and they show here.")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Open Settings" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "settings", row: "homeMixes" });
  });
});

const qItem = (n: number, albumId: string): QueueItem =>
  ({ itemId: `i${n}`, track: { id: `t${n}`, title: `Song ${n}`, albumId, albumTitle: `Album ${albumId}`, artistTitle: "X", thumb: null } as Track }) as QueueItem;

describe("Turntable's up next", () => {
  it("shows different records, one sleeve per run with its count", () => {
    const q = { userQueue: [qItem(1, "a")], nextUp: [qItem(2, "a"), qItem(3, "a"), qItem(4, "b"), qItem(5, "c"), qItem(6, "c"), qItem(7, "d")] } as QueueView;
    const s = upNextSleeves(q);
    expect(s.map((x) => [x.item.track.albumId, x.count])).toEqual([
      ["a", 3],
      ["b", 1],
      ["c", 2],
    ]);
    expect(upNextSleeves(null)).toEqual([]);
  });

  it("draws the count on the sleeve and says it", async () => {
    await settings({ layout: frame("turntable") });
    const s = await mock.playerState();
    const q = { ...s.queue, userQueue: [], nextUp: [qItem(1, "a"), qItem(2, "a"), qItem(3, "b")] } as QueueView;
    act(() => usePlayer.setState({ state: { ...s, status: "paused", queue: q } }));
    render(<AppShell />);
    expect(await screen.findByTestId("tt-next-count")).toHaveTextContent("2");
    expect(screen.getByRole("button", { name: /^Up next: Song 1 by X, then 1 more from Album a\. Open the queue$/ })).toBeInTheDocument();
  });
});

describe("remembered choices in the layouts", () => {
  it("Gatefold keeps lyrics or credits across songs and restarts", async () => {
    localStorage.removeItem(LEAF_TAB_KEY);
    expect(loadLeafTab()).toBe("lyrics");
    await settings({ layout: frame("gatefold") });
    const s = await mock.playerState();
    act(() => usePlayer.setState({ state: { ...s, status: "paused" } }));
    const { unmount } = render(<AppShell />);
    await userEvent.setup().click(await screen.findByRole("tab", { name: "Credits" }));
    expect(localStorage.getItem(LEAF_TAB_KEY)).toBe("credits");
    unmount();
    render(<AppShell />);
    expect(await screen.findByRole("tab", { name: "Credits" })).toHaveAttribute("aria-selected", "true");
    localStorage.removeItem(LEAF_TAB_KEY);
  });

  it("Console lets go of a picked song when you go to another page", async () => {
    resetConsole();
    await settings({ layout: frame("console") });
    render(<AppShell />);
    const t = (await mock.playerState()).track!;
    act(() => useConsole.setState({ selected: t }));
    act(() => nav.go({ name: "settings" }));
    await waitFor(() => expect(useConsole.getState().selected).toBeNull());
    resetConsole();
  });
});

describe("layout fixes", () => {
  it("Gatefold leaves the right page about 600 px at the fold point", () => {
    const m = gatefoldCss.match(/\.gf-leaf \{[^}]*width: clamp\((\d+)px, calc\((\d+)vw - (\d+)px\), (\d+)px\)/);
    expect(m).not.toBeNull();
    const [min, vw, less, max] = m!.slice(1).map(Number);
    const leaf = (w: number) => Math.min(max, Math.max(min, (vw / 100) * w - less));
    // The window less the outer gaps, the icon rail and the spine.
    const page = (w: number) => w - 3 * 8 - FRAME_RAIL - GATEFOLD_SPINE - leaf(w);
    expect(page(1000)).toBeGreaterThanOrEqual(590);
    expect(leaf(1600)).toBe(max);
  });

  it("frames leave no empty band over the page", () => {
    // Pages start near the top, pinned rows pin to the edge, and the header's
    // backdrop shows only behind the sticky title.
    expect(framesCss).toMatch(/\.app\.frame \.main \{[^}]*--page-top: var\(--space-8\);[^}]*--main-header-h: 0px;/);
    expect(framesCss).toMatch(/\.app\.frame \.main-header::before \{ opacity: 0;/);
    expect(framesCss).toMatch(/\.app\.frame \.main-header:has\(\.main-header__bar\.is-shown\)::before/);
  });
});

describe("Index sections", () => {
  it("reach Radio and mixes and the archive", async () => {
    await settings({ layout: frame("index") });
    render(<AppShell />);
    const nav = await screen.findByRole("navigation", { name: "Main" });
    const user = userEvent.setup();
    await user.click(within(nav).getByRole("button", { name: "Radio and mixes" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "mixes" });
    expect(within(nav).getByRole("button", { name: "Radio and mixes" })).toHaveAttribute("aria-current", "page");
    await user.click(within(nav).getByRole("button", { name: "Archive" }));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "archive" });
    expect(await screen.findByRole("heading", { name: "Archive" })).toBeInTheDocument();
  });
});

// Keep the player store from leaking a queue into other files.
afterEach(() => {
  act(() => usePlayer.setState({ state: null }));
  void player;
});
