/**
 * Songs are dimmed only when they truly can't play, and a dimmed list always
 * says why. Every rule that can dim a phone song row is faked here.
 */
import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { demoHealth, HEALTH_DEMOS, type HealthDemo } from "../lib/mock/health";
import type { Health } from "../lib/types";
import { CachedOnlyStrip } from "../phone/PhoneShell";
import { SongRow, songCantPlay } from "../phone/pages/common";
import { usePlayer } from "../stores/player";
import { cachedOnlyReason } from "../shell/bannerView";
import { connectApp, useApp } from "../stores/app";

const mock = backend as MockBackend;
const SERVER = { name: "Basement Server", version: "1.42.2" };
const NOW = 1_760_000_000_000;
const h = (d: HealthDemo) => demoHealth(d, SERVER, NOW);
const row = { playing: false, downloaded: false };

describe("which phone song rows are dimmed", () => {
  it("nothing is dimmed while the server can be reached, on any route", () => {
    for (const d of ["local", "remote", "relay", "scanning", "diskFull", "noMusicLibrary"] as HealthDemo[])
      expect(songCantPlay("t1", { health: h(d), cached: new Set() }, row), d).toBe(false);
  });

  it("only cached music playing: songs not on the phone are dimmed", () => {
    for (const d of ["offline", "noInternet", "filesUnavailable", "goneOffline", "starting"] as HealthDemo[])
      expect(songCantPlay("t1", { health: h(d), cached: new Set() }, row), d).toBe(true);
  });

  it("a downloaded song, one in the audio cache, and the playing song stay lit", () => {
    const s = { health: h("offline"), cached: new Set(["cached"]) };
    expect(songCantPlay("t1", s, { playing: false, downloaded: true })).toBe(false);
    expect(songCantPlay("cached", s, row)).toBe(false);
    expect(songCantPlay("t1", s, { playing: true, downloaded: false }), "it is playing, so it can play").toBe(false);
  });

  it("while the list of songs on the phone isn't known, nothing is dimmed", () => {
    expect(songCantPlay("t1", { health: h("offline"), cached: null }, row)).toBe(false);
  });

  it("a playable list on the phone is full strength, the playing song too", () => {
    const tracks = mock.fx.tracks.slice(0, 3);
    act(() => {
      useApp.setState({ health: h("local"), cached: new Set() });
      usePlayer.setState({ state: { ...(usePlayer.getState().state ?? {}), track: tracks[0], status: "playing" } as never });
    });
    const { container } = render(
      <ul className="ph-songs">
        {tracks.map((t, i) => (
          <SongRow key={t.id} track={t} index={i + 1} art={false} onPlay={() => {}} />
        ))}
      </ul>,
    );
    expect(container.querySelectorAll(".ph-song")).toHaveLength(3);
    expect(container.querySelectorAll(".is-unplayable")).toHaveLength(0);
    // Server gone: the others dim, the playing one doesn't.
    act(() => useApp.setState({ health: h("offline") }));
    expect(container.querySelectorAll(".is-unplayable")).toHaveLength(2);
    act(() => usePlayer.setState({ state: null }));
  });

  it("no health answer yet dims nothing", () => {
    expect(songCantPlay("t1", { health: null, cached: new Set() }, row)).toBe(false);
  });
});

describe("why songs are dimmed", () => {
  it("every state that dims songs says why, and the rest say nothing", () => {
    for (const d of HEALTH_DEMOS) {
      const x = h(d);
      const reason = cachedOnlyReason(x, "phone");
      if (x.cachedOnly) expect(reason, d).toMatch(/\S/);
      else expect(reason, d).toBeNull();
    }
    expect(cachedOnlyReason(h("offline"), "phone")).toBe("Can't reach your server, so only songs on this phone play. Platter reconnects by itself.");
    expect(cachedOnlyReason(h("goneOffline"), "phone")).toBe("You chose Go offline, so only songs on this phone play.");
    expect(cachedOnlyReason(h("filesUnavailable"))).toBe(
      "Your server is up, but your music files aren't available, so only songs on this computer play.",
    );
  });

  it("the phone shows the reason over the tabs and takes it away on reconnect", () => {
    act(() => useApp.setState({ health: h("offline") }));
    render(<CachedOnlyStrip />);
    expect(screen.getByTestId("phone-offline-strip")).toHaveTextContent("Can't reach your server");
    act(() => useApp.setState({ health: h("local") }));
    expect(screen.queryByTestId("phone-offline-strip")).toBeNull();
  });
});

describe("health at startup", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    act(() => useApp.setState({ health: null }));
  });

  it("a change that arrives before the first answer isn't undone by it", async () => {
    let answer: (x: Health) => void = () => {};
    vi.spyOn(mock, "health").mockImplementation(() => new Promise<Health>((r) => (answer = r)));
    const off = connectApp(mock);
    // Let the listener register, then the server comes back.
    await waitFor(() => expect(mock.health).toHaveBeenCalled());
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
      mock.setHealth("local");
    });
    await waitFor(() => expect(useApp.getState().health?.status).toBe("local"));
    // The first answer, from while it was still offline, comes in late.
    await act(async () => answer(h("offline")));
    expect(useApp.getState().health?.cachedOnly).toBe(false);
    off();
  });
});
