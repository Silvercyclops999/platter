/**
 * The sleep timer on the computer: the moon in the player bar opens the
 * choices, sets the core's timer, lights while it runs with a tooltip
 * saying what's left, turns it off, follows the core's event, and hides
 * while this window controls another device.
 */
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { SleepButton } from "../shell/SleepButton";
import { ContextMenuHost } from "../components/ContextMenu";
import { usePlayer } from "../stores/player";
import { followSleep, resetSleep, sleepTip, useSleep } from "../stores/sleep";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

beforeEach(async () => {
  resetSleep();
  useToasts.setState({ toasts: [] });
  await followSleep();
});
afterEach(async () => {
  vi.restoreAllMocks();
  await backend.sleepCancel();
  resetSleep();
});

function renderBar() {
  return render(
    <>
      <SleepButton />
      <ContextMenuHost />
    </>,
  );
}

describe("the moon in the player bar", () => {
  it("sets a timer in minutes and lights with what's left", async () => {
    const user = userEvent.setup();
    const set = vi.spyOn(backend, "sleepSet");
    renderBar();
    const moon = screen.getByRole("button", { name: "Sleep timer" });
    expect(moon).toHaveAttribute("aria-pressed", "false");
    await user.click(moon);
    await user.click(await screen.findByRole("menuitemradio", { name: "30 minutes" }));
    expect(set).toHaveBeenCalledWith({ kind: "minutes", minutes: 30 });
    const lit = await screen.findByRole("button", { name: /^Sleep timer: Stops in (29|30) min$/ });
    expect(lit).toHaveAttribute("aria-pressed", "true");
  });

  it("offers the end of the song and album only while something plays, and turns off", async () => {
    const user = userEvent.setup();
    const state = await mock.playerState();
    act(() => usePlayer.getState().set({ ...state, track: null }));
    renderBar();
    await user.click(screen.getByRole("button", { name: "Sleep timer" }));
    expect(await screen.findByRole("menuitemradio", { name: "End of this song" })).toHaveAttribute("aria-disabled", "true");
    await user.keyboard("{Escape}");
    act(() => usePlayer.getState().set(state));
    await user.click(screen.getByRole("button", { name: "Sleep timer" }));
    await user.click(await screen.findByRole("menuitemradio", { name: "End of this song" }));
    await screen.findByRole("button", { name: "Sleep timer: Stops after this song" });
    await user.click(screen.getByRole("button", { name: "Sleep timer: Stops after this song" }));
    await user.click(await screen.findByRole("menuitem", { name: "Turn off the timer" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Sleep timer" })).toHaveAttribute("aria-pressed", "false"));
    expect(await backend.sleepGet()).toBeNull();
  });

  it("follows the core: a timer that runs out clears the moon", async () => {
    renderBar();
    act(() => mock.emit("sleep://changed", { kind: "album", albumId: "a1" }));
    expect(screen.getByRole("button", { name: "Sleep timer: Stops after this album" })).toBeInTheDocument();
    act(() => mock.emit("sleep://changed", null));
    expect(screen.getByRole("button", { name: "Sleep timer" })).toHaveAttribute("aria-pressed", "false");
  });

  it("a refused timer says why and stays off", async () => {
    const user = userEvent.setup();
    vi.spyOn(backend, "sleepSet").mockRejectedValue({ kind: "invalid", message: "Nothing is playing." });
    renderBar();
    await user.click(screen.getByRole("button", { name: "Sleep timer" }));
    await user.click(await screen.findByRole("menuitemradio", { name: "5 minutes" }));
    await waitFor(() => expect(useToasts.getState().toasts.slice(-1)[0]?.message).toBe("Nothing is playing."));
    expect(useSleep.getState().timer).toBeNull();
  });

  it("hides while this window controls another device", async () => {
    const remote = { deviceId: "d1" } as unknown as NonNullable<ReturnType<typeof usePlayer.getState>["remote"]>;
    act(() => usePlayer.setState({ remote }));
    renderBar();
    expect(screen.queryByRole("button", { name: /Sleep timer/ })).not.toBeInTheDocument();
    act(() => usePlayer.setState({ remote: null }));
  });

  it("words the tooltip", () => {
    expect(sleepTip(null)).toBe("Sleep timer");
    expect(sleepTip({ kind: "minutes", minutes: 60, endsAtMs: 3_600_000 }, 0)).toBe("Stops in 1 h");
    expect(sleepTip({ kind: "track", trackId: "t" })).toBe("Stops after this song");
  });
});
