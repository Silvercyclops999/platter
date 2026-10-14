/**
 * Playlist editing: create and naming, the song finder, the add-to-playlist
 * submenu and its duplicate dialog, reorder by mouse and keys, remove with
 * undo, removing several, delete, edit details, smart playlists, and the two
 * playlist app states (Plex refuses a write; deleted on another device).
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost, placeSubmenu } from "../components/ContextMenu";
import { PlaylistDialogs } from "../components/PlaylistDialogs";
import { editablePlaylists } from "../components/PlaylistPicker";
import { Toaster } from "../components/Toast";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { nextPlaylistName } from "../lib/mock/playlists";
import type { Playlist } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { moveAfter, PlaylistPage, sortRows, stepAfter } from "../pages/PlaylistPage";
import { Sidebar } from "../shell/Sidebar";
import { connectApp, useApp } from "../stores/app";
import { addToPlaylist, createPlaylist, usePlaylistUi } from "../stores/playlists";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;
const SMART = "503";

let disconnect: () => void;
beforeAll(() => {
  disconnect = connectApp(mock);
});
afterAll(() => disconnect());

beforeEach(async () => {
  useRouter.getState().reset();
  useToasts.setState({ toasts: [] });
  usePlaylistUi.setState({ renameId: null, editing: null, deleting: null, duplicate: null, removing: null });
  useUi.setState({ menu: null });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  await mock.settingsSet({ playlistAskDuplicates: true, playlistNewName: "My playlist #N" });
  const lists = await backend.playlists();
  act(() => useApp.getState().setPlaylists(lists));
});

function Hosts() {
  return (
    <>
      <ContextMenuHost />
      <PlaylistDialogs />
      <Toaster />
    </>
  );
}

function page(id: string) {
  return render(
    <>
      <PlaylistPage id={id} />
      <Hosts />
    </>,
  );
}

const rowTitles = () => Array.from(document.querySelectorAll(".track-row .track-row__title")).map((el) => el.textContent);
const tracksOf = async (id: string) => (await backend.playlistPage(id)).tracks.map((t) => t.title);
const toastText = () => useToasts.getState().toasts.map((t) => t.message);

/** A playlist with these fixture tracks, made through the backend. */
async function makePlaylist(title: string, n: number): Promise<Playlist> {
  return backend.playlistCreate(title, mock.fx.tracks.slice(0, n).map((t) => t.id));
}

describe("helpers", () => {
  it("names count up from the first free number", () => {
    expect(nextPlaylistName([], "My playlist #N")).toBe("My playlist #1");
    expect(nextPlaylistName(["My playlist #1", "My playlist #3"], "My playlist #N")).toBe("My playlist #2");
    expect(nextPlaylistName(["Mix #1 (new)"], "Mix #N (new)")).toBe("Mix #2 (new)");
    expect(nextPlaylistName(["Untitled"], "Untitled")).toBe("Untitled");
  });

  it("moves rows to an insertion point as 'after this row'", () => {
    const o = ["a", "b", "c", "d"];
    expect(moveAfter(o, ["d"], 0)).toBeNull();
    expect(moveAfter(o, ["a"], 3)).toBe("c");
    expect(moveAfter(o, ["a", "c"], 4)).toBe("d");
    expect(moveAfter(o, ["b"], 1)).toBeUndefined(); // already there
    expect(stepAfter(o, ["c"], -1)).toBe("a");
    expect(stepAfter(o, ["b"], -1)).toBeNull();
    expect(stepAfter(o, ["a"], -1)).toBeUndefined();
    expect(stepAfter(o, ["b", "c"], 1)).toBe("d");
    expect(stepAfter(o, ["d"], 1)).toBeUndefined();
  });

  it("sorts rows; custom order is the playlist's own", () => {
    const [x, y] = mock.fx.tracks;
    const rows = [
      { itemId: "1", track: { ...x, title: "b", durationMs: 1 } },
      { itemId: "2", track: { ...y, title: "a", durationMs: 2 } },
    ];
    expect(sortRows(rows, "custom").map((r) => r.itemId)).toEqual(["1", "2"]);
    expect(sortRows(rows, "title").map((r) => r.itemId)).toEqual(["2", "1"]);
    expect(sortRows(rows, "duration").map((r) => r.itemId)).toEqual(["1", "2"]);
  });

  it("the picker lists editable playlists, most recently changed first", () => {
    const p = (id: string, updatedAt: number, smart = false): Playlist => ({ id, title: id, smart, trackCount: 0, durationMs: 0, thumb: null, updatedAt });
    expect(editablePlaylists([p("a", 1), p("b", 3), p("s", 9, true), p("c", 2)], "").map((x) => x.id)).toEqual(["b", "c", "a"]);
    expect(editablePlaylists([p("ab", 1), p("b", 3)], "a").map((x) => x.id)).toEqual(["ab"]);
    expect(editablePlaylists([p("a", 1), p("b", 3)], "", "b").map((x) => x.id)).toEqual(["a"]);
  });

  it("submenus open to the right, or to the left at the window edge", () => {
    const view = { width: 1000, height: 800 };
    expect(placeSubmenu({ left: 100, right: 300, top: 50 }, { width: 280, height: 300 }, view)).toEqual({ x: 296, y: 46 });
    expect(placeSubmenu({ left: 700, right: 900, top: 50 }, { width: 280, height: 300 }, view).x).toBe(424);
    expect(placeSubmenu({ left: 100, right: 300, top: 700 }, { width: 280, height: 300 }, view).y).toBe(492);
  });
});

