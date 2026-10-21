/**
 * Album, artist and playlist pages for an id that isn't in the library: a
 * sentence and a way back, never the raw id.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { LoadError } from "../components/LoadError";
import { backend } from "../lib/backend";
import { DEFAULT_SETTINGS } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { ArtistPage } from "../pages/ArtistPage";
import { PlaylistPage } from "../pages/PlaylistPage";
import { LikedPage } from "../pages/LikedPage";
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
    expect(within(status).getByRole("heading", { name: `This ${what} isn't in your library` })).toBeInTheDocument();
    expect(status).toHaveTextContent("It may have been removed from your server");
    expect(status).not.toHaveTextContent("30251");
    expect(status).not.toHaveTextContent("Couldn't load");
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(route().name).toBe("search");
  });

  it("offers Home when there's nowhere to go back to", async () => {
    render(<AlbumPage id="ffffffff:1" />);
    expect(await screen.findByRole("status")).toHaveTextContent("This album isn't in your library");
    await userEvent.click(screen.getByRole("button", { name: "Go home" }));
    expect(route()).toEqual({ name: "home" });
  });
});

describe("a page that couldn't load", () => {
  it("says what failed and why, and Try again loads it", async () => {
    const user = userEvent.setup();
    const real = backend.likedPage.bind(backend);
    const spy = vi.spyOn(backend, "likedPage").mockRejectedValueOnce({ kind: "network", message: "The server didn't answer" });
    render(<LikedPage />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't load Liked Songs");
    expect(alert).toHaveTextContent("The server didn't answer");
    expect(within(alert).getByRole("button", { name: "Go home" })).toBeInTheDocument();
    spy.mockImplementation(real);
    await user.click(within(alert).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(spy).toHaveBeenCalledTimes(2);
    spy.mockRestore();
  });

  it("an album that fails for another reason gets Try again and Back", async () => {
    useRouter.getState().navigate({ name: "search", query: "demo" });
    useRouter.getState().navigate({ name: "album", id: "a1" });
    vi.spyOn(backend, "albumPage").mockRejectedValueOnce({ kind: "network", message: "Timed out" });
    render(<AlbumPage id="a1" />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't load this album");
    expect(within(alert).getByRole("button", { name: "Try again" })).toBeInTheDocument();
    await userEvent.click(within(alert).getByRole("button", { name: "Back" }));
    expect(route().name).toBe("search");
    vi.restoreAllMocks();
  });

  it("cuts a very long reason short", () => {
    render(<LoadError what="Home" error={{ message: "x ".repeat(200) }} />);
    expect(screen.getByRole("alert").querySelector(".load-error__why")!.textContent!.length).toBeLessThanOrEqual(160);
  });
});
