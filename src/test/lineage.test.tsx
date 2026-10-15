import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ArchiveDialogs } from "../components/ArchiveDialogs";
import { ContextMenuHost } from "../components/ContextMenu";
import { StatsDialogHost } from "../components/StatsDialog";
import { trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { compareApproximate, compareMap, compareMapLoop } from "../lib/compare";
import type { MockBackend } from "../lib/mock";
import { ARCHIVE_ARTIST_ID } from "../lib/mock/archive";
import { DEFAULT_SETTINGS } from "../lib/types";
import { LineagePage, lineageSpan, nodeDate } from "../pages/LineagePage";
import { SettingsPage } from "../pages/SettingsPage";
import { openVersions } from "../stores/archive";
import { connectPlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";
import { useStatsDialog } from "../stores/stats";

const mock = backend as MockBackend;
const trackNamed = (title: string) => mock.fx.tracks.find((t) => t.title === title && t.artistId === ARCHIVE_ARTIST_ID)!;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

beforeEach(async () => {
  mock.archive.marks.clear();
  mock.compareFails = false;
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
  useRouter.getState().reset();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("compare moment mapping", () => {
  it("keeps the same time within 5%, else maps by proportion", () => {
    expect(compareApproximate(200_000, 205_000)).toBe(false);
    expect(compareApproximate(100_000, 95_000)).toBe(false);
    expect(compareApproximate(100_000, 94_999)).toBe(true);
    expect(compareMap(60_000, 200_000, 205_000)).toBe(60_000);
    expect(compareMap(50_000, 200_000, 100_000)).toBe(25_000);
    expect(compareMap(204_000, 205_000, 200_000)).toBe(199_999);
    expect(compareMap(10_000, 0, 100_000)).toBe(0);
    expect(compareMapLoop({ aMs: 20_000, bMs: 40_000 }, 200_000, 100_000)).toEqual({ aMs: 10_000, bMs: 20_000 });
  });
});

describe("lineage data", () => {
  it("runs oldest first with stages, and undated versions last", async () => {
    const l = await backend.trackLineage(trackNamed("Glasshouse (Demo)").id);
    expect(l.title).toBe("Glasshouse");
    expect(l.nodes.map((n) => [n.track.title, n.stage])).toEqual([
      ["Glasshouse (Demo)", "demo"],
      ["Glasshouse (Leak)", "leak"],
      ["Glasshouse", "release"],
      ["Glasshouse (v2)", "session"],
      ["Glasshouse (Live)", "live"],
      ["Glasshouse (2025 Remaster)", "remaster"],
      ["Glasshouse (Piano Demo)", "demo"],
    ]);
    expect(l.nodes[l.nodes.length - 1].when).toBeNull();
    expect(nodeDate(l.nodes[0])).toBe("Sep 14, 2018");
    expect(nodeDate(l.nodes[l.nodes.length - 1])).toBe("Date unknown");
    expect(lineageSpan(l)).toBe("2018 to 2025");
    // Only released versions can be shared.
    expect(l.nodes.filter((n) => n.shareable).map((n) => n.track.title)).toEqual(["Glasshouse", "Glasshouse (Live)", "Glasshouse (2025 Remaster)"]);
  });
});

describe("ways in", () => {
  it("song menus offer Lineage for songs with other versions, while it's on", async () => {
    const labels = () =>
      trackMenuItems(trackWithVersions())
        .filter((i) => !i.when || i.when())
        .map((i) => i.label);
    expect(labels()).toContain("Lineage");
    const single = mock.fx.tracks.find((t) => t.artistId !== ARCHIVE_ARTIST_ID)!;
    expect(trackMenuItems(single).map((i) => i.label)).not.toContain("Lineage");
    await backend.settingsSet({ showLineage: false });
    await loadSettings();
    expect(labels()).not.toContain("Lineage");
  });

  it("Other versions links to the lineage", async () => {
    const user = userEvent.setup();
    const t = trackNamed("Glasshouse");
    render(<ArchiveDialogs />);
    act(() => openVersions(t));
    const dialog = await screen.findByRole("dialog", { name: "Versions of Glasshouse" });
    await user.click(await within(dialog).findByRole("button", { name: "See lineage" }));
    expect(route()).toEqual({ name: "lineage", id: t.id });
  });

  it("Your stats links to the lineage for a song with versions", async () => {
    const user = userEvent.setup();
    const t = trackNamed("Glasshouse");
    render(<StatsDialogHost />);
    act(() => useStatsDialog.getState().open({ kind: "song", id: t.id, title: t.title, versions: 6 }));
    await user.click(await screen.findByRole("button", { name: "Lineage" }));
    expect(route()).toEqual({ name: "lineage", id: t.id });
    act(() => useStatsDialog.getState().open({ kind: "song", id: t.id, title: t.title, versions: 0 }));
    await screen.findByRole("button", { name: "Done" });
    expect(screen.queryByRole("button", { name: "Lineage" })).toBeNull();
  });
});

function trackWithVersions() {
  return { ...trackNamed("Glasshouse"), otherVersions: 6 };
}

describe("lineage view", () => {
  it("shows every version on a timeline with stage, status, quality and era", async () => {
    render(<LineagePage id={trackNamed("Glasshouse").id} />);
    expect(await screen.findByRole("heading", { level: 1, name: "Glasshouse" })).toBeInTheDocument();
    const list = screen.getByRole("list", { name: "Versions of Glasshouse, oldest first" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows).toHaveLength(7);
    expect(within(rows[0]).getByText("Sep 14, 2018")).toBeInTheDocument();
    expect(within(rows[0]).getByText("Demo")).toBeInTheDocument();
    expect(within(rows[0]).getByText("Unreleased")).toBeInTheDocument();
    expect(within(rows[0]).getByText("CDQ")).toBeInTheDocument();
    expect(within(rows[0]).getByRole("button", { name: "Glass Season era" })).toBeInTheDocument();
    expect(within(rows[1]).getByText("LQ")).toBeInTheDocument();
    expect(rows[2]).toHaveAttribute("aria-current", "true");
    expect(within(rows[2]).getByText("This one")).toBeInTheDocument();
    expect(within(rows[6]).getByText("Date unknown")).toBeInTheDocument();
    expect(screen.getByText("7 versions", { exact: false })).toBeInTheDocument();
  });

  it("every node plays", async () => {
    const user = userEvent.setup();
    const spy = vi.spyOn(backend, "playContext");
    render(<LineagePage id={trackNamed("Glasshouse").id} />);
    await user.click(await screen.findByRole("button", { name: "Play Glasshouse (Leak)" }));
    await waitFor(() => expect(spy).toHaveBeenCalled());
    expect(spy.mock.calls[0][0].startTrackId).toBe(trackNamed("Glasshouse (Leak)").id);
  });

  it("unreleased versions have no Share, released ones do", async () => {
    const user = userEvent.setup();
    render(
      <>
        <LineagePage id={trackNamed("Glasshouse").id} />
        <ContextMenuHost />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: "More options for Glasshouse (Leak)" }));
    await screen.findByRole("menuitem", { name: "Add to queue" });
    expect(screen.queryByRole("menuitem", { name: "Share" })).toBeNull();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "More options for Glasshouse (Live)" }));
    expect(await screen.findByRole("menuitem", { name: "Share" })).toBeInTheDocument();
  });

  it("says so when lineage is off", async () => {
    await backend.settingsSet({ showLineage: false });
    await loadSettings();
    render(<LineagePage id={trackNamed("Glasshouse").id} />);
    expect(await screen.findByRole("heading", { name: "Lineage is off" })).toBeInTheDocument();
  });
});

