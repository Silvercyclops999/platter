import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { useRouter } from "../stores/router";
import { useUi } from "../stores/ui";
import { body } from "./rows";
import { listenAlong } from "../stores/social";

// Sessions are off in the app until the relay transport lands; keep covering them.
listenAlong.enabled = true;

const mock = backend as MockBackend;
const [t1, t2] = mock.fx.tracks;

function setup(onPlay = vi.fn()) {
  render(
    <>
      <TrackList label="Tracks">
        <TrackRow track={t1} index={1} onPlay={onPlay} />
        <TrackRow track={t2} index={2} onPlay={vi.fn()} />
      </TrackList>
      <ContextMenuHost />
    </>,
  );
  return onPlay;
}

describe("TrackRow", () => {
  afterEach(() => act(() => useUi.setState({ menu: null })));

  it("shows number, title, artist, album, quality badge and duration", () => {
    setup();
    const row = screen.getByRole("row", { name: new RegExp(t1.title) });
    expect(row).toHaveTextContent("1");
    expect(row).toHaveTextContent(t1.title);
    expect(row).toHaveTextContent(t1.albumTitle);
    expect(row).toHaveTextContent(t1.codec!.toUpperCase());
    expect(row).toHaveTextContent(/\d:\d\d/);
    expect(screen.getAllByRole("button", { name: new RegExp(`^Play ${t1.title} by `) })[0]).toBeInTheDocument();
  });

  it("plays on Enter and double-click", async () => {
    const user = userEvent.setup();
    const onPlay = setup();
    const row = screen.getByRole("row", { name: new RegExp(t1.title) });
    row.focus();
    await user.keyboard("{Enter}");
    expect(onPlay).toHaveBeenCalledTimes(1);
    await user.dblClick(row);
    expect(onPlay).toHaveBeenCalledTimes(2);
  });

  it("ArrowDown moves focus to the next row", async () => {
    const user = userEvent.setup();
    setup();
    const rows = body(screen.getAllByRole("row"));
    rows[0].focus();
    await user.keyboard("{ArrowDown}");
    expect(rows[1]).toHaveFocus();
    await user.keyboard("{ArrowUp}");
    expect(rows[0]).toHaveFocus();
  });

  it("right-click opens a context menu navigable by keyboard; Escape restores focus", async () => {
    const user = userEvent.setup();
    setup();
    const row = screen.getByRole("row", { name: new RegExp(t1.title) });
    row.focus();
    fireEvent.contextMenu(row, { clientX: 100, clientY: 100 });
    const menu = await screen.findByRole("menu", { name: `Options for ${t1.title}` });
    const items = screen.getAllByRole("menuitem");
    expect(items.map((i) => i.textContent)).toEqual(["Add to playlist", "Add to Liked Songs", "Add to queue", "Start a listening session", `Go to ${t1.artistTitle}`, "Go to album", "Go to song radio", "Bookmark", "Your stats", "Don't play this artist", "Add lyrics", "Share", "Copy song title", "Edit tags…"]);
    expect(items[0]).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(items[1]).toHaveFocus();
    await user.keyboard("{End}");
    expect(items[items.length - 1]).toHaveFocus();
    await user.keyboard("{ArrowDown}"); // wraps
    expect(items[0]).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(menu).not.toBeInTheDocument();
    expect(row).toHaveFocus();
  });

  it("Shift+F10 opens the menu; choosing an item runs it", async () => {
    const user = userEvent.setup();
    const spy = vi.spyOn(backend, "queueAdd");
    setup();
    const row = screen.getByRole("row", { name: new RegExp(t1.title) });
    row.focus();
    await user.keyboard("{Shift>}{F10}{/Shift}");
    await screen.findByRole("menu");
    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");
    expect(spy).toHaveBeenCalledWith([t1.id], "end");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("'Go to album' navigates", async () => {
    const user = userEvent.setup();
    useRouter.getState().reset();
    setup();
    fireEvent.contextMenu(screen.getByRole("row", { name: new RegExp(t1.title) }), { clientX: 10, clientY: 10 });
    await user.click(await screen.findByRole("menuitem", { name: "Go to album" }));
    const s = useRouter.getState();
    expect(s.entries[s.index].route).toEqual({ name: "album", id: t1.albumId });
  });
});
