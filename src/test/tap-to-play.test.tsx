/**
 * Tap a song you don't have: search rows show Get and a tap gets and plays
 * it, the player shows the "Getting it" line in place of the seek bar, the
 * three switches under Lidarr's More, and the last get's timings. Lidarr and
 * MusicBrainz are the browser mock's fakes.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MockCatalogue } from "../lib/mock/catalogue";
import { DEFAULT_SETTINGS, type DoctorFinding, type Tap } from "../lib/types";
import { ArtistPage, popularItems } from "../pages/ArtistPage";
import { DoctorPage } from "../pages/DoctorPage";
import { lastGetText } from "../pages/LidarrSettings";
import { SearchPage } from "../pages/SearchPage";
import { resetCatalogueCache } from "../pages/SearchCatalogue";
import { SettingsPage } from "../pages/SettingsPage";
import { advancedView, inMore, settingsView } from "../pages/settingsMeta";
import { PlayerBar } from "../shell/PlayerBar";
import { useApp } from "../stores/app";
import { connectPlayer, usePlayer } from "../stores/player";
import { resetRequests } from "../stores/requests";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { dismissTap, getting, gettingFill, gettingLabel, useTaps } from "../stores/taps";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const FIRST_FROST = MockCatalogue.albumId(14);
const SLOW = { timeout: 4000 };

async function setUp(patch: Record<string, unknown> = {}, typedOnce = true) {
  mock.lidarr.reset("ready");
  mock.lidarr.setTypedOnce(typedOnce);
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: "http://10.0.0.5:8686", ...ONLINE_EXTRAS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
}

function tap(patch: Partial<Tap> = {}): Tap {
  return {
    id: "tap-x",
    title: "First Frost",
    artist: "Cold Coast",
    album: "First Frost",
    releaseGroupId: FIRST_FROST,
    thumb: null,
    durationMs: 200_000,
    phase: "downloading",
    progress: 62,
    message: null,
    requestId: "r1",
    timings: { foundMs: null, downloadedMs: null, playingMs: null, inLibraryMs: null },
    wanted: true,
    fromDownload: false,
    ...patch,
  };
}

const route = () => useRouter.getState().entries[useRouter.getState().index].route;

let disconnect: () => void;
beforeAll(async () => {
  disconnect = connectPlayer(mock);
  await waitFor(() => expect(usePlayer.getState().state?.track).toBeTruthy());
});
afterAll(() => disconnect());

beforeEach(async () => {
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
  useTaps.setState({ current: null });
  mock.resetTaps();
  resetCatalogueCache();
  resetRequests();
  await setUp();
});

afterEach(() => vi.restoreAllMocks());

async function songRow() {
  render(<SearchPage query="first frost" />);
  const list = await screen.findByRole("list", { name: /Results/ }, SLOW);
  return within(list).findByLabelText(/^First Frost, Song/, {}, SLOW);
}

describe("search rows for songs you don't have", () => {
  it("show Get instead of the length, and a tap gets the song and shows it coming", async () => {
    const sent = vi.spyOn(mock, "lidarrTap");
    const row = await songRow();
    expect(row).toHaveAccessibleName(/a tap gets it and plays it$/);
    expect(within(row).getByText("Get")).toBeInTheDocument();
    expect(row.querySelector(".sr-row__by")?.textContent).not.toMatch(/\d:\d\d/);
    const before = route();
    await userEvent.click(within(row).getByRole("button", { name: "First Frost" }));
    await waitFor(() => expect(sent).toHaveBeenCalledWith(expect.objectContaining({ title: "First Frost", artist: "Cold Coast", releaseGroupId: FIRST_FROST })));
    await waitFor(() => expect(useTaps.getState().current?.title).toBe("First Frost"));
    expect(getting(useTaps.getState().current)).toBe(true);
    expect(route(), "stays where it was").toEqual(before);
  });

  it("show Get on a device that reaches Lidarr only through the Bridge or a computer at home", async () => {
    await setUp({ lidarrUrl: "" });
    mock.lidarr.homeSide = true;
    const row = await songRow();
    expect(await within(row).findByText("Get", {}, SLOW)).toBeInTheDocument();
  });

  it("open the album's page as before when the setting is off", async () => {
    await setUp({ lidarrTapToGet: false });
    const sent = vi.spyOn(mock, "lidarrTap");
    const row = await songRow();
    expect(within(row).queryByText("Get")).toBeNull();
    await userEvent.click(within(row).getByRole("button", { name: "First Frost" }));
    expect(route()).toEqual({ name: "catalogueAlbum", id: FIRST_FROST });
    expect(sent).not.toHaveBeenCalled();
  });

  it("send the very first request to its album page, where the name is typed once", async () => {
    await setUp({}, false);
    const row = await songRow();
    await userEvent.click(within(row).getByRole("button", { name: "First Frost" }));
    await waitFor(() => expect(route()).toEqual({ name: "catalogueAlbum", id: FIRST_FROST }), SLOW);
    expect(useToasts.getState().toasts.map((t) => t.message).join(" ")).toMatch(/typing the album's name, once/);
  });
});

describe("the Getting it line", () => {
  it("takes the seek bar's place while the song comes, and music playing keeps playing", async () => {
    render(<PlayerBar />);
    expect(screen.queryByRole("status", { name: /Getting/ })).toBeNull();
    act(() => useTaps.setState({ current: tap() }));
    const line = await screen.findByRole("status", { name: /Getting/ });
    expect(line).toHaveTextContent("Downloading 62%");
    expect(screen.getAllByText("First Frost").length).toBeGreaterThan(0);
    expect(screen.queryByRole("slider", { name: /Seek/i })).toBeNull();
    // Ready: the seek bar is back.
    act(() => useTaps.setState({ current: tap({ phase: "playing", fromDownload: true }) }));
    await waitFor(() => expect(screen.queryByRole("status", { name: /Getting/ })).toBeNull());
  });

  it("says when nothing can be found, and OK clears it", async () => {
    const msg = "Your indexers have no copy of this yet. Platter will keep looking and tell you.";
    render(<PlayerBar />);
    act(() => useTaps.setState({ current: tap({ phase: "waiting", progress: null, message: msg }) }));
    const line = await screen.findByRole("status", { name: /Getting/ });
    expect(line).toHaveTextContent(msg);
    await userEvent.click(within(line).getByRole("button", { name: "OK" }));
  });

  it("labels and fills each step", () => {
    expect(gettingLabel("searching", null)).toBe("Searching");
    expect(gettingLabel("downloading", 62)).toBe("Downloading 62%");
    expect(gettingLabel("ready", null)).toBe("Ready");
    expect(gettingFill("downloading", 50)).toBeGreaterThan(gettingFill("searching", null));
    expect(gettingFill("ready", null)).toBe(1);
    expect(getting(tap({ wanted: false })), "only the latest tap shows").toBe(false);
    dismissTap();
  });
});

describe("settings", () => {
  beforeEach(() => {
    settingsView.inlinePages = false;
    advancedView.startOpen = false;
  });
  afterEach(() => {
    settingsView.inlinePages = true;
    advancedView.startOpen = true;
  });

  it("has the three switches under Lidarr's More, all on and synced", async () => {
    expect(["lidarrTapToGet", "lidarrPreferFastest", "lidarrPlayArriving"].every((key) => inMore({ key, page: "lidarr" }))).toBe(true);
    const user = userEvent.setup();
    render(<SettingsPage page="lidarr" />);
    const page = await screen.findByRole("region", { name: "Lidarr" });
    await user.click(await within(page).findByRole("button", { name: /^More in Lidarr/ }, SLOW));
    const tapOn = await within(page).findByRole("switch", { name: "Tap a song you don't have to get and play it" }, SLOW);
    const fastest = within(page).getByRole("switch", { name: "Prefer the fastest download" });
    const arriving = within(page).getByRole("switch", { name: "Play while it's still arriving" });
    expect([tapOn, fastest, arriving].every((s) => (s as HTMLInputElement).checked || s.getAttribute("aria-checked") === "true")).toBe(true);
    await user.click(fastest);
    await waitFor(async () => expect((await mock.settingsGet()).lidarrPreferFastest).toBe(false));
    await user.click(tapOn);
    await waitFor(async () => expect((await mock.settingsGet()).lidarrTapToGet).toBe(false));
    // The finer two hide with it off.
    await waitFor(() => expect(within(page).queryByRole("switch", { name: "Prefer the fastest download" })).toBeNull());
  });

  it("shows how long the last get took", () => {
    expect(lastGetText(null)).toBeNull();
    expect(lastGetText({ title: "First Frost", at: 1, timings: { foundMs: 4_200, downloadedMs: 31_000, playingMs: 38_400, inLibraryMs: null } })).toBe(
      "Last get: found 4 s, downloaded 31 s, playing at 38 s.",
    );
    expect(lastGetText({ title: "x", at: 1, timings: { foundMs: 900, downloadedMs: null, playingMs: null, inLibraryMs: 60_000 } })).toBe("Last get: found 1 s.");
  });
});

describe("an artist's Popular", () => {
  it("places the top songs you don't have where they rank, only when a tap can get them", () => {
    const own = [{ id: "a" }, { id: "b" }, { id: "c" }] as never[];
    const more = [
      { title: "X", release: null, after: 0 },
      { title: "Y", release: "Z", after: 2 },
      { title: "W", release: null, after: 9 },
    ];
    const keys = (tap: boolean) => popularItems(own, more, tap).map((i) => ("get" in i ? i.get.title : (i.own as { id: string }).id));
    expect(keys(true)).toEqual(["X", "a", "b", "Y", "c", "W"]);
    expect(keys(false)).toEqual(["a", "b", "c"]);
  });

  it("offers them with Get, and a tap gets and plays the song", async () => {
    const sent = vi.spyOn(mock, "lidarrTap");
    const artist = mock.fx.artists[0];
    render(<ArtistPage id={artist.id} />);
    const row = await screen.findByRole("row", { name: /^Night Drive by .*a tap gets it and plays it$/ }, SLOW);
    expect(within(row).getByText("Get")).toBeInTheDocument();
    await userEvent.click(within(row).getByRole("button", { name: "Night Drive" }));
    await waitFor(() => expect(sent).toHaveBeenCalledWith(expect.objectContaining({ title: "Night Drive", artist: artist.title, album: `${artist.title} Live` })));
  });

  it("leaves them out without Lidarr", async () => {
    await setUp({ lidarrUrl: "" });
    const artist = mock.fx.artists[0];
    render(<ArtistPage id={artist.id} />);
    await waitFor(() => expect(screen.getAllByRole("row", { name: new RegExp(` by ${artist.title}`) }).length).toBeGreaterThan(0), SLOW);
    expect(screen.queryByRole("row", { name: /Night Drive/ })).toBeNull();
  });
});

describe("Library doctor", () => {
  it("lists a tapped song the server hasn't shown, as plain text, and Look again asks the server to catch up", async () => {
    const late: DoctorFinding = {
      id: "notInPlex:rg:dawn",
      kind: "notInPlex",
      artistId: "",
      artist: "Northlight",
      albumId: "",
      album: "Second Wind",
      thumb: null,
      title: "Dawn isn't in your library yet",
      text: "Lidarr downloaded Dawn by Northlight, but your server hasn't shown it. Look again asks your server to catch up.",
      evidence: ["Lidarr imported Second Wind into its album folder."],
      fix: "lookAgain",
      tracks: [],
      others: [],
      table: [],
    };
    vi.spyOn(mock, "doctorReport").mockResolvedValue({ on: true, findings: [late], ignored: [], count: 1, running: false, checkedAt: null, tagsRead: 0, tagsTotal: 0 });
    const sync = vi.spyOn(mock, "syncNow").mockResolvedValue(undefined);
    render(<DoctorPage />);
    expect(await screen.findByRole("heading", { name: "Dawn isn't in your library yet" }, SLOW)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Second Wind" })).toBeNull();
    expect(screen.getAllByText(/Songs not in your library yet/).length).toBeGreaterThan(0);
    await userEvent.click(screen.getByRole("button", { name: "Look again" }));
    expect(sync).toHaveBeenCalledWith(false);
  });
});

describe("an artist you don't have", () => {
  it("shows Get albums once the server answers, after a first answer that came before it", async () => {
    const real = mock.lidarrStatus.bind(mock);
    // Plex still being looked for: not yet known to be the owner.
    vi.spyOn(mock, "lidarrStatus").mockImplementationOnce(async () => ({ ...(await real()), ready: false, notOwner: true }));
    render(<ArtistPage id={MockCatalogue.artistId(1)} catalogue />);
    expect(await screen.findByText(/Set up Lidarr in Settings/, {}, SLOW)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Get albums" })).toBeNull();
    act(() => mock.emit("auth://changed", { state: "signedIn", username: "listener", needsServerPick: false }));
    expect(await screen.findByRole("button", { name: "Get albums" }, SLOW)).toBeInTheDocument();
  });
});
