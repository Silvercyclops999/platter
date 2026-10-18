import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { HistoryMix, OnThisDay, OnThisDayYear, TimeTravel, TopSong } from "../lib/statsTypes";
import { DEFAULT_SETTINGS } from "../lib/types";
import { OnThisDayPage, OnThisDayShelf, TimeTravelShelf, mixTitle, yearSummary, yearsAgo } from "../pages/HistoryShelves";
import { SettingsPage } from "../pages/SettingsPage";
import { useApp } from "../stores/app";
import { player } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const SLOW = { timeout: 4000 };
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

const song = (id: string | null, title: string, plays: number): TopSong => ({
  key: id ? `k:${id}` : `n:x|${title.toLowerCase()}`,
  trackId: id,
  title,
  artist: "Band",
  album: null,
  plays,
  ms: plays * 200_000,
  unreleased: false,
  track: null,
});

const year = (y: number, songs: TopSong[]): OnThisDayYear => ({
  year: y,
  date: `${y}-10-17`,
  plays: songs.reduce((n, s) => n + s.plays, 0),
  ms: 0,
  songs,
  albums: [],
  trackIds: songs.flatMap((s) => (s.trackId ? [s.trackId] : [])),
  covers: [],
});

const DAY: OnThisDay = {
  shown: true,
  date: "2026-10-17",
  years: [year(2023, [song("t3", "Three", 5)]), year(2021, [song("t1", "One", 2), song(null, "Gone Song", 1)])],
};

const mix = (kind: HistoryMix["kind"], y: number, ids: string[]): HistoryMix => ({ id: `${kind}:${y}`, kind, year: y, plays: 3, trackIds: ids, covers: [] });

const TRAVEL: TimeTravel = { shown: true, yourYear: mix("year", 2022, ["t3", "t2"]), decades: [mix("decade", 1960, ["t4"]), mix("decade", 1990, ["t2", "t1"])] };

async function setSettings(patch: Partial<typeof DEFAULT_SETTINGS> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  await setSettings();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("words", () => {
  it("say how long ago, what a year holds and each mix's name", () => {
    expect(yearsAgo(2025, "2026-10-17")).toBe("1 year ago");
    expect(yearsAgo(2021, "2026-10-17")).toBe("5 years ago");
    expect(yearSummary(DAY.years[1])).toBe("2 songs, 1 not in your library");
    expect(yearSummary(DAY.years[0])).toBe("1 song");
    expect(mixTitle(TRAVEL.decades[1])).toBe("1990s");
    expect(mixTitle(TRAVEL.yourYear!)).toBe("Your year: 2022");
  });
});

describe("On this day", () => {
  it("shows a card per year, busiest first, and a card plays only library songs", async () => {
    vi.spyOn(backend, "homeOnThisDay").mockResolvedValue(DAY);
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(<OnThisDayShelf />);
    const cards = await screen.findAllByTestId(/^on-this-day-/, {}, SLOW);
    expect(cards.map((c) => c.dataset.testid)).toEqual(["on-this-day-2023", "on-this-day-2021"]);
    expect(within(cards[1]).getByText("2 songs, 1 not in your library")).toBeInTheDocument();
    await user.click(within(cards[1]).getByRole("button", { name: "Play 5 years ago" }));
    expect(play).toHaveBeenCalledWith(["t1"], "t1");
    await user.click(within(cards[0]).getByRole("button", { name: "3 years ago" }));
    expect(route()).toEqual({ name: "onThisDay" });
  });

  it("is hidden when there's nothing on this date", async () => {
    const load = vi.spyOn(backend, "homeOnThisDay").mockResolvedValue({ shown: true, date: "2026-10-17", years: [] });
    const { container } = render(<OnThisDayShelf />);
    await waitFor(() => expect(load).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("is hidden and not asked for while the setting is off", async () => {
    await setSettings({ homeOnThisDay: false });
    const load = vi.spyOn(backend, "homeOnThisDay").mockResolvedValue(DAY);
    const { container } = render(<OnThisDayShelf />);
    expect(container).toBeEmptyDOMElement();
    expect(load).not.toHaveBeenCalled();
  });

  it("the page lists each year's songs and marks the ones not in the library", async () => {
    vi.spyOn(backend, "homeOnThisDay").mockResolvedValue(DAY);
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(<OnThisDayPage />);
    expect(await screen.findByRole("heading", { name: "2021" }, SLOW)).toBeInTheDocument();
    const gone = screen.getByText("Gone Song").closest("li")!;
    expect(within(gone).getByText("Not in your library")).toBeInTheDocument();
    expect(within(gone).queryByRole("button")).toBeNull();
    await user.click(screen.getByRole("button", { name: "One" }));
    expect(play).toHaveBeenCalledWith(["t1"], "t1");
  });
});

describe("Time travel", () => {
  it("shows your year first, then the decades oldest first, and its play button plays", async () => {
    vi.spyOn(backend, "homeTimeTravel").mockResolvedValue(TRAVEL);
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(<TimeTravelShelf />);
    const cards = await screen.findAllByTestId(/^time-travel-/, {}, SLOW);
    expect(cards.map((c) => c.dataset.testid)).toEqual(["time-travel-year:2022", "time-travel-decade:1960", "time-travel-decade:1990"]);
    // The title is a name, not a second play button.
    expect(within(cards[2]).queryByRole("button", { name: "1990s" })).toBeNull();
    await user.click(within(cards[2]).getByRole("button", { name: "Play 1990s" }));
    expect(play).toHaveBeenCalledWith(["t2", "t1"], "t2");
    await user.click(within(cards[0]).getByRole("button", { name: "Play Your year: 2022" }));
    expect(play).toHaveBeenLastCalledWith(["t3", "t2"], "t3");
  });

  it("is hidden with no music, and while the setting is off", async () => {
    const load = vi.spyOn(backend, "homeTimeTravel").mockResolvedValue({ shown: true, yourYear: null, decades: [] });
    const first = render(<TimeTravelShelf />);
    await waitFor(() => expect(load).toHaveBeenCalled());
    expect(first.container).toBeEmptyDOMElement();
    first.unmount();
    load.mockClear();
    await setSettings({ homeTimeTravel: false });
    const { container } = render(<TimeTravelShelf />);
    expect(container).toBeEmptyDOMElement();
    expect(load).not.toHaveBeenCalled();
  });

  it("the demo library has decades to travel to", async () => {
    const t = await mock.homeTimeTravel();
    expect(t.shown).toBe(true);
    expect(t.decades.length).toBeGreaterThan(0);
    expect(t.decades.map((d) => d.year)).toEqual([...t.decades.map((d) => d.year)].sort((a, b) => a - b));
  });
});

describe("settings", () => {
  it("both start on and have a row in Settings", async () => {
    expect(DEFAULT_SETTINGS.homeOnThisDay && DEFAULT_SETTINGS.homeTimeTravel).toBe(true);
    render(<SettingsPage />);
    for (const k of ["homeOnThisDay", "homeTimeTravel"]) {
      expect(await screen.findByText((_, el) => el?.getAttribute("data-setting") === k, {}, SLOW)).toBeInTheDocument();
    }
  });

  it("turning one off reaches the core's settings", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const row = await screen.findByText((_, el) => el?.getAttribute("data-setting") === "homeTimeTravel", {}, SLOW);
    await user.click(within(row as HTMLElement).getByRole("switch"));
    await waitFor(async () => expect((await mock.settingsGet()).homeTimeTravel).toBe(false));
  });
});
