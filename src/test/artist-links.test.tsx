import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { TrackList, TrackRow, trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { Track } from "../lib/types";
import { PlayerBar } from "../shell/PlayerBar";
import { useApp } from "../stores/app";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { DEFAULT_SETTINGS } from "../lib/types";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const [t1] = mock.fx.tracks;
const amp = mock.fx.artists.find((a) => a.title.includes("&"))!;
// Album artist, a known name with "&" in it, and a name the library doesn't have.
const featured: Track = { ...t1, id: "feat-1", trackArtist: `${t1.artistTitle} feat. ${amp.title} & Nobody Known` };

const route = () => {
  const { entries, index } = useRouter.getState();
  return entries[index].route;
};

beforeEach(async () => {
  const names = await mock.artistNames();
  act(() => useApp.getState().setArtistNames(names));
});
afterEach(() => {
  act(() => useUi.setState({ menu: null }));
  act(() => useApp.setState({ artistLookup: new Map() }));
  act(() => useSettings.setState({ settings: null }));
  useRouter.getState().reset();
});

function renderRow(onPlay = vi.fn()) {
  render(
    <>
      <TrackList label="Tracks">
        <TrackRow track={featured} index={1} onPlay={onPlay} />
      </TrackList>
      <ContextMenuHost />
    </>,
  );
  return { onPlay, row: screen.getByRole("listitem", { name: new RegExp(featured.title) }) };
}

describe("artist links in a track row", () => {
  it("links every credited artist on its own, featured names without a library artist too", async () => {
    const user = userEvent.setup();
    const { row, onPlay } = renderRow();
    const links = within(row).getAllByRole("link");
    expect(links.map((l) => l.textContent)).toEqual([t1.artistTitle, amp.title, "Nobody Known", t1.albumTitle]);
    expect(row).toHaveTextContent(`${t1.artistTitle}, ${amp.title}, Nobody Known`);
    await user.click(within(row).getByRole("link", { name: "Nobody Known" }));
    expect(route()).toEqual({ name: "artist", id: "credit:nobody known" });
    expect(onPlay).not.toHaveBeenCalled();
  });

  it("leaves names without a library artist as text when featured links are off", () => {
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, linkFeatured: false } }));
    const { row } = renderRow();
    expect(within(row).getAllByRole("link").map((l) => l.textContent)).toEqual([t1.artistTitle, amp.title, t1.albumTitle]);
    expect(within(row).queryByRole("link", { name: "Nobody Known" })).toBeNull();
    expect(trackMenuItems(featured).map((i) => i.label)).not.toContain("Go to Nobody Known");
  });

  it("goes to that artist without playing or selecting the row", async () => {
    const user = userEvent.setup();
    const { row, onPlay } = renderRow();
    const link = within(row).getByRole("link", { name: amp.title });
    await user.click(link);
    expect(route()).toEqual({ name: "artist", id: amp.id });
    expect(row).not.toHaveAttribute("data-selected");
    await user.dblClick(link);
    expect(onPlay).not.toHaveBeenCalled();
  });

  it("is keyboard focusable and opens on Enter", async () => {
    const user = userEvent.setup();
    const { row, onPlay } = renderRow();
    const link = within(row).getByRole("link", { name: t1.artistTitle });
    expect(link).toHaveAttribute("tabindex", "0");
    link.focus();
    await user.keyboard("{Enter}");
    expect(route()).toEqual({ name: "artist", id: t1.artistId });
    expect(onPlay).not.toHaveBeenCalled();
  });

  it("the whole line truncates as one", () => {
    const { row } = renderRow();
    const line = within(row).getByRole("link", { name: amp.title }).parentElement!;
    expect(line).toHaveClass("truncate");
    expect(line.tagName).toBe("SPAN");
  });
});

describe("track menu", () => {
  it("has one 'Go to' item per linked artist, in line order, where 'Go to artist' was", () => {
    const labels = trackMenuItems(featured).map((i) => i.label);
    expect(labels).toEqual(["Add to playlist", "Add to Liked Songs", "Add to queue", `Go to ${t1.artistTitle}`, `Go to ${amp.title}`, "Go to Nobody Known", "Go to album", "Share", "Copy song title"]);
  });

  it("each item goes to its artist", () => {
    const item = trackMenuItems(featured).find((i) => i.label === `Go to ${amp.title}`)!;
    item.onSelect?.();
    expect(route()).toEqual({ name: "artist", id: amp.id });
  });

  it("copies the original artist line", async () => {
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    trackMenuItems(featured).find((i) => i.label === "Copy song title")!.onSelect?.();
    expect(write).toHaveBeenCalledWith(`${featured.title} - ${featured.trackArtist}`);
  });
});

describe("artist links in the player bar", () => {
  it("links each artist and keeps the title's menu", async () => {
    const base = await mock.playerState();
    act(() => usePlayer.getState().set({ ...base, track: featured }));
    render(
      <>
        <PlayerBar />
        <ContextMenuHost />
      </>,
    );
    const bar = screen.getByRole("contentinfo", { name: "Player" });
    expect(within(bar).getByRole("link", { name: t1.artistTitle })).toBeInTheDocument();
    fireEvent.click(within(bar).getByRole("link", { name: amp.title }));
    expect(route()).toEqual({ name: "artist", id: amp.id });
    expect(within(bar).getByRole("link", { name: "Nobody Known" })).toBeInTheDocument();
    fireEvent.contextMenu(within(bar).getByRole("link", { name: featured.title }), { clientX: 20, clientY: 700 });
    const menu = await screen.findByRole("menu", { name: `Options for ${featured.title}` });
    expect(within(menu).getByRole("menuitem", { name: `Go to ${amp.title}` })).toBeInTheDocument();
  });
});
