/**
 * Music that isn't in the library: search's "Not in your library" section
 * (after typing pauses, only when the library found little or it's opened,
 * hidden offline or with the setting off), catalogue artist and album pages
 * (dimmed, Request through Lidarr, Add to Missing, store links), Fans also
 * like with artists outside the library, New to you and "Not interested",
 * and the four settings. MusicBrainz, ListenBrainz and Lidarr are the
 * browser mock's fakes.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
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
import { SearchPage, interleave } from "../pages/SearchPage";
import { resetCatalogueCache } from "../pages/SearchCatalogue";
import { resetArtistPhotos } from "../lib/artistPhotos";
import { resetRequests } from "../stores/requests";
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
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: lidarr ? "http://10.0.0.5:8686" : "", ...ONLINE_EXTRAS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  mock.catalogue.searches.length = 0;
  mock.catalogue.parts.length = 0;
  mock.catalogue.photos.length = 0;
  resetCatalogueCache();
  resetArtistPhotos();
  resetRequests();
  await setUp();
});

describe("search", () => {
  it("shows music you don't have as the results when the library found nothing, an exact artist on top", async () => {
    render(<SearchPage query="cold coast" />);
    const top = await screen.findByRole("region", { name: "Top result" }, SLOW);
    expect(within(top).getByRole("button", { name: "Cold Coast" })).toBeInTheDocument();
    const list = await screen.findByRole("list", { name: "Results" }, SLOW);
    expect(await within(list).findByRole("button", { name: "Low Tide Radio" }, SLOW)).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /No results for/ })).toBeNull();
    await waitFor(() => expect(mock.catalogue.parts.map((p) => p.split(":")[0]).sort()).toEqual(["albums", "artists", "songs"]), SLOW);
    expect(mock.catalogue.parts.every((p) => p.endsWith(":musicBrainz")), "no Lidarr set up: MusicBrainz").toBe(true);
    await userEvent.click(within(top).getByRole("button", { name: "Cold Coast" }));
    expect(route()).toEqual({ name: "catalogueArtist", mbid: COLD });
  });

  it("weaves music you don't have in with the library's results, with an In your library cue", async () => {
    render(<SearchPage query="tide" />);
    const list = await screen.findByRole("list", { name: "Results" }, SLOW);
    const catalogueRow = await within(list).findByLabelText(/^Low Tide Radio, Album.*not in your library$/, {}, SLOW);
    const rows = within(list).getAllByRole("listitem");
    // Woven in after every few library rows, the library's first.
    const kinds = rows.map((r) => (r.classList.contains("sr-row--catalogue") ? "cat" : "lib"));
    expect(kinds.slice(0, 4)).toEqual(["lib", "lib", "lib", "cat"]);
    expect(rows.indexOf(catalogueRow)).toBeGreaterThan(3);
    // The library artist the catalogue also has folds into it, with a small cue.
    await waitFor(() => expect(screen.getAllByText("In your library").length).toBeGreaterThan(0), SLOW);
    expect(screen.queryByRole("button", { name: "Show music that isn't in your library" })).toBeNull();
  });

  it("uses Lidarr's search when it's set up, each kind showing as it arrives", async () => {
    await setUp({}, "ready");
    render(<SearchPage query="low tide" />);
    const list = await screen.findByRole("list", { name: /Results/ }, SLOW);
    await within(list).findByLabelText(/^Low Tide Radio, Album/, {}, SLOW);
    await waitFor(() => expect(mock.catalogue.parts).toContain("songs:low tide:musicBrainz"), SLOW);
    // Only this query's (an earlier test's slow answer can still land).
    const parts = mock.catalogue.parts.filter((p) => p.includes(":low tide:"));
    expect([...parts].sort()).toEqual(["albums:low tide:lidarr", "artists:low tide:lidarr", "songs:low tide:musicBrainz"]);
    expect(parts.indexOf("songs:low tide:musicBrainz"), "songs come from MusicBrainz, later").toBe(2);
    expect(await within(list).findByRole("button", { name: /^Get the album, Low Tide Radio/ }, SLOW)).toBeInTheDocument();
  });

  it("weaves one catalogue row after every few library rows", () => {
    const out = interleave([1, 2, 3, 4, 5, 6, 7], ["a", "b", "c", "d"]);
    expect(out.map((x) => x.lib ?? x.cat)).toEqual([1, 2, 3, "a", 4, 5, 6, "b", 7, "c", "d"]);
    expect(interleave([], ["a"]).map((x) => x.cat)).toEqual(["a"]);
  });

  it("keeps a query's results for the session", async () => {
    const { unmount } = render(<SearchPage query="saltmarsh" />);
    await screen.findByLabelText(/^Saltmarsh, Album/, {}, SLOW);
    const asked = mock.catalogue.parts.length;
    unmount();
    render(<SearchPage query="saltmarsh" />);
    await screen.findByLabelText(/^Saltmarsh, Album/, {}, SLOW);
    expect(mock.catalogue.parts.length).toBe(asked);
  });
  it("asks nothing with the setting off or while offline", async () => {
    await setUp({ catalogueSearch: false });
    const { unmount } = render(<SearchPage query="cold coast" />);
    await screen.findByRole("heading", { name: /No results for/ }, SLOW);
    await new Promise((r) => setTimeout(r, 900));
    expect(screen.queryByLabelText(/not in your library$/)).toBeNull();
    expect(mock.catalogue.parts).toEqual([]);
    unmount();

    await setUp();
    act(() => useApp.setState({ health: { ...(useApp.getState().health as Health), status: "offline", cachedOnly: true } }));
    render(<SearchPage query="cold coast" />);
    await new Promise((r) => setTimeout(r, 900));
    expect(screen.queryByLabelText(/not in your library$/)).toBeNull();
    expect(mock.catalogue.parts).toEqual([]);
    act(() => useApp.setState({ health: { ...(useApp.getState().health as Health), status: "local", cachedOnly: false } }));
  });
});

describe("catalogue artist page", () => {
  it("looks like a normal page: photo, a discography with popular releases and types, the bio's source", async () => {
    render(<ArtistPage id={COLD} catalogue />);
    expect(await screen.findByRole("heading", { level: 1, name: "Cold Coast" })).toBeInTheDocument();
    expect(screen.getAllByText("Not in your library").length).toBeGreaterThan(0);
    expect(screen.getByRole("heading", { name: "Discography" })).toBeInTheDocument();
    // The Get albums kinds (studio albums to start with) decide the types.
    const chips = screen.getByRole("group", { name: "Release type" });
    expect(within(chips).getAllByRole("button").map((b) => b.textContent)).toEqual(["Popular releases", "Albums"]);
    expect(screen.getByText("Latest release · Album")).toBeInTheDocument();
    const cards = screen.getAllByLabelText("Low Tide Radio by Cold Coast");
    expect(cards[0].className).not.toContain("missing-release");
    expect(document.querySelector(".detail-header__art img"), "the header has a photo").not.toBeNull();
    // No Lidarr: no Get, a note says how.
    expect(within(cards[0]).queryByRole("button", { name: /Get/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Get albums" })).toBeNull();
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

  it("the first Get goes through Lidarr's preview, naming the artist by MusicBrainz id", async () => {
    await setUp({}, "ready");
    const preview = vi.spyOn(mock, "lidarrRequestPreview");
    render(<ArtistPage id={COLD} catalogue />);
    const card = (await screen.findAllByLabelText("Low Tide Radio by Cold Coast"))[0];
    await userEvent.click(await within(card).findByRole("button", { name: "Get Low Tide Radio" }, SLOW));
    const dialog = await screen.findByRole("dialog", {}, { timeout: 8000 });
    expect(within(dialog).getByText(/Low Tide Radio/)).toBeInTheDocument();
    // The open dialog asks for the preview from an effect, which can run after it shows.
    await waitFor(() => expect(preview).toHaveBeenCalledWith(expect.objectContaining({ releaseGroupId: LOW_TIDE, artist: "Cold Coast", artistMbid: COLD })));
    preview.mockRestore();
  });
});

describe("catalogue album page", () => {
  it("shows the tracklist dimmed with store links and Add to Missing", async () => {
    const open = vi.spyOn(mock, "importOpenLink");
    render(<AlbumPage id={LOW_TIDE} catalogue />);
    expect(await screen.findByRole("heading", { level: 1, name: "Low Tide Radio" })).toBeInTheDocument();
    const tracks = screen.getByRole("list", { name: "Low Tide Radio songs, not in your library" });
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
  it("adds artists outside the library to Fans also like, with photos, unless turned off", async () => {
    const a = mock.fx.artists[0];
    const { unmount } = render(<ArtistPage id={a.id} />);
    const h = await screen.findByRole("heading", { name: "Fans also like" }, SLOW);
    const shelf = h.closest("section")!;
    const card = await within(shelf).findByLabelText("Cold Coast, not in your library", {}, SLOW);
    expect(card.className).not.toContain("missing-release");
    await waitFor(() => expect(card.querySelector("img"), "a photo, not the blank icon").not.toBeNull(), SLOW);
    expect(mock.catalogue.photos).toContain(COLD);
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
    for (const k of ["catalogueSearch", "catalogueSource", "askBeforeGet", "getAlbumsKinds", "fansAlsoLikeCatalogue", "homeNewToYou", "dismissedArtists"]) {
      expect(await screen.findByText((_, el) => el?.getAttribute("data-setting") === k, {}, SLOW)).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole("button", { name: "Show Cold Coast in suggestions again" }));
    await waitFor(() => expect(useSettings.getState().settings?.dismissedArtists).toEqual([]));
  });

  it("the dismissed list says when it's empty", () => {
    render(<DismissedArtists />);
    expect(screen.getByText(/None yet/)).toBeInTheDocument();
  });

  it("New to you, Fans also like and catalogue search start off, since they ask other services", () => {
    expect(DEFAULT_SETTINGS.catalogueSearch).toBe(true);
    expect(DEFAULT_SETTINGS.fansAlsoLikeCatalogue || DEFAULT_SETTINGS.homeNewToYou).toBe(false);
    expect(DEFAULT_SETTINGS.dismissedArtists).toEqual([]);
    expect([DEFAULT_SETTINGS.catalogueSource, DEFAULT_SETTINGS.askBeforeGet, DEFAULT_SETTINGS.getAlbumsKinds]).toEqual(["lidarr", false, ["studio"]]);
  });
});
