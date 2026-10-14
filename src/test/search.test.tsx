import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import { DEFAULT_SETTINGS, type SearchArgs, type SearchResults, type Track } from "../lib/types";
import { MAX_QUERY, SearchPage } from "../pages/SearchPage";
import { SearchBox } from "../shell/SearchBox";
import { dropdownHits, mixedHits, typedPrefixLength } from "../shell/searchModel";
import { useShortcuts } from "../shell/useShortcuts";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const route = () => useRouter.getState().entries[useRouter.getState().index].route;

function Host() {
  useShortcuts();
  return <SearchBox />;
}

const box = () => screen.getByRole("combobox", { name: "Search" });
const options = () => within(screen.getByRole("listbox")).getAllByRole("option");

function track(id: string, title: string): Track {
  return {
    id,
    title,
    albumId: "al1",
    albumTitle: "Album",
    artistId: "ar1",
    artistTitle: "Artist",
    trackArtist: null,
    trackNumber: 1,
    discNumber: 1,
    durationMs: 180_000,
    thumb: null,
    partKey: "/p",
    codec: null,
    container: null,
    bitrate: null,
    bitDepth: null,
    sampleRate: null,
    userRating: null,
    viewCount: 0,
    lastViewedAt: null,
  } as Track;
}

function results(songTitles: string[]): SearchResults {
  return {
    top: null,
    songs: songTitles.map((t, i) => track(`t${i}-${t}`, t)),
    artists: [],
    albums: [],
    playlists: [],
    tookMs: 1,
    source: "local",
  };
}

beforeEach(async () => {
  useUi.setState({ paletteOpen: false, searchFocusTick: 0, menu: null });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useRouter.getState().reset({ name: "home" });
  await backend.searchRecentsClear();
});
afterEach(() => vi.restoreAllMocks());

// ---------------------------------------------------------------- results page

