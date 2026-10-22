/**
 * Save a playlist as a file: the playlist page's menu hands the songs, in
 * order, to the core's save dialog; a saved file is named, a cancel says
 * nothing, and a failed write says why.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { PlaylistPage, sortRows } from "../pages/PlaylistPage";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const toastText = () => useToasts.getState().toasts.map((t) => t.message);

beforeEach(() => {
  useRouter.getState().reset();
  useToasts.setState({ toasts: [] });
  useUi.setState({ menu: null });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  mock.playlistFiles = [];
  mock.exportCancels = false;
});
afterEach(() => {
  vi.restoreAllMocks();
  mock.exportCancels = false;
});

async function openMenu(title: string, sortBy?: string) {
  const user = userEvent.setup();
  const ids = mock.fx.tracks.slice(0, 3).map((t) => t.id);
  // Out of title order, so sorting moves them.
  if (sortBy) ids.reverse();
  const pl = await backend.playlistCreate(title, ids);
  render(
    <>
      <PlaylistPage id={pl.id} />
      <ContextMenuHost />
      <Toaster />
    </>,
  );
  if (sortBy) {
    await user.click(await screen.findByRole("button", { name: "Sort songs: Custom order" }));
    await user.click(within(await screen.findByRole("menu")).getByRole("menuitemradio", { name: sortBy }));
  }
  await user.click(await screen.findByRole("button", { name: `More options for ${title}` }));
  const menu = await screen.findByRole("menu");
  return { user, ids, item: within(menu).getByRole("menuitem", { name: "Save as a file" }) };
}

describe("save a playlist as a file", () => {
  it("hands the songs over in order and names the saved file", async () => {
    const { user, ids, item } = await openMenu("Road trip");
    await user.click(item);
    await waitFor(() => expect(toastText()).toContain("Saved Road trip.m3u8"));
    expect(mock.playlistFiles).toEqual([{ title: "Road trip", trackIds: ids }]);
  });

  it("saves in the order shown when the page is sorted", async () => {
    const { user, ids, item } = await openMenu("Sorted", "Title");
    await user.click(item);
    await waitFor(() => expect(mock.playlistFiles).toHaveLength(1));
    const rows = ids.map((id, i) => ({ track: mock.fx.tracks.find((t) => t.id === id)!, itemId: String(i) }));
    const sorted = sortRows(rows, "title").map((r) => r.track.id);
    expect(sorted).not.toEqual(ids);
    expect(mock.playlistFiles[0].trackIds).toEqual(sorted);
  });

  it("a cancelled save dialog says nothing", async () => {
    mock.exportCancels = true;
    const { user, item } = await openMenu("Kept here");
    await user.click(item);
    await new Promise((r) => setTimeout(r, 300));
    expect(toastText()).toEqual([]);
    expect(mock.playlistFiles).toEqual([]);
  });

  it("app state: a failed write says why", async () => {
    vi.spyOn(backend, "playlistSaveFile").mockRejectedValueOnce({ kind: "io", message: "Couldn't save the playlist file: the disk is full." });
    const { user, item } = await openMenu("Too big");
    await user.click(item);
    await waitFor(() => expect(toastText()).toContain("Couldn't save the playlist file: the disk is full."));
  });
});
