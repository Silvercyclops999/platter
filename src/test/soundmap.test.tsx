/**
 * The sound map page: points from the core with a legend, colour by genre
 * or era, a lasso round songs to play or make a playlist, the empty state
 * while songs are analysed, finding a song, and the setting that hides it.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { SoundMapPage, walkOrder } from "../pages/SoundMapPage";
import { MapCanvas, type MapApi } from "../pages/soundmap/MapCanvas";
import {
  buildGrid,
  eraOf,
  fitView,
  inPolygon,
  joinChunks,
  lassoSelect,
  legend,
  nearest,
  nearestToward,
  rebase,
  toScreen,
  wheelPixels,
} from "../pages/soundmap/geometry";
import { AppMenu } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { player } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useSoundMap } from "../stores/soundmap";

const mock = backend as MockBackend;
const total = mock.fx.tracks.length;
const fmt = (n: number) => n.toLocaleString();
const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

beforeEach(async () => {
  // jsdom has no canvas drawing; the map still lays out and handles input.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  mock.soundmap.limit = null;
  mock.soundmap.analysing = true;
  mock.soundmap.revision = 0;
  useSoundMap.setState({ info: null, loading: false });
  useRouter.getState().reset();
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  await mock.settingsSet({ ...DEFAULT_SETTINGS, automix: true });
  useSettings.setState({ settings: null });
  await loadSettings();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("the map's maths", () => {
  const chunk = {
    ids: ["a", "b", "c", "d"],
    x: [0, 1, 0, 1],
    y: [0, 0, 1, 1],
    genre: [0, 0, 1, -1],
    genres: ["Soul", "Jazz"],
    year: [1965, 1994, 2021, 0],
    unreleased: [false, true, false, false],
    next: null,
  };
  it("joins chunks, groups by genre or era, and selects inside a lasso", () => {
    const d = joinChunks([chunk, { ...chunk, ids: ["e"], x: [5], y: [5], genre: [0], genres: ["Soul"], year: [2001], unreleased: [false] }]);
    expect(d.ids).toEqual(["a", "b", "c", "d", "e"]);
    expect(d.genre).toEqual(["Soul", "Soul", "Jazz", null, "Soul"]);
    expect(d.unreleased[1]).toBe(1);
    const g = legend(d, "genre");
    expect(g.groups.map((x) => [x.label, x.count])).toEqual([
      ["Soul", 3],
      ["Jazz", 1],
      ["Other", 1],
    ]);
    const e = legend(d, "era");
    expect(e.groups.find((x) => x.label === "1990s")?.count).toBe(1);
    expect(e.groups.find((x) => x.label === "Year unknown")?.count).toBe(1);
    expect([eraOf(1965), eraOf(1975), eraOf(2024), eraOf(0)]).toEqual([0, 1, 6, -1]);
    const square: [number, number][] = [
      [-0.5, -0.5],
      [1.5, -0.5],
      [1.5, 0.5],
      [-0.5, 0.5],
    ];
    expect(inPolygon(square, 0, 0)).toBe(true);
    expect(inPolygon(square, 0, 1)).toBe(false);
    expect(lassoSelect(d, square).map((i) => d.ids[i])).toEqual(["a", "b"]);
    // A walk goes from the leftmost song to the nearest each time.
    expect(walkOrder(d, [4, 1, 0, 3]).map((i) => d.ids[i])).toEqual(["a", "b", "d", "e"]);
  });
});

describe("the Sound map page", () => {
  it("shows every analysed song with a genre legend, and colours by era on request", async () => {
    const user = userEvent.setup();
    render(<SoundMapPage />);
    expect(await screen.findByText(new RegExp(`${fmt(total)} songs on the map`))).toBeInTheDocument();
    expect(screen.getByRole("application", { name: new RegExp(`Sound map of ${fmt(total)} songs`) })).toBeInTheDocument();
    const genres = screen.getByRole("list", { name: "Genres" });
    const genreItems = within(genres).getAllByRole("listitem");
    expect(genreItems.length).toBeGreaterThan(2);
    expect(within(genres).getByText("Unreleased")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Era" }));
    const eras = await screen.findByRole("list", { name: "Eras" });
    expect(screen.getByRole("button", { name: "Era" })).toHaveAttribute("aria-pressed", "true");
    const counts = within(eras)
      .getAllByRole("listitem")
      .map((li) => Number(li.querySelector(".soundmap__legend-count")?.textContent?.replace(/,/g, "") ?? 0));
    expect(counts.reduce((a, b) => a + b, 0)).toBe(total);
  });

  it("loads a big map in chunks", async () => {
    const spy = vi.spyOn(mock, "soundmapPoints");
    const page = await import("../pages/SoundMapPage");
    expect(page.CHUNK).toBeGreaterThan(1000);
    render(<SoundMapPage />);
    await screen.findByText(new RegExp(`${fmt(total)} songs on the map`));
    const fetched = await Promise.all(spy.mock.results.map((r) => r.value));
    expect(fetched.reduce((a, c) => a + c.ids.length, 0)).toBe(total);
  });

  it("a lasso selects songs to make a playlist from", async () => {
    const user = userEvent.setup();
    const create = vi.spyOn(mock, "playlistCreate");
    render(<SoundMapPage />);
    await screen.findByText(new RegExp(`${fmt(total)} songs on the map`));
    await user.click(screen.getByRole("button", { name: "Lasso" }));
    const canvas = screen.getByTestId("soundmap-canvas");
    // Round the whole map (the canvas is 800 by 600 where jsdom gives no size).
    const path = [
      [2, 2],
      [400, 1],
      [798, 2],
      [799, 300],
      [798, 598],
      [400, 599],
      [2, 598],
      [1, 300],
    ];
    fireEvent.pointerDown(canvas, { button: 0, pointerId: 1, clientX: path[0][0], clientY: path[0][1] });
    for (const [x, y] of path.slice(1)) fireEvent.pointerMove(canvas, { pointerId: 1, clientX: x, clientY: y });
    fireEvent.pointerUp(canvas, { pointerId: 1, clientX: 2, clientY: 300 });
    const sel = await screen.findByRole("region", { name: "Selection" });
    expect(within(sel).getByText(`${fmt(total)} songs selected`)).toBeInTheDocument();

    await user.click(within(sel).getByRole("button", { name: "Make playlist" }));
    const dialog = await screen.findByRole("dialog", { name: "Make playlist" });
    const name = within(dialog).getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Late night patch");
    await user.click(within(dialog).getByRole("button", { name: "Make playlist" }));
    await waitFor(() => expect(create).toHaveBeenCalled());
    const [title, ids] = create.mock.calls[0];
    expect(title).toBe("Late night patch");
    expect(new Set(ids)).toEqual(new Set(mock.fx.tracks.map((t) => t.id)));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Selection" })).not.toBeInTheDocument());
  });

  it("finds an artist and a song and flies there", async () => {
    const user = userEvent.setup();
    render(<SoundMapPage />);
    await screen.findByText(new RegExp(`${fmt(total)} songs on the map`));
    const t = mock.fx.tracks[3];
    await user.type(screen.getByRole("combobox", { name: "Find a song or artist on the map" }), t.title);
    const songs = await screen.findAllByRole("option", { name: new RegExp(`^${esc(t.title)}\\s*Song · `) });
    await user.click(songs[0]);
    expect(await screen.findByRole("button", { name: `Play ${t.title}` })).toBeInTheDocument();
  });

  it("search is a combobox: arrows move through the results, Enter goes, Shift+Enter plays, Escape closes then clears", async () => {
    const user = userEvent.setup();
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    render(<SoundMapPage />);
    await screen.findByText(new RegExp(`${fmt(total)} songs on the map`));
    const t = mock.fx.tracks[3];
    const box = screen.getByRole("combobox", { name: "Find a song or artist on the map" });
    await user.type(box, t.title);
    const list = await screen.findByRole("listbox", { name: "On the map" });
    expect(box).toHaveAttribute("aria-expanded", "true");
    expect(box).toHaveAttribute("aria-controls", list.id);
    const options = within(list).getAllByRole("option");
    expect(box).not.toHaveAttribute("aria-activedescendant");

    await user.keyboard("{ArrowDown}");
    expect(box).toHaveAttribute("aria-activedescendant", options[0].id);
    expect(options[0]).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{ArrowUp}");
    expect(box).toHaveAttribute("aria-activedescendant", options[options.length - 1].id);

    // Escape closes the list and keeps the search; the next one clears it.
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(box).toHaveValue(t.title);
    await user.keyboard("{Escape}");
    expect(box).toHaveValue("");

    // Shift+Enter on a song plays it.
    await user.type(box, t.title);
    const song = await screen.findAllByRole("option", { name: new RegExp(`^${esc(t.title)}\\s*Song · `) });
    const all = within(screen.getByRole("listbox")).getAllByRole("option");
    const k = all.indexOf(song[0]);
    for (let i = 0; i <= k; i++) await user.keyboard("{ArrowDown}");
    expect(box).toHaveAttribute("aria-activedescendant", song[0].id);
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    await waitFor(() => expect(play).toHaveBeenCalled());
    const ids = play.mock.calls[0][0];
    expect(mock.fx.tracks.find((x) => x.id === ids[0])?.title).toBe(t.title);
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("says the map fills in as songs are analysed, with the count and a way to Automix", async () => {
    mock.soundmap.limit = 0;
    mock.soundmap.analysing = false;
    render(<SoundMapPage />);
    const empty = await screen.findByTestId("soundmap-empty");
    expect(within(empty).getByText("The map fills in as songs are analysed")).toBeInTheDocument();
    expect(empty).toHaveTextContent(`0 of ${fmt(total)} songs analysed so far`);
    expect(empty).toHaveTextContent("Turn on Automix to start");
    expect(within(empty).getByRole("button", { name: "Automix settings" })).toBeInTheDocument();
    expect(screen.queryByTestId("soundmap-canvas")).not.toBeInTheDocument();
  });

  it("a partly done map says how far analysis has got", async () => {
    mock.soundmap.limit = 10;
    render(<SoundMapPage />);
    expect(await screen.findByText(/10 songs on the map/)).toBeInTheDocument();
    expect(screen.getByText(/More appear as Automix analyses them/)).toBeInTheDocument();
  });
});

/** Legend counts added up: how many points the page holds. */
function pointsShown(): number {
  const list = screen.getByRole("list", { name: /Genres|Eras/ });
  return within(list)
    .getAllByRole("listitem")
    .map((li) => Number(li.querySelector(".soundmap__legend-count")?.textContent?.replace(/,/g, "") ?? 0))
    .reduce((a, b) => a + b, 0);
}

