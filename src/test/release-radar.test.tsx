/**
 * Release radar in the UI: the Home shelf and its page (in your library,
 * can't request and why, Request when Lidarr is ready, type filters, Check
 * now), following that gets new albums (computer menu, phone sheet), and the
 * settings.
 * MusicBrainz and Lidarr are the browser mock's.
 */
import { ONLINE_EXTRAS } from "./onlineExtras";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { ContextMenuHost } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { filterRadar, radarDate, radarKind } from "../lib/radar";
import { DEFAULT_SETTINGS, type RadarItem } from "../lib/types";
import { ArtistPage } from "../pages/ArtistPage";
import { HomePage } from "../pages/HomePage";
import { LidarrRequestsList } from "../pages/LidarrRequest";
import { ReleaseRadarPage } from "../pages/ReleaseRadar";
import { SettingsPage } from "../pages/SettingsPage";
import { useApp } from "../stores/app";
import { useRouter } from "../stores/router";
import { useSocial } from "../stores/social";
import { loadSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

async function setUp(lidarr: "ready" | "demo" | "none", patch: Record<string, unknown> = {}) {
  mock.lidarr.reset(lidarr === "none" ? "demo" : lidarr);
  mock.radar.reset();
  await mock.settingsSet({ ...DEFAULT_SETTINGS, lidarrUrl: lidarr === "none" ? "" : "http://192.168.x.x:8686", ...ONLINE_EXTRAS, ...patch });
  useSettings.setState({ settings: null });
  await loadSettings();
  useSocial.setState({ follows: [], followIds: new Set() });
  for (const f of await mock.followsList()) await mock.followSet(f, false);
}

beforeEach(() => {
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  useToasts.setState({ toasts: [] });
  useRouter.getState().reset();
});

describe("wording", () => {
  const now = Date.UTC(2027, 0, 15, 9);
  it("says when a release came out, or when it's out", () => {
    expect(radarDate("2027-01-15", now)).toBe("Today");
    expect(radarDate("2027-01-14", now)).toBe("Yesterday");
    expect(radarDate("2027-01-10", now)).toBe("5 days ago");
    expect(radarDate("2026-12-02", now)).toBe("Dec 2");
    expect(radarDate("2027-02-03", now)).toBe("Out Feb 3");
    expect(radarDate("2027-03", now)).toBe("Mar 2027");
    expect(radarDate("2026", now)).toBe("2026");
    expect(radarKind("ep")).toBe("EP");
    expect(radarKind("single")).toBe("Single");
    expect(radarKind("album")).toBe("Album");
  });

  it("filters by type", () => {
    const items = ["album", "ep", "single"].map((kind) => ({ kind }) as RadarItem);
    expect(filterRadar(items, "albums").map((i) => i.kind)).toEqual(["album"]);
    expect(filterRadar(items, "epsSingles").map((i) => i.kind)).toEqual(["ep", "single"]);
    expect(filterRadar(items, "all")).toHaveLength(3);
  });
});

describe("the Home shelf", () => {
  it("shows new releases with Show all to the page", async () => {
    const user = userEvent.setup();
    await setUp("ready");
    render(<HomePage />);
    const shelf = (await screen.findByRole("heading", { name: "Release radar" })).closest("section")!;
    expect(await within(shelf).findByLabelText(`Paper Moon by ${mock.fx.artists[0].title}`)).toBeInTheDocument();
    expect(within(shelf).getAllByText("New").length).toBeGreaterThan(0);
    await user.click(within(shelf).getByRole("button", { name: /Show all/ }));
    expect(useRouter.getState().entries[useRouter.getState().entries.length - 1].route).toEqual({ name: "releaseRadar" });
  });

  it("is hidden with the setting off, or with nothing found", async () => {
    await setUp("ready", { releaseRadar: false });
    const { unmount } = render(<HomePage />);
    await screen.findByTestId("home");
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByRole("heading", { name: "Release radar" })).toBeNull();
    unmount();
    await setUp("ready");
    mock.radar.setEmpty(true);
    render(<HomePage />);
    await screen.findByTestId("home");
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByRole("heading", { name: "Release radar" })).toBeNull();
  });
});

