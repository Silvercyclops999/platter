import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { SearchResults, Track } from "../lib/types";
import { CommandPalette } from "../shell/CommandPalette";
import { player } from "../stores/player";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const base = mock.fx.tracks[0];
const song = (id: string, title: string): Track => ({ ...base, id, title });
const answer = (songs: Track[]): SearchResults => ({ songs, artists: [], albums: [], playlists: [], tookMs: 1, source: "local" });

/** Search answers held until the test lets each one through. */
function heldSearch() {
  const waiting = new Map<string, (r: SearchResults) => void>();
  vi.spyOn(mock, "search").mockImplementation(({ query }) => new Promise((r) => waiting.set(query, r)));
  return {
    answer: async (query: string, r: SearchResults) => {
      await waitFor(() => expect(waiting.has(query)).toBe(true));
      await act(async () => waiting.get(query)!(r));
    },
  };
}

function openPalette() {
  render(<CommandPalette />);
  act(() => useUi.getState().setPaletteOpen(true));
}

const options = () => screen.getAllByRole("option");
const activeOption = () => options().find((o) => o.getAttribute("aria-selected") === "true");

afterEach(() => {
  act(() => useUi.getState().setPaletteOpen(false));
  vi.restoreAllMocks();
});

describe("command palette", () => {
  it("never shows a slow answer for an older query under a newer one", async () => {
    const user = userEvent.setup();
    const search = heldSearch();
    const play = vi.spyOn(player, "playTracks").mockResolvedValue(undefined as never);
    openPalette();
    const box = await screen.findByRole("combobox");
    await user.type(box, "ab");
    await search.answer("ab", answer([song("old", "Old song")]));
    expect(screen.getByRole("option", { name: /Old song/ })).toBeInTheDocument();
    await user.type(box, "c");
    // The old answer drops away at once; Enter doesn't play it.
    expect(screen.queryByRole("option", { name: /Old song/ })).not.toBeInTheDocument();
    await search.answer("abc", answer([song("new", "New song")]));
    expect(activeOption()).toHaveTextContent("New song");
    await user.keyboard("{Enter}");
    await waitFor(() => expect(play).toHaveBeenCalledWith(["new"], "new", "search"));
  });

  it("goes back to the first row when new results come in, even with as many rows", async () => {
    const user = userEvent.setup();
    const search = heldSearch();
    openPalette();
    const box = await screen.findByRole("combobox");
    await user.type(box, "zz");
    await search.answer("zz", answer([song("a", "Zz one"), song("b", "Zz two")]));
    await user.keyboard("{ArrowDown}");
    expect(activeOption()).toHaveTextContent("Zz two");
    await user.type(box, "z");
    await search.answer("zzz", answer([song("c", "Zzz one"), song("d", "Zzz two")]));
    expect(activeOption()).toHaveTextContent("Zzz one");
  });

  it("scrolls the highlighted row into view", async () => {
    const user = userEvent.setup();
    const seen: string[] = [];
    const had = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (this: Element) {
      seen.push(this.textContent ?? "");
    };
    try {
      openPalette();
      await screen.findByRole("combobox");
      for (let i = 0; i < 5; i++) await user.keyboard("{ArrowDown}");
      expect(activeOption()).toBe(options()[5]);
      expect(seen[seen.length - 1]).toBe(options()[5].textContent);
    } finally {
      Element.prototype.scrollIntoView = had;
    }
  });
});