describe("create", () => {
  it("the sidebar's + makes the next 'My playlist #N', opens it and puts focus on the name", async () => {
    const user = userEvent.setup();
    render(
      <>
        <Sidebar collapsed={false} />
        <Hosts />
      </>,
    );
    await user.click(screen.getByRole("button", { name: "Create playlist" }));
    await waitFor(() => expect(route().name).toBe("playlist"));
    const r = route() as { name: "playlist"; id: string };
    const made = (await backend.playlists()).find((p) => p.id === r.id)!;
    expect(made.title).toMatch(/^My playlist #\d+$/);
    expect(usePlaylistUi.getState().renameId).toBe(r.id);

    const second = await createPlaylist();
    const n = (t: string) => Number(t.split("#")[1]);
    expect(n(second!.title)).toBe(n(made.title) + 1);
  });

  it("typing renames it straight away; Enter saves", async () => {
    const user = userEvent.setup();
    const p = await createPlaylist();
    page(p!.id);
    const name = await screen.findByRole("textbox", { name: "Playlist name" });
    await waitFor(() => expect(name).toHaveFocus());
    await user.keyboard("Road songs{Enter}");
    await waitFor(async () => expect((await backend.playlistPage(p!.id)).playlist.title).toBe("Road songs"));
    expect(screen.queryByRole("textbox", { name: "Playlist name" })).not.toBeInTheDocument();
  });

  it("uses the name template from Settings", async () => {
    await mock.settingsSet({ playlistNewName: "Mix #N" });
    const p = await createPlaylist({ open: false });
    expect(p!.title).toBe("Mix #1");
  });
});

describe("empty playlist", () => {
  it("shows the finder; Add keeps it open and the song shows as already added", async () => {
    const user = userEvent.setup();
    const p = await backend.playlistCreate("Empty one", []);
    page(p.id);
    const finder = await screen.findByRole("region", { name: "Let's find something for your playlist" });
    const t = mock.fx.tracks[4];
    await user.type(within(finder).getByRole("searchbox", { name: "Search for songs" }), t.title);
    const add = await within(finder).findAllByRole("button", { name: `Add ${t.title} to Empty one` }, { timeout: 2000 });
    await user.click(add[0]);
    await waitFor(() => expect(toastText()).toContain("Added to Empty one"));
    expect(await tracksOf(p.id)).toHaveLength(1);
    // Still open, and the song now says it's there.
    const stillOpen = await screen.findByRole("region", { name: "Let's find something for your playlist" });
    expect(within(stillOpen).getAllByRole("button", { name: `${t.title} is already in Empty one` }).length).toBeGreaterThan(0);
    await waitFor(() => expect(rowTitles()).toHaveLength(1));
  });
});

describe("add to playlist submenu", () => {
  async function openPicker(user: ReturnType<typeof userEvent.setup>, trackIndex: number) {
    const t = mock.fx.tracks[trackIndex];
    render(
      <>
        <TrackList label="Tracks">
          <TrackRow track={t} index={1} onPlay={() => {}} />
        </TrackList>
        <Hosts />
      </>,
    );
    fireEvent.contextMenu(screen.getByRole("listitem"), { clientX: 10, clientY: 10 });
    await user.click(await screen.findByRole("menuitem", { name: "Add to playlist" }));
    return screen.findByRole("menu", { name: "Add to playlist" });
  }

  it("lists New playlist and your playlists, not smart ones; a find field filters", async () => {
    const user = userEvent.setup();
    const sub = await openPicker(user, 0);
    const names = within(sub).getAllByRole("menuitem").map((m) => m.textContent);
    expect(names[0]).toBe("New playlist");
    expect(names).toContain("Late Night Drive");
    expect(names).not.toContain("Most Played");
    expect(within(sub).getByRole("searchbox", { name: "Find a playlist" })).toHaveFocus();
    await user.keyboard("sunday");
    expect(within(sub).getAllByRole("menuitem").map((m) => m.textContent)).toEqual(["New playlist", "Sunday Coffee"]);
  });

  it("a song that's already there asks: Don't add (the default) adds nothing", async () => {
    const user = userEvent.setup();
    const target = await makePlaylist("Dup test", 2);
    await waitFor(() => expect(useApp.getState().playlists?.some((p) => p.id === target.id)).toBe(true));
    const sub = await openPicker(user, 0);
    await user.click(within(sub).getByRole("menuitem", { name: "Dup test" }));
    const dlg = await screen.findByRole("dialog", { name: "Already added" });
    expect(dlg).toHaveTextContent(`This song is already in "Dup test".`);
    expect(within(dlg).queryByRole("button", { name: "Add new ones" })).not.toBeInTheDocument();
    await waitFor(() => expect(within(dlg).getByRole("button", { name: "Don't add" })).toHaveFocus());
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect((await tracksOf(target.id)).length).toBe(2);
  });

  it("Add anyway adds it again", async () => {
    const user = userEvent.setup();
    const target = await makePlaylist("Dup anyway", 1);
    await waitFor(() => expect(useApp.getState().playlists?.some((p) => p.id === target.id)).toBe(true));
    const sub = await openPicker(user, 0);
    await user.click(within(sub).getByRole("menuitem", { name: "Dup anyway" }));
    await user.click(await screen.findByRole("button", { name: "Add anyway" }));
    await waitFor(async () => expect((await tracksOf(target.id)).length).toBe(2));
    expect(toastText()).toContain("Added to Dup anyway");
  });

  it("for several songs, Add new ones skips the ones already there", async () => {
    const user = userEvent.setup();
    const target = await makePlaylist("Dup several", 2);
    render(<Hosts />);
    const ids = mock.fx.tracks.slice(1, 4).map((t) => t.id); // one already there, two new
    await act(() => addToPlaylist(target, ids));
    const dlg = await screen.findByRole("dialog", { name: "Already added" });
    expect(dlg).toHaveTextContent("Some of these songs are already in");
    await user.click(within(dlg).getByRole("button", { name: "Add new ones" }));
    await waitFor(async () => expect((await tracksOf(target.id)).length).toBe(4));
  });

  it("asks nothing when the setting is off", async () => {
    const target = await makePlaylist("No ask", 1);
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, playlistAskDuplicates: false } }));
    render(<Hosts />);
    await act(() => addToPlaylist(target, [mock.fx.tracks[0].id]));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect((await tracksOf(target.id)).length).toBe(2);
  });

  it("New playlist makes one holding the song", async () => {
    const user = userEvent.setup();
    const sub = await openPicker(user, 6);
    await user.click(within(sub).getByRole("menuitem", { name: "New playlist" }));
    await waitFor(() => expect(toastText().some((m) => m.startsWith("Added to My playlist #"))).toBe(true));
    const made = (await backend.playlists()).sort((a, b) => Number(b.id) - Number(a.id))[0];
    expect(await tracksOf(made.id)).toEqual([mock.fx.tracks[6].title]);
  });
});

