/**
 * Music that isn't in the library: search's "Not in your library" section
 * (after typing pauses, only when the library found little or it's opened,
 * hidden offline or with the setting off), catalogue artist and album pages
 * (dimmed, Request through Lidarr, Add to Missing, store links), Fans also
 * like with artists outside the library, New to you and "Not interested",
 * and the four settings. MusicBrainz, ListenBrainz and Lidarr are the
 * browser mock's fakes.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MockCatalogue } from "../lib/mock/catalogue";
import { DEFAULT_SETTINGS, type Health } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { ArtistPage } from "../pages/ArtistPage";
import { DismissedArtists } from "../pages/DiscoverSettings";
import { ImportPage } from "../pages/ImportPage";
import { NewToYouPage, NewToYouShelf } from "../pages/NewToYou";
import { SearchPage } from "../pages/SearchPage";
import { SettingsPage } from "../pages/SettingsPage";
import { useApp } from "../stores/app";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;
const COLD = MockCatalogue.artistId(1);
const LOW_TIDE = MockCatalogue.albumId(11);
const SLOW = { timeout: 4000 };

async function setUp(patch: Record<string, unknown> = {}, lidarr: "ready" | "demo" | null = null) {
  if (lidarr) mock.lidarr.reset(lidarr);
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: lidarr ? "http://10.0.0.5:8686" : "", ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  mock.catalogue.searches.length = 0;
  await setUp();
});

describe("search", () => {
  it("shows music that isn't in the library once typing pauses, when the library found nothing", async () => {
    render(<SearchPage query="cold coast" />);
    const section = await screen.findByRole("region", { name: "Not in your library" }, SLOW);
    const list = await within(section).findByRole("list", { name: "Not in your library" }, SLOW);
    expect(within(list).getByRole("button", { name: "Cold Coast" })).toBeInTheDocument();
    expect(within(list).getByRole("button", { name: "Low Tide Radio" })).toBeInTheDocument();
    expect(mock.catalogue.searches).toEqual(["cold coast"]);
    await userEvent.click(within(list).getByRole("button", { name: "Cold Coast" }));
    expect(route()).toEqual({ name: "catalogueArtist", mbid: COLD });
  });

  it("waits to be opened when the library already found plenty", async () => {
    render(<SearchPage query="tide" />);
    const open = await screen.findByRole("button", { name: "Show music that isn't in your library" }, SLOW);
    await new Promise((r) => setTimeout(r, 900));
    expect(mock.catalogue.searches).toEqual([]);
    await userEvent.click(open);
    await screen.findByRole("heading", { name: "Not in your library" }, SLOW);
    await waitFor(() => expect(mock.catalogue.searches).toEqual(["tide"]), SLOW);
  });

  it("asks nothing with the setting off or while offline", async () => {
    await setUp({ catalogueSearch: false });
    const { unmount } = render(<SearchPage query="cold coast" />);
    await screen.findByRole("heading", { name: /No results for/ }, SLOW);
    await new Promise((r) => setTimeout(r, 900));
    expect(screen.queryByRole("region", { name: "Not in your library" })).toBeNull();
    expect(mock.catalogue.searches).toEqual([]);
    unmount();

    await setUp();
    act(() => useApp.setState({ health: { ...(useApp.getState().health as Health), status: "offline", cachedOnly: true } }));
    render(<SearchPage query="cold coast" />);
    await new Promise((r) => setTimeout(r, 900));
    expect(screen.queryByRole("region", { name: "Not in your library" })).toBeNull();
    expect(mock.catalogue.searches).toEqual([]);
    act(() => useApp.setState({ health: { ...(useApp.getState().health as Health), status: "local", cachedOnly: false } }));
  });
});

describe("catalogue artist page", () => {
  it("lists official releases by type, dimmed, with the bio's source", async () => {
    render(<ArtistPage id={COLD} catalogue />);
    expect(await screen.findByRole("heading", { level: 1, name: "Cold Coast" })).toBeInTheDocument();
    expect(screen.getAllByText("Not in your library").length).toBeGreaterThan(0);
    for (const t of ["Albums", "EPs", "Singles", "Live albums and compilations"]) {
      expect(screen.getByRole("heading", { name: t })).toBeInTheDocument();
    }
    const card = screen.getByLabelText("Low Tide Radio by Cold Coast, not in your library");
    expect(card.className).toContain("missing-release");
    // No Lidarr: no Request, a note says how.
    expect(within(card).queryByRole("button", { name: /Request/ })).toBeNull();
    expect(screen.getByText(/Set up Lidarr in Settings/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "From Wikipedia" })).toBeInTheDocument();
    // Similar artists: the library one opens its own page.
    const fans = screen.getByRole("heading", { name: "Fans also like" }).closest("section")!;
    await userEvent.click(within(fans).getByRole("button", { name: mock.fx.artists[2].title }));
    expect(route()).toEqual({ name: "artist", id: mock.fx.artists[2].id });
  });

  it("Not interested is remembered in the synced setting, with undo", async () => {
    render(<ArtistPage id={COLD} catalogue />);
    await userEvent.click(await screen.findByRole("button", { name: "Not interested" }));
    await waitFor(() => expect(useSettings.getState().settings?.dismissedArtists).toEqual([{ mbid: COLD, name: "Cold Coast" }]));
    expect((await mock.settingsGet()).dismissedArtists).toEqual([{ mbid: COLD, name: "Cold Coast" }]);
    const t = useToasts.getState().toasts[useToasts.getState().toasts.length - 1];
    expect(t.action?.label).toBe("Undo");
    act(() => t.action!.run());
    await waitFor(() => expect(useSettings.getState().settings?.dismissedArtists).toEqual([]));
  });

  it("offers Request through Lidarr's preview, naming the artist by MusicBrainz id", async () => {
    await setUp({}, "ready");
    const preview = vi.spyOn(mock, "lidarrRequestPreview");
    render(<ArtistPage id={COLD} catalogue />);
    const card = await screen.findByLabelText("Low Tide Radio by Cold Coast, not in your library");
    await userEvent.click(await within(card).findByRole("button", { name: "Request Low Tide Radio" }, SLOW));
    const dialog = await screen.findByRole("dialog", {}, SLOW);
    expect(within(dialog).getByText(/Low Tide Radio/)).toBeInTheDocument();
    expect(preview).toHaveBeenCalledWith(expect.objectContaining({ releaseGroupId: LOW_TIDE, artist: "Cold Coast", artistMbid: COLD }));
    preview.mockRestore();
  });
});

describe("catalogue album page", () => {
  it("shows the tracklist dimmed with store links and Add to Missing", async () => {
    const open = vi.spyOn(mock, "importOpenLink");
    render(<AlbumPage id={LOW_TIDE} catalogue />);
    expect(await screen.findByRole("heading", { level: 1, name: "Low Tide Radio" })).toBeInTheDocument();
    const tracks = screen.getByRole("list", { name: "Low Tide Radio tracks, not in your library" });
    expect(within(tracks).getAllByRole("listitem")).toHaveLength(10);
    expect(screen.queryByRole("button", { name: /^Play/ })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Search Bandcamp for Low Tide Radio" }));
    expect(open).toHaveBeenCalledWith(expect.stringMatching(/^https:\/\/bandcamp\.com\/search\?q=Cold%20Coast%20Low%20Tide%20Radio&item_type=a$/));
    await userEvent.click(screen.getByRole("button", { name: "Add to Missing" }));
    expect(await screen.findByRole("button", { name: "On your Missing list" }, SLOW)).toHaveAttribute("aria-pressed", "true");
    expect((await mock.importMissingAlbums()).map((a) => a.title)).toContain("Low Tide Radio");
    open.mockRestore();
    await mock.importMissingAlbumRemove(LOW_TIDE);
  });

  it("the Missing list keeps albums added by hand", async () => {
    await mock.importMissingAlbumAdd({ releaseGroupId: LOW_TIDE, title: "Low Tide Radio", artist: "Cold Coast", artistMbid: COLD });
    render(<ImportPage view="missing" />);
    const section = await screen.findByRole("region", { name: "Albums you added" }, SLOW);
    expect(within(section).getByRole("button", { name: "Low Tide Radio" })).toBeInTheDocument();
    expect(within(section).getByRole("button", { name: "Search Qobuz for Low Tide Radio" })).toBeInTheDocument();
    await userEvent.click(within(section).getByRole("button", { name: "Remove Low Tide Radio from Missing" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Albums you added" })).toBeNull());
  });
});

describe("Fans also like and New to you", () => {
  it("adds artists outside the library to Fans also like, dimmed, unless turned off", async () => {
    const a = mock.fx.artists[0];
    const { unmount } = render(<ArtistPage id={a.id} />);
    const h = await screen.findByRole("heading", { name: "Fans also like" }, SLOW);
    const shelf = h.closest("section")!;
    const card = await within(shelf).findByLabelText("Cold Coast, not in your library", {}, SLOW);
    expect(card.className).toContain("missing-release");
    unmount();

    await setUp({ fansAlsoLikeCatalogue: false });
    render(<ArtistPage id={a.id} />);
    const h2 = await screen.findByRole("heading", { name: "Fans also like" }, SLOW);
    await new Promise((r) => setTimeout(r, 100));
    expect(within(h2.closest("section")!).queryByLabelText(/not in your library/)).toBeNull();
  });

  it("New to you shows on Home, and Not interested takes an artist off", async () => {
    render(<NewToYouShelf />);
    const h = await screen.findByRole("heading", { name: "New to you" }, SLOW);
    const shelf = h.closest("section")!;
    expect(within(shelf).getAllByText(`Because you play ${mock.fx.artists[0].title}`).length).toBeGreaterThan(0);
    await userEvent.click(within(shelf).getByRole("button", { name: "Not interested in Cold Coast" }));
    await waitFor(() => expect(within(shelf).queryByText("Cold Coast")).toBeNull());
    expect(useSettings.getState().settings?.dismissedArtists.map((d) => d.mbid)).toEqual([COLD]);
  });

  it("New to you is hidden when off, and its page says so", async () => {
    await setUp({ homeNewToYou: false });
    const { container } = render(<NewToYouShelf />);
    await new Promise((r) => setTimeout(r, 50));
    expect(container).toBeEmptyDOMElement();
    render(<NewToYouPage />);
    expect(screen.getByText("New to you is off. Turn it on in Settings.")).toBeInTheDocument();
  });
});

describe("settings", () => {
  it("has all four, and the dismissed list undoes one at a time", async () => {
    await setUp({ dismissedArtists: [{ mbid: COLD, name: "Cold Coast" }] });
    render(<SettingsPage />);
    for (const k of ["catalogueSearch", "fansAlsoLikeCatalogue", "homeNewToYou", "dismissedArtists"]) {
      expect(await screen.findByText((_, el) => el?.getAttribute("data-setting") === k, {}, SLOW)).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole("button", { name: "Show Cold Coast in suggestions again" }));
    await waitFor(() => expect(useSettings.getState().settings?.dismissedArtists).toEqual([]));
  });

  it("the dismissed list says when it's empty", () => {
    render(<DismissedArtists />);
    expect(screen.getByText(/None yet/)).toBeInTheDocument();
  });

  it("New to you and Fans also like start on and are synced", () => {
    expect(DEFAULT_SETTINGS.catalogueSearch && DEFAULT_SETTINGS.fansAlsoLikeCatalogue && DEFAULT_SETTINGS.homeNewToYou).toBe(true);
    expect(DEFAULT_SETTINGS.dismissedArtists).toEqual([]);
  });
});
