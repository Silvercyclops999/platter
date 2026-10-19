/**
 * Wrapped and Top Songs in the UI: the year's colour and pattern, what
 * search finds, the words on each card, the summary card's layout and its
 * unreleased rule, the Top Songs page (rank column, labels, find, save as
 * playlist), Home's promotion, the settings, and the cards' navigation
 * (auto-advance, arrow keys, click, hold to pause, Space, Esc, reduced
 * motion). The core's rules (ranking, ties, local years, the 30-play
 * threshold, personality, private sessions, the Home window) are tested
 * in src-tauri/src/stats/wrapped_tests.rs.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { CARD_SIZES, type Measure } from "../lib/shareCard";
import type { Personality, TopSong, Wrapped } from "../lib/statsTypes";
import { DEFAULT_SETTINGS } from "../lib/types";
import {
  andList,
  artistWords,
  busiestHour,
  daysAndHours,
  dayWords,
  hoursAndMinutes,
  layoutSummaryCard,
  minutesWords,
  personalityWords,
  songWords,
  summaryContent,
  topSongsMatches,
  yearPattern,
  yearTone,
} from "../lib/wrapped";
import { HomePage } from "../pages/HomePage";
import { SearchPage } from "../pages/SearchPage";
import { findRows, rowNotes, TopSongsPage } from "../pages/TopSongsPage";
import { AppMenu } from "../shell/TopBar";
import { CARD_MS, HOLD_MS, WrappedView, wrappedCards } from "../shell/Wrapped";
import { useApp } from "../stores/app";
import { player } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useWrapped } from "../stores/wrapped";
import { usePhoneForm } from "../phone/form";
import { followStatusBar } from "../phone/PhoneShell";

const mock = backend as MockBackend;

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
  useWrapped.setState({ year: null });
  delete document.documentElement.dataset.reducedMotion;
  vi.restoreAllMocks();
});

afterEach(() => {
  vi.useRealTimers();
});

/** Every character 0.55 of the size wide. */
const measure: Measure = (t, size) => t.length * size * 0.55;

function song(title: string, plays: number, unreleased = false): TopSong {
  return { key: `n:${title}`, trackId: null, title, artist: "Band", album: null, plays, ms: plays * 180_000, unreleased, track: null };
}

const personality: Personality = { time: "night", timeShare: 0.58, repeatShare: 0.64, repeats: true, newShare: 0.3, mostlyNew: false };

function year(patch: Partial<Wrapped> = {}): Wrapped {
  const hours = Array.from({ length: 24 }, (_, h) => (h === 23 ? 900_000 : h === 9 ? 300_000 : 0));
  return {
    year: 2025,
    totals: { ms: 41_204 * 60_000, plays: 9000, songs: 800, artists: 120, albums: 300, days: 300 },
    prevMs: 35_000 * 60_000,
    topArtists: [{ key: "a", artistId: null, name: "Ada Ferro", thumb: "/a/1", plays: 1200, ms: 4_000_000 }],
    topSongs: [song("Night Bus", 86), song("Secret Demo", 70, true), song("Harbour", 40)],
    topAlbums: [{ key: "b", albumId: null, title: "Low Light", artist: "Ada Ferro", thumb: null, plays: 300, ms: 1, unreleased: false }],
    topGenres: [
      { name: "Indie", plays: 3000, ms: 1, thumb: null },
      { name: "Jazz", plays: 1000, ms: 1, thumb: null },
      { name: "Soul", plays: 900, ms: 1, thumb: null },
    ],
    personality,
    hours,
    biggestDay: { key: "2025-03-14", plays: 84, ms: (6 * 60 + 12) * 60_000 },
    replayed: { song: song("Harbour", 11), day: "2025-06-02", at: null },
    firstSong: null,
    newArtists: 12,
    ...patch,
  };
}

