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
import { contextRoute } from "../shell/nowPlaying";
import { openPageFind } from "../shell/pageFind";
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
    expect(screen.getByText(totalsLine(all.total!, all.totalMs!))).toBeInTheDocument();
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

  it("asks again when the library changes, keeping the rows and the selection on screen meanwhile", async () => {
    const ask = vi.spyOn(backend, "songsPage");
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    const rows = () => within(grid()).getAllByRole("row").filter((r) => r.hasAttribute("data-track-id"));
    const id = rows()[2].getAttribute("data-track-id");
    fireEvent.click(rows()[2]);
    const before = shownTitles();
    const n = ask.mock.calls.length;
    let answer!: () => void;
    ask.mockImplementationOnce((q, offset, limit) => new Promise((ok) => (answer = () => ok(mock.songsPage(q, offset, limit)))));
    act(() => mock.emit("library://changed", null));
    await waitFor(() => expect(ask.mock.calls.length).toBeGreaterThan(n));
    // No skeletons while the new rows come.
    expect(shownTitles()).toEqual(before);
    expect(grid().querySelector(".songs__row--wait")).toBeNull();
    await act(async () => answer());
    await waitFor(() => expect(grid().querySelector(`[data-track-id="${id}"]`)).toHaveAttribute("aria-selected", "true"));
  });

  it("a later page that fails says so once and isn't asked for again until Try again", async () => {
    const user = userEvent.setup();
    const one = mock.fx.tracks[0];
    let failing = true;
    const ask = vi.spyOn(backend, "songsPage").mockImplementation(async (_q, offset, limit) => {
      if (offset > 0 && failing) throw { kind: "internal", message: "database is locked" };
      return {
        rows: Array.from({ length: limit }, (_, i): SongRow => ({ track: { ...one, id: `s${offset + i}`, title: `Song ${offset + i}` }, year: 2000, addedAt: 0 })),
        total: offset > 0 ? null : 1_000,
        totalMs: offset > 0 ? null : 1_000 * 180_000,
      };
    });
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    // End: lands on a row that is still loading, and keeps the keyboard there.
    const first = within(grid()).getAllByRole("row").find((r) => r.getAttribute("data-row") === "0")!;
    first.focus();
    // jsdom doesn't scroll: give the scroller a height and make scrollTo move it, so far-off rows render.
    const had = HTMLElement.prototype.scrollTo;
    const tall = Object.getOwnPropertyDescriptor(Element.prototype, "scrollHeight");
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", { configurable: true, get: () => 100_000 });
    HTMLElement.prototype.scrollTo = function (this: HTMLElement, a?: ScrollToOptions | number) {
      Object.defineProperty(this, "scrollTop", { configurable: true, writable: true, value: typeof a === "object" ? (a.top ?? 0) : 0 });
      this.dispatchEvent(new Event("scroll"));
    } as typeof had;
    try {
      fireEvent.keyDown(first, { key: "End" });
      await waitFor(() => expect(document.activeElement).toHaveAttribute("data-row", "999"));
    } finally {
      HTMLElement.prototype.scrollTo = had;
      delete (HTMLElement.prototype as { scrollHeight?: number }).scrollHeight;
      if (tall) Object.defineProperty(Element.prototype, "scrollHeight", tall);
    }
    expect(await screen.findByText(/Some songs didn't load/)).toBeInTheDocument();
    const calls = ask.mock.calls.length;
    await new Promise((r) => setTimeout(r, 300));
    expect(ask.mock.calls.length).toBe(calls);
    failing = false;
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(grid().querySelector('[data-track-id="s999"]')).not.toBeNull());
    expect(screen.queryByText(/Some songs didn't load/)).toBeNull();
  });

  it("Ctrl+F goes to its find field, Esc clears it, and playing from it leads back here", async () => {
    const user = userEvent.setup();
    page();
    await waitFor(() => expect(shownTitles().length).toBeGreaterThan(5));
    const box = screen.getByRole("searchbox", { name: "Find a song" });
    expect(openPageFind()).toBe(true);
    expect(box).toHaveFocus();
    await user.type(box, "zzzz");
    await user.keyboard("{Escape}");
    expect(box).toHaveValue("");
    expect(contextRoute({ kind: "tracks", id: "songs", title: "Your songs" })).toEqual({ name: "songs" });
    expect(contextRoute({ kind: "tracks", id: "albums" })).toBeNull();
  });
});