describe("Compare", () => {
  async function openCompare(a: string, b: string) {
    const user = userEvent.setup();
    const r = render(<LineagePage id={trackNamed("Glasshouse").id} />);
    await user.click(await screen.findByRole("button", { name: `Compare ${a}` }));
    expect(await screen.findByText(`Pick one more version to compare with ${a}.`)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: `Compare ${b}` }));
    return { user, ...r };
  }

  it("prepares both files first, then switches at the same moment with X", async () => {
    const prepare = vi.spyOn(backend, "comparePrepare");
    const { user } = await openCompare("Glasshouse", "Glasshouse (2025 Remaster)");
    const panel = screen.getByTestId("compare");
    expect(within(panel).getByText("Preparing compare…")).toBeInTheDocument();
    expect(within(panel).queryByRole("button", { name: /Switch/ })).toBeNull();
    expect(prepare).toHaveBeenCalledWith(trackNamed("Glasshouse").id, trackNamed("Glasshouse (2025 Remaster)").id);
    const sw = await within(panel).findByRole("button", { name: /Switch/ }, { timeout: 3000 });
    // Lengths within 5%: no approximate label. Lossless both: level matched.
    expect(within(panel).queryByText("Approximate")).toBeNull();
    expect(within(panel).getByText("Level matched")).toBeInTheDocument();

    const switchTo = vi.spyOn(backend, "compareSwitch");
    await user.click(within(panel).getByRole("button", { name: "Play" }));
    await waitFor(() => expect(switchTo).toHaveBeenLastCalledWith("a", true));
    await waitFor(() => expect(within(panel).getByRole("button", { pressed: true, name: /^A/ })).toBeInTheDocument());
    fireEvent.keyDown(window, { key: "x" });
    await waitFor(() => expect(switchTo).toHaveBeenLastCalledWith("b", undefined));
    await waitFor(() => expect(within(panel).getByRole("button", { pressed: true, name: /^B/ })).toBeInTheDocument());
    await user.click(sw);
    await waitFor(() => expect(switchTo).toHaveBeenLastCalledWith("a", undefined));
  });

  it("labels far-apart lengths approximate, and says when levels can't match", async () => {
    await openCompare("Glasshouse (Piano Demo)", "Glasshouse (Leak)");
    const panel = screen.getByTestId("compare");
    expect(await within(panel).findByText("Approximate", undefined, { timeout: 3000 })).toBeInTheDocument();
    expect(within(panel).getByText("Not level matched")).toBeInTheDocument();
  });

  it("level matching follows its setting", async () => {
    await backend.settingsSet({ compareLevelMatch: false });
    await loadSettings();
    await openCompare("Glasshouse", "Glasshouse (2025 Remaster)");
    expect(await within(screen.getByTestId("compare")).findByText("Not level matched", undefined, { timeout: 3000 })).toBeInTheDocument();
  });

  it("a file that can't be fetched shows the reason and Try again", async () => {
    mock.compareFails = true;
    const { user } = await openCompare("Glasshouse", "Glasshouse (Live)");
    const panel = screen.getByTestId("compare");
    expect(await within(panel).findByRole("alert", undefined, { timeout: 3000 })).toHaveTextContent("Couldn't prepare Compare. Couldn't get Glasshouse (Live)");
    mock.compareFails = false;
    await user.click(within(panel).getByRole("button", { name: "Try again" }));
    expect(await within(panel).findByRole("button", { name: /Switch/ }, { timeout: 3000 })).toBeInTheDocument();
  });

  it("sets a loop region and carries it across a switch", async () => {
    const off = connectPlayer();
    const { user } = await openCompare("Glasshouse", "Glasshouse (2025 Remaster)");
    const panel = screen.getByTestId("compare");
    await within(panel).findByRole("button", { name: /Switch/ }, { timeout: 3000 });
    await user.click(within(panel).getByRole("button", { name: "Play" }));
    await waitFor(() => expect(within(panel).getByRole("button", { name: "Loop from here" })).toBeEnabled());
    await user.click(within(panel).getByRole("button", { name: "Loop from here" }));
    const loop = vi.spyOn(backend, "compareLoop");
    // Move on a few seconds, then end the loop there.
    await act(() => backend.playSeek(30_000));
    await waitFor(() => expect(within(panel).getByRole("button", { name: "End loop here" })).toBeEnabled());
    await user.click(within(panel).getByRole("button", { name: "End loop here" }));
    await waitFor(() => expect(loop).toHaveBeenCalled());
    const set = loop.mock.calls[0][0]!;
    expect(set.aMs).toBeLessThan(2_000);
    expect(set.bMs).toBeGreaterThanOrEqual(30_000);
    expect(await within(panel).findByRole("button", { name: "Clear loop" })).toBeInTheDocument();
    // Lengths within 5%: the loop keeps its times in the other version.
    const now = await backend.compareSwitch("b");
    expect(now.abLoop).toEqual(set);
    off();
  });

  it("puts the listening back as it was, and the compared plays never count", async () => {
    const album = mock.fx.albums.find((a) => a.artistId !== ARCHIVE_ARTIST_ID)!;
    await backend.playContext({ context: { kind: "album", id: album.id } });
    await backend.setRepeat("all");
    await backend.playSeek(20_000);
    const before = await backend.playerState();
    const recents = JSON.stringify(await backend.recents());
    await backend.comparePrepare(trackNamed("Glasshouse").id, trackNamed("Glasshouse (Live)").id);
    await backend.compareSwitch("a", true);
    await backend.compareSwitch("b");
    expect((await backend.playerState()).track?.id).toBe(trackNamed("Glasshouse (Live)").id);
    await backend.compareEnd();
    const after = await backend.playerState();
    expect(after.track?.id).toBe(before.track?.id);
    expect(after.queue).toEqual(before.queue);
    expect([after.status, after.repeat, after.shuffle]).toEqual([before.status, before.repeat, before.shuffle]);
    expect(after.positionMs).toBeGreaterThanOrEqual(before.positionMs);
    expect(JSON.stringify(await backend.recents())).toBe(recents);
  });

  it("a failed prepare leaves the listening alone", async () => {
    const album = mock.fx.albums.find((a) => a.artistId !== ARCHIVE_ARTIST_ID)!;
    await backend.playContext({ context: { kind: "album", id: album.id } });
    const before = await backend.playerState();
    mock.compareFails = true;
    await expect(backend.comparePrepare(trackNamed("Glasshouse").id, trackNamed("Glasshouse (Live)").id)).rejects.toMatchObject({ kind: "network" });
    const after = await backend.playerState();
    expect([after.track?.id, after.queue]).toEqual([before.track?.id, before.queue]);
  });

  it("closing Compare gives the sound settings back", async () => {
    const end = vi.spyOn(backend, "compareEnd");
    const { user } = await openCompare("Glasshouse", "Glasshouse (Live)");
    await user.click(screen.getByRole("button", { name: "Close Compare" }));
    expect(screen.queryByTestId("compare")).toBeNull();
    await waitFor(() => expect(end).toHaveBeenCalled());
  });
});

describe("settings", () => {
  it("Show lineage and Compare level matching are on by default and switch", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    for (const name of ["Show lineage", "Compare level matching"]) {
      const sw = await screen.findByRole("switch", { name });
      expect(sw).toBeChecked();
      await user.click(sw);
    }
    await waitFor(async () => {
      const s = await backend.settingsGet();
      expect([s.showLineage, s.compareLevelMatch]).toEqual([false, false]);
    });
  });
});
