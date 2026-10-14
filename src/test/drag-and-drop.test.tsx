/**
 * Drag and drop: what each source carries, which library rows light up or
 * dim, dropping on a playlist (toast, duplicate dialog), on Liked Songs (only
 * unliked songs, dates kept), into the queue panel at a spot and on the queue
 * button, reorder lines and cancel, auto-scroll, and the setting.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Card } from "../components/Card";
import { ContextMenuHost } from "../components/ContextMenu";
import { DragAnnouncer } from "../components/DragAnnouncer";
import { PlaylistDialogs } from "../components/PlaylistDialogs";
import { Toaster } from "../components/Toast";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Playlist, type Track } from "../lib/types";
import { PlayerBar } from "../shell/PlayerBar";
import { QueuePanel } from "../shell/QueuePanel";
import { Sidebar } from "../shell/Sidebar";
import { useApp } from "../stores/app";
import {
  SONGS_TYPE,
  albumDrag,
  autoScrollStep,
  connectDragAutoScroll,
  endSongDrag,
  libraryDropRule,
  playlistDrag,
  tracksDrag,
  useDrag,
  SCROLL_MAX,
} from "../stores/drag";
import { useLibraryView } from "../stores/library";
import { useLikes } from "../stores/likes";
import { usePins } from "../stores/pins";
import { usePlayer } from "../stores/player";
import { usePlaylistUi } from "../stores/playlists";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";
import { body } from "./rows";

const mock = backend as MockBackend;
const SMART = "503";

// jsdom has no DragEvent; without one the pointer position never reaches handlers.
if (!("DragEvent" in window)) (window as unknown as { DragEvent: typeof MouseEvent }).DragEvent = class DragEvent extends MouseEvent {};

function dt() {
  const data = new Map<string, string>();
  return {
    setData: (k: string, v: string) => data.set(k, v),
    getData: (k: string) => data.get(k) ?? "",
    get types() {
      return [...data.keys()];
    },
    setDragImage: vi.fn(),
    effectAllowed: "",
    dropEffect: "",
  };
}

const toastText = () => useToasts.getState().toasts.map((t) => t.message);
const songsOf = async (id: string) => (await backend.playlistPage(id)).tracks.map((t) => t.id);

async function makePlaylist(title: string, ids: string[]): Promise<Playlist> {
  const p = await backend.playlistCreate(title, ids);
  const lists = await backend.playlists();
  act(() => useApp.getState().setPlaylists(lists));
  return p;
}

/** A track list of fixture tracks. */
function List({ tracks }: { tracks: Track[] }) {
  return (
    <TrackList label="Songs">
      {tracks.map((t, i) => (
        <TrackRow key={t.id} track={t} index={i + 1} onPlay={() => {}} />
      ))}
    </TrackList>
  );
}

async function renderSidebar() {
  act(() => useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } }));
  await act(() => useLibraryView.getState().loadItems());
  return render(
    <>
      <Sidebar collapsed={false} />
      <ContextMenuHost />
      <PlaylistDialogs />
      <Toaster />
      <DragAnnouncer />
    </>,
  );
}

/** The sidebar row for a title. */
const libRow = (title: string) => within(screen.getByRole("list", { name: "Your Library items" })).getByText(title).closest<HTMLElement>(".lib-row")!;

beforeEach(async () => {
  useRouter.getState().reset();
  useToasts.setState({ toasts: [] });
  usePlaylistUi.setState({ renameId: null, editing: null, deleting: null, duplicate: null, removing: null });
  useUi.setState({ menu: null, sidebarCollapsed: false, sidebarArt: true });
  usePins.setState({ pins: [] });
  useLibraryView.setState({ filter: "playlist", sub: null, sort: "alpha", view: "list", customOrder: [], recents: {} });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  await mock.settingsSet({ playlistAskDuplicates: true });
  const lists = await backend.playlists();
  act(() => useApp.getState().setPlaylists(lists));
  const liked = await backend.likedIds();
  act(() => useLikes.getState().setIds(liked));
  endSongDrag();
});

