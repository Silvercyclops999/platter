/**
 * Home led by records in Platter's own looks: the album you were in the
 * middle of, then shelves of records, before the usual shelves. Classic
 * keeps the usual Home, empty shelves and an empty hero hide, the setting
 * turns it off, and the cards move with the arrow keys.
 */
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resolveTheme } from "../design/themes";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Album, type HomeData, type RecordHome, type RecordProgress } from "../lib/types";
import { HomePage } from "../pages/HomePage";
import { gridColumns, progressOf, resumeLine } from "../pages/RecordHome";
import { SettingsPage } from "../pages/SettingsPage";
import { player } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useTheme } from "../stores/theme";
import { ONLINE_EXTRAS } from "./onlineExtras";

const mock = backend as MockBackend;
const SLOW = { timeout: 4000 };
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

function album(n: number): Album {
  return {
    id: `al${n}`,
    title: `Album ${n}`,
    sortTitle: `Album ${n}`,
    artistId: "ar1",
    artistTitle: "Some Artist",
    year: 2000 + n,
    thumb: `/art/${n}`,
    format: "album",
    subformats: [],
    section: "albums",
    genres: [],
    trackCount: 10,
    addedAt: 1_790_000_000,
    viewCount: n,
    lastViewedAt: null,
    tint: null,
  };
}

const progress = (n: number, next = 4): RecordProgress => ({
  album: album(n),
  nextTrackId: `t${n}-${next}`,
  nextTitle: `Song ${next}`,
  nextNumber: next,
  total: 10,
  playedAt: Date.now() - 3_600_000,
});

function records(over: Partial<RecordHome> = {}): RecordHome {
  return {
    shown: true,
    resume: progress(1),
    newArrivals: [album(20), album(21)],
    spunThisWeek: [
      { album: album(30), plays: 6 },
      { album: album(31), plays: 1 },
    ],
    unfinished: [progress(40, 3)],
    notInAYear: [{ album: album(50), plays: 22, lastPlayed: Date.UTC(2024, 2, 10) }],
    ...over,
  };
}

const home: HomeData = {
  shortcuts: [{ kind: "album", album: album(1) }],
  recentlyPlayed: [{ kind: "album", album: album(2) }],
  jumpBackIn: [album(3)],
  playlists: [],
  recentlyAdded: [album(4)],
  topArtists: [],
};

const STUDIO = resolveTheme({ theme: "studio", accent: "brass", themeFile: null, followSystem: false }, true);
const CLASSIC = resolveTheme({ theme: "dark", accent: "ivory", themeFile: null, followSystem: false }, true);

const titles = () => screen.queryAllByRole("heading", { level: 2 }).map((h) => h.textContent);

async function renderHome(r: RecordHome | null = records(), settings: Partial<typeof DEFAULT_SETTINGS> = {}) {
  vi.spyOn(backend, "home").mockResolvedValue(home);
  const load = vi.spyOn(backend, "homeRecords").mockResolvedValue(r ?? { ...records(), shown: false });
  const s = { ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS, homeMixes: false, releaseRadar: false, homeNewToYou: false, homeOnThisDay: false, homeTimeTravel: false, ...settings };
  await mock.settingsSet(s);
  useSettings.setState({ settings: s });
  render(
    <div className="main__scroll">
      <HomePage />
    </div>,
  );
  await screen.findByRole("heading", { name: "Recently played" }, SLOW);
  return load;
}

beforeEach(() => {
  useTheme.setState({ resolved: STUDIO });
  useRouter.getState().reset();
  Element.prototype.scrollTo = vi.fn();
});
afterEach(() => {
  vi.restoreAllMocks();
  useTheme.setState({ resolved: STUDIO });
});

describe("record decisions", () => {
  it("says where Resume starts and how far in that is", () => {
    expect(resumeLine(progress(1, 6), false)).toBe("Picks up at song 6 of 10, Song 6.");
    expect(resumeLine(progress(1, 6), true)).toBe("Song 6 of 10, Song 6.");
    expect(progressOf(progress(1, 6))).toBeCloseTo(0.5);
    expect(progressOf(progress(1, 1))).toBe(0);
  });

  it("lays cards in a grid row when wide and a scrolling row when narrow", () => {
    expect(gridColumns(0)).toBe(0);
    expect(gridColumns(700)).toBe(0);
    expect(gridColumns(1000)).toBe(5);
    expect(gridColumns(1400)).toBe(7);
    expect(gridColumns(4000)).toBe(7);
  });
});

