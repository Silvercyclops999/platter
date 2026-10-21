/**
 * Every kind of playlist opens on the phone: Plex playlists, smart playlists
 * made in Platter, playlists shared with you and Blends. Each one's page
 * failing to load is faked and offers to try again.
 */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import { MOCK_BLEND_ID, MOCK_SHARED_ID } from "../lib/mock/circle";
import { usePhoneForm } from "../phone/form";
import type { MockBackend } from "../lib/mock";
import type { Playlist } from "../lib/types";
import { PhonePlaylist, shuffled } from "../phone/pages/Playlist";
import { PhoneShell } from "../phone/PhoneShell";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { player } from "../stores/player";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

const SLOW = { timeout: 6000 };

beforeEach(() => {
  act(() => {
    usePhoneForm.setState({ phone: true });
    usePhoneUi.setState({ sheets: [] });
    useToasts.setState({ toasts: [] });
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => usePhoneForm.setState({ phone: false }));
});

async function smartList(title: string, value = "0") {
  return backend.smartSave({
    title,
    rules: { matchAll: true, rules: [{ field: "playCount", op: "gt", value }], sort: "mostPlayed", update: "live" },
    platter: true,
  });
}

describe("playlists on the phone", () => {
  it("a Plex playlist still opens and plays as a playlist", async () => {
    const play = vi.spyOn(player, "playPlaylist").mockResolvedValue(undefined);
    render(<PhonePlaylist id="500" />);
    const pill = await screen.findByRole("button", { name: /^Play / }, SLOW);
    fireEvent.click(pill);
    expect(play).toHaveBeenCalledWith("500", undefined, undefined, undefined);
  });

  it("a smart playlist made in Platter opens with its songs and plays them", async () => {
    const saved = await smartList("Played ones");
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    render(<PhonePlaylist id={saved.id} />);
    expect(await screen.findByRole("heading", { name: "Played ones" }, SLOW)).toBeInTheDocument();
    expect(screen.getByText(/^Smart playlist, /)).toBeInTheDocument();
    const songs = (await backend.smartPage(saved.id)).tracks.map((t) => t.id);
    expect(songs.length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Play Played ones" }));
    expect(play).toHaveBeenCalledWith(songs, undefined);
  });

  it("a smart playlist nothing matches says so", async () => {
    const saved = await smartList("Nothing yet", "999999");
    render(<PhonePlaylist id={saved.id} />);
    expect(await screen.findByText("No songs match these rules right now.", undefined, SLOW)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Play Nothing yet" })).toBeDisabled();
  });

  it("a playlist shared with you opens with whose it is and plays the songs you have", async () => {
    const page = await backend.collabPage(MOCK_SHARED_ID);
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    render(<PhonePlaylist id={MOCK_SHARED_ID} />);
    expect(await screen.findByRole("heading", { name: page.summary.title }, SLOW)).toBeInTheDocument();
    expect(screen.getByText(new RegExp(`^Shared by ${page.summary.owner.name}, `))).toBeInTheDocument();
    const have = page.rows.flatMap((r) => (r.track ? [r.track.id] : []));
    const missing = page.rows.length - have.length;
    if (missing > 0) expect(screen.getByText(/in your library\.$/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: `Play ${page.summary.title}` }));
    await waitFor(() => expect(play).toHaveBeenCalledWith(have, undefined));
  });

  it("a Blend opens with everyone's songs and plays them", async () => {
    const page = await backend.blendPage(MOCK_BLEND_ID);
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    render(<PhonePlaylist id={MOCK_BLEND_ID} />);
    expect(await screen.findByRole("heading", { name: page.summary.title }, SLOW)).toBeInTheDocument();
    expect(screen.getByText(/^Blend, /)).toBeInTheDocument();
    const first = page.rows[0].track;
    fireEvent.click(screen.getAllByText(first.title)[0]);
    await waitFor(() => expect(play).toHaveBeenCalledWith(page.rows.map((r) => r.track.id), first.id));
  });

  it("each kind that fails to load says so and tries again", async () => {
    const saved = await smartList("Flaky");
    const cases: [string, keyof typeof backend, string][] = [
      [saved.id, "smartPage", "this smart playlist"],
      [MOCK_SHARED_ID, "collabPage", "this playlist"],
      [MOCK_BLEND_ID, "blendPage", "this Blend"],
    ];
    for (const [id, call, what] of cases) {
      const real = (backend[call] as (id: string) => Promise<unknown>).bind(backend);
      const spy = vi.spyOn(backend, call as "blendPage").mockRejectedValueOnce(new Error("gone")).mockImplementation((x: string) => real(x) as never);
      const { unmount } = render(<PhonePlaylist id={id} />);
      expect(await screen.findByText(`Couldn't load ${what}`, undefined, SLOW)).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: /try again/i }));
      await waitFor(() => expect(screen.queryByText(`Couldn't load ${what}`)).toBeNull(), SLOW);
      expect(spy).toHaveBeenCalledTimes(2);
      unmount();
      spy.mockRestore();
    }
  });

  it("shuffle plays the list once in a new order", () => {
    let n = 0;
    const order = shuffled(["a", "b", "c", "d"], () => [0.1, 0.9, 0.5][n++ % 3]);
    expect([...order].sort()).toEqual(["a", "b", "c", "d"]);
    expect(order).not.toEqual(["a", "b", "c", "d"]);
  });

  it("a change to a smart playlist's songs keeps the list on screen while it loads again", async () => {
    const saved = await smartList("Live one");
    render(<PhonePlaylist id={saved.id} />);
    await screen.findByRole("heading", { name: "Live one" }, SLOW);
    const again = vi.spyOn(backend, "smartPage");
    act(() => mock.emit("smart://changed", []));
    expect(screen.getByRole("heading", { name: "Live one" })).toBeInTheDocument();
    await waitFor(() => expect(again).toHaveBeenCalled());
    expect(screen.getByRole("heading", { name: "Live one" })).toBeInTheDocument();
  });

  it("shuffle with a genre picked plays only that genre", async () => {
    const page = await backend.blendPage(MOCK_BLEND_ID);
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    render(<PhonePlaylist id={MOCK_BLEND_ID} />);
    const pills = await screen.findByRole("group", { name: "Genres" }, SLOW);
    const pill = pills.querySelector("button")!;
    const genre = pill.textContent!;
    fireEvent.click(pill);
    fireEvent.click(screen.getByRole("button", { name: "Shuffle play" }));
    const want = page.rows.filter((r) => (r.track.genres ?? []).includes(genre)).map((r) => r.track.id);
    const [order, start] = play.mock.calls[0];
    expect([...order].sort()).toEqual([...want].sort());
    expect(start).toBe(order[0]);
  });
});

