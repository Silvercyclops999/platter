/**
 * The level bars mark the playing song's row on the phone (album page,
 * playlists and liked songs, the queue, search), not the record's tile on
 * Home, which keeps only the accent on its title.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type PlayerState, type QueueItem, type QueueView, type Track } from "../lib/types";
import { usePhoneForm } from "../phone/form";
import { SongRow } from "../phone/pages/common";
import { PhonePage, PhoneShell } from "../phone/PhoneShell";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";

const mock = backend as MockBackend;
const tracks = (): Track[] => mock.fx.tracks;
const item = (t: Track, i: number): QueueItem => ({ itemId: `q${i}`, track: t, source: "context" });

function play(t: Track, context: QueueView["context"] = { kind: "album", id: t.albumId, title: t.albumTitle }) {
  const s: PlayerState = {
    status: "playing",
    track: t,
    positionMs: 10_000,
    durationMs: t.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: item(t, 0), userQueue: [], nextUp: [item(tracks()[1], 1)], history: [], context },
  };
  act(() => usePlayer.setState({ state: s, local: s, positionMs: s.positionMs, durationMs: s.durationMs, remote: null }));
}

const bars = (el: HTMLElement) => within(el).queryAllByRole("img", { name: "Playing" });

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  usePhoneUi.setState({ tab: "home", nowPlaying: false, lyrics: false, sheets: [] });
  useRouter.getState().reset({ name: "home" });
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
  vi.restoreAllMocks();
});

describe("the playing song's row", () => {
  it("numbered rows (an album) put the bars where the number was; rows with art put them before the title", () => {
    const [a, b] = tracks();
    play(a);
    const { container } = render(
      <ul>
        <SongRow track={a} index={1} art={false} onPlay={() => undefined} />
        <SongRow track={b} index={2} art={false} onPlay={() => undefined} />
        <SongRow track={a} onPlay={() => undefined} />
        <SongRow track={b} onPlay={() => undefined} />
      </ul>,
    );
    const rows = [...container.querySelectorAll<HTMLElement>("li.ph-song")];
    expect(bars(rows[0])).toHaveLength(1);
    expect(rows[0].querySelector(".ph-song__num .ph-eq")).toBeTruthy();
    expect(bars(rows[1])).toHaveLength(0);
    expect(bars(rows[2])).toHaveLength(1);
    expect(rows[2].querySelector(".ph-row__title.is-playing .ph-eq")).toBeTruthy();
    expect(bars(rows[3])).toHaveLength(0);
  });

  it("the album page marks the playing song", async () => {
    const t = tracks()[0];
    play(t);
    render(<PhonePage route={{ name: "album", id: t.albumId }} />);
    const list = (await screen.findAllByRole("list", { name: /songs$/i }))[0];
    await waitFor(() => expect(bars(list)).toHaveLength(1));
    expect(within(list).getByText(t.title)).toHaveClass("is-playing");
  });

  it("the queue's playing song wears the bars", async () => {
    const t = tracks()[0];
    play(t);
    render(<PhoneShell />);
    act(() => phone.push({ kind: "queue" }));
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    expect(bars(queue.querySelector<HTMLElement>(".ph-queue__now")!)).toHaveLength(1);
    expect(bars(within(queue).getByRole("region", { name: `Next from: ${t.albumTitle}` }))).toHaveLength(0);
  });

  it("search marks the playing song among its results", async () => {
    const t = tracks()[0];
    play(t);
    render(<PhonePage route={{ name: "search" }} />);
    fireEvent.change(screen.getByRole("searchbox", { name: "Search your library" }), { target: { value: t.title } });
    const results = await screen.findByRole("region", { name: "Results" }, { timeout: 4000 });
    await waitFor(() => expect(bars(results)).toHaveLength(1));
  });
});

describe("Home's shortcut tiles", () => {
  it("the playing record's tile keeps only the accent: no bars", async () => {
    const home = await backend.home();
    const album = home.shortcuts.find((i) => i.kind === "album");
    expect(album).toBeTruthy();
    if (album?.kind !== "album") return;
    const t = tracks().find((x) => x.albumId === album.album.id) ?? tracks()[0];
    play(t, { kind: "album", id: album.album.id, title: album.album.title });
    render(<PhonePage route={{ name: "home" }} />);
    const grid = await screen.findByRole("list", { name: "Recent and pinned" });
    const tile = within(grid).getByText(album.album.title).closest("button")!;
    expect(tile).toHaveClass("is-playing");
    expect(bars(grid)).toHaveLength(0);
  });
});
