/**
 * Crates: the Crates page (tabs, filter, sort, flipping through a crate
 * with the keyboard), a crate's page (Play all, the credit page link, the
 * sound map link), the album page's crate shelves (two at most, the rest
 * under More crates) in Studio and Classic, and both settings.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resolveTheme } from "../design/themes";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type AlbumShelf } from "../lib/types";
import { CratePage, CratesPage } from "../pages/CratesPage";
import { CrateShelves } from "../pages/CrateShelves";
import { crateLine, crateSentence, shelfTitle, yearSpan } from "../pages/crates";
import { player } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useTheme } from "../stores/theme";
import { ONLINE_EXTRAS } from "./onlineExtras";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;
const STUDIO = resolveTheme({ theme: "studio", accent: "brass", themeFile: null, followSystem: false }, true);
const CLASSIC = resolveTheme({ theme: "dark", accent: "ivory", themeFile: null, followSystem: false }, true);

async function setUp(patch: Record<string, unknown> = {}) {
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...ONLINE_EXTRAS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  useTheme.setState({ resolved: STUDIO });
  useRouter.getState().reset();
  await setUp();
});
afterEach(() => {
  vi.restoreAllMocks();
  useTheme.setState({ resolved: STUDIO });
});

describe("words", () => {
  it("leads each crate with what it holds", () => {
    expect(yearSpan(1991, 2004)).toBe("1991 to 2004");
    expect(yearSpan(1994, 1994)).toBe("1994");
    expect(yearSpan(null, null)).toBe("");
    expect(crateLine({ kind: "label", count: 14, artists: 9, fromYear: 1991, toYear: 2004 })).toBe("14 records by 9 artists, 1991 to 2004");
    expect(crateLine({ kind: "label", count: 3, artists: 3, fromYear: null, toYear: null })).toBe("3 records, each by a different artist");
    expect(crateLine({ kind: "producer", count: 2, artists: 1, fromYear: 2001, toYear: 2001 })).toBe("2 records by 1 artist, 2001");
    const c = { count: 6, artists: 4, fromYear: 1990, toYear: 1999 };
    expect(crateSentence({ ...c, kind: "label", name: "Low Tide" })).toBe("6 of your records came out on Low Tide, by 4 artists, from 1990 to 1999.");
    expect(crateSentence({ ...c, kind: "producer", name: "Max Volume" })).toBe("Max Volume produced 6 of your records, for 4 artists, from 1990 to 1999.");
    expect(crateSentence({ ...c, kind: "studio", name: "Sun" })).toBe("6 of your records were made at Sun, by 4 artists, from 1990 to 1999.");
    expect(crateSentence({ ...c, kind: "era", name: "1990s" })).toBe("6 of your records came out in the 1990s, by 4 artists.");
    expect(shelfTitle({ kind: "label", name: "Low Tide" })).toBe("More from Low Tide");
    expect(shelfTitle({ kind: "producer", name: "Max Volume" })).toBe("More produced by Max Volume");
    expect(shelfTitle({ kind: "studio", name: "Sun" })).toBe("Also recorded at Sun");
  });
});

describe("the Crates page", () => {
  it("shows a row of sleeves per crate, filters by name and sorts", async () => {
    const user = userEvent.setup();
    const labels = (await backend.crates("label")).crates;
    expect(labels.length).toBeGreaterThan(1);
    render(<CratesPage kind="label" />);
    const rows = await screen.findAllByTestId("crate");
    expect(rows).toHaveLength(Math.min(labels.length, 12));
    const first = labels[0];
    const row = rows[0];
    expect(within(row).getByRole("heading", { level: 2, name: first.name })).toBeInTheDocument();
    expect(within(row).getByText(crateLine(first))).toBeInTheDocument();
    expect(within(row).getAllByRole("listitem").length).toBeGreaterThanOrEqual(Math.min(first.count, 12));
    expect(screen.getByText(new RegExp(`^${labels.length} labels with two records or more`))).toBeInTheDocument();

    // A to Z.
    await user.click(screen.getByRole("button", { name: "A to Z" }));
    const sorted = [...labels].sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
    await waitFor(() => expect(screen.getAllByTestId("crate")[0]).toHaveTextContent(sorted[0].name));

    // The filter keeps crates whose name matches.
    const last = labels[labels.length - 1];
    await user.type(screen.getByRole("searchbox", { name: "Filter labels" }), last.name);
    await waitFor(() => expect(screen.getAllByTestId("crate").every((r) => r.textContent!.includes(last.name))).toBe(true));
    await user.clear(screen.getByRole("searchbox", { name: "Filter labels" }));
    await user.type(screen.getByRole("searchbox", { name: "Filter labels" }), "zzzz no such label");
    expect(await screen.findByText(/No labels match/)).toBeInTheDocument();
    expect(screen.queryAllByTestId("crate")).toHaveLength(0);
  });

  it("switches kind with the tabs, and a crate's name opens its page", async () => {
    const user = userEvent.setup();
    useRouter.getState().navigate({ name: "crates" });
    render(<CratesPage kind="label" />);
    const rows = await screen.findAllByTestId("crate");
    await user.click(screen.getByRole("tab", { name: "Eras" }));
    expect(route()).toEqual({ name: "crates", kind: "era" });
    const first = (await backend.crates("label")).crates[0];
    await user.click(within(rows[0]).getByRole("button", { name: first.name }));
    expect(route()).toEqual({ name: "crate", kind: "label", key: first.key });
  });

  it("flips through a crate with the arrow keys, Home and End", async () => {
    const user = userEvent.setup();
    const first = (await backend.crates("label")).crates[0];
    render(<CratesPage kind="label" />);
    const row = (await screen.findAllByTestId("crate"))[0];
    const list = within(row).getByRole("list");
    const links = () => [...list.querySelectorAll<HTMLElement>(".sleeve-card__link, .crate__all")];
    links()[0].focus();
    await user.keyboard("{ArrowRight}");
    expect(links()[1]).toHaveFocus();
    await user.keyboard("{ArrowLeft}");
    expect(links()[0]).toHaveFocus();
    await user.keyboard("{ArrowLeft}");
    expect(links()[0]).toHaveFocus();
    await user.keyboard("{End}");
    expect(links()[links().length - 1]).toHaveFocus();
    await user.keyboard("{Home}");
    expect(links()[0]).toHaveFocus();
    expect(links()[0]).toHaveTextContent(first.albums[0].title);
  });

  it("uses cards in Classic", async () => {
    useTheme.setState({ resolved: CLASSIC });
    render(<CratesPage kind="genre" />);
    const row = (await screen.findAllByTestId("crate"))[0];
    expect(row.querySelector(".card")).not.toBeNull();
    expect(row.querySelector(".sleeve-card")).toBeNull();
  });

  it("says crates are off while the setting is off, and asks nothing", async () => {
    await setUp({ showCrates: false });
    const load = vi.spyOn(backend, "crates");
    render(<CratesPage kind="label" />);
    expect(await screen.findByText("Crates are turned off")).toBeInTheDocument();
    expect(load).not.toHaveBeenCalled();
    expect(await backend.crates("label")).toEqual({ kind: "label", crates: [] });
  });
});

describe("a crate's page", () => {
  it("has a plate with the crate's name, plays everything in it and shuffles", async () => {
    const user = userEvent.setup();
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    const first = (await backend.crates("label")).crates[0];
    const page = (await backend.cratePage("label", first.key))!;
    render(<CratePage kind="label" crateKey={first.key} />);
    expect(await screen.findByRole("heading", { level: 1, name: first.name })).toBeInTheDocument();
    expect(screen.getByText(crateSentence({ ...page, count: page.albums.length }))).toBeInTheDocument();
    expect(within(screen.getByRole("list", { name: `${first.name}: records` })).getAllByRole("listitem")).toHaveLength(page.albums.length);
    await user.click(screen.getByRole("button", { name: "Play all" }));
    expect(play).toHaveBeenCalledWith(page.trackIds, page.trackIds[0]);
    await user.click(screen.getByRole("button", { name: "Shuffle" }));
    const order = play.mock.calls[1][0];
    expect([...order].sort()).toEqual([...page.trackIds].sort());
  });

  it("opens a record on the sound map, and a producer's credit page", async () => {
    const user = userEvent.setup();
    const producer = (await backend.crates("producer")).crates[0];
    expect(producer).toBeTruthy();
    render(<CratePage kind="producer" crateKey={producer.key} />);
    await screen.findByRole("heading", { level: 1, name: producer.name });
    const sounds = await screen.findAllByRole("button", { name: "Sounds like this record" });
    const page = (await backend.cratePage("producer", producer.key))!;
    await user.click(sounds[0]);
    expect(route()).toEqual({ name: "soundMap", album: page.albums.find((a) => a.onMap)!.album.id });
    await user.click(screen.getByRole("button", { name: `Everything ${producer.name} is credited on` }));
    expect(route()).toEqual({ name: "credits", person: producer.name });
  });

  it("hides the sound map link while the sound map is off, and says when a crate is gone", async () => {
    await setUp({ showSoundMap: false });
    const first = (await backend.crates("label")).crates[0];
    const { unmount } = render(<CratePage kind="label" crateKey={first.key} />);
    await screen.findByRole("heading", { level: 1, name: first.name });
    expect(screen.queryByRole("button", { name: "Sounds like this record" })).toBeNull();
    unmount();
    render(<CratePage kind="label" crateKey="no such label" />);
    expect(await screen.findByText(/This crate is empty now/)).toBeInTheDocument();
  });

  it("lists an era's years, each its own crate", async () => {
    const user = userEvent.setup();
    const era = (await backend.crates("era")).crates.find((c) => c.count > 2)!;
    const page = (await backend.cratePage("era", era.key))!;
    expect(page.years.length).toBeGreaterThan(0);
    render(<CratePage kind="era" crateKey={era.key} />);
    const years = await screen.findByRole("navigation", { name: "Years in this era" });
    await user.click(within(years).getByRole("button", { name: String(page.years[0]) }));
    expect(route()).toEqual({ name: "crate", kind: "year", key: String(page.years[0]) });
  });
});

describe("album page shelves", () => {
  const shelf = (kind: AlbumShelf["kind"], name: string, n: number): AlbumShelf => ({
    kind,
    key: name.toLowerCase(),
    name,
    count: n + 1,
    albums: mock.fx.albums.slice(0, n),
  });
  const three = () => [shelf("label", "Low Tide", 3), shelf("producer", "Max Volume", 2), shelf("studio", "Harbour", 2)];

  it("shows two shelves at most and lists the rest under More crates (Studio)", async () => {
    const user = userEvent.setup();
    vi.spyOn(backend, "albumCrates").mockResolvedValue(three());
    render(<CrateShelves albumId="1" />);
    expect(await screen.findByRole("heading", { level: 2, name: "More from Low Tide" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 2, name: "More produced by Max Volume" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { level: 2, name: "Also recorded at Harbour" })).toBeNull();
    const more = screen.getByRole("navigation", { name: "More crates" });
    await user.click(within(more).getByRole("button", { name: /Also recorded at Harbour/ }));
    expect(route()).toEqual({ name: "crate", kind: "studio", key: "harbour" });
    expect(screen.getByTestId("crate-shelf-label").querySelectorAll(".sleeve-card")).toHaveLength(3);
    await user.click(within(screen.getByTestId("crate-shelf-label")).getByRole("button", { name: "Open crate" }));
    expect(route()).toEqual({ name: "crate", kind: "label", key: "low tide" });
  });

  it("uses the usual shelves in Classic, and shows nothing when there are no crates", async () => {
    useTheme.setState({ resolved: CLASSIC });
    const load = vi.spyOn(backend, "albumCrates").mockResolvedValue(three().slice(0, 1));
    const { unmount } = render(<CrateShelves albumId="1" />);
    const heading = await screen.findByRole("heading", { level: 2, name: "More from Low Tide" });
    expect(heading.closest(".shelf")!.querySelectorAll(".card")).toHaveLength(3);
    expect(screen.queryByRole("navigation", { name: "More crates" })).toBeNull();
    unmount();
    load.mockResolvedValue([]);
    render(<CrateShelves albumId="2" />);
    await waitFor(() => expect(load).toHaveBeenCalledWith("2"));
    expect(screen.queryByTestId("crate-shelves")).toBeNull();
  });

  it("asks for nothing while either setting is off", async () => {
    const load = vi.spyOn(backend, "albumCrates");
    await setUp({ crateShelves: false });
    const { unmount } = render(<CrateShelves albumId="1" />);
    unmount();
    await setUp({ showCrates: false });
    render(<CrateShelves albumId="1" />);
    expect(load).not.toHaveBeenCalled();
    expect(screen.queryByTestId("crate-shelves")).toBeNull();
    expect(await mock.albumCrates(mock.fx.albums[0].id)).toEqual([]);
  });

  it("finds real shelves for a mock album, never the album itself", async () => {
    const label = (await backend.crates("label")).crates[0];
    const album = label.albums[0];
    const shelves = await backend.albumCrates(album.id);
    expect(shelves[0]).toMatchObject({ kind: "label", key: label.key, count: label.count });
    for (const s of shelves) {
      expect(["label", "producer", "studio"]).toContain(s.kind);
      expect(s.albums.some((a) => a.id === album.id)).toBe(false);
    }
  });
});
