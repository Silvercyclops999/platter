import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { setReducedMotion } from "../design/motion";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { LibraryPage } from "../pages/LibraryPage";
import { SettingsPage } from "../pages/SettingsPage";
import { SPINE_FIXED, SPINE_MAX, SPINE_MIN, coverOnLeft, nearestOnShelf, packShelves, spineWidth } from "../shell/shelf";
import { loadLibraryView, useLibraryView, type LibEntry } from "../stores/library";
import { usePins } from "../stores/pins";
import { player } from "../stores/player";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

const e = (kind: LibEntry["kind"], id: string, title: string, more: Partial<LibEntry> = {}): LibEntry => ({
  kind,
  id,
  title,
  sortTitle: title,
  creator: null,
  creatorId: null,
  thumb: null,
  smart: false,
  format: null,
  addedAt: null,
  playedAt: null,
  pinned: false,
  ...more,
});

const spines = () => [...document.querySelectorAll<HTMLElement>(".lib-spine")];
const spineButtons = () => [...document.querySelectorAll<HTMLButtonElement>(".lib-spine__open")];
const titles = (sel: string) => [...document.querySelectorAll(sel)].map((x) => x.textContent);

async function renderShelf(filter: "album" | "all" = "album") {
  act(() => {
    useLibraryView.setState({ filter, sub: null, sort: "alpha", view: "list", wideView: "shelf", customOrder: [], recents: {} });
    useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  });
  useRouter.getState().navigate({ name: "library" });
  await act(() => useLibraryView.getState().loadItems());
  return render(
    <>
      <LibraryPage />
      <ContextMenuHost />
    </>,
  );
}

beforeEach(() => {
  usePins.setState({ pins: [] });
  useRouter.getState().reset();
  useUi.setState({ menu: null, sidebarCollapsed: false, sidebarArt: true });
  setReducedMotion(false);
});

afterEach(() => {
  vi.restoreAllMocks();
  setReducedMotion(false);
});

describe("shelf packing", () => {
  it("sizes spines from the song count in a narrow range, or one width with the setting off", () => {
    expect(spineWidth(1, true)).toBe(SPINE_MIN);
    expect(spineWidth(12, true)).toBeGreaterThan(spineWidth(6, true));
    expect(spineWidth(80, true)).toBe(SPINE_MAX);
    expect(spineWidth(12, false)).toBe(SPINE_FIXED);
    expect(spineWidth(null, true)).toBe(SPINE_FIXED);
  });

  it("fills shelves in order and wraps when a record won't fit", () => {
    const list = Array.from({ length: 30 }, (_, i) => e("album", `a${i}`, `A${i}`, { trackCount: 10 }));
    const l = packShelves(list, 300, true);
    // 10 songs: 32 wide plus a 2 gap, so 8 fit on a 300 shelf.
    expect(l.starts).toEqual([0, 8, 16, 24]);
    expect(l.shelfOf[9]).toBe(1);
    expect(l.x[9]).toBe(34);
    // Up from the second record on shelf two lands on the second of shelf one.
    expect(nearestOnShelf(l, list.length, 0, l.x[9] + l.w[9] / 2)).toBe(1);
    // The last record on a shelf opens its cover to the left.
    expect(coverOnLeft(l, 7, 300)).toBe(true);
    expect(coverOnLeft(l, 0, 300)).toBe(false);
  });

  it("keeps the shelf view in wide mode only", () => {
    const saved = loadLibraryView(JSON.stringify({ view: "shelf", wideView: "shelf" }));
    expect(saved.wideView).toBe("shelf");
    expect(saved.view).toBe("list");
  });
});

