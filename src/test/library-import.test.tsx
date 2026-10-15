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
import { importSourceName } from "../lib/importStrings";
import { ImportPage } from "../pages/ImportPage";
import { LibraryImportSettings, importMatches } from "../pages/LibraryImportSettings";
import { countParts, resetImportListeners, skippedLine, songsLine } from "../pages/importShared";
import { useRouter } from "../stores/router";
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
    const switches = await within(section).findAllByRole("switch");
    expect(switches).toHaveLength(3);
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
    expect(report).toHaveTextContent("5,210 plays");
    expect(toasts()).toContain(`${importSourceName("streaming")}: 33 of 38 songs found`);
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
  it("is hidden while no source is on", async () => {
    render(<ImportPage />);
    expect(await screen.findByText("Library import is off.")).toBeInTheDocument();
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

  it("is read only, with songs not in the library dimmed and labelled", async () => {
    await open("Road Trip");
    const rows = within(screen.getByRole("list", { name: "Road Trip" })).getAllByRole("listitem");
    expect(rows).toHaveLength(12);
    const missing = rows.filter((r) => r.dataset.status === "missing");
    expect(missing).toHaveLength(2);
    for (const r of missing) {
      expect(r).toHaveClass("import-row--missing");
      expect(r).toHaveTextContent("Not in your library");
      expect(r).not.toHaveAttribute("tabindex");
    }
    expect(rows[0]).toHaveClass("import-row--playable");
    expect(screen.getByText(/10 of 12 songs in your library/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /remove/i })).toBeNull();
  });

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
      skipped: { privatePlays: 1, podcasts: 0, noMetadata: 0, shortPlays: 2, incompleteRows: 0, unknownFiles: 0 },
    };
    expect(songsLine(r)).toBe("812 of 900 songs found");
    expect(countParts(r)).toEqual(["100 of 120 liked", "1 playlist", "1 play"]);
    expect(skippedLine(r.skipped)).toBe("Left out: 1 private play, 2 plays under 30 seconds.");
    expect(skippedLine({ ...r.skipped, privatePlays: 0, shortPlays: 0 })).toBeNull();
  });
});
