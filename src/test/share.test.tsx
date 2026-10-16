import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost, openContextMenu } from "../components/ContextMenu";
import { shareItem } from "../components/ShareMenu";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockMatched, mockUnreleased } from "../lib/mock/fixtures";
import { mockHasNoMatch } from "../lib/mock/share";
import { DEFAULT_SETTINGS, type PlayerState, type ShareKind, type Track } from "../lib/types";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { connectShare, copyWhenReady, openDeepLink, pendingStartFor } from "../stores/share";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const fx = mock.fx;
const demoAlbum = fx.albums.find((a) => a.subformats.includes("demo"))!;
const releasedAlbums = fx.albums.filter((a) => !mockUnreleased(a, true));
const buildAlbum = fx.albums.find((a) => !mockMatched(a))!;
const buildTrack = fx.tracks.find((t) => t.albumId === buildAlbum.id)!;
const released = fx.tracks.find((t) => releasedAlbums.some((a) => a.id === t.albumId) && !mockHasNoMatch(t.id))!;
const noMatch = fx.tracks.find((t) => releasedAlbums.some((a) => a.id === t.albumId) && mockHasNoMatch(t.id))!;
const demoTrack = fx.tracks.find((t) => t.albumId === demoAlbum.id)!;
const playlist = fx.playlists[0];

let clip: string[] = [];

/** A clipboard that records writes (user-event puts in its own at setup). */
function fakeClipboard() {
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText: vi.fn(async (t: string) => void clip.push(t)) },
    configurable: true,
  });
}

beforeEach(async () => {
  clip = [];
  fakeClipboard();
  useToasts.setState({ toasts: [] });
  await backend.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS });
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => useUi.setState({ menu: null }));
  useRouter.getState().reset();
  usePlayer.setState({ state: null, local: null, positionMs: 0 });
});

const toasts = () => useToasts.getState().toasts;

/** Open a menu with just Share in it, then the Share submenu. */
async function openShare(kind: ShareKind, id: string) {
  const user = userEvent.setup();
  fakeClipboard();
  render(<ContextMenuHost />);
  act(() => openContextMenu("Options", [shareItem(kind, id)], { clientX: 10, clientY: 10 }));
  await user.click(await screen.findByRole("menuitem", { name: "Share" }));
  const sub = await screen.findByRole("menu", { name: "Share" });
  return { user, sub };
}

const items = (sub: HTMLElement) => within(sub).getAllByRole("menuitem").map((i) => i.textContent);

