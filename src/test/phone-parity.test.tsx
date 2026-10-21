/**
 * The phone's pages for every route: each route the router has resolves to
 * a page made for the phone, each page is reachable from Home, Library or
 * You (or from the page that links to it on a computer), Get on the
 * phone's artist page follows a request through to the library, Library
 * import and Missing work on the phone, versions of a song show from
 * search and the song's sheet, and the Library tour fixes hold.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import routerSource from "../stores/router.ts?raw";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MockCatalogue } from "../lib/mock/catalogue";
import { DEFAULT_SETTINGS, type Track } from "../lib/types";
import { GET_UNDO_MS } from "../pages/GetButton";
import { resetCatalogueCache } from "../pages/SearchCatalogue";
import { resetArtistPhotos } from "../lib/artistPhotos";
import { usePhoneForm } from "../phone/form";
import { alphaKey, isLikedCopy, letterOf, phoneRows, phoneSubtitle } from "../phone/libraryRows";
import { baseTitle, groupEditions } from "../phone/editions";
import { hitLine, versionsCount } from "../phone/pages/Search";
import { countText, phoneDownloadProgress, phoneDownloadState } from "../phone/PhoneDownload";
import { DeviceSheet, OutputButton, playingLine, sortPlaces } from "../phone/DeviceSheet";
import { resetRouteWatch, routeLine } from "../phone/audioRoute";
import { PHONE_PAGES } from "../phone/routes";
import { PhonePage, PhoneShell } from "../phone/PhoneShell";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { shortLine } from "../pages/SettingsPage";
import { phoneText } from "../pages/settingsMeta";
import { useApp } from "../stores/app";
import { refreshRequests, resetRequests } from "../stores/requests";
import { useRouter, type Route } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useSocial } from "../stores/social";
import { useToasts } from "../stores/toasts";
import type { LibEntry, LibSort } from "../stores/library";

const mock = backend as MockBackend;
const SLOW = { timeout: 6000 };
const AFTER_UNDO = { timeout: GET_UNDO_MS + 6000 };
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

async function setUp(patch: Record<string, unknown> = {}, lidarr = false) {
  if (lidarr) {
    mock.lidarr.reset("ready");
    mock.lidarr.setTypedOnce(true);
  }
  await mock.settingsSet({ ...DEFAULT_SETTINGS, ...(lidarr ? { lidarrUrl: "http://10.0.0.5:8686" } : {}), ...ONLINE_EXTRAS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  usePhoneUi.setState({ tab: "home", nowPlaying: false, lyrics: false, sheets: [] });
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset({ name: "home" });
  resetCatalogueCache();
  resetArtistPhotos();
  resetRequests();
  await setUp();
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
  vi.restoreAllMocks();
});

/** One route of every name the router has, with values the browser mock knows. */
const SAMPLES: { [N in Route["name"]]: Extract<Route, { name: N }> } = {
  home: { name: "home" },
  search: { name: "search" },
  library: { name: "library" },
  artist: { name: "artist", id: "100" },
  discography: { name: "discography", id: "100" },
  lineage: { name: "lineage", id: "10000" },
  era: { name: "era", id: "190", era: "Glass Season" },
  album: { name: "album", id: "1000" },
  playlist: { name: "playlist", id: "500" },
  liked: { name: "liked" },
  settings: { name: "settings" },
  recents: { name: "recents" },
  whatsNew: { name: "whatsNew" },
  releaseRadar: { name: "releaseRadar" },
  lyrics: { name: "lyrics" },
  homeShelf: { name: "homeShelf", shelf: "recentlyAdded" },
  artistShelf: { name: "artistShelf", id: "100", shelf: "fansAlsoLike" },
  mix: { name: "mix", id: "daily:1" },
  radio: { name: "radio", seed: "artist:100" },
  bookmarks: { name: "bookmarks" },
  stats: { name: "stats" },
  profile: { name: "profile" },
  topSongs: { name: "topSongs" },
  import: { name: "import" },
  bring: { name: "bring" },
  doctor: { name: "doctor" },
  devices: { name: "devices" },
  soundMap: { name: "soundMap" },
  crates: { name: "crates" },
  crate: { name: "crate", kind: "genre", key: "rock" },
  catalogueArtist: { name: "catalogueArtist", mbid: MockCatalogue.artistId(1) },
  catalogueAlbum: { name: "catalogueAlbum", id: MockCatalogue.albumId(11) },
  newToYou: { name: "newToYou" },
  onThisDay: { name: "onThisDay" },
  shelf: { name: "shelf" },
  diary: { name: "diary" },
  blendNew: { name: "blendNew" },
  credits: { name: "credits", person: "Juno Ashby" },
  mixes: { name: "mixes" },
  archive: { name: "archive" },
  downloads: { name: "downloads" },
  help: { name: "help" },
};

