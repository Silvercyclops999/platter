/**
 * Listening stats: the range choices and number wording, the recap card's
 * layout (unreleased songs and covers stay off it), the mock's ranking
 * rules, the Stats page (hero, ranges, top lists, Play these, Save as
 * playlist, charts and their text alternatives, the calendar's ranges,
 * moments, monthly recaps, compare), the stats strip and its menu item,
 * the Stats symbol, every stats setting, and the page's failure states.
 */
import { act, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { LikedCover } from "../components/LikedCover";
import { StatsStrip } from "../components/StatsStrip";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { layoutRecapCard, recapContent } from "../lib/recapCard";
import type { Measure } from "../lib/shareCard";
import { addDays, addMonths, barFractions, change, formatMinutes, hourLabel, localDay, parseDay, playedFromLabel, rangeLabel, sparkPath, toRange } from "../lib/stats";
import { calendarGrid, calendarMonths, calendarSummary, calendarWindow, clockSummary, level, shareOfTop } from "../lib/statsCharts";
import type { MonthRecap, TopSong } from "../lib/statsTypes";
import { DEFAULT_SETTINGS } from "../lib/types";
import { BarChart, HourClock, ListeningCalendar } from "../pages/StatsCharts";
import { momentCards, momentDate } from "../pages/StatsMoments";
import { StatsPage } from "../pages/StatsPage";
import { AppMenu } from "../shell/TopBar";
import { useLibraryLoader, useLibraryModel } from "../shell/useLibrary";
import { useApp } from "../stores/app";
import { player } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useStatsDialog, yourStatsItem } from "../stores/stats";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const PAD_X = 80;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

async function settings(patch: Partial<typeof DEFAULT_SETTINGS> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  await settings();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset({ name: "home" });
  useStatsDialog.setState({ subject: null });
  vi.restoreAllMocks();
});

describe("stats helpers", () => {
  const today = "2026-10-15";
  it("turns range choices into local days", () => {
    expect(toRange({ kind: "4w" }, today)).toEqual({ from: "2026-09-18", to: today });
    expect(toRange({ kind: "6m" }, today)).toEqual({ from: "2026-04-16", to: today });
    expect(toRange({ kind: "year" }, today)).toEqual({ from: "2026-01-01", to: today });
    expect(toRange({ kind: "pickYear", year: 2025 }, today)).toEqual({ from: "2025-01-01", to: "2025-12-31" });
    expect(toRange({ kind: "pickMonth", month: "2024-02" }, today)).toEqual({ from: "2024-02-01", to: "2024-02-29" });
    expect(toRange({ kind: "all" }, today)).toEqual({ from: null, to: null });
    expect(toRange({ kind: "custom", from: "2026-05-10", to: "2026-05-01" }, today)).toEqual({ from: "2026-05-01", to: "2026-05-10" });
    expect(addMonths("2026-03-31", -1)).toBe("2026-02-28");
  });

  it("says what a range is and words the numbers", () => {
    expect(rangeLabel({ kind: "4w" })).toBe("Last 4 weeks");
    expect(rangeLabel({ kind: "pickMonth", month: "2026-09" })).toBe("September 2026");
    expect(rangeLabel({ kind: "custom", from: "2026-05-01", to: "2026-05-10" })).toBe("May 1, 2026 to May 10, 2026");
    expect(formatMinutes(60_000)).toBe("1 minute");
    expect(formatMinutes(3_600_000)).toBe("60 minutes");
    expect(hourLabel(0)).toBe("12 AM");
    expect(hourLabel(21)).toBe("9 PM");
    expect(change(120, 100)).toBe("+20%");
    expect(change(80, 100)).toBe("-20%");
    expect(change(80, 0)).toBeNull();
    expect(barFractions([0, 5, 10])).toEqual([0, 0.5, 1]);
    expect(barFractions([0, 0])).toEqual([0, 0]);
    expect(sparkPath([0, 4], 100, 20)).toBe("M2.0 18.0 L98.0 2.0");
    expect(playedFromLabel({ kind: "album", title: "First" })).toBe("from First");
    expect(playedFromLabel({ kind: "imported", title: null })).toBe("in your imported history");
    expect(playedFromLabel({ kind: "imported", title: null, id: "plex_history" })).toBe("from your Plex history");
    expect(playedFromLabel({ kind: "imported", title: null, id: "tautulli" })).toBe("from Tautulli");
    expect(playedFromLabel({ kind: "imported", title: null, id: "lastfm" })).toBe("from Last.fm");
    expect(playedFromLabel({ kind: "imported", title: null, id: "file" })).toBe("in your imported history");
    expect(playedFromLabel(null)).toBeNull();
  });
});