describe("Share submenu", () => {
  it("is in a song's menu", async () => {
    render(
      <>
        <TrackList label="Tracks">
          <TrackRow track={released} index={1} onPlay={vi.fn()} />
        </TrackList>
        <ContextMenuHost />
      </>,
    );
    fireEvent.contextMenu(screen.getByRole("row", { name: new RegExp(released.title) }), { clientX: 5, clientY: 5 });
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "Share" })).toHaveAttribute("aria-haspopup", "menu");
  });

  it("copies the public link for a released song", async () => {
    const { user, sub } = await openShare("track", released.id);
    await waitFor(() => expect(items(sub)).toEqual(["Copy song link", "Copy Platter link", "Share card…"]));
    await user.click(within(sub).getByRole("menuitem", { name: "Copy song link" }));
    await waitFor(() => expect(clip).toHaveLength(1));
    expect(clip[0]).toMatch(/^https:\/\/song\.link\/i\/\d+$/);
    expect(toasts().map((t) => t.message)).toEqual(["Link copied"]);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("copies an album's public link from album.link", async () => {
    const album = releasedAlbums.find((a) => !mockHasNoMatch(a.id))!;
    const { user, sub } = await openShare("album", album.id);
    await user.click(await within(sub).findByRole("menuitem", { name: "Copy album link" }));
    await waitFor(() => expect(clip[0]).toMatch(/^https:\/\/album\.link\/i\/\d+$/));
  });

  it("greys the public link for unreleased music and never looks it up", async () => {
    const spy = vi.spyOn(backend, "sharePublicLink");
    const { user, sub } = await openShare("track", demoTrack.id);
    const item = await within(sub).findByRole("menuitem", { name: /Copy song link/ });
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveTextContent("Not released");
    await user.click(item);
    expect(spy).not.toHaveBeenCalled();
    expect(clip).toEqual([]);
  });

  it("albums Plex couldn't match count as unreleased unless the setting is off", async () => {
    const { sub } = await openShare("track", buildTrack.id);
    const item = await within(sub).findByRole("menuitem", { name: /Copy song link/ });
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveTextContent("Not released");
    act(() => useUi.setState({ menu: null }));
    cleanup();
    await backend.settingsSet({ countUnmatchedUnreleased: false });
    const again = await openShare("track", buildTrack.id);
    const on = await within(again.sub).findByRole("menuitem", { name: /Copy song link/ });
    expect(on).not.toHaveAttribute("aria-disabled", "true");
    expect(on).not.toHaveTextContent("Not released");
  });

  it("copies the Platter link, and playlists and artists only get that", async () => {
    const { user, sub } = await openShare("playlist", playlist.id);
    await waitFor(() => expect(items(sub)).toEqual(["Copy Platter link", "Share card…"]));
    await user.click(within(sub).getByRole("menuitem", { name: "Copy Platter link" }));
    await waitFor(() => expect(clip).toEqual([expect.stringMatching(new RegExp(`^platter://playlist/${playlist.id}\\?s=[0-9a-f]{8}$`))]));
    expect(toasts()[0].message).toBe("Link copied");
  });

  it("leaves the public link out when it's turned off in Settings", async () => {
    await backend.settingsSet({ sharePublicLinks: false });
    const { sub } = await openShare("track", released.id);
    await waitFor(() => expect(items(sub)).toEqual(["Copy Platter link", "Share card…"]));
  });

  it("offers the Platter link when no sure match is found", async () => {
    const { user, sub } = await openShare("track", noMatch.id);
    await user.click(await within(sub).findByRole("menuitem", { name: "Copy song link" }));
    await waitFor(() => expect(toasts()).toHaveLength(1));
    const t = toasts()[0];
    expect(t.message).toBe("Couldn't find this song online");
    expect(t.action?.label).toBe("Copy Platter link");
    expect(clip).toEqual([]);
    t.action!.run();
    await waitFor(() => expect(clip).toEqual([expect.stringMatching(new RegExp(`^platter://song/${noMatch.id}`))]));
  });

  it("says so when the link search can't be reached (and still offers the Platter link)", async () => {
    mock.share.failNext = true;
    const { user, sub } = await openShare("track", released.id);
    await user.click(await within(sub).findByRole("menuitem", { name: "Copy song link" }));
    await waitFor(() => expect(toasts()).toHaveLength(1));
    expect(toasts()[0]).toMatchObject({ kind: "error", message: expect.stringMatching(/Couldn't reach/) });
    expect(toasts()[0].action?.label).toBe("Copy Platter link");
  });

  it("the song playing now gets links at the current time", async () => {
    usePlayer.setState({ state: { status: "playing", track: released } as PlayerState, positionMs: 83_400 });
    const { user, sub } = await openShare("track", released.id);
    await waitFor(() => expect(items(sub)).toEqual(["Copy song link", "Copy Platter link", "Copy link at 1:23", "Share card…"]));
    await user.click(within(sub).getByRole("menuitem", { name: "Copy link at 1:23" }));
    await waitFor(() => expect(clip).toHaveLength(1));
    const [pub, app] = clip[0].split("\n");
    expect(pub).toMatch(/^https:\/\/song\.link\/i\/\d+ \(from 1:23\)$/);
    expect(app).toMatch(new RegExp(`^platter://song/${released.id}\\?t=83&s=`));
    expect(toasts()[0].message).toBe("Links copied");
  });

  it("a song that isn't playing has no timed link", async () => {
    usePlayer.setState({ state: { status: "playing", track: noMatch } as PlayerState, positionMs: 5000 });
    const { sub } = await openShare("track", released.id);
    await waitFor(() => expect(items(sub)).toEqual(["Copy song link", "Copy Platter link", "Share card…"]));
  });
});

describe("clipboard", () => {
  it("starts the write at once and fills it in when the link arrives", async () => {
    const write = vi.fn(async () => {});
    class FakeItem {
      constructor(public data: Record<string, Promise<Blob>>) {}
    }
    vi.stubGlobal("ClipboardItem", FakeItem);
    Object.defineProperty(navigator, "clipboard", { value: { write, writeText: vi.fn() }, configurable: true });
    let resolve!: (s: string) => void;
    const done = copyWhenReady(new Promise((r) => (resolve = r)));
    expect(write).toHaveBeenCalledTimes(1);
    resolve("https://song.link/i/1");
    await done;
    const item = (write.mock.calls[0] as unknown as [FakeItem[]])[0][0];
    expect(await (await item.data["text/plain"]).text()).toBe("https://song.link/i/1");
    vi.unstubAllGlobals();
  });
});

describe("opening platter:// links", () => {
  const album = releasedAlbums[0];
  const route = () => useRouter.getState().entries[useRouter.getState().index].route;

  it("an album link opens the album", async () => {
    const stop = connectShare(backend);
    act(() => mock.share.openLink(`platter://album/${album.id}`));
    await waitFor(() => expect(route()).toEqual({ name: "album", id: album.id }));
    stop();
  });

  it("artist and playlist links open their pages", async () => {
    await openDeepLink({ status: "open", kind: "artist", id: fx.artists[0].id, albumId: null, atSeconds: null });
    expect(route()).toEqual({ name: "artist", id: fx.artists[0].id });
    await openDeepLink({ status: "open", kind: "playlist", id: playlist.id, albumId: null, atSeconds: null });
    expect(route()).toEqual({ name: "playlist", id: playlist.id });
  });

  it("links from another server or for missing items say they aren't in the library", async () => {
    const stop = connectShare(backend);
    act(() => mock.share.openLink(`platter://album/${album.id}?s=00000000`));
    await waitFor(() => expect(toasts().map((t) => t.message)).toEqual(["This isn't in your library"]));
    act(() => useToasts.setState({ toasts: [] }));
    act(() => mock.share.openLink("platter://song/does-not-exist"));
    await waitFor(() => expect(toasts().map((t) => t.message)).toEqual(["This isn't in your library"]));
    expect(route()).toEqual({ name: "home" });
    stop();
  });

  it("a malformed link says it can't be opened", async () => {
    await openDeepLink(mock.share.resolve("platter://video/1"));
    expect(toasts()[0]).toMatchObject({ kind: "error", message: "Platter couldn't open that link" });
  });

  it("a link that started the app opens once the UI is up", async () => {
    mock.share.openLink(`platter://album/${album.id}`);
    const stop = connectShare(backend);
    await waitFor(() => expect(route()).toEqual({ name: "album", id: album.id }));
    stop();
  });

  it("a song link at a time opens its album and starts there when the song plays", async () => {
    const stop = connectShare(backend);
    const seek = vi.spyOn(backend, "playSeek");
    const play = vi.spyOn(backend, "playContext");
    act(() => mock.share.openLink(`platter://song/${released.id}?t=83`));
    await waitFor(() => expect(route()).toEqual({ name: "album", id: released.albumId }));
    await waitFor(() => expect(toasts()).toHaveLength(1));
    const t = toasts()[0];
    expect(t.message).toBe(`Play "${released.title}" to start at 1:23`);
    expect(pendingStartFor()).toMatchObject({ trackId: released.id, ms: 83_000 });
    t.action!.run();
    expect(play).toHaveBeenCalledWith(expect.objectContaining({ startTrackId: released.id }));
    // Loading doesn't seek; playing does, once.
    act(() => usePlayer.setState({ state: { status: "loading", track: released } as PlayerState }));
    expect(seek).not.toHaveBeenCalled();
    act(() => usePlayer.setState({ state: { status: "playing", track: released } as PlayerState }));
    expect(seek).toHaveBeenCalledWith(83_000);
    act(() => usePlayer.setState({ state: { status: "paused", track: released } as PlayerState }));
    act(() => usePlayer.setState({ state: { status: "playing", track: released } as PlayerState }));
    expect(seek).toHaveBeenCalledTimes(1);
    stop();
  });

  it("a link to the song already playing waits for play instead of jumping", async () => {
    const stop = connectShare(backend);
    const seek = vi.spyOn(backend, "playSeek");
    act(() => usePlayer.setState({ state: { status: "playing", track: released } as PlayerState }));
    await openDeepLink({ status: "open", kind: "track", id: released.id, albumId: released.albumId, atSeconds: 45 });
    expect(seek).not.toHaveBeenCalled();
    act(() => usePlayer.setState({ state: { status: "loading", track: released } as PlayerState }));
    act(() => usePlayer.setState({ state: { status: "playing", track: released } as PlayerState }));
    expect(seek).toHaveBeenCalledWith(45_000);
    stop();
  });

  it("the start time is dropped when another song plays first", async () => {
    const stop = connectShare(backend);
    const seek = vi.spyOn(backend, "playSeek");
    const other: Track = fx.tracks.find((x) => x.id !== released.id)!;
    await openDeepLink({ status: "open", kind: "track", id: released.id, albumId: released.albumId, atSeconds: 30 });
    act(() => usePlayer.setState({ state: { status: "playing", track: other } as PlayerState }));
    expect(pendingStartFor()).toBeNull();
    act(() => usePlayer.setState({ state: { status: "playing", track: released } as PlayerState }));
    expect(seek).not.toHaveBeenCalled();
    stop();
  });
});
