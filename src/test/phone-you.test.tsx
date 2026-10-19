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
import { CRATES_NOTE } from "../phone/pages/Crates";
import { PhoneShell } from "../phone/PhoneShell";
import { PhoneServerPick, PhoneSignIn } from "../phone/PhoneSignIn";
import { phoneLookOf, phoneLookPatch } from "../phone/phoneSettings";
import { phone, usePhoneUi } from "../phone/phoneUi";
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
  it("are a type pill with a one-line note the first time only", async () => {
    render(<PhoneShell />);
    act(() => phone.tab("library"));
    const pill = await screen.findByRole("button", { name: "Crates", pressed: false });
    fireEvent.click(pill);
    expect(await screen.findByTestId("crates-note")).toHaveTextContent(CRATES_NOTE);
    expect(screen.getByRole("tablist", { name: "Sort records by" })).toBeInTheDocument();
    // Another type turns crates off; coming back, the note has been seen.
    fireEvent.click(screen.getByRole("button", { name: "Albums" }));
    expect(screen.queryByTestId("crates-note")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Crates" }));
    expect(await screen.findByRole("tablist", { name: "Sort records by" })).toBeInTheDocument();
    expect(screen.queryByTestId("crates-note")).not.toBeInTheDocument();
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
  it("Home starts with the playing song as a big card that opens the player", async () => {
    play(mock.fx.tracks[0]);
    render(<PhoneShell />);
    const card = await screen.findByTestId("home-now");
    expect(within(card).getByText(mock.fx.tracks[0].title)).toBeInTheDocument();
    expect(within(card).getByText("Paused")).toBeInTheDocument();
    fireEvent.click(within(card).getByRole("button", { name: /Open the player/ }));
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
