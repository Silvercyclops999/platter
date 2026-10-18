import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import axe from "axe-core";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { AppShell } from "../shell/AppShell";
import { PlayerBar } from "../shell/PlayerBar";
import { connectPlayer, player, usePlayer } from "../stores/player";
import { useRouter, type Route } from "../stores/router";
import { toast } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;

/**
 * axe on the whole document. Colour contrast can't be measured in jsdom (the
 * token tests check it), and the column resize handles are separators that
 * sit between the regions on purpose, so the "everything in a landmark" rule
 * skips them.
 */
async function audit(): Promise<string[]> {
  const res = await axe.run(
    { include: [document.body], exclude: [".resize-handle"] },
    { rules: { "color-contrast": { enabled: false } }, resultTypes: ["violations"] },
  );
  return res.violations.flatMap((v) => v.nodes.map((n) => `${v.id}: ${n.target.join(" ")}`));
}

function go(route: Route) {
  act(() => useRouter.getState().reset(route));
}

async function settle() {
  await act(() => new Promise((r) => setTimeout(r, 300)));
}

beforeEach(() => {
  useRouter.getState().reset();
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, outputOpen: false, sidebarCollapsed: false, sidebarWidth: 300, rightWidth: 340 });
});

const album = () => mock.fx.albums[0];
const screens: [string, () => Route, string | null][] = [
  ["Home", () => ({ name: "home" }), "Home"],
  ["Album", () => ({ name: "album", id: album().id }), null],
  ["Artist", () => ({ name: "artist", id: album().artistId }), null],
  ["Playlist", () => ({ name: "playlist", id: mock.fx.playlists[0].id }), null],
  ["Liked Songs", () => ({ name: "liked" }), "Liked Songs"],
  ["Search", () => ({ name: "search", query: album().title.split(" ")[0] }), "Search"],
  ["Your Library", () => ({ name: "library" }), "Your Library"],
  ["Recents", () => ({ name: "recents" }), "Recents"],
  ["Settings", () => ({ name: "settings" }), "Settings"],
];

describe("axe audit of the main screens", () => {
  it.each(screens)("%s has no violations, one h1 and a named main region", async (_name, route, label) => {
    render(<AppShell />);
    go(route());
    await waitFor(() => expect(document.querySelector(".page h1")).not.toBeNull(), { timeout: 3000 });
    await settle();
    expect(await audit()).toEqual([]);
    expect(document.querySelectorAll(".page h1")).toHaveLength(1);
    const main = screen.getByRole("main");
    // Fixed titles, or the page's own heading (an album's title).
    expect(main).toHaveAccessibleName(label ?? document.querySelector(".page h1")!.textContent!.trim());
    // Settings renders every group at once and axe walks all of it: slow on CI.
  }, 45_000);

  it("the queue panel has no violations", async () => {
    await act(() => mock.playContext({ context: { kind: "album", id: album().id } }));
    await act(() => mock.playPause());
    useUi.setState({ queueOpen: true });
    render(<AppShell />);
    await screen.findByRole("tab", { name: "Queue" });
    await settle();
    expect(await audit()).toEqual([]);
  });
});