describe("Home led by records", () => {
  it("opens with the record to resume, then the record shelves, then the usual shelves", async () => {
    await renderHome();
    await screen.findByTestId("record-hero", {}, SLOW);
    expect(screen.getByText("Pick up where you left off")).toBeInTheDocument();
    expect(screen.getByText("Picks up at song 4 of 10, Song 4.")).toBeInTheDocument();
    const order = titles();
    const at = (t: string) => order.indexOf(t);
    expect(order[0]).toBe("Album 1");
    expect(at("New arrivals")).toBeGreaterThan(0);
    expect(at("New arrivals")).toBeLessThan(at("Spun this week"));
    expect(at("Spun this week")).toBeLessThan(at("Finish what you started"));
    expect(at("Finish what you started")).toBeLessThan(at("Not played in a year"));
    expect(at("Not played in a year")).toBeLessThan(at("Recently played"));
    // The shortcut grid comes after the records too.
    const hero = screen.getByTestId("record-hero");
    const grid = screen.getByRole("list", { name: "Shortcuts" });
    expect(hero.compareDocumentPosition(grid) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // New arrivals stand in for Recently added.
    expect(order).not.toContain("Recently added");
    // Each card says something about its record.
    expect(screen.getByText("6 plays this week")).toBeInTheDocument();
    expect(screen.getByText("1 play this week")).toBeInTheDocument();
    expect(screen.getByText("Song 3 of 10")).toBeInTheDocument();
    expect(screen.getByText(/^Last played March 2024$/)).toBeInTheDocument();
  });

  it("Resume plays the album from where it stopped; a stopped card does too", async () => {
    const play = vi.spyOn(player, "playAlbum").mockResolvedValue(undefined as never);
    await renderHome();
    const hero = await screen.findByTestId("record-hero", {}, SLOW);
    fireEvent.click(within(hero).getByRole("button", { name: "Resume" }));
    expect(play).toHaveBeenCalledWith("al1", "t1-4");
    fireEvent.click(screen.getByRole("button", { name: "Resume Album 40" }));
    expect(play).toHaveBeenCalledWith("al40", "t40-3");
    fireEvent.click(screen.getByRole("button", { name: "Play Album 20" }));
    expect(play).toHaveBeenLastCalledWith("al20");
  });

  it("opens the album from the hero and the cards", async () => {
    await renderHome();
    const hero = await screen.findByTestId("record-hero", {}, SLOW);
    fireEvent.click(within(hero).getByRole("button", { name: "Open album" }));
    expect(route()).toEqual({ name: "album", id: "al1" });
    fireEvent.click(screen.getByRole("button", { name: "Album 50" }));
    expect(route()).toEqual({ name: "album", id: "al50" });
  });

  it("hides the hero with nothing to resume and each empty shelf", async () => {
    await renderHome(records({ resume: null, spunThisWeek: [], notInAYear: [] }));
    await screen.findByRole("heading", { name: "New arrivals" }, SLOW);
    expect(screen.queryByTestId("record-hero")).toBeNull();
    const order = titles();
    expect(order).not.toContain("Spun this week");
    expect(order).not.toContain("Not played in a year");
    expect(order).toContain("Finish what you started");
  });

  it("with nothing at all, Home is the usual one", async () => {
    await renderHome(records({ resume: null, newArrivals: [], spunThisWeek: [], unfinished: [], notInAYear: [] }));
    await waitFor(() => expect(backend.homeRecords).toHaveBeenCalled());
    expect(screen.queryByTestId("home-records")).toBeNull();
    expect(titles()).toContain("Recently added");
  });

  it("Classic keeps the usual Home and never asks for records", async () => {
    useTheme.setState({ resolved: CLASSIC });
    const load = await renderHome();
    expect(load).not.toHaveBeenCalled();
    expect(screen.queryByTestId("record-hero")).toBeNull();
    expect(titles()).toEqual(["Recently played", "Jump back in", "Recently added"]);
  });

  it("turning the setting off restores the usual order", async () => {
    const load = await renderHome(records(), { homeRecordsFirst: false });
    expect(load).not.toHaveBeenCalled();
    expect(screen.queryByTestId("home-records")).toBeNull();
    expect(titles()).toEqual(["Recently played", "Jump back in", "Recently added"]);
  });

  it("the Artists and Playlists chips leave the records out", async () => {
    await renderHome();
    await screen.findByTestId("record-hero", {}, SLOW);
    fireEvent.click(screen.getByRole("button", { name: "Artists" }));
    await waitFor(() => expect(screen.queryByTestId("home-records")).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "Albums" }));
    expect(await screen.findByTestId("home-records")).toBeInTheDocument();
  });

  it("arrow keys move between a shelf's records, Home and End to its ends", async () => {
    await renderHome(records({ newArrivals: [album(20), album(21), album(22)] }));
    const shelf = await screen.findByTestId("records-new", {}, SLOW);
    const links = within(shelf).getAllByRole("button", { name: /^Album 2\d$/ });
    expect(links).toHaveLength(3);
    links[0].focus();
    fireEvent.keyDown(links[0], { key: "ArrowRight" });
    expect(document.activeElement).toBe(links[1]);
    fireEvent.keyDown(links[1], { key: "End" });
    expect(document.activeElement).toBe(links[2]);
    fireEvent.keyDown(links[2], { key: "ArrowRight" });
    expect(document.activeElement).toBe(links[2]);
    fireEvent.keyDown(links[2], { key: "Home" });
    expect(document.activeElement).toBe(links[0]);
    // From a card's play button too.
    const playBtn = within(shelf).getByRole("button", { name: "Play Album 21" });
    playBtn.focus();
    fireEvent.keyDown(playBtn, { key: "ArrowLeft" });
    expect(document.activeElement).toBe(links[0]);
  });

  it("the stopped records show how far in they are", async () => {
    await renderHome();
    await screen.findByTestId("record-hero", {}, SLOW);
    const bar = screen.getByRole("progressbar", { name: "Album 40: song 3 of 10" });
    expect(bar).toHaveAttribute("aria-valuenow", "2");
    expect(screen.getByRole("progressbar", { name: "Song 4 of 10" })).toHaveAttribute("aria-valuemax", "10");
  });
});

