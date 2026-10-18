/**
 * Settings stay easy: every row has a description and a section, rarely
 * needed rows fold under Advanced (and search still finds them), Reset this
 * section puts a section back, and settings export and import as one file
 * that never holds a key.
 */
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type Settings } from "../lib/types";
import { exportSettings, importSettings, NOT_EXPORTED } from "../lib/settingsFile";
import { SettingsPage } from "../pages/SettingsPage";
import { ADVANCED, DESCRIPTION_MAX, GROUP_HELP, HELP_BY_KEY, advancedView, resetPatch, settingsOfRows } from "../pages/settingsMeta";
import { ContextMenuHost } from "../components/ContextMenu";
import { helpPage } from "../help/pages";
import { useHelp } from "../stores/help";
import { loadSettings, useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;

beforeEach(async () => {
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

afterEach(() => {
  advancedView.startOpen = true;
  mock.settingsFileToOpen = null;
});

/**
 * Settings with no row of their own, and the row that sets them (or why
 * there's none: kept by the core, or set by a control on another screen).
 */
const NOT_A_ROW: Record<string, string> = {
  volume: "the player bar",
  artCacheMb: "artCache",
  crossfadeSeconds: "crossfade",
  eqBands: "eqEnabled",
  eqPreset: "eqEnabled",
  eqProfiles: "eqEnabled",
  eqOutputs: "eqEnabled",
  eqFallback: "eqEnabled",
  eqLastOutput: "kept by the core",
  serverView: "libraries",
  musicFolderServer: "musicFolder",
  downloadLimitGb: "downloadLimit",
  mixExcludeArtists: "mixExclude",
  mixExcludeGenres: "mixExclude",
  mixExcludeAlbums: "mixExclude",
  includePlexHistory: "the Stats rows",
  includeTautulliHistory: "the Stats rows",
  includeLastfmHistory: "the Stats rows",
  lidarrPracticeUntil: "lidarrWork",
  miniplayerBounds: "kept by the core",
  importSources: "the Library import section",
  playedMinPlays: "showPlayedALot",
  playedPeriod: "showPlayedALot",
  bringInWaves: "bringIn",
  bringInPerDay: "bringIn",
  bringInMinFreeGb: "bringIn",
  bringInMaxQueue: "bringIn",
  lookStepDone: "kept by the first-run step",
  newLookOffered: "kept by the one-time new look offer",
  turntableColumn: "layout",
  consoleColumns: "layout",
  consoleSort: "kept by the Console table's headers",
  consoleTree: "layout",
  consoleDetails: "layout",
  discordCoverArchive: "discordCoverSource",
  discordUploadCovers: "discordCoverSource",
  discordUploadArt: "discordCoverSource",
  discordCoverHost: "discordCoverSource",
};

/** Rows that show only in some states (each was checked in its own test file). */
const SHOWN_WHEN: Record<string, string> = {
  plexEditShow: "a server that can edit (plex-edit.test.tsx)",
  plexEditLock: "a server that can edit",
  plexEditMatchUnreleased: "a server that can edit",
  syncFolder: "sync turned on (sync.test.tsx)",
  aiModel: "an AI provider chosen (curate.test.tsx)",
  upgradesAsk: "Lidarr checked and ready (lidarr-upgrade tests)",
  upgradeOfficialPrefix: "Lidarr checked and ready",
  subsonicCheckMinutes: "a Subsonic server (subsonic-servers.test.tsx)",
  saveQueueOnServer: "a server that keeps queues (subsonic-servers.test.tsx)",
  lidarrHeaders: "an away address for Lidarr (lidarr.test.tsx)",
  nowPlayingFeedPort: "the widget feed turned on (settings-layout.test.tsx)",
  phoneMobileDownloads: "the phone layout (phone.test.tsx)",
  phoneKeepRecent: "the phone layout",
  phoneMobileQuality: "the phone layout",
  phoneHaptics: "the phone layout (phone.test.tsx)",
};

describe("every setting", () => {
  it("has a row with a title, a one-line description and a section", async () => {
    // Rows that wait for their feature: Discord's status and transcription on.
    await mock.settingsSet({ lidarrUrl: "http://10.0.0.5:8686", tautulliUrl: "http://10.0.0.5:8181", discordStatus: true, transcribeLyrics: true });
    await mock.lidarrKeySet("abc123");
    await loadSettings();
    render(<SettingsPage />);
    await screen.findByRole("switch", { name: "Private session" });
    const rows = [...document.querySelectorAll<HTMLElement>("[data-setting]")];
    // Settings set from a row's checklist count as that row's.
    const keys = new Set([
      ...rows.map((r) => r.dataset.setting!),
      ...[...document.querySelectorAll<HTMLElement>("[data-setting-part]")].map((r) => r.dataset.settingPart!),
    ]);
    const long: string[] = [];
    for (const row of rows) {
      const key = row.dataset.setting;
      const title = row.querySelector(".setting-row__title")?.textContent?.trim();
      const descEl = row.querySelector(".setting-row__desc")?.cloneNode(true) as HTMLElement | undefined;
      descEl?.querySelectorAll(".setting-row__note, button").forEach((n) => n.remove());
      const desc = descEl?.textContent?.trim();
      expect(title, `${key} has a title`).toBeTruthy();
      expect(desc && desc.length > 10, `${key} has a description`).toBe(true);
      // One line; anything longer goes behind the row's "?".
      if (desc!.length > DESCRIPTION_MAX) long.push(`${key}: ${desc}`);
      expect(row.closest("section")?.querySelector("h2")?.textContent, `${key} is in a section`).toBeTruthy();
    }
    expect(long).toEqual([]);
    const missing = Object.keys(DEFAULT_SETTINGS).filter((k) => !keys.has(k) && !NOT_A_ROW[k] && !SHOWN_WHEN[k]);
    expect(missing).toEqual([]);
    for (const via of Object.values(NOT_A_ROW)) {
      if (/^[a-z][A-Za-z]+$/.test(via)) expect(keys.has(via), `${via} is a row`).toBe(true);
    }
  });

  it("names only real rows as Advanced, and Learn more points at real help", async () => {
    await mock.settingsSet({ lidarrUrl: "http://10.0.0.5:8686", discordStatus: true, transcribeLyrics: true });
    await mock.lidarrKeySet("abc123");
    await loadSettings();
    render(<SettingsPage />);
    await screen.findByRole("switch", { name: "Private session" });
    const keys = new Set([...document.querySelectorAll<HTMLElement>("[data-setting]")].map((r) => r.dataset.setting!));
    // Rows that only show with a server feature or a running service the mock lacks here.
    const notShown = [...ADVANCED].filter((k) => !keys.has(k) && !SHOWN_WHEN[k]);
    expect(notShown).toEqual([]);
    for (const [k, [page, anchor]] of [...Object.entries(HELP_BY_KEY), ...Object.entries(GROUP_HELP)]) {
      const p = helpPage(page);
      expect(p, `${k}: ${page} exists`).toBeTruthy();
      if (anchor) expect(p!.blocks.some((b) => b.t === "h" && b.id === anchor), `${k}: ${page}#${anchor}`).toBe(true);
    }
  });
});

describe("Advanced", () => {
  it("folds rarely needed rows away in each section, and opens on a click", async () => {
    advancedView.startOpen = false;
    const user = userEvent.setup();
    render(<SettingsPage />);
    const playback = await screen.findByRole("region", { name: "Playback" });
    expect(within(playback).getByRole("combobox", { name: "Shuffle style" })).toBeInTheDocument();
    expect(within(playback).queryByRole("spinbutton", { name: "Songs to load ahead" })).toBeNull();
    const toggle = within(playback).getByRole("button", { name: /^Advanced/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(within(playback).getByRole("spinbutton", { name: "Songs to load ahead" })).toBeInTheDocument();
    // Other sections stay folded.
    const library = screen.getByRole("region", { name: "Library" });
    expect(within(library).getByRole("button", { name: /^Advanced/ })).toHaveAttribute("aria-expanded", "false");
    expect(within(library).queryByRole("switch", { name: "Forgive typos in search" })).toBeNull();
  });

  it("settings search finds Advanced rows without opening anything", async () => {
    advancedView.startOpen = false;
    await mock.settingsSet({ discordStatus: true });
    await loadSettings();
    render(<SettingsPage />);
    await screen.findByRole("region", { name: "Playback" });
    fireEvent.change(screen.getByRole("searchbox", { name: "Search settings" }), { target: { value: "load ahead" } });
    expect(await screen.findByRole("spinbutton", { name: "Songs to load ahead" })).toBeInTheDocument();
    fireEvent.change(screen.getByRole("searchbox", { name: "Search settings" }), { target: { value: "discord status shows" } });
    expect(await screen.findByRole("combobox", { name: "Discord status shows" })).toBeInTheDocument();
  });
});

describe("Reset this section", () => {
  it("asks first, then puts only that section back", async () => {
    const user = userEvent.setup();
    await mock.settingsSet({ shuffleStyle: "standard", precacheCount: 9, theme: "light", lidarrUrl: "http://10.0.0.5:8686" });
    await loadSettings();
    render(
      <>
        <SettingsPage />
        <ContextMenuHost />
      </>,
    );
    // Reset this section is in the group heading's ⋯ menu.
    const resetFrom = async (group: string) => {
      await user.click(await screen.findByRole("button", { name: `More options for ${group}` }));
      await user.click(await screen.findByRole("menuitem", { name: "Reset this section" }));
    };
    await resetFrom("Playback");
    const dialog = screen.getByRole("dialog", { name: "Reset Playback settings?" });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(useSettings.getState().settings?.shuffleStyle).toBe("standard");
    await resetFrom("Playback");
    await user.click(within(screen.getByRole("dialog", { name: "Reset Playback settings?" })).getByRole("button", { name: "Reset" }));
    await waitFor(() => expect(useSettings.getState().settings?.shuffleStyle).toBe("spread"));
    const s = await mock.settingsGet();
    expect(s.precacheCount).toBe(DEFAULT_SETTINGS.precacheCount);
    expect(s.theme).toBe("light");
    // Addresses you set up stay, even when their section is reset.
    await resetFrom("Integrations");
    await user.click(within(screen.getByRole("dialog", { name: "Reset Integrations settings?" })).getByRole("button", { name: "Reset" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect((await mock.settingsGet()).lidarrUrl).toBe("http://10.0.0.5:8686");
  });

  it("covers composite rows and never addresses or folders", () => {
    const keys = settingsOfRows(["crossfade", "artCache", "lidarrUrl", "musicFolder", "mixExclude", "notASetting"], DEFAULT_SETTINGS);
    expect(keys.sort()).toEqual(["artCacheMb", "crossfade", "crossfadeSeconds", "mixExcludeAlbums", "mixExcludeArtists", "mixExcludeGenres"].sort());
    expect(resetPatch(["crossfadeSeconds"], DEFAULT_SETTINGS)).toEqual({ crossfadeSeconds: DEFAULT_SETTINGS.crossfadeSeconds });
  });
});

describe("export and import", () => {
  it("round-trips every setting through one file with no keys in it", async () => {
    const user = userEvent.setup();
    await mock.settingsSet({ theme: "light", crossfadeSeconds: 9, mixExcludeGenres: ["Polka"], showHelpButtons: false, lidarrUrl: "http://10.0.0.5:8686" });
    await mock.lidarrKeySet("lidarrsecret123");
    await mock.settingsSet({ musicFolder: "/Volumes/Music" } as Partial<Settings>);
    await loadSettings();
    render(<SettingsPage />);
    const row = (await screen.findByText("Export or import settings")).closest("[data-setting]") as HTMLElement;
    await user.click(within(row).getByRole("button", { name: "Export" }));
    await waitFor(() => expect(mock.settingsFileSaved).not.toBeNull());
    const file = mock.settingsFileSaved!;
    const text = JSON.stringify(file);
    expect(text).not.toContain("lidarrsecret123");
    for (const k of NOT_EXPORTED) expect(k in file.settings, `${k} stays on this computer`).toBe(false);
    expect(file.settings.theme).toBe("light");

    // Change things, then import the file back.
    await mock.settingsSet({ ...DEFAULT_SETTINGS });
    await loadSettings();
    mock.settingsFileToOpen = JSON.parse(text);
    await user.click(within(row).getByRole("button", { name: "Import" }));
    await waitFor(() => expect(useSettings.getState().settings?.theme).toBe("light"));
    const s = await mock.settingsGet();
    expect(s.crossfadeSeconds).toBe(9);
    expect(s.mixExcludeGenres).toEqual(["Polka"]);
    expect(s.showHelpButtons).toBe(false);
  });

  it("skips what it can't use and refuses other files", () => {
    const { patch, report } = importSettings({ kind: "platter-settings", version: 1, settings: { theme: "light", crossfadeSeconds: "nine", nope: 1, musicFolder: "/x" } });
    expect(patch).toEqual({ theme: "light" });
    expect(report).toEqual({ applied: 1, skipped: ["crossfadeSeconds", "nope"] });
    expect(() => importSettings({ settings: {} })).toThrow();
    expect(exportSettings(DEFAULT_SETTINGS).kind).toBe("platter-settings");
  });
});

describe("help buttons and the new settings", () => {
  it("both start on, sync, and the ? buttons hide when turned off", async () => {
    expect(DEFAULT_SETTINGS.showSetupGuides && DEFAULT_SETTINGS.showHelpButtons).toBe(true);
    const user = userEvent.setup();
    render(<SettingsPage />);
    const tx = (await screen.findAllByRole("button", { name: "Help with Lidarr" }))[0];
    await user.click(tx);
    expect(useUi.getState().helpOpen).toBe(true);
    expect(useHelp.getState()).toMatchObject({ slug: "lidarr" });
    await user.click(screen.getByRole("switch", { name: 'Show "?" help buttons' }));
    await waitFor(() => expect(screen.queryAllByRole("button", { name: "Help with Lidarr" })).toHaveLength(0));
    expect(screen.getByRole("switch", { name: "Show setup guides for new features" })).toBeChecked();
    // Learn more stays: it's part of the description.
    expect(screen.getAllByRole("button", { name: /^Learn more about/ }).length).toBeGreaterThan(10);
    useUi.getState().setHelpOpen(false);
  });
});
