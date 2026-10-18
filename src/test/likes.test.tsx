/**
 * Likes: the save control (like at once, rollback when Plex refuses), the
 * saved-in menu (batch apply, cancel, rating hint), the context toggle, the
 * shortcut, and the Liked Songs page (sort, find, chips and counts, live add,
 * dimmed unlike with undo, empty state).
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { SaveButton } from "../components/SaveButton";
import { SavedInHost } from "../components/SavedInMenu";
import { Toaster } from "../components/Toast";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { Track } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { likedChips, likedRows, matchesFind, sortLiked } from "../pages/liked";
import { LikedPage } from "../pages/LikedPage";
import { SettingsPage } from "../pages/SettingsPage";
import { isLikeShortcut, useShortcuts } from "../shell/useShortcuts";
import { connectApp, useApp } from "../stores/app";
import { connectLikes, isLiked, ratingHint, starsLabel, useLikes } from "../stores/likes";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";
import { usePlayer } from "../stores/player";

const mock = backend as MockBackend;
const toastText = () => useToasts.getState().toasts.map((t) => t.message);
const unliked = () => mock.fx.tracks.filter((t) => !t.userRating);
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

let disconnect: Array<() => void> = [];
beforeAll(() => {
  disconnect = [connectApp(mock), connectLikes(mock)];
});
afterAll(() => disconnect.forEach((d) => d()));

beforeEach(async () => {
  useRouter.getState().reset();
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null });
  useLikes.setState({ savedIn: null });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  await mock.settingsSet({ likedMeans: "five", likeControlRows: "hover" });
  const lists = await backend.playlists();
  const ids = await backend.likedIds();
  act(() => {
    useApp.getState().setPlaylists(lists);
    useLikes.getState().setIds(ids);
  });
});

function Hosts() {
  return (
    <>
      <ContextMenuHost />
      <SavedInHost />
      <Toaster />
    </>
  );
}

function rows(tracks: Track[]) {
  return render(
    <>
      <TrackList label="Songs">
        {tracks.map((t, i) => (
          <TrackRow key={t.id} track={t} index={i + 1} onPlay={() => {}} />
        ))}
      </TrackList>
      <Hosts />
    </>,
  );
}

describe("helpers", () => {
  const t = (over: Partial<Track>): Track => ({ ...mock.fx.tracks[0], ...over });

  it("chips are the most common genres and moods, top 12, ties by name", () => {
    const list = [t({ genres: ["Rock", "Calm"] }), t({ genres: ["Rock"] }), t({ genres: ["Ambient", "rock"] }), t({ genres: [] })];
    expect(likedChips(list)).toEqual([
      { tag: "Rock", count: 3 },
      { tag: "Ambient", count: 1 },
      { tag: "Calm", count: 1 },
    ]);
    const many = Array.from({ length: 20 }, (_, i) => t({ genres: [`G${String(i).padStart(2, "0")}`] }));
    expect(likedChips(many)).toHaveLength(12);
  });

  it("find matches title, artists and album, ignoring case and accents", () => {
    const s = t({ title: "Señal", trackArtist: "Ana Paz", artistTitle: "Band", albumTitle: "Mareas" });
    expect(matchesFind(s, "senal")).toBe(true);
    expect(matchesFind(s, "PAZ")).toBe(true);
    expect(matchesFind(s, "mar band")).toBe(true);
    expect(matchesFind(s, "nope")).toBe(false);
  });

  it("date added sorts newest first; the rest A to Z", () => {
    const a = t({ id: "a", title: "Beta", ratedAt: 10, albumTitle: "Z" });
    const b = t({ id: "b", title: "alpha", ratedAt: 30, albumTitle: "Y" });
    const c = t({ id: "c", title: "Gamma", ratedAt: 20, albumTitle: "X" });
    expect(sortLiked([a, b, c], "added").map((x) => x.id)).toEqual(["b", "c", "a"]);
    expect(sortLiked([a, b, c], "title").map((x) => x.id)).toEqual(["b", "a", "c"]);
    expect(sortLiked([a, b, c], "album").map((x) => x.id)).toEqual(["c", "b", "a"]);
    expect(likedRows([a, b, c], { sort: "added", chip: null, query: "ga" }).map((x) => x.id)).toEqual(["c"]);
  });

  it("the rating hint only shows for one to four star songs", () => {
    expect(starsLabel(6)).toBe("3 stars");
    expect(starsLabel(9)).toBe("4.5 stars");
    expect(starsLabel(2)).toBe("1 star");
    expect(ratingHint({ userRating: 6 }, false, 10)).toBe("Rated 3 stars in Plex");
    expect(ratingHint({ userRating: 6 }, true, 10)).toBe("Was 3 stars in Plex");
    expect(ratingHint({ userRating: 10 }, true, 10)).toBeNull();
    expect(ratingHint({ userRating: null }, false, 10)).toBeNull();
    expect(ratingHint({ userRating: 8 }, true, 8)).toBeNull();
  });

  it("Alt+Shift+B is the like shortcut, by key position too", () => {
    const k = (over: Partial<KeyboardEvent>) => ({ altKey: true, shiftKey: true, ctrlKey: false, metaKey: false, key: "B", code: "KeyB", ...over });
    expect(isLikeShortcut(k({}))).toBe(true);
    expect(isLikeShortcut(k({ key: "ı" }))).toBe(true);
    expect(isLikeShortcut(k({ shiftKey: false }))).toBe(false);
    expect(isLikeShortcut(k({ ctrlKey: true }))).toBe(false);
  });
});

describe("save control", () => {
  // Rows hide the plus until hover; jsdom can't hover, so show it always.
  beforeEach(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, likeControlRows: "always" } }));

  it("one click likes at once, with Change in the toast; liked rows keep the tick", async () => {
    const user = userEvent.setup();
    const t = unliked()[0];
    rows([t]);
    const btn = screen.getByRole("button", { name: `Add ${t.title} to Liked Songs` });
    expect(btn).not.toHaveClass("is-liked");
    await user.click(btn);
    // Shown at once, before the core answers.
    expect(isLiked(t.id)).toBe(true);
    const tick = await screen.findByRole("button", { name: `${t.title} is in Liked Songs. Add to playlist` });
    expect(tick).toHaveClass("is-liked");
    await waitFor(() => expect(toastText()).toContain("Added to Liked Songs"));
    expect((await backend.likedIds())[0]).toBe(t.id);
    // Change opens the saved-in menu for that song.
    await user.click(screen.getByRole("button", { name: "Change" }));
    expect(await screen.findByRole("dialog", { name: `Add ${t.title} to playlist` })).toBeInTheDocument();
  });

  it("Plex refusing a like puts it back with a toast (app state)", async () => {
    const t = unliked()[1];
    rows([t]);
    mock.likes.refuseNextWrite();
    // fireEvent: the rollback lands a tick later, so the like shows first.
    fireEvent.click(screen.getByRole("button", { name: `Add ${t.title} to Liked Songs` }));
    expect(isLiked(t.id)).toBe(true);
    await waitFor(() => expect(toastText().some((m) => m.includes("Your server didn't save the like") && m.includes(t.title))).toBe(true));
    await waitFor(() => expect(isLiked(t.id)).toBe(false));
    expect(screen.getByRole("button", { name: `Add ${t.title} to Liked Songs` })).toBeInTheDocument();
  });

  it("a failed call rolls back at once", async () => {
    const user = userEvent.setup();
    const t = unliked()[2];
    rows([t]);
    const orig = backend.likesSet;
    backend.likesSet = () => Promise.reject({ kind: "network", message: "Offline" });
    try {
      await user.click(screen.getByRole("button", { name: `Add ${t.title} to Liked Songs` }));
      await waitFor(() => expect(toastText()).toContain("Couldn't add to Liked Songs: Offline"));
      expect(isLiked(t.id)).toBe(false);
    } finally {
      backend.likesSet = orig;
    }
  });

  it("row visibility follows the setting; the player bar always shows it", async () => {
    const t = unliked()[3];
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS } }));
    const { unmount } = rows([t]);
    expect(document.querySelector(".track-row .save-btn")).toHaveClass("save-btn--row");
    expect(document.querySelector(".track-row .save-btn")).not.toHaveClass("is-always");
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, likeControlRows: "always" } }));
    expect(document.querySelector(".track-row .save-btn")).toHaveClass("is-always");
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, likeControlRows: "off" } }));
    expect(document.querySelector(".track-row .save-btn")).toBeNull();
    unmount();
    render(<SaveButton track={t} place="bar" />);
    expect(screen.getByRole("button", { name: `Add ${t.title} to Liked Songs` })).toHaveClass("save-btn--bar");
  });
});

describe("saved-in menu", () => {
  async function open(t: Track) {
    const user = userEvent.setup();
    act(() => useLikes.getState().openSavedIn(t, null));
    render(<Hosts />);
    const dialog = await screen.findByRole("dialog", { name: `Add ${t.title} to playlist` });
    return { user, dialog };
  }

  it("lists Liked Songs first and the playlists holding the song, ticked; the rest unticked, newest first", async () => {
    const pl = mock.fx.playlists.find((p) => !p.smart)!;
    const t = mock.fx.tracks.find((x) => isLiked(x.id) && (mock.fx.playlistTracks.get(pl.id) ?? []).includes(x.id)) ?? mock.fx.tracks.find((x) => isLiked(x.id))!;
    const { dialog } = await open(t);
    const saved = within(dialog).getByRole("group", { name: "Saved in" });
    await waitFor(() => expect(within(dialog).queryByRole("group", { name: "Other playlists" })).toBeInTheDocument());
    const savedRows = within(saved).getAllByRole("checkbox");
    expect(savedRows[0]).toHaveTextContent("Liked Songs");
    savedRows.forEach((r) => r.getAttribute("aria-disabled") !== "true" && expect(r).toHaveAttribute("aria-checked", "true"));
    const others = within(within(dialog).getByRole("group", { name: "Other playlists" })).getAllByRole("checkbox");
    others.forEach((r) => expect(r).toHaveAttribute("aria-checked", "false"));
    const titles = others.map((r) => r.querySelector(".saved-in__name")!.textContent);
    const want = mock.fx.playlists
      .filter((p) => !p.smart && titles.includes(p.title))
      .sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0) || a.title.localeCompare(b.title))
      .map((p) => p.title);
    expect(titles).toEqual(want);
  });

  it("ticks apply together on Done: unlike and add to a playlist at once", async () => {
    const t = mock.fx.tracks.find((x) => isLiked(x.id))!;
    const { user, dialog } = await open(t);
    const others = await within(dialog).findByRole("group", { name: "Other playlists" });
    const target = within(others).getAllByRole("checkbox")[0];
    const name = target.querySelector(".saved-in__name")!.textContent!;
    await user.click(within(dialog).getByRole("checkbox", { name: /Liked Songs/ }));
    await user.click(target);
    expect(target).toHaveAttribute("aria-checked", "true");
    // Nothing changed yet.
    expect(isLiked(t.id)).toBe(true);
    await user.click(within(dialog).getByRole("button", { name: "Done" }));
    await waitFor(() => expect(toastText()).toContain("Changes saved"));
    expect(isLiked(t.id)).toBe(false);
    const pl = mock.fx.playlists.find((p) => p.title === name)!;
    expect((await backend.playlistPage(pl.id)).tracks.map((x) => x.id)).toContain(t.id);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("Cancel and Escape change nothing", async () => {
    const t = mock.fx.tracks.find((x) => isLiked(x.id))!;
    const { user, dialog } = await open(t);
    await user.click(within(dialog).getByRole("checkbox", { name: /Liked Songs/ }));
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(isLiked(t.id)).toBe(true);
    act(() => useLikes.getState().openSavedIn(t, null));
    const again = await screen.findByRole("dialog");
    await user.click(within(again).getByRole("checkbox", { name: /Liked Songs/ }));
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(isLiked(t.id)).toBe(true);
    expect(toastText()).toEqual([]);
  });

  it("find narrows the playlists", async () => {
    const t = mock.fx.tracks.find((x) => isLiked(x.id))!;
    const { user, dialog } = await open(t);
    await within(dialog).findByRole("group", { name: "Other playlists" });
    await user.type(within(dialog).getByRole("searchbox", { name: "Find a playlist" }), "zzzz");
    expect(within(dialog).getByText('No playlists match "zzzz"')).toBeInTheDocument();
  });

  it("shows a 1 to 4 star rating as a grey hint by Liked Songs", async () => {
    const t = mock.fx.tracks.find((x) => x.userRating === 6)!;
    expect(t).toBeTruthy();
    const { dialog } = await open(t);
    expect(within(dialog).getByText(/Rated 3 stars in Plex/)).toBeInTheDocument();
  });
});

describe("context menu and shortcut", () => {
  it("the like item is second from the top and flips between add and remove", async () => {
    const user = userEvent.setup();
    const t = unliked()[4];
    rows([t]);
    fireEvent.contextMenu(screen.getByRole("row", { name: new RegExp(t.title) }));
    let menu = await screen.findByRole("menu");
    expect(within(menu).getAllByRole("menuitem")[1]).toHaveTextContent("Add to Liked Songs");
    await user.click(within(menu).getByRole("menuitem", { name: "Add to Liked Songs" }));
    expect(isLiked(t.id)).toBe(true);
    fireEvent.contextMenu(screen.getByRole("row", { name: new RegExp(t.title) }));
    menu = await screen.findByRole("menu");
    await user.click(within(menu).getByRole("menuitem", { name: "Remove from Liked Songs" }));
    expect(isLiked(t.id)).toBe(false);
    await waitFor(() => expect(toastText()).toContain("Removed from Liked Songs"));
  });

  it("a multi-selection likes every selected song", async () => {
    const user = userEvent.setup();
    const [a, b] = unliked().slice(5, 7);
    rows([a, b]);
    await user.click(screen.getByRole("row", { name: new RegExp(a.title) }));
    await user.keyboard("{Control>}");
    await user.click(screen.getByRole("row", { name: new RegExp(b.title) }));
    await user.keyboard("{/Control}");
    fireEvent.contextMenu(screen.getByRole("row", { name: new RegExp(b.title) }));
    const menu = await screen.findByRole("menu", { name: "Options for 2 songs" });
    expect(within(menu).getAllByRole("menuitem")[1]).toHaveTextContent("Add to Liked Songs");
    await user.click(within(menu).getByRole("menuitem", { name: "Add to Liked Songs" }));
    expect(isLiked(a.id) && isLiked(b.id)).toBe(true);
    await waitFor(() => expect(toastText()).toContain("Added 2 songs to Liked Songs"));
  });

  it("Alt+Shift+B likes and unlikes the playing song", async () => {
    function Keys() {
      useShortcuts();
      return null;
    }
    render(<Keys />);
    const state = await backend.playerState();
    act(() => usePlayer.setState({ state }));
    const t = state.track!;
    const was = isLiked(t.id);
    fireEvent.keyDown(window, { key: "B", code: "KeyB", altKey: true, shiftKey: true });
    await waitFor(() => expect(isLiked(t.id)).toBe(!was));
    fireEvent.keyDown(window, { key: "B", code: "KeyB", altKey: true, shiftKey: true });
    await waitFor(() => expect(isLiked(t.id)).toBe(was));
  });
});

describe("Liked Songs page", () => {
  const pageRows = () => Array.from(document.querySelectorAll(".liked .track-row .track-row__title")).map((el) => el.textContent);

  function page() {
    return render(
      <>
        <LikedPage />
        <Hosts />
      </>,
    );
  }

  it("lists liked songs newest first with count, date added, and sorts", async () => {
    const user = userEvent.setup();
    page();
    const liked = (await backend.likedPage()).tracks;
    await waitFor(() => expect(pageRows().length).toBeGreaterThan(0));
    expect(pageRows()[0]).toBe(liked[0].title);
    expect(screen.getByText(`${liked.length} songs,`)).toBeInTheDocument();
    expect(document.querySelector(".track-list__added")).toHaveTextContent("Date added");
    await user.click(screen.getByRole("button", { name: /^Sort and view/ }));
    await user.click(screen.getByRole("menuitemradio", { name: "Title" }));
    const sorted = [...liked].sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: "base", numeric: true }));
    expect(pageRows()[0]).toBe(sorted[0].title);
    await user.click(screen.getByRole("button", { name: /^Sort and view/ }));
    await user.click(screen.getByRole("menuitemradio", { name: "Date added" }));
  });

  it("find filters as you type; Escape clears", async () => {
    const user = userEvent.setup();
    page();
    const liked = (await backend.likedPage()).tracks;
    await waitFor(() => expect(pageRows().length).toBeGreaterThan(0));
    await user.click(screen.getByRole("button", { name: "Find in Liked Songs" }));
    const field = screen.getByRole("searchbox", { name: "Find in Liked Songs" });
    await user.type(field, liked[2].title);
    expect(pageRows()).toContain(liked[2].title);
    expect(pageRows().length).toBeLessThan(liked.length);
    await user.keyboard("{Escape}");
    expect(field).toHaveValue("");
    expect(pageRows()).toContain(liked[0].title);
  });

  it("a chip filters, updates the count and hides the other chips; clearing restores", async () => {
    const user = userEvent.setup();
    page();
    const liked = (await backend.likedPage()).tracks;
    const chips = likedChips(liked);
    const group = await screen.findByRole("group", { name: "Filter by genre or mood" });
    expect(within(group).getAllByRole("button")).toHaveLength(chips.length);
    await user.click(within(group).getByRole("button", { name: chips[0].tag }));
    expect(screen.getByText(`${chips[0].count} songs,`)).toBeInTheDocument();
    expect(within(group).getAllByRole("button")).toHaveLength(2);
    await user.click(within(group).getByRole("button", { name: "Clear filter" }));
    expect(within(group).getAllByRole("button")).toHaveLength(chips.length);
    expect(screen.getByText(`${liked.length} songs,`)).toBeInTheDocument();
  });

  it("a new like appears at the top with the fade", async () => {
    page();
    await waitFor(() => expect(pageRows().length).toBeGreaterThan(0));
    const t = unliked()[10];
    act(() => mock.likes.likeElsewhere(t.id));
    await waitFor(() => expect(pageRows()[0]).toBe(t.title));
    expect(document.querySelector(".liked .track-row")).toHaveClass("is-entering");
  });

  it("unliking leaves the row dimmed with Undo; Undo brings it back", async () => {
    const user = userEvent.setup();
    page();
    await waitFor(() => expect(pageRows().length).toBeGreaterThan(0));
    const first = document.querySelector<HTMLElement>(".liked .track-row")!;
    const title = first.querySelector(".track-row__title")!.textContent!;
    fireEvent.contextMenu(first);
    await user.click(await screen.findByRole("menuitem", { name: "Remove from Liked Songs" }));
    await waitFor(() => expect(document.querySelector(".liked .track-row")).toHaveClass("is-dimmed"));
    expect(pageRows()[0]).toBe(title);
    await user.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(document.querySelector(".liked .track-row")).not.toHaveClass("is-dimmed"));
  });

  it("empty: a short line and Find songs opens search", async () => {
    const user = userEvent.setup();
    mock.likes.clearAll();
    page();
    expect(await screen.findByText("Songs you like show up here")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Find songs" }));
    expect(route()).toEqual({ name: "search" });
  });
});

describe("settings", () => {
  it("Liked means and the row control live in Library, and change what counts and shows", async () => {
    const user = userEvent.setup();
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS } }));
    render(<SettingsPage />);
    const means = screen.getByRole("combobox", { name: "Liked means" });
    expect(means).toHaveValue("five");
    const rowsSel = screen.getByRole("combobox", { name: "Show the like control on song rows" });
    expect(rowsSel).toHaveValue("hover");
    const fourStar = mock.fx.tracks.find((t) => t.userRating === 8)!;
    expect(isLiked(fourStar.id)).toBe(false);
    await user.selectOptions(means, "fourUp");
    expect((await backend.settingsGet()).likedMeans).toBe("fourUp");
    await waitFor(() => expect(isLiked(fourStar.id)).toBe(true));
    await user.selectOptions(rowsSel, "off");
    expect((await backend.settingsGet()).likeControlRows).toBe("off");
    await user.selectOptions(means, "five");
    await waitFor(() => expect(isLiked(fourStar.id)).toBe(false));
  });
});