describe("reorder", () => {
  function dt() {
    const data = new Map<string, string>();
    return {
      setData: (k: string, v: string) => data.set(k, v),
      getData: (k: string) => data.get(k) ?? "",
      get types() {
        return [...data.keys()];
      },
      effectAllowed: "",
      dropEffect: "",
    };
  }

  it("drag a row in custom order: a drop line shows and the list renumbers", async () => {
    const p = await makePlaylist("Drag me", 4);
    page(p.id);
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(4));
    const before = rowTitles();
    const rows = screen.getAllByRole("listitem");
    expect(rows[3]).toHaveAttribute("draggable", "true");
    const transfer = dt();
    fireEvent.dragStart(rows[3], { dataTransfer: transfer });
    fireEvent.dragOver(rows[0], { dataTransfer: transfer, clientY: 0 });
    expect(rows[0]).toHaveClass("is-drop-before");
    fireEvent.drop(rows[0], { dataTransfer: transfer, clientY: 0 });
    expect(rowTitles()).toEqual([before[3], before[0], before[1], before[2]]);
    await waitFor(async () => expect(await tracksOf(p.id)).toEqual([before[3], before[0], before[1], before[2]]));
    expect(screen.getAllByRole("listitem")[0]).toHaveTextContent(/^1/);
  });

  it("Alt+Shift+Up and Down move the selected rows", async () => {
    const user = userEvent.setup();
    const p = await makePlaylist("Keys", 3);
    page(p.id);
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(3));
    const before = rowTitles();
    const row = screen.getAllByRole("listitem")[0];
    await user.click(row.querySelector(".track-row__title")!);
    await user.keyboard("{Alt>}{Shift>}{ArrowDown}{/Shift}{/Alt}");
    expect(rowTitles()).toEqual([before[1], before[0], before[2]]);
    await waitFor(async () => expect(await tracksOf(p.id)).toEqual([before[1], before[0], before[2]]));
    await user.keyboard("{Alt>}{Shift>}{ArrowUp}{/Shift}{/Alt}");
    await waitFor(async () => expect(await tracksOf(p.id)).toEqual(before));
  });

  it("other sorts show no drag and keys don't move rows", async () => {
    const user = userEvent.setup();
    const p = await makePlaylist("Sorted", 3);
    page(p.id);
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(3));
    await user.click(screen.getByRole("button", { name: "Sort songs: Custom order" }));
    await user.click(screen.getByRole("menuitemradio", { name: "Title" }));
    for (const r of screen.getAllByRole("listitem")) expect(r).not.toHaveAttribute("draggable", "true");
    const stored = await tracksOf(p.id);
    await user.click(screen.getAllByRole("listitem")[0].querySelector(".track-row__title")!);
    await user.keyboard("{Alt>}{Shift>}{ArrowDown}{/Shift}{/Alt}");
    expect(await tracksOf(p.id)).toEqual(stored);
  });
});

