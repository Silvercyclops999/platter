import { render, screen, waitFor, within } from "@testing-library/react";
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
  PairWithCode,
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

const mock = backend as MockBackend;

beforeEach(async () => {
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

  it("not paired: says what it does for you in order, then the three steps", async () => {
    const user = userEvent.setup();
    render(<BridgeSettings />);
    const card = await screen.findByTestId("bridge-setup");
    expect(card).toHaveTextContent("Platter works fully without it");
    // What it does, in this order; only what it does today has words, the rest say Coming.
    const benefits = within(within(card).getByRole("list", { name: "What it does for you" })).getAllByRole("listitem");
    expect(benefits.map((b) => b.querySelector(".bridge-benefit__title")?.textContent)).toEqual([
      "Lyrics for every song",
      "Sound analysis on the server",
      "Downloads ready when you leave",
      "Lidarr from anywhere",
      "One listening history",
    ]);
    expect(benefits.map((b) => b.textContent?.includes("Coming"))).toEqual([false, true, true, false, true]);
    expect(benefits[0]).toHaveTextContent("Your computer never does the heavy work.");
    expect(BENEFITS.filter((b) => b.live).map((b) => b.key)).toEqual(["lyrics", "lidarr"]);
    // Three steps: copy one line (or the Unraid template), start it, type its code.
    expect(within(card).getByText("Set it up in three steps")).toBeInTheDocument();
    expect(within(card).getByTestId("bridge-line")).toHaveTextContent(DOCKER_LINE);
    expect(DOCKER_LINE).toContain("/music:ro");
    expect(DOCKER_LINE.includes("\n")).toBe(false);
    await user.click(within(card).getByRole("tab", { name: "Unraid" }));
    expect(within(card).getByRole("tabpanel")).toHaveTextContent("platter-bridge.xml");
    expect(within(card).getByRole("button", { name: "Copy the template link" })).toBeInTheDocument();
    expect(card).toHaveTextContent("Start it");
    expect(card).toHaveTextContent("It shows a code starting with PBB- in its log.");
    expect(within(card).getByTestId("bridge-add")).toBeInTheDocument();
  });

  it("adds a Bridge from the code it shows, then says how each thing is going", async () => {
    const user = userEvent.setup();
    render(<BridgeSettings />);
    const add = await screen.findByTestId("bridge-add");
    const field = within(add).getByRole("textbox", { name: "The code from the Bridge, starting with PBB-" });
    // A code of the other kind is turned away with words.
    await user.type(field, "PB1-q3Vx8bN2kLm9TzR4wYc7HdE1fGj5sAo6pUi0eWn3vXb8yZaQk");
    await user.click(within(add).getByRole("button", { name: "Add the Bridge" }));
    expect(await within(add).findByRole("alert")).toHaveTextContent("starts with PBB-");
    await user.clear(field);
    await user.type(field, "PBB-q3Vx8bN2kLm9TzR4wYc7HdE1fGj5sAo6pUi0eWn3vXb8yZaQk{Enter}");
    const row = await screen.findByTestId("bridge-row");
    expect(screen.queryByTestId("bridge-setup")).toBeNull();
    expect(row).toHaveTextContent("Shed Bridge");
    expect(row).toHaveTextContent("Answering");
    expect(within(row).getByTestId("bridge-lyrics")).toHaveTextContent("Works 1 AM to 7 AM. 0 songs done, about 4,602 to go.");
    expect(within(row).getByTestId("bridge-lyrics")).toHaveTextContent("This computer leaves background transcribing to it.");
    expect(within(row).getByTestId("bridge-env")).toHaveTextContent("Set on the Bridge itself: Lidarr's address and Lidarr's key. It uses those, not what this computer sends.");
    expect(within(row).getByTestId("bridge-plex-link")).toHaveTextContent("open plex.tv/link and enter K3QZ");
    expect(within(row).getByTestId("bridge-music-path")).toHaveTextContent("MUSIC_PLEX_PATH to /data/media/Music on the Bridge so it can add albums itself");
  });

  it("pairs with a code made here (under More), then shows the Bridge and lets you unpair it", async () => {
    // Long enough that a busy machine still sees the code before the Bridge pairs.
    mock.bridgePairAfterMs = 1500;
    const user = userEvent.setup();
    render(
      <>
        <BridgeSettings />
        <PairWithCode />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: "Pair a Bridge" }));
    const code = await screen.findByTestId("bridge-code");
    expect(code).toHaveTextContent(/PB1-\S+/);
    expect(code).toHaveTextContent("BRIDGE_PAIR_CODE");
    const row = await screen.findByTestId("bridge-row", {}, { timeout: 5000 });
    expect(row).toHaveTextContent("Platter Bridge");
    expect(row).toHaveTextContent("read-write");
    expect(row).toHaveTextContent("Last catch-up");
    expect(row).toHaveTextContent("Second Wind");
    expect(within(row).getByTestId("bridge-lyrics")).toHaveTextContent("Transcribing Second Wind. 1,204 songs done, about 3,398 to go.");
    await waitFor(() => expect(screen.queryByTestId("bridge-code")).toBeNull());
    // Unpair asks once more.
    await user.click(within(row).getByRole("button", { name: "Unpair" }));
    await user.click(within(row).getByRole("button", { name: "Unpair Platter Bridge" }));
    expect(await screen.findByTestId("bridge-setup")).toBeInTheDocument();
    expect(screen.queryByTestId("bridge-row")).toBeNull();
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

    it("pairing from the card brings the switches and Connected at once", async () => {
      const user = userEvent.setup();
      render(<SettingsPage page="bridge" />);
      const page = await screen.findByRole("region", { name: "Bridge" });
      const add = await within(page).findByTestId("bridge-add");
      await user.type(within(add).getByRole("textbox"), "PBB-q3Vx8bN2kLm9TzR4wYc7HdE1fGj5sAo6pUi0eWn3vXb8yZaQk{Enter}");
      await within(page).findByTestId("bridge-row");
      await waitFor(() => expect(page.querySelector(".settings__page-blurb")).toHaveTextContent("Connected"));
      await user.click(within(page).getByRole("button", { name: /^More in Bridge/ }));
      expect(await within(page).findByRole("switch", { name: "Lyrics for every song" })).toBeChecked();
    });

    it("before pairing, More holds only the other way to pair", async () => {
      const user = userEvent.setup();
      render(<SettingsPage page="bridge" />);
      const page = await screen.findByRole("region", { name: "Bridge" });
      await within(page).findByTestId("bridge-setup");
      await user.click(within(page).getByRole("button", { name: /^More in Bridge/ }));
      expect([...page.querySelectorAll<HTMLElement>("[data-setting]")].map((r) => r.dataset.setting)).toEqual(["bridgePairCode"]);
      expect(within(page).getByRole("button", { name: "Pair a Bridge" })).toBeInTheDocument();
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
        "Platter Bridge 0.2.29 is behind this app (0.2.31). Update it on your server: on Unraid the Docker tab shows the update, or the Auto Update Applications plugin does it for you.",
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
      expect(v).toHaveTextContent("Platter Bridge is newer than this app; update Platter.");
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
