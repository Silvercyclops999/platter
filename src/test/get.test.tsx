/**
 * One-click Get (states that follow Lidarr, Undo, Needs you, the first
 * request typed once, Ask before each Get), Get albums on a catalogue
 * artist page, and photos of artists outside the library (lazy, a few at a
 * time, a monogram when nobody has one). Lidarr, MusicBrainz and the
 * picture sources are the browser mock's fakes.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MockCatalogue } from "../lib/mock/catalogue";
import { resetArtistPhotos, PHOTO_PARALLEL } from "../lib/artistPhotos";
import { DEFAULT_SETTINGS, type CatArtist } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { ArtistPage } from "../pages/ArtistPage";
import { CatalogueArtistCard } from "../pages/catalogue";
import { GET_UNDO_MS } from "../pages/GetButton";
import { SearchPage } from "../pages/SearchPage";
import { resetCatalogueCache } from "../pages/SearchCatalogue";
import { useApp } from "../stores/app";
import { refreshRequests, resetRequests } from "../stores/requests";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const COLD = MockCatalogue.artistId(1);
const LOW_TIDE = MockCatalogue.albumId(11);
const LOWLIGHT = MockCatalogue.albumId(62);
/** Long enough for the undo window, the call and a render. */
const AFTER_UNDO = { timeout: GET_UNDO_MS + 5000 };
const SLOW = { timeout: 4000 };

async function setUp(patch: Record<string, unknown> = {}, typedOnce = true) {
  mock.lidarr.reset("ready");
  mock.lidarr.setTypedOnce(typedOnce);
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: "http://10.0.0.5:8686", ...ONLINE_EXTRAS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  mock.catalogue.photos.length = 0;
  mock.getQueued = [];
  resetCatalogueCache();
  resetArtistPhotos();
  resetRequests();
  await setUp();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function getOnAlbumPage(rgid = LOW_TIDE, title = "Low Tide Radio") {
  render(<AlbumPage id={rgid} catalogue />);
  await screen.findByRole("heading", { level: 1, name: title }, SLOW);
  return screen.findByRole("button", { name: `Get ${title}` }, SLOW);
}

describe("one-click Get", () => {
  it("sends in one click after a few seconds to undo, then follows Lidarr", async () => {
    const get = vi.spyOn(mock, "lidarrGet");
    await userEvent.click(await getOnAlbumPage());
    expect(screen.getByText("Checking")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Undo Get Low Tide Radio" })).toBeInTheDocument();
    expect(await screen.findByText("Looking on your indexers", {}, AFTER_UNDO)).toBeInTheDocument();
    expect(get).toHaveBeenCalledWith(expect.objectContaining({ releaseGroupId: LOW_TIDE, artist: "Cold Coast", artistMbid: COLD }));
    expect(screen.queryByRole("dialog"), "no dialog when nothing needs you").toBeNull();

    // Lidarr works on it: the button follows the queue and history.
    for (const [state, progress, label] of [
      ["downloading", 40, "Downloading 40%"],
      ["importing", null, "Importing"],
      ["imported", null, "In your library"],
    ] as const) {
      mock.lidarr.advance(LOW_TIDE, state, progress);
      await act(() => refreshRequests());
      expect(await screen.findByText(label, {}, SLOW)).toBeInTheDocument();
    }
  });

  it("can be undone before anything is sent", async () => {
    const get = vi.spyOn(mock, "lidarrGet");
    await userEvent.click(await getOnAlbumPage());
    await userEvent.click(screen.getByRole("button", { name: "Undo Get Low Tide Radio" }));
    expect(await screen.findByRole("button", { name: "Get Low Tide Radio" })).toBeInTheDocument();
    await new Promise((r) => setTimeout(r, GET_UNDO_MS + 300));
    expect(get).not.toHaveBeenCalled();
    expect(mock.lidarr.list()).toEqual([]);
  });

  it("asks for the album's name only for the very first request", async () => {
    await setUp({}, false);
    await userEvent.click(await getOnAlbumPage());
    const dialog = await screen.findByRole("dialog", {}, AFTER_UNDO);
    const field = await within(dialog).findByLabelText(/Type the album's name/, {}, SLOW);
    await userEvent.type(field, "Low Tide Radio");
    await userEvent.click(within(dialog).getByRole("button", { name: "Request" }));
    expect(await screen.findByText("Looking on your indexers", {}, SLOW)).toBeInTheDocument();
  });

  it("says Needs you with the reason when a check says no, and the dialog shows it", async () => {
    await userEvent.click(await getOnAlbumPage(LOWLIGHT, "Lowlight"));
    const needs = await screen.findByRole("button", { name: /^Lowlight needs you: .*unreleased/ }, AFTER_UNDO);
    expect(mock.lidarr.list()).toEqual([]);
    await userEvent.click(needs);
    expect(await screen.findByRole("dialog", {}, SLOW)).toBeInTheDocument();
  });

  it("Not found yet offers Search again and another edition", async () => {
    await userEvent.click(await getOnAlbumPage());
    await screen.findByText("Looking on your indexers", {}, AFTER_UNDO);
    mock.lidarr.advance(LOW_TIDE, "notFound");
    await act(() => refreshRequests());
    expect(await screen.findByText("Not found yet", {}, SLOW)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Search again for Low Tide Radio" }));
    expect(await screen.findByText("Looking on your indexers", {}, SLOW)).toBeInTheDocument();
    expect(mock.lidarr.searchesAgain).toHaveLength(1);
    mock.lidarr.advance(LOW_TIDE, "notFound");
    await act(() => refreshRequests());
    await userEvent.click(await screen.findByRole("button", { name: "Try another edition of Low Tide Radio" }, SLOW));
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "search", query: "Cold Coast Low Tide Radio" });
  });

  it("with Ask before each Get on, the dialog opens every time", async () => {
    await setUp({ askBeforeGet: true });
    const get = vi.spyOn(mock, "lidarrGet");
    await userEvent.click(await getOnAlbumPage());
    expect(await screen.findByRole("dialog", {}, SLOW)).toBeInTheDocument();
    expect(get).not.toHaveBeenCalled();
  });

  it("is on album rows in search, and Get the album on song rows", async () => {
    render(<SearchPage query="low tide" />);
    expect(await screen.findByRole("button", { name: "Get Low Tide Radio" }, SLOW)).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: /^Get the album, Low Tide Radio/ }, SLOW)).toBeInTheDocument();
  });
});