/** Every route name in the router's own source, so a new one fails here until it has a phone page. */
function routerNames(): string[] {
  const union = routerSource.slice(routerSource.indexOf("export type Route ="), routerSource.indexOf("export interface HistoryEntry"));
  return [...union.matchAll(/name: "(\w+)"/g)].map((m) => m[1]).sort();
}

describe("every route has a phone page", () => {
  it("the table names exactly the router's routes", () => {
    expect(Object.keys(PHONE_PAGES).sort()).toEqual(routerNames());
    expect(Object.keys(SAMPLES).sort()).toEqual(routerNames());
  });

  // Route names whose phone page is the phone's own (no borrowed computer page).
  const routes = Object.values(SAMPLES).filter((r) => r.name !== "lyrics");
  it.each(routes.map((r) => [r.name, r] as const))("%s shows a phone page, never a computer page in a narrow column", async (_name, r) => {
    act(() => useRouter.getState().reset(r));
    const { container, unmount } = render(<PhonePage route={r} />);
    await waitFor(() => expect(container.querySelector(".ph-page")).not.toBeNull(), SLOW);
    expect(container.querySelector(".ph-desk"), "no borrowed narrow column").toBeNull();
    expect(container.querySelector(".page-pad:not(.ph-wrap .page-pad)"), "a computer page only inside a phone frame").toBeNull();
    unmount();
  });

  it("lyrics is never a page: it opens the player's lyrics and steps back", async () => {
    act(() => {
      useRouter.getState().reset({ name: "home" });
      useRouter.getState().navigate({ name: "album", id: "1000" });
    });
    render(<PhonePage route={{ name: "lyrics" }} />);
    await waitFor(() => expect(usePhoneUi.getState().lyrics).toBe(true));
    expect(usePhoneUi.getState().nowPlaying).toBe(true);
    expect(route()).toEqual({ name: "home" });
  });

  it("devices is the one page that stays on the computer, and it says why", async () => {
    render(<PhonePage route={{ name: "devices" }} />);
    expect(await screen.findByText(/A phone can't see those drives/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Downloads on this phone" })).toBeInTheDocument();
  });
});

describe("every page can be reached", () => {
  const opens = async (where: string, label: RegExp | string, want: Partial<Route>) => {
    const button = await screen.findByRole("button", { name: label }, SLOW);
    fireEvent.click(button);
    await waitFor(() => expect(route()).toMatchObject(want), SLOW);
    void where;
  };

  it("from Home: Show all, mixes and the Explore list", async () => {
    render(<PhoneShell />);
    await opens("home", "Show all: Recently added", { name: "homeShelf", shelf: "recentlyAdded" });
    act(() => phone.tab("home"));
    await opens("home", "Show all: Your mixes", { name: "mixes" });
    for (const [label, name] of [
      ["What's new", "whatsNew"],
      ["Release radar", "releaseRadar"],
      ["New to you", "newToYou"],
      ["On this day", "onThisDay"],
      ["Archive", "archive"],
      ["Sound map", "soundMap"],
    ] as const) {
      act(() => phone.tab("home"));
      const explore = await screen.findByRole("region", { name: "Explore" }, SLOW);
      fireEvent.click(within(explore).getByRole("button", { name: label }));
      await waitFor(() => expect(route().name).toBe(name));
    }
  });

  it("from You: Stats, Devices, Settings and Help", async () => {
    render(<PhoneShell />);
    for (const [label, name] of [
      ["Stats", "stats"],
      ["Devices", "devices"],
      ["Settings", "settings"],
      ["Help", "help"],
    ] as const) {
      act(() => phone.tab("you"));
      const you = await screen.findByTestId("phone-you", {}, SLOW);
      fireEvent.click(within(within(you).getByRole("list", { name: "You" })).getByRole("button", { name: label }));
      await waitFor(() => expect(route().name).toBe(name));
    }
  });

  it("what left You is one step from where it belongs", async () => {
    render(<PhoneShell />);
    // Stats leads to Top Songs, the diary, everything you played and bookmarks.
    for (const [label, name] of [
      ["Your Top Songs", "topSongs"],
      ["Listening diary", "diary"],
      ["Recents", "recents"],
      ["Bookmarks", "bookmarks"],
    ] as const) {
      act(() => useRouter.getState().navigate({ name: "stats" }));
      fireEvent.click(await screen.findByRole("button", { name: new RegExp(`^${label}`) }, SLOW));
      await waitFor(() => expect(route().name).toBe(name));
    }
    // Library's + makes a Blend.
    act(() => phone.tab("library"));
    fireEvent.click(await screen.findByRole("button", { name: "Create" }, SLOW));
    fireEvent.click(await screen.findByRole("button", { name: /^Blend/ }, SLOW));
    await waitFor(() => expect(route().name).toBe("blendNew"));
    // Help is a page, and a help page opens in it.
    act(() => useRouter.getState().navigate({ name: "help" }));
    fireEvent.click(await screen.findByRole("button", { name: /^Getting started/ }, SLOW));
    await waitFor(() => expect(route()).toMatchObject({ name: "help", slug: "getting-started" }));
  });

  it("from an artist: radio, eras, the discography, Appears on and Fans also like", async () => {
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "artist", id: "190" }));
    await opens("artist", "Vesper Lowe radio", { name: "radio", seed: "artist:190" });
    act(() => useRouter.getState().navigate({ name: "artist", id: "190" }));
    const eras = await screen.findByRole("region", { name: "Eras" }, SLOW);
    fireEvent.click(within(eras).getAllByRole("button")[0]);
    await waitFor(() => expect(route().name).toBe("era"));
    act(() => useRouter.getState().navigate({ name: "artist", id: "100" }));
    const disco = await screen.findByRole("region", { name: "Discography" }, SLOW);
    fireEvent.click(within(disco).getByRole("button", { name: "See all" }));
    await waitFor(() => expect(route()).toMatchObject({ name: "discography", id: "100" }));
    act(() => useRouter.getState().navigate({ name: "artist", id: "100" }));
    await opens("artist", "See all: Fans also like", { name: "artistShelf", shelf: "fansAlsoLike" });
  });

  it("from an album: its credits, its era and the artist's discography", async () => {
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "album", id: "1000" }));
    await opens("album", "See discography", { name: "discography" });
  });

  it("from a song: its versions and the timeline of them", async () => {
    const t = { ...mock.fx.tracks[0], otherVersions: 2 } as Track;
    render(<PhoneShell />);
    act(() => phone.push({ kind: "track", track: t }));
    fireEvent.click(await screen.findByRole("button", { name: "More" }));
    fireEvent.click(await screen.findByRole("button", { name: "Versions (3)" }));
    expect(usePhoneUi.getState().sheets.slice(-1)[0]).toEqual({ kind: "versions", track: t });
  });
});

