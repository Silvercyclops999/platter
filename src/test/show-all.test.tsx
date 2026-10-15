import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { ArtistPage } from "../pages/ArtistPage";
import { ArtistShelfPage } from "../pages/HomeShelfPage";
import { useRouter } from "../stores/router";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

function withShelves() {
  const real = mock.artistPage.bind(mock);
  const others = mock.fx.albums.filter((a) => a.artistId !== mock.fx.artists[0].id).slice(0, 3);
  return vi.spyOn(mock, "artistPage").mockImplementation(async (id) => ({
    ...(await real(id)),
    similar: mock.fx.artists.slice(1, 6),
    appearsOn: others,
  }));
}

afterEach(() => vi.restoreAllMocks());

describe("Show all pages", () => {
  it("artist shelves open their own grid page", async () => {
    withShelves();
    useRouter.getState().reset();
    const user = userEvent.setup();
    const a = mock.fx.artists[0];
    render(<ArtistPage id={a.id} />);
    const fans = await screen.findByRole("heading", { name: "Fans also like" });
    await user.click(within(fans.closest("section")!).getByRole("button", { name: "Show all" }));
    expect(route()).toEqual({ name: "artistShelf", id: a.id, shelf: "fansAlsoLike" });
    const appears = screen.getByRole("heading", { name: "Appears on" });
    await user.click(within(appears.closest("section")!).getByRole("button", { name: "Show all" }));
    expect(route()).toEqual({ name: "artistShelf", id: a.id, shelf: "appearsOn" });
  });

  it("is a plain grid: the title, the same cards, nothing else", async () => {
    withShelves();
    // Only the shelf's own cards: MusicBrainz's extra fans would land at a moment of their own.
    vi.spyOn(mock, "catalogueFansAlsoLike").mockResolvedValue([]);
    render(<ArtistShelfPage id={mock.fx.artists[0].id} shelf="fansAlsoLike" />);
    const grid = await screen.findByRole("list", { name: "Fans also like" });
    await vi.waitFor(() => expect(within(grid).getAllByRole("listitem")).toHaveLength(5));
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Fans also like");
    // Artist cards keep their round art.
    expect(grid.querySelectorAll(".card--artist")).toHaveLength(5);
    // No header play button, sort or filter.
    expect(screen.queryByRole("group")).toBeNull();
    expect(document.querySelector(".action-row, .detail-header")).toBeNull();
  });

  it("albums on the Appears on page keep their artist line", async () => {
    withShelves();
    render(<ArtistShelfPage id={mock.fx.artists[0].id} shelf="appearsOn" />);
    const grid = await screen.findByRole("list", { name: "Appears on" });
    await vi.waitFor(() => expect(within(grid).getAllByRole("listitem")).toHaveLength(3));
    expect(grid.querySelectorAll(".card--album")).toHaveLength(3);
  });
});
