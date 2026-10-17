import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { Track } from "../lib/types";
import { discGroups, formatReleaseDate, labelLine, loadAlbumView } from "../pages/album";
import { AlbumPage } from "../pages/AlbumPage";
import { QueuePanel } from "../shell/QueuePanel";
import { connectPlayer, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";
import { body } from "./rows";

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts;
let disconnect: () => void;

beforeAll(async () => {
  disconnect = connectPlayer(mock);
  await waitFor(() => expect(usePlayer.getState().state).not.toBeNull());
});
afterAll(() => disconnect());
afterEach(() => {
  vi.restoreAllMocks();
  act(() => useUi.setState({ menu: null }));
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  localStorage.clear();
});

const t = (id: string, disc: number, n: number) => ({ id, discNumber: disc, trackNumber: n }) as Track;

describe("album page helpers", () => {
  it("groups tracks by disc; one disc has no separator", () => {
    expect(discGroups([t("a", 1, 1), t("b", 1, 2)])).toEqual([{ disc: null, tracks: [t("a", 1, 1), t("b", 1, 2)] }]);
    const g = discGroups([t("a", 1, 1), t("b", 1, 2), t("c", 2, 1)]);
    expect(g.map((x) => [x.disc, x.tracks.map((y) => y.id)])).toEqual([
      [1, ["a", "b"]],
      [2, ["c"]],
    ]);
    expect(discGroups([])).toEqual([{ disc: null, tracks: [] }]);
  });

  it("formats the release date in full and the label line", () => {
    expect(formatReleaseDate("2013-06-18", "en-US")).toBe("June 18, 2013");
    expect(formatReleaseDate("2013-06-18T00:00:00Z", "en-US")).toBe("June 18, 2013");
    expect(formatReleaseDate(null)).toBeNull();
    expect(formatReleaseDate("June")).toBeNull();
    expect(formatReleaseDate("2013-13-01")).toBeNull();
    expect(labelLine("Night Records", 2013)).toBe("© 2013 Night Records");
    expect(labelLine("Night Records", null)).toBe("© Night Records");
    expect(labelLine("  ", 2013)).toBeNull();
    expect(labelLine(null, 2013)).toBeNull();
  });
});

describe("album page", () => {
  const live = () => mock.fx.albums.find((a) => a.section === "live")!;
  const single = () => mock.fx.albums.find((a) => a.format === "album" && a.section === "albums")!;

  it("starts each disc with a Disc N row and numbers tracks from 1 on each disc", async () => {
    const album = live();
    const tracks = mock.fx.tracksByAlbum.get(album.id)!;
    render(<AlbumPage id={album.id} />);
    const list = await screen.findByRole("grid", { name: `${album.title} songs` });
    await waitFor(() => expect(within(list).getByRole("row", { name: "Disc 1" })).toBeInTheDocument());
    const items = body(within(list).getAllByRole("row"));
    const disc2 = items.findIndex((i) => i.getAttribute("aria-label") === "Disc 2");
    expect(items[0]).toHaveAccessibleName("Disc 1");
    expect(disc2).toBe(tracks.filter((x) => x.discNumber === 1).length + 1);
    // The first song after each separator is number 1.
    expect(items[1].querySelector(".track-row__index")).toHaveTextContent("1");
    expect(items[disc2 + 1].querySelector(".track-row__index")).toHaveTextContent("1");
  });

  it("has no disc rows on a one-disc album", async () => {
    const album = single();
    render(<AlbumPage id={album.id} />);
    const list = await screen.findByRole("grid", { name: `${album.title} songs` });
    await waitFor(() => expect(body(within(list).getAllByRole("row")).length).toBe(mock.fx.tracksByAlbum.get(album.id)!.length));
    expect(within(list).queryByRole("row", { name: /^Disc / })).toBeNull();
  });

  it("shows the release date and label under the tracklist, and hides them when Plex has neither", async () => {
    const album = single();
    const page = await mock.albumPage(album.id);
    vi.spyOn(backend, "albumPage").mockResolvedValueOnce({ ...page, released: "2013-06-18", studio: "Night Records" });
    const { container, unmount } = render(<AlbumPage id={album.id} />);
    await screen.findByText(formatReleaseDate("2013-06-18")!);
    expect(screen.getByText(`© ${album.year} Night Records`)).toBeInTheDocument();
    unmount();

    vi.spyOn(backend, "albumPage").mockResolvedValueOnce({ ...page, released: null, studio: null });
    const again = render(<AlbumPage id={album.id} />);
    await screen.findByRole("grid", { name: `${album.title} songs` });
    expect(again.container.querySelector(".album-footer")).toBeNull();
    expect(container.querySelector(".album-footer")).toBeNull();
  });

  it("shows the artist photo in the meta line", async () => {
    const album = single();
    const { container } = render(<AlbumPage id={album.id} />);
    await screen.findByRole("grid", { name: `${album.title} songs` });
    await waitFor(() => expect(container.querySelector(".detail-header__artist-photo")).not.toBeNull());
  });

  it("names the plays column and shows a clock over the durations", async () => {
    const album = single();
    const { container } = render(<AlbumPage id={album.id} />);
    await screen.findByRole("grid", { name: `${album.title} songs` });
    expect(container.querySelector(".track-list__plays")).toHaveTextContent("Plays");
    expect(container.querySelector(".track-list__dur svg")).not.toBeNull();
  });

  it("switches between list and compact view and remembers it", async () => {
    const user = userEvent.setup();
    const album = single();
    const { container } = render(
      <>
        <AlbumPage id={album.id} />
        <ContextMenuHost />
      </>,
    );
    await screen.findByRole("grid", { name: `${album.title} songs` });
    expect(container.querySelector(".album-tracks--list")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "View as: List" }));
    await user.click(await screen.findByRole("menuitemradio", { name: "Compact" }));
    expect(container.querySelector(".album-tracks--compact")).not.toBeNull();
    expect(loadAlbumView()).toBe("compact");
    expect(container.querySelector(".album-tracks .track-row__artist")).toBeNull();
  });

  it("'See discography' opens the artist's discography page", async () => {
    const user = userEvent.setup();
    const album = single();
    render(<AlbumPage id={album.id} />);
    await user.click(await screen.findByRole("button", { name: "See discography" }));
    const { entries, index } = useRouter.getState();
    expect(entries[index].route).toEqual({ name: "discography", id: album.artistId });
  });

  it("'Add to queue' in the album menu queues every song and shows a toast with the cover", async () => {
    const user = userEvent.setup();
    const album = single();
    const n = mock.fx.tracksByAlbum.get(album.id)!.length;
    const add = vi.spyOn(backend, "queueAdd");
    render(
      <>
        <AlbumPage id={album.id} />
        <ContextMenuHost />
      </>,
    );
    await screen.findByRole("grid", { name: `${album.title} songs` });
    await user.click(screen.getByRole("button", { name: `More options for ${album.title}` }));
    await user.click(await screen.findByRole("menuitem", { name: "Add to queue" }));
    await waitFor(() => expect(toasts().map((x) => x.message)).toEqual([`Added ${n} songs to queue`]));
    expect(add).toHaveBeenCalledWith(mock.fx.tracksByAlbum.get(album.id)!.map((x) => x.id), "end");
    expect(toasts()[0].art?.path).toBe(album.thumb);
  });

  it("'Add to queue' on one song toasts with that song's cover; a failure toasts the error instead", async () => {
    const user = userEvent.setup();
    const album = single();
    const first = mock.fx.tracksByAlbum.get(album.id)![0];
    render(
      <>
        <AlbumPage id={album.id} />
        <ContextMenuHost />
      </>,
    );
    const row = await screen.findByRole("row", { name: new RegExp(first.title) });
    fireEvent.contextMenu(row, { clientX: 10, clientY: 10 });
    await user.click(await screen.findByRole("menuitem", { name: "Add to queue" }));
    await waitFor(() => expect(toasts().map((x) => x.message)).toEqual(["Added to queue"]));
    expect(toasts()[0].art?.path).toBe(first.thumb);

    useToasts.setState({ toasts: [] });
    vi.spyOn(backend, "queueAdd").mockRejectedValueOnce(new Error("Plex didn't answer"));
    fireEvent.contextMenu(row, { clientX: 10, clientY: 10 });
    await user.click(await screen.findByRole("menuitem", { name: "Add to queue" }));
    await waitFor(() => expect(toasts().length).toBe(1));
    expect(toasts()[0].kind).toBe("error");
  });
});

