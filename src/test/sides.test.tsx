/**
 * Record sides in the UI: side headings in the tracklist, Play side from a
 * heading and the album menu (only that side is queued), Split into sides,
 * the two settings, and the record player's pause to flip with a fake player.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Album, type AlbumSides, type PlayerState, type Track } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { SettingsPage } from "../pages/SettingsPage";
import { FullWindow } from "../shell/FullWindow";
import { FULL_WINDOW_PREFS_DEFAULT, fullWindow, useFullWindow } from "../stores/fullWindow";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { ONLINE_EXTRAS } from "./onlineExtras";

const mock = backend as MockBackend;

async function setUp(patch: Record<string, unknown> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

/** An album with a vinyl edition's two sides, and one with none. */
async function albums(): Promise<{ vinyl: Album; sides: AlbumSides; plain: Album }> {
  let vinyl: Album | null = null;
  let sides: AlbumSides | null = null;
  let plain: Album | null = null;
  for (const a of mock.fx.albums) {
    const s = await backend.albumSides(a.id);
    const n = mock.fx.tracksByAlbum.get(a.id)?.length ?? 0;
    if (!vinyl && s.sides.length === 2 && s.source === "musicbrainz") {
      vinyl = a;
      sides = s;
    }
    if (!plain && !s.detected && n >= 3) plain = a;
    if (vinyl && plain) break;
  }
  return { vinyl: vinyl!, sides: sides!, plain: plain! };
}

const tracksOf = (id: string): Track[] => mock.fx.tracksByAlbum.get(id) ?? [];

async function openAlbum(album: Album) {
  const user = userEvent.setup();
  render(
    <>
      <AlbumPage id={album.id} />
      <ContextMenuHost />
    </>,
  );
  await screen.findByRole("heading", { level: 1, name: album.title });
  return user;
}

