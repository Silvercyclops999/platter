import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../App";
import { QualityBadge } from "../components/Badge";
import { backend } from "../lib/backend";
import { NEUTRAL_HEADER } from "../lib/color";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { useArtTint } from "../lib/useTint";
import { NOTICES, SettingsPage, SOURCE_NOTICES } from "../pages/SettingsPage";
import { MainView } from "../shell/MainView";
import { PlayerBar } from "../shell/PlayerBar";
import { TopBar } from "../shell/TopBar";
import { useShortcuts } from "../shell/useShortcuts";
import { useApp } from "../stores/app";
import { useRouter } from "../stores/router";
import { loadSettings, updateSettings, useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;
const route = () => useRouter.getState().entries[useRouter.getState().index].route;

function Host() {
  useShortcuts();
  return (
    <>
      <TopBar />
      <MainView />
    </>
  );
}

const SIGNED_IN = {
  state: "signedIn" as const,
  username: "listener",
  needsServerPick: false,
  server: { machineId: "mock-machine-01", name: "Basement Server", owned: true, online: true, connection: "local" as const },
};

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  useApp.setState({ auth: SIGNED_IN, health: null, sync: { phase: "idle", done: 0, total: 0 } });
  useToasts.setState({ toasts: [] });
  await loadSettings();
});
afterEach(() => vi.restoreAllMocks());

const row = (title: string | RegExp) => {
  const el = screen.getByText(title, { selector: ".setting-row__title" }).closest(".setting-row");
  if (!el) throw new Error(`no row ${title}`);
  return within(el as HTMLElement);
};