describe("remove", () => {
  it("'Remove from this playlist' removes at once; Undo puts it back where it was", async () => {
    const user = userEvent.setup();
    const p = await makePlaylist("Undo me", 3);
    page(p.id);
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(3));
    const before = rowTitles();
    fireEvent.contextMenu(screen.getAllByRole("listitem")[1], { clientX: 5, clientY: 5 });
    const menu = await screen.findByRole("menu");
    const labels = within(menu).getAllByRole("menuitem").map((m) => m.textContent);
    expect(labels.slice(0, 3)).toEqual(["Add to playlist", "Remove from this playlist", "Add to queue"]);
    await user.click(within(menu).getByRole("menuitem", { name: "Remove from this playlist" }));
    expect(rowTitles()).toEqual([before[0], before[2]]);
    await waitFor(async () => expect(await tracksOf(p.id)).toEqual([before[0], before[2]]));
    expect(toastText()).toContain("Removed from Undo me");
    await user.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(async () => expect(await tracksOf(p.id)).toEqual(before));
  });

  it("several selected rows and Delete ask first", async () => {
    const user = userEvent.setup();
    const p = await makePlaylist("Multi", 4);
    page(p.id);
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(4));
    const before = rowTitles();
    const rows = screen.getAllByRole("listitem");
    await user.click(rows[0].querySelector(".track-row__title")!);
    await user.keyboard("{Shift>}");
    await user.click(rows[1].querySelector(".track-row__title")!);
    await user.keyboard("{/Shift}{Delete}");
    const dlg = await screen.findByRole("dialog", { name: "Remove 2 songs?" });
    await user.click(within(dlg).getByRole("button", { name: "Cancel" }));
    expect(await tracksOf(p.id)).toEqual(before);
    await user.click(rows[0].querySelector(".track-row__title")!);
    await user.keyboard("{Shift>}");
    await user.click(rows[1].querySelector(".track-row__title")!);
    await user.keyboard("{/Shift}{Delete}");
    await user.click(within(await screen.findByRole("dialog", { name: "Remove 2 songs?" })).getByRole("button", { name: "Remove" }));
    await waitFor(async () => expect(await tracksOf(p.id)).toEqual(before.slice(2)));
  });
});