beforeEach(async () => {
  useRouter.getState().reset();
  mock.sides.reset();
  await setUp();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("side headings", () => {
  it("an album with a vinyl edition shows Side A and Side B before their songs", async () => {
    const { vinyl, sides } = await albums();
    await openAlbum(vinyl);
    const a = await screen.findByRole("row", { name: "Side A" });
    const b = screen.getByRole("row", { name: "Side B" });
    expect(within(a).getByText(`${sides.sides[0].trackIds.length} songs`, { exact: false })).toBeInTheDocument();
    // Side B's heading comes right before its first song.
    const firstB = tracksOf(vinyl.id).find((t) => t.id === sides.sides[1].trackIds[0])!;
    const rows = screen.getAllByRole("row");
    const at = rows.indexOf(b);
    expect(rows[at + 1]).toHaveTextContent(firstB.title);
  });

  it("an album with no vinyl edition has none", async () => {
    const { plain } = await albums();
    await openAlbum(plain);
    await screen.findAllByRole("row");
    expect(screen.queryByRole("row", { name: /^Side / })).toBeNull();
  });
});

describe("play side", () => {
  it("Play side B on its heading queues only side B, from the album", async () => {
    const { vinyl, sides } = await albums();
    const spy = vi.spyOn(backend, "playContext");
    const user = await openAlbum(vinyl);
    await user.click(await screen.findByRole("button", { name: "Play side B" }));
    await waitFor(() => expect(spy).toHaveBeenCalled());
    expect(spy.mock.calls[0][0]).toEqual({ context: { kind: "album", id: vinyl.id, trackIds: sides.sides[1].trackIds } });
    // The mock core plays it: side B's songs only.
    await waitFor(async () => expect((await backend.playerState()).track?.id).toBe(sides.sides[1].trackIds[0]));
    const { queue } = await backend.playerState();
    const ids = [queue.current?.track.id ?? sides.sides[1].trackIds[0], ...queue.nextUp.map((q) => q.track.id)];
    expect(ids).toEqual(sides.sides[1].trackIds);
  });

  it("the album menu has Play side A and Play side B", async () => {
    const { vinyl, sides } = await albums();
    const spy = vi.spyOn(backend, "playContext");
    const user = await openAlbum(vinyl);
    await screen.findByRole("row", { name: "Side A" });
    await user.click(screen.getByRole("button", { name: `More options for ${vinyl.title}` }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "Play side B" })).toBeInTheDocument();
    await user.click(within(menu).getByRole("menuitem", { name: "Play side A" }));
    await waitFor(() => expect(spy).toHaveBeenCalled());
    expect(spy.mock.calls[0][0].context.trackIds).toEqual(sides.sides[0].trackIds);
  });
});

describe("split into sides", () => {
  it("picks where side B starts, saves it and the tracklist follows", async () => {
    const { plain } = await albums();
    const tracks = tracksOf(plain.id);
    const set = vi.spyOn(backend, "setAlbumSides");
    const user = await openAlbum(plain);
    await screen.findAllByRole("row");
    await user.click(screen.getByRole("button", { name: `More options for ${plain.title}` }));
    await user.click(await screen.findByRole("menuitem", { name: "Split into sides…" }));
    const dialog = await screen.findByRole("dialog", { name: `Split ${plain.title} into sides` });
    expect(within(dialog).queryByRole("checkbox", { name: `A new side starts on ${tracks[0].title}` })).toBeNull();
    await user.click(within(dialog).getByRole("checkbox", { name: `A new side starts on ${tracks[2].title}` }));
    expect(within(dialog).getByRole("status")).toHaveTextContent("Sides A to B.");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(set).toHaveBeenCalledWith(plain.id, [tracks[2].id]);
    expect(await screen.findByRole("row", { name: "Side B" })).toBeInTheDocument();
    expect((await backend.albumSides(plain.id)).source).toBe("manual");
  });

  it("No sides takes a split away", async () => {
    const { plain } = await albums();
    const tracks = tracksOf(plain.id);
    await backend.setAlbumSides(plain.id, [tracks[1].id]);
    const user = await openAlbum(plain);
    await screen.findByRole("row", { name: "Side B" });
    await user.click(screen.getByRole("button", { name: `More options for ${plain.title}` }));
    await user.click(await screen.findByRole("menuitem", { name: "Split into sides…" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "No sides" }));
    await waitFor(() => expect(screen.queryByRole("row", { name: /^Side / })).toBeNull());
  });
});

describe("settings", () => {
  it("Show vinyl sides is on in Library; Pause to flip is off in Playback", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    // Both sit under their section's Advanced: on, and off, as most people leave them.
    const open = async (region: HTMLElement) => {
      const adv = within(region).queryByRole("button", { name: /^Advanced/ });
      if (adv && adv.getAttribute("aria-expanded") !== "true") await user.click(adv);
    };
    const library = await screen.findByRole("region", { name: "Library" });
    await open(library);
    const show = within(library).getByRole("switch", { name: "Show vinyl sides" });
    expect(show).toBeChecked();
    const playback = screen.getByRole("region", { name: "Playback" });
    await open(playback);
    const flip = within(playback).getByRole("switch", { name: "Pause to flip at the end of a side" });
    expect(flip).not.toBeChecked();
    await user.click(flip);
    await waitFor(async () => expect((await backend.settingsGet()).flipPause).toBe(true));
    await user.click(show);
    await waitFor(async () => expect((await backend.settingsGet()).showVinylSides).toBe(false));
  });

  it("off, there are no side headings and no side items in the menu", async () => {
    const { vinyl } = await albums();
    await setUp({ showVinylSides: false });
    const user = await openAlbum(vinyl);
    await screen.findAllByRole("row");
    expect(screen.queryByRole("row", { name: /^Side / })).toBeNull();
    await user.click(screen.getByRole("button", { name: `More options for ${vinyl.title}` }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: /^Play side/ })).toBeNull();
    expect(within(menu).queryByRole("menuitem", { name: "Split into sides…" })).toBeNull();
  });
});