describe("queue panel", () => {
  beforeEach(async () => {
    await act(() => mock.queueClear());
  });

  it("has no empty placeholder: with nothing after the current song there's only Now playing", async () => {
    const album = mock.fx.albums.find((a) => a.trackCount === 1)!;
    const only = mock.fx.tracksByAlbum.get(album.id)![0];
    await act(() => mock.playContext({ context: { kind: "album", id: album.id } }));
    await waitFor(() => expect(usePlayer.getState().state!.track?.id).toBe(only.id));
    render(<QueuePanel />);
    expect(screen.getByRole("region", { name: "Now playing" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: /^Next/ })).toBeNull();
    expect(screen.queryByText("Nothing else queued.")).toBeNull();
    await act(() => mock.playPause());
  });

  it("offers 'Remove from queue' on queued and context rows, not on the playing one", async () => {
    const album = mock.fx.albums.find((a) => a.trackCount >= 4)!;
    const tracks = mock.fx.tracksByAlbum.get(album.id)!;
    const other = mock.fx.tracks.find((x) => x.albumId !== album.id)!;
    await act(() => mock.playContext({ context: { kind: "album", id: album.id } }));
    await act(() => mock.queueAdd([other.id], "end"));
    await waitFor(() => expect(usePlayer.getState().state!.queue.userQueue).toHaveLength(1));
    const remove = vi.spyOn(backend, "queueRemove");
    render(
      <>
        <QueuePanel />
        <ContextMenuHost />
      </>,
    );
    const menuOf = async (row: HTMLElement) => {
      fireEvent.contextMenu(row, { clientX: 10, clientY: 10 });
      const menu = await screen.findByRole("menu");
      const labels = within(menu).getAllByRole("menuitem").map((i) => i.textContent);
      act(() => useUi.setState({ menu: null }));
      return labels;
    };
    const now = within(screen.getByRole("region", { name: "Now playing" })).getByRole("row");
    expect(await menuOf(now)).not.toContain("Remove from queue");
    const queued = within(screen.getByRole("region", { name: "Next in queue" })).getByRole("row");
    expect(await menuOf(queued)).toContain("Remove from queue");
    const ctxRow = body(within(screen.getByRole("region", { name: `Next from: ${album.title}` })).getAllByRole("row"))[0];
    expect(await menuOf(ctxRow)).toContain("Remove from queue");

    fireEvent.contextMenu(ctxRow, { clientX: 10, clientY: 10 });
    fireEvent.click(await screen.findByRole("menuitem", { name: "Remove from queue" }));
    expect(remove).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(within(screen.getByRole("region", { name: `Next from: ${album.title}` })).queryByText(tracks[1].title)).toBeNull(),
    );
    expect(toasts()).toEqual([]);
    await act(() => mock.queueClear());
    await act(() => mock.playPause());
  });

  it("Recently played lists the listen log with repeats, and plays from a row", async () => {
    const user = userEvent.setup();
    const [a, b] = mock.fx.tracks;
    vi.spyOn(backend, "recentTracks").mockResolvedValue([a, b, a]);
    const play = vi.spyOn(backend, "playContext").mockResolvedValue();
    render(<QueuePanel />);
    await user.click(screen.getByRole("tab", { name: "Recently played" }));
    const list = await screen.findByRole("grid", { name: "Recently played" });
    expect(body(within(list).getAllByRole("row")).map((i) => i.getAttribute("data-track-id"))).toEqual([a.id, b.id, a.id]);
    fireEvent.doubleClick(body(within(list).getAllByRole("row"))[1]);
    await waitFor(() => expect(play).toHaveBeenCalled());
    expect(play.mock.calls[0][0]).toMatchObject({ context: { kind: "tracks", trackIds: [a.id, b.id, a.id] }, startTrackId: b.id });
  });

  it("Recently played says so when there's nothing, including when the log can't be read", async () => {
    const user = userEvent.setup();
    vi.spyOn(backend, "recentTracks").mockRejectedValue(new Error("database is locked"));
    render(<QueuePanel />);
    await user.click(screen.getByRole("tab", { name: "Recently played" }));
    expect(await screen.findByText("Songs you play will show up here.")).toBeInTheDocument();
  });
});
