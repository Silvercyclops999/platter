import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { useRouter } from "../stores/router";
import { useUi } from "../stores/ui";

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
    const row = screen.getByRole("listitem", { name: new RegExp(t1.title) });
    expect(row).toHaveTextContent("1");
    expect(row).toHaveTextContent(t1.title);
    expect(row).toHaveTextContent(t1.albumTitle);
    expect(row).toHaveTextContent(t1.codec!.toUpperCase());
    expect(row).toHaveTextContent(/\d:\d\d/);
    expect(screen.getAllByRole("button", { name: `Play ${t1.title}` })[0]).toBeInTheDocument();
  });

  it("plays on Enter and double-click", async () => {
    const user = userEvent.setup();
    const onPlay = setup();
    const row = screen.getByRole("listitem", { name: new RegExp(t1.title) });
    row.focus();
    await user.keyboard("{Enter}");
    expect(onPlay).toHaveBeenCalledTimes(1);
    await user.dblClick(row);
    expect(onPlay).toHaveBeenCalledTimes(2);
  });

  it("ArrowDown moves focus to the next row", async () => {
    const user = userEvent.setup();
    setup();
    const rows = screen.getAllByRole("listitem");
    rows[0].focus();
    await user.keyboard("{ArrowDown}");
    expect(rows[1]).toHaveFocus();
    await user.keyboard("{ArrowUp}");
    expect(rows[0]).toHaveFocus();
  });

  it("right-click opens a context menu navigable by keyboard; Escape restores focus", async () => {
    const user = userEvent.setup();
    setup();
    const row = screen.getByRole("listitem", { name: new RegExp(t1.title) });
    row.focus();
    fireEvent.contextMenu(row, { clientX: 100, clientY: 100 });
    const menu = await screen.findByRole("menu", { name: `Options for ${t1.title}` });
    const items = screen.getAllByRole("menuitem");
    expect(items.map((i) => i.textContent)).toEqual(["Add to playlist", "Add to queue", `Go to ${t1.artistTitle}`, "Go to album", "Copy song title"]);
    expect(items[0]).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(items[1]).toHaveFocus();
    await user.keyboard("{End}");
    expect(items[4]).toHaveFocus();
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
    const row = screen.getByRole("listitem", { name: new RegExp(t1.title) });
    row.focus();
    await user.keyboard("{Shift>}{F10}{/Shift}");
    await screen.findByRole("menu");
    await user.keyboard("{ArrowDown}{Enter}");
    expect(spy).toHaveBeenCalledWith([t1.id], "end");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("'Go to album' navigates", async () => {
    const user = userEvent.setup();
    useRouter.getState().reset();
    setup();
    fireEvent.contextMenu(screen.getByRole("listitem", { name: new RegExp(t1.title) }), { clientX: 10, clientY: 10 });
    await user.click(await screen.findByRole("menuitem", { name: "Go to album" }));
    const s = useRouter.getState();
    expect(s.entries[s.index].route).toEqual({ name: "album", id: t1.albumId });
  });
});
