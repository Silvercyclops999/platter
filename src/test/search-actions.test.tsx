import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type SearchResults, type Track } from "../lib/types";
import { SearchPage } from "../pages/SearchPage";
import { SearchBox } from "../shell/SearchBox";
import { SHORTCUTS } from "../shell/shortcuts";
import { uniqueCompletions } from "../shell/searchModel";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts.map((t) => t.message);
const box = () => screen.getByRole("combobox", { name: "Search" });
const listbox = () => screen.queryByRole("listbox");
const options = () => within(screen.getByRole("listbox")).getAllByRole("option");

let songs: Track[];
/** The like control's name, liked or not. */
const likeName = (t: Track) => new RegExp(`^(Add ${t.title} to Liked Songs|${t.title} is in Liked Songs)`);

function results(over: Partial<SearchResults> = {}): SearchResults {
  return { top: null, songs, artists: [], albums: [], playlists: [], tookMs: 1, source: "local", ...over };
}

beforeEach(async () => {
  songs = mock.fx.tracks.slice(0, 3);
  useUi.setState({ paletteOpen: false, searchFocusTick: 0, menu: null });
  // Rows hide the like plus until hover; jsdom can't hover, so show it always.
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS, likeControlRows: "always" } });
  useRouter.getState().reset({ name: "home" });
  useToasts.setState({ toasts: [] });
  await backend.searchRecentsClear();
  vi.spyOn(backend, "search").mockResolvedValue(results());
  vi.spyOn(backend, "searchCompletions").mockResolvedValue([]);
});
afterEach(() => {
  act(() => useUi.setState({ menu: null }));
  vi.restoreAllMocks();
});

function Dropdown() {
  return (
    <>
      <SearchBox />
      <ContextMenuHost />
    </>
  );
}

/** Types a query and waits for the first song row. */
async function openWithSongs(user: ReturnType<typeof userEvent.setup>) {
  render(<Dropdown />);
  await user.click(box());
  await user.type(box(), "song");
  return await waitFor(() => {
    const row = options().find((o) => o.getAttribute("aria-label") === `${songs[0].title}, Song · ${songs[0].artistTitle}`);
    expect(row).toBeDefined();
    return row!;
  });
}

describe("song actions in the search dropdown", () => {
  it("song rows have play on the cover, add to queue, like and more options", async () => {
    const user = userEvent.setup();
    const row = await openWithSongs(user);
    const t = songs[0];
    expect(within(row).getByRole("button", { name: `Play ${t.title}` }).closest(".searchbox__art")).not.toBeNull();
    expect(within(row).getByRole("button", { name: `Add ${t.title} to queue` })).toBeInTheDocument();
    expect(within(row).getByRole("button", { name: likeName(t) })).toBeInTheDocument();
    expect(within(row).getByRole("button", { name: `More options for ${t.title}` })).toBeInTheDocument();
  });

  it("the hover add to queue queues the song and keeps the dropdown and the text", async () => {
    const add = vi.spyOn(backend, "queueAdd").mockResolvedValue();
    const user = userEvent.setup();
    const row = await openWithSongs(user);
    await user.click(within(row).getByRole("button", { name: `Add ${songs[0].title} to queue` }));
    expect(add).toHaveBeenCalledWith([songs[0].id], "end");
    await waitFor(() => expect(toasts()).toEqual(["Added to queue"]));
    expect(listbox()).toBeInTheDocument();
    expect(box()).toHaveValue("song");
    expect(box()).toHaveFocus();
    expect(useRouter.getState().entries[useRouter.getState().index].route.name).toBe("home");
  });

  it("Shift+Enter adds the highlighted song to the queue; Enter still plays it", async () => {
    const add = vi.spyOn(backend, "queueAdd").mockResolvedValue();
    const play = vi.spyOn(backend, "playContext").mockResolvedValue();
    const user = userEvent.setup();
    const row = await openWithSongs(user);
    const at = options().indexOf(row);
    for (let i = 0; i <= at; i++) await user.keyboard("{ArrowDown}");
    expect(row).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(add).toHaveBeenCalledWith([songs[0].id], "end");
    await waitFor(() => expect(toasts()).toEqual(["Added to queue"]));
    expect(listbox()).toBeInTheDocument();
    expect(box()).toHaveValue("song");
    expect(play).not.toHaveBeenCalled();

    await user.keyboard("{Enter}");
    await waitFor(() => expect(play).toHaveBeenCalled());
    expect(listbox()).not.toBeInTheDocument();
  });

  it("the shortcut is listed with the others", () => {
    expect(SHORTCUTS.find((s) => s.id === "queueFromSearch")?.keys).toEqual(["Shift", "Enter"]);
  });

  it("right-click opens the song menu with Add to queue near the top, over the open dropdown", async () => {
    const add = vi.spyOn(backend, "queueAdd").mockResolvedValue();
    const user = userEvent.setup();
    const row = await openWithSongs(user);
    await user.pointer({ keys: "[MouseRight]", target: row });
    const menu = await screen.findByRole("menu", { name: `Options for ${songs[0].title}` });
    const labels = within(menu).getAllByRole("menuitem").map((m) => m.textContent);
    expect(labels.indexOf("Add to queue")).toBeGreaterThanOrEqual(0);
    expect(labels.indexOf("Add to queue")).toBeLessThan(4);
    expect(listbox()).toBeInTheDocument();
    await user.click(within(menu).getByRole("menuitem", { name: "Add to queue" }));
    expect(add).toHaveBeenCalledWith([songs[0].id], "end");
    await waitFor(() => expect(toasts()).toEqual(["Added to queue"]));
    expect(listbox()).toBeInTheDocument();
    expect(box()).toHaveFocus();
  });

  it("Escape closes the more options menu first, then the dropdown", async () => {
    const user = userEvent.setup();
    const row = await openWithSongs(user);
    await user.click(within(row).getByRole("button", { name: `More options for ${songs[0].title}` }));
    expect(await screen.findByRole("menu")).toBeInTheDocument();
    expect(listbox()).toBeInTheDocument();

    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
    expect(listbox()).toBeInTheDocument();
    expect(box()).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(listbox()).not.toBeInTheDocument();
    expect(box()).toHaveValue("song");
  });

  it("drops completions that only differ in case or spacing", async () => {
    vi.mocked(backend.searchCompletions).mockResolvedValue(["Odessa wren", "odessa  Wren ", "Odessa Wren (feat. Lake)", "ODESSA WREN"]);
    const user = userEvent.setup();
    render(<Dropdown />);
    await user.click(box());
    await user.type(box(), "ode");
    await waitFor(() => expect(options().some((o) => o.classList.contains("searchbox__row--completion"))).toBe(true));
    const shown = options()
      .filter((o) => o.classList.contains("searchbox__row--completion"))
      .map((o) => o.textContent);
    expect(shown).toEqual(["Odessa wren", "Odessa Wren (feat. Lake)"]);
  });

  it("uniqueCompletions keeps the first of each and every real difference", () => {
    expect(uniqueCompletions(["A b", " a  B", "A b (feat. C)", "", "a b (FEAT. c)"])).toEqual(["A b", "A b (feat. C)"]);
  });
});