describe("the setting", () => {
  beforeEach(async () => {
    await mock.settingsSet({ ...DEFAULT_SETTINGS });
    await loadSettings();
  });

  it("is on by default and turning it off reaches the core", async () => {
    expect(DEFAULT_SETTINGS.homeRecordsFirst).toBe(true);
    const user = userEvent.setup();
    render(<SettingsPage />);
    const row = (await screen.findByText((_, el) => el?.getAttribute("data-setting") === "homeRecordsFirst", {}, SLOW)) as HTMLElement;
    expect(within(row).getByText("Home leads with records")).toBeInTheDocument();
    expect(within(row).queryByText("Only applies in Studio, Studio Black and Sleeve.")).toBeNull();
    await user.click(within(row).getByRole("switch"));
    await waitFor(async () => expect((await mock.settingsGet()).homeRecordsFirst).toBe(false));
  });

  it("says it only applies in the Studio looks while another look is on", async () => {
    useTheme.setState({ resolved: CLASSIC });
    render(<SettingsPage />);
    const row = (await screen.findByText((_, el) => el?.getAttribute("data-setting") === "homeRecordsFirst", {}, SLOW)) as HTMLElement;
    expect(within(row).getByText("Only applies in Studio, Studio Black and Sleeve.")).toBeInTheDocument();
  });

  it("the mock follows it", async () => {
    expect((await mock.homeRecords()).shown).toBe(true);
    await mock.settingsSet({ ...DEFAULT_SETTINGS, homeRecordsFirst: false });
    expect(await mock.homeRecords()).toMatchObject({ shown: false, resume: null, newArrivals: [] });
  });
});