describe("Follow on the phone's artist page", () => {
  async function clearFollows() {
    for (const f of await mock.followsList()) await mock.followSet(f, false);
    useSocial.setState({ follows: [], followIds: new Set() });
  }

  it("follow gets new albums; Following opens a sheet to just show them or unfollow", async () => {
    await setUp({ releaseRadar: false }, true);
    await clearFollows();
    const status = vi.spyOn(backend, "lidarrStatus");
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "artist", id: "100" }));
    const follow = await screen.findByRole("button", { name: /^Follow / }, SLOW);
    // The chip knows Lidarr is ready once its status has answered.
    await waitFor(() => expect(status).toHaveBeenCalled());
    await act(() => Promise.allSettled(status.mock.results.map((r) => r.value)));
    fireEvent.click(follow);
    await waitFor(async () => expect((await mock.followsList()).map((f) => f.id)).toEqual(["100"]));
    await waitFor(async () => expect((await mock.settingsGet()).releaseRadar).toBe(true), SLOW);
    // Lidarr is ready, so Following opens the sheet instead of unfollowing.
    fireEvent.click(await screen.findByRole("button", { name: /^Following / }, SLOW));
    expect(usePhoneUi.getState().sheets.map((x) => x.kind)).toEqual(["following"]);
    expect(await screen.findByText("New albums come in by themselves.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Just show new albums" }));
    await waitFor(async () => expect((await mock.followsList())[0].justShow).toBe(true));
    act(() => phone.push({ kind: "following", artist: { id: "100", title: "x", thumb: null } }));
    expect(await screen.findByText("New albums show on Release radar.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Unfollow" }));
    await waitFor(async () => expect(await mock.followsList()).toEqual([]));
  });

  it("without Lidarr, Following just unfollows", async () => {
    await clearFollows();
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "artist", id: "100" }));
    fireEvent.click(await screen.findByRole("button", { name: /^Follow / }, SLOW));
    fireEvent.click(await screen.findByRole("button", { name: /^Following / }, SLOW));
    await waitFor(async () => expect(await mock.followsList()).toEqual([]));
    expect(usePhoneUi.getState().sheets).toEqual([]);
  });
});