describe("song actions on the results page", () => {
  it("song rows have play on the cover, add to queue, like and more options", async () => {
    const add = vi.spyOn(backend, "queueAdd").mockResolvedValue();
    const user = userEvent.setup();
    render(<SearchPage query="song" />);
    const list = await screen.findByRole("list", { name: "Results" });
    const t = songs[0];
    const row = within(list).getByRole("listitem", { name: new RegExp(`^${t.title}, Song`) });
    expect(within(row).getByRole("button", { name: `Play ${t.title}` }).closest(".sr-row__art")).not.toBeNull();
    expect(within(row).getByRole("button", { name: likeName(t) })).toBeInTheDocument();
    expect(within(row).getByRole("button", { name: `More options for ${t.title}` })).toBeInTheDocument();
    await user.click(within(row).getByRole("button", { name: `Add ${t.title} to queue` }));
    expect(add).toHaveBeenCalledWith([t.id], "end");
    await waitFor(() => expect(toasts()).toEqual(["Added to queue"]));
  });

  it("right-click on a song row opens the song menu with Add to queue", async () => {
    const user = userEvent.setup();
    render(
      <>
        <SearchPage query="song" />
        <ContextMenuHost />
      </>,
    );
    const list = await screen.findByRole("list", { name: "Results" });
    const row = within(list).getByRole("listitem", { name: new RegExp(`^${songs[1].title}, Song`) });
    await user.pointer({ keys: "[MouseRight]", target: row });
    const menu = await screen.findByRole("menu", { name: `Options for ${songs[1].title}` });
    expect(within(menu).getByRole("menuitem", { name: "Add to queue" })).toBeInTheDocument();
  });

  it("a song top result gets play on the cover, like and more options", async () => {
    const t = songs[0];
    vi.mocked(backend.search).mockResolvedValue(results({ top: { kind: "track", track: t }, songs: songs.slice(1) }));
    const user = userEvent.setup();
    render(
      <>
        <SearchPage query="song" />
        <ContextMenuHost />
      </>,
    );
    const top = await screen.findByRole("region", { name: "Top result" });
    expect(within(top).getAllByRole("button", { name: `Play ${t.title}` }).some((b) => b.closest(".sr-top__art"))).toBe(true);
    expect(within(top).getByRole("button", { name: likeName(t) })).toBeInTheDocument();
    await user.click(within(top).getByRole("button", { name: `More options for ${t.title}` }));
    const menu = await screen.findByRole("menu", { name: `Options for ${t.title}` });
    expect(within(menu).getByRole("menuitem", { name: "Add to queue" })).toBeInTheDocument();
  });

  it("the songs table plays from the cover art and keeps like and more options", async () => {
    const user = userEvent.setup();
    render(<SearchPage query="song" />);
    await user.click(await screen.findByRole("button", { name: "Songs" }));
    const grid = await screen.findByRole("grid", { name: "Songs" });
    const t = songs[0];
    const row = within(grid).getByRole("row", { name: new RegExp(`^${t.title}`) });
    expect(within(row).getByRole("button", { name: new RegExp(`^Play ${t.title}`) }).closest(".track-row__art")).not.toBeNull();
    expect(within(row).getByRole("button", { name: likeName(t) })).toBeInTheDocument();
    expect(within(row).getByRole("button", { name: new RegExp(`^More options for ${t.title}`) })).toBeInTheDocument();
  });
});