describe("the page", () => {
  it("says what's in the library, what can't be requested and why, and offers Request", async () => {
    await setUp("ready");
    render(<ReleaseRadarPage />);
    await screen.findByRole("heading", { level: 1, name: "Release radar" });
    const list = await screen.findByRole("list", { name: "New releases" });
    const [a0, , a2, a3] = mock.fx.artists;
    const own = mock.fx.albums.find((al) => al.artistId === a2.id)!;
    const inLib = within(list).getByLabelText(`${own.title} by ${a2.title}`);
    expect(within(inLib).getByText("In your library")).toBeInTheDocument();
    expect(within(inLib).queryByRole("button", { name: /^Request/ })).toBeNull();
    const build = [...mock.fx.albums].reverse().find((al) => al.artistId === a3.id)!;
    const blocked = within(list).getByLabelText(`${build.title} by ${a3.title}`);
    expect(within(blocked).getByText("Can't request")).toBeInTheDocument();
    expect(within(blocked).getByText(/an unreleased album already in your library/)).toBeInTheDocument();
    const paper = within(list).getByLabelText(`Paper Moon by ${a0.title}`);
    expect(within(paper).getByRole("button", { name: "Request Paper Moon" })).toBeInTheDocument();
    expect(within(paper).getByText(/^Album · /)).toBeInTheDocument();
    // An announced release is marked as coming.
    const soon = within(list).getByLabelText(`Glass Weather by ${a0.title}`);
    expect(within(soon).getByText(/^Album · Out /)).toBeInTheDocument();
    // The cover comes from the cover archive, through the art cache.
    expect(paper.querySelector(".card__art")).not.toBeNull();
    expect(screen.getByText(/from the 3 artists you follow and play most/)).toBeInTheDocument();
  });

  it("has no Request without a ready Lidarr", async () => {
    await setUp("demo");
    render(<ReleaseRadarPage />);
    const list = await screen.findByRole("list", { name: "New releases" });
    expect(within(list).queryByRole("button", { name: /^Request/ })).toBeNull();
    expect(within(list).getAllByText("In your library").length).toBe(1);
  });

  it("requests through the same dialog as anywhere, and shows where it stands", async () => {
    const user = userEvent.setup();
    await setUp("ready");
    mock.lidarr.setTypedOnce(true);
    render(<ReleaseRadarPage />);
    await user.click(await screen.findByRole("button", { name: "Request Paper Moon" }));
    const dialog = await screen.findByRole("dialog", { name: "Request Paper Moon" });
    await user.click(await within(dialog).findByRole("button", { name: "Request" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const card = screen.getByLabelText(`Paper Moon by ${mock.fx.artists[0].title}`);
    expect(await within(card).findByText("Looking on your indexers")).toBeInTheDocument();
  });

  it("filters by type and looks again on Check now", async () => {
    const user = userEvent.setup();
    await setUp("ready");
    const check = vi.spyOn(mock, "releaseRadarCheck");
    render(<ReleaseRadarPage />);
    const list = await screen.findByRole("list", { name: "New releases" });
    await user.click(screen.getByRole("button", { name: "EPs and singles" }));
    const kinds = within(screen.getByRole("list", { name: "New releases" }))
      .getAllByText(/^(Album|EP|Single) · /)
      .map((e) => e.textContent!.split(" · ")[0]);
    expect(kinds.length).toBeGreaterThan(0);
    expect(kinds.every((k) => k !== "Album")).toBe(true);
    expect(list).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Check now" }));
    expect(check).toHaveBeenCalled();
  });

  it("explains when it's off or nothing was found yet", async () => {
    await setUp("ready", { releaseRadar: false });
    const { unmount } = render(<ReleaseRadarPage />);
    expect(await screen.findByText("Release radar is off")).toBeInTheDocument();
    // Turn on, right there: the page fills in.
    await userEvent.click(screen.getByRole("button", { name: "Turn on" }));
    await waitFor(() => expect(useSettings.getState().settings?.releaseRadar).toBe(true));
    expect(await screen.findByRole("button", { name: "Check now" })).toBeInTheDocument();
    unmount();
    await setUp("ready");
    mock.radar.reset(true);
    render(<ReleaseRadarPage />);
    expect(await screen.findByText("No releases yet")).toBeInTheDocument();
  });
});

describe("following gets new albums", () => {
  const artist = () => mock.fx.artists[0];
  let lidarrStatus: MockInstance<typeof backend.lidarrStatus>;
  beforeEach(() => {
    lidarrStatus = vi.spyOn(backend, "lidarrStatus");
    try {
      localStorage.clear();
    } catch {
      // Not there in some runs.
    }
  });
  afterEach(() => lidarrStatus.mockRestore());

  async function settled() {
    await waitFor(() => expect(lidarrStatus).toHaveBeenCalled());
    await act(() => Promise.allSettled(lidarrStatus.mock.results.map((r) => r.value)));
  }

  async function openMenu(user: ReturnType<typeof userEvent.setup>) {
    await screen.findByRole("heading", { level: 1, name: artist().title });
    await settled();
    await user.click(screen.getByRole("button", { name: `More options for ${artist().title}` }));
    return screen.findByRole("menu");
  }

  it("follow turns the radar on, says so once, and the menu can just show their albums", async () => {
    const user = userEvent.setup();
    await setUp("ready", { releaseRadar: false });
    render(
      <>
        <ArtistPage id={artist().id} />
        <ContextMenuHost />
      </>,
    );
    await screen.findByRole("heading", { level: 1, name: artist().title });
    await settled();
    let menu = await openMenu(user);
    expect(within(menu).queryByRole("menuitemradio", { name: "Just show new albums" })).toBeNull();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: `Follow ${artist().title}` }));
    await waitFor(async () => expect((await mock.settingsGet()).releaseRadar).toBe(true));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Following. Their new albums will come in by themselves."));
    menu = await openMenu(user);
    const item = within(menu).getByRole("menuitemradio", { name: "Just show new albums" });
    expect(item).toHaveAttribute("aria-checked", "false");
    await user.click(item);
    await waitFor(async () => expect((await mock.followsList())[0].justShow).toBe(true));
    menu = await openMenu(user);
    expect(within(menu).getByRole("menuitemradio", { name: "Just show new albums" })).toHaveAttribute("aria-checked", "true");
    // The second follow doesn't say it again.
    useToasts.setState({ toasts: [] });
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: `Following ${artist().title}` }));
    await user.click(await screen.findByRole("button", { name: `Follow ${artist().title}` }));
    await waitFor(async () => expect((await mock.followsList()).length).toBe(1));
    expect(useToasts.getState().toasts.map((t) => t.message)).not.toContain("Following. Their new albums will come in by themselves.");
  });

  it("without a ready Lidarr, following is only a follow", async () => {
    const user = userEvent.setup();
    await setUp("demo", { releaseRadar: false });
    render(
      <>
        <ArtistPage id={artist().id} />
        <ContextMenuHost />
      </>,
    );
    await screen.findByRole("heading", { level: 1, name: artist().title });
    await settled();
    await user.click(screen.getByRole("button", { name: `Follow ${artist().title}` }));
    await waitFor(async () => expect((await mock.followsList()).length).toBe(1));
    expect((await mock.settingsGet()).releaseRadar).toBe(false);
    const menu = await openMenu(user);
    expect(within(menu).queryByRole("menuitemradio", { name: "Just show new albums" })).toBeNull();
  });
});