describe("Get on the phone's artist page", () => {
  it("requests a record you don't have, can be undone, then follows Lidarr to the library", async () => {
    await setUp({}, true);
    const get = vi.spyOn(mock, "lidarrGet");
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "artist", id: "100" }));
    const disco = await screen.findByRole("region", { name: "Discography" }, SLOW);
    fireEvent.click(within(disco).getByRole("button", { name: "Albums" }));
    const button = await within(disco).findByRole("button", { name: "Get Paper Lanterns" }, SLOW);
    fireEvent.click(button);
    expect(within(disco).getByText("Checking")).toBeInTheDocument();
    expect(within(disco).getByRole("button", { name: "Undo Get Paper Lanterns" })).toBeInTheDocument();
    // Waiting: sent, and Lidarr is looking.
    expect(await within(disco).findByLabelText("Looking on your indexers", {}, AFTER_UNDO)).toBeInTheDocument();
    expect(get).toHaveBeenCalledWith(expect.objectContaining({ title: "Paper Lanterns", artistId: "100" }));
    const rg = "6f1c0c55-0000-4000-8000-000000000001";
    mock.lidarr.advance(rg, "downloading", 50);
    await act(() => refreshRequests());
    expect(await within(disco).findByLabelText("Downloading 50%", {}, SLOW)).toBeInTheDocument();
    mock.lidarr.advance(rg, "imported");
    await act(() => refreshRequests());
    expect(await within(disco).findByText("In your library", {}, SLOW)).toBeInTheDocument();
  });

  it("says so when the request fails, and the button comes back", async () => {
    await setUp({}, true);
    vi.spyOn(mock, "lidarrGet").mockRejectedValue({ kind: "network", message: "Lidarr didn't answer" });
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "artist", id: "100" }));
    const disco = await screen.findByRole("region", { name: "Discography" }, SLOW);
    fireEvent.click(within(disco).getByRole("button", { name: "Albums" }));
    fireEvent.click(await within(disco).findByRole("button", { name: "Get Northern Rooms" }, SLOW));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Lidarr didn't answer"), AFTER_UNDO);
    expect(await within(disco).findByRole("button", { name: "Get Northern Rooms" }, SLOW)).toBeInTheDocument();
  });

  it("without Lidarr there's nothing to press", async () => {
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "artist", id: "100" }));
    await screen.findByRole("region", { name: "Discography" }, SLOW);
    expect(screen.queryByRole("button", { name: /^Get / })).toBeNull();
  });
});