describe("settings page", () => {
  it("shows every section and row with its default", async () => {
    render(<SettingsPage />);
    const headings = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    expect(headings).toEqual(["Account", "Audio quality", "Playback", "Library", "Storage", "Downloads and offline", "Search", "Appearance", "Home", "Mixes", "App", "Integrations", "Transitions", "Loudness", "Equalizer", "Sharing", "About", "Now playing"]);

    expect(screen.getByRole("combobox", { name: "Streaming quality at home" })).toHaveValue("original");
    expect(screen.getByRole("combobox", { name: "Streaming quality away from home" })).toHaveValue("320");
    expect(screen.getByRole("switch", { name: "Show format badge" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("combobox", { name: "Shuffle style" })).toHaveValue("spread");
    expect(screen.getByRole("switch", { name: "Albums and playlists resume where you left off" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("spinbutton", { name: "Pre-cache ahead" })).toHaveValue(5);
    expect(screen.getByRole("switch", { name: "Report plays to Plex" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("switch", { name: "Private session" })).toHaveAttribute("aria-checked", "false");
    expect(screen.getByRole("combobox", { name: "Artist page Popular" })).toHaveValue("global");
    expect(screen.getByRole("combobox", { name: "Artwork and cache size" })).toHaveValue("2048");
    expect(screen.getByRole("switch", { name: "Artwork colours" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("switch", { name: "Show suggestions as you type" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("switch", { name: "Remember recent searches" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("button", { name: "Clear recent searches" })).toBeEnabled();
    expect(screen.getByRole("combobox", { name: "Reduce motion" })).toHaveValue("system");
    expect(screen.getByRole("switch", { name: "Show the shortcut grid" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("switch", { name: "Colour wash from artwork" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("button", { name: "Resync library" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Check for updates" })).toBeEnabled();
    expect(screen.getByRole("combobox", { name: "Download quality" })).toHaveValue("original");
    expect(screen.getByRole("combobox", { name: "Download storage limit" })).toHaveValue("20");
    expect(screen.getByRole("switch", { name: "Download only on your home network" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("switch", { name: "Download Liked Songs" })).toHaveAttribute("aria-checked", "false");
    expect(screen.getByRole("switch", { name: "Go offline" })).toHaveAttribute("aria-checked", "false");
    expect(screen.getByRole("button", { name: "Change location" })).toBeEnabled();
    expect(screen.getByRole("combobox", { name: "When the queue ends" })).toHaveValue("keepPlaying");
    expect(screen.getByRole("switch", { name: "Smart shuffle" })).toHaveAttribute("aria-checked", "false");
    expect(screen.getByRole("switch", { name: "Show made-for-you mixes on Home" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("combobox", { name: "Variety" })).toHaveValue("balanced");
    expect(screen.getByRole("switch", { name: "Include unreleased music" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("combobox", { name: "Refresh the daylist" })).toHaveValue("3");
    expect(screen.getByRole("combobox", { name: "Leave out of mixes" })).toHaveValue("");

    // Account, version and art cache usage arrive from the core.
    expect(screen.getByText("Signed in as listener")).toBeInTheDocument();
    expect(screen.getByText("Basement Server, Local connection")).toBeInTheDocument();
    expect(await screen.findByText("You have version 0.1.0.")).toBeInTheDocument();
    expect(await screen.findByText("Using 412 MB.")).toBeInTheDocument();
    expect(screen.getByText(/GPL-3.0-or-later/)).toBeInTheDocument();
    expect(screen.getByText(/A music player for Plex\./)).toBeInTheDocument();
    expect(screen.getByText("Not affiliated with or endorsed by Plex, Inc.")).toBeInTheDocument();
  });

  it("mix settings save, and what's left out of mixes can be added and removed", async () => {
    const user = userEvent.setup();
    const set = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    await user.selectOptions(screen.getByRole("combobox", { name: "Variety" }), "adventurous");
    expect(set).toHaveBeenCalledWith({ mixVariety: "adventurous" });
    await user.selectOptions(screen.getByRole("combobox", { name: "Refresh the daylist" }), "6");
    expect(set).toHaveBeenCalledWith({ daylistHours: 6 });
    await user.selectOptions(screen.getByRole("combobox", { name: "When the queue ends" }), "stop");
    expect(set).toHaveBeenCalledWith({ queueEnd: "stop" });
    await user.click(screen.getByRole("switch", { name: "Smart shuffle" }));
    expect(set).toHaveBeenCalledWith({ smartShuffle: true });
    await user.type(screen.getByRole("combobox", { name: "Leave out of mixes" }), "Polka");
    await user.click(screen.getByRole("button", { name: "Leave out" }));
    expect(set).toHaveBeenCalledWith({ mixExcludeGenres: ["Polka"] });
    await user.click(await screen.findByRole("button", { name: "Include Polka again" }));
    expect(set).toHaveBeenCalledWith({ mixExcludeGenres: [] });
  });

  it("app rows for the system media controls, tray and launch at login", async () => {
    const user = userEvent.setup();
    const set = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    const session = screen.getByRole("switch", { name: "Show in the system media controls" });
    const keys = screen.getByRole("switch", { name: "Media keys" });
    const tray = screen.getByRole("switch", { name: /^Close to the (tray|menu bar)$/ });
    const login = screen.getByRole("switch", { name: "Open Platter when you sign in" });
    expect(session).toHaveAttribute("aria-checked", "true");
    expect(keys).toHaveAttribute("aria-checked", "true");
    expect(tray).toHaveAttribute("aria-checked", "false");
    expect(login).toHaveAttribute("aria-checked", "false");
    expect(keys).toHaveAccessibleDescription(/your computer sends these keys there/);

    await user.click(session);
    expect(set).toHaveBeenCalledWith({ mediaSession: false });
    expect(keys).not.toHaveAccessibleDescription(/your computer sends these keys there/);
    await user.click(keys);
    expect(set).toHaveBeenCalledWith({ mediaKeys: false });
    await user.click(tray);
    expect(set).toHaveBeenCalledWith({ closeToTray: true });
    await user.click(login);
    expect(set).toHaveBeenCalledWith({ launchAtLogin: true });

    useSettings.setState({ settings: null });
    await loadSettings();
    expect(useSettings.getState().settings).toMatchObject({
      mediaSession: false,
      mediaKeys: false,
      closeToTray: true,
      launchAtLogin: true,
    });
  });

  it("finds the app rows by search", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.type(screen.getByRole("searchbox", { name: "Search settings" }), "lock screen");
    expect(screen.getByRole("switch", { name: "Show in the system media controls" })).toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "Media keys" })).not.toBeInTheDocument();
  });

  it("each row has a description for screen readers", () => {
    render(<SettingsPage />);
    const sw = screen.getByRole("switch", { name: "Artwork colours" });
    expect(sw).toHaveAccessibleDescription(/take their colour from the artwork/);
    const sel = screen.getByRole("combobox", { name: "Streaming quality at home" });
    expect(sel).toHaveAccessibleDescription(/home network/);
  });

  it("toggles persist through the core", async () => {
    const user = userEvent.setup();
    const set = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    const priv = screen.getByRole("switch", { name: "Private session" });
    const report = screen.getByRole("switch", { name: "Report plays to Plex" });
    await user.click(priv);
    expect(priv).toHaveAttribute("aria-checked", "true");
    expect(set).toHaveBeenCalledWith({ privateSession: true });
    await user.click(report);
    expect(set).toHaveBeenCalledWith({ reportToPlex: false });
    await user.click(screen.getByRole("switch", { name: "Show format badge" }));
    expect(set).toHaveBeenCalledWith({ showFormatBadge: false });
    const avatar = screen.getByRole("switch", { name: "Show my Plex profile picture" });
    expect(avatar).toHaveAttribute("aria-checked", "true");
    expect(avatar).toHaveAccessibleDescription(/This device only/);
    await user.click(avatar);
    expect(set).toHaveBeenCalledWith({ showPlexAvatar: false });
    await user.click(screen.getByRole("switch", { name: "Albums and playlists resume where you left off" }));
    expect(set).toHaveBeenCalledWith({ resumeContexts: false });
    await user.click(screen.getByRole("switch", { name: "Artwork colours" }));
    expect(set).toHaveBeenCalledWith({ artworkColours: false });

    // A fresh load reads the stored values back.
    useSettings.setState({ settings: null });
    await loadSettings();
    expect(useSettings.getState().settings).toMatchObject({
      privateSession: true,
      reportToPlex: false,
      showFormatBadge: false,
      showPlexAvatar: false,
      resumeContexts: false,
      artworkColours: false,
    });
  });

  it("dropdowns and the pre-cache field save their values", async () => {
    const user = userEvent.setup();
    const set = vi.spyOn(backend, "settingsSet");
    render(<SettingsPage />);
    await user.selectOptions(screen.getByRole("combobox", { name: "Streaming quality at home" }), "192");
    expect(set).toHaveBeenCalledWith({ qualityHome: "192" });
    await user.selectOptions(screen.getByRole("combobox", { name: "Streaming quality away from home" }), "original");
    expect(set).toHaveBeenCalledWith({ qualityAway: "original" });
    await user.selectOptions(screen.getByRole("combobox", { name: "Shuffle style" }), "standard");
    expect(set).toHaveBeenCalledWith({ shuffleStyle: "standard" });
    await user.selectOptions(screen.getByRole("combobox", { name: "Artist page Popular" }), "myPlays");
    expect(set).toHaveBeenCalledWith({ popularSource: "myPlays" });
    await user.selectOptions(screen.getByRole("combobox", { name: "Artwork and cache size" }), "5120");
    expect(set).toHaveBeenCalledWith({ artCacheMb: 5120 });
    await user.selectOptions(screen.getByRole("combobox", { name: "Reduce motion" }), "on");
    expect(set).toHaveBeenCalledWith({ reduceMotion: "on" });

    const pre = screen.getByRole("spinbutton", { name: "Pre-cache ahead" });
    fireEvent.change(pre, { target: { value: "12" } });
    expect(set).toHaveBeenCalledWith({ precacheCount: 12 });
    set.mockClear();
    fireEvent.change(pre, { target: { value: "80" } });
    expect(set).not.toHaveBeenCalled();
    fireEvent.blur(pre);
    expect(pre).toHaveValue(12);

    useSettings.setState({ settings: null });
    await loadSettings();
    expect(useSettings.getState().settings).toMatchObject({
      qualityHome: "192",
      qualityAway: "original",
      shuffleStyle: "standard",
      popularSource: "myPlays",
      artCacheMb: 5120,
      reduceMotion: "on",
      precacheCount: 12,
    });
  });

  it("search filters rows by title and description, with an empty state", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const search = screen.getByRole("searchbox", { name: "Search settings" });
    await user.type(search, "quality");
    expect(screen.getByText("Streaming quality at home")).toBeInTheDocument();
    expect(screen.getByText("Streaming quality away from home")).toBeInTheDocument();
    expect(screen.queryByText("Private session")).not.toBeInTheDocument();
    expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual(["Audio quality"]);

    // Description words match too.
    await user.clear(search);
    await user.type(search, "neutral");
    expect(screen.getByText("Artwork colours")).toBeInTheDocument();
    expect(screen.getAllByRole("heading", { level: 2 })).toHaveLength(1);

    await user.clear(search);
    await user.type(search, "zzzz");
    expect(screen.queryAllByRole("heading", { level: 2 })).toHaveLength(0);
    expect(screen.getByRole("status")).toHaveTextContent('No settings match "zzzz"');
  });

  it("clear cache empties the artwork cache and says how much was freed", async () => {
    const user = userEvent.setup();
    const clear = vi.spyOn(backend, "artCacheClear");
    render(<SettingsPage />);
    await screen.findByText("Using 412 MB.");
    await user.click(screen.getByRole("button", { name: "Clear cache" }));
    expect(clear).toHaveBeenCalled();
    await waitFor(() => expect(useToasts.getState().toasts.some((t) => t.message === "Cleared 412 MB of artwork")).toBe(true));
    expect(await screen.findByText("Using 0 MB.")).toBeInTheDocument();
  });

  it("resync starts a full sync and is disabled while one runs", async () => {
    const user = userEvent.setup();
    const sync = vi.spyOn(backend, "syncNow").mockResolvedValue();
    render(<SettingsPage />);
    await user.click(screen.getByRole("button", { name: "Resync library" }));
    expect(sync).toHaveBeenCalledWith(true);
    act(() => useApp.getState().setSync({ phase: "tracks", done: 10, total: 100 }));
    expect(screen.getByRole("button", { name: "Resync library" })).toBeDisabled();
    expect(screen.getByText("Syncing now.")).toBeInTheDocument();
  });

  it("the mock's full resync reports progress, then finishes", async () => {
    vi.useFakeTimers();
    try {
      const seen: string[] = [];
      const un = await backend.listen("sync://progress", (s) => seen.push(s.phase));
      await backend.syncNow(true);
      await vi.advanceTimersByTimeAsync(20_000);
      un();
      expect(seen).toContain("artists");
      expect(seen).toContain("tracks");
      expect(seen[seen.length - 1]).toBe("idle");
    } finally {
      vi.useRealTimers();
    }
  });

  it("sign out asks first", async () => {
    const user = userEvent.setup();
    const out = vi.spyOn(backend, "authSignOut").mockResolvedValue();
    render(<SettingsPage />);
    await user.click(screen.getByRole("button", { name: "Sign out" }));
    let dialog = screen.getByRole("dialog", { name: "Sign out of Plex?" });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(out).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Sign out" }));
    dialog = screen.getByRole("dialog", { name: "Sign out of Plex?" });
    await user.click(within(dialog).getByRole("button", { name: "Sign out" }));
    expect(out).toHaveBeenCalledTimes(1);
  });

  it("switch server lists servers and picks one", async () => {
    const user = userEvent.setup();
    const pick = vi.spyOn(backend, "serverSelect");
    render(<SettingsPage />);
    await user.click(screen.getByRole("button", { name: "Switch server" }));
    const dialog = screen.getByRole("dialog", { name: "Switch server" });
    const offline = await within(dialog).findByRole("button", { name: /Cabin NAS/ });
    expect(offline).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: /Basement Server/ }));
    expect(pick).toHaveBeenCalledWith("mock-machine-01");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("check for updates says up to date, or offers a restart when one is ready", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText("You're up to date.")).toBeInTheDocument();

    vi.spyOn(backend, "updateCheck").mockResolvedValue({ available: true, version: "0.2.0", notes: null });
    const install = vi.spyOn(backend, "updateInstallOnRestart").mockResolvedValue();
    await user.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText("Version 0.2.0 is ready. Restart to install it.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Restart to install" }));
    expect(install).toHaveBeenCalled();
  });

  it("about shows notices and opens the project page", async () => {
    const user = userEvent.setup();
    const open = vi.spyOn(backend, "openProjectPage");
    render(<SettingsPage />);
    await user.click(screen.getByRole("button", { name: "Project page" }));
    expect(open).toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Open-source notices" }));
    const dialog = screen.getByRole("dialog", { name: "Open-source notices" });
    expect(within(dialog).getAllByRole("listitem")).toHaveLength(NOTICES.length + SOURCE_NOTICES.length);
    expect(within(dialog).getByText("Tauri")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("notes when a quality setting doesn't apply right now", () => {
    useApp.setState({ health: { status: "relay", connection: "relay" } as never });
    render(<SettingsPage />);
    expect(row("Streaming quality away from home").getByText(/limits streams to 128 kbps/)).toBeInTheDocument();
    expect(row("Streaming quality at home").getByText("You're away from home right now.")).toBeInTheDocument();
  });

  it("shows skeletons until settings arrive", () => {
    useSettings.setState({ settings: null });
    render(<SettingsPage />);
    // Core settings wait for the core; the now-playing rows are local and show at once.
    expect(screen.queryByRole("switch", { name: "Private session" })).not.toBeInTheDocument();
    expect(document.querySelector(".skeleton")).not.toBeNull();
  });

  it("rolls back and explains when saving fails", async () => {
    vi.spyOn(backend, "settingsSet").mockRejectedValue({ kind: "io", message: "disk full" });
    await updateSettings({ privateSession: true });
    expect(useSettings.getState().settings?.privateSession).toBe(false);
    expect(useToasts.getState().toasts.some((t) => t.message.includes("disk full"))).toBe(true);
  });
});

describe("settings effects", () => {
  function Tinted() {
    useArtTint("#cc3322", null);
    return null;
  }

  it("artwork colours off gives headers the neutral tint", async () => {
    useUi.getState().setTint(null);
    render(<Tinted />);
    await waitFor(() => expect(useUi.getState().tint?.vivid).not.toBe(NEUTRAL_HEADER.vivid));
    expect(useUi.getState().tint).not.toBeNull();
    await act(() => updateSettings({ artworkColours: false }));
    expect(useUi.getState().tint).toEqual(NEUTRAL_HEADER);
    await act(() => updateSettings({ artworkColours: true }));
    expect(useUi.getState().tint).not.toEqual(NEUTRAL_HEADER);
  });

  it("format badge setting hides the quality badge", async () => {
    render(<QualityBadge track={{ codec: "flac", bitDepth: 24, sampleRate: 96000, bitrate: 2800 }} />);
    expect(screen.getByText(/FLAC/)).toBeInTheDocument();
    await act(() => updateSettings({ showFormatBadge: false }));
    expect(screen.queryByText(/FLAC/)).not.toBeInTheDocument();
  });

  it("reduce motion on or off overrides the system setting", async () => {
    const original = window.matchMedia;
    window.matchMedia = (() => ({
      matches: true,
      addEventListener: () => {},
      removeEventListener: () => {},
    })) as unknown as typeof window.matchMedia;
    try {
      render(<App />);
      const attr = () => document.documentElement.dataset.reducedMotion;
      await waitFor(() => expect(attr()).toBe("true"));
      await act(() => updateSettings({ reduceMotion: "off" }));
      expect(attr()).toBe("false");
      await act(() => updateSettings({ reduceMotion: "on" }));
      expect(attr()).toBe("true");
      await act(() => updateSettings({ reduceMotion: "system" }));
      expect(attr()).toBe("true");
    } finally {
      window.matchMedia = original;
      delete document.documentElement.dataset.reducedMotion;
    }
  });

  it("resume off starts an album at its first track in the mock too", async () => {
    const album = mock.fx.albums[3];
    await backend.playContext({ context: { kind: "album", id: album.id }, startTrackId: undefined });
    await backend.queueJump((await backend.playerState()).queue.nextUp[1].itemId);
    const resumed = (await backend.playerState()).track?.id;
    await backend.playContext({ context: { kind: "album", id: mock.fx.albums[4].id } });
    await backend.playContext({ context: { kind: "album", id: album.id } });
    expect((await backend.playerState()).track?.id).toBe(resumed);

    await updateSettings({ resumeContexts: false });
    await backend.playContext({ context: { kind: "album", id: mock.fx.albums[4].id } });
    await backend.playContext({ context: { kind: "album", id: album.id } });
    const first = (await backend.albumPage(album.id)).tracks[0].id;
    expect((await backend.playerState()).track?.id).toBe(first);
    await backend.playPause();
  });
});

describe("settings navigation", () => {
  it("the player bar shows a private session indicator only when on", async () => {
    render(<PlayerBar />);
    expect(screen.queryByRole("button", { name: "Private session is on" })).not.toBeInTheDocument();
    await updateSettings({ privateSession: true });
    const pill = await screen.findByRole("button", { name: "Private session is on" });
    fireEvent.click(pill);
    expect(route()).toEqual({ name: "settings" });
  });

  it("Ctrl+, opens settings from anywhere, and the app menu does too", async () => {
    useRouter.getState().reset({ name: "home" });
    render(<Host />);
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    expect(route()).toEqual({ name: "settings" });
    expect(await screen.findByRole("heading", { name: "Settings", level: 1 })).toBeInTheDocument();

    useRouter.getState().reset({ name: "home" });
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Settings", level: 1 })).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: /^App menu/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Settings/ }));
    expect(route()).toEqual({ name: "settings" });
  });
});