describe("structure", () => {
  it("has the landmarks: banner with search, navigation, main, the player and the side panel", async () => {
    useUi.setState({ queueOpen: true });
    render(<AppShell />);
    await settle();
    // The top bar (headers inside regions aren't banners for screen readers).
    const banner = screen.getAllByRole("banner").find((el) => el.classList.contains("topbar"))!;
    expect(within(banner).getByRole("search")).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Main" })).toBeInTheDocument();
    expect(screen.getByRole("main", { name: "Home" })).toHaveAttribute("id", "main");
    expect(screen.getByRole("complementary", { name: "Player" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Queue" })).toBeInTheDocument();
    // Library items say where they are in the whole list.
    const items = await within(screen.getByRole("navigation", { name: "Main" })).findAllByRole("listitem");
    expect(items[0]).toHaveAttribute("aria-posinset", "1");
    expect(Number(items[0].getAttribute("aria-setsize"))).toBeGreaterThan(1);
    // The skip link goes to main.
    expect(screen.getByRole("link", { name: "Skip to main content" })).toHaveAttribute("href", "#main");
  });

  it("track lists are grids with row and column counts, numbered and selectable rows", async () => {
    render(<AppShell />);
    go({ name: "album", id: album().id });
    const tracks = mock.fx.tracksByAlbum.get(album().id)!;
    const grid = await screen.findByRole("grid", { name: `${album().title} songs` });
    await waitFor(() => expect(grid.querySelectorAll(".track-row")).toHaveLength(tracks.length));
    const discRows = grid.querySelectorAll(".album-disc, .album-side").length;
    expect(grid).toHaveAttribute("aria-rowcount", String(tracks.length + discRows + 1));
    expect(grid).toHaveAttribute("aria-colcount", "4");
    expect(within(grid).getAllByRole("columnheader").map((h) => h.textContent)).toEqual(["#", "Title", "Plays", "Duration"]);
    const first = grid.querySelector<HTMLElement>(".track-row")!;
    expect(first).toHaveAttribute("aria-rowindex", String(2 + (discRows ? 1 : 0)));
    expect(first).toHaveAttribute("aria-selected", "false");
    fireEvent.click(first);
    expect(first).toHaveAttribute("aria-selected", "true");
    // Buttons say what they act on.
    const t = tracks[0];
    expect(within(first).getByRole("button", { name: `Play ${t.title} by ${t.artistTitle}` })).toBeInTheDocument();
    expect(within(first).getByRole("button", { name: `More options for ${t.title} by ${t.artistTitle}` })).toBeInTheDocument();
  });
});

describe("the player for screen readers", () => {
  let disconnect: () => void;
  beforeAll(async () => {
    disconnect = connectPlayer(mock);
    await waitFor(() => expect(usePlayer.getState().state?.track).toBeTruthy());
  });
  afterAll(() => disconnect());

  it("names the playing song in a region and announces changes politely", async () => {
    render(<PlayerBar />);
    await act(() => mock.playContext({ context: { kind: "album", id: album().id } }));
    const t = () => usePlayer.getState().state!.track!;
    await waitFor(() => expect(screen.getByRole("region", { name: `Now playing: ${t().title} by ${t().artistTitle}` })).toBeInTheDocument());
    const first = t().title;
    await act(() => player.next());
    await waitFor(() => expect(t().title).not.toBe(first));
    await waitFor(() => expect(screen.getByTestId("now-playing-announcer")).toHaveTextContent(`Now playing: ${t().title} by ${t().artistTitle}`));
    expect(screen.getByTestId("now-playing-announcer")).toHaveAttribute("aria-live", "polite");
    await act(() => mock.playPause());
  });

  it("the progress slider speaks time of total and the volume slider a percentage", async () => {
    render(<PlayerBar />);
    await act(() => mock.playContext({ context: { kind: "album", id: album().id } }));
    await act(() => mock.playPause());
    const seek = await screen.findByRole("slider", { name: "Seek" });
    expect(seek.getAttribute("aria-valuetext")).toMatch(/^\d+:\d\d of \d+:\d\d$/);
    expect(screen.getByRole("slider", { name: "Volume" }).getAttribute("aria-valuetext")).toMatch(/^\d+%$/);
    // Toggles carry their pressed state.
    expect(screen.getByRole("button", { name: "Queue" })).toHaveAttribute("aria-pressed");
  });

  it("controls that can't be used say why", () => {
    usePlayer.setState({ state: { ...usePlayer.getState().state!, track: null } as never });
    render(<PlayerBar />);
    expect(screen.getByRole("slider", { name: "Seek, nothing playing" })).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("button", { name: "Expanded view, nothing playing" })).toBeDisabled();
  });
});

describe("toasts", () => {
  it("go to one polite status region, errors too", () => {
    render(<Toaster />);
    const region = screen.getByRole("status", { name: "Notifications" });
    expect(region).toHaveAttribute("aria-live", "polite");
    act(() => void toast.info("Added to queue"));
    act(() => void toast.error("Couldn't save"));
    expect(region).toHaveTextContent("Added to queue");
    expect(region).toHaveTextContent("Couldn't save");
    expect(screen.getAllByRole("status")).toHaveLength(1);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