describe("Library import on the phone", () => {
  beforeEach(() => {
    mock.importMock.reset();
  });

  it("shows each report and Bring in my listening, and turning it on shows where the waves stand", async () => {
    await setUp({ importSources: ["streaming", "transfer"] });
    mock.importMock.run("streaming", "my_data.zip");
    mock.importMock.run("transfer", "library.csv");
    render(<PhonePage route={{ name: "import" }} />);
    expect(await screen.findByTestId("report-streaming", {}, SLOW)).toHaveTextContent("33 of 38 songs found");
    expect(screen.getByTestId("report-transfer")).toHaveTextContent("8 of 10 songs found");
    const bring = screen.getByRole("region", { name: "Bring in my listening" });
    const sw = within(bring).getByRole("switch");
    expect(sw).toHaveAttribute("aria-checked", "false");
    fireEvent.click(sw);
    await waitFor(() => expect(useSettings.getState().settings?.bringIn).toBe(true));
    expect((await within(bring).findAllByText(/Wave \d+ of|Every wave/, {}, SLOW)).length).toBeGreaterThan(0);
  });

  it("takes a match from the review queue", async () => {
    await setUp({ importSources: ["streaming"] });
    mock.importMock.run("streaming", "my_data.zip");
    const review = vi.spyOn(mock, "importReview");
    render(<PhonePage route={{ name: "import" }} />);
    const accept = (await screen.findAllByRole("button", { name: /^Accept / }, SLOW))[0];
    fireEvent.click(accept);
    await waitFor(() => expect(review).toHaveBeenCalledWith(expect.anything(), { kind: "accept" }));
  });

  it("Missing lists the songs with where to buy them, and Get asks Lidarr", async () => {
    await setUp({ importSources: ["streaming"] }, true);
    mock.importMock.run("streaming", "my_data.zip");
    render(<PhonePage route={{ name: "import", view: "missing" }} />);
    const list = await screen.findByRole("list", { name: "Missing songs" }, SLOW);
    const first = within(list).getAllByRole("listitem")[0];
    fireEvent.click(within(first).getAllByRole("button")[0]);
    expect(await within(first).findAllByRole("button", { name: /^Search .* for / }, SLOW)).toHaveLength(3);
    expect(await within(first).findByRole("button", { name: /^Request / }, SLOW)).toBeInTheDocument();
  });
});

describe("versions of a song", () => {
  it("counts the versions search folded, else the library's count while it's on", () => {
    const t = { id: "t1", otherVersions: 3 } as Track;
    expect(versionsCount(t, { versions: { t1: [{} as Track, {} as Track] } } as never, true)).toBe(2);
    expect(versionsCount(t, null, true)).toBe(3);
    expect(versionsCount(t, null, false)).toBe(0);
  });

  it("the sheet lists each version with its record, and plays the one tapped", async () => {
    const t = { ...mock.fx.tracks[0] } as Track;
    const items = [
      { track: { ...t, id: "v1", title: "Song", albumTitle: "First Record" }, status: "released", current: true, year: 2019, date: null, edition: null, version: null, quality: null },
      { track: { ...t, id: "v2", title: "Song (Live)", albumTitle: "Live Record" }, status: "released", current: false, year: 2021, date: null, edition: null, version: null, quality: null },
    ];
    vi.spyOn(mock, "trackVersions").mockResolvedValue(items as never);
    const play = vi.spyOn(mock, "playContext");
    render(<PhoneShell />);
    act(() => phone.push({ kind: "versions", track: t }));
    const list = await screen.findByRole("list", { name: `Versions of ${t.title}` }, SLOW);
    expect(within(list).getByText("Live Record · 2021")).toBeInTheDocument();
    fireEvent.click(within(list).getByText("Song (Live)"));
    await waitFor(() => expect(play).toHaveBeenCalledWith(expect.objectContaining({ startTrackId: "v2" })));
  });
});

