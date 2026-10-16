/**
 * Album, artist and playlist pages for an id that isn't in the library: a
 * sentence and a way back, never the raw id.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import { DEFAULT_SETTINGS } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { ArtistPage } from "../pages/ArtistPage";
import { PlaylistPage } from "../pages/PlaylistPage";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";

const route = () => useRouter.getState().entries[useRouter.getState().index].route;

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
  useRouter.getState().reset({ name: "home" });
});

describe("a page for something not in the library", () => {
  it.each([
    ["album", (id: string) => <AlbumPage id={id} />],
    ["artist", (id: string) => <ArtistPage id={id} />],
    ["playlist", (id: string) => <PlaylistPage id={id} />],
  ] as const)("says the %s isn't in your library and goes back", async (what, page) => {
    useRouter.getState().navigate({ name: "search", query: "demo" });
    useRouter.getState().navigate({ name: what, id: "30251" } as never);
    render(page("30251"));
    const status = await screen.findByRole("status");
    expect(status).toHaveTextContent(`This ${what} isn't in your library`);
    expect(status).not.toHaveTextContent("30251");
    expect(status).not.toHaveTextContent("Couldn't load");
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(route().name).toBe("search");
  });

  it("offers Home when there's nowhere to go back to", async () => {
    render(<AlbumPage id="ffffffff:1" />);
    expect(await screen.findByRole("status")).toHaveTextContent("This album isn't in your library");
    await userEvent.click(screen.getByRole("button", { name: "Go Home" }));
    expect(route()).toEqual({ name: "home" });
  });
});