afterEach(() => {
  vi.restoreAllMocks();
  endSongDrag();
});

// ---------------------------------------------------------------- sources

describe("drag sources", () => {
  it("a row carries its song, and gets the selected highlight", () => {
    const tracks = mock.fx.tracks.slice(0, 3);
    render(<List tracks={tracks} />);
    const rows = body(screen.getAllByRole("row"));
    expect(rows[1]).toHaveAttribute("draggable", "true");
    const t = dt();
    fireEvent.dragStart(rows[1], { dataTransfer: t });
    expect(t.types).toContain(SONGS_TYPE);
    expect(useDrag.getState().songs!.tracks!.map((x) => x.id)).toEqual([tracks[1].id]);
    expect(useDrag.getState().message).toBe(`Dragging ${tracks[1].title}`);
    expect(t.setDragImage).toHaveBeenCalled();
    expect(rows[1]).toHaveClass("is-picked");
    fireEvent.dragEnd(rows[1], { dataTransfer: t });
    expect(useDrag.getState().songs).toBeNull();
  });

  it("a row in a selection carries the whole selection in list order", () => {
    const tracks = mock.fx.tracks.slice(0, 4);
    render(<List tracks={tracks} />);
    const rows = body(screen.getAllByRole("row"));
    fireEvent.click(rows[3]);
    fireEvent.click(rows[0], { ctrlKey: true, metaKey: true });
    fireEvent.click(rows[2], { ctrlKey: true, metaKey: true });
    const t = dt();
    fireEvent.dragStart(rows[2], { dataTransfer: t });
    const songs = useDrag.getState().songs!;
    expect(songs.tracks!.map((x) => x.id)).toEqual([tracks[0].id, tracks[2].id, tracks[3].id]);
    // The drag picture: a count badge and the first title.
    const ghost = t.setDragImage.mock.calls[0][0] as HTMLElement;
    expect(ghost.querySelector(".drag-ghost__count")).toHaveTextContent("3");
    expect(ghost.querySelector(".drag-ghost__title")).toHaveTextContent(tracks[0].title);
    expect(songs.label).toBe("3 songs");
    expect(useDrag.getState().message).toBe("Dragging 3 songs");
  });

  it("a row outside the selection carries only itself", () => {
    const tracks = mock.fx.tracks.slice(0, 3);
    render(<List tracks={tracks} />);
    const rows = body(screen.getAllByRole("row"));
    fireEvent.click(rows[0]);
    fireEvent.click(rows[1], { ctrlKey: true, metaKey: true });
    fireEvent.dragStart(rows[2], { dataTransfer: dt() });
    expect(useDrag.getState().songs!.tracks!.map((x) => x.id)).toEqual([tracks[2].id]);
  });

  it("album and playlist cards carry all their songs, loaded on drop", async () => {
    const album = mock.fx.albums.find((a) => a.trackCount >= 3)!;
    render(<Card kind="album" title={album.title} thumb={null} onOpen={() => {}} drag={() => albumDrag(album.id, album.title)} />);
    const card = document.querySelector(".card")!;
    expect(card).toHaveAttribute("draggable", "true");
    fireEvent.dragStart(card, { dataTransfer: dt() });
    const songs = useDrag.getState().songs!;
    expect(songs.label).toBe(album.title);
    expect((await songs.load()).map((t) => t.id)).toEqual(mock.fx.tracksByAlbum.get(album.id)!.map((t) => t.id));

    const p = await makePlaylist("Card source", mock.fx.tracks.slice(0, 2).map((t) => t.id));
    const pd = playlistDrag(p.id, p.title);
    expect(pd.playlistId).toBe(p.id);
    expect((await pd.load()).map((t) => t.id)).toEqual(mock.fx.tracks.slice(0, 2).map((t) => t.id));
  });

  it("sidebar playlists drag; Liked Songs does not", async () => {
    const p = await makePlaylist("Sidebar source", [mock.fx.tracks[0].id]);
    await renderSidebar();
    expect(libRow(p.title)).toHaveAttribute("draggable", "true");
    expect(libRow("Liked Songs")).not.toHaveAttribute("draggable");
    fireEvent.dragStart(libRow(p.title), { dataTransfer: dt() });
    expect(useDrag.getState().songs!.playlistId).toBe(p.id);
  });

  it("the player bar's song drags", async () => {
    const album = mock.fx.albums.find((a) => a.trackCount >= 3)!;
    await act(() => mock.playContext({ context: { kind: "album", id: album.id } }));
    await act(() => mock.playPause());
    await act(async () => usePlayer.getState().set(await mock.playerState()));
    const track = usePlayer.getState().state!.track!;
    render(<PlayerBar />);
    const now = document.querySelector(".player-bar__now")!;
    expect(now).toHaveAttribute("draggable", "true");
    fireEvent.dragStart(now, { dataTransfer: dt() });
    expect(useDrag.getState().songs!.tracks!.map((t) => t.id)).toEqual([track.id]);
  });
});

