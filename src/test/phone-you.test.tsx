/**
 * The phone's sign-in and server pick, the You tab, Settings in phone-sized
 * screens (desktop-only rows left out, the Look row), crates in Library with
 * their one-time note, Home's now-playing card, discs on album pages, the
 * cover-first player and the safe areas.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type PlayerState, type QueueItem, type Track } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { DESKTOP_ONLY, onPhone, phoneText } from "../pages/settingsMeta";
import { usePhoneForm } from "../phone/form";
import { PhoneShell } from "../phone/PhoneShell";
import { PhoneServerPick, PhoneSignIn } from "../phone/PhoneSignIn";
import { phoneLookOf, phoneLookPatch } from "../phone/phoneSettings";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { installTapGuard } from "../phone/gestures";
import { albumPlate, plateText } from "../phone/bits";
import { playingFrom, shownDate } from "../phone/NowPlaying";
import { inLibraryLine } from "../phone/pages/Artist";
import { ofKind } from "../phone/pages/Search";
import { latestOnce } from "../phone/pages/You";
import phoneCss from "../phone/phone.css?raw";
import indexHtml from "../../index.html?raw";
import { useApp } from "../stores/app";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, useSettings } from "../stores/settings";
import { useWrapped } from "../stores/wrapped";

const mock = backend as MockBackend;
const lastRoute = () => {
  const e = useRouter.getState().entries;
  return e[e.length - 1]?.route;
};
const item = (t: Track, i: number): QueueItem => ({ itemId: `q${i}`, track: t, source: "context" });

function play(t: Track, status: PlayerState["status"] = "paused") {
  const s: PlayerState = {
    status,
    track: t,
    positionMs: 30_000,
    durationMs: t.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: item(t, 0), userQueue: [], nextUp: [], history: [], context: { kind: "album", id: t.albumId, title: t.albumTitle } },
  };
  act(() => usePlayer.setState({ state: s, local: s, positionMs: s.positionMs, durationMs: s.durationMs, remote: null }));
}

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  usePhoneUi.setState({ tab: "home", nowPlaying: false, lyrics: false, sheets: [], crates: false, homeCard: false });
  useRouter.getState().reset({ name: "home" });
  try {
    localStorage.clear();
  } catch {
    // No storage in this run.
  }
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
  act(() => usePlayer.setState({ state: null, local: null, positionMs: 0, durationMs: 0, remote: null }));
  vi.restoreAllMocks();
});

describe("phone sign-in", () => {
  it("starts with Plex, waits with the code, and cancels back", async () => {
    await mock.authSignOut();
    act(() => useApp.getState().setAuth({ state: "signedOut", needsServerPick: false }));
    render(<PhoneSignIn />);
    expect(screen.getByTestId("phone-signin")).toHaveAttribute("data-step", "welcome");
    expect(screen.getByRole("button", { name: "Use a Jellyfin server" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Plex" }));
    expect(await screen.findByText("K7QX")).toBeInTheDocument();
    expect(screen.getByTestId("phone-signin")).toHaveAttribute("data-step", "waiting");
    expect(screen.getByRole("status")).toHaveTextContent("Waiting for you to approve");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.getByTestId("phone-signin")).toHaveAttribute("data-step", "welcome"));
  });

  it("adds a Jellyfin server in its own screen, with a way back", async () => {
    render(<PhoneSignIn />);
    fireEvent.click(screen.getByRole("button", { name: "Use a Jellyfin server" }));
    expect(screen.getByRole("heading", { name: "Add a Jellyfin server" })).toBeInTheDocument();
    expect(screen.getByLabelText(/server address/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(screen.getByTestId("phone-signin")).toHaveAttribute("data-step", "welcome");
  });

  it("shows the error when plex.tv can't be reached", async () => {
    vi.spyOn(backend, "authStart").mockRejectedValueOnce({ kind: "network", message: "Couldn't reach plex.tv" });
    render(<PhoneSignIn />);
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Plex" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't reach plex.tv");
  });

  it("picks a server: offline ones can't be picked, an online one connects", async () => {
    const select = vi.spyOn(backend, "serverSelect");
    render(<PhoneServerPick />);
    const list = await screen.findByRole("list", { name: "Servers" });
    const rows = await within(list).findAllByRole("button");
    expect(rows).toHaveLength(2);
    const offline = rows.find((r) => r.textContent?.includes("Offline"))!;
    expect(offline).toBeDisabled();
    const online = rows.find((r) => r.textContent?.includes("Online"))!;
    fireEvent.click(online);
    expect(select).toHaveBeenCalledWith("mock-machine-01");
    expect(await within(online).findByText("Connecting")).toBeInTheDocument();
  });
});

describe("the You tab", () => {
  it("shows the profile, this month's readouts, Wrapped and recent plays", async () => {
    act(() => useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } }));
    render(<PhoneShell />);
    fireEvent.click(within(screen.getByRole("navigation", { name: "Tabs" })).getByRole("button", { name: "You" }));
    const you = await screen.findByTestId("phone-you");
    expect(within(you).getByRole("heading", { level: 1 })).toHaveTextContent("listener");
    const numbers = await within(you).findByTestId("you-numbers");
    expect(within(numbers).getByText("Minutes")).toBeInTheDocument();
    expect(within(numbers).getByText("Plays")).toBeInTheDocument();
    fireEvent.click(within(you).getByRole("button", { name: /Wrapped/ }));
    expect(useWrapped.getState().year).not.toBeNull();
    act(() => useWrapped.getState().close());
    expect(within(you).getByRole("heading", { name: "Recently played" })).toBeInTheDocument();
    fireEvent.click(within(you).getAllByRole("button", { name: "Settings" })[0]);
    expect(lastRoute()).toEqual({ name: "settings" });
  });

  it("lists each record once, at its latest play", () => {
    const a = mock.fx.albums[0];
    const albumItem = { kind: "album" as const, album: a };
    const list = [
      { kind: "context" as const, item: albumItem, at: 3_000_000, tracks: [] },
      { kind: "liked" as const, at: 2_000_000, tracks: [] },
      { kind: "context" as const, item: albumItem, at: 1_000_000, tracks: [] },
      { kind: "liked" as const, at: 500_000, tracks: [] },
    ];
    expect(latestOnce(list).map((r) => r.title)).toEqual([a.title, "Liked Songs"]);
  });
});

describe("phone settings screens", () => {
  it("lists the sections with search on top, and a section opens as its own page", async () => {
    useRouter.getState().reset({ name: "settings" });
    render(<SettingsPage />);
    expect(screen.getByRole("searchbox")).toHaveAttribute("placeholder", "Search settings");
    const list = screen.getByRole("list", { name: "Settings sections" });
    fireEvent.click(within(list).getByRole("button", { name: /Playback/ }));
    expect(lastRoute()).toEqual({ name: "settings", section: "Playback" });
  });

  it("search finds rows from every section", async () => {
    render(<SettingsPage />);
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "crossfade" } });
    expect(await screen.findByText("Crossfade songs")).toBeInTheDocument();
  });

  it("leaves out rows for the tray, the window, keys, the miniplayer, layouts, Discord and Stream Deck", async () => {
    render(<SettingsPage section="App" />);
    await screen.findByRole("heading", { name: "App", level: 1 });
    for (const key of ["closeToTray", "launchAtLogin", "keyboardShortcuts", "mediaKeys"]) expect(document.querySelector(`[data-setting="${key}"]`)).toBeNull();
    expect(document.querySelector('[data-setting="notifications"]')).not.toBeNull();
  });

  it("Appearance has the phone's Look first, and no layout or miniplayer rows", async () => {
    render(<SettingsPage section="Appearance" />);
    await screen.findByRole("heading", { name: "Appearance", level: 1 });
    const rows = [...document.querySelectorAll("[data-setting]")].map((r) => r.getAttribute("data-setting"));
    expect(rows[0]).toBe("phoneLook");
    for (const key of ["layout", "miniplayerOnTop", "theme", "followSystem", "recordMotion"]) expect(rows).not.toContain(key);
  });

  it("integrations that need a computer have no page on the phone", async () => {
    render(<SettingsPage section="Integrations" />);
    const more = await screen.findByRole("list", { name: "More in Integrations" });
    expect(within(more).queryByText("Discord")).not.toBeInTheDocument();
    expect(within(more).queryByText("Stream Deck")).not.toBeInTheDocument();
    expect(within(more).getByText("Lidarr")).toBeInTheDocument();
  });

  it("the computer keeps every row", () => {
    expect(onPhone({ key: "closeToTray" })).toBe(false);
    expect(onPhone({ key: "crossfade" })).toBe(true);
    expect(onPhone({ key: "showToMyDevices" })).toBe(true);
    expect(onPhone({ key: "discordStatus", page: "discord" })).toBe(false);
    expect(DESKTOP_ONLY.has("layout")).toBe(true);
  });

  it("says phone where a row says computer", () => {
    expect(phoneText("Only music on this computer plays.")).toBe("Only music on this phone plays.");
    expect(phoneText("Your computer's name")).toBe("Your phone's name");
  });

  it("Look: Match device follows the phone, the others pick one look", async () => {
    expect(phoneLookOf({ theme: "studio", followSystem: true, themeFile: null })).toBe("device");
    expect(phoneLookOf({ theme: "sleeve", followSystem: false, themeFile: null })).toBe("sleeve");
    expect(phoneLookOf({ theme: "dark", followSystem: false, themeFile: null })).toBe("other");
    expect(phoneLookPatch("device", { theme: "sleeve" })).toEqual({ theme: "studio", followSystem: true, themeFile: null });
    render(<SettingsPage section="Appearance" />);
    const row = (await screen.findByText("Look")).closest("[data-setting]") as HTMLElement;
    fireEvent.change(within(row).getByRole("combobox"), { target: { value: "sleeve" } });
    await waitFor(() => expect(useSettings.getState().settings?.theme).toBe("sleeve"));
    expect(useSettings.getState().settings?.followSystem).toBe(false);
  });

  it("a new phone matches the device", () => {
    expect(phoneLookOf(DEFAULT_SETTINGS)).toBe("device");
  });
});

describe("crates in Library", () => {
  it("stay off the phone's Library for now, even with Show crates on", async () => {
    render(<PhoneShell />);
    act(() => phone.tab("library"));
    await screen.findByRole("heading", { name: "Library" });
    expect(screen.getByRole("button", { name: "Albums" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Crates" })).not.toBeInTheDocument();
  });

  it("hide with Show crates off", async () => {
    await backend.settingsSet({ showCrates: false });
    await loadSettings();
    render(<PhoneShell />);
    act(() => phone.tab("library"));
    await screen.findByRole("heading", { name: "Library" });
    expect(screen.queryByRole("button", { name: "Crates" })).not.toBeInTheDocument();
  });

  it("a crate's page lights the Library tab", async () => {
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "crates" }));
    await waitFor(() => expect(usePhoneUi.getState().tab).toBe("library"));
  });
});

describe("Home and now playing", () => {
  it("the player lives at the bottom: Home has no playing card, the mini player opens the player", async () => {
    play(mock.fx.tracks[0]);
    render(<PhoneShell />);
    const mini = await screen.findByRole("button", { name: new RegExp(`Now playing: ${mock.fx.tracks[0].title}.*Open the player`) });
    expect(screen.queryByTestId("home-now")).toBeNull();
    fireEvent.click(mini);
    expect(usePhoneUi.getState().nowPlaying).toBe(true);
    const np = await screen.findByRole("dialog", { name: "Now playing" });
    // Cover first: no record or tonearm drawn on the phone.
    expect(np.querySelector(".ph-np__cover")).not.toBeNull();
    expect(np.querySelector(".turntable")).toBeNull();
  });

  it("an album on two discs shows Disc 1 and Disc 2", async () => {
    const album = mock.fx.albums.find((a) => (mock.fx.tracksByAlbum.get(a.id) ?? []).some((t) => t.discNumber === 2))!;
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "album", id: album.id }));
    expect(await screen.findByRole("region", { name: "Disc 1" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Disc 2" })).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "Disc 2 songs" })).toBeInTheDocument();
  });
});

describe("safe areas", () => {
  it("the page asks for the whole screen and the phone CSS keeps clear of the notch and home indicator", () => {
    expect(indexHtml).toMatch(/viewport-fit=cover/);
    expect(phoneCss).toMatch(/env\(safe-area-inset-top/);
    expect(phoneCss).toMatch(/env\(safe-area-inset-bottom/);
    // Tabs and the mini player sit above the home indicator.
    expect(phoneCss).toMatch(/\.ph-tabs \{[^}]*padding-bottom: var\(--ph-safe-bottom\)/);
  });
});

describe("touch and sheets", () => {
  it("a press that moved is a swipe, not a tap", () => {
    const root = document.createElement("div");
    const button = document.createElement("button");
    root.appendChild(button);
    document.body.appendChild(root);
    const clicked = vi.fn();
    button.addEventListener("click", clicked);
    const off = installTapGuard(root);
    const press = (dx: number) => {
      button.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 10, clientY: 10, pointerId: 1 }));
      button.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, clientX: 10 + dx, clientY: 10, pointerId: 1 }));
      button.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, clientX: 10 + dx, clientY: 10, pointerId: 1 }));
      button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    };
    press(40);
    expect(clicked).not.toHaveBeenCalled();
    press(3);
    expect(clicked).toHaveBeenCalledTimes(1);
    off();
    root.remove();
  });

  it("tapping the dim area closes every sheet; the sleep timer's goes back to the queue", async () => {
    render(<PhoneShell />);
    act(() => phone.push({ kind: "queue" }));
    act(() => phone.push({ kind: "librarySort" }));
    fireEvent.click(screen.getAllByTestId("sheet-dim").slice(-1)[0]);
    expect(usePhoneUi.getState().sheets).toEqual([]);
    act(() => phone.push({ kind: "queue" }));
    act(() => phone.push({ kind: "sleep" }));
    fireEvent.click(screen.getAllByTestId("sheet-dim").slice(-1)[0]);
    expect(usePhoneUi.getState().sheets.map((s) => s.kind)).toEqual(["queue"]);
  });

  it("Add to playlist lists your own playlists", async () => {
    const t = mock.fx.tracks[0];
    render(<PhoneShell />);
    act(() => phone.push({ kind: "addTo", track: t }));
    const sheet = await screen.findByRole("dialog", { name: "Add to playlist" });
    const own = (await backend.playlists()).filter((p) => !p.smart);
    expect(await within(sheet).findByText(own[0].title)).toBeInTheDocument();
  });

  it("the song sheet has an icon for each action, and lyrics for the playing song", async () => {
    const t = mock.fx.tracks[0];
    play(t);
    render(<PhoneShell />);
    act(() => phone.push({ kind: "track", track: t }));
    const sheet = await screen.findByRole("dialog", { name: t.title });
    expect(within(sheet).getByRole("button", { name: "Lyrics" })).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Credits" })).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Share" })).toBeInTheDocument();
    expect(sheet.querySelectorAll(".ph-row__icon").length).toBeGreaterThan(6);
  });
});

describe("what the phone says", () => {
  it("format plates only for lossless files or downloads", () => {
    expect(plateText({ codec: "mp3", bitDepth: null, sampleRate: 44100, bitrate: 320 }, false)).toBeNull();
    expect(plateText({ codec: "mp3", bitDepth: null, sampleRate: 44100, bitrate: 320 }, true)).toBe("Downloaded");
    const flac = { codec: "flac", bitDepth: 16, sampleRate: 44100, bitrate: 900 };
    expect(albumPlate([flac, flac])).toBe(plateText(flac, false));
    expect(albumPlate([flac, { ...flac, codec: "mp3" }])).toBeNull();
  });

  it("a 1 January date shows as its year", () => {
    expect(shownDate("2007-01-01")).toBe("2007");
    expect(shownDate("2007-03-03")).toMatch(/2007/);
    expect(shownDate("2007-03-03")).not.toBe("2007-03-03");
  });

  it("Playing from follows the queue past the end of the record", () => {
    const ctx = { kind: "album", title: "Night Ferry" };
    expect(playingFrom(ctx)).toBe("Night Ferry");
    expect(playingFrom(ctx, "autoplay")).toBe("Autoplay after Night Ferry");
    expect(playingFrom(ctx, "user")).toBe("Your queue");
  });

  it("an artist's song count the server didn't give stays out", () => {
    expect(inLibraryLine(41, 0)).toBe("In your library: 41 albums");
    expect(inLibraryLine(1, 1)).toBe("In your library: 1 album, 1 song");
  });

  it("search narrows to one type of result", () => {
    const hits = [{ kind: "artist" as const, artist: mock.fx.artists[0] }, { kind: "album" as const, album: mock.fx.albums[0] }];
    expect(ofKind(hits, "all")).toHaveLength(2);
    expect(ofKind(hits, "album")).toEqual([hits[1]]);
  });

  it("the phone's settings rows never send you to the computer", () => {
    expect(onPhone({ key: "updates" })).toBe(false);
    expect(onPhone({ key: "outputDevice" })).toBe(false);
  });
});

describe("pages the phone borrows", () => {
  it("a page with no phone version opens in one column; file and drive tools say they're on the computer", async () => {
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "recents" }));
    await waitFor(() => expect(document.querySelector('.ph-desk[data-route="recents"]')).not.toBeNull());
    act(() => useRouter.getState().navigate({ name: "devices" }));
    expect(await screen.findByText("On your computer")).toBeInTheDocument();
  });
});