describe("the year's look", () => {
  it("gives each year a colour that its neighbours don't share", () => {
    for (let y = 2015; y <= 2035; y++) {
      expect(yearTone(y)).not.toEqual(yearTone(y + 1));
      expect(yearTone(y)).not.toEqual(yearTone(y + 2));
      expect(yearTone(y)).toEqual(yearTone(y));
    }
    expect(yearTone(null).light).toBe("#ECE4D3");
  });

  it("cuts a record pattern seeded by the year", () => {
    const a = yearPattern(2025, 1200, 360);
    expect(yearPattern(2025, 1200, 360)).toEqual(a);
    expect(yearPattern(2024, 1200, 360)).not.toEqual(a);
    expect(a.bands).toBeGreaterThanOrEqual(4);
    expect(a.grooves.every((g) => g.r > a.label && g.r < a.radius)).toBe(true);
    // Quiet gaps between songs: as many wide steps as bands, less one.
    const steps = a.grooves.slice(1).map((g, i) => g.r - a.grooves[i].r);
    const pitch = Math.min(...steps);
    expect(steps.filter((s) => s > pitch * 2).length).toBe(a.bands - 1);
    // A squarer box keeps the record high on the right, clear of the type.
    const tile = yearPattern(2025, 400, 400);
    expect(tile.cx).toBeGreaterThan(280);
    expect(tile.cy).toBeLessThan(220);
  });
});

describe("search", () => {
  const ix = { years: [2025, 2024], allTime: true };
  it("finds the lists for top songs and wrapped", () => {
    expect(topSongsMatches("top songs", ix)).toEqual([2025, 2024, null]);
    expect(topSongsMatches("Your Top Songs", ix)).toEqual([2025, 2024, null]);
    expect(topSongsMatches("wrapped", ix)).toEqual([2025, 2024]);
    expect(topSongsMatches("top songs 2024", ix)).toEqual([2024]);
    expect(topSongsMatches("wrapped 2025", ix)).toEqual([2025]);
    expect(topSongsMatches("all time top songs", ix)).toEqual([2025, 2024, null]);
    expect(topSongsMatches("top so", ix)).toEqual([2025, 2024, null]);
  });

  it("stays out of other searches", () => {
    expect(topSongsMatches("top songs 1999", ix)).toEqual([]);
    expect(topSongsMatches("night bus", ix)).toEqual([]);
    expect(topSongsMatches("to", ix)).toEqual([]);
    expect(topSongsMatches("2025", ix)).toEqual([]);
    expect(topSongsMatches("top gun soundtrack", ix)).toEqual([]);
  });

  it("shows the lists above the results", async () => {
    render(<SearchPage query="top songs" />);
    const section = (await screen.findByRole("heading", { name: "Your Top Songs" })).closest("section")!;
    expect(within(section).getByRole("button", { name: "Your Top Songs 2025" })).toBeInTheDocument();
    expect(within(section).getByRole("button", { name: "All-Time Top Songs" })).toBeInTheDocument();
    await userEvent.setup().click(within(section).getByRole("button", { name: "Your Top Songs 2025" }));
    const { entries, index } = useRouter.getState();
    expect(entries[index].route).toEqual({ name: "topSongs", year: 2025 });
  });
});

describe("the words on the cards", () => {
  it("leads with time, the change and the top artist", () => {
    expect(daysAndHours((28 * 24 + 14) * 3_600_000)).toBe("28 days and 14 hours");
    expect(daysAndHours(3 * 3_600_000)).toBe("3 hours");
    expect(hoursAndMinutes((6 * 60 + 12) * 60_000)).toBe("6 hours and 12 minutes");
    expect(hoursAndMinutes(60 * 60_000)).toBe("1 hour");
    const m = minutesWords(year());
    expect(m.big).toBe("41,204");
    expect(m.lead).toBe("You spent 28 days and 14 hours with music in 2025.");
    expect(m.change).toBe("18% more than 2024.");
    expect(minutesWords(year({ prevMs: 50_000 * 60_000 })).change).toBe("18% less than 2024.");
    expect(minutesWords(year({ prevMs: 0 })).change).toBe("The first year in your history.");
    expect(artistWords(year())!.lead).toBe("1 in every 8 songs you played was by Ada Ferro.");
    expect(songWords(year())!).toEqual({ lead: "You played “Night Bus” 86 times.", sub: "16 more plays than anything else you played in 2025." });
    expect(andList(["Jazz", "Soul", "Folk"])).toBe("Jazz, Soul and Folk");
  });

  it("names the listening personality from its rules", () => {
    const p = personalityWords(personality, year().hours);
    expect(p.lead).toBe("You're a night listener: 58% of your minutes came between 10 PM and 5 AM.");
    expect(p.traits.map((t) => [t.value, t.label])).toEqual([
      ["11 PM", "Busiest hour"],
      ["64%", "On repeat"],
      ["70%", "Old favourites"],
    ]);
    expect(p.name).toEqual(["Night listener", "On repeat"]);
    const q = personalityWords({ ...personality, time: "morning", repeats: false, repeatShare: 0.2, mostlyNew: true, newShare: 0.75 });
    expect(q.traits.map((t) => t.label)).toEqual(["Morning listener", "Deep cuts", "New to you"]);
    expect(q.traits[1].value).toBe("80%");
    expect(busiestHour([0, 5, 5, 1])).toBe(1);
    expect(busiestHour([0, 0])).toBeNull();
  });

  it("finds the biggest day and the song replayed most in a day", () => {
    const d = dayWords(year())!;
    expect(d.lead).toBe("March 14 was your biggest day: 6 hours and 12 minutes.");
    expect(d.replay).toBe("On June 2 you played “Harbour” 11 times.");
    expect(dayWords(year({ biggestDay: null }))).toBeNull();
  });

  it("leaves out cards with nothing behind them", () => {
    expect(wrappedCards(year())).toEqual(["minutes", "artists", "songs", "albums", "personality", "day", "summary"]);
    expect(wrappedCards(year({ topAlbums: [], personality: null }))).toEqual(["minutes", "artists", "songs", "day", "summary"]);
    expect(wrappedCards(year({ totals: { ms: 0, plays: 0, songs: 0, artists: 0, albums: 0, days: 0 } }))).toEqual([]);
  });
});