/** Draws a lasso round the whole canvas (800 by 600 where jsdom gives no size). */
function lassoAll(canvas: HTMLElement, end: "up" | "cancel" = "up") {
  const path = [
    [2, 2],
    [400, 1],
    [798, 2],
    [799, 300],
    [798, 598],
    [400, 599],
    [2, 598],
    [1, 300],
  ];
  fireEvent.pointerDown(canvas, { button: 0, pointerId: 1, clientX: path[0][0], clientY: path[0][1] });
  for (const [x, y] of path.slice(1)) fireEvent.pointerMove(canvas, { pointerId: 1, clientX: x, clientY: y });
  if (end === "cancel") fireEvent.pointerCancel(canvas, { pointerId: 1, clientX: 2, clientY: 300 });
  fireEvent.pointerUp(canvas, { pointerId: 1, clientX: 2, clientY: 300 });
}

describe("the map keeps up with changes", () => {
  it("skips a reload when nothing changed, and keeps the selection by song when the points change", async () => {
    const user = userEvent.setup();
    const points = vi.spyOn(mock, "soundmapPoints");
    render(<SoundMapPage />);
    await screen.findByText(new RegExp(`${fmt(total)} songs on the map`));
    await user.click(screen.getByRole("button", { name: "Lasso" }));
    lassoAll(screen.getByTestId("soundmap-canvas"));
    const sel = await screen.findByRole("region", { name: "Selection" });
    expect(within(sel).getByText(`${fmt(total)} songs selected`)).toBeInTheDocument();
    const loads = points.mock.calls.length;

    // Told the map changed, but the core says it's the same map: nothing loaded.
    const info = vi.spyOn(mock, "soundmapInfo");
    act(() => mock.emit("soundmap://changed", null));
    await waitFor(() => expect(info).toHaveBeenCalled());
    await waitFor(() => expect(info.mock.results.length).toBeGreaterThan(0));
    await info.mock.results[0].value;
    expect(points.mock.calls.length).toBe(loads);

    // One song left the map: loaded again, and the selection keeps the rest.
    mock.soundmap.limit = total - 1;
    mock.soundmap.revision++;
    act(() => mock.emit("soundmap://changed", null));
    await waitFor(() => expect(within(screen.getByRole("region", { name: "Selection" })).getByText(`${fmt(total - 1)} songs selected`)).toBeInTheDocument());
    expect(points.mock.calls.length).toBeGreaterThan(loads);
  });

  it("a slow load that finishes after a newer one is ignored", async () => {
    const real = mock.soundmapPoints.bind(mock);
    let release: (() => void) | null = null;
    let first = true;
    vi.spyOn(mock, "soundmapPoints").mockImplementation((after, limit, area) => {
      const out = real(after, limit, area);
      if (!first) return out;
      first = false;
      // The first load hangs until let go.
      return new Promise((done) => {
        release = () => void out.then(done);
      });
    });
    render(<SoundMapPage />);
    await waitFor(() => expect(release).not.toBeNull());
    mock.soundmap.limit = 10;
    mock.soundmap.revision++;
    act(() => mock.emit("soundmap://changed", null));
    await screen.findByText(/10 songs on the map/);
    await waitFor(() => expect(pointsShown()).toBe(10));
    // The old, bigger map arrives late and changes nothing.
    act(() => release!());
    await act(async () => {
      await Promise.resolve();
    });
    expect(pointsShown()).toBe(10);
  });

  it("a cancelled pointer neither selects nor plays", async () => {
    const user = userEvent.setup();
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    render(<SoundMapPage />);
    await screen.findByText(new RegExp(`${fmt(total)} songs on the map`));
    const canvas = screen.getByTestId("soundmap-canvas");
    // A tap right on a song that the system cancels: not played.
    const d = joinChunks([mock.soundmap.points(null, 20_000, null)]);
    const [sx, sy] = toScreen(fitView(d, 800, 600), 800, 600, d.x[0], d.y[0]);
    fireEvent.pointerDown(canvas, { button: 0, pointerId: 3, clientX: sx, clientY: sy });
    fireEvent.pointerCancel(canvas, { pointerId: 3, clientX: sx, clientY: sy });
    expect(play).not.toHaveBeenCalled();
    // The same tap let go normally plays it (so the place was right).
    fireEvent.pointerDown(canvas, { button: 0, pointerId: 4, clientX: sx, clientY: sy });
    fireEvent.pointerUp(canvas, { pointerId: 4, clientX: sx, clientY: sy });
    await waitFor(() => expect(play).toHaveBeenCalledTimes(1));
    // A lasso that the system cancels: nothing selected.
    await user.click(screen.getByRole("button", { name: "Lasso" }));
    lassoAll(canvas, "cancel");
    expect(screen.queryByRole("region", { name: "Selection" })).not.toBeInTheDocument();
    // A full lasso afterwards still works, so the map wasn't left mid gesture.
    lassoAll(canvas);
    await screen.findByRole("region", { name: "Selection" });
    expect(play).toHaveBeenCalledTimes(1);
  });

  it("keys: Enter picks the song in the middle, arrows step between songs, Enter plays, and it's announced", async () => {
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined);
    render(<SoundMapPage />);
    await screen.findByText(new RegExp(`${fmt(total)} songs on the map`));
    // The same points the page has, to know which songs the keys should reach.
    const d = joinChunks([mock.soundmap.points(null, 20_000, null)]);
    const v = fitView(d, 800, 600);
    const g = buildGrid(d);
    let start = -1;
    for (let r = 20; start < 0; r *= 2) start = nearest(d, g, v.cx, v.cy, r / v.scale);
    const dirs: [string, number, number][] = [
      ["ArrowRight", 1, 0],
      ["ArrowLeft", -1, 0],
      ["ArrowUp", 0, 1],
      ["ArrowDown", 0, -1],
    ];
    const [key, dx, dy] = dirs.find(([, x, y]) => nearestToward(d, start, x, y) >= 0)!;
    const next = nearestToward(d, start, dx, dy);
    const title = (i: number) => mock.fx.tracks.find((t) => t.id === d.ids[i])!.title;

    const canvas = screen.getByRole("application", { name: /Sound map of/ });
    canvas.focus();
    fireEvent.keyDown(canvas, { key: "Enter" });
    expect(await screen.findByRole("button", { name: `Play ${title(start)}` })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("soundmap-status")).toHaveTextContent(`${title(start)} by`));
    fireEvent.keyDown(canvas, { key });
    expect(await screen.findByRole("button", { name: `Play ${title(next)}` })).toBeInTheDocument();
    fireEvent.keyDown(canvas, { key: "Enter" });
    await waitFor(() => expect(play).toHaveBeenCalled());
    expect(play.mock.calls[0][0]).toEqual([d.ids[next]]);
  });

  it("announces the selection size in a live region that's always there", async () => {
    const user = userEvent.setup();
    render(<SoundMapPage />);
    await screen.findByText(new RegExp(`${fmt(total)} songs on the map`));
    const status = screen.getByTestId("soundmap-status");
    expect(status).toHaveAttribute("aria-live", "polite");
    await user.click(screen.getByRole("button", { name: "Lasso" }));
    lassoAll(screen.getByTestId("soundmap-canvas"));
    await waitFor(() => expect(status).toHaveTextContent(`${fmt(total)} songs selected`));
  });
});