describe("delete and details", () => {
  it("delete asks, then goes Home with a toast", async () => {
    const user = userEvent.setup();
    const p = await makePlaylist("Delete me", 2);
    act(() => useRouter.getState().navigate({ name: "playlist", id: p.id }));
    page(p.id);
    await user.click(await screen.findByRole("button", { name: "More options for Delete me" }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete" }));
    const dlg = await screen.findByRole("dialog", { name: "Delete Delete me?" });
    await user.click(within(dlg).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(route().name).toBe("home"));
    expect(toastText()).toContain("Deleted Delete me");
    expect((await backend.playlists()).some((x) => x.id === p.id)).toBe(false);
  });

  it("edit details: name and description with counters, saved together", async () => {
    const user = userEvent.setup();
    const p = await makePlaylist("Details", 1);
    page(p.id);
    await user.click(await screen.findByRole("button", { name: "Details" }));
    const dlg = await screen.findByRole("dialog", { name: "Edit details" });
    const name = within(dlg).getByRole("textbox", { name: /Name/ });
    expect(within(dlg).getByText("7/100")).toBeInTheDocument();
    await user.clear(name);
    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    expect(within(dlg).getByRole("alert")).toHaveTextContent("Playlist name is required.");
    await user.type(name, "Better name");
    await user.type(within(dlg).getByRole("textbox", { name: /Description/ }), "For Sundays");
    expect(within(dlg).getByText("11/300")).toBeInTheDocument();
    await user.click(within(dlg).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    const saved = (await backend.playlistPage(p.id)).playlist;
    expect([saved.title, saved.summary]).toEqual(["Better name", "For Sundays"]);
    expect(await screen.findByText("For Sundays")).toBeInTheDocument();
  });

  it("Escape cancels edit details", async () => {
    const user = userEvent.setup();
    const p = await makePlaylist("Keep", 1);
    page(p.id);
    await user.click(await screen.findByRole("button", { name: "Edit details: choose a cover" }));
    const dlg = await screen.findByRole("dialog", { name: "Edit details" });
    await user.type(within(dlg).getByRole("textbox", { name: /Name/ }), " changed");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect((await backend.playlistPage(p.id)).playlist.title).toBe("Keep");
  });
});

describe("smart playlists", () => {
  it("are read only: no edits, no drag, no remove; the menu has play, queue and duplicate", async () => {
    const user = userEvent.setup();
    page(SMART);
    await waitFor(() => expect(screen.getAllByRole("listitem").length).toBeGreaterThan(1));
    expect(screen.getByText("Smart playlist")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Most Played" })).not.toBeInTheDocument();
    for (const r of screen.getAllByRole("listitem")) expect(r).not.toHaveAttribute("draggable", "true");
    await user.click(screen.getByRole("button", { name: "More options for Most Played" }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).getAllByRole("menuitem").map((m) => m.textContent)).toEqual(["Add to queue", "Duplicate", "Pin to sidebar"]);
    await user.keyboard("{Escape}");
    fireEvent.contextMenu(screen.getAllByRole("listitem")[0], { clientX: 5, clientY: 5 });
    const rowMenu = await screen.findByRole("menu");
    expect(within(rowMenu).queryByRole("menuitem", { name: "Remove from this playlist" })).not.toBeInTheDocument();
  });

  it("Duplicate makes an ordinary copy", async () => {
    const user = userEvent.setup();
    page(SMART);
    await user.click(await screen.findByRole("button", { name: "More options for Most Played" }));
    await user.click(await screen.findByRole("menuitem", { name: "Duplicate" }));
    await waitFor(() => expect(toastText()).toContain("Made Most Played (copy)"));
    const copy = (await backend.playlists()).find((p) => p.title === "Most Played (copy)")!;
    expect(copy.smart).toBe(false);
    expect(copy.trackCount).toBe((await backend.playlistPage(SMART)).tracks.length);
  });
});

describe("app states", () => {
  it("Plex refuses a playlist write: toast, and the playlist shows as Plex has it", async () => {
    const p = await makePlaylist("Refused", 2);
    page(p.id);
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(2));
    mock.pl.refuseNextWrite();
    await act(() => backend.playlistUpdate(p.id, { title: "Never" }));
    await waitFor(() => expect(toastText()).toContain(`Plex didn't accept a change to "Refused". Showing it as it is in Plex.`));
    await waitFor(async () => expect((await backend.playlistPage(p.id)).playlist.title).toBe("Refused"));
    expect(await screen.findByRole("button", { name: "Refused" })).toBeInTheDocument();
  });

  it("playlist deleted on another device while open: the page says so and goes Home", async () => {
    const p = await makePlaylist("Gone soon", 1);
    act(() => useRouter.getState().navigate({ name: "playlist", id: p.id }));
    page(p.id);
    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(1));
    act(() => mock.pl.deleteElsewhere(p.id));
    await waitFor(() => expect(route().name).toBe("home"));
    expect(toastText()).toContain("This playlist was deleted.");
  });
});
