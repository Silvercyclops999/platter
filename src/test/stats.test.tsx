/**
 * Listening stats: the range choices and number wording, the recap card's
 * layout (unreleased songs stay off it), the mock's ranking rules, the
 * Stats page (ranges, top lists, Play these, Save as playlist, monthly
 * recaps, compare), the stats strip and its menu item, every stats setting,
 * and the page's failure state.
 */
import { act, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { StatsStrip } from "../components/StatsStrip";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { layoutRecapCard, recapContent } from "../lib/recapCard";
import type { Measure } from "../lib/shareCard";
import { addMonths, barFractions, change, formatMinutes, hourLabel, playedFromLabel, rangeLabel, sparkPath, toRange } from "../lib/stats";
import type { MonthRecap, TopSong } from "../lib/statsTypes";
import { DEFAULT_SETTINGS } from "../lib/types";
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
    expect(await screen.findByText("Minutes listened")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /Last 4 weeks/ })).toBeInTheDocument();
    expect(screen.getByRole("figure", { name: /Minutes listened per day/ })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Your top songs" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "All time" }));
    expect(await screen.findByRole("heading", { name: /All time/ })).toBeInTheDocument();
    expect(await screen.findByRole("figure", { name: /Minutes listened per month/ })).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Artists" }));
    expect(await screen.findByRole("heading", { name: "Your top artists" })).toBeInTheDocument();
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
    await screen.findByText("Minutes listened");
    expect(screen.queryByTestId("recap-card")).toBeNull();
  });

  it("compare is off by default, and shows the overlap with a friend when on", async () => {
    const { unmount } = render(<StatsPage />);
    await screen.findByText("Minutes listened");
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

  it("app state: counting fails, so the page says so", async () => {
    vi.spyOn(backend, "statsOverview").mockRejectedValue({ kind: "internal", message: "database is locked" });
    render(<StatsPage />);
    expect(await screen.findByText("Couldn't count your stats: database is locked")).toBeInTheDocument();
  });

  it("with nothing played, says how stats fill up", async () => {
    vi.spyOn(backend, "statsSpan").mockResolvedValue({ first: null, last: null, months: [], today: "2026-10-15" });
    render(<StatsPage />);
    expect(await screen.findByText("Nothing to count yet")).toBeInTheDocument();
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
    expect(within(strip).getByText(/in your top artists/).closest("dd")).toHaveTextContent("#1 in your top artists");
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