describe("monthly recap card", () => {
  const song = (title: string, unreleased: boolean): TopSong => ({ key: title, trackId: null, title, artist: "Band", album: null, plays: 3, ms: 1, unreleased, track: null });
  const recap: MonthRecap = {
    month: "2026-09",
    totals: { ms: 600 * 60_000, plays: 200, songs: 50, artists: 10, albums: 12, days: 20 },
    prevMs: 500 * 60_000,
    topSongs: [song("Out Now", false), song("Secret Demo", true), song("Also Out", false)],
    topArtists: [{ key: "band", artistId: null, name: "Band", thumb: null, plays: 9, ms: 1 }],
    topAlbum: null,
    topAlbums: [
      { key: "a", albumId: "a", title: "Out", artist: "Band", thumb: "/al/out", plays: 9, ms: 1, unreleased: false },
      { key: "b", albumId: "b", title: "Demos", artist: "Band", thumb: "/al/demos", plays: 5, ms: 1, unreleased: true },
      { key: "c", albumId: "c", title: "Live", artist: "Band", thumb: "/al/live", plays: 3, ms: 1, unreleased: false },
    ],
    topGenre: "Rock",
    daily: Array.from({ length: 30 }, (_, i) => ({ key: `2026-09-${String(i + 1).padStart(2, "0")}`, plays: i, ms: i * 60_000 })),
    biggestDay: null,
  };
  const measure: Measure = (t, size) => t.length * size * 0.55;

  it("leaves unreleased songs off the card", () => {
    const c = recapContent(recap);
    expect(c.songs.map((s) => s.title)).toEqual(["Out Now", "Also Out"]);
    expect(c.minutes).toBe(600);
    expect(c.change).toBe("+20% on August");
    expect(c.month).toBe("September 2026");
    expect(c.covers).toEqual(["/al/out", "/al/live"]);
  });

  it("lays out inside the card in both shapes", () => {
    for (const format of ["story", "square"] as const) {
      const l = layoutRecapCard(format, recapContent(recap), measure);
      const all = l.texts.flatMap((t) => t.lines);
      expect(all).toContain("600");
      expect(all).toContain("TOP SONGS");
      expect(all.join(" ")).not.toContain("Secret Demo");
      expect(l.bars).toHaveLength(30);
      for (const t of l.texts) {
        expect(t.y + (t.lines.length - 1) * t.size * t.lineHeight).toBeLessThan(l.mark.y - 20);
        expect(t.x).toBeGreaterThanOrEqual(0);
      }
      for (const b of l.bars) expect(b.x + b.w).toBeLessThanOrEqual(l.width - 79);
      // Two released covers, top right, inside the card and clear of the bars.
      expect(l.covers).toHaveLength(2);
      const barTop = Math.min(...l.bars.map((b) => b.y));
      for (const c of l.covers) {
        expect(c.x + c.size).toBeLessThanOrEqual(l.width - 79);
        expect(c.y + c.size).toBeLessThan(barTop);
      }
      const month = l.texts.find((t) => t.lines[0] === "September 2026")!;
      expect(PAD_X + measure(month.lines[0], month.size, 800)).toBeLessThan(Math.min(...l.covers.map((c) => c.x)));
    }
  });
});

describe("mock stats rules", () => {
  it("ranks by plays then minutes, and charts by day or month", async () => {
    const o = await backend.statsOverview({ from: null, to: null });
    const ranked = [...o.topSongs].sort((a, b) => b.plays - a.plays || b.ms - a.ms);
    expect(o.topSongs.map((s) => s.key)).toEqual(ranked.map((s) => s.key));
    expect(o.bucket).toBe("month");
    expect(o.totals.plays).toBeGreaterThan(1000);
    expect(o.series.reduce((s, p) => s + p.plays, 0)).toBe(o.totals.plays);
    const span = await backend.statsSpan();
    const w = await backend.statsOverview(toRange({ kind: "4w" }, span.today));
    expect(w.bucket).toBe("day");
    expect(w.series).toHaveLength(28);
    await expect(backend.statsOverview({ from: "2026-05-02", to: "2026-05-01" })).rejects.toMatchObject({ kind: "invalid" });
  });
});