describe("record shelf view", () => {
  it("is picked from the sort and view menu and stands albums in sort order", async () => {
    const user = userEvent.setup();
    act(() => useLibraryView.setState({ wideView: "grid" }));
    await renderShelf();
    act(() => useLibraryView.setState({ wideView: "grid" }));
    const inGrid = titles(".lib-cell__title");
    expect(inGrid.length).toBeGreaterThan(3);
    await user.click(screen.getByRole("button", { name: /^Sort and view/ }));
    await user.click(await screen.findByRole("menuitemradio", { name: "Shelf" }));
    expect(useLibraryView.getState().wideView).toBe("shelf");
    const onShelf = titles(".lib-spine__title");
    const shown = Math.min(onShelf.length, inGrid.length);
    expect(shown).toBeGreaterThan(3);
    expect(onShelf.slice(0, shown)).toEqual(inGrid.slice(0, shown));
    // Spine text is the artist and title; the button says both.
    const first = mock.fx.albums.find((a) => a.title === inGrid[0])!;
    expect(spineButtons()[0]).toHaveAccessibleName(`${first.title}, ${first.artistTitle}`);
  });

  it("moves with the arrows, opens on Enter and plays on Space", async () => {
    const play = vi.spyOn(player, "playAlbum").mockResolvedValue(undefined as never);
    await renderShelf();
    const btns = spineButtons();
    act(() => btns[0].focus());
    // Focus pulls the record out with its cover.
    expect(spines()[0]).toHaveClass("is-out");
    expect(screen.getByTestId("spine-cover")).toBeInTheDocument();
    fireEvent.keyDown(btns[0], { key: "ArrowRight" });
    await waitFor(() => expect(document.activeElement).toBe(spineButtons()[1]));
    expect(spines()[0]).not.toHaveClass("is-out");
    fireEvent.keyDown(spineButtons()[1], { key: "ArrowLeft" });
    await waitFor(() => expect(document.activeElement).toBe(spineButtons()[0]));
    const titleOf = (b: HTMLElement) => b.querySelector(".lib-spine__title")!.textContent;
    const id = mock.fx.albums.find((a) => a.title === titleOf(spineButtons()[0]))!.id;
    fireEvent.keyDown(spineButtons()[0], { key: " " });
    expect(play).toHaveBeenCalledWith(id);
    expect(route().name).toBe("library");
    const user = userEvent.setup();
    await user.keyboard("{Enter}");
    expect(route()).toMatchObject({ name: "album", id });
  });

  it("plays from the cover's button and opens the album menu on right-click", async () => {
    const user = userEvent.setup();
    const play = vi.spyOn(player, "playAlbum").mockResolvedValue(undefined as never);
    await renderShelf();
    const spine = spines()[0];
    // (React reads enter and leave from mouseover and mouseout.)
    fireEvent.mouseOver(spine);
    expect(spine).toHaveClass("is-out");
    const title = spine.querySelector(".lib-spine__title")!.textContent!;
    await user.click(screen.getByRole("button", { name: `Play ${title}` }));
    expect(play).toHaveBeenCalledTimes(1);
    fireEvent.contextMenu(spine);
    expect(await screen.findByRole("menu", { name: `Options for ${title}` })).toBeInTheDocument();
    fireEvent.mouseOut(spine);
    expect(spine).not.toHaveClass("is-out");
  });

  it("colours a spine from the server's cover colour", async () => {
    await renderShelf();
    const titled = mock.fx.albums.filter((a) => a.tint);
    const spine = spines().find((s) => titled.some((a) => a.title === s.querySelector(".lib-spine__title")!.textContent));
    expect(spine).toBeDefined();
    expect(spine!.style.getPropertyValue("--spine-colour")).toMatch(/^rgb/);
  });

  it("follows the spine width setting", async () => {
    await renderShelf();
    const widths = () => spines().map((s) => s.style.width);
    expect(new Set(widths()).size).toBeGreaterThan(1);
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, spineWidthByTracks: false } }));
    expect(new Set(widths())).toEqual(new Set([`${SPINE_FIXED}px`]));
  });

  it("stands artists and playlists on the shelves as tiles", async () => {
    await renderShelf("all");
    expect(spines().length).toBeGreaterThan(0);
    const tiles = [...document.querySelectorAll(".lib-cell--shelf")];
    expect(tiles.length).toBeGreaterThan(0);
    const names = tiles.map((t) => t.querySelector(".lib-cell__title")!.textContent);
    const others = [...mock.fx.artists.map((a) => a.title), ...mock.fx.playlists.map((p) => p.title)];
    // No album is a tile; artists and playlists are.
    const albumOnly = mock.fx.albums.map((x) => x.title).filter((t) => !others.includes(t));
    expect(names.some((n) => albumOnly.includes(n!))).toBe(false);
    expect(names.some((n) => mock.fx.artists.some((x) => x.title === n))).toBe(true);
    expect(names.some((n) => mock.fx.playlists.some((x) => x.title === n))).toBe(true);
    // The tiles and spines share the arrow keys.
    const all = [...document.querySelectorAll<HTMLElement>(".lib-items--shelf [data-index]")].map((x) => Number(x.dataset.index));
    expect(all).toEqual([...all].sort((a, b) => a - b));
  });

  it("only highlights under reduced motion", async () => {
    setReducedMotion(true);
    await renderShelf();
    const spine = spines()[0];
    fireEvent.mouseOver(spine);
    expect(spine).toHaveClass("is-lit");
    expect(spine).not.toHaveClass("is-out");
    expect(screen.queryByTestId("spine-cover")).toBeNull();
  });
});

describe("spine width setting", () => {
  it("is a Library row in Settings, on by default", async () => {
    const user = userEvent.setup();
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS } }));
    const update = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    const sw = screen.getByRole("switch", { name: "Spine width follows track count" });
    expect(sw).toBeChecked();
    await user.click(sw);
    expect(update).toHaveBeenCalledWith(expect.objectContaining({ spineWidthByTracks: false }));
  });
});