describe("the Library tour fixes", () => {
  const e = (x: Partial<LibEntry>): LibEntry => ({ kind: "playlist", id: x.title ?? "x", title: "x", sortTitle: "x", creator: "listener", creatorId: null, thumb: null, smart: false, format: null, addedAt: null, playedAt: null, pinned: false, ...x });

  it("Bookmarks, Stats and another player's liked list stay off; playlists say their song count", () => {
    const rows = phoneRows([e({ title: "Liked Songs", liked: true }), e({ title: "Bookmarks", bookmarks: true }), e({ title: "Stats", stats: true }), e({ title: "❤️ Tracks" }), e({ id: "p1", title: "Road" })], "recents", new Map([["p1", 12]]));
    expect(rows.map((r) => r.title)).toEqual(["Liked Songs", "Road"]);
    expect(phoneSubtitle(rows[1], true)).toBe("12 songs");
    expect(phoneSubtitle(rows[1], false)).toBe("Playlist · 12 songs");
    expect(isLikedCopy({ kind: "playlist", title: "Tracks" })).toBe(false);
  });

  it("Liked Songs stays at the top in every sort", () => {
    const list = [e({ id: "p1", title: "Road" }), e({ kind: "album", id: "a1", title: "Blue" }), e({ title: "Liked Songs", liked: true })];
    for (const sort of ["recents", "added", "alpha", "creator"] as LibSort[]) expect(phoneRows(list, sort)[0].title).toBe("Liked Songs");
  });

  it("A to Z goes by the name as written, ignoring a leading The", () => {
    const rows = phoneRows(
      [e({ kind: "artist", title: "Tyler, The Creator", sortTitle: "Tyler, The Creator" }), e({ kind: "artist", title: "Kanye West", sortTitle: "West Kanye" }), e({ kind: "artist", title: "The Kills", sortTitle: "Kills" }), e({ kind: "artist", title: "James Blake", sortTitle: "Blake James" })],
      "alpha",
    );
    expect(rows.map((r) => r.title)).toEqual(["James Blake", "Kanye West", "The Kills", "Tyler, The Creator"]);
    expect(alphaKey("The Kills")).toBe("Kills");
    expect(letterOf("The Kills")).toBe("K");
    expect(letterOf("2Pac")).toBe("#");
  });

  it("editions of one record fold into one card", () => {
    expect(baseTitle("Glass Season (2025 Remaster)")).toBe("glass season");
    expect(baseTitle("Late Orchestration - Deluxe Edition")).toBe("late orchestration");
    expect(baseTitle("Live at the Mill")).toBe("live at the mill");
    const groups = groupEditions([
      { id: "a", title: "Glass Season (Deluxe)" },
      { id: "b", title: "Glass Season" },
      { id: "c", title: "Night Ferry" },
      { id: "d", title: "Glass Season [Remastered 2011]" },
    ]);
    expect(groups.map((g) => [g.main.id, g.all.length])).toEqual([
      ["b", 3],
      ["c", 1],
    ]);
  });
});

describe("phone Settings words", () => {
  it("keeps one short line per row, and says things the phone's way", () => {
    expect(shortLine("Plays songs at a similar volume. Uses the loudness the server measured.")).toBe("Plays songs at a similar volume.");
    expect(phoneText("12 GB free on this drive")).toBe("12 GB free on this phone");
    expect(phoneText("Keep settings the same on your computers")).toBe("Keep settings the same on your devices");
    expect(phoneText("Always play Default Device with Flat.")).toBe("Always play this phone's speaker with Flat.");
    expect(phoneText("Shows the now playing view on the right, unless the queue is open there.")).toBe("Shows the now playing view.");
  });
});

describe("downloads on the phone", () => {
  const it0 = { state: "downloaded" as const, total: 19, done: 12, failed: 0, missing: 0, progress: 0.63 };
  it("an item is downloaded only when every one of its songs is, whatever the core's word", () => {
    expect(phoneDownloadState(it0)).toBe("downloading");
    expect(phoneDownloadState({ ...it0, done: 19 })).toBe("done");
    expect(phoneDownloadState({ ...it0, state: "downloaded", total: 0, done: 0 })).toBe("waiting");
    expect(phoneDownloadState({ ...it0, state: "queued", done: 0 })).toBe("waiting");
    expect(phoneDownloadState({ ...it0, done: 17, failed: 2 })).toBe("error");
    expect(phoneDownloadState(undefined)).toBe("none");
  });

  it("the ring never fills before the last song, and says 12/19", () => {
    expect(phoneDownloadProgress({ total: 19, done: 18, progress: 1 })).toBeLessThan(1);
    expect(phoneDownloadProgress({ total: 19, done: 19, progress: 1 })).toBe(1);
    expect(phoneDownloadProgress({ total: 0, done: 0, progress: 1 })).toBe(0);
    expect(countText({ total: 19, done: 12 })).toBe("12/19");
  });

  it("the Downloads page lists what's on the phone and is reached from Library's Downloads", async () => {
    render(<PhoneShell />);
    act(() => phone.tab("library"));
    const show = await screen.findByRole("group", { name: "Show" }, SLOW);
    expect(within(show).getAllByRole("button").map((b) => b.textContent)).toEqual(["Playlists", "Albums", "Artists", "Downloads"]);
    fireEvent.click(within(show).getByRole("button", { name: "Downloads" }));
    fireEvent.click(await screen.findByRole("button", { name: /^Manage downloads/ }, SLOW));
    await waitFor(() => expect(route()).toEqual({ name: "downloads" }));
    expect(await screen.findByRole("switch", { name: "Go offline" }, SLOW)).toBeInTheDocument();
  });
});