describe("the summary card", () => {
  it("keeps unreleased songs off unless they're turned on", () => {
    const off = summaryContent(year());
    expect(off.songs.map((s) => s.title)).toEqual(["Night Bus", "Harbour"]);
    expect(off.withUnreleased).toBe(false);
    const on = summaryContent(year(), true);
    expect(on.songs.map((s) => s.title)).toEqual(["Night Bus", "Secret Demo", "Harbour"]);
    const texts = layoutSummaryCard(off, measure).texts.flatMap((t) => t.lines);
    expect(texts.some((l) => l.includes("Secret Demo"))).toBe(false);
  });

  it("lays out a 1080 by 1920 story with everything inside it and clear of the mark", () => {
    const c = summaryContent(year({ topSongs: Array.from({ length: 5 }, (_, i) => song(`A rather long song title number ${i + 1} that needs cutting`, 50 - i)) }));
    const l = layoutSummaryCard(c, measure);
    expect([l.width, l.height]).toEqual([CARD_SIZES.story.width, CARD_SIZES.story.height]);
    expect(l.numerals.text).toBe("2025");
    expect(l.band).toEqual({ x: 0, y: 0, w: 1080, h: 640 });
    for (const t of l.texts) {
      expect(t.x).toBeGreaterThanOrEqual(80);
      const last = t.y + (t.lines.length - 1) * t.size * t.lineHeight;
      expect(last).toBeLessThan(l.mark.y);
      for (const line of t.lines) expect(t.x + measure(line, t.size, t.weight)).toBeLessThanOrEqual(1080 - 80 + 1);
    }
    const lines = l.texts.flatMap((t) => t.lines);
    expect(lines).toContain("41,204");
    expect(lines).toContain("minutes listened · +18% on 2024");
    expect(lines).toContain("Night listener");
    expect(lines).toContain("On repeat");
    expect(lines.filter((x) => x.endsWith("…")).length).toBeGreaterThan(0);
    // The year sits under the record's label.
    const label = { top: l.pattern.cy - l.pattern.label, left: l.pattern.cx - l.pattern.label };
    const numeralsTop = l.numerals.y - l.numerals.size * 0.72;
    const numeralsRight = l.numerals.x + measure("2025", l.numerals.size, 800) * 0.85;
    expect(numeralsTop > label.top + l.pattern.label * 2 || numeralsRight < label.left).toBe(true);
  });
});

