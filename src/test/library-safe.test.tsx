/**
 * Your library is safe: the library line says all is fine and when it was
 * checked, or what left the server this week; the list of what left groups
 * songs by day, opening it marks them seen, a song leads to search to look
 * for it again, and a list that can't be read says so.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { GoneSong } from "../lib/types";
import { GonePage } from "../pages/GonePage";
import { PhoneGone } from "../phone/pages/Gone";
import { GoneNote } from "../shell/Sidebar";
import { useApp } from "../stores/app";
import { useRouter } from "../stores/router";
import { goneByDay, safetyLine, useSafety } from "../stores/safety";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;
const NOW = 1_800_000_000;

function song(n: number, goneAt: number, seen = false): GoneSong {
  return { id: `g${n}`, title: `Song ${n}`, albumId: "a1", albumTitle: "Harbor Lights", artistTitle: "Vesper Lowe", thumb: null, goneAt, seen };
}

function setGone(list: GoneSong[]) {
  (mock as unknown as { gone: GoneSong[] | null }).gone = list;
}

beforeEach(() => {
  useSafety.setState({ safety: null, gone: null, goneFailed: false });
  setGone([]);
});
afterEach(() => vi.restoreAllMocks());

describe("the library line", () => {
  it("says nothing before the first sync has finished", () => {
    expect(safetyLine({ songs: 0, playlists: 0 }, null, NOW)).toBeNull();
  });

  it("says the library is all here and when it was checked", () => {
    expect(safetyLine({ songs: 0, playlists: 0 }, NOW - 300, NOW)).toEqual({ alert: false, title: "Your library is all here", sub: "Checked 5 minutes ago" });
    expect(safetyLine(null, NOW - 30, NOW)?.sub).toBe("Checked just now");
  });

  it("says what left the server", () => {
    expect(safetyLine({ songs: 1, playlists: 0 }, NOW, NOW)?.title).toBe("1 song left your server");
    expect(safetyLine({ songs: 12, playlists: 1 }, NOW, NOW)?.title).toBe("12 songs and 1 playlist left your server");
    expect(safetyLine({ songs: 0, playlists: 2 }, NOW, NOW)).toEqual({ alert: true, title: "2 playlists left your server", sub: "See which" });
  });

  it("groups the list by the day songs went", () => {
    const days = goneByDay([song(1, NOW - 60), song(2, NOW - 120), song(3, NOW - 86_400 - 60), song(4, NOW - 9 * 86_400)], NOW);
    expect(days.map((d) => d.songs.length)).toEqual([2, 1, 1]);
    expect(days[0].day).toBe("Today");
    expect(days[1].day).toBe("Yesterday");
  });
});

describe("the list of what left", () => {
  it("lists songs by day, marks them seen when opened, and a song looks for itself in search", async () => {
    const user = userEvent.setup();
    const now = Math.floor(Date.now() / 1000);
    setGone([song(1, now - 60), song(2, now - 120)]);
    expect(await backend.librarySafety()).toEqual({ songs: 2, playlists: 0 });
    render(<PhoneGone />);
    const today = await screen.findByRole("region", { name: "Today" });
    expect(within(today).getByText("Song 1")).toBeInTheDocument();
    expect(within(today).getAllByText("Vesper Lowe · Harbor Lights")).toHaveLength(2);
    await waitFor(async () => expect(await backend.librarySafety()).toEqual({ songs: 0, playlists: 0 }));
    await user.click(screen.getByRole("button", { name: /^Song 2/ }));
    expect(route()).toEqual({ name: "search", query: "Song 2 Vesper Lowe" });
  });

  it("says so when nothing has left", async () => {
    render(<GonePage />);
    expect(await screen.findByText(/Nothing has left your server/)).toBeInTheDocument();
  });

  it("says the list couldn't be read, and doesn't mark anything seen", async () => {
    setGone([song(1, Math.floor(Date.now() / 1000) - 60)]);
    vi.spyOn(backend, "libraryGone").mockRejectedValue(new Error("database is locked"));
    const seen = vi.spyOn(backend, "libraryGoneSeen");
    render(<PhoneGone />);
    expect(await screen.findByText("Couldn't read the list")).toBeInTheDocument();
    expect(screen.queryByText(/Nothing has left/)).toBeNull();
    expect(seen).not.toHaveBeenCalled();
  });
});

describe("the sidebar note", () => {
  it("shows only while something left, and opens the list", async () => {
    const user = userEvent.setup();
    act(() => useApp.setState({ sync: { phase: "idle", done: 0, total: 0, lastSyncAt: Math.floor(Date.now() / 1000) - 60 } }));
    const { unmount } = render(<GoneNote />);
    await waitFor(() => expect(useSafety.getState().safety).not.toBeNull());
    expect(screen.queryByRole("button")).toBeNull();
    unmount();

    setGone([song(1, Math.floor(Date.now() / 1000) - 60)]);
    render(<GoneNote />);
    await user.click(await screen.findByRole("button", { name: "1 song left your server" }));
    expect(route()).toEqual({ name: "gone" });
  });

  it("keeps the last line when the check can't be read", async () => {
    act(() => useSafety.setState({ safety: { songs: 3, playlists: 0 } }));
    act(() => useApp.setState({ sync: { phase: "idle", done: 0, total: 0, lastSyncAt: Math.floor(Date.now() / 1000) - 60 } }));
    vi.spyOn(backend, "librarySafety").mockRejectedValue(new Error("offline"));
    render(<GoneNote />);
    expect(await screen.findByRole("button", { name: "3 songs left your server" })).toBeInTheDocument();
  });
});