describe("results page", () => {
  it("chips pass the filter to the core and show one type at a time", async () => {
    const spy = vi.spyOn(backend, "search");
    const user = userEvent.setup();
    render(<SearchPage query="Odessa" />);
    expect(await screen.findByRole("region", { name: "Top result" })).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "Results" })).toBeInTheDocument();
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ query: "Odessa", filter: "all" }));
    const chips = within(screen.getByRole("group", { name: "Filter results" })).getAllByRole("button").map((b) => b.textContent);
    expect(chips).toEqual(["All", "Songs", "Albums", "Artists", "Playlists"]);

    await user.click(screen.getByRole("button", { name: "Songs" }));
    expect(screen.getByRole("button", { name: "Songs" })).toHaveAttribute("aria-pressed", "true");
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ filter: "songs" }));
    expect((await screen.findAllByRole("row", { name: /by Odessa Wren/ })).length).toBeGreaterThan(4);
    expect(screen.queryByRole("region", { name: "Top result" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Artists" }));
    expect(await screen.findByRole("list", { name: "Artists" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Albums" }));
    expect(spy).toHaveBeenLastCalledWith(expect.objectContaining({ filter: "albums" }));
  });

  it("puts the artist on top for an exact artist name", async () => {
    render(<SearchPage query="odessa wren" />);
    const top = await screen.findByRole("region", { name: "Top result" });
    expect(within(top).getByRole("button", { name: "Odessa Wren" })).toBeInTheDocument();
    expect(top).toHaveTextContent("Artist");
  });

  it("All is one mixed list without repeating the top result", () => {
    const res = results(["A", "B", "C"]);
    res.top = { kind: "track", track: res.songs[0] };
    res.albums = [{ id: "al", title: "Al", artistTitle: "x" } as never];
    const keys = mixedHits(res).map((h) => (h.track ?? h.album)!.id);
    expect(keys).toEqual(["t1-B", "t2-C", "al"]);
  });

  it("never shows results for an older query", async () => {
    const pending = new Map<string, (r: SearchResults) => void>();
    vi.spyOn(backend, "search").mockImplementation(
      (args: SearchArgs) => new Promise<SearchResults>((resolve) => pending.set(args.query, resolve)),
    );
    const { rerender } = render(<SearchPage query="gl" />);
    rerender(<SearchPage query="glo" />);
    rerender(<SearchPage query="glow" />);
    await act(async () => pending.get("glow")!(results(["Glow"])));
    expect(await screen.findByRole("listitem", { name: /^Glow, Song/ })).toBeInTheDocument();
    await act(async () => {
      pending.get("gl")!(results(["Glass Stale"]));
      pending.get("glo")!(results(["Gloom Stale"]));
    });
    expect(screen.queryByRole("listitem", { name: /Stale/ })).not.toBeInTheDocument();
  });

  it("clips very long queries and treats blank ones as empty", async () => {
    const spy = vi.spyOn(backend, "search");
    const { rerender } = render(<SearchPage query={"x".repeat(10_000)} />);
    await waitFor(() => expect(spy).toHaveBeenCalled());
    expect(spy.mock.calls[0][0].query.length).toBe(MAX_QUERY);
    expect(await screen.findByRole("heading", { name: /No results for/ })).toBeInTheDocument();
    spy.mockClear();
    rerender(<SearchPage query="    " />);
    expect(spy).not.toHaveBeenCalled();
    expect(screen.getByRole("heading", { name: "Search your library" })).toBeInTheDocument();
  });

  it("shows a calm message when search fails", async () => {
    vi.spyOn(backend, "search").mockRejectedValue({ kind: "internal", message: "boom" });
    render(<SearchPage query="anything" />);
    expect(await screen.findByRole("heading", { name: "Search isn't available right now" })).toBeInTheDocument();
  });

  it("opening a result remembers it as a recent search", async () => {
    const add = vi.spyOn(backend, "searchRecentAdd");
    const user = userEvent.setup();
    render(<SearchPage query="odessa wren" />);
    const top = await screen.findByRole("region", { name: "Top result" });
    await user.click(within(top).getByRole("button", { name: "Odessa Wren" }));
    expect(add).toHaveBeenCalledWith("artist", expect.any(String));
    expect(route().name).toBe("artist");
  });
});

// ---------------------------------------------------------------- dropdown

describe("search box dropdown", () => {
  it("shows recent searches on focus: opened items with art, play and remove", async () => {
    const [album] = (await backend.search({ query: "o", filter: "albums", limit: 1 })).albums;
    const [artist] = (await backend.search({ query: "odessa", filter: "artists", limit: 1 })).artists;
    await backend.searchRecentAdd("album", album.id);
    await backend.searchRecentAdd("artist", artist.id);
    const user = userEvent.setup();
    render(<Host />);
    await user.click(box());
    expect(await screen.findByRole("listbox", { name: "Recent searches" })).toBeInTheDocument();
    const rows = options();
    expect(rows[0]).toHaveAccessibleName(`${artist.title}, Artist`);
    expect(rows[1]).toHaveAccessibleName(new RegExp(`^${album.title}, Album`));
    expect(within(rows[0]).getByRole("button", { name: `Play ${artist.title}` })).toBeInTheDocument();

    await user.click(within(rows[0]).getByRole("button", { name: `Remove ${artist.title} from recent searches` }));
    await waitFor(() => expect(options()).toHaveLength(1));
    expect((await backend.searchRecents()).map((h) => h.kind)).toEqual(["album"]);
    expect(box()).toHaveFocus();
  });

  it("typing never adds a recent search; opening a result does", async () => {
    const add = vi.spyOn(backend, "searchRecentAdd");
    const user = userEvent.setup();
    render(<Host />);
    await user.click(box());
    await user.type(box(), "odessa wren");
    await waitFor(() => expect(options().some((o) => /, Artist$/.test(o.getAttribute("aria-label") ?? ""))).toBe(true));
    expect(add).not.toHaveBeenCalled();
    const artistRow = options().find((o) => /, Artist$/.test(o.getAttribute("aria-label") ?? ""))!;
    await user.click(artistRow);
    expect(add).toHaveBeenCalledWith("artist", expect.any(String));
    expect(route().name).toBe("artist");
  });

  it("with remembering off, shows no recents and adds none", async () => {
    const [album] = (await backend.search({ query: "o", filter: "albums", limit: 1 })).albums;
    await backend.searchRecentAdd("album", album.id);
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, searchRecents: false } });
    const add = vi.spyOn(backend, "searchRecentAdd");
    const user = userEvent.setup();
    render(<Host />);
    await user.click(box());
    await act(async () => {});
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    await user.type(box(), "odessa wren");
    await waitFor(() => expect(screen.getByRole("listbox")).toBeInTheDocument());
    await user.click(options().find((o) => /, Artist$/.test(o.getAttribute("aria-label") ?? ""))!);
    expect(add).not.toHaveBeenCalled();
  });

  it("clearing recent searches empties the list", async () => {
    const [album] = (await backend.search({ query: "o", filter: "albums", limit: 1 })).albums;
    await backend.searchRecentAdd("album", album.id);
    await backend.searchRecentsClear();
    expect(await backend.searchRecents()).toEqual([]);
  });

  it("shows up to four completions with the typed part bold, then results with their type", async () => {
    const user = userEvent.setup();
    render(<Host />);
    await user.click(box());
    await user.type(box(), "od");
    await waitFor(() => expect(screen.getByRole("listbox", { name: "Suggestions" })).toBeInTheDocument());
    await waitFor(() => expect(options().length).toBeGreaterThan(1));
    const rows = options();
    const completions = rows.filter((r) => r.classList.contains("searchbox__row--completion"));
    expect(completions.length).toBeGreaterThan(0);
    expect(completions.length).toBeLessThanOrEqual(4);
    expect(completions[0].querySelector("strong")!.textContent!.toLowerCase()).toBe("od");
    const hits = rows.filter((r) => !r.classList.contains("searchbox__row--completion"));
    expect(hits[0]).toHaveAccessibleName(/, Artist$/);
    // Completions come first.
    expect(rows.indexOf(completions[completions.length - 1])).toBeLessThan(rows.indexOf(hits[0]));
  });

  it("with suggestions off, typing shows no dropdown", async () => {
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS, searchSuggestions: false } });
    const spy = vi.spyOn(backend, "searchCompletions");
    const user = userEvent.setup();
    render(<Host />);
    await user.click(box());
    await user.type(box(), "odessa");
    await act(async () => {});
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(spy).not.toHaveBeenCalled();
    await user.keyboard("{Enter}");
    expect(route()).toEqual({ name: "search", query: "odessa" });
  });

  it("arrows move a highlight without changing the text", async () => {
    const user = userEvent.setup();
    render(<Host />);
    await user.click(box());
    await user.type(box(), "odessa");
    await waitFor(() => expect(options().length).toBeGreaterThan(2));
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(options()[1]).toHaveAttribute("aria-selected", "true");
    expect(box()).toHaveAttribute("aria-activedescendant", options()[1].id);
    expect(box()).toHaveValue("odessa");
    await user.keyboard("{ArrowUp}{ArrowUp}{ArrowUp}");
    expect(options().every((o) => o.getAttribute("aria-selected") === "false")).toBe(true);
    expect(box()).toHaveFocus();
  });

  it("first Escape closes the dropdown and keeps the text, the second clears it", async () => {
    const user = userEvent.setup();
    render(<Host />);
    await user.click(box());
    await user.type(box(), "odessa");
    await waitFor(() => expect(screen.getByRole("listbox")).toBeInTheDocument());
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(box()).toHaveValue("odessa");
    await user.keyboard("{Escape}");
    expect(box()).toHaveValue("");
    expect(box()).toHaveFocus();
  });

  it("Enter in the box opens the results page; Enter on a completion searches it", async () => {
    const user = userEvent.setup();
    render(<Host />);
    await user.click(box());
    await user.type(box(), "odessa");
    await user.keyboard("{Enter}");
    expect(route()).toEqual({ name: "search", query: "odessa" });
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();

    await user.clear(box());
    await user.type(box(), "od");
    await waitFor(() => expect(options()[0]).toHaveClass("searchbox__row--completion"));
    const text = options()[0].textContent!;
    await user.keyboard("{ArrowDown}{Enter}");
    expect(route()).toEqual({ name: "search", query: text });
    expect(box()).toHaveValue(text);
  });

  it("Enter on a highlighted result opens it", async () => {
    const add = vi.spyOn(backend, "searchRecentAdd");
    const user = userEvent.setup();
    render(<Host />);
    await user.click(box());
    await user.type(box(), "odessa wren");
    await waitFor(() => expect(options().some((o) => /, Artist$/.test(o.getAttribute("aria-label") ?? ""))).toBe(true));
    const at = options().findIndex((o) => /, Artist$/.test(o.getAttribute("aria-label") ?? ""));
    for (let i = 0; i <= at; i++) await user.keyboard("{ArrowDown}");
    await user.keyboard("{Enter}");
    expect(route().name).toBe("artist");
    expect(add).toHaveBeenCalledWith("artist", expect.any(String));
  });

  it("Ctrl+L focuses the box from anywhere", async () => {
    useRouter.getState().reset({ name: "album", id: "nope" });
    render(<Host />);
    fireEvent.keyDown(window, { key: "l", ctrlKey: true });
    await waitFor(() => expect(box()).toHaveFocus());
    expect(route().name).toBe("album");
  });
});

describe("search helpers", () => {
  it("bolds exactly the typed words, ignoring case and accents", () => {
    expect(typedPrefixLength("Lovesick Blues", "lovesick bl")).toBe(11);
    expect(typedPrefixLength("Beyoncé", "beyonce")).toBe(7);
    expect(typedPrefixLength("Tame Impala", "TAME")).toBe(4);
    expect(typedPrefixLength("Glove Box", "love")).toBe(0);
    expect(typedPrefixLength("Anything", "")).toBe(0);
  });

  it("dropdown results lead with the best artist and don't repeat", () => {
    const res = results(["S1", "S2"]);
    res.artists = [{ id: "a1", title: "Art" } as never];
    res.top = { kind: "track", track: res.songs[0] };
    const kinds = dropdownHits(res).map((h) => h.kind);
    expect(kinds).toEqual(["artist", "track", "track"]);
  });
});