// ---------------------------------------------------------------- pause to flip

/** A fake player: the state the core would send, set by hand. */
function playerAt(track: Track, status: PlayerState["status"] = "playing") {
  const state: PlayerState = {
    status,
    track,
    positionMs: 0,
    durationMs: track.durationMs,
    volume: 1,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { userQueue: [], nextUp: [], history: [], context: { kind: "album", id: track.albumId, title: track.albumTitle } },
  };
  act(() => usePlayer.setState({ state }));
}

describe("pause to flip", () => {
  let pause: ReturnType<typeof vi.spyOn>;
  let resume: ReturnType<typeof vi.spyOn>;
  let seek: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    pause = vi.spyOn(backend, "playPause").mockResolvedValue(undefined);
    resume = vi.spyOn(backend, "playResume").mockResolvedValue(undefined);
    seek = vi.spyOn(backend, "playSeek").mockResolvedValue(undefined);
    useFullWindow.setState({ open: false, mode: "artwork", osFullscreen: false, prefs: { ...FULL_WINDOW_PREFS_DEFAULT } });
  });

  /** The record player open on side A's last song, the album's sides loaded. */
  async function onLastOfSideA(patch: Record<string, unknown> = { flipPause: true }) {
    const { vinyl, sides } = await albums();
    await setUp({ recordMotion: "full", ...patch });
    const tracks = tracksOf(vinyl.id);
    const byId = (id: string) => tracks.find((t) => t.id === id)!;
    const a = sides.sides[0].trackIds;
    const b = sides.sides[1].trackIds;
    const load = vi.spyOn(backend, "albumSides");
    playerAt(byId(a[a.length - 1]));
    render(<FullWindow />);
    act(() => fullWindow.open());
    if (patch.flipPause && patch.showVinylSides !== false && patch.recordMotion !== "off") {
      await waitFor(() => expect(load).toHaveBeenCalledWith(vinyl.id));
      await act(async () => {
        await load.mock.results[0].value;
      });
    }
    return { byId, a, b };
  }

  it("the end of side A pauses at side B's start until Flip", async () => {
    const { byId, b } = await onLastOfSideA();
    playerAt(byId(b[0]));
    const flip = await screen.findByRole("button", { name: "Flip to side B" });
    expect(screen.getByRole("status")).toHaveTextContent("End of side A");
    expect(pause).toHaveBeenCalledTimes(1);
    expect(seek).toHaveBeenCalledWith(0);
    // The tonearm lifts.
    expect(document.querySelector(".turntable")).toHaveAttribute("data-state", "paused");
    playerAt(byId(b[0]), "paused");
    await userEvent.setup().click(flip);
    expect(resume).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: /Flip to side/ })).toBeNull();
  });

  it("playing on another way puts Flip away", async () => {
    const { byId, b } = await onLastOfSideA();
    playerAt(byId(b[0]));
    await screen.findByRole("button", { name: "Flip to side B" });
    playerAt(byId(b[0]), "paused");
    playerAt(byId(b[0]), "playing");
    await waitFor(() => expect(screen.queryByRole("button", { name: /Flip to side/ })).toBeNull());
    expect(resume).not.toHaveBeenCalled();
  });

  it("jumping into the middle of side B plays on", async () => {
    const { byId, b } = await onLastOfSideA();
    playerAt(byId(b[1]));
    await act(async () => {});
    expect(pause).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /Flip to side/ })).toBeNull();
  });

  it("off by default, it never pauses", async () => {
    const { byId, b } = await onLastOfSideA({ flipPause: false });
    playerAt(byId(b[0]));
    await act(async () => {});
    expect(pause).not.toHaveBeenCalled();
  });

  it("only with the record player showing", async () => {
    const { byId, b } = await onLastOfSideA({ flipPause: true, recordMotion: "off" });
    playerAt(byId(b[0]));
    await act(async () => {});
    expect(pause).not.toHaveBeenCalled();
  });
});