describe("Top Songs list", () => {
  it("ranks by the # column, with labels under titles where they belong", async () => {
    const real = await backend.topSongsList(2025);
    const list = { ...real, rows: real.rows.map((r, i) => ({ ...r, firstOfYear: i === 2 })) };
    vi.spyOn(backend, "topSongsList").mockResolvedValue(list);
    render(<TopSongsPage year={2025} />);
    expect(await screen.findByRole("heading", { level: 1, name: "Your Top Songs 2025" })).toBeInTheDocument();
    expect(screen.getByText("The songs you loved most this year")).toBeInTheDocument();
    expect(screen.getByText("Made for listener")).toBeInTheDocument();
    expect(screen.getByText(`${list.rows.length} songs,`)).toBeInTheDocument();
    const rows = await screen.findAllByRole("row");
    // The header row, then one per song; the number is the rank.
    expect(rows.length).toBe(list.rows.length + 1);
    expect(within(rows[1]).getByText("1")).toBeInTheDocument();
    expect(rows[1]).toHaveAccessibleName(new RegExp(`^${list.rows[0].track.title}.*number 1`));
    expect(screen.queryByText(/^\d+ plays?$/)).not.toBeInTheDocument();
    expect(within(rows[3]).getByText("First play of the year")).toBeInTheDocument();
    expect(screen.getAllByText("First play of the year")).toHaveLength(1);
  });

  it("shows plays on yearly lists when that's on, and always on All-Time", async () => {
    await settings({ topSongsShowPlays: true });
    const { unmount } = render(<TopSongsPage year={2025} />);
    await screen.findByRole("heading", { level: 1, name: "Your Top Songs 2025" });
    expect((await screen.findAllByText(/^\d+ plays?$/)).length).toBeGreaterThan(50);
    unmount();
    await settings();
    const all = await backend.topSongsList(null);
    render(<TopSongsPage year={null} />);
    await screen.findByRole("heading", { level: 1, name: "All-Time Top Songs" });
    expect(await screen.findAllByText(/^\d+ plays?$/)).toHaveLength(all.rows.length);
    expect(screen.queryByText("First play of the year")).not.toBeInTheDocument();
  });

  it("marks unreleased songs, and finds in the list keeping each rank", () => {
    const rows = [
      { track: { title: "Night Bus", artistTitle: "Ada", albumTitle: "Low", trackArtist: null }, plays: 9, ms: 1, unreleased: false, firstOfYear: true },
      { track: { title: "Harbour", artistTitle: "Ada", albumTitle: "Low", trackArtist: null }, plays: 1, ms: 1, unreleased: true, firstOfYear: false },
    ] as unknown as Parameters<typeof findRows>[0];
    expect(rowNotes(rows[0], { allTime: false, showPlays: false })).toEqual(["First play of the year"]);
    expect(rowNotes(rows[1], { allTime: true, showPlays: false })).toEqual(["1 play", "Unreleased"]);
    expect(findRows(rows, "harb").map((r) => r.rank)).toEqual([2]);
    expect(findRows(rows, "").map((r) => r.rank)).toEqual([1, 2]);
  });

  it("finds in the list from the action row", async () => {
    const user = userEvent.setup();
    const list = await backend.topSongsList(2025);
    const target = list.rows[7].track;
    render(<TopSongsPage year={2025} />);
    await screen.findByRole("heading", { level: 1, name: "Your Top Songs 2025" });
    await user.click(await screen.findByRole("button", { name: "Find in Your Top Songs 2025" }));
    await user.type(screen.getByRole("searchbox", { name: "Find in Your Top Songs 2025" }), target.title);
    const rows = screen.getAllByRole("row").slice(1);
    expect(rows.length).toBeLessThan(list.rows.length);
    const hit = rows.find((r) => within(r).queryByText(target.title));
    expect(hit && within(hit).getByText("8")).toBeTruthy();
  });

  it("saves to Plex as a normal playlist, in rank order", async () => {
    const user = userEvent.setup();
    const list = await backend.topSongsList(2025);
    const create = vi.spyOn(backend, "playlistCreate");
    render(<TopSongsPage year={2025} />);
    await screen.findAllByRole("row");
    await user.click(await screen.findByRole("button", { name: "Save to Plex as a playlist" }));
    await waitFor(() => expect(create).toHaveBeenCalledWith("Your Top Songs 2025", list.rows.map((r) => r.track.id)));
    const toasts = useToasts.getState().toasts;
    expect(toasts[toasts.length - 1]?.message).toBe("Saved to Plex as a playlist: Your Top Songs 2025");
  });

  it("plays from a row in the list's own order", async () => {
    const user = userEvent.setup();
    const list = await backend.topSongsList(2025);
    const play = vi.spyOn(player, "playMix").mockResolvedValue(undefined);
    render(<TopSongsPage year={2025} />);
    await screen.findAllByRole("row");
    await user.click(screen.getByRole("button", { name: "Play Your Top Songs 2025" }));
    expect(play).toHaveBeenCalledWith("topSongs:2025", list.rows.map((r) => r.track.id), undefined);
  });

  it("explains a year without a list", async () => {
    render(<TopSongsPage year={1999} />);
    expect(await screen.findByText("There's no Top Songs list for 1999. A year needs 30 plays for one.")).toBeInTheDocument();
  });
});