describe("the More sheet for each kind of playlist", () => {
  const list = (id: string, title: string): Playlist => ({ id, title, smart: id.startsWith("smart:"), trackCount: 3, durationMs: 1, thumb: null });

  it("a smart playlist made here is deleted as a smart playlist, never through Plex", async () => {
    const saved = await smartList("Gone soon");
    const plex = vi.spyOn(backend, "playlistDelete");
    const smart = vi.spyOn(backend, "smartDelete");
    render(<PhoneShell />);
    act(() => phone.push({ kind: "playlist", playlist: list(saved.id, "Gone soon") }));
    expect(screen.queryByRole("button", { name: "Share" })).toBeNull();
    fireEvent.click(await screen.findByRole("button", { name: "Delete playlist" }));
    await waitFor(() => expect(smart).toHaveBeenCalledWith(saved.id));
    expect(plex).not.toHaveBeenCalled();
  });

  it("shared playlists and Blends offer Add to queue, not a share card or delete", async () => {
    const page = await backend.blendPage(MOCK_BLEND_ID);
    const add = vi.spyOn(player, "addToQueue").mockResolvedValue(undefined as never);
    render(<PhoneShell />);
    act(() => phone.push({ kind: "playlist", playlist: list(MOCK_BLEND_ID, "Our Blend") }));
    expect(screen.queryByRole("button", { name: "Share" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Delete playlist" })).toBeNull();
    fireEvent.click(await screen.findByRole("button", { name: "Add to queue" }));
    await waitFor(() => expect(add).toHaveBeenCalled());
    expect(add.mock.calls[0][0].map((t) => t.id)).toEqual(page.rows.map((r) => r.track.id));
  });

  it("Add to queue that can't load the songs says so", async () => {
    vi.spyOn(backend, "collabPage").mockRejectedValue(new Error("gone"));
    render(<PhoneShell />);
    act(() => phone.push({ kind: "playlist", playlist: list(MOCK_SHARED_ID, "Road Trip") }));
    fireEvent.click(await screen.findByRole("button", { name: "Add to queue" }));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Couldn't add Road Trip to the queue"));
  });
});
