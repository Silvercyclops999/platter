/**
 * Phone search when the library doesn't have what's asked for: an artist
 * named exactly comes first with Get, music you don't have isn't listed
 * twice, other versions are a quiet word (in the song's sheet, not a button
 * on every row), and a phone that reaches Lidarr only through the Bridge
 * gets Get without anyone setting Lidarr up on it.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MockCatalogue } from "../lib/mock/catalogue";
import { DEFAULT_SETTINGS, type SearchResults, type Track } from "../lib/types";
import { catalogueTop, nameMatches, resetCatalogueCache, type CatalogueResults } from "../pages/SearchCatalogue";
import { CHECK_FAILED, HOME_CHECKING, HOME_UNREACHABLE, PHONE_SET_UP, lidarrNote } from "../pages/lidarrShared";
import { resetArtistPhotos } from "../lib/artistPhotos";
import { usePhoneForm } from "../phone/form";
import { PhoneSearch } from "../phone/pages/Search";
import { PhoneCatalogueArtist } from "../phone/pages/CatalogueArtist";
import { usePhoneUi } from "../phone/phoneUi";
import { useApp } from "../stores/app";
import { resetRequests } from "../stores/requests";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";

const mock = backend as MockBackend;
const SLOW = { timeout: 6000 };
const COLD = MockCatalogue.artistId(1);

async function setUp(lidarr: "ready" | "demo" = "demo") {
  mock.lidarr.reset(lidarr);
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: lidarr === "ready" ? "http://10.0.0.5:8686" : "", ...ONLINE_EXTRAS });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  usePhoneUi.setState({ tab: "search", nowPlaying: false, lyrics: false, sheets: [] });
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  resetCatalogueCache();
  resetArtistPhotos();
  resetRequests();
  await setUp();
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
  vi.restoreAllMocks();
});

function search(query: string) {
  useRouter.getState().reset({ name: "search", query });
  return render(<PhoneSearch />);
}

describe("an artist you don't have", () => {
  it("named exactly, is the first result with Get, and isn't listed again below", async () => {
    await setUp("ready");
    search("cold coast");
    const top = await screen.findByRole("button", { name: "Cold Coast, artist, not in your library" }, SLOW);
    const results = screen.getByRole("region", { name: "Results" });
    expect(within(results).getAllByRole("button")[0]).toBe(top);
    expect(await within(top).findByText("Get", {}, SLOW)).toBeInTheDocument();
    // The records follow under Not in your library; the artist isn't there twice.
    const rest = await screen.findByRole("region", { name: "Not in your library" }, SLOW);
    expect(within(rest).getByText("Low Tide Radio")).toBeInTheDocument();
    expect(within(rest).queryByText("Cold Coast", { selector: ".ph-row__title" })).toBeNull();
    // Something was found, so no "nothing matches" line.
    expect(screen.queryByText(/Nothing in your library matches/)).toBeNull();
    fireEvent.click(top);
    expect(useRouter.getState().entries[useRouter.getState().index].route).toEqual({ name: "catalogueArtist", mbid: COLD });
  });

  it("comes first over library results that only partly match", async () => {
    const song = { ...mock.fx.tracks[0], id: "x1", title: "Coastline (feat. Cold Hands)" } as Track;
    vi.spyOn(mock, "search").mockResolvedValue({ songs: [song], artists: [], albums: [], playlists: [], top: { kind: "track", track: song }, tookMs: 1, source: "local" } as SearchResults);
    search("cold coast");
    const top = await screen.findByRole("button", { name: "Cold Coast, artist, not in your library" }, SLOW);
    const results = screen.getByRole("region", { name: "Results" });
    expect(within(results).getAllByRole("listitem")[0]).toContainElement(top);
    expect(within(results).getByText("Coastline (feat. Cold Hands)")).toBeInTheDocument();
    // Without Lidarr there's no Get mark.
    expect(within(top).queryByText("Get")).toBeNull();
  });

  it("stays below a library result named exactly", async () => {
    // A loosely matching library artist leads the phone's list; the song named exactly is the top result.
    const song = { ...mock.fx.tracks[0], id: "x2", title: "Cold Coast" } as Track;
    const loose = { ...mock.fx.artists[0], id: "a-loose", title: "Coastal Cold Front" };
    vi.spyOn(mock, "search").mockResolvedValue({ songs: [song], artists: [loose], albums: [], playlists: [], top: { kind: "track", track: song }, tookMs: 1, source: "local" } as SearchResults);
    search("cold coast");
    const rest = await screen.findByRole("region", { name: "Not in your library" }, SLOW);
    await within(rest).findByRole("button", { name: "Cold Coast, artist, not in your library" }, SLOW);
    const results = screen.getByRole("region", { name: "Results" });
    const first = within(results).getAllByRole("listitem")[0];
    expect(within(first).getByText("Coastal Cold Front")).toBeInTheDocument();
    expect(first.closest("section[aria-label='Not in your library']")).toBeNull();
  });
});

describe("an artist's name as people type it", () => {
  it("matches stylised spellings, typos, a leading the and run-together words, not other names", () => {
    expect(nameMatches("juice world", "Juice WRLD")).toBe(true);
    expect(nameMatches("cold coasst", "Cold Coast")).toBe(true);
    expect(nameMatches("kold coast", "Cold Coast")).toBe(false);
    expect(nameMatches("the cold coast", "Cold Coast")).toBe(true);
    expect(nameMatches("coldcoast", "Cold Coast")).toBe(true);
    expect(nameMatches("juice world", "In A Perfect World")).toBe(false);
    expect(nameMatches("cold coast", "Coastal Cold Front")).toBe(false);
    // Read the way the library's search reads names.
    expect(nameMatches("simon and garfunkel", "Simon & Garfunkel")).toBe(true);
    expect(nameMatches("guns and roses", "Guns N' Roses")).toBe(true);
    expect(nameMatches("asap rocky", "A$AP Rocky")).toBe(true);
    expect(nameMatches("cold caost", "Cold Coast")).toBe(true);
    expect(nameMatches("mr", "Mayor")).toBe(false);
  });

  it("puts the artist you don't have on top over library results that don't name it", () => {
    const wrld = { mbid: "m1", name: "Juice WRLD" } as CatalogueResults["artists"][number];
    const other = { mbid: "m2", name: "World Order" } as CatalogueResults["artists"][number];
    const c = { artists: [other, wrld], albums: [], songs: [], inLibrary: new Set(), pending: [], source: "", failed: false } as unknown as CatalogueResults;
    const song = { kind: "track", track: { ...mock.fx.tracks[0], title: "In A Perfect World" } } as SearchResults["top"];
    expect(catalogueTop("juice world", c, song)?.name).toBe("Juice WRLD");
    expect(catalogueTop("world", c, song)).toBeNull();
  });

  it("when nothing matched every word, lists what's out there first and the library's near misses after", async () => {
    const song = { ...mock.fx.tracks[0], id: "p1", title: "Coast Road" } as Track;
    vi.spyOn(mock, "search").mockResolvedValue({ songs: [song], artists: [], albums: [], playlists: [], top: { kind: "track", track: song }, tookMs: 1, source: "local", partial: true } as SearchResults);
    search("cold coast");
    const top = await screen.findByRole("button", { name: "Cold Coast, artist, not in your library" }, SLOW);
    const results = screen.getByRole("region", { name: "Results" });
    expect(within(results).getAllByRole("button")[0]).toBe(top);
    const close = await screen.findByRole("region", { name: "Close matches in your library" }, SLOW);
    expect(within(close).getByText("Coast Road")).toBeInTheDocument();
    const rest = screen.getByRole("region", { name: "Not in your library" });
    expect(rest.compareDocumentPosition(close) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("keeps the library's result first when it's named the way you typed it", async () => {
    const song = { ...mock.fx.tracks[0], id: "p2", title: "Cold Coast" } as Track;
    vi.spyOn(mock, "search").mockResolvedValue({ songs: [song], artists: [], albums: [], playlists: [], top: { kind: "track", track: song }, tookMs: 1, source: "local", partial: true } as SearchResults);
    search("cold coast");
    await screen.findByRole("region", { name: "Not in your library" }, SLOW);
    expect(screen.queryByRole("region", { name: "Close matches in your library" })).toBeNull();
    const results = screen.getByRole("region", { name: "Results" });
    expect(within(within(results).getAllByRole("listitem")[0]).getByText("Cold Coast")).toBeInTheDocument();
  });
});

describe("other versions on a result", () => {
  it("are a quiet word on its grey line, and a line in the song's sheet", async () => {
    const song = { ...mock.fx.tracks[0], id: "v1", title: "Glass Bell", otherVersions: 0 } as Track;
    vi.spyOn(mock, "search").mockResolvedValue({
      songs: [song],
      artists: [],
      albums: [],
      playlists: [],
      top: { kind: "track", track: song },
      versions: { v1: [{} as Track, {} as Track, {} as Track, {} as Track] },
      tookMs: 1,
      source: "local",
    } as SearchResults);
    search("glass bell");
    const row = (await screen.findByText("Glass Bell", {}, SLOW)).closest("li")!;
    expect(within(row).getByText(/5 versions/)).toBeInTheDocument();
    expect(within(row).queryByRole("button", { name: /^\d+ versions$/ })).toBeNull();
    // Tapping a song with versions opens them to pick one, it doesn't play the first.
    const play = vi.spyOn(mock, "playContext");
    fireEvent.click(within(row).getByText("Glass Bell"));
    expect(play).not.toHaveBeenCalled();
    expect(usePhoneUi.getState().sheets.slice(-1)[0]).toMatchObject({ kind: "versions", track: { id: "v1" } });
    usePhoneUi.setState({ sheets: [] });
    fireEvent.click(within(row).getByRole("button", { name: "More for Glass Bell" }));
    const sheet = usePhoneUi.getState().sheets.slice(-1)[0] as { kind: string; track: Track };
    expect(sheet.kind).toBe("track");
    expect(sheet.track.otherVersions).toBe(4);
  });
});

describe("Lidarr through the Bridge on a phone", () => {
  it("never asks to set Lidarr up; Get albums shows once the check that runs by itself passes", async () => {
    mock.lidarr.homeSide = true;
    useRouter.getState().reset({ name: "catalogueArtist", mbid: COLD });
    render(<PhoneCatalogueArtist mbid={COLD} />);
    expect(await screen.findByRole("heading", { level: 1, name: "Cold Coast" }, SLOW)).toBeInTheDocument();
    expect(screen.queryByText(/Set up Lidarr/)).toBeNull();
    expect(await screen.findByRole("button", { name: "Get albums" }, SLOW)).toBeInTheDocument();
    expect(mock.lidarr.autoChecked).toBe(true);
    expect(screen.queryByText(/Set up Lidarr/)).toBeNull();
  });

  it("while the Bridge can't be reached, says getting music is paused, not to set anything up", async () => {
    mock.lidarr.bridgeQuiet = true;
    render(<PhoneCatalogueArtist mbid={COLD} />);
    expect(await screen.findByText(HOME_UNREACHABLE, {}, SLOW)).toBeInTheDocument();
    expect(screen.queryByText(/Set up Lidarr/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Get albums" })).toBeNull();
  });

  it("in search, the artist you don't have shows with Get once the Bridge's check passes", async () => {
    mock.lidarr.homeSide = true;
    search("cold coast");
    const top = await screen.findByRole("button", { name: "Cold Coast, artist, not in your library" }, SLOW);
    expect(await within(top).findByText("Get", {}, SLOW)).toBeInTheDocument();
    expect(screen.queryByText(/Set up Lidarr/)).toBeNull();
  });

  it("in search, the artist you don't have never vanishes: it shows with why Get isn't there", async () => {
    mock.lidarr.bridgeQuiet = true;
    search("cold coast");
    const top = await screen.findByRole("button", { name: "Cold Coast, artist, not in your library" }, SLOW);
    expect(within(top).queryByText("Get")).toBeNull();
    expect(await screen.findByText(HOME_UNREACHABLE, {}, SLOW)).toBeInTheDocument();
  });

  it("in search, a lookup that fails says so instead of showing nothing", async () => {
    vi.spyOn(mock, "catalogueSearchPart").mockRejectedValue(new Error("down"));
    search("cold coast");
    expect(await screen.findByText(/Couldn't look beyond your library just now/, {}, SLOW)).toBeInTheDocument();
  });

  it("with no Lidarr anywhere, still says how to get albums", async () => {
    render(<PhoneCatalogueArtist mbid={COLD} />);
    expect(await screen.findByText(PHONE_SET_UP, {}, SLOW)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Get albums" })).toBeNull();
    await act(() => new Promise((r) => setTimeout(r, 400)));
    expect(mock.lidarr.autoChecked).toBe(false);
  });
});

describe("the line where Get would be", () => {
  const base = { urlSet: false, awaySet: false, hasKey: false, headerNames: [], unencrypted: [], ready: false };
  it("says to set up only when Lidarr isn't reached, or its check here didn't pass", () => {
    expect(lidarrNote(null, "set up")).toBeNull();
    expect(lidarrNote({ ...base, ready: true }, "set up")).toBeNull();
    // Through the Bridge, its check running by itself, or not passed: never nothing.
    expect(lidarrNote({ ...base, homeSide: true }, "set up")).toBe(HOME_CHECKING);
    expect(lidarrNote({ ...base, homeSide: true, checkFailed: true }, "set up")).toBe(CHECK_FAILED);
    // Set up here with a key, but the check didn't pass (or the key isn't sent there).
    expect(lidarrNote({ ...base, urlSet: true, hasKey: true }, "set up")).toBe("set up");
    expect(lidarrNote({ ...base, urlSet: true, hasKey: true, moved: "moved" }, "set up")).toBe("set up");
    expect(lidarrNote({ ...base, homeQuiet: true }, "set up")).toBe(HOME_UNREACHABLE);
    expect(lidarrNote(base, "set up")).toBe("set up");
  });
});