describe("Home", () => {
  const ix = { years: [2026, 2025], allTime: true, wrappedYears: [2026, 2025], currentYear: 2026 };

  it("shows the new list while the core promotes it", async () => {
    vi.spyOn(backend, "topSongsIndex").mockResolvedValue({ ...ix, promoted: 2026 });
    render(<HomePage />);
    const promo = await screen.findByTestId("top-songs-promo");
    expect(within(promo).getByRole("heading", { name: "Your Top Songs 2026 is here" })).toBeInTheDocument();
    await userEvent.setup().click(within(promo).getByRole("button", { name: "2026 Wrapped" }));
    expect(useWrapped.getState().year).toBe(2026);
  });

  it("keeps Top Songs off Home the rest of the year", async () => {
    const index = vi.spyOn(backend, "topSongsIndex").mockResolvedValue({ ...ix, promoted: null });
    render(<HomePage />);
    await screen.findByRole("heading", { name: "Made for you" });
    await waitFor(() => expect(index).toHaveBeenCalled());
    expect(screen.queryByTestId("top-songs-promo")).not.toBeInTheDocument();
    // (A list saved as a playlist is an ordinary playlist and may show; the lists themselves don't.)
    expect(document.querySelector(".ts-tile, .ts-promo")).toBeNull();
  });
});

describe("settings", () => {
  it("Wrapped and Top Songs can be turned off", async () => {
    await settings({ wrappedTopSongs: false });
    const user = userEvent.setup();
    render(<AppMenu />);
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    expect(screen.queryByRole("menuitem", { name: /Wrapped$/ })).not.toBeInTheDocument();
    await expect(backend.topSongsIndex()).rejects.toMatchObject({ kind: "invalid" });
  });

  it("the profile menu opens this year's Wrapped any time", async () => {
    const user = userEvent.setup();
    render(<AppMenu />);
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    await user.click(screen.getByRole("menuitem", { name: `${new Date().getFullYear()} Wrapped` }));
    expect(useWrapped.getState().year).toBe(new Date().getFullYear());
  });

  it("defaults: on, and no plays under titles", () => {
    expect(DEFAULT_SETTINGS.wrappedTopSongs).toBe(true);
    expect(DEFAULT_SETTINGS.topSongsShowPlays).toBe(false);
  });
});

