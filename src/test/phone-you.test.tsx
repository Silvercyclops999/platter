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
import { DESKTOP_ONLY, NOT_RESET, onPhone, phoneText } from "../pages/settingsMeta";
import { ASSUMED_CAPS, useCaps } from "../stores/caps";
import { usePhoneForm } from "../phone/form";
import { PhoneShell } from "../phone/PhoneShell";
import { PhoneServerPick, PhoneSignIn } from "../phone/PhoneSignIn";
import { phoneLookOf, phoneLookPatch } from "../phone/phoneSettings";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { installTapGuard } from "../phone/gestures";
import { holdLightStatusBar, lightTextHeld, mixIsDark, resetStatusBarHolds } from "../phone/statusBar";
import { albumPlate, plateText } from "../phone/bits";
import { playingFrom, shownDate, SleeveCard } from "../phone/NowPlaying";
import { PhoneAlbum, SONG_BY_SONG_OPEN_UP_TO } from "../phone/pages/Album";
import { SleeveNotesPanel } from "../pages/SleeveNotes";
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
  it("starts with Plex, waits with the code as a quiet line, and cancels back", async () => {
    await mock.authSignOut();
    act(() => useApp.getState().setAuth({ state: "signedOut", needsServerPick: false }));
    render(<PhoneSignIn />);
    expect(screen.getByTestId("phone-signin")).toHaveAttribute("data-step", "welcome");
    // Plex is the one big button; the other servers wait behind one quiet one.
    expect(screen.queryByRole("button", { name: "Use a Jellyfin server" })).toBeNull();
    expect(screen.getByRole("button", { name: "Use another server" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Plex" }));
    expect(await screen.findByText("K7QX")).toBeInTheDocument();
    expect(screen.getByText(/If plex.tv asks for a code/)).toBeInTheDocument();
    expect(screen.queryByText(/Step 2 of 2/)).toBeNull();
    expect(screen.getByTestId("phone-signin")).toHaveAttribute("data-step", "waiting");
    expect(screen.getByRole("status")).toHaveTextContent("Waiting for you to approve");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.getByTestId("phone-signin")).toHaveAttribute("data-step", "welcome"));
  });

  it("adds a Jellyfin server in its own screen, with a way back", async () => {
    render(<PhoneSignIn />);
    fireEvent.click(screen.getByRole("button", { name: "Use another server" }));
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
  it("shows the profile, this month's readouts, Wrapped, then Stats, Devices, Settings and Help only", async () => {
    act(() => useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } }));
    render(<PhoneShell />);
    fireEvent.click(within(screen.getByRole("navigation", { name: "Tabs" })).getByRole("button", { name: "You" }));
    const you = await screen.findByTestId("phone-you");
    expect(within(you).getByRole("heading", { level: 1 })).toHaveTextContent("listener");
    // This month as the profile page says it: one sentence, how long and mostly to whom.
    await waitFor(() => expect(within(you).getByTestId("you-month")).toHaveTextContent(/^\d[\d,]* (min|hrs?) this month, /));
    fireEvent.click(within(you).getByRole("button", { name: /Wrapped/ }));
    expect(useWrapped.getState().year).not.toBeNull();
    act(() => useWrapped.getState().close());
    const links = within(you).getByRole("list", { name: "You" });
    expect(within(links).getAllByRole("button").map((b) => b.textContent)).toEqual(["Stats", "Devices", "Settings", "Help"]);
    // Nothing else: no recent plays, tools or a second way into Settings.
    expect(within(you).queryByRole("heading", { name: "Recently played" })).toBeNull();
    expect(within(you).getAllByRole("button", { name: "Settings" })).toHaveLength(1);
    expect(within(you).queryByRole("button", { name: /Library doctor|Downloads|Bookmarks|Create a Blend/ })).toBeNull();
    fireEvent.click(within(links).getByRole("button", { name: "Settings" }));
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

  it("Go offline and Private session are quick tiles above the sections, each on and off in a tap", async () => {
    useRouter.getState().reset({ name: "settings" });
    render(<SettingsPage />);
    const quick = await screen.findByRole("group", { name: "Quick switches" });
    expect(quick.compareDocumentPosition(screen.getByRole("list", { name: "Settings sections" })) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(quick).getAllByRole("switch").map((t) => t.querySelector(".ph-quick__name")?.textContent)).toEqual(["Go offline", "Private session"]);
    const sw = await within(quick).findByRole("switch", { name: /Go offline/ });
    await waitFor(() => expect(sw).not.toBeDisabled());
    expect(sw).toHaveAttribute("aria-checked", "false");
    expect(sw).toHaveTextContent("Off");
    fireEvent.click(sw);
    await waitFor(() => expect(useSettings.getState().settings?.goOffline).toBe(true));
    await waitFor(() => expect(within(quick).getByRole("switch", { name: /Go offline/ })).toHaveClass("is-on"));
    expect(within(quick).getByRole("switch", { name: /Go offline/ })).toHaveTextContent("On");
    fireEvent.click(within(quick).getByRole("switch", { name: /Go offline/ }));
    await waitFor(() => expect(useSettings.getState().settings?.goOffline).toBe(false));
    fireEvent.click(within(quick).getByRole("switch", { name: /Private session/ }));
    await waitFor(() => expect(useSettings.getState().settings?.privateSession).toBe(true));
    fireEvent.click(within(quick).getByRole("switch", { name: /Private session/ }));
    await waitFor(() => expect(useSettings.getState().settings?.privateSession).toBe(false));
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
    // Nothing is set up, so every integration waits under More (open in tests).
    const more = await screen.findByRole("list", { name: "More pages in Integrations" });
    expect(within(more).queryByText("Discord")).not.toBeInTheDocument();
    expect(within(more).queryByText("Stream Deck")).not.toBeInTheDocument();
    expect(within(more).getByText("Lidarr")).toBeInTheDocument();
  });

  it("Listen on works from the phone: its name, being a Plex player, and what it lists", async () => {
    act(() => useCaps.setState({ all: ASSUMED_CAPS }));
    render(<SettingsPage section="Downloads and devices" />);
    await screen.findByRole("heading", { name: "Downloads and devices", level: 1 });
    for (const key of ["showDevices", "moveStopsOldDevice", "actAsPlexPlayer", "showToMyDevices", "computerName"]) expect(document.querySelector(`[data-setting="${key}"]`)).not.toBeNull();
    expect(screen.getByText("This phone's name")).toBeInTheDocument();
    expect(screen.getByText("Let other Plex apps play on this phone")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("This phone's name")).toBeInTheDocument();
    // Copying to USB drives and folders on disk stay on the computer.
    for (const key of ["devices", "deviceFormat", "downloadFolder"]) expect(document.querySelector(`[data-setting="${key}"]`)).toBeNull();
  });

  it("a server that can't take Plex players leaves that row out", async () => {
    act(() => useCaps.setState({ all: { ...ASSUMED_CAPS, caps: { ...ASSUMED_CAPS.caps, remotePlayers: false } } }));
    try {
      render(<SettingsPage section="Downloads and devices" />);
      await screen.findByRole("heading", { name: "Downloads and devices", level: 1 });
      expect(document.querySelector('[data-setting="actAsPlexPlayer"]')).toBeNull();
      expect(document.querySelector('[data-setting="computerName"]')).not.toBeNull();
    } finally {
      act(() => useCaps.setState({ all: ASSUMED_CAPS }));
    }
  });

  it("a reset on the computer keeps its name", () => {
    expect(NOT_RESET.has("computerName")).toBe(true);
  });

  it("the computer keeps every row", () => {
    expect(onPhone({ key: "closeToTray" })).toBe(false);
    expect(onPhone({ key: "crossfade" })).toBe(true);
    expect(onPhone({ key: "showToMyDevices" })).toBe(true);
    expect(onPhone({ key: "computerName" })).toBe(true);
    expect(onPhone({ key: "actAsPlexPlayer" })).toBe(true);
    expect(onPhone({ key: "devices", page: "devices" })).toBe(false);
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
    // On the phone a choice opens a page of its choices.
    fireEvent.click(within(row).getByRole("button", { name: /Look/ }));
    fireEvent.click(await screen.findByRole("radio", { name: "Sleeve" }));
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

describe("the dock and the status bar", () => {
  it("with the mini player up the dock is one opaque strip down to the bottom, and takes its own taps", () => {
    expect(phoneCss).toMatch(/\.phone\.has-mini \.ph-dock:not\(\.is-mini-away\) \{[^}]*background-color: var\(--color-bg-base\);[^}]*pointer-events: auto;/);
  });

  it("light text over a dark cover colour, dark text over a light one", () => {
    expect(mixIsDark("#1a1410", 0.46)).toBe(false);
    expect(mixIsDark("#1a1410", 0.9)).toBe(true);
    expect(mixIsDark("#f2e6c8", 0.9)).toBe(false);
    expect(mixIsDark("#101010", 0.46, "#000000")).toBe(true);
    expect(mixIsDark(null, 1)).toBe(false);
  });

  it("holds stack, and each lets go once", () => {
    resetStatusBarHolds();
    const a = holdLightStatusBar();
    const b = holdLightStatusBar();
    a();
    a();
    expect(lightTextHeld()).toBe(true);
    b();
    expect(lightTextHeld()).toBe(false);
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
    // A short list first; the rest behind More.
    expect(within(sheet).getAllByRole("button").filter((b) => b.classList.contains("ph-row--action")).map((b) => b.textContent)).toEqual([
      "Add to Liked Songs",
      "Add to playlist",
      "Add to queue",
      "Go to album",
      "Go to artist",
      "More",
    ]);
    fireEvent.click(within(sheet).getByRole("button", { name: "More" }));
    expect(within(sheet).getByRole("button", { name: "Lyrics" })).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Credits" })).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Share" })).toBeInTheDocument();
    expect(sheet.querySelectorAll(".ph-row__icon").length).toBeGreaterThan(6);
  });
});