describe("Stats page", () => {
  it("shows totals, a chart and top lists, and changes range", async () => {
    const user = userEvent.setup();
    render(<StatsPage />);
    expect(await screen.findByRole("heading", { level: 1, name: "Stats" })).toBeInTheDocument();
    const hero = await screen.findByTestId("stats-hero");
    const o = await backend.statsOverview(toRange({ kind: "4w" }, (await backend.statsSpan()).today));
    expect(within(hero).getByTestId("hero-minutes")).toHaveTextContent(`${Math.round(o.totals.ms / 60_000).toLocaleString()}minutes listened`);
    expect(within(hero).getByText("Top artist")).toBeInTheDocument();
    expect(within(hero).getByText(o.topArtists[0].name)).toBeInTheDocument();
    expect(within(hero).getByText("Longest streak")).toBeInTheDocument();
    expect(within(hero).getByText("New artists found")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /Last 4 weeks/ })).toBeInTheDocument();
    expect(screen.getByRole("figure", { name: /Minutes listened per day/ })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Your top songs" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "All time" }));
    expect(await screen.findByRole("heading", { name: /All time/ })).toBeInTheDocument();
    expect(await screen.findByRole("figure", { name: /Minutes listened per month/ })).toBeInTheDocument();
    expect(within(await screen.findByTestId("stats-hero")).getByText("Artists you've played")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Artists" }));
    expect(await screen.findByRole("heading", { name: "Your top artists" })).toBeInTheDocument();
    expect(screen.getByTestId("stats-feature")).toHaveAccessibleName(/^Your top artist: /);
    await user.click(screen.getByRole("tab", { name: "Genres" }));
    // Every row has artwork: a cover, or a lettered tile for a genre without one.
    const rows = within(screen.getByRole("tabpanel")).getAllByRole("listitem");
    for (const r of rows) expect(r.querySelector(".art, .stats-genre-tile")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "A month" }));
    expect(await screen.findByRole("combobox", { name: "Month" })).toBeInTheDocument();
  });

  it("Play these plays the top songs; Save as playlist makes one", async () => {
    const user = userEvent.setup();
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined as never);
    const create = vi.spyOn(backend, "playlistCreate");
    render(<StatsPage />);
    await user.click(await screen.findByRole("button", { name: "Play these" }));
    await waitFor(() => expect(play).toHaveBeenCalled());
    const ids = play.mock.calls[0][0];
    const top = (await backend.statsOverview(toRange({ kind: "4w" }, (await backend.statsSpan()).today))).topSongs;
    expect(ids[0]).toBe(top[0].trackId);
    await user.click(screen.getByRole("button", { name: "Save as playlist" }));
    await waitFor(() => expect(create).toHaveBeenCalledWith("Top songs, Last 4 weeks", ids));
  });

  it("monthly recaps show while that setting is on and open a share card", async () => {
    const user = userEvent.setup();
    const { unmount } = render(<StatsPage />);
    const cards = await screen.findAllByTestId("recap-card");
    expect(cards.length).toBeGreaterThan(3);
    const share = await within(cards[1]).findByRole("button", { name: /^Share / });
    await user.click(share);
    expect(await screen.findByTestId("recap-share")).toBeInTheDocument();
    expect(screen.getByText("Unreleased songs are left off the card. Nothing is uploaded.")).toBeInTheDocument();
    unmount();
    await settings({ monthlyStatsOnProfile: false });
    render(<StatsPage />);
    await screen.findByTestId("stats-hero");
    expect(screen.queryByTestId("recap-card")).toBeNull();
  });

  it("compare is off by default, and shows the overlap with a friend when on", async () => {
    const { unmount } = render(<StatsPage />);
    await screen.findByTestId("stats-hero");
    expect(screen.queryByRole("heading", { name: "Compare with a friend" })).toBeNull();
    await expect(backend.statsFriends()).rejects.toMatchObject({ kind: "invalid" });
    unmount();
    await settings({ statsCompare: true });
    render(<StatsPage />);
    expect(await screen.findByRole("heading", { name: "Compare with a friend" })).toBeInTheDocument();
    const view = await screen.findByTestId("compare-view");
    expect(within(view).getByText(/% taste overlap/)).toBeInTheDocument();
    expect(within(view).getByText("Paper Satellites · not in your library")).toBeInTheDocument();
  });

  it("the summary a friend gets has names and weights only, without unreleased songs", async () => {
    await settings({ statsCompare: true });
    const s = await backend.statsCompareSummary(false, false);
    expect(Object.keys(s).sort()).toEqual(["artists", "name", "songs", "unreleasedOk", "version"]);
    const isUnreleased = (t: { albumId: string }) => mock.fx.albums.some((a) => a.id === t.albumId && a.format === null && a.subformats.length === 0);
    const released = new Set(mock.fx.tracks.filter((t) => !isUnreleased(t)).map((t) => t.title));
    const onlyUnreleased = new Set(mock.fx.tracks.filter((t) => isUnreleased(t) && !released.has(t.title)).map((t) => t.title));
    expect(onlyUnreleased.size).toBeGreaterThan(0);
    expect(s.songs.some((x) => onlyUnreleased.has(x.title))).toBe(false);
  });

  it("says when the Stats page is off", async () => {
    await settings({ statsPage: false });
    render(<StatsPage />);
    expect(await screen.findByText("Stats are off")).toBeInTheDocument();
  });

  it("app state: counting fails, so the page says so and tries again", async () => {
    const user = userEvent.setup();
    const real = backend.statsOverview.bind(backend);
    const spy = vi.spyOn(backend, "statsOverview").mockRejectedValue({ kind: "internal", message: "database is locked" });
    render(<StatsPage />);
    expect(await screen.findByRole("heading", { name: "Couldn't count your stats" })).toBeInTheDocument();
    expect(screen.getByText("database is locked")).toBeInTheDocument();
    spy.mockImplementation(real);
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByTestId("stats-hero")).toBeInTheDocument();
  });

  it("app state: moments fail, so the rest of the page still shows", async () => {
    vi.spyOn(backend, "statsMoments").mockRejectedValue({ kind: "internal", message: "database is locked" });
    render(<StatsPage />);
    expect(await screen.findByTestId("stats-hero")).toBeInTheDocument();
    await screen.findByRole("heading", { name: "Your top songs" });
    expect(screen.queryByRole("heading", { name: "Moments" })).toBeNull();
  });

  it("shows skeletons while counting", async () => {
    vi.spyOn(backend, "statsOverview").mockReturnValue(new Promise(() => {}));
    render(<StatsPage />);
    expect(await screen.findByTestId("stats-loading")).toBeInTheDocument();
    expect(screen.queryByTestId("stats-hero")).toBeNull();
  });

  it("with nothing played, says how stats fill up", async () => {
    vi.spyOn(backend, "statsSpan").mockResolvedValue({ first: null, last: null, months: [], today: "2026-10-15" });
    render(<StatsPage />);
    expect(await screen.findByText("Nothing to count yet")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import past plays" })).toBeInTheDocument();
    expect(screen.queryByTestId("stats-hero")).toBeNull();
  });
});