describe("Wrapped cards", () => {
  async function open(w: Wrapped = year()) {
    vi.spyOn(backend, "statsWrapped").mockResolvedValue(w);
    const onClose = vi.fn();
    render(<WrappedView year={w.year} onClose={onClose} />);
    const view = await screen.findByTestId("wrapped");
    await waitFor(() => expect(view).toHaveAttribute("data-card", "minutes"));
    return { view, onClose, stage: view.querySelector(".wrapped__stage") as HTMLElement };
  }
  const key = (k: string) => fireEvent.keyDown(window, { key: k });
  const tick = (ms: number) => act(() => void vi.advanceTimersByTime(ms));

  it("leads with the minutes, as a dialog named for the year", async () => {
    const { view } = await open();
    expect(view).toHaveAttribute("role", "dialog");
    expect(view).toHaveAccessibleName("2025 Wrapped");
    expect(screen.getByRole("heading", { name: "You spent 28 days and 14 hours with music in 2025." })).toBeInTheDocument();
    expect(screen.getByText("41,204")).toBeInTheDocument();
  });

  it("moves on by itself, and with the arrow keys both ways", async () => {
    const { view } = await open();
    vi.useFakeTimers();
    key("ArrowRight");
    expect(view).toHaveAttribute("data-card", "artists");
    tick(CARD_MS - 50);
    expect(view).toHaveAttribute("data-card", "artists");
    tick(100);
    expect(view).toHaveAttribute("data-card", "songs");
    key("ArrowLeft");
    expect(view).toHaveAttribute("data-card", "artists");
    key("ArrowLeft");
    key("ArrowLeft");
    expect(view).toHaveAttribute("data-card", "minutes");
  });

  it("a click moves on, a click on the left third goes back", async () => {
    const { view, stage } = await open();
    stage.getBoundingClientRect = () => ({ left: 0, top: 0, width: 900, height: 600, right: 900, bottom: 600, x: 0, y: 0, toJSON: () => ({}) });
    fireEvent.pointerDown(stage, { button: 0, clientX: 700 });
    fireEvent.pointerUp(stage, { button: 0, clientX: 700 });
    expect(view).toHaveAttribute("data-card", "artists");
    fireEvent.pointerDown(stage, { button: 0, clientX: 100 });
    fireEvent.pointerUp(stage, { button: 0, clientX: 100 });
    expect(view).toHaveAttribute("data-card", "minutes");
  });

  it("holding pauses, and letting go goes on from there without skipping", async () => {
    const { view, stage } = await open();
    vi.useFakeTimers();
    key("ArrowRight");
    tick(CARD_MS / 2);
    fireEvent.pointerDown(stage, { button: 0, clientX: 700 });
    tick(HOLD_MS + 10);
    expect(view).toHaveAttribute("data-held", "true");
    expect(view).toHaveAttribute("data-paused", "true");
    tick(CARD_MS * 3);
    expect(view).toHaveAttribute("data-card", "artists");
    fireEvent.pointerUp(stage, { button: 0, clientX: 700 });
    expect(view).toHaveAttribute("data-held", "false");
    expect(view).toHaveAttribute("data-card", "artists");
    // What was left of the card, not a fresh start.
    tick(CARD_MS / 2 + 50);
    expect(view).toHaveAttribute("data-card", "songs");
  });

  it("Space pauses and resumes; Esc closes", async () => {
    const { view, onClose } = await open();
    vi.useFakeTimers();
    key(" ");
    expect(view).toHaveAttribute("data-paused", "true");
    tick(CARD_MS * 2);
    expect(view).toHaveAttribute("data-card", "minutes");
    key(" ");
    tick(CARD_MS + 10);
    expect(view).toHaveAttribute("data-card", "artists");
    key("Escape");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("stays still under reduced motion", async () => {
    document.documentElement.dataset.reducedMotion = "true";
    const { view } = await open();
    vi.useFakeTimers();
    expect(view).toHaveAttribute("data-paused", "true");
    tick(CARD_MS * 3);
    expect(view).toHaveAttribute("data-card", "minutes");
    expect(screen.queryByRole("button", { name: "Pause" })).not.toBeInTheDocument();
    key("ArrowRight");
    expect(view).toHaveAttribute("data-card", "artists");
  });

  it("plays a top song from its card", async () => {
    const tracks = (await backend.topSongsList(2025)).rows.slice(0, 2).map((r) => r.track);
    const songs = tracks.map((t, i): TopSong => ({ ...song(t.title, 50 - i), trackId: t.id, track: t }));
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    const { view } = await open(year({ topSongs: songs }));
    key("ArrowRight");
    key("ArrowRight");
    expect(view).toHaveAttribute("data-card", "songs");
    await userEvent.setup().click(screen.getByRole("button", { name: `Play ${tracks[1].title} by Band` }));
    expect(play).toHaveBeenCalledWith([tracks[0].id, tracks[1].id], tracks[1].id);
    expect(view).toHaveAttribute("data-card", "songs");
  });

  it("marks unreleased songs on the cards and leaves them off the image until turned on", async () => {
    const { view } = await open();
    for (let n = 0; n < 6; n++) key("ArrowRight");
    expect(view).toHaveAttribute("data-card", "summary");
    const canvas = screen.getByRole("img", { name: "Your 2025 Wrapped as an image" });
    expect(canvas).toHaveAttribute("data-unreleased", "off");
    expect(screen.getByText("Unreleased songs are left off the image.")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("switch", { name: "Include unreleased songs" }));
    expect(canvas).toHaveAttribute("data-unreleased", "on");
    key("ArrowLeft");
    key("ArrowLeft");
    key("ArrowLeft");
    key("ArrowLeft");
    expect(view).toHaveAttribute("data-card", "songs");
    expect(screen.getByText("Unreleased")).toBeInTheDocument();
  });

  describe("on the phone", () => {
    beforeEach(() => usePhoneForm.setState({ phone: true }));
    afterEach(() => usePhoneForm.setState({ phone: false }));
    const box = (el: HTMLElement) => {
      el.getBoundingClientRect = () => ({ left: 0, top: 0, width: 390, height: 844, right: 390, bottom: 844, x: 0, y: 0, toJSON: () => ({}) });
    };

    it("fills the screen with a close mark at the top right; pause and the arrows stay only for keyboards and screen readers", async () => {
      const { view, onClose } = await open();
      expect(view).toHaveClass("wrapped--phone");
      for (const name of ["Previous card", "Next card", "Pause"]) expect(screen.getByRole("button", { name })).toHaveClass("wrapped__quiet");
      fireEvent.click(screen.getByRole("button", { name: "Next card" }));
      expect(view).toHaveAttribute("data-card", "artists");
      fireEvent.click(screen.getByRole("button", { name: "Pause" }));
      expect(view).toHaveAttribute("data-paused", "true");
      await userEvent.setup().click(screen.getByRole("button", { name: "Close" }));
      expect(onClose).toHaveBeenCalledOnce();
    });

    it("a tap on the right third goes on, on the left third goes back", async () => {
      const { view, stage } = await open();
      box(stage);
      fireEvent.pointerDown(stage, { button: 0, pointerId: 1, clientX: 330, clientY: 400 });
      fireEvent.pointerUp(stage, { button: 0, pointerId: 1, clientX: 330, clientY: 400 });
      expect(view).toHaveAttribute("data-card", "artists");
      fireEvent.pointerDown(stage, { button: 0, pointerId: 2, clientX: 40, clientY: 400 });
      fireEvent.pointerUp(stage, { button: 0, pointerId: 2, clientX: 40, clientY: 400 });
      expect(view).toHaveAttribute("data-card", "minutes");
    });

    it("a pull down closes; a short one snaps back", async () => {
      const { view, stage, onClose } = await open();
      box(stage);
      // A slow finger: no flick.
      let t = 0;
      vi.spyOn(performance, "now").mockImplementation(() => (t += 200));
      fireEvent.pointerDown(stage, { button: 0, pointerId: 1, clientX: 200, clientY: 300 });
      fireEvent.pointerMove(stage, { pointerId: 1, clientX: 200, clientY: 330 });
      fireEvent.pointerMove(stage, { pointerId: 1, clientX: 200, clientY: 340 });
      expect(view).toHaveClass("is-pulled");
      fireEvent.pointerUp(stage, { button: 0, pointerId: 1, clientX: 200, clientY: 340 });
      expect(onClose).not.toHaveBeenCalled();
      expect(view).not.toHaveClass("is-pulled");
      expect(view).toHaveAttribute("data-card", "minutes");
      fireEvent.pointerDown(stage, { button: 0, pointerId: 2, clientX: 200, clientY: 300 });
      fireEvent.pointerMove(stage, { pointerId: 2, clientX: 200, clientY: 400 });
      fireEvent.pointerMove(stage, { pointerId: 2, clientX: 200, clientY: 620 });
      fireEvent.pointerUp(stage, { button: 0, pointerId: 2, clientX: 200, clientY: 620 });
      expect(onClose).toHaveBeenCalledOnce();
    });

    it("a swipe in from the left edge and the phone's Back both close", async () => {
      const { stage, onClose } = await open();
      box(stage);
      fireEvent.pointerDown(stage, { button: 0, pointerId: 1, clientX: 6, clientY: 400 });
      fireEvent.pointerMove(stage, { pointerId: 1, clientX: 60, clientY: 402 });
      fireEvent.pointerMove(stage, { pointerId: 1, clientX: 220, clientY: 404 });
      fireEvent.pointerUp(stage, { button: 0, pointerId: 1, clientX: 220, clientY: 404 });
      expect(onClose).toHaveBeenCalledOnce();
      key("GoBack");
      expect(onClose).toHaveBeenCalledTimes(2);
    });

    it("the close mark closes, and the header strip is never a tap zone", async () => {
      const { view, stage, onClose } = await open();
      box(stage);
      const bar = view.querySelector(".wrapped__bar") as HTMLElement;
      bar.getBoundingClientRect = () => ({ left: 0, top: 0, width: 390, height: 120, right: 390, bottom: 120, x: 0, y: 0, toJSON: () => ({}) });
      // A press in the strip that reaches the cards (beside the title) doesn't page.
      fireEvent.pointerDown(stage, { button: 0, pointerId: 1, clientX: 360, clientY: 90 });
      fireEvent.pointerUp(stage, { button: 0, pointerId: 1, clientX: 360, clientY: 90 });
      expect(view).toHaveAttribute("data-card", "minutes");
      // The X keeps its press to itself and closes.
      const close = screen.getByRole("button", { name: "Close" });
      fireEvent.pointerDown(close, { button: 0, pointerId: 2, clientX: 360, clientY: 90 });
      fireEvent.pointerUp(close, { button: 0, pointerId: 2, clientX: 360, clientY: 90 });
      fireEvent.click(close);
      expect(onClose).toHaveBeenCalledOnce();
      expect(view).toHaveAttribute("data-card", "minutes");
    });

    it("holds the page under it still, takes its touches, and puts things back on close", async () => {
      const phoneEl = document.createElement("div");
      phoneEl.className = "phone";
      const page = document.createElement("div");
      page.className = "ph-page";
      phoneEl.appendChild(page);
      document.body.appendChild(phoneEl);
      Object.defineProperty(page, "scrollTop", { value: 0, writable: true, configurable: true });
      vi.spyOn(backend, "statsWrapped").mockResolvedValue(year());
      const { unmount } = render(<WrappedView year={2025} onClose={() => {}} />, { container: phoneEl.appendChild(document.createElement("div")) });
      const view = await screen.findByTestId("wrapped");
      expect(phoneEl).toHaveClass("is-under-full");
      expect(document.documentElement.style.overflow).toBe("hidden");
      const move = new Event("touchmove", { bubbles: true, cancelable: true });
      view.dispatchEvent(move);
      expect(move.defaultPrevented).toBe(true);
      // Something scrolled the page anyway (a focus, a bounce): it goes back.
      page.scrollTop = 370;
      unmount();
      expect(phoneEl).not.toHaveClass("is-under-full");
      expect(document.documentElement.style.overflow).toBe("");
      expect(page.scrollTop).toBe(0);
      phoneEl.remove();
    });

    it("asks for light status bar text while it shows, and gives the look its own back after", async () => {
      const style = vi.spyOn(backend, "setStatusBarStyle").mockResolvedValue(undefined);
      document.documentElement.dataset.themeBase = "light";
      const stop = followStatusBar();
      expect(style).toHaveBeenLastCalledWith(true);
      const { unmount } = render(<WrappedView year={2025} onClose={() => {}} />);
      expect(style).toHaveBeenLastCalledWith(false);
      unmount();
      expect(style).toHaveBeenLastCalledWith(true);
      stop();
      delete document.documentElement.dataset.themeBase;
    });

    it("one finger at a time: a second finger is ignored, and only the first one's cancel counts", async () => {
      const { view, stage } = await open();
      box(stage);
      fireEvent.pointerDown(stage, { button: 0, pointerId: 1, clientX: 330, clientY: 400 });
      fireEvent.pointerDown(stage, { button: 0, pointerId: 2, clientX: 40, clientY: 400 });
      fireEvent.pointerCancel(stage, { pointerId: 2 });
      fireEvent.pointerUp(stage, { button: 0, pointerId: 2, clientX: 40, clientY: 400 });
      expect(view).toHaveAttribute("data-card", "minutes");
      fireEvent.pointerUp(stage, { button: 0, pointerId: 1, clientX: 330, clientY: 400 });
      expect(view).toHaveAttribute("data-card", "artists");
    });

    it("under reduced motion the cards don't follow the finger, but a pull down still closes", async () => {
      document.documentElement.dataset.reducedMotion = "true";
      const { view, stage, onClose } = await open();
      box(stage);
      fireEvent.pointerDown(stage, { button: 0, pointerId: 1, clientX: 200, clientY: 300 });
      fireEvent.pointerMove(stage, { pointerId: 1, clientX: 200, clientY: 400 });
      fireEvent.pointerMove(stage, { pointerId: 1, clientX: 200, clientY: 620 });
      expect(view).not.toHaveClass("is-pulled");
      expect(view.style.transform).toBe("");
      fireEvent.pointerUp(stage, { button: 0, pointerId: 1, clientX: 200, clientY: 620 });
      expect(onClose).toHaveBeenCalledOnce();
    });

    it("a sideways swipe away from the edge pages", async () => {
      const { view, stage, onClose } = await open();
      box(stage);
      fireEvent.pointerDown(stage, { button: 0, pointerId: 1, clientX: 300, clientY: 400 });
      fireEvent.pointerMove(stage, { pointerId: 1, clientX: 250, clientY: 402 });
      fireEvent.pointerMove(stage, { pointerId: 1, clientX: 120, clientY: 404 });
      fireEvent.pointerUp(stage, { button: 0, pointerId: 1, clientX: 120, clientY: 404 });
      expect(view).toHaveAttribute("data-card", "artists");
      expect(onClose).not.toHaveBeenCalled();
    });
  });

  it("says so when a year has nothing to wrap", async () => {
    vi.spyOn(backend, "statsWrapped").mockResolvedValue(year({ totals: { ms: 0, plays: 0, songs: 0, artists: 0, albums: 0, days: 0 } }));
    render(<WrappedView year={2025} onClose={() => {}} />);
    expect(await screen.findByRole("heading", { name: "Nothing to look back on in 2025 yet." })).toBeInTheDocument();
  });
});