describe("what the phone says", () => {
  it("format plates only for lossless files", () => {
    expect(plateText({ codec: "mp3", bitDepth: null, sampleRate: 44100, bitrate: 320 })).toBeNull();
    const flac = { codec: "flac", bitDepth: 16, sampleRate: 44100, bitrate: 900 };
    expect(albumPlate([flac, flac])).toBe(plateText(flac));
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

describe("the phone's other pages", () => {
  it("open as phone pages; Devices points at downloads, not drives", async () => {
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "recents" }));
    await waitFor(() => expect(document.querySelector('[data-phone-page="recents"]')).not.toBeNull());
    act(() => useRouter.getState().navigate({ name: "devices" }));
    expect(await screen.findByRole("button", { name: "Downloads on this phone" })).toBeInTheDocument();
    expect(screen.queryByText(/USB drive/)).toBeNull();
  });
});

describe("help on the phone", () => {
  it("leaves out the pages about things only the computer has, in the list and in search", async () => {
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "help" }));
    const list = within(await screen.findByRole("list", { name: "Help pages" }));
    expect(list.getByText("Getting started")).toBeInTheDocument();
    for (const t of ["Layouts", "Tag editor", "Stream Deck"]) expect(list.queryByText(t)).toBeNull();
    fireEvent.change(screen.getByPlaceholderText("Search help"), { target: { value: "stream deck" } });
    await waitFor(() => expect(screen.queryByText(/^Stream Deck/)).toBeNull());
  });
});

