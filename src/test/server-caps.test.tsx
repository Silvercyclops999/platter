/**
 * Server capabilities: features that need one hide when the item's server
 * doesn't have it, and a Plex server has everything they need, so nothing
 * changes for Plex.
 */
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import { ALL_CAPS, CAP_NAMES, capFor, serverFor, tagOf } from "../lib/caps";
import type { MockBackend } from "../lib/mock";
import { plexCaps } from "../lib/mock/servers";
import type { BackendCaps, Caps, CapName } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { EditDetailsDialog } from "../components/PlaylistDialogs";
import { SettingsPage } from "../pages/SettingsPage";
import { TopBar } from "../shell/TopBar";
import { useApp } from "../stores/app";
import { ASSUMED_CAPS, ensureCaps, hasCap, useCaps } from "../stores/caps";
import { useSettings } from "../stores/settings";

const mock = backend as MockBackend;

function without(...off: CapName[]): Caps {
  const c = { ...ALL_CAPS };
  for (const n of off) c[n] = false;
  return c;
}

function one(caps: Caps, tag = "0a1b2c3d"): BackendCaps {
  return { caps, servers: [{ tag, kind: "plex", name: "Home", primary: true, caps }] };
}

beforeEach(async () => {
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useApp.setState({ auth: { state: "signedIn", username: "listener", needsServerPick: false } });
  mock.homeUsersMock.notAHome = false;
  // Let the first answer from the core land before a test sets its own.
  ensureCaps();
  await waitFor(() => expect(useCaps.getState().all.servers.length).toBe(1));
  useCaps.setState({ all: one(plexCaps()) });
});

describe("capability lookups", () => {
  it("finds an item's server by the tag its id carries", () => {
    expect(tagOf("0a1b2c3d:123")).toBe("0a1b2c3d");
    expect(tagOf("0a1b2c3d:/library/metadata/1/thumb/2")).toBe("0a1b2c3d");
    expect(tagOf("123")).toBeNull();
    expect(tagOf("local:abc")).toBeNull();
    const plex = { tag: "0a1b2c3d", kind: "plex" as const, name: "Home", primary: true, caps: ALL_CAPS };
    const other = { tag: "ffffffff", kind: "jellyfin" as const, name: "Den", primary: false, caps: without("lyrics") };
    const all: BackendCaps = { caps: ALL_CAPS, servers: [plex, other] };
    expect(serverFor(all, "ffffffff:9")?.name).toBe("Den");
    expect(serverFor(all, "9")?.name).toBe("Home");
    // An unknown tag belongs to the primary's account.
    expect(serverFor(all, "12345678:9")?.name).toBe("Home");
    expect(capFor(all, "ffffffff:9", "lyrics")).toBe(false);
    expect(capFor(all, "0a1b2c3d:9", "lyrics")).toBe(true);
    expect(capFor({ caps: without("sonic"), servers: [] }, "x", "sonic")).toBe(false);
  });

  it("assumes everything until the core answers, and Plex keeps every feature Platter shows", () => {
    expect(ASSUMED_CAPS.caps).toEqual(ALL_CAPS);
    expect(Object.keys(ALL_CAPS).sort()).toEqual([...CAP_NAMES].sort());
    const plex = plexCaps();
    const off = CAP_NAMES.filter((c) => !plex[c]);
    expect(off.sort()).toEqual(["backdatedScrobble", "resumeQueue", "serverRadio"]);
    for (const c of ["metadataEdit", "homeUsers", "remotePlayers", "friendActivity", "realFilePaths", "trackDetails", "serverSmartPlaylists"] as const) {
      expect(hasCap(c, "0a1b2c3d:1")).toBe(true);
    }
  });

  it("asks the core what the servers can do, with their names", async () => {
    const caps = await backend.backendCaps();
    expect(caps.servers).toEqual([expect.objectContaining({ kind: "plex", primary: true, caps: plexCaps() })]);
  });
});

describe("hidden without the capability", () => {
  it("the top bar has friend activity and Home users with Plex", async () => {
    const user = userEvent.setup();
    render(<TopBar />);
    expect(screen.getByRole("button", { name: "Friend activity" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Switch account" }));
    expect(await screen.findByRole("group", { name: "Switch Plex Home user" })).toBeInTheDocument();
  });

  it("the top bar drops them on a server without them", async () => {
    act(() => useCaps.setState({ all: one(without("friendActivity", "homeUsers")) }));
    const user = userEvent.setup();
    render(<TopBar />);
    expect(screen.queryByRole("button", { name: "Friend activity" })).toBeNull();
    await user.click(screen.getByRole("button", { name: /^App menu/ }));
    await screen.findByRole("menu");
    expect(screen.queryByRole("group", { name: "Switch Plex Home user" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "Switch account" })).toBeNull();
  });

  it("Settings leaves out rows the servers can't use", async () => {
    const { unmount } = render(<SettingsPage />);
    expect(document.querySelector('[data-setting-part="showPlexPlayers"]')).toBeTruthy();
    expect(document.querySelector('[data-setting="actAsPlexPlayer"]')).toBeTruthy();
    expect(document.querySelector('[data-setting="showFriendActivity"]')).toBeTruthy();
    unmount();
    act(() => useCaps.setState({ all: one(without("remotePlayers", "friendActivity")) }));
    render(<SettingsPage />);
    expect(document.querySelector('[data-setting-part="showPlexPlayers"]')).toBeNull();
    expect(document.querySelector('[data-setting="actAsPlexPlayer"]')).toBeNull();
    expect(document.querySelector('[data-setting="showFriendActivity"]')).toBeNull();
    expect(document.querySelector('[data-setting="shareActivity"]')).toBeTruthy();
  });

  it("edits only items on a server that can be edited", () => {
    act(() => useCaps.setState({ all: one(without("metadataEdit")) }));
    expect(hasCap("metadataEdit", "0a1b2c3d:5")).toBe(false);
    act(() => useCaps.setState({ all: one(ALL_CAPS) }));
    expect(hasCap("metadataEdit", "0a1b2c3d:5")).toBe(true);
  });

  it("a playlist's details offer a cover and a description only where its server keeps them", () => {
    const pl = { id: "0a1b2c3d:77", title: "Road", smart: false, trackCount: 1, durationMs: 0, thumb: null, summary: null, updatedAt: 1 };
    const { unmount } = render(<EditDetailsDialog playlist={pl} onClose={() => {}} />);
    expect(screen.getByRole("button", { name: "Choose a cover image" })).toBeInTheDocument();
    expect(screen.getByLabelText(/^Description/)).toBeInTheDocument();
    unmount();
    act(() => useCaps.setState({ all: one(without("playlistPoster", "playlistSummary")) }));
    render(<EditDetailsDialog playlist={pl} onClose={() => {}} />);
    expect(screen.queryByRole("button", { name: "Choose a cover image" })).toBeNull();
    expect(screen.queryByLabelText(/^Description/)).toBeNull();
    expect(screen.getByLabelText(/^Name/)).toBeInTheDocument();
  });
});