describe("the map canvas", () => {
  const chunk = (ids: string[], x: number[], y: number[]) => ({
    ids,
    x,
    y,
    genre: ids.map(() => -1),
    genres: [],
    year: ids.map(() => 0),
    unreleased: ids.map(() => false),
    next: null,
  });
  const small = joinChunks([chunk(["a", "b", "c"], [-0.5, 0, 0.5], [-0.5, 0, 0.5])]);
  const big = joinChunks([chunk(["a", "b", "c", "d"], [-0.5, 0, 0.5, 3], [-0.5, 0, 0.5, 3])]);
  const props = (data: typeof small, api: { current: MapApi | null }) => ({
    data,
    legend: legend(data, "genre"),
    colours: ["#999"],
    ring: "#fff",
    highlight: "#fff",
    mode: "move" as const,
    selected: null,
    focus: -1,
    api,
    label: "Map",
    onHover: () => {},
    onPick: () => {},
    onLasso: () => {},
    onEscape: () => {},
    onFocus: () => {},
    onPlay: () => {},
  });

  it("plus and minus zoom, but not with Ctrl or Cmd held", () => {
    const api: { current: MapApi | null } = { current: null };
    render(<MapCanvas {...props(small, api)} />);
    const canvas = screen.getByRole("application", { name: "Map" });
    expect(api.current!.zoom()).toBeCloseTo(1);
    expect(fireEvent.keyDown(canvas, { key: "+", ctrlKey: true })).toBe(true);
    expect(fireEvent.keyDown(canvas, { key: "=", metaKey: true })).toBe(true);
    expect(fireEvent.keyDown(canvas, { key: "-", ctrlKey: true })).toBe(true);
    expect(api.current!.zoom()).toBeCloseTo(1);
    expect(fireEvent.keyDown(canvas, { key: "+" })).toBe(false);
    expect(api.current!.zoom()).toBeCloseTo(1.4);
  });

  it("keeps the zoom when the points change", () => {
    const api: { current: MapApi | null } = { current: null };
    const r = render(<MapCanvas {...props(small, api)} />);
    const canvas = screen.getByRole("application", { name: "Map" });
    fireEvent.keyDown(canvas, { key: "+" });
    fireEvent.keyDown(canvas, { key: "+" });
    const z = api.current!.zoom();
    expect(z).toBeCloseTo(1.96);
    r.rerender(<MapCanvas {...props(big, api)} />);
    expect(api.current!.zoom()).toBeCloseTo(z);
  });
});