// ---------------------------------------------------------------- targets

describe("drop targets in the library", () => {
  it("rules: your playlists take songs; smart playlists, albums, the dragged playlist itself, and Liked with all-liked songs dim", () => {
    const liked = mock.fx.tracks.find((t) => useLikes.getState().ids.has(t.id))!;
    const unliked = mock.fx.tracks.find((t) => !useLikes.getState().ids.has(t.id))!;
    const row = { kind: "playlist", id: "1", smart: false };
    expect(libraryDropRule(row, tracksDrag([unliked]))).toBe("ok");
    expect(libraryDropRule({ ...row, smart: true }, tracksDrag([unliked]))).toBe("dim");
    expect(libraryDropRule({ kind: "album", id: "9", smart: false }, tracksDrag([unliked]))).toBe("dim");
    expect(libraryDropRule(row, playlistDrag("1", "Self"))).toBe("dim");
    const likedRow = { kind: "playlist", id: "liked", smart: false, liked: true };
    expect(libraryDropRule(likedRow, tracksDrag([liked]))).toBe("dim");
    expect(libraryDropRule(likedRow, tracksDrag([liked, unliked]))).toBe("ok");
    // Unknown songs (an album) might not all be liked yet.
    expect(libraryDropRule(likedRow, albumDrag("1", "A"))).toBe("ok");
  });

  it("the playlist under the pointer gets the outline, refused rows dim for the whole drag", async () => {
    const mine = await makePlaylist("Target", [mock.fx.tracks[0].id]);
    const liked = mock.fx.tracks.find((t) => useLikes.getState().ids.has(t.id))!;
    await renderSidebar();
    const smart = (await backend.playlists()).find((p) => p.id === SMART)!;
    const t = dt();
    render(<List tracks={[liked]} />);
    fireEvent.dragStart(body(screen.getAllByRole("row")).find((r) => r.classList.contains("track-row"))!, { dataTransfer: t });
    expect(libRow(mine.title)).not.toHaveClass("is-drop-dim");
    expect(libRow(smart.title)).toHaveClass("is-drop-dim");
    expect(libRow("Liked Songs")).toHaveClass("is-drop-dim");
    fireEvent.dragOver(libRow(mine.title), { dataTransfer: t });
    expect(libRow(mine.title)).toHaveClass("is-drop-target");
    expect(screen.getByTestId("drag-status")).toHaveTextContent(`Drop on ${mine.title}`);
    // A refused row never takes the drop.
    fireEvent.dragOver(libRow(smart.title), { dataTransfer: t });
    expect(libRow(smart.title)).not.toHaveClass("is-drop-target");
    fireEvent.dragEnd(document.querySelector(".track-row")!, { dataTransfer: t });
    expect(libRow(smart.title)).not.toHaveClass("is-drop-dim");
    expect(libRow(mine.title)).not.toHaveClass("is-drop-target");
  });

  it("a drop appends to the playlist with the toast", async () => {
    const p = await makePlaylist("Drop here", [mock.fx.tracks[0].id]);
    await renderSidebar();
    const songs = [mock.fx.tracks[4], mock.fx.tracks[5]];
    render(<List tracks={songs} />);
    const rows = Array.from(document.querySelectorAll<HTMLElement>(".track-row"));
    fireEvent.click(rows[0]);
    fireEvent.click(rows[1], { ctrlKey: true, metaKey: true });
    const t = dt();
    fireEvent.dragStart(rows[0], { dataTransfer: t });
    fireEvent.dragOver(libRow(p.title), { dataTransfer: t });
    fireEvent.drop(libRow(p.title), { dataTransfer: t });
    await waitFor(async () => expect(await songsOf(p.id)).toEqual([mock.fx.tracks[0].id, songs[0].id, songs[1].id]));
    await waitFor(() => expect(toastText()).toContain(`Added to ${p.title}`));
    expect(screen.getByTestId("drag-status")).toHaveTextContent(`Added 2 songs to ${p.title}`);
  });

  it("a song already there asks with the duplicate dialog; Don't add keeps the playlist as it was", async () => {
    const p = await makePlaylist("Has it", [mock.fx.tracks[1].id]);
    await renderSidebar();
    render(<List tracks={[mock.fx.tracks[1]]} />);
    const t = dt();
    fireEvent.dragStart(document.querySelector(".track-row")!, { dataTransfer: t });
    fireEvent.drop(libRow(p.title), { dataTransfer: t });
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent(p.title);
    fireEvent.click(within(dialog).getByRole("button", { name: /don't add/i }));
    expect(await songsOf(p.id)).toEqual([mock.fx.tracks[1].id]);
  });

  it("dropping a playlist on another adds its songs", async () => {
    const from = await makePlaylist("From", [mock.fx.tracks[7].id, mock.fx.tracks[8].id]);
    const to = await makePlaylist("To", [mock.fx.tracks[0].id]);
    await renderSidebar();
    const t = dt();
    fireEvent.dragStart(libRow(from.title), { dataTransfer: t });
    // A playlist can't take its own songs.
    expect(libRow(from.title)).toHaveClass("is-drop-dim");
    fireEvent.drop(libRow(to.title), { dataTransfer: t });
    await waitFor(async () => expect(await songsOf(to.id)).toEqual([mock.fx.tracks[0].id, mock.fx.tracks[7].id, mock.fx.tracks[8].id]));
  });

  it("Liked Songs likes only the songs that aren't liked, so liked ones keep their date", async () => {
    const liked = mock.fx.tracks.find((t) => useLikes.getState().ids.has(t.id))!;
    const fresh = mock.fx.tracks.find((t) => !useLikes.getState().ids.has(t.id))!;
    const dateBefore = (await backend.likedPage()).tracks.find((t) => t.id === liked.id)?.ratedAt;
    const spy = vi.spyOn(backend, "likesSet");
    await renderSidebar();
    render(<List tracks={[liked, fresh]} />);
    const rows = Array.from(document.querySelectorAll<HTMLElement>(".track-row"));
    fireEvent.click(rows[0]);
    fireEvent.click(rows[1], { ctrlKey: true, metaKey: true });
    const t = dt();
    fireEvent.dragStart(rows[0], { dataTransfer: t });
    expect(libRow("Liked Songs")).not.toHaveClass("is-drop-dim");
    fireEvent.drop(libRow("Liked Songs"), { dataTransfer: t });
    await waitFor(() => expect(spy).toHaveBeenCalledWith([fresh.id], true));
    expect(spy).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(toastText()).toContain("Added to Liked Songs"));
    expect((await backend.likedPage()).tracks.find((t) => t.id === liked.id)?.ratedAt).toBe(dateBefore);
    await backend.likesSet([fresh.id], false);
  });
});

// ---------------------------------------------------------------- queue

describe("the queue", () => {
  async function setUpQueue(n: number) {
    const album = mock.fx.albums.find((a) => a.trackCount >= 3)!;
    await act(() => mock.queueClear());
    await act(() => mock.playContext({ context: { kind: "album", id: album.id } }));
    const others = mock.fx.tracks.filter((t) => t.albumId !== album.id).slice(0, n);
    if (n) await act(() => mock.queueAdd(others.map((t) => t.id), "end"));
    await act(async () => usePlayer.getState().set(await mock.playerState()));
    return others;
  }
  const userRows = () => body(within(screen.getByRole("region", { name: "Next in queue" })).getAllByRole("row"));

  afterEach(async () => {
    await act(() => mock.queueClear());
    await act(() => mock.playPause());
  });

  it("songs dropped between rows go into Next in queue at that spot, with a drop line", async () => {
    const others = await setUpQueue(2);
    const spy = vi.spyOn(backend, "queueAdd");
    render(
      <>
        <QueuePanel />
        <List tracks={[mock.fx.tracks[9]]} />
      </>,
    );
    const source = screen.getByRole("grid", { name: "Songs" }).querySelector(".track-row")!;
    const t = dt();
    fireEvent.dragStart(source, { dataTransfer: t });
    // Over the bottom half of the first row: after it.
    vi.spyOn(userRows()[0], "getBoundingClientRect").mockReturnValue({ top: 0, height: 56, bottom: 56, left: 0, right: 300, width: 300, x: 0, y: 0, toJSON: () => ({}) });
    fireEvent.dragOver(userRows()[0], { dataTransfer: t, clientY: 40 });
    expect(userRows()[0]).toHaveClass("is-drop-after");
    fireEvent.drop(userRows()[0], { dataTransfer: t, clientY: 40 });
    await waitFor(() => expect(spy).toHaveBeenCalledWith([mock.fx.tracks[9].id], { index: 1 }));
    const state = await mock.playerState();
    expect(state.queue.userQueue.map((i) => i.track.id)).toEqual([others[0].id, mock.fx.tracks[9].id, others[1].id]);
    expect(document.querySelector(".is-drop-after, .is-drop-before")).toBeNull();
  });

  it("an empty queue takes a drop anywhere in the panel, with a line", async () => {
    await setUpQueue(0);
    const spy = vi.spyOn(backend, "queueAdd");
    render(
      <>
        <QueuePanel />
        <List tracks={[mock.fx.tracks[9]]} />
      </>,
    );
    const t = dt();
    fireEvent.dragStart(screen.getByRole("grid", { name: "Songs" }).querySelector(".track-row")!, { dataTransfer: t });
    const body = document.querySelector(".queue-panel__body")!;
    fireEvent.dragOver(body, { dataTransfer: t });
    expect(document.querySelector(".queue-drop-line")).not.toBeNull();
    fireEvent.drop(body, { dataTransfer: t });
    await waitFor(() => expect(spy).toHaveBeenCalledWith([mock.fx.tracks[9].id], { index: 0 }));
  });

  it("the queue button adds to the end of the queue with a toast", async () => {
    await setUpQueue(1);
    const spy = vi.spyOn(backend, "queueAdd");
    render(
      <>
        <PlayerBar />
        <List tracks={[mock.fx.tracks[9], mock.fx.tracks[10]]} />
        <Toaster />
      </>,
    );
    const rows = Array.from(screen.getByRole("grid", { name: "Songs" }).querySelectorAll<HTMLElement>(".track-row"));
    fireEvent.click(rows[0]);
    fireEvent.click(rows[1], { ctrlKey: true, metaKey: true });
    const t = dt();
    fireEvent.dragStart(rows[0], { dataTransfer: t });
    const button = screen.getByRole("button", { name: "Queue" }).closest(".drop-button")!;
    fireEvent.dragOver(button, { dataTransfer: t });
    expect(button).toHaveClass("is-drop-target");
    fireEvent.drop(button, { dataTransfer: t });
    await waitFor(() => expect(spy).toHaveBeenCalledWith([mock.fx.tracks[9].id, mock.fx.tracks[10].id], "end"));
    await waitFor(() => expect(toastText()).toContain("Added 2 songs to queue"));
  });

  it("reorder shows a drop line; a drag that ends without a drop (Escape) changes nothing", async () => {
    const others = await setUpQueue(3);
    const spy = vi.spyOn(backend, "queueMove");
    render(<QueuePanel />);
    const t = dt();
    fireEvent.dragStart(userRows()[2], { dataTransfer: t });
    fireEvent.dragOver(userRows()[0], { dataTransfer: t, clientY: 0 });
    expect(userRows()[0]).toHaveClass("is-drop-before");
    fireEvent.dragEnd(userRows()[2], { dataTransfer: t });
    expect(document.querySelector(".is-drop-before, .is-drop-after")).toBeNull();
    expect(spy).not.toHaveBeenCalled();
    expect((await mock.playerState()).queue.userQueue.map((i) => i.track.id)).toEqual(others.map((o) => o.id));
    // And a real drop moves it.
    const t2 = dt();
    fireEvent.dragStart(userRows()[2], { dataTransfer: t2 });
    fireEvent.dragOver(userRows()[0], { dataTransfer: t2, clientY: 0 });
    fireEvent.drop(userRows()[0], { dataTransfer: t2, clientY: 0 });
    await waitFor(() => expect(spy).toHaveBeenCalledWith(usePlayer.getState().state!.queue.userQueue[2].itemId, 0));
  });
});

// ---------------------------------------------------------------- the rest

describe("auto-scroll", () => {
  it("steps up near the top edge, down near the bottom, not in the middle", () => {
    expect(autoScrollStep(0, 400, 200)).toBe(0);
    expect(autoScrollStep(0, 400, 2)).toBeLessThan(0);
    expect(autoScrollStep(0, 400, 398)).toBeGreaterThan(0);
    expect(autoScrollStep(0, 400, 0)).toBe(-SCROLL_MAX);
    expect(autoScrollStep(0, 400, 500)).toBe(0);
  });

  it("a drag over a list near its bottom edge scrolls it", () => {
    const stop = connectDragAutoScroll();
    const scroller = document.createElement("div");
    scroller.className = "main__scroll";
    const inner = document.createElement("div");
    scroller.appendChild(inner);
    document.body.appendChild(scroller);
    vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue({ top: 100, bottom: 500, height: 400, left: 0, right: 300, width: 300, x: 0, y: 100, toJSON: () => ({}) });
    fireEvent.dragOver(inner, { clientY: 495 });
    expect(scroller.scrollTop).toBeGreaterThan(0);
    stop();
    scroller.remove();
  });
});

describe("the setting", () => {
  it("off: rows don't start drags, and cards and library rows aren't draggable", async () => {
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, dragAndDrop: false } }));
    render(<List tracks={mock.fx.tracks.slice(0, 2)} />);
    for (const r of document.querySelectorAll(".track-row")) expect(r).not.toHaveAttribute("draggable");
    const album = mock.fx.albums[0];
    render(<Card kind="album" title={album.title} thumb={null} onOpen={() => {}} drag={() => albumDrag(album.id, album.title)} />);
    expect(document.querySelector(".card")).not.toHaveAttribute("draggable");
    const p = await makePlaylist("No drag", [mock.fx.tracks[0].id]);
    await renderSidebar();
    expect(libRow(p.title)).not.toHaveAttribute("draggable");
  });

  it("off: a reorderable row still reorders but carries no songs", () => {
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, dragAndDrop: false } }));
    const track = mock.fx.tracks[0];
    const onDragStart = vi.fn();
    render(
      <TrackList label="Songs">
        <TrackRow track={track} index={1} onPlay={() => {}} draggable onDragStart={onDragStart} />
      </TrackList>,
    );
    const row = document.querySelector(".track-row")!;
    expect(row).toHaveAttribute("draggable", "true");
    const t = dt();
    fireEvent.dragStart(row, { dataTransfer: t });
    expect(onDragStart).toHaveBeenCalled();
    expect(t.types).not.toContain(SONGS_TYPE);
    expect(useDrag.getState().songs).toBeNull();
  });
});
