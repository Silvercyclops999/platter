import { act, render, screen, waitFor, within } from "@testing-library/react";
import { StrictMode } from "react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type BridgeInfo, type BridgeLyricsLine } from "../lib/types";
import { compareSemver } from "../lib/version";
import {
  BENEFITS,
  BRIDGE_UPDATE_DOCS,
  BridgeSettings,
  DOCKER_LINE,
  useBridgeStore,
  bridgeLabel,
  bridgeMatches,
  bridgeVersionLine,
  envList,
  lyricsWords,
  takesOver,
} from "../pages/BridgeSettings";
import { LidarrWorkStatus } from "../pages/LidarrModes";
import { SettingsPage } from "../pages/SettingsPage";
import { MORE_OPEN_KEY, advancedView, inMore, settingsView } from "../pages/settingsMeta";
import { loadSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

beforeEach(async () => {
  useBridgeStore.setState({ overview: null });
  mock.bridgeState = { bridges: [], pairing: null, queued: [] };
  mock.bridgePairAfterMs = 20;
  await mock.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

const line = (p: Partial<BridgeLyricsLine>): BridgeLyricsLine => ({ state: "working", done: 0, left: 0, hours: "night", song: null, problem: null, ...p });

const paired = (lyrics: BridgeLyricsLine | null | undefined, online = true): BridgeInfo => ({
  id: "b1",
  name: "Shed Bridge",
  version: "0.2.34",
  pairedAt: 1,
  lastSeen: 1,
  online,
  music: "readOnly",
  status: {
    name: "Shed Bridge",
    version: "0.2.34",
    at: 1,
    doing: "Waiting for the next catch-up",
    lastCatchUp: null,
    music: "readOnly",
    plex: true,
    lidarr: true,
    problem: null,
    fromEnv: [],
    plexLink: null,
    musicPlexPath: null,
    ...(lyrics === undefined ? {} : { lyrics }),
  },
});

describe("Settings > Bridge", () => {
  it("is found by the words people use for it", () => {
    for (const q of ["bridge", "unraid", "docker", "server lidarr", "lyrics", "night", ""]) expect(bridgeMatches(q)).toBe(true);
    expect(bridgeMatches("equalizer")).toBe(false);
  });

  it("no Bridge yet: says what it does for you in order, then the two steps, with no code", async () => {
    const user = userEvent.setup();
    render(<BridgeSettings />);
    const card = await screen.findByTestId("bridge-setup");
    expect(card).toHaveTextContent("Platter works fully without it");
    expect(within(card).getByRole("heading", { level: 3 })).toHaveTextContent(/^Bridge$/);
    // Only what it does today, in order. Nothing that's still coming.
    const benefits = within(within(card).getByRole("list", { name: "What it does for you" })).getAllByRole("listitem");
    expect(benefits.map((b) => b.querySelector(".bridge-benefit__title")?.textContent)).toEqual(["Lyrics for every song", "Lidarr from anywhere"]);
    expect(card).not.toHaveTextContent("Coming");
    expect(benefits[0]).toHaveTextContent("Your computer never does the heavy work.");
    expect(BENEFITS.filter((b) => b.live).map((b) => b.key)).toEqual(["lyrics", "lidarr"]);
    // Two steps: copy one line (or the Unraid template), start it and sign it in to Plex.
    expect(within(card).getByText("Set it up in two steps")).toBeInTheDocument();
    expect(within(card).getByTestId("bridge-line")).toHaveTextContent(DOCKER_LINE);
    expect(DOCKER_LINE).toContain("/music:ro");
    expect(DOCKER_LINE.includes("\n")).toBe(false);
    await user.click(within(card).getByRole("tab", { name: "Unraid" }));
    expect(within(card).getByRole("tabpanel")).toHaveTextContent("platter-bridge.xml");
    expect(within(card).getByRole("button", { name: "Copy the template link" })).toBeInTheDocument();
    expect(card).toHaveTextContent("Start it and sign it in to Plex");
    expect(card).toHaveTextContent("Open plex.tv/link and enter it, signed in as the owner of your Plex server.");
    expect(card).toHaveTextContent("finds the Bridge by itself");
    expect(card).not.toHaveTextContent("PBB-");
    expect(within(card).queryByRole("textbox")).toBeNull();
  });

  it("a Bridge on the same Plex account shows up by itself, then says how each thing is going", async () => {
    render(<BridgeSettings />);
    await screen.findByTestId("bridge-setup");
    act(() => mock.bridgeFound());
    const row = await screen.findByTestId("bridge-row");
    expect(screen.queryByTestId("bridge-setup")).toBeNull();
    // One Bridge is just "Bridge"; its own name waits under Details.
    expect(within(row).getByRole("heading", { level: 3 })).toHaveTextContent(/^Bridge$/);
    expect(row).toHaveTextContent("Working");
    expect(row).not.toHaveTextContent("Shed Bridge");
    const live = within(within(row).getByRole("list", { name: "What it does for you" })).getAllByRole("listitem");
    expect(live.map((l) => l.querySelector(".bridge-live__title")?.textContent)).toEqual(["Lyrics", "Lidarr"]);
    expect(within(row).getByTestId("bridge-lyrics")).toHaveTextContent("Works 1 AM to 7 AM. 0 songs done, about 4,602 to go.");
    expect(within(row).getByTestId("bridge-lyrics")).toHaveTextContent("This computer leaves background transcribing to it.");
    expect(within(row).getByText("Details").closest("details")).not.toHaveAttribute("open");
    expect(within(row).getByTestId("bridge-env")).toHaveTextContent("Set on the Bridge itself: Lidarr's address and Lidarr's key. It uses those, not what this computer sends.");
    expect(within(row).getByTestId("bridge-plex-link")).toHaveTextContent("open plex.tv/link and enter K3QZ");
    expect(within(row).getByTestId("bridge-music-path")).toHaveTextContent("MUSIC_PLEX_PATH to /data/media/Music on the Bridge so it can add albums itself");
  });

  it("unpairing a Bridge found on the account asks once more and says it won't come back on its own", async () => {
    const user = userEvent.setup();
    render(<BridgeSettings />);
    await screen.findByTestId("bridge-setup");
    act(() => mock.bridgeFound());
    const row = await screen.findByTestId("bridge-row");
    await user.click(within(row).getByRole("button", { name: "Unpair" }));
    await user.click(within(row).getByRole("button", { name: "Unpair Bridge" }));
    expect(await screen.findByTestId("bridge-setup")).toBeInTheDocument();
    expect(screen.queryByTestId("bridge-row")).toBeNull();
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message).join(" ")).toMatch(/^The Bridge was unpaired. It won't link to this device again/));
  });

  it("says plainly when the lyrics work is off, can't run, or the Bridge is too old for it", async () => {
    expect(lyricsWords(line({ state: "working", song: "Night Drive", done: 1, left: 2 }), true).text).toBe("Transcribing Night Drive. 1 song done, about 2 to go.");
    expect(lyricsWords(line({ state: "waiting", hours: "always", done: 5, left: 9 }), true).text).toBe("Works any time the server isn't busy. 5 songs done, about 9 to go.");
    expect(lyricsWords(line({ state: "busy", left: 3 }), true).text).toContain("Waiting while your server is busy.");
    expect(lyricsWords(line({ state: "downloading" }), true).text).toBe("Downloading the speech model. It does this once.");
    expect(lyricsWords(line({ state: "done", done: 3200 }), true).text).toBe("Every song has lyrics. 3,200 done on the Bridge.");
    expect(lyricsWords(line({ state: "problem", problem: "The speech model didn't download. The Bridge tries again in an hour." }), true)).toEqual({
      text: "The speech model didn't download. The Bridge tries again in an hour.",
      problem: true,
    });
    expect(lyricsWords(line({ state: "working" }), false).text).toBe("Off.");
    expect(lyricsWords(undefined, true).text).toBe("Needs a newer Bridge. Update it on your server to get this.");
    // Only a Bridge that answers and is well takes the background batch off this computer.
    expect(takesOver(paired(line({ state: "waiting" })), true)).toBe(true);
    expect(takesOver(paired(line({ state: "waiting" }), false), true)).toBe(false);
    expect(takesOver(paired(line({ state: "problem" })), true)).toBe(false);
    expect(takesOver(paired(undefined), true)).toBe(false);
    expect(takesOver(paired(line({ state: "working" })), false)).toBe(false);
    // On the card.
    mock.bridgeState.bridges = [paired(line({ state: "noMusic", problem: "Mount your music folder at /music (read-only is fine), so the Bridge can hear your songs." }))];
    const { unmount } = render(<BridgeSettings />);
    const ly = await screen.findByTestId("bridge-lyrics");
    expect(ly).toHaveTextContent("Mount your music folder at /music");
    expect(ly).not.toHaveTextContent("leaves background transcribing");
    unmount();
    mock.bridgeState.bridges = [paired(undefined)];
    render(<BridgeSettings />);
    expect(await screen.findByTestId("bridge-lyrics")).toHaveTextContent("Needs a newer Bridge");
  });

  it("calls one Bridge just Bridge, and only names them when there's more than one", () => {
    const a = paired(null);
    const b = { ...paired(null), id: "b2" };
    const c = { ...paired(null), id: "b3", name: "Attic" };
    expect(bridgeLabel(a, [a])).toBeUndefined();
    expect(bridgeLabel(a, [a, c])).toBe("Shed Bridge");
    expect(bridgeLabel(c, [a, c])).toBe("Attic");
    expect([a, b].map((x) => bridgeLabel(x, [a, b]))).toEqual(["Shed Bridge 1", "Shed Bridge 2"]);
  });

  it("names the Bridge's own settings plainly", () => {
    expect(envList(["lidarrUrl"])).toBe("Lidarr's address");
    expect(envList(["lidarrUrl", "lidarrKey", "plexToken"])).toBe("Lidarr's address, Lidarr's key and the Plex token");
    expect(envList([])).toBe("");
  });

  it("shows Lidarr settings changes a Bridge queued, with exactly what it would send", async () => {
    const user = userEvent.setup();
    mock.bridgeState.queued = [
      { id: "q1", what: "Change Lidarr's file naming settings", method: "PUT", path: "/config/naming/1", body: '{"renameTracks":false}', at: 1, device: "Platter Bridge" },
    ];
    render(<BridgeSettings />);
    const q = await screen.findByTestId("bridge-queued");
    expect(q).toHaveTextContent("Waiting for you: Change Lidarr's file naming settings");
    expect(q).toHaveTextContent("never does that on its own");
    expect(q).toHaveTextContent("PUT /config/naming/1");
    await user.click(within(q).getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByTestId("bridge-queued")).toBeNull();
  });

  describe("the page: the card first, each benefit's switch under its More", () => {
    beforeEach(() => {
      settingsView.inlinePages = false;
      advancedView.startOpen = false;
      localStorage.removeItem(MORE_OPEN_KEY);
    });
    afterEach(() => {
      settingsView.inlinePages = true;
      advancedView.startOpen = true;
    });

    it("has the switches once paired, all on, synced, and they switch off", async () => {
      expect(["bridgeLyrics", "bridgeLyricsHours", "bridgeLidarr"].every((key) => inMore({ key, page: "bridge" }))).toBe(true);
      mock.bridgeState.bridges = [paired(line({ state: "working" }))];
      const user = userEvent.setup();
      render(<SettingsPage page="bridge" />);
      const page = await screen.findByRole("region", { name: "Bridge" });
      const card = await within(page).findByTestId("bridge-row");
      const more = within(page).getByRole("button", { name: /^More in Bridge/ });
      // The card comes before More.
      expect(card.compareDocumentPosition(more) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(page.querySelector('[data-setting="bridgeLyrics"]')).toBeNull();
      await user.click(more);
      const lyrics = await within(page).findByRole("switch", { name: "Lyrics for every song" });
      const lidarr = within(page).getByRole("switch", { name: "Lidarr from anywhere" });
      expect(lyrics).toBeChecked();
      expect(lidarr).toBeChecked();
      expect(page.querySelector('[data-setting="bridgeLyrics"]')).not.toHaveTextContent("This device only");
      expect(within(page).getByRole("combobox", { name: "Night hours" })).toHaveTextContent("1 AM to 7 AM");
      await user.click(lyrics);
      await waitFor(async () => expect((await mock.settingsGet()).bridgeLyrics).toBe(false));
      await waitFor(() => expect(within(card).getByTestId("bridge-lyrics")).toHaveTextContent("Off."));
      await user.click(lidarr);
      await waitFor(async () => expect((await mock.settingsGet()).bridgeLidarr).toBe(false));
    });

    it("pairing from the card brings the switches and Connected at once, and unpairing takes them away", async () => {
      const user = userEvent.setup();
      // StrictMode runs effects twice: every part must still hear bridge://changed.
      render(
        <StrictMode>
          <SettingsPage page="bridge" />
        </StrictMode>,
      );
      const page = await screen.findByRole("region", { name: "Bridge" });
      await within(page).findByTestId("bridge-setup");
      act(() => mock.bridgeFound());
      await within(page).findByTestId("bridge-row");
      await waitFor(() => expect(page.querySelector(".settings__page-blurb")).toHaveTextContent("Connected"));
      await user.click(within(page).getByRole("button", { name: /^More in Bridge/ }));
      expect(await within(page).findByRole("switch", { name: "Lyrics for every song" })).toBeChecked();
      const card = within(page).getByTestId("bridge-row");
      await user.click(within(card).getByRole("button", { name: "Unpair" }));
      await user.click(within(card).getByRole("button", { name: "Unpair Bridge" }));
      await within(page).findByTestId("bridge-setup");
      await waitFor(() => expect(page.querySelector(".settings__page-blurb")).toHaveTextContent("Not set up"));
      expect(within(page).queryByRole("switch", { name: "Lyrics for every song" })).toBeNull();
    });

    it("before a Bridge turns up, there are no codes anywhere on the page", async () => {
      render(<SettingsPage page="bridge" />);
      const page = await screen.findByRole("region", { name: "Bridge" });
      await within(page).findByTestId("bridge-setup");
      expect(page.querySelector('[data-setting="bridgePairCode"], [data-setting="bridgeAddAnother"]')).toBeNull();
      expect(within(page).queryByRole("button", { name: "Pair a Bridge" })).toBeNull();
      expect(page).not.toHaveTextContent("PBB-");
    });
  });

  describe("when the Bridge and this app aren't the same version", () => {
    const pairedAt = (version: string, statusVersion?: string) => {
      const b = paired(null);
      mock.bridgeState.bridges = [{ ...b, version, status: statusVersion === undefined ? null : { ...b.status!, version: statusVersion } }];
    };
    afterEach(() => {
      mock.appVersion = "0.2.20";
    });

    it("says a Bridge behind this app needs updating, with a link to how", async () => {
      mock.appVersion = "0.2.31";
      // What the Bridge said last wins over the version it paired with.
      pairedAt("0.2.20", "0.2.29");
      mock.helpLinksOpened.length = 0;
      const user = userEvent.setup();
      render(<BridgeSettings />);
      const v = await screen.findByTestId("bridge-version");
      expect(v).toHaveTextContent(
        "The Bridge is out of date. Update it on your server.",
      );
      await user.click(within(v).getByRole("button", { name: "How to update" }));
      await waitFor(() => expect(mock.helpLinksOpened).toEqual([BRIDGE_UPDATE_DOCS]));
      expect(BRIDGE_UPDATE_DOCS).toBe("https://github.com/Silvercyclops999/platter/blob/main/docs/BRIDGE.md#updates");
    });

    it("says to update Platter when the Bridge is newer", async () => {
      mock.appVersion = "0.2.29";
      pairedAt("0.2.31");
      render(<BridgeSettings />);
      const v = await screen.findByTestId("bridge-version");
      expect(v).toHaveTextContent("The Bridge is newer than this app; update Platter.");
      expect(within(v).queryByRole("button")).toBeNull();
    });

    it("says nothing when they match, or when the Bridge's version isn't known", async () => {
      mock.appVersion = "0.2.31";
      pairedAt("0.2.31", "0.2.31");
      const { unmount } = render(<BridgeSettings />);
      await screen.findByTestId("bridge-row");
      expect(screen.queryByTestId("bridge-version")).toBeNull();
      unmount();
      pairedAt("", "");
      render(<BridgeSettings />);
      await screen.findByTestId("bridge-row");
      expect(screen.queryByTestId("bridge-version")).toBeNull();
    });

    it("ignores pre-release tags", async () => {
      mock.appVersion = "0.2.31-beta.2";
      pairedAt("0.2.31");
      render(<BridgeSettings />);
      await screen.findByTestId("bridge-row");
      expect(screen.queryByTestId("bridge-version")).toBeNull();
    });

    it("compares versions as numbers, not text", () => {
      expect(compareSemver("0.2.29", "0.2.31")).toBe(-1);
      expect(compareSemver("0.2.10", "0.2.9")).toBe(1);
      expect(compareSemver("1.0.0", "0.99.99")).toBe(1);
      expect(compareSemver("0.2.31", "0.2.31")).toBe(0);
      expect(compareSemver("v0.2.31", "0.2.31")).toBe(0);
      expect(compareSemver("0.2.31-rc.1", "0.2.31")).toBe(0);
      expect(compareSemver("0.2.30-beta", "0.2.31+build.5")).toBe(-1);
      expect(compareSemver("0.3", "0.3.0")).toBe(0);
      expect(compareSemver("", "0.2.31")).toBeNull();
      expect(compareSemver("unknown", "0.2.31")).toBeNull();
      expect(compareSemver("0.2.31", null)).toBeNull();
      expect(bridgeVersionLine("0.2.31", null)).toBeNull();
      expect(bridgeVersionLine("0.2.29", "0.2.31")?.behind).toBe(true);
    });
  });

  it("the Lidarr work line names the Bridge when one is paired", async () => {
    mock.bridgeState.bridges = [
      { id: "b1", name: "Shed Bridge", version: "0.2.20", pairedAt: 1, lastSeen: 1, online: true, status: null, music: "none" },
    ];
    render(<LidarrWorkStatus />);
    expect(await screen.findByTestId("lidarr-bridge-line")).toHaveTextContent("Shed Bridge does this work while no computer has Platter open.");
  });
});
