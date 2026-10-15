import { act, render, waitFor } from "@testing-library/react";
import { Profiler, type ProfilerOnRenderCallback } from "react";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { MainView } from "../shell/MainView";
import { NowPlayingPanel } from "../shell/NowPlayingPanel";
import { PlayerBar } from "../shell/PlayerBar";
import { QueuePanel } from "../shell/QueuePanel";
import { Sidebar } from "../shell/Sidebar";
import { TopBar } from "../shell/TopBar";
import { connectPlayer, usePlayer } from "../stores/player";

const mock = backend as MockBackend;
let disconnect: () => void;

beforeAll(async () => {
  disconnect = connectPlayer(mock);
  await waitFor(() => expect(usePlayer.getState().state?.track).toBeTruthy());
});
afterAll(() => disconnect());

/** Commits per profiled region, like the React DevTools profiler shows. */
function counter() {
  const commits = new Map<string, number>();
  const onRender: ProfilerOnRenderCallback = (id, phase) => {
    if (phase !== "mount") commits.set(id, (commits.get(id) ?? 0) + 1);
  };
  return { commits, onRender, reset: () => commits.clear() };
}

describe("render budget", () => {
  it("a position tick re-renders only the seek bar, not the rest of the window", async () => {
    const c = counter();
    const region = (id: string, el: React.ReactNode) => (
      <Profiler id={id} onRender={c.onRender}>
        {el}
      </Profiler>
    );
    render(
      <>
        {region("top", <TopBar />)}
        {region("sidebar", <Sidebar collapsed={false} auto={false} />)}
        {region("main", <MainView />)}
        {region("queue", <QueuePanel />)}
        {region("nowPlaying", <NowPlayingPanel />)}
        {region("bar", <PlayerBar />)}
      </>,
    );
    // Let the first loads land, then count only what the ticks cause.
    await act(async () => {
      await new Promise((r) => setTimeout(r, 300));
    });
    c.reset();
    const { durationMs } = usePlayer.getState();
    const dur = Math.max(durationMs, 200_000);
    for (let i = 1; i <= 8; i++) {
      act(() => usePlayer.getState().setPosition(1_000 * i, dur));
    }
    expect(c.commits.get("bar") ?? 0).toBeGreaterThan(0);
    for (const quiet of ["top", "sidebar", "main", "queue", "nowPlaying"]) {
      expect(c.commits.get(quiet) ?? 0, `${quiet} re-rendered on a position tick`).toBe(0);
    }
  });
});