describe("Get albums", () => {
  it("asks which kinds, shows the count, then queues them", async () => {
    render(<ArtistPage id={COLD} catalogue />);
    await userEvent.click(await screen.findByRole("button", { name: "Get albums" }, SLOW));
    const dialog = await screen.findByRole("dialog", { name: "Get albums by Cold Coast" });
    expect(within(dialog).getByRole("checkbox", { name: "Studio albums" })).toBeChecked();
    expect(within(dialog).getByRole("checkbox", { name: "EPs" })).not.toBeChecked();
    expect(await within(dialog).findByRole("button", { name: "Get 2 albums" }, SLOW)).toBeInTheDocument();
    const list = within(dialog).getByRole("list", { name: "Albums to get" });
    expect(within(list).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Harbour Static · 2018", "Low Tide Radio · 2021"]);
    await userEvent.click(within(dialog).getByRole("checkbox", { name: "EPs" }));
    await userEvent.click(await within(dialog).findByRole("button", { name: "Get 3 albums" }, SLOW));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(mock.getQueued.map((q) => q.title)).toEqual(["Harbour Static", "Weatherglass", "Low Tide Radio"]);
    expect(useToasts.getState().toasts.slice(-1)[0]?.message).toMatch(/3 albums by Cold Coast queued/);
  });

  it("starts with the kinds from Settings", async () => {
    await setUp({ getAlbumsKinds: ["studio", "live"] });
    render(<ArtistPage id={COLD} catalogue />);
    await userEvent.click(await screen.findByRole("button", { name: "Get albums" }, SLOW));
    const dialog = await screen.findByRole("dialog", { name: "Get albums by Cold Coast" });
    expect(within(dialog).getByRole("checkbox", { name: "Live albums" })).toBeChecked();
    expect(await within(dialog).findByRole("button", { name: "Get 3 albums" }, SLOW)).toBeInTheDocument();
  });
});

describe("pictures of artists you don't have", () => {
  const artist = (n: number): CatArtist => ({ mbid: MockCatalogue.artistId(n), name: ["", "Cold Coast", "Tidewater Hymns", "Fen Lights", "Marrow & Vine", "Ostra", "Paper Satellites"][n], disambiguation: null, kind: null, photo: null, libraryId: null });

  it("show a photo, or a monogram when no source has one, never the blank icon", async () => {
    render(
      <>
        <CatalogueArtistCard artist={artist(2)} />
        <CatalogueArtistCard artist={artist(5)} />
      </>,
    );
    const tide = screen.getByLabelText("Tidewater Hymns, not in your library");
    await waitFor(() => expect(tide.querySelector("img")).not.toBeNull(), SLOW);
    const ostra = screen.getByLabelText("Ostra, not in your library");
    await waitFor(() => expect(mock.catalogue.photos).toContain(MockCatalogue.artistId(5)), SLOW);
    expect(ostra.querySelector(".art__monogram")?.textContent).toBe("O");
    expect(ostra.querySelector("svg"), "no blank person icon").toBeNull();
  });

  it("load as cards come into view, a few at a time", async () => {
    const seen: ((on: boolean) => void)[] = [];
    vi.stubGlobal(
      "IntersectionObserver",
      class {
        private cb: (e: { isIntersecting: boolean }[]) => void;
        constructor(cb: (e: { isIntersecting: boolean }[]) => void) {
          this.cb = cb;
        }
        observe() {
          seen.push((on) => this.cb([{ isIntersecting: on }]));
        }
        disconnect() {}
      },
    );
    let running = 0;
    let most = 0;
    const real = mock.catalogueArtistPhoto.bind(mock);
    vi.spyOn(mock, "catalogueArtistPhoto").mockImplementation(async (id, name) => {
      running += 1;
      most = Math.max(most, running);
      await new Promise((r) => setTimeout(r, 40));
      const p = await real(id, name);
      running -= 1;
      return p;
    });
    render(
      <>
        {[1, 2, 3, 4, 5, 6].map((n) => (
          <CatalogueArtistCard key={n} artist={artist(n)} />
        ))}
      </>,
    );
    await new Promise((r) => setTimeout(r, 100));
    expect(mock.catalogue.photos, "nothing is asked off screen").toEqual([]);
    act(() => seen.forEach((s) => s(true)));
    await waitFor(() => expect(mock.catalogue.photos).toHaveLength(6), SLOW);
    expect(most).toBeLessThanOrEqual(PHOTO_PARALLEL);
  });

  it("come with Lidarr's search results, and the rest are asked for as rows show", async () => {
    render(<SearchPage query="tidewater hymns" />);
    const top = await screen.findByRole("region", { name: "Top result" }, SLOW);
    await waitFor(() => expect(top.querySelector("img")).not.toBeNull(), SLOW);
    expect(mock.catalogue.photos).toContain(MockCatalogue.artistId(2));
  });
});