describe("devices and search", () => {
  it("other Platters are listed first", () => {
    const p = (name: string, via: "platter" | "plex" | "cast") => ({ id: name, name, via }) as never;
    expect(sortPlaces([p("Den TV", "cast"), p("Laptop", "platter"), p("Attic", "plex")]).map((x: { name: string }) => x.name)).toEqual(["Laptop", "Attic", "Den TV"]);
  });

  it("names where the phone's sound goes: the speaker or headphones by name, nothing on its own speaker", async () => {
    expect(routeLine({ kind: "speaker", name: "iPhone" })).toBeNull();
    expect(routeLine({ kind: "bluetooth", name: "JBL Flip 5" })?.line).toBe("Bluetooth · JBL Flip 5");
    expect(routeLine({ kind: "airplay", name: "Living Room" })?.line).toBe("AirPlay · Living Room");
    resetRouteWatch();
    mock.route = { kind: "bluetooth", name: "JBL Flip 5" };
    render(
      <>
        <OutputButton compact />
        <DeviceSheet />
      </>,
    );
    expect(await screen.findByText("Bluetooth · JBL Flip 5")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Playing on Bluetooth · JBL Flip 5. Choose where music plays" })).toBeInTheDocument();
    act(() => mock.emitRoute({ kind: "headphones", name: "AirPods Pro" }));
    expect(await screen.findByText("Headphones · AirPods Pro")).toBeInTheDocument();
    act(() => mock.emitRoute({ kind: "speaker", name: "iPhone" }));
    await waitFor(() => expect(screen.queryByText(/AirPods Pro/)).toBeNull());
    mock.route = null;
    resetRouteWatch();
  });

  it("with Stats off the Stats page still leads to the diary, Recents and Bookmarks; a computer-only tab opens Overview", async () => {
    await setUp({ statsPage: false });
    const { unmount } = render(<PhonePage route={{ name: "stats" }} />);
    expect(await screen.findByText("Stats are off")).toBeInTheDocument();
    for (const t of ["Listening diary", "Recents", "Bookmarks"]) expect(screen.getByText(t)).toBeInTheDocument();
    unmount();
    await setUp();
    render(<PhonePage route={{ name: "stats", tab: "wrapped" }} />);
    expect(await screen.findByRole("tab", { name: "Overview", selected: true })).toBeInTheDocument();
  });

  it("a paused player's line says Paused once, a playing one says Playing", () => {
    expect(playingLine("Paused: Unhomme • Kanye West")).toBe("Paused: Unhomme • Kanye West");
    expect(playingLine("Unhomme • Kanye West")).toBe("Playing: Unhomme • Kanye West");
  });

  it("every pill shows results in one row style; under a type pill the type word goes", () => {
    const album = { key: "album:1", kind: "album", id: "1", title: "Ivory", byline: "Copperline Choir", typeLabel: "Album", thumb: null, round: false } as const;
    expect(hitLine(album, false)).toBe("Album · Copperline Choir");
    expect(hitLine(album, true)).toBe("Copperline Choir");
    expect(hitLine({ ...album, kind: "artist", byline: "", typeLabel: "Artist" }, true)).toBe("Artist");
  });
});