describe("getting to Stats", () => {
  it("the app menu and the library have Stats while the page is on", async () => {
    const user = userEvent.setup();
    const { unmount } = render(<AppMenu />);
    await user.click(screen.getByRole("button", { name: /App menu/ }));
    await user.click(screen.getByRole("menuitem", { name: "Stats" }));
    expect(route()).toEqual({ name: "stats" });
    unmount();
    const { result } = renderHook(() => {
      useLibraryLoader();
      return useLibraryModel("");
    });
    await waitFor(() => expect(result.current.all.some((e) => e.stats)).toBe(true));
    await act(async () => settings({ statsPage: false }));
    await waitFor(() => expect(result.current.all.some((e) => e.stats)).toBe(false));
    render(<AppMenu />);
    await user.click(screen.getByRole("button", { name: /App menu/ }));
    expect(screen.queryByRole("menuitem", { name: "Stats" })).toBeNull();
  });
});

describe("stats strip", () => {
  const playedArtist = async () => {
    const o = await backend.statsOverview({ from: null, to: null });
    return o.topArtists[0];
  };

  it("shows plays, rank and a sparkline on a played artist", async () => {
    const a = await playedArtist();
    render(<StatsStrip kind="artist" id={a.artistId!} />);
    const strip = await screen.findByTestId("stats-strip");
    expect(within(strip).getByText(`${a.plays.toLocaleString()} plays`)).toBeInTheDocument();
    expect(
      within(strip)
        .getByText(/in your top artists/)
        .closest("dd"),
    ).toHaveTextContent("#1 in your top artists");
    expect(within(strip).getByRole("img", { name: /Plays per month, last 12 months/ })).toBeInTheDocument();
    expect(within(strip).getByText("First listen")).toBeInTheDocument();
  });

  it("hides when its setting is off, and so does Your stats in menus", async () => {
    const a = await playedArtist();
    const item = yourStatsItem("artist", a.artistId!, a.name);
    expect(item.when?.()).toBe(true);
    item.onSelect?.();
    expect(useStatsDialog.getState().subject).toEqual({ kind: "artist", id: a.artistId, title: a.name });
    await settings({ statsStrip: false });
    expect(item.when?.()).toBe(false);
    render(<StatsStrip kind="artist" id={a.artistId!} />);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByTestId("stats-strip")).toBeNull();
  });

  it("stays hidden for something never played", async () => {
    const spy = vi.spyOn(backend, "statsItem");
    render(<StatsStrip kind="album" id="no-such-album" />);
    await waitFor(() => expect(spy).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByTestId("stats-strip")).toBeNull();
  });
});

