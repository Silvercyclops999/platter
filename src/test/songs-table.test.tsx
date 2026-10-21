/**
 * Your Library > Songs: every song as a table that sorts by any column,
 * shows or hides columns from a header's menu, counts what's in view or
 * selected, plays from a row in the table's order, says when it can't load,
 * and draws only the rows on screen of a very large library.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { SongRow, SongsQuery } from "../lib/types";
import { DEFAULT_COLUMNS, SongsPage, clickSort, loadTablePrefs, totalsLine, useSongsTable } from "../pages/SongsPage";

const mock = backend as MockBackend;

beforeEach(() => {
  localStorage.removeItem("platter.songsTable.v1");
  useSongsTable.setState({ columns: DEFAULT_COLUMNS, sort: "title", desc: false });
});
afterEach(() => vi.restoreAllMocks());

function page() {
  return render(
    <>
      <div className="main__scroll">
        <SongsPage />
      </div>
      <ContextMenuHost />
    </>,
  );
}

const grid = () => screen.getByRole("grid", { name: "Your songs" });
const shownTitles = () => Array.from(grid().querySelectorAll(".songs__title")).map((e) => e.firstChild?.textContent);
const lastQuery = (spy: { mock: { calls: unknown[][] } }) => spy.mock.calls[spy.mock.calls.length - 1][0] as SongsQuery;

describe("helpers", () => {
  it("a header click sorts by it, most first for counts and dates, and flips on a second click", () => {
    expect(clickSort({ sort: "title", desc: false }, "album")).toEqual({ sort: "album", desc: false });
    expect(clickSort({ sort: "title", desc: false }, "plays")).toEqual({ sort: "plays", desc: true });
    expect(clickSort({ sort: "plays", desc: true }, "plays")).toEqual({ sort: "plays", desc: false });
  });

  it("words the totals", () => {
    expect(totalsLine(12408, 34 * 86_400_000 + 2 * 3_600_000)).toBe("12,408 songs, 34 days 2 hr");
    expect(totalsLine(1, 200_000)).toBe("1 song, 3 min 20 sec");
    expect(totalsLine(3, 600_000, true)).toBe("3 songs selected, 10 min");
  });

  it("reads saved choices, and falls back on anything it doesn't know", () => {
    expect(loadTablePrefs(null)).toEqual({ columns: DEFAULT_COLUMNS, sort: "title", desc: false });
    expect(loadTablePrefs("not json").sort).toBe("title");
    expect(loadTablePrefs(JSON.stringify({ columns: ["time", "bogus", "album"], sort: "nope", desc: true }))).toEqual({ columns: ["album", "time"], sort: "title", desc: true });
  });
});

describe("the table", () => {
  it("lists songs by title with the totals of the library", async () => {
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    const titles = shownTitles();
    expect([...titles].sort((a, b) => a!.localeCompare(b!, undefined, { sensitivity: "base" }))).toEqual(titles);
    const all = await mock.songsPage({ sort: "title", desc: false, find: "" }, 0, 1);
    expect(screen.getByText(totalsLine(all.total, all.totalMs))).toBeInTheDocument();
  });

  it("sorts by a column and flips it", async () => {
    const user = userEvent.setup();
    const ask = vi.spyOn(backend, "songsPage");
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    await user.click(screen.getByRole("button", { name: "Plays" }));
    await waitFor(() => expect(lastQuery(ask)).toMatchObject({ sort: "plays", desc: true }));
    expect(screen.getByRole("columnheader", { name: "Plays" })).toHaveAttribute("aria-sort", "descending");
    await user.click(screen.getByRole("button", { name: "Plays" }));
    await waitFor(() => expect(lastQuery(ask)).toMatchObject({ sort: "plays", desc: false }));
    expect(JSON.parse(localStorage.getItem("platter.songsTable.v1")!)).toMatchObject({ sort: "plays", desc: false });
  });

  it("right-clicking a header shows and hides columns", async () => {
    const user = userEvent.setup();
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    fireEvent.contextMenu(screen.getByRole("columnheader", { name: "Album" }));
    const menu = await screen.findByRole("menu", { name: "Columns" });
    await user.click(within(menu).getByRole("menuitemradio", { name: "Album" }));
    await waitFor(() => expect(screen.queryByRole("columnheader", { name: "Album" })).toBeNull());
    fireEvent.contextMenu(screen.getByRole("columnheader", { name: "Title" }));
    await user.click(within(await screen.findByRole("menu", { name: "Columns" })).getByRole("menuitemradio", { name: "Format" }));
    expect(screen.getByRole("columnheader", { name: "Format" })).toBeInTheDocument();
    expect(useSongsTable.getState().columns).toEqual(["year", "format", "plays", "added", "time"]);
  });

  it("finds by words, and says when nothing matches", async () => {
    const user = userEvent.setup();
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    const want = mock.fx.tracks[3].title;
    await user.type(screen.getByRole("searchbox", { name: "Find a song" }), want);
    await waitFor(() => expect(shownTitles()).toContain(want));
    await user.clear(screen.getByRole("searchbox", { name: "Find a song" }));
    await user.type(screen.getByRole("searchbox", { name: "Find a song" }), "zzzz qqqq");
    expect(await screen.findByText('No songs match "zzzz qqqq"')).toBeInTheDocument();
  });

  it("selecting rows counts the selection", async () => {
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    const rows = within(grid()).getAllByRole("row").filter((r) => r.hasAttribute("data-row"));
    fireEvent.click(rows[0]);
    fireEvent.click(rows[2], { shiftKey: true });
    expect(screen.getByText(/^3 songs selected, /)).toBeInTheDocument();
    fireEvent.click(rows[1], { ctrlKey: true });
    expect(screen.getByText(/^2 songs selected, /)).toBeInTheDocument();
  });

  it("plays from a row in the table's order, as one list", async () => {
    const play = vi.spyOn(backend, "playContext");
    const ids = vi.spyOn(backend, "songsPlayIds");
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    const row = within(grid()).getAllByRole("row").filter((r) => r.hasAttribute("data-row"))[3];
    fireEvent.doubleClick(row);
    await waitFor(() => expect(play).toHaveBeenCalled());
    const start = row.getAttribute("data-track-id")!;
    expect(ids).toHaveBeenCalledWith({ sort: "title", desc: false, find: "" }, start);
    const args = play.mock.calls[0][0];
    expect(args.context).toMatchObject({ kind: "tracks", id: "songs" });
    expect(args.startTrackId).toBe(start);
    expect(args.shuffle).toBe(false);
  });

  it("a failed load says so, and Try again loads", async () => {
    const user = userEvent.setup();
    vi.spyOn(backend, "songsPage").mockRejectedValueOnce({ kind: "internal", message: "database is locked" });
    page();
    expect(await screen.findByText("Couldn't load your songs")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
  });

  it("an empty library says so", async () => {
    vi.spyOn(backend, "songsPage").mockResolvedValue({ rows: [], total: 0, totalMs: 0 });
    page();
    expect(await screen.findByText("No songs in your library yet.")).toBeInTheDocument();
  });

  it("draws only the rows on screen of 100,000 songs, a page at a time", async () => {
    const one = mock.fx.tracks[0];
    const ask = vi.spyOn(backend, "songsPage").mockImplementation(async (_q, offset, limit) => ({
      rows: Array.from({ length: limit }, (_, i): SongRow => ({ track: { ...one, id: `s${offset + i}`, title: `Song ${offset + i}` }, year: 2000, addedAt: 0 })),
      total: 100_000,
      totalMs: 100_000 * 180_000,
    }));
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    expect(grid().querySelectorAll("[data-row]").length).toBeLessThan(60);
    expect(grid()).toHaveAttribute("aria-rowcount", "100001");
    expect(ask).toHaveBeenCalledTimes(1);
    expect(screen.getByText("100,000 songs, 208 days 8 hr")).toBeInTheDocument();
  });

  it("starts over when the library changes", async () => {
    const ask = vi.spyOn(backend, "songsPage");
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    const n = ask.mock.calls.length;
    act(() => mock.emit("library://changed", null));
    await waitFor(() => expect(ask.mock.calls.length).toBeGreaterThan(n));
  });
});
