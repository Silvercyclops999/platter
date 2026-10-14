/**
 * Listen on: Plex players in the device picker, moving playback there,
 * remote mode in the player bar ("Playing on" strip, controls sent to the
 * player), coming back here, the settings, and the app states (player
 * unreachable, player stopped by someone else).
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockPlatterDevices, mockPlayers } from "../lib/mock/remote";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { AppShell } from "../shell/AppShell";
import { playerSub } from "../shell/ListenOn";
import { connectApp } from "../stores/app";
import { connectPlayer, player, usePlayer, withRemote } from "../stores/player";
import { useRemote } from "../stores/remote";
import { useSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";

const mock = backend as MockBackend;

let disconnect: () => void;
beforeAll(async () => {
  const a = connectApp(mock);
  const p = connectPlayer(mock);
  disconnect = () => {
    a();
    p();
  };
  await waitFor(() => expect(usePlayer.getState().state).toBeTruthy());
});
afterAll(() => disconnect());

beforeEach(async () => {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
  localStorage.clear();
  // Back here, no note, starting players.
  if (usePlayer.getState().remote) await act(() => mock.remoteReturn().then((r) => usePlayer.getState().setRemote(r)));
  act(() => usePlayer.getState().setRemote({ session: null, stoppedOn: null }));
  await act(() => mock.playPause());
  mock.remote.setPlayers(mockPlayers(mock.fx.tracks[7]));
  mock.remote.setPlatterDevices(mockPlatterDevices());
  await mock.settingsSet({ showPlexPlayers: true, moveStopsOldDevice: true });
  useSettings.setState({ settings: { ...DEFAULT_SETTINGS } });
  useRemote.setState({ players: null, busy: undefined });
  useUi.setState({ queueOpen: false, nowPlayingOpen: false, nowPlayingExpanded: false, outputOpen: true, outputReturn: null, menu: null });
  useToasts.setState({ toasts: [] });
});

const picker = () => screen.getByRole("complementary", { name: "Connect to a device" });
const listenOn = () => within(picker()).getByRole("region", { name: "Listen on" });
const findListenOn = () => within(picker()).findByRole("region", { name: "Listen on" });
const playHere = () => act(() => mock.playResume());

async function moveToDesk() {
  render(<AppShell />);
  const row = await within(picker()).findByRole("button", { name: "Play on Studio Desk" });
  fireEvent.click(row);
  await screen.findByRole("button", { name: /Playing on Studio Desk/ });
}

describe("the Listen on section", () => {
  it("lists Plex players with their icon kind and what each is playing", async () => {
    render(<AppShell />);
    const section = await findListenOn();
    expect(within(section).getByRole("heading", { name: "Listen on" })).toBeInTheDocument();
    const kitchen = await within(section).findByRole("button", { name: "Play on Kitchen" });
    const sample = mock.fx.tracks[7];
    expect(kitchen).toHaveTextContent(sample.title);
    expect(within(section).getByRole("button", { name: "Play on Studio Desk" })).toHaveTextContent("Plexamp");
    expect(within(section).getByRole("button", { name: "Play on Living Room TV" })).toHaveTextContent("Not active");
    // No "This computer" row while playing here.
    expect(within(section).queryByRole("button", { name: "Play on this computer" })).toBeNull();
  });

  it("says when no players are found", async () => {
    mock.remote.setPlayers([]);
    mock.remote.setLanDevices([]);
    mock.remote.setPlatterDevices([]);
    render(<AppShell />);
    expect(await within(await findListenOn()).findByText(/No devices found/)).toBeInTheDocument();
    mock.remote.resetLan();
  });

  it("is hidden when the setting is off", async () => {
    mock.remote.setPlatterDevices([]);
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, showPlexPlayers: false, showCastDevices: false } }));
    render(<AppShell />);
    await within(picker()).findByRole("heading", { name: "This computer" });
    expect(within(picker()).queryByRole("region", { name: "Listen on" })).toBeNull();
  });

  it("grey lines: playing, product, or not active", () => {
    const [desk, kitchen, , tv] = mockPlayers(mock.fx.tracks[7]);
    expect(playerSub(desk)).toBe("Plexamp");
    expect(playerSub(kitchen)).toContain("•");
    expect(playerSub(tv)).toBe("Plex for Samsung • Not active");
  });
});

describe("moving playback to a player", () => {
  it("pauses here, shows the strip and the device as current", async () => {
    await playHere();
    await waitFor(() => expect(usePlayer.getState().local?.status).toBe("playing"));
    const before = usePlayer.getState().local!;
    await moveToDesk();
    expect(usePlayer.getState().local?.status).toBe("paused");
    const s = usePlayer.getState();
    expect(s.remote?.playerName).toBe("Studio Desk");
    expect(s.state?.track?.id).toBe(before.track?.id);
    expect(s.remote!.positionMs).toBeGreaterThanOrEqual(before.positionMs);
    const current = within(picker()).getByRole("group", { name: "Listening on" });
    expect(current).toHaveTextContent("Studio Desk");
    // This computer is offered to come back.
    expect(within(listenOn()).getByRole("button", { name: "Play on this computer" })).toBeInTheDocument();
    expect(within(listenOn()).queryByRole("button", { name: "Play on Studio Desk" })).toBeNull();
  });

  it("the strip opens the device picker", async () => {
    await moveToDesk();
    act(() => useUi.getState().setOutputOpen(false));
    fireEvent.click(screen.getByRole("button", { name: /Playing on Studio Desk/ }));
    expect(useUi.getState().outputOpen).toBe(true);
  });

  it("keeps playing here when the setting says not to stop the old device", async () => {
    act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, moveStopsOldDevice: false } }));
    await mock.settingsSet({ moveStopsOldDevice: false });
    await playHere();
    await waitFor(() => expect(usePlayer.getState().local?.status).toBe("playing"));
    await moveToDesk();
    expect(usePlayer.getState().local?.status).toBe("playing");
  });
});

describe("remote mode", () => {
  it("player bar controls go to the player", async () => {
    await moveToDesk();
    await waitFor(() => expect(usePlayer.getState().state?.status).toBe("playing"), { timeout: 2000 });
    const bar = screen.getByRole("contentinfo", { name: "Player" });
    fireEvent.click(within(bar).getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(mock.remote.state().session?.status).toBe("paused"));
    expect(usePlayer.getState().local?.status).toBe("paused");

    await act(() => player.seek(30_000));
    expect(mock.remote.state().session?.positionMs).toBe(30_000);
    expect(usePlayer.getState().positionMs).toBe(30_000);

    await act(() => player.setShuffle(true));
    expect(mock.remote.state().session?.shuffle).toBe(true);
    await act(() => player.cycleRepeat("off"));
    expect(mock.remote.state().session?.repeat).toBe("all");
    await act(() => player.setVolume(0.3));
    expect(mock.remote.state().session?.volume).toBe(0.3);
    await act(() => player.toggleMute());
    expect(mock.remote.state().session?.volume).toBe(0);
    await act(() => player.toggleMute());
    expect(mock.remote.state().session?.volume).toBe(0.3);

    const first = usePlayer.getState().state?.track?.id;
    await act(() => player.next());
    expect(usePlayer.getState().state?.track?.id).not.toBe(first);
  });

  it("local position ticks don't move the bar while remote", async () => {
    await moveToDesk();
    const pos = usePlayer.getState().positionMs;
    act(() => usePlayer.getState().setPosition(pos + 99_000, 300_000));
    expect(usePlayer.getState().positionMs).toBe(pos);
  });

  it("playing an album while remote sends it to the same player", async () => {
    await moveToDesk();
    const album = mock.fx.albums[3];
    await act(() => player.playAlbum(album.id));
    const s = usePlayer.getState();
    expect(s.remote?.playerName).toBe("Studio Desk");
    expect(s.state?.track?.albumId).toBe(album.id);
    expect(s.local?.status).not.toBe("playing");
  });

  it("the merge keeps the local snapshot under the remote one", () => {
    const local = usePlayer.getState().local!;
    expect(withRemote(local, null)).toBe(local);
    const merged = withRemote(local, {
      playerId: "p",
      playerName: "P",
      kind: "speaker",
      status: "connecting",
      track: null,
      positionMs: 5,
      durationMs: 0,
      volume: null,
      shuffle: true,
      repeat: "one",
    })!;
    expect(merged.status).toBe("loading");
    expect(merged.track).toEqual(local.track);
    expect(merged.queue).toEqual(local.queue);
    expect([merged.positionMs, merged.shuffle, merged.repeat, merged.abLoop]).toEqual([5, true, "one", null]);
  });
});

describe("coming back to this computer", () => {
  it("resumes here at the player's song and position", async () => {
    await moveToDesk();
    await act(() => player.seek(45_000));
    fireEvent.click(within(listenOn()).getByRole("button", { name: "Play on this computer" }));
    await waitFor(() => expect(usePlayer.getState().remote).toBeNull());
    const local = usePlayer.getState().local!;
    expect(local.positionMs).toBe(45_000);
    expect(screen.queryByRole("button", { name: /Playing on/ })).toBeNull();
  });
});

describe("app states", () => {
  it("player unreachable: a refused move says so and playback stays here", async () => {
    render(<AppShell />);
    fireEvent.click(await within(await findListenOn()).findByRole("button", { name: "Play on Living Room TV" }));
    await waitFor(() => expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Couldn't reach Living Room TV"));
    expect(usePlayer.getState().remote).toBeNull();
  });

  it("player unreachable mid-session: toast, back here paused at the last position", async () => {
    await moveToDesk();
    await act(() => player.seek(20_000));
    act(() => mock.remote.loseRemote());
    await waitFor(() => expect(usePlayer.getState().remote).toBeNull());
    expect(useToasts.getState().toasts.map((t) => t.message)).toContain("Couldn't reach Studio Desk");
    expect(usePlayer.getState().local?.status).toBe("paused");
    expect(usePlayer.getState().local?.positionMs).toBe(20_000);
  });

  it("player stopped by someone else: the strip says so and controls are local again", async () => {
    await moveToDesk();
    act(() => mock.remote.stopElsewhere());
    expect(await screen.findByRole("status")).toHaveTextContent("Stopped on Studio Desk");
    expect(usePlayer.getState().remote).toBeNull();
    // Play now plays here.
    await act(() => player.toggle());
    await waitFor(() => expect(usePlayer.getState().local?.status).toBe("playing"));
  });
});

describe("settings", () => {
  it("has the three Listen on rows under Playback", async () => {
    render(<SettingsPage />);
    for (const title of [
      "Show Plex players in the device picker",
      "Stop on the old device when moving playback",
      "Let other Plex apps play on this computer",
    ]) {
      expect(await screen.findByText(title)).toBeInTheDocument();
    }
    expect(screen.getAllByText(/Opens a network port/)[0]).toHaveTextContent("This device only.");
    expect(DEFAULT_SETTINGS.showPlexPlayers && !DEFAULT_SETTINGS.actAsPlexPlayer && DEFAULT_SETTINGS.moveStopsOldDevice).toBe(true);
  });
});
