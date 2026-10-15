import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost, placeMenu } from "../components/ContextMenu";
import { TrackList, TrackRow, trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { PlayerBar } from "../shell/PlayerBar";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { useUi } from "../stores/ui";
import { listenAlong } from "../stores/social";

// Sessions are off in the app until the relay transport lands; keep covering them.
listenAlong.enabled = true;

const mock = backend as MockBackend;
const [t1] = mock.fx.tracks;
const labels = () => within(screen.getByRole("menu")).getAllByRole("menuitem").map((i) => i.textContent);

afterEach(() => {
  vi.restoreAllMocks();
  act(() => useUi.setState({ menu: null }));
  useRouter.getState().reset();
});

function row() {
  render(
    <>
      <TrackList label="Tracks">
        <TrackRow track={t1} index={1} onPlay={vi.fn()} />
      </TrackList>
      <ContextMenuHost />
    </>,
  );
  return screen.getByRole("row", { name: new RegExp(t1.title) });
}

describe("track menu", () => {
  it("leaves out 'Go to album' on that album's own page", async () => {
    useRouter.getState().reset({ name: "album", id: t1.albumId });
    fireEvent.contextMenu(row(), { clientX: 10, clientY: 10 });
    await screen.findByRole("menu");
    expect(labels()).toEqual(["Add to playlist", "Add to Liked Songs", "Add to queue", "Start a listening session", `Go to ${t1.artistTitle}`, "Go to song radio", "Bookmark", "Your stats", "Don't play this artist", "Add lyrics", "Share", "Copy song title"]);
  });

  it("keeps 'Go to album' everywhere else, including another album's page", async () => {
    useRouter.getState().reset({ name: "album", id: `${t1.albumId}-other` });
    fireEvent.contextMenu(row(), { clientX: 10, clientY: 10 });
    await screen.findByRole("menu");
    expect(labels()).toEqual(["Add to playlist", "Add to Liked Songs", "Add to queue", "Start a listening session", `Go to ${t1.artistTitle}`, "Go to album", "Go to song radio", "Bookmark", "Your stats", "Don't play this artist", "Add lyrics", "Share", "Copy song title"]);
  });

  it("queue items put 'Remove from queue' right after 'Add to queue'", () => {
    const items = trackMenuItems(t1, { afterQueue: [{ label: "Remove from queue" }] });
    expect(items.map((i) => i.label)).toEqual(["Add to playlist", "Add to Liked Songs", "Add to queue", "Remove from queue", "Start a listening session", "Add to the session's queue", `Go to ${t1.artistTitle}`, "Go to album", "Go to song radio", "Bookmark", "Your stats", "Don't play this artist", "Add lyrics", "Wrong lyrics?", "Share", "Copy song title"]);
  });

  it("the options button opens the menu aligned to the button", async () => {
    const r = row();
    fireEvent.click(within(r).getByRole("button", { name: new RegExp(`^More options for ${t1.title} by `) }));
    await screen.findByRole("menu");
    expect(useUi.getState().menu?.anchor).toBeDefined();
  });

  it("right-click opens at the pointer, with no anchor", async () => {
    fireEvent.contextMenu(row(), { clientX: 40, clientY: 50 });
    await screen.findByRole("menu");
    const m = useUi.getState().menu!;
    expect([m.x, m.y, m.anchor]).toEqual([40, 50, undefined]);
  });
});

describe("menu placement", () => {
  const view = { width: 1200, height: 800 };
  const size = { width: 220, height: 200 };

  it("puts the top-left corner at the pointer", () => {
    expect(placeMenu({ x: 300, y: 200 }, size, view)).toEqual({ x: 300, y: 200 });
  });

  it("flips up near the bottom, bottom edge at the pointer", () => {
    expect(placeMenu({ x: 300, y: 700 }, size, view)).toEqual({ x: 300, y: 500 });
  });

  it("stays on screen at the right edge", () => {
    expect(placeMenu({ x: 1150, y: 100 }, size, view).x).toBe(1200 - 220 - 8);
  });

  it("aligns to an options button: right edges in the right half, left edges in the left half", () => {
    const right = { left: 1000, right: 1032, top: 100, bottom: 132 };
    expect(placeMenu({ x: 1000, y: 132, anchor: right }, size, view)).toEqual({ x: 1032 - 220, y: 136 });
    const left = { left: 100, right: 132, top: 100, bottom: 132 };
    expect(placeMenu({ x: 100, y: 132, anchor: left }, size, view)).toEqual({ x: 100, y: 136 });
  });

  it("opens above the button when there's no room below", () => {
    const low = { left: 100, right: 132, top: 700, bottom: 732 };
    expect(placeMenu({ x: 100, y: 732, anchor: low }, size, view).y).toBe(700 - 4 - 200);
  });
});

describe("now-playing menu", () => {
  it("right-clicking the player-bar title opens the same menu as an album track", async () => {
    const base = await mock.playerState();
    act(() => usePlayer.getState().set({ ...base, track: t1 }));
    render(
      <>
        <PlayerBar />
        <ContextMenuHost />
      </>,
    );
    fireEvent.contextMenu(screen.getByRole("link", { name: t1.title }), { clientX: 20, clientY: 700 });
    const menu = await screen.findByRole("menu", { name: `Options for ${t1.title}` });
    expect(menu).toBeInTheDocument();
    expect(labels()).toEqual(["Add to playlist", "Add to Liked Songs", "Add to queue", "Start a listening session", `Go to ${t1.artistTitle}`, "Go to album", "Go to song radio", "Bookmark", "Your stats", "Don't play this artist", "Add lyrics", "Share", "Copy song title"]);
  });
});