describe("the map's helpers", () => {
  it("rebases a view, reads wheel lines and pages, and finds the next song in a direction", () => {
    expect(rebase({ cx: 1, cy: 2, scale: 300 }, 100, 50)).toEqual({ cx: 1, cy: 2, scale: 150 });
    expect(rebase({ cx: 0, cy: 0, scale: 300 }, 0, 50).scale).toBe(300);
    expect([wheelPixels(3, 0, 600), wheelPixels(3, 1, 600), wheelPixels(1, 2, 600)]).toEqual([3, 48, 600]);
    const d = joinChunks([
      {
        ids: ["m", "r", "far", "u", "diag"],
        x: [0, 1, 5, 0, 1],
        y: [0, 0, 0.2, 2, 3],
        genre: [-1, -1, -1, -1, -1],
        genres: [],
        year: [0, 0, 0, 0, 0],
        unreleased: [false, false, false, false, false],
        next: null,
      },
    ]);
    expect(d.ids[nearestToward(d, 0, 1, 0)]).toBe("r");
    expect(d.ids[nearestToward(d, 0, 0, 1)]).toBe("u");
    expect(nearestToward(d, 0, -1, 0)).toBe(-1);
    expect(nearestToward(d, 0, 0, -1)).toBe(-1);
  });
});

