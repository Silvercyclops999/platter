import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { buildRecents, seedListens, type MockListen } from "../lib/mock/recents";
import type { RecentsEntry, Track } from "../lib/types";
import { RecentsPage } from "../pages/RecentsPage";
import { dayLabel, entryLines, groupByDay } from "../pages/recents";
import { AppMenu } from "../shell/TopBar";
import { useRouter } from "../stores/router";
import { body } from "./rows";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;
const MIN = 60_000;

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("Recents rules", () => {
  const now = new Date(2026, 9, 14, 15, 0);
  it("labels days Today, Yesterday, then dates", () => {
    expect(dayLabel(new Date(2026, 9, 14, 0, 5).getTime(), now)).toBe("Today");
    expect(dayLabel(new Date(2026, 9, 13, 23, 59).getTime(), now)).toBe("Yesterday");
    expect(dayLabel(new Date(2026, 9, 12, 9).getTime(), now)).toBe("Mon, Oct 12");
    expect(dayLabel(new Date(2025, 11, 30, 9).getTime(), now)).toBe("Tue, Dec 30, 2025");
  });

  it("groups newest-first entries by day", () => {
    const e = (at: Date): RecentsEntry => ({ kind: "songs", at: at.getTime(), tracks: [] });
    const groups = groupByDay([e(new Date(2026, 9, 14, 12)), e(new Date(2026, 9, 14, 9)), e(new Date(2026, 9, 13, 20)), e(new Date(2026, 9, 10, 8))], now);
    expect(groups.map((g) => [g.label, g.entries.length])).toEqual([
      ["Today", 2],
      ["Yesterday", 1],
      ["Sat, Oct 10", 1],
    ]);
  });

  it("writes each row's lines by kind", () => {
    const [t] = mock.fx.tracks;
    const album = mock.fx.albums[0];
    expect(entryLines({ kind: "context", item: { kind: "album", album }, at: 0, tracks: [t, t] })).toEqual({
      title: album.title,
      sub: `Album · ${album.artistTitle} · 2 songs played`,
    });
    expect(entryLines({ kind: "songs", at: 0, tracks: [t] })).toEqual({ title: null, sub: "1 song played" });
    expect(entryLines({ kind: "likedAdded", at: 0, tracks: [t] })).toEqual({ title: "Liked Songs", sub: "1 song added" });
    expect(entryLines({ kind: "liked", at: 0, tracks: [t, t, t] }).sub).toBe("Playlist · 3 songs played");
  });

  it("splits plays into sessions by context and 30 minute gaps, and keeps repeats", () => {
    const fx = mock.fx;
    const album = fx.albums[0];
    const [a, b] = fx.tracks.filter((t) => t.albumId === album.id);
    const other = fx.tracks.find((t) => t.albumId !== album.id)!;
    const byId = (id: string) => fx.tracks.find((t) => t.id === id);
    const at = Date.UTC(2026, 9, 14, 12);
    const play = (trackId: string, m: number, kind: MockListen["kind"], id?: string): MockListen => ({ trackId, at: at + m * MIN, end: at + (m + 3) * MIN, kind, id });
    const log = [
      play(a.id, 0, "album", album.id),
      play(b.id, 3, "album", album.id),
      play(a.id, 6, "album", album.id),
      play(other.id, 9, "songs"),
      play(a.id, 60, "album", album.id),
    ];
    const got = buildRecents(fx, log, [] as Track[], byId, at + 2 * 60 * MIN);
    expect(got.map((e) => [e.kind, e.tracks.map((t) => t.id)])).toEqual([
      ["context", [a.id]],
      ["songs", [other.id]],
      ["context", [a.id, b.id, a.id]],
    ]);
  });
});

describe("Recents page", () => {
  it("shows day groups of sessions that open to their songs", async () => {
    // At noon, so the seeded sessions land on Today and Yesterday whenever this runs.
    const noon = new Date(2026, 9, 14, 12).getTime();
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(noon);
    const byId = (id: string) => mock.fx.tracks.find((t) => t.id === id);
    const liked = mock.fx.tracks.filter((t) => t.userRating === 10).slice(0, 6);
    vi.spyOn(backend, "recents").mockResolvedValue(buildRecents(mock.fx, seedListens(mock.fx, liked.map((t) => t.id), noon), [], byId, noon));
    const user = userEvent.setup();
    render(<RecentsPage />);
    expect(screen.getByRole("heading", { level: 1, name: "Recents" })).toBeInTheDocument();
    const today = await screen.findByRole("region", { name: "Today" });
    expect(screen.getByRole("region", { name: "Yesterday" })).toBeInTheDocument();
    const rows = within(today).getAllByRole("listitem");
    expect(rows.length).toBeGreaterThan(1);
    // The newest session is an album: "Album · artist · N songs played".
    const album = mock.fx.albums.find((a) => within(rows[0]).queryByText(a.title));
    expect(album).toBeTruthy();
    expect(within(rows[0]).getByText(`Album · ${album!.artistTitle} · 3 songs played`)).toBeInTheDocument();
    // Liked Songs shows as its own session.
    expect(within(today).getAllByText("Liked Songs").length).toBeGreaterThan(0);

    const expand = within(rows[0]).getByRole("button", { name: `Show songs for ${album!.title}` });
    expect(expand).toHaveAttribute("aria-expanded", "false");
    await user.click(expand);
    expect(within(rows[0]).getByRole("button", { name: `Hide songs for ${album!.title}` })).toHaveAttribute("aria-expanded", "true");
    await waitFor(() => expect(body(within(rows[0]).getAllByRole("row")).filter((r) => r.classList.contains("track-row"))).toHaveLength(3));
    vi.useRealTimers();
  });

  it("says so when nothing has been played", async () => {
    vi.spyOn(backend, "recents").mockResolvedValue([]);
    render(<RecentsPage />);
    expect(await screen.findByText("Nothing played yet")).toBeInTheDocument();
  });

  it("says so when history can't be read", async () => {
    vi.spyOn(backend, "recents").mockRejectedValue({ kind: "internal", message: "database is locked" });
    render(<RecentsPage />);
    expect(await screen.findByText(/Couldn't load your recents/)).toBeInTheDocument();
  });

  it("opens from the app menu", async () => {
    const user = userEvent.setup();
    useRouter.getState().reset();
    render(<AppMenu />);
    await user.click(screen.getByRole("button", { name: /App menu/ }));
    await user.click(screen.getByRole("menuitem", { name: "Recents" }));
    expect(route()).toEqual({ name: "recents" });
  });
});