describe("settings", () => {
  it("has the radar and its notifications, and new albums from follows with their limit once Lidarr is ready", async () => {
    const user = userEvent.setup();
    await setUp("ready");
    render(<SettingsPage />);
    for (const k of ["releaseRadar", "followGet"]) {
      await waitFor(() => expect(document.querySelector(`[data-setting="${k}"]`)).not.toBeNull());
    }
    // Its notification is one of the Notifications choices.
    expect(screen.getByRole("checkbox", { name: "What the release radar finds" })).toBeChecked();
    const radar = document.querySelector('[data-setting="releaseRadar"]') as HTMLElement;
    await user.click(within(radar).getByRole("switch"));
    await waitFor(() => expect(useSettings.getState().settings?.releaseRadar).toBe(false));
    // Get them or just show them; the limit and singles sit under More.
    const row = document.querySelector('[data-setting="followGet"]') as HTMLElement;
    expect(within(row).getByRole("combobox")).toHaveTextContent("Get them");
    await mock.settingsSet({ followGet: false });
    expect((await mock.settingsGet()).followGet).toBe(false);
    await expect(mock.settingsSet({ autoRequestCap: 4 })).rejects.toBeTruthy();
    await mock.settingsSet({ autoRequestCap: 5 });
    expect((await mock.settingsGet()).autoRequestCap).toBe(5);
  });

  it("keeps new albums from follows out of sight until Lidarr is ready", async () => {
    await setUp("none");
    render(<SettingsPage />);
    await waitFor(() => expect(document.querySelector('[data-setting="releaseRadar"]')).not.toBeNull());
    expect(document.querySelector('[data-setting="followGet"]')).toBeNull();
  });

  it("marks automatic requests in the Requests list", async () => {
    await setUp("ready");
    mock.lidarr.requests = [
      {
        id: "r1",
        releaseGroupId: "x",
        title: "Paper Moon",
        artist: "A",
        artistId: null,
        state: "failed",
        destination: null,
        lidarrAlbumId: null,
        addedArtist: false,
        alreadyInLidarr: false,
        createdAt: 1,
        updatedAt: 1,
        message: "MusicBrainz lists this only as a bootleg or promo, not an official release.",
        notes: [],
        auto: true,
        protected: false,
      },
    ];
    render(<LidarrRequestsList />);
    expect(await screen.findByText("New from an artist you follow. Lidarr couldn't take it.")).toBeInTheDocument();
    expect(screen.getByText(/only as a bootleg or promo/)).toBeInTheDocument();
  });

  it("an imported request says what Lidarr got, and that a lossy copy gets replaced", async () => {
    mock.lidarr.requests = [
      {
        id: "g1",
        releaseGroupId: "y",
        title: "Panes",
        artist: "Glasshouse",
        artistId: null,
        state: "imported",
        destination: null,
        lidarrAlbumId: 9,
        addedArtist: false,
        alreadyInLidarr: false,
        createdAt: 1,
        updatedAt: 1,
        message: null,
        notes: [],
        auto: false,
        protected: false,
        grabbed: { line: "MP3 320, Newsletter Edit", upgradeNote: "Lidarr will replace it with FLAC when one turns up." },
      },
    ];
    render(<LidarrRequestsList />);
    expect((await screen.findByTestId("lidarr-grabbed")).textContent).toBe("MP3 320, Newsletter Edit. Lidarr will replace it with FLAC when one turns up.");
  });
});