describe("the Show sound map setting", () => {
  it("is on by default and turning it off hides the app menu entry and the page", async () => {
    const user = userEvent.setup();
    const menu = render(<AppMenu />);
    fireEvent.click(screen.getByRole("button", { name: /^App menu/ }));
    expect(await screen.findByRole("menuitem", { name: "Sound map" })).toBeInTheDocument();
    menu.unmount();

    render(<SettingsPage />);
    const toggle = await screen.findByRole("switch", { name: "Show the sound map" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    await user.click(toggle);
    await waitFor(() => expect(useSettings.getState().settings?.showSoundMap).toBe(false));

    render(<AppMenu />);
    const avatars = screen.getAllByRole("button", { name: /^App menu/ });
    fireEvent.click(avatars[avatars.length - 1]);
    expect(screen.getByRole("menuitem", { name: "Devices" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Sound map" })).not.toBeInTheDocument();
  });

  it("the entry waits for analysis when Automix is off", async () => {
    await mock.settingsSet({ automix: false });
    await loadSettings();
    mock.soundmap.limit = 0;
    render(<AppMenu />);
    fireEvent.click(screen.getByRole("button", { name: /^App menu/ }));
    await waitFor(() => expect(useSoundMap.getState().info).not.toBeNull());
    expect(screen.queryByRole("menuitem", { name: "Sound map" })).not.toBeInTheDocument();
  });

  it("the page says when it's off", async () => {
    await mock.settingsSet({ showSoundMap: false });
    await loadSettings();
    render(<SoundMapPage />);
    expect(await screen.findByText("The sound map is turned off.")).toBeInTheDocument();
  });
});
