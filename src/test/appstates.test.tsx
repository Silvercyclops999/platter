/**
 * App states: every banner the core can send, what the UI shows for it, and
 * the behavior around it (sign in again, dimming what can't play, toasts).
 * Each state is faked through the mock backend or the store.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import App from "../App";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { demoHealth, HEALTH_DEMOS, LOCAL_NETWORK_BLOCKED, type HealthDemo } from "../lib/mock/health";
import { OFFLINE_NOTHING_PLAYS, OFFLINE_SEARCH, OFFLINE_SKIPPED, offlineLeftOutMessage } from "../lib/offline";
import { HomePage } from "../pages/HomePage";
import { SearchPage } from "../pages/SearchPage";
import type { Health } from "../lib/types";
import { agoText, healthBanner, offlineCallout } from "../shell/bannerView";
import { OfflineIndicator } from "../shell/TopBar";
import { HealthBanner } from "../shell/HealthBanner";
import { routeBadge, statusInfo } from "../shell/StatusDot";
import { connectApp, isUnavailable, useApp } from "../stores/app";
import { PlayerBar } from "../shell/PlayerBar";
import { connectPlayer, usePlayer } from "../stores/player";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";
import { body } from "./rows";

const mock = backend as MockBackend;
const SERVER = { name: "Basement Server", version: "1.42.2" };
const NOW = 1_760_000_000_000;
const h = (d: HealthDemo) => demoHealth(d, SERVER, NOW);

// What the user sees for each state, word for word.
const EXPECTED: Record<HealthDemo, { message: string; kind: string; action?: string } | null> = {
  local: null,
  remote: null,
  relay: {
    kind: "warning",
    message: "Connected through Plex Relay, so quality is reduced. Turn on Remote Access in Plex for full quality.",
  },
  // Plain offline needs nothing from the user: the top bar icon says it.
  offline: null,
  noInternet: null,
  starting: { kind: "progress", message: "Server is starting…" },
  updating: { kind: "progress", message: "Server is updating…" },
  startTimeout: { kind: "error", message: "Your server is taking too long to come back. Check that Plex is running." },
  signIn: { kind: "warning", message: "Please sign in again.", action: "signIn" },
  noAuth: { kind: "warning", message: "Please sign in again.", action: "signIn" },
  accessRemoved: { kind: "error", message: "You no longer have access to this server.", action: "signOut" },
  filesUnavailable: { kind: "error", message: "Your server is up, but your music files aren't available." },
  noMusicLibrary: { kind: "info", message: "No music library found.", action: "libraryGuide" },
  diskFull: { kind: "warning", message: "Not enough space." },
  localNetwork: { kind: "warning", message: LOCAL_NETWORK_BLOCKED, action: "localNetwork" },
  localNetworkRemote: { kind: "warning", message: LOCAL_NETWORK_BLOCKED, action: "localNetwork" },
  scanning: { kind: "progress", message: "Adding new music… 120 of 340" },
  // Go offline: the top bar icon says it; the app menu turns it off.
  goneOffline: null,
};

describe("banner for each app state", () => {
  it.each(HEALTH_DEMOS)("%s", (demo) => {
    const v = healthBanner(h(demo), NOW);
    const want = EXPECTED[demo];
    if (!want) {
      expect(v).toBeNull();
      return;
    }
    expect(v).toEqual({ kind: want.kind, message: want.message, action: want.action });
  });

  it("the offline callout says why for each cause and keeps 'last seen' current", () => {
    const offline = "Platter is offline.";
    expect(offlineCallout(h("noInternet"), NOW)).toEqual({ title: offline, body: "Check your connection; it reconnects by itself." });
    expect(offlineCallout(h("offline"), NOW + 6 * 60_000)).toEqual({
      title: offline,
      body: "Can't reach your server (last seen 10 min ago). Platter reconnects by itself.",
    });
    expect(offlineCallout({ ...h("offline"), lastSeenAt: null }, NOW)!.body).toBe("Can't reach your server. Platter reconnects by itself.");
    expect(offlineCallout(h("localNetwork"), NOW)).toEqual({ title: offline, body: LOCAL_NETWORK_BLOCKED });
    // Chosen by the user: says so, and where to turn it off.
    expect(offlineCallout(h("goneOffline"), NOW)).toEqual({
      title: "You chose Go offline.",
      body: "Only music on this computer plays. Turn it off in your profile menu.",
    });
    expect(offlineCallout(h("goneOffline"), NOW, true)!.body).toBe(
      "Only music on this computer plays. Turn it off in your profile menu, or File › Go Offline.",
    );
    for (const d of ["local", "remote", "relay", "localNetworkRemote", "signIn", "filesUnavailable"] as HealthDemo[])
      expect(offlineCallout(h(d), NOW), d).toBeNull();
    expect(offlineCallout(null)).toBeNull();
    expect(LOCAL_NETWORK_BLOCKED).toBe(
      "macOS is blocking Platter from your home network. Open System Settings, Privacy & Security, Local Network, and turn on Platter.",
    );
    expect(agoText(30_000)).toBe("just now");
    expect(agoText(3 * 3_600_000)).toBe("3 h ago");
  });

  it("a server-side banner wins over scan progress", () => {
    const x: Health = { ...h("relay"), scan: { done: 1, total: 2 } };
    expect(healthBanner(x, NOW)!.kind).toBe("warning");
  });

  it("only cached music plays when the server or files are gone", () => {
    for (const d of ["offline", "noInternet", "starting", "signIn", "filesUnavailable"] as HealthDemo[]) expect(h(d).cachedOnly, d).toBe(true);
    for (const d of ["local", "remote", "relay", "diskFull", "noMusicLibrary", "scanning"] as HealthDemo[]) expect(h(d).cachedOnly, d).toBe(false);
  });
});

describe("status dot", () => {
  it("shows Remote / Relay badges and a plain label", () => {
    expect(routeBadge(h("remote"))).toBe("Remote");
    expect(routeBadge(h("relay"))).toBe("Relay");
    expect(routeBadge(h("local"))).toBeNull();
    expect(statusInfo(h("offline"))).toEqual({ tone: "offline", label: "Server offline" });
    expect(statusInfo(h("noInternet"))).toEqual({ tone: "offline", label: "You're offline" });
  });
});

describe("HealthBanner component", () => {
  afterEach(() => act(() => useApp.setState({ health: null })));

  it("renders the message with its action", async () => {
    const user = userEvent.setup();
    act(() => useApp.setState({ health: h("noMusicLibrary") }));
    render(<HealthBanner />);
    expect(screen.getByRole("status")).toHaveTextContent("No music library found.");
    await user.click(screen.getByRole("button", { name: "How to add one" }));
    const dlg = screen.getByRole("dialog", { name: "Add a music library in Plex" });
    expect(within(dlg).getAllByText(/Add Library/).length).toBeGreaterThan(0);
  });

  it("plain offline shows a top bar button, not a banner, and it leaves on reconnect", async () => {
    const user = userEvent.setup();
    act(() => useApp.setState({ health: demoHealth("noInternet", SERVER) }));
    const { container } = render(<HealthBanner />);
    expect(container).toBeEmptyDOMElement();
    render(<OfflineIndicator />);
    const button = screen.getByRole("button", { name: "You're offline" });
    expect(button).toHaveAttribute("aria-expanded", "false");
    // No tooltip.
    await user.hover(button);
    await new Promise((r) => setTimeout(r, 700));
    expect(screen.queryByRole("tooltip")).toBeNull();
    // A click opens the callout and does nothing else.
    await user.click(button);
    const callout = screen.getByRole("dialog", { name: "Platter is offline." });
    expect(callout).toHaveTextContent("Check your connection; it reconnects by itself.");
    expect(button).toHaveAttribute("aria-expanded", "true");
    // Reconnecting takes the button and the open callout away.
    act(() => useApp.setState({ health: demoHealth("local", SERVER) }));
    expect(screen.queryByTestId("offline-indicator")).toBeNull();
    expect(screen.queryByTestId("offline-callout")).toBeNull();
    // Offline again: the callout starts closed.
    act(() => useApp.setState({ health: demoHealth("noInternet", SERVER) }));
    expect(screen.getByTestId("offline-indicator")).toHaveAttribute("aria-expanded", "false");
  });

  it("the offline callout closes on the button, a click elsewhere, or Escape", async () => {
    const user = userEvent.setup();
    act(() => useApp.setState({ health: demoHealth("offline", SERVER) }));
    render(
      <>
        <OfflineIndicator />
        <p>elsewhere</p>
      </>,
    );
    const button = screen.getByTestId("offline-indicator");
    await user.click(button);
    expect(screen.getByTestId("offline-callout")).toHaveTextContent(/Can't reach your server/);
    // Clicking inside keeps it.
    await user.click(screen.getByTestId("offline-callout"));
    expect(screen.getByTestId("offline-callout")).toBeInTheDocument();
    await user.click(button);
    expect(screen.queryByTestId("offline-callout")).toBeNull();
    await user.click(button);
    await user.click(screen.getByText("elsewhere"));
    expect(screen.queryByTestId("offline-callout")).toBeNull();
    await user.click(button);
    await user.keyboard("{Escape}");
    expect(screen.queryByTestId("offline-callout")).toBeNull();
    expect(button).toHaveFocus();
  });

  it("Go offline: the callout says it was chosen and where to turn it off", async () => {
    const user = userEvent.setup();
    act(() => useApp.setState({ health: demoHealth("goneOffline", SERVER) }));
    render(<OfflineIndicator />);
    await user.click(screen.getByRole("button", { name: "You're offline" }));
    const callout = screen.getByRole("dialog", { name: "You chose Go offline." });
    expect(callout).toHaveTextContent("Turn it off in your profile menu");
  });

  it("macOS blocking the home network: banner with a button to the settings", async () => {
    const user = userEvent.setup();
    const before = mock.localNetworkOpened;
    act(() => useApp.setState({ health: h("localNetwork") }));
    render(<HealthBanner />);
    expect(screen.getByText(LOCAL_NETWORK_BLOCKED)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open Settings" }));
    await waitFor(() => expect(mock.localNetworkOpened).toBe(before + 1));
  });

  it("Sign in opens the sign-in screen", async () => {
    const user = userEvent.setup();
    act(() => useApp.setState({ health: h("signIn") }));
    render(<HealthBanner />);
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(useUi.getState().signInOpen).toBe(true);
    act(() => useUi.setState({ signInOpen: false }));
  });

  it("shows nothing when all is well", () => {
    act(() => useApp.setState({ health: h("local") }));
    const { container } = render(<HealthBanner />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe("tracks that can't play are dimmed", () => {
  const [a, b] = mock.fx.tracks;
  afterEach(() => act(() => useApp.setState({ health: null, cached: new Set() })));

  it("dims only uncached tracks while cached-only", () => {
    act(() => useApp.setState({ health: h("offline"), cached: new Set([a.id]) }));
    render(
      <TrackList label="Tracks">
        <TrackRow track={a} index={1} onPlay={() => {}} />
        <TrackRow track={b} index={2} onPlay={() => {}} />
      </TrackList>,
    );
    const rows = body(screen.getAllByRole("row"));
    expect(rows[0]).not.toHaveClass("is-unavailable");
    expect(rows[1]).toHaveClass("is-unavailable");
    expect(rows[1]).toHaveAccessibleName(/not available right now/);
  });

  it("nothing is dimmed when connected", () => {
    expect(isUnavailable(b.id, { health: h("local"), cached: new Set() })).toBe(false);
    expect(isUnavailable(b.id, { health: h("filesUnavailable"), cached: new Set() })).toBe(true);
  });
});

describe("wired to the core", () => {
  let disconnect: () => void;
  beforeAll(() => {
    const a = connectApp(mock);
    const p = connectPlayer(mock);
    disconnect = () => {
      a();
      p();
    };
  });
  afterAll(() => disconnect());
  afterEach(() => act(() => useToasts.setState({ toasts: [] })));

  it("entering offline mode loads the cached track list", async () => {
    act(() => mock.setHealth("offline"));
    await waitFor(() => expect(useApp.getState().cached.size).toBeGreaterThan(0));
    const albums = new Set(mock.fx.albums.slice(0, 2).map((x) => x.id));
    for (const id of useApp.getState().cached) expect(albums.has(mock.fx.tracks.find((t) => t.id === id)!.albumId)).toBe(true);
  });

  it("server back → toast", async () => {
    act(() => mock.setHealth("offline"));
    act(() => mock.setHealth("local"));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Your server is back."));
  });

  it("a skipped track shows 'Skipped: file unavailable'", async () => {
    act(() => mock.emit("player://error", { trackId: "x", kind: "notFound", message: "Skipped: file unavailable" }));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Skipped: file unavailable"));
  });

  it("offline: songs failing one after another are told once, in plain words", async () => {
    act(() => mock.setHealth("offline"));
    for (const id of ["x", "y", "z"])
      act(() => mock.emit("player://error", { trackId: id, kind: "network", message: "Skipped: file unavailable" }));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain(OFFLINE_SKIPPED));
    const msgs = useToasts.getState().toasts.map((t) => t.message);
    expect(msgs.filter((m) => m === OFFLINE_SKIPPED)).toHaveLength(1);
    expect(msgs).not.toContain("Skipped: file unavailable");
    act(() => mock.setHealth("local"));
  });

  it("offline: play keeps to songs on this computer, says so once, and refuses when none can", async () => {
    act(() => mock.setHealth("offline"));
    const cached = new Set(mock.fx.albums.slice(0, 2).map((a) => a.id));
    const kept = mock.fx.tracks.find((t) => cached.has(t.albumId))!;
    const gone = mock.fx.tracks.filter((t) => !cached.has(t.albumId)).slice(0, 2);
    await mock.playContext({ context: { kind: "tracks", trackIds: [gone[0].id, kept.id, gone[1].id] }, startTrackId: gone[0].id });
    await waitFor(() => expect(usePlayer.getState().state?.track?.id).toBe(kept.id));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain(offlineLeftOutMessage(2)));
    await mock.playContext({ context: { kind: "tracks", trackIds: gone.map((t) => t.id) } });
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain(OFFLINE_NOTHING_PLAYS));
    expect(useToasts.getState().toasts.filter((t) => t.message === offlineLeftOutMessage(2))).toHaveLength(1);
    // Back online: every song plays again.
    act(() => mock.setHealth("local"));
    await mock.playContext({ context: { kind: "tracks", trackIds: [gone[0].id] } });
    await waitFor(() => expect(usePlayer.getState().state?.track?.id).toBe(gone[0].id));
    await mock.playPause();
  });

  it("offline: Home keeps to what can play and comes back whole on reconnect", async () => {
    act(() => mock.setHealth("offline"));
    render(<HomePage />);
    const shelf = await screen.findByRole("region", { name: "On this computer" });
    const cachedTitles = mock.fx.albums.slice(0, 2).map((a) => a.title);
    for (const t of cachedTitles) expect(within(shelf).getAllByText(t).length).toBeGreaterThan(0);
    // Albums with nothing on this computer leave every shelf.
    const other = mock.fx.albums.slice(2).map((a) => a.title);
    const shown = screen.getAllByRole("region").flatMap((r) => other.filter((t) => within(r).queryAllByText(t).length > 0));
    expect(shown).toEqual([]);
    act(() => mock.setHealth("local"));
    await waitFor(() => expect(screen.queryByRole("region", { name: "On this computer" })).toBeNull());
  });

  it("offline: search says the server is needed for what the local index can't answer", async () => {
    act(() => mock.setHealth("offline"));
    const spy = vi.spyOn(mock, "search").mockRejectedValueOnce({ kind: "network", message: "offline" });
    render(<SearchPage query="zzz nothing matches" />);
    expect(await screen.findByText(OFFLINE_SEARCH)).toBeInTheDocument();
    spy.mockRestore();
    act(() => mock.setHealth("local"));
    await waitFor(() => expect(screen.queryByText(OFFLINE_SEARCH)).toBeNull());
  });

  it("headphones unplugged → paused; Play resumes", async () => {
    const user = userEvent.setup();
    render(<PlayerBar />);
    await user.click(await screen.findByRole("button", { name: "Play" }));
    await screen.findByRole("button", { name: "Pause" });
    act(() => mock.loseDevice());
    await screen.findByRole("button", { name: "Play" });
    expect(usePlayer.getState().state!.status).toBe("paused");
    await user.click(screen.getByRole("button", { name: "Play" }));
    await screen.findByRole("button", { name: "Pause" });
    await user.click(screen.getByRole("button", { name: "Pause" }));
  });

  it("update downloaded → restart toast with a button", async () => {
    act(() => mock.emit("update://ready", { version: "0.2.0" }));
    await waitFor(() => {
      const t = useToasts.getState().toasts.find((x) => x.message.includes("0.2.0"));
      expect(t?.message).toBe("Platter 0.2.0 is ready. Restart to install it.");
      expect(t?.action?.label).toBe("Restart");
    });
  });

  it("library replaced toast comes through", async () => {
    act(() => mock.emit("toast", { kind: "info", message: "This library changed." }));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("This library changed."));
  });
});

describe("token expired or revoked", () => {
  it("keeps the app open with a sign-in banner; Sign in → Not now returns", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("navigation");
    act(() => mock.expireSignIn());
    expect(await screen.findByText("Please sign in again.")).toBeInTheDocument();
    // still the app, with the player bar, not the welcome screen
    expect(screen.queryByRole("button", { name: "Sign in with Plex" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("button", { name: "Sign in with Plex" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Not now" }));
    expect(await screen.findByText("Please sign in again.")).toBeInTheDocument();
    // signing in for real returns to normal
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await user.click(await screen.findByRole("button", { name: "Sign in with Plex" }));
    act(() => mock.approvePin());
    act(() => mock.setHealth("local"));
    await waitFor(() => expect(screen.queryByText("Please sign in again.")).toBeNull());
    expect(useUi.getState().signInOpen).toBe(false);
  });
});
