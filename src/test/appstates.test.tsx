/**
 * App states: every banner the core can send, what the UI shows for it, and
 * the behavior around it (sign in again, dimming what can't play, toasts).
 * Each state is faked through the mock backend or the store.
 */
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import App from "../App";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { demoHealth, HEALTH_DEMOS, type HealthDemo } from "../lib/mock/health";
import type { Health } from "../lib/types";
import { agoText, healthBanner } from "../shell/bannerView";
import { HealthBanner } from "../shell/HealthBanner";
import { routeBadge, statusInfo } from "../shell/StatusDot";
import { connectApp, isUnavailable, useApp } from "../stores/app";
import { PlayerBar } from "../shell/PlayerBar";
import { connectPlayer, usePlayer } from "../stores/player";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

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
  offline: { kind: "offline", message: "Your server is offline. Last seen 4 min ago." },
  noInternet: { kind: "offline", message: "You're offline." },
  starting: { kind: "progress", message: "Server is starting…" },
  updating: { kind: "progress", message: "Server is updating…" },
  startTimeout: { kind: "error", message: "Your server is taking too long to come back. Check that Plex is running." },
  signIn: { kind: "warning", message: "Please sign in again.", action: "signIn" },
  noAuth: { kind: "warning", message: "Please sign in again.", action: "signIn" },
  accessRemoved: { kind: "error", message: "You no longer have access to this server.", action: "signOut" },
  filesUnavailable: { kind: "error", message: "Your server is up, but your music files aren't available." },
  noMusicLibrary: { kind: "info", message: "No music library found.", action: "libraryGuide" },
  diskFull: { kind: "warning", message: "Not enough space." },
  scanning: { kind: "progress", message: "Adding new music… 120 of 340" },
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

  it("keeps 'Last seen' current", () => {
    expect(healthBanner(h("offline"), NOW + 6 * 60_000)!.message).toBe("Your server is offline. Last seen 10 min ago.");
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

  it("offline banners are alerts", () => {
    act(() => useApp.setState({ health: demoHealth("offline", SERVER) }));
    render(<HealthBanner />);
    expect(screen.getByRole("alert")).toHaveTextContent(/Your server is offline\. Last seen \d+ min ago\./);
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
    const rows = screen.getAllByRole("listitem");
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