describe("stats charts", () => {
  const days = (n: number, from = "2026-09-01") => Array.from({ length: n }, (_, i) => ({ key: addDays(from, i), plays: i % 3, ms: (i % 3) * 600_000 }));

  it("the bar chart has a summary, a label for every bar, and moves by keyboard", async () => {
    const user = userEvent.setup();
    const points = days(10);
    render(<BarChart points={points} bucket="day" label="Minutes listened per day, test" />);
    const fig = screen.getByRole("figure", { name: "Minutes listened per day, test" });
    expect(fig).toHaveAccessibleDescription(/Minutes listened per day\. 6 of 10 days with listening, averaging 15 minutes\. Most: Sep 3, 2026, 20 minutes\./);
    expect(screen.getAllByRole("img", { name: /^Sep \d+, 2026: \d+ minutes, \d+ plays?$/ })).toHaveLength(10);
    const bars = screen.getByRole("group", { name: /Arrow keys move through the days/ });
    await user.tab();
    expect(bars).toHaveFocus();
    // Focus starts on the last day with listening (Sep 9), then arrows move.
    expect(screen.getByTestId("bars-readout")).toHaveTextContent("Sep 9, 2026 20 minutes · 2 plays");
    await user.keyboard("{ArrowRight}");
    expect(screen.getByTestId("bars-readout")).toHaveTextContent("Sep 10, 2026 0 minutes · 0 plays");
    await user.keyboard("{Home}");
    expect(screen.getByTestId("bars-readout")).toHaveTextContent("Sep 1, 2026");
    expect(bars).toHaveAttribute("aria-activedescendant", expect.stringMatching(/b0$/));
  });

  it("the 24-hour clock says the busiest hour and how the day splits", async () => {
    const user = userEvent.setup();
    const hours = Array.from({ length: 24 }, (_, h) => (h >= 18 && h < 24 ? 60_000 * (h - 10) : h === 8 ? 120_000 : 0));
    render(<HourClock hours={hours} busiest={23} weekdays={[1, 2, 3, 4, 5, 6, 7]} busiestWeekday={6} />);
    expect(clockSummary(hours, 23)).toBe(
      "Listening by hour of the day. Busiest hour: 11 PM. night (12 AM to 6 AM) 0%, morning (6 AM to 12 PM) 3%, afternoon (12 PM to 6 PM) 0%, evening (6 PM to 12 AM) 97%.",
    );
    expect(screen.getByRole("figure", { name: "Listening by hour of the day" })).toHaveAccessibleDescription(clockSummary(hours, 23));
    expect(screen.getAllByRole("img", { name: /^\d+ (AM|PM): \d+ minutes$/ })).toHaveLength(24);
    expect(screen.getByRole("img", { name: "Busiest weekday: Saturday" })).toBeInTheDocument();
    screen.getByRole("group", { name: /Arrow keys move through the hours/ }).focus();
    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("group", { name: /hours/ })).toHaveAttribute("aria-activedescendant", expect.stringMatching(/h0$/));
    expect(clockSummary(Array(24).fill(0), null)).toBe("Listening by hour of the day: nothing yet.");
  });

  it("the calendar shows the whole year for a year, and 53 weeks otherwise", () => {
    const y = calendarWindow("2025-01-01", "2025-12-31");
    expect(y).toEqual({ from: "2024-12-29", to: "2026-01-03", title: "2025" });
    expect(calendarWindow("2026-01-01", "2026-10-15").title).toBe("2026");
    const w = calendarWindow("2026-09-18", "2026-10-15");
    expect(w.to).toBe("2026-10-17");
    expect(w.from).toBe("2025-10-12");
    expect(parseDay(w.from).getDay()).toBe(0);
    const grid = calendarGrid(w, [], { from: "2026-09-18", to: "2026-10-15" });
    expect(grid.weeks).toHaveLength(53);
    expect(grid.weeks.every((wk) => wk.length === 7)).toBe(true);
  });

  it("calendar cells scale to the range's busiest day, dim outside it, and skip a year's padding", () => {
    const w = calendarWindow("2025-01-01", "2025-12-31");
    const pts = [
      { key: "2024-12-30", plays: 9, ms: 9_000_000 },
      { key: "2025-03-01", plays: 1, ms: 100_000 },
      { key: "2025-03-02", plays: 4, ms: 3_600_000 },
      { key: "2025-06-10", plays: 2, ms: 900_000 },
    ];
    const g = calendarGrid(w, pts, { from: "2025-03-01", to: "2025-03-31" });
    const cell = (d: string) => g.weeks.flat().find((c) => c.day === d)!;
    expect(cell("2024-12-30")).toMatchObject({ inWindow: false, level: 0 });
    expect(cell("2025-03-02")).toMatchObject({ level: 4, inRange: true });
    expect(cell("2025-03-01").level).toBe(1);
    expect(cell("2025-06-10")).toMatchObject({ inRange: false, after: true });
    expect(cell("2025-06-10").level).toBeGreaterThan(0);
    expect(g.daysListened).toBe(2);
    expect(g.busiest?.day).toBe("2025-03-02");
    expect(g.months.map((m) => m.label)).toEqual(["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]);
    expect(calendarMonths(g)).toEqual([{ month: "March 2025", days: 2, minutes: 62 }]);
    expect(calendarSummary(w, g)).toBe("Listening calendar, 2025. 2 days with listening in this range. Busiest day: Mar 2, 2025, 60 minutes.");
    expect([0, 1, 50, 250, 1000].map((ms) => level(ms, 1000))).toEqual([0, 1, 1, 2, 4]);
  });

  it("the calendar loads its days, has a text table, and moves by day and week", async () => {
    const user = userEvent.setup();
    const spy = vi.spyOn(backend, "statsDays").mockResolvedValue([
      { key: "2026-10-01", plays: 3, ms: 600_000 },
      { key: "2026-10-08", plays: 6, ms: 1_200_000 },
    ]);
    render(<ListeningCalendar from="2026-09-18" to="2026-10-15" />);
    const fig = await screen.findByTestId("stats-calendar");
    expect(spy).toHaveBeenCalledWith("2025-10-12", "2026-10-17");
    await waitFor(() => expect(fig).toHaveAccessibleDescription("Listening calendar, Oct 12, 2025 to Oct 17, 2026. 2 days with listening in this range. Busiest day: Oct 8, 2026, 20 minutes."));
    const table = within(fig).getByRole("table", { name: "Listening by month" });
    expect(within(table).getByRole("row", { name: "October 2026 2 30" })).toBeInTheDocument();
    const grid = within(fig).getByRole("group", { name: /Arrow keys move by day and week/ });
    act(() => grid.focus());
    expect(within(fig).getByText("Thu, Oct 8, 2026: 20 minutes, 6 plays")).toBeInTheDocument();
    await user.keyboard("{ArrowLeft}");
    expect(within(fig).getByText("Thu, Oct 1, 2026: 10 minutes, 3 plays")).toBeInTheDocument();
    await user.keyboard("{ArrowUp}");
    expect(within(fig).getByText("Wed, Sep 30, 2026: no listening")).toBeInTheDocument();
    await user.keyboard("{ArrowLeft}{ArrowLeft}{ArrowLeft}");
    expect(within(fig).getByText("Wed, Sep 9, 2026: no listening (outside this range)")).toBeInTheDocument();
  });

  it("each top row's bar is its share of the #1", () => {
    expect(shareOfTop([10, 5, 2, 0])).toEqual([1, 0.5, 0.2, 0]);
    expect(shareOfTop([])).toEqual([]);
  });
});

describe("moments", () => {
  const song = (title: string, id: string | null): TopSong => ({ key: title, trackId: id, title, artist: "Band", album: null, plays: 4, ms: 1, unreleased: false, track: null });

  it("words each moment and leaves out the ones that didn't happen", () => {
    const m = {
      firstSong: { song: song("Opener", null), day: "2026-01-01", at: new Date(2026, 0, 1, 7, 5).getTime() },
      replayed: { song: song("Again", null), day: "2026-03-04", at: null },
      discovery: { artist: { key: "new", artistId: "ar9", name: "New Band", thumb: null, plays: 42, ms: 1 }, day: "2025-12-31" },
      comeback: null,
      newArtists: 3,
    };
    const cards = momentCards(m, "2026", true, "2026");
    expect(cards.map((c) => c.id)).toEqual(["first", "replayed", "discovery"]);
    expect(cards[0]).toMatchObject({ eyebrow: "First song of 2026", title: "Opener", sub: "Band" });
    expect(cards[0].detail).toMatch(/^Jan 1 at 7:05/);
    expect(cards[1].detail).toBe("4 plays on Mar 4");
    expect(cards[2]).toMatchObject({ eyebrow: "Biggest discovery", title: "New Band", sub: "First played Dec 31, 2025", detail: "42 plays since" });
    cards[2].action?.run();
    expect(route()).toEqual({ name: "artist", id: "ar9" });
    expect(momentCards({ firstSong: null, replayed: null, discovery: null, comeback: null, newArtists: 0 }, "Last 4 weeks", false, "2026")).toEqual([]);
    expect(momentDate("2025-08-04", "2026")).toBe("Aug 4, 2025");
  });

  it("the mock follows the core's rules: the first play, three plays a day, new artists and long breaks", async () => {
    const span = await backend.statsSpan();
    const r = toRange({ kind: "6m" }, span.today);
    const m = await backend.statsMoments(r);
    const plays = mock.stats.plays().filter((p) => localDay(new Date(p.at)) >= r.from! && localDay(new Date(p.at)) <= r.to!);
    expect(m.firstSong?.song.trackId).toBe(plays[0].trackId);
    expect(m.replayed!.song.plays).toBeGreaterThanOrEqual(3);
    expect(m.newArtists).not.toBeNull();
    if (m.comeback) expect(m.comeback.gapDays).toBeGreaterThanOrEqual(30);
    const all = await backend.statsMoments({ from: null, to: null });
    expect(all.newArtists).toBeNull();
    expect(all.discovery).toBeNull();
  });

  it("the Stats page shows the moments as cards that play their song", async () => {
    const user = userEvent.setup();
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined as never);
    render(<StatsPage />);
    expect(await screen.findByRole("heading", { name: "Moments" })).toBeInTheDocument();
    const first = await screen.findByTestId("moment-first");
    expect(first).toHaveTextContent("First song of the range");
    await user.click(within(first).getByRole("button"));
    expect(play).toHaveBeenCalled();
  });
});

describe("the Stats symbol", () => {
  it("draws its own glyph on an accent tile in the library and the app menu", async () => {
    const user = userEvent.setup();
    const { container, unmount } = render(<LikedCover icon="stats" size={48} />);
    const tile = container.querySelector(".liked-cover--stats")!;
    expect(tile).toHaveAttribute("aria-hidden", "true");
    expect(tile.querySelector("svg.stats-glyph rect")).not.toBeNull();
    unmount();
    render(<AppMenu />);
    await user.click(screen.getByRole("button", { name: /App menu/ }));
    expect(screen.getByRole("menuitem", { name: "Stats" }).querySelector("svg.stats-glyph")).not.toBeNull();
  });
});
