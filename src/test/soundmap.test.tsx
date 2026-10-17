/**
 * The sound map page: points from the core with a legend, colour by genre
 * or era, a lasso round songs to play or make a playlist, the empty state
 * while songs are analysed, finding a song, and the setting that hides it.
 */
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { SoundMapPage, walkOrder } from "../pages/SoundMapPage";
import { eraOf, inPolygon, joinChunks, lassoSelect, legend } from "../pages/soundmap/geometry";
import { AppMenu } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useSoundMap } from "../stores/soundmap";

const mock = backend as MockBackend;
const total = mock.fx.tracks.length;
const fmt = (n: number) => n.toLocaleString();

beforeEach(async () => {
  // jsdom has no canvas drawing; the map still lays out and handles input.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  mock.soundmap.limit = null;
  mock.soundmap.analysing = true;
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
    expect(screen.getByRole("img", { name: new RegExp(`Sound map of ${fmt(total)} songs`) })).toBeInTheDocument();
    const genres = screen.getByRole("list", { name: "Genres" });
    const genreItems = within(genres).getAllByRole("listitem");
    expect(genreItems.length).toBeGreaterThan(2);
    expect(within(genres).getByText("Unreleased")).toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: "Era" }));
    const eras = await screen.findByRole("list", { name: "Eras" });
    expect(screen.getByRole("radio", { name: "Era" })).toHaveAttribute("aria-checked", "true");
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
    await user.click(screen.getByRole("radio", { name: "Lasso" }));
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
    await user.type(screen.getByRole("searchbox", { name: "Find a song or artist on the map" }), t.title);
    const songs = await screen.findAllByRole("button", { name: new RegExp(`^${t.title.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*Song · `) });
    await user.click(songs[0]);
    expect(await screen.findByRole("button", { name: `Play ${t.title}` })).toBeInTheDocument();
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

describe("the Show sound map setting", () => {
  it("is on by default and turning it off hides the app menu entry and the page", async () => {
    const user = userEvent.setup();
    const menu = render(<AppMenu />);
    fireEvent.click(screen.getByRole("button", { name: /^App menu/ }));
    expect(await screen.findByRole("menuitem", { name: "Sound map" })).toBeInTheDocument();
    menu.unmount();

    render(<SettingsPage />);
    const toggle = await screen.findByRole("switch", { name: "Show sound map" });
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
