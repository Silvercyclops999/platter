/**
 * Library import: the Settings section (each source off until picked,
 * Choose file once on, the report line), the import page (hidden while no
 * source is on, reports, the review queue), an imported playlist with its
 * missing songs dimmed, the two Plex writes only on an explicit click, and
 * the Missing list's ranking and store links.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { importSourceName, storeLinks } from "../lib/importStrings";
import { ImportPage } from "../pages/ImportPage";
import { LibraryImportSettings, importMatches } from "../pages/LibraryImportSettings";
import {
  countParts,
  historyLine,
  missingFocusKey,
  missingKey,
  reportEmpty,
  reportLine,
  reportPercent,
  resetImportListeners,
  showInMissing,
  skippedLine,
  songsLine,
} from "../pages/importShared";
import { listHeaderLine } from "../pages/ImportListView";
import { player } from "../stores/player";
import { useUi } from "../stores/ui";
import { nav, useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useLikes } from "../stores/likes";

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts.map((t) => t.message);
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

async function enable(...sources: string[]) {
  await mock.settingsSet({ importSources: sources });
  await loadSettings();
}

beforeEach(async () => {
  mock.importMock.reset();
  mock.importLinksOpened.length = 0;
  resetImportListeners();
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset({ name: "home" });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("Settings > Library import", () => {
  it("lists every source with its help, all off, with nothing to pick until one is on", async () => {
    render(<LibraryImportSettings />);
    const section = await screen.findByTestId("library-import-settings");
    expect(within(section).getByRole("heading", { name: "Library import" })).toBeInTheDocument();
    const switches = (await within(section).findAllByRole("switch")).filter((s) => !s.closest("[data-setting='showPlayedALot'], [data-setting='bringIn']"));
    expect(switches).toHaveLength(5);
    for (const s of switches) expect(s).toHaveAttribute("aria-checked", "false");
    expect(within(section).getByText(/Account > Privacy settings/)).toBeInTheDocument();
    expect(within(section).getByText(/TuneMyMusic or Soundiiz/)).toBeInTheDocument();
    expect(within(section).getByText(/at least artist and title columns/)).toBeInTheDocument();
    expect(within(section).queryByRole("button", { name: "Choose file" })).toBeNull();

    fireEvent.click(within(section).getByRole("switch", { name: "CSV or M3U list" }));
    await waitFor(() => expect(useSettings.getState().settings?.importSources).toEqual(["lists"]));
    expect(await within(section).findByRole("button", { name: "Choose file" })).toBeInTheDocument();
    expect(within(section).getByRole("button", { name: "Choose folder" })).toBeInTheDocument();
    expect((await mock.settingsGet()).importSources).toEqual(["lists"]);
  });

  it("imports from a picked file and shows the report line", async () => {
    await enable("streaming");
    render(<LibraryImportSettings />);
    fireEvent.click(await screen.findByRole("button", { name: "Choose file" }));
    const report = await screen.findByTestId("import-report-streaming");
    expect(report).toHaveTextContent("33 of 38 songs found");
    expect(report).toHaveTextContent("17 of 20 liked");
    expect(report).toHaveTextContent("2 playlists");
    expect(report).toHaveTextContent("11 of 14 saved albums in your library");
    expect(report).toHaveTextContent("5,210 plays imported");
    expect(report).toHaveTextContent("610 of 1,240 played songs are in your library");
    expect(toasts().some((t) => t.startsWith(`${importSourceName("streaming")}: 33 of 38 songs found. 5,210 plays imported`))).toBe(true);
    fireEvent.click(within(report).getByRole("button", { name: "See report" }));
    expect(route()).toEqual({ name: "import" });
  });

  it("says when an import fails", async () => {
    await enable("lists");
    mock.importMock.failNext = "Couldn't find artist and title columns in that file.";
    render(<LibraryImportSettings />);
    fireEvent.click(await screen.findByRole("button", { name: "Choose file" }));
    await waitFor(() => expect(toasts()).toContain("Couldn't import from CSV or M3U list: Couldn't find artist and title columns in that file."));
    expect(screen.queryByTestId("import-report-lists")).toBeNull();
  });

  it("removes what a source imported", async () => {
    await enable("transfer");
    mock.importMock.run("transfer", "x.csv");
    render(<LibraryImportSettings />);
    const report = await screen.findByTestId("import-report-transfer");
    fireEvent.click(within(report).getByRole("button", { name: "Remove imported data" }));
    await waitFor(() => expect(screen.queryByTestId("import-report-transfer")).toBeNull());
    expect((await mock.importOverview()).reports).toEqual([]);
  });

  it("shows for searches about it", () => {
    expect(importMatches("")).toBe(true);
    expect(importMatches("import")).toBe(true);
    expect(importMatches("deezer")).toBe(true);
    expect(importMatches("crossfade")).toBe(false);
  });
});

describe("Import page", () => {
  it("is Bring music over while nothing is imported", async () => {
    render(<ImportPage />);
    expect(await screen.findByRole("heading", { level: 1, name: /Bring music over/ })).toBeInTheDocument();
    expect(screen.queryByText("Library import is off.")).toBeNull();
    expect(screen.queryByText(/songs found/)).toBeNull();
    render(<ImportPage view="missing" />);
    expect(screen.queryByRole("heading", { name: "Missing songs" })).toBeNull();
  });

  it("shows each source's report, and the review queue takes accept, another pick and none", async () => {
    await enable("streaming", "transfer");
    mock.importMock.run("streaming", "my_data.zip");
    mock.importMock.run("transfer", "library.csv");
    render(<ImportPage />);
    const streaming = await screen.findByTestId("report-streaming");
    expect(streaming).toHaveTextContent("33 of 38 songs found");
    expect(streaming).toHaveTextContent("2 to check");
    expect(streaming).toHaveTextContent("Left out: 37 private plays, 212 podcast episodes");
    expect(screen.getByTestId("report-transfer")).toHaveTextContent("8 of 10 songs found");

    const queue = await screen.findByRole("heading", { name: "Check these matches" });
    expect(queue).toBeInTheDocument();
    const items = screen.getAllByRole("listitem").filter((li) => li.classList.contains("import-review__item"));
    expect(items).toHaveLength(2);
    const first = items[0];
    const cands = within(first).getAllByRole("button", { name: /^(Accept|Use) / });
    expect(cands[0]).toHaveTextContent("Accept");
    fireEvent.click(cands[1]);
    await waitFor(() => expect(screen.getAllByRole("listitem").filter((li) => li.classList.contains("import-review__item"))).toHaveLength(1));
    const second = screen.getAllByRole("listitem").find((li) => li.classList.contains("import-review__item"))!;
    fireEvent.click(within(second).getByRole("button", { name: /None of these/ }));
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Check these matches" })).toBeNull());
    const o = await mock.importOverview();
    expect(o.reviewCount).toBe(0);
    expect(o.reports.find((r) => r.source === "streaming")!.songs).toMatchObject({ matched: 34, unsure: 0, missing: 4 });
  });

  it("lists imported playlists with their match rate and opens one", async () => {
    await enable("streaming");
    mock.importMock.run("streaming", "my_data.zip");
    render(<ImportPage />);
    const road = await screen.findByRole("button", { name: /Road Trip/ });
    expect(road).toHaveTextContent("10 of 12 in your library");
    expect(screen.getByRole("button", { name: /Liked songs/ })).toHaveTextContent("17 of 20 in your library");
    fireEvent.click(road);
    expect(route()).toMatchObject({ name: "import", view: expect.stringMatching(/^list:pl:/) });
  });
});

describe("An imported playlist", () => {
  async function open(name: string) {
    await enable("streaming");
    mock.importMock.run("streaming", "my_data.zip");
    const list = (await mock.importOverview()).lists.find((l) => l.name === name)!;
    render(<ImportPage view={`list:${list.id}`} />);
    await screen.findByRole("heading", { name });
    return list;
  }

  it("saves to Plex only after the click and the confirm", async () => {
    const create = vi.spyOn(mock, "playlistCreate");
    const like = vi.spyOn(mock, "likesSet");
    await open("Road Trip");
    expect(create).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Save as playlist" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent('A new Plex playlist named "Road Trip" gets the 10 songs in your library. 2 songs aren\'t in your library and stay out.');
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(create).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Save as playlist" }));
    fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Save playlist" }));
    await waitFor(() => expect(create).toHaveBeenCalledTimes(1));
    expect(create.mock.calls[0][0]).toBe("Road Trip");
    expect(create.mock.calls[0][1]).toHaveLength(10);
    expect(like).not.toHaveBeenCalled();
  });

  it("likes the matched songs in Plex only after the click and the confirm", async () => {
    useLikes.setState({ ids: new Set() });
    const like = vi.spyOn(mock, "likesSet");
    const create = vi.spyOn(mock, "playlistCreate");
    await open("Liked songs");
    expect(like).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Like matched songs" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("17 songs in your library will be liked. 3 songs aren't in your library and stay out.");
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Like 17 songs" }));
    });
    await waitFor(() => expect(like).toHaveBeenCalledTimes(1));
    expect(like.mock.calls[0][0]).toHaveLength(17);
    expect(like.mock.calls[0][1]).toBe(true);
    expect(create).not.toHaveBeenCalled();
  });

  it("shows songs not in the library dimmed and labelled, with covers like any list", async () => {
    await open("Road Trip");
    const rows = within(screen.getByRole("list", { name: "Road Trip" })).getAllByRole("listitem");
    expect(rows).toHaveLength(12);
    const missing = rows.filter((r) => r.dataset.status === "missing");
    expect(missing).toHaveLength(2);
    for (const r of missing) {
      expect(r).toHaveClass("import-row--missing");
      expect(r).toHaveTextContent("Not in your library");
      // Still a row: focusable and selectable, with the placeholder cover.
      expect(r).toHaveAttribute("tabindex", "0");
      expect(r.querySelector(".art__fallback")).not.toBeNull();
      expect(r.querySelector("img")).toBeNull();
    }
    expect(rows[0]).toHaveClass("import-row--playable");
    // Songs in the library show their album's cover.
    expect(rows[0].querySelector(".import-row__art img, .import-row__art .art__skeleton")).not.toBeNull();
    expect(screen.getByText(/10 of 12 songs in your library, \d+ min/)).toBeInTheDocument();
  });

  it("selects a dimmed row and explains why it's dimmed", async () => {
    await open("Road Trip");
    const row = within(screen.getByRole("list", { name: "Road Trip" })).getAllByRole("listitem").find((r) => r.dataset.status === "missing")!;
    fireEvent.click(row);
    expect(row).toHaveClass("is-selected");
    fireEvent.mouseEnter(row);
    expect(await screen.findByRole("tooltip", {}, { timeout: 2000 })).toHaveTextContent("couldn't find this song in your library");
  });

  it("says why a dimmed row doesn't play, and links to it on the Missing list", async () => {
    const play = vi.spyOn(player, "playTracks");
    await open("Road Trip");
    const row = within(screen.getByRole("list", { name: "Road Trip" })).getAllByRole("listitem").find((r) => r.dataset.status === "missing")!;
    fireEvent.doubleClick(row);
    await waitFor(() => expect(toasts()).toContain("This song isn't in your library"));
    expect(play).not.toHaveBeenCalled();
    useToasts.setState({ toasts: [] });
    fireEvent.keyDown(row, { key: "Enter" });
    await waitFor(() => expect(toasts()).toContain("This song isn't in your library"));
    const t = useToasts.getState().toasts[0];
    expect(t.action?.label).toBe("See in Missing");
    act(() => t.action!.run());
    expect(route()).toEqual({ name: "import", view: "missing" });
    expect(missingFocusKey()).toBe(missingKey("Harbor Lights", "Paper Boats"));
    expect(play).not.toHaveBeenCalled();
  });

  it("plays a song in the library from where it's double-clicked", async () => {
    const play = vi.spyOn(player, "playTracks");
    await open("Road Trip");
    const rows = within(screen.getByRole("list", { name: "Road Trip" })).getAllByRole("listitem");
    fireEvent.doubleClick(rows[4]);
    expect(play).toHaveBeenCalledTimes(1);
    const [ids, start] = play.mock.calls[0];
    expect(ids).toHaveLength(10);
    expect(start).toBe(ids[3]);
  });

  it("asks to check an unsure match instead of playing it", async () => {
    await open("Rainy Day");
    const row = within(screen.getByRole("list", { name: "Rainy Day" })).getAllByRole("listitem").find((r) => r.dataset.status === "unsure")!;
    fireEvent.doubleClick(row);
    await waitFor(() => expect(toasts()).toContain("Platter isn't sure this song is in your library"));
  });

  it("Play, Shuffle and Add to queue skip dimmed rows", async () => {
    const play = vi.spyOn(player, "playTracks");
    const queue = vi.spyOn(player, "addToQueue");
    await open("Road Trip");
    fireEvent.click(screen.getByRole("button", { name: "Play Road Trip" }));
    expect(play.mock.calls[0][0]).toHaveLength(10);
    fireEvent.click(screen.getByRole("button", { name: "Shuffle Road Trip" }));
    const shuffled = play.mock.calls[1][0];
    expect([...shuffled].sort()).toEqual([...play.mock.calls[0][0]].sort());
    fireEvent.click(screen.getByRole("button", { name: "More options for Road Trip" }));
    const item = useUi.getState().menu!.items.find((i) => i.label === "Add to queue")!;
    item.onSelect!();
    expect(queue.mock.calls[0][0]).toHaveLength(10);
  });

  it("hides the big play button when nothing in the list can play, and says so", async () => {
    await enable("lists");
    mock.importMock.run("lists", "Evening.csv");
    const list = (await mock.importOverview()).lists[0];
    for (const r of (await mock.importListPage(list.id)).rows.filter((x) => x.track)) await mock.importListRemove(list.id, r.songId);
    render(<ImportPage view={`list:${list.id}`} />);
    await screen.findByRole("heading", { name: "Evening" });
    expect(screen.getByText(/0 of 1 song in your library/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Play Evening" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Shuffle Evening" }));
    expect(toasts()).toContain("None of these songs are in your library yet.");
  });

  it("gives a dimmed row a short menu and a library song its usual one", async () => {
    await open("Road Trip");
    const rows = within(screen.getByRole("list", { name: "Road Trip" })).getAllByRole("listitem");
    const dim = rows.find((r) => r.dataset.status === "missing")!;
    fireEvent.contextMenu(dim);
    const labels = useUi.getState().menu!.items.map((i) => i.label);
    expect(labels).toEqual(["Find in Missing", "Search Bandcamp", "Search Qobuz", "Search iTunes", "Remove from this list"]);
    useUi.getState().closeMenu();
    fireEvent.contextMenu(rows[0]);
    const full = useUi.getState().menu!.items.map((i) => i.label);
    expect(full).toContain("Add to playlist");
    expect(full).toContain("Add to queue");
    expect(full.indexOf("Remove from this list")).toBeGreaterThan(full.indexOf("Add to playlist"));
  });

  it("opens a store search from a dimmed row's menu", async () => {
    await open("Road Trip");
    const dim = within(screen.getByRole("list", { name: "Road Trip" })).getAllByRole("listitem").find((r) => r.dataset.status === "missing")!;
    fireEvent.contextMenu(dim);
    useUi.getState().menu!.items.find((i) => i.label === "Search Bandcamp")!.onSelect!();
    await waitFor(() => expect(mock.importLinksOpened).toEqual([storeLinks("Harbor Lights", "Paper Boats").bandcamp]));
  });

  it("removes a song from this list only", async () => {
    const list = await open("Road Trip");
    const dim = within(screen.getByRole("list", { name: "Road Trip" })).getAllByRole("listitem").find((r) => r.dataset.status === "missing")!;
    fireEvent.contextMenu(dim);
    useUi.getState().menu!.items.find((i) => i.label === "Remove from this list")!.onSelect!();
    await waitFor(() => expect(within(screen.getByRole("list", { name: "Road Trip" })).getAllByRole("listitem")).toHaveLength(11));
    expect(screen.getByText(/10 of 11 songs in your library/)).toBeInTheDocument();
    expect(toasts()).toContain("Removed from Road Trip");
    expect((await mock.importListPage(list.id)).rows).toHaveLength(11);
    // Other lists that want the song keep it.
    expect((await mock.importMissing()).some((s) => s.title === "Paper Boats")).toBe(true);
  });
});

describe("A list that's gone", () => {
  it("opens a centred page with the reason, Remove from your library and Home", async () => {
    await enable("streaming");
    mock.importMock.run("streaming", "my_data.zip");
    const list = (await mock.importOverview()).lists.find((l) => l.name === "Road Trip")!;
    nav.go({ name: "import" });
    nav.go({ name: "import", view: `list:${list.id}` });
    await mock.importForget("streaming");
    render(<ImportPage view={`list:${list.id}`} />);
    expect(await screen.findByRole("heading", { name: "This playlist isn't available" })).toBeInTheDocument();
    expect(screen.getByText(/import was removed/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Home" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Remove from your library" }));
    expect(route()).toEqual({ name: "import" });
    // Back no longer lands on it.
    expect(useRouter.getState().entries.some((e) => e.route.name === "import" && e.route.view === `list:${list.id}`)).toBe(false);
  });

  it("Home goes home", async () => {
    await enable("streaming");
    render(<ImportPage view="list:pl:999" />);
    fireEvent.click(await screen.findByRole("button", { name: "Home" }));
    expect(route()).toEqual({ name: "home" });
  });
});

describe("header and links", () => {
  const row = (track: boolean, ms = 60_000) => ({
    songId: 1,
    artist: "A",
    title: "T",
    album: null,
    durationMs: ms,
    status: track ? ("matched" as const) : ("missing" as const),
    track: track ? ({ durationMs: ms } as never) : null,
  });

  it("counts and times only songs that can play", () => {
    expect(listHeaderLine([row(true), row(true, 120_000), row(false, 600_000)])).toBe("2 of 3 songs in your library, 3 min");
    expect(listHeaderLine([row(true), row(true)])).toBe("2 songs, 2 min");
    expect(listHeaderLine([row(false)])).toBe("0 of 1 song in your library");
  });

  it("builds the same store links as the core", () => {
    expect(storeLinks("Harbor Lights", "Paper Boats").bandcamp).toBe("https://bandcamp.com/search?q=Harbor%20Lights%20Paper%20Boats&item_type=t");
    expect(storeLinks("AC/DC", "T.N.T.").qobuz).toBe("https://www.qobuz.com/search?q=AC%2FDC%20T%2EN%2ET%2E");
  });

  it("history forgets a page that's gone", () => {
    const r = useRouter.getState();
    r.reset({ name: "home" });
    r.navigate({ name: "import", view: "list:pl:1" });
    r.navigate({ name: "import" });
    r.navigate({ name: "import", view: "list:pl:1" });
    useRouter.getState().drop({ name: "import", view: "list:pl:1" });
    const s = useRouter.getState();
    expect(s.entries.map((e) => e.route)).toEqual([{ name: "home" }, { name: "import" }]);
    expect(s.index).toBe(1);
  });
});
describe("Missing songs", () => {
  it("ranks by how many lists want each song and opens store searches", async () => {
    await enable("streaming", "transfer");
    mock.importMock.run("streaming", "my_data.zip");
    mock.importMock.run("transfer", "library.csv");
    render(<ImportPage view="missing" />);
    const list = await screen.findByRole("list", { name: "Missing songs" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows[0]).toHaveTextContent("Paper Boats");
    expect(rows[0]).toHaveTextContent("Wanted by 3 lists");
    const wants = rows.map((r) => Number(/Wanted by (\d+) lists/.exec(r.textContent ?? "")?.[1] ?? 1));
    expect([...wants].sort((a, b) => b - a)).toEqual(wants);
    fireEvent.click(within(rows[0]).getByRole("button", { name: "Search Bandcamp for Paper Boats" }));
    await waitFor(() => expect(mock.importLinksOpened).toEqual(["https://bandcamp.com/search?q=Harbor%20Lights%20Paper%20Boats&item_type=t"]));
    expect(within(rows[0]).getByRole("button", { name: /Qobuz/ })).toBeInTheDocument();
    expect(within(rows[0]).getByRole("button", { name: /iTunes/ })).toBeInTheDocument();
    // Nothing here asks Lidarr for anything.
    expect(screen.queryByRole("button", { name: /request/i })).toBeNull();
  });

  it("brings the song from See in Missing into view", async () => {
    await enable("streaming", "transfer");
    mock.importMock.run("streaming", "my_data.zip");
    mock.importMock.run("transfer", "library.csv");
    showInMissing("Cass Morrow", "Thin Ice Waltz");
    render(<ImportPage view="missing" />);
    const list = await screen.findByRole("list", { name: "Missing songs" });
    const focused = within(list).getAllByRole("listitem").filter((r) => r.classList.contains("is-focused"));
    expect(focused).toHaveLength(1);
    expect(focused[0]).toHaveAttribute("aria-label", "Thin Ice Waltz by Cass Morrow");
    await waitFor(() => expect(document.activeElement).toBe(focused[0]));
    expect(missingFocusKey()).toBeNull();
  });

  it("refuses links that aren't store searches", async () => {
    await expect(backend.importOpenLink("https://example.com/")).rejects.toMatchObject({ kind: "invalid" });
  });
});

describe("report wording", () => {
  it("counts and what was left out", () => {
    const r = {
      source: "streaming" as const,
      importedAt: 0,
      fileName: null,
      songs: { total: 900, matched: 812, unsure: 10, missing: 78 },
      liked: { total: 120, matched: 100 },
      playlists: 1,
      albums: { total: 0, matched: 0 },
      plays: { total: 1, matched: 1 },
      playsAdded: 1,
      history: { plays: 0, playsMatched: 0, songs: 0, songsMatched: 0, firstPlayed: null, lastPlayed: null },
      skipped: { privatePlays: 1, podcasts: 0, noMetadata: 0, shortPlays: 2, incompleteRows: 0, unknownFiles: 0, othersPlaylists: 0 },
    };
    expect(songsLine(r)).toBe("812 of 900 songs found");
    expect(reportLine(r)).toBe("812 of 900 songs found");
    expect(countParts(r)).toEqual(["100 of 120 liked", "1 playlist"]);
    expect(skippedLine(r.skipped)).toBe("Left out: 1 private play, 2 plays under 30 seconds.");
    expect(skippedLine({ ...r.skipped, privatePlays: 0, shortPlays: 0 })).toBeNull();
  });

  const empty = {
    source: "streaming" as const,
    importedAt: 0,
    fileName: null,
    songs: { total: 0, matched: 0, unsure: 0, missing: 0 },
    liked: { total: 0, matched: 0 },
    playlists: 0,
    albums: { total: 0, matched: 0 },
    plays: { total: 0, matched: 0 },
    playsAdded: 0,
    history: { plays: 0, playsMatched: 0, songs: 0, songsMatched: 0, firstPlayed: null, lastPlayed: null },
    skipped: { privatePlays: 0, podcasts: 0, noMetadata: 0, shortPlays: 0, incompleteRows: 0, unknownFiles: 0, othersPlaylists: 0 },
  };
  const history = {
    plays: 153_497,
    playsMatched: 20_000,
    songs: 14_572,
    songsMatched: 509,
    firstPlayed: new Date(2020, 2, 1).getTime(),
    lastPlayed: new Date(2026, 4, 1).getTime(),
  };

  it("describes a history-only import by plays, years and songs, never as 0 of 0", () => {
    const r = { ...empty, plays: { total: 153_497, matched: 20_000 }, playsAdded: 153_497, history };
    const line = reportLine(r);
    expect(line).toBe(`${(153_497).toLocaleString()} plays imported, 2020 to 2026. 509 of ${(14_572).toLocaleString()} songs are in your library.`);
    expect(line).not.toContain("0 of 0");
    expect(historyLine(r)).toBe(line);
    expect(reportPercent(r)).toBe(3);
    expect(countParts(r)).toEqual([]);
    expect(reportEmpty(r)).toBe(false);
    // One year, one song.
    const one = { ...r, history: { ...history, plays: 3, songs: 1, songsMatched: 0, lastPlayed: new Date(2020, 5, 1).getTime() } };
    expect(reportLine(one)).toBe("3 plays imported, in 2020. 0 of 1 song is in your library.");
  });

  it("shows both songs and plays when an import has both", () => {
    const r = { ...empty, songs: { total: 900, matched: 812, unsure: 10, missing: 78 }, history };
    expect(reportLine(r)).toBe(
      `812 of 900 songs found. ${(153_497).toLocaleString()} plays imported, 2020 to 2026. 509 of ${(14_572).toLocaleString()} played songs are in your library.`,
    );
    expect(reportPercent(r)).toBe(90);
  });

  it("says plainly when nothing was usable", () => {
    expect(reportEmpty(empty)).toBe(true);
    expect(reportLine(empty)).toBe("Nothing in that file could be used: no songs, playlists or plays.");
    expect(historyLine(empty)).toBeNull();
  });
});