describe("sleeve notes on the phone", () => {
  const withNotes = () => mock.fx.albums.find((a) => (Number(a.id) || a.title.length) % 11 !== 10 && a.section !== "demos")!;

  it("are the computer's printed sleeve, compact, under its own heading", async () => {
    const album = withNotes();
    render(<SleeveCard track={{ albumId: album.id, albumTitle: album.title, artistTitle: album.artistTitle, thumb: album.thumb }} />);
    const panel = await screen.findByRole("region", { name: "Sleeve notes" });
    expect(panel).toHaveClass("sleeve__liner", "sleeve__liner--compact");
    expect(panel.querySelector(".sleeve-facts__row dt")?.textContent).toBe("Released");
    expect(within(panel).getByRole("button", { name: "Open the album" })).toBeInTheDocument();
  });

  it("print every fact the computer's release side prints, full dates included", async () => {
    await mock.settingsSet({ ...DEFAULT_SETTINGS, infoMusicbrainz: true });
    await loadSettings();
    const album = withNotes();
    const desk = render(<SleeveNotesPanel albumId={album.id} title={album.title} />);
    const deskPanel = await screen.findByRole("region", { name: "The release" });
    const labels = (el: HTMLElement) => [...el.querySelectorAll(".sleeve-facts__row")].map((r) => `${r.querySelector("dt")?.textContent}=${r.querySelector(".sleeve-facts__value")?.textContent}`);
    const want = labels(deskPanel);
    expect(want.length).toBeGreaterThan(4);
    desk.unmount();
    render(<SleeveCard track={{ albumId: album.id, albumTitle: album.title, artistTitle: album.artistTitle, thumb: album.thumb }} />);
    const panel = await screen.findByRole("region", { name: "Sleeve notes" });
    expect(labels(panel)).toEqual(want);
  });

  it("sit under the right heading level: h3 among now playing's cards, h2 under the album page's h1", async () => {
    const album = withNotes();
    const t = { albumId: album.id, albumTitle: album.title, artistTitle: album.artistTitle, thumb: album.thumb };
    const { unmount } = render(<SleeveCard track={t} />);
    expect(await screen.findByRole("heading", { name: "Sleeve notes", level: 3 })).toBeInTheDocument();
    unmount();
    render(<SleeveCard track={t} link={false} level={2} />);
    expect(await screen.findByRole("heading", { name: "Sleeve notes", level: 2 })).toBeInTheDocument();
    // "Open the album" is a full 44 pt target.
    expect(phoneCss).toMatch(/\.ph-card__link \{[^}]*min-width: 44px;[^}]*min-height: 44px;/);
  });

  it("the album page keeps Song by song, folded behind its own row when it's long", async () => {
    await mock.settingsSet({ ...DEFAULT_SETTINGS, infoMusicbrainz: true });
    await loadSettings();
    const album = withNotes();
    render(<PhoneAlbum id={album.id} />);
    const credits = await screen.findByRole("region", { name: "Credits" }, { timeout: 4000 });
    const toggle = await within(credits).findByRole("button", { name: /Song by song/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(credits.querySelector(".sleeve-songs")).toBeNull();
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(credits.querySelectorAll(".sleeve-song").length).toBeGreaterThan(SONG_BY_SONG_OPEN_UP_TO);
  });

  it("the album page sets its credits like the computer's, a role and its names", async () => {
    const album = withNotes();
    render(<PhoneAlbum id={album.id} />);
    const credits = await screen.findByRole("region", { name: "Credits" }, { timeout: 4000 });
    expect(credits).toHaveClass("sleeve__credits");
    expect(credits.querySelector(".sleeve-roles__row dt")).not.toBeNull();
    expect(credits.querySelector(".ph-pill")).toBeNull();
  });
});
