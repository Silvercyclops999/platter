import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { PlayerBar, SEEK_PAGE_STEP, wheelVolumeDelta } from "../shell/PlayerBar";
import { QueuePanel } from "../shell/QueuePanel";
import { connectPlayer, resetVolumeNudge, usePlayer } from "../stores/player";
import { body } from "./rows";

const mock = backend as MockBackend;
let disconnect: () => void;

beforeAll(async () => {
  disconnect = connectPlayer(mock);
  await waitFor(() => expect(usePlayer.getState().state?.track).toBeTruthy());
});
afterAll(() => disconnect());
afterEach(() => {
  vi.restoreAllMocks();
  resetVolumeNudge();
});

/** Give a slider's track a 100 px wide box at x = 0. */
function sizeTrack(slider: HTMLElement) {
  const track = slider.querySelector(".slider__track") as HTMLElement;
  track.getBoundingClientRect = () => ({ left: 0, width: 100, top: 0, height: 4, right: 100, bottom: 4, x: 0, y: 0, toJSON() {} });
}

async function setVolumeState(volume: number, muted: boolean) {
  await act(async () => {
    await mock.setVolume(volume);
    await mock.setMuted(muted);
  });
  await waitFor(() => {
    const s = usePlayer.getState().state!;
    expect(s.volume).toBe(volume);
    expect(s.muted).toBe(muted);
  });
}

async function expectCore(volume: number, muted: boolean) {
  await waitFor(async () => {
    const s = await mock.playerState();
    expect(s.volume).toBeCloseTo(volume, 4);
    expect(s.muted).toBe(muted);
  });
}

const volumeSlider = () => screen.getByRole("slider", { name: "Volume" });

describe("volume and mute", () => {
  it("dragging to 0 mutes, and unmute restores the level from before the drag", async () => {
    const user = userEvent.setup();
    await setVolumeState(0.6, false);
    render(<PlayerBar />);
    const s = volumeSlider();
    sizeTrack(s);
    fireEvent.pointerDown(s, { button: 0, clientX: 30, pointerId: 1 });
    fireEvent.pointerMove(s, { clientX: 10, pointerId: 1 });
    fireEvent.pointerMove(s, { clientX: -40, pointerId: 1 });
    fireEvent.pointerUp(s, { clientX: -40, pointerId: 1 });
    await expectCore(0.6, true);
    await user.click(await screen.findByRole("button", { name: "Unmute" }));
    await expectCore(0.6, false);
    await waitFor(() => expect(volumeSlider()).toHaveAttribute("aria-valuenow", "0.6"));
    expect(screen.getByRole("button", { name: "Mute" })).toBeInTheDocument();
  });

  it("clicking the slider while muted unmutes at the new level, and mute cycles keep it", async () => {
    const user = userEvent.setup();
    await setVolumeState(0.6, true);
    const muteSpy = vi.spyOn(backend, "setMuted");
    render(<PlayerBar />);
    const s = volumeSlider();
    sizeTrack(s);
    fireEvent.pointerDown(s, { button: 0, clientX: 25, pointerId: 1 });
    // The press itself unmutes, before letting go.
    await waitFor(() => expect(muteSpy).toHaveBeenCalledWith(false));
    fireEvent.pointerUp(s, { clientX: 25, pointerId: 1 });
    await expectCore(0.25, false);
    await user.click(await screen.findByRole("button", { name: "Mute" }));
    await expectCore(0.25, true);
    await user.click(await screen.findByRole("button", { name: "Unmute" }));
    await expectCore(0.25, false);
  });

  it("wheel up while muted unmutes counting up from 0", async () => {
    await setVolumeState(0.6, true);
    render(<PlayerBar />);
    fireEvent.wheel(volumeSlider(), { deltaY: -100 });
    await expectCore(0.1, false);
  });

  it("wheel down to 0 mutes and remembers the last level", async () => {
    await setVolumeState(0.1, false);
    render(<PlayerBar />);
    fireEvent.wheel(volumeSlider(), { deltaY: 100 });
    await expectCore(0.1, true);
    expect(await screen.findByRole("button", { name: "Unmute" })).toBeInTheDocument();
    // Lowering further while muted does nothing.
    const vol = vi.spyOn(backend, "setVolume");
    resetVolumeNudge();
    fireEvent.wheel(volumeSlider(), { deltaY: 100 });
    expect(vol).not.toHaveBeenCalled();
  });

  it("the speaker button shows unmute at level 0 and unmuting restores a usable level", async () => {
    await setVolumeState(0, false);
    const user = userEvent.setup();
    render(<PlayerBar />);
    await user.click(screen.getByRole("button", { name: "Unmute" }));
    await expectCore(0.5, false);
  });

  it("slider keys move 10 points; Home counts as muted", async () => {
    const user = userEvent.setup();
    await setVolumeState(0.5, false);
    const vol = vi.spyOn(backend, "setVolume");
    render(<PlayerBar />);
    volumeSlider().focus();
    await user.keyboard("{ArrowUp}");
    expect(vol).toHaveBeenLastCalledWith(0.6);
    await expectCore(0.6, false);
    await user.keyboard("{PageDown}");
    await expectCore(0.5, false);
    await user.keyboard("{Home}");
    await expectCore(0.5, true);
    await user.keyboard("{End}");
    await expectCore(1, false);
  });
});

describe("wheel deltas", () => {
  it("a mouse notch is 10 points, wheel up is louder", () => {
    expect(wheelVolumeDelta({ deltaX: 0, deltaY: -100, deltaMode: 0 })).toBeCloseTo(0.1);
    expect(wheelVolumeDelta({ deltaX: 0, deltaY: 100, deltaMode: 0 })).toBeCloseTo(-0.1);
    expect(wheelVolumeDelta({ deltaX: 0, deltaY: -3, deltaMode: 1 })).toBeCloseTo(0.1);
  });

  it("trackpad deltas are proportional and capped at 10 points per event", () => {
    expect(wheelVolumeDelta({ deltaX: 0, deltaY: -20, deltaMode: 0 })).toBeCloseTo(0.02);
    expect(wheelVolumeDelta({ deltaX: 0, deltaY: -400, deltaMode: 0 })).toBeCloseTo(0.1);
    expect(wheelVolumeDelta({ deltaX: 0, deltaY: 0, deltaMode: 0 })).toBe(0);
  });
});

describe("seek bar", () => {
  beforeEach(async () => {
    const s = await mock.playerState();
    if (s.status === "playing") await act(() => mock.playPause());
  });

  it("pressing seeks at once, the time holds while dragging, release commits, Escape doesn't cancel", async () => {
    const seek = vi.spyOn(backend, "playSeek").mockResolvedValue(undefined); // keep the store in control
    render(<PlayerBar />);
    act(() => usePlayer.getState().setPosition(10_000, 200_000));
    const s = screen.getByRole("slider", { name: "Seek" });
    sizeTrack(s);
    fireEvent.pointerDown(s, { button: 0, clientX: 25, pointerId: 1 });
    expect(seek).toHaveBeenCalledWith(50_000);
    expect(screen.queryByRole("tooltip")).toBeNull();
    fireEvent.pointerMove(s, { clientX: 75, pointerId: 1 });
    expect(seek).toHaveBeenCalledTimes(1);
    expect(screen.getByText("2:30")).toBeInTheDocument();
    // Position ticks don't move the time while the button is held.
    act(() => usePlayer.getState().setPosition(51_000, 200_000));
    expect(screen.getByText("2:30")).toBeInTheDocument();
    fireEvent.keyDown(s, { key: "Escape" });
    expect(s).toHaveClass("is-dragging");
    fireEvent.pointerUp(s, { clientX: 75, pointerId: 1 });
    expect(seek).toHaveBeenLastCalledWith(150_000);
    expect(s).not.toHaveClass("is-dragging");
  });

  it("keys: arrows 5 s, Page Up/Down 25 s, Home/End", async () => {
    const user = userEvent.setup();
    const seek = vi.spyOn(backend, "playSeek").mockResolvedValue(undefined); // keep the store in control
    render(<PlayerBar />);
    act(() => usePlayer.getState().setPosition(60_000, 200_000));
    screen.getByRole("slider", { name: "Seek" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(seek).toHaveBeenLastCalledWith(65_000);
    await user.keyboard("{PageUp}");
    expect(seek).toHaveBeenLastCalledWith(65_000 + SEEK_PAGE_STEP);
    await user.keyboard("{PageDown}{PageDown}");
    expect(seek).toHaveBeenLastCalledWith(40_000);
    await user.keyboard("{Home}");
    expect(seek).toHaveBeenLastCalledWith(0);
    await user.keyboard("{End}");
    expect(seek).toHaveBeenLastCalledWith(200_000);
  });
});

describe("clear queue", () => {
  it("asks first; cancel keeps the queue, confirm clears only the user queue", async () => {
    const user = userEvent.setup();
    const album = mock.fx.albums.find((a) => a.trackCount >= 4)!;
    const tracks = mock.fx.tracksByAlbum.get(album.id)!;
    const other = mock.fx.tracks.find((t) => t.albumId !== album.id)!;
    await act(() => mock.playContext({ context: { kind: "album", id: album.id } }));
    await act(() => mock.queueAdd([other.id], "end"));
    await waitFor(() => expect(usePlayer.getState().state!.queue.userQueue).toHaveLength(1));
    const clear = vi.spyOn(backend, "queueClear");

    render(<QueuePanel />);
    await user.click(screen.getByRole("button", { name: "Clear queue" }));
    let dialog = screen.getByRole("dialog", { name: "Clear your queue?" });
    expect(dialog).toHaveTextContent("removes the songs you added to your queue");
    expect(dialog).toHaveTextContent("can't undo");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(clear).not.toHaveBeenCalled();
    expect(screen.getByRole("region", { name: "Next in queue" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Clear queue" }));
    dialog = screen.getByRole("dialog", { name: "Clear your queue?" });
    await user.click(within(dialog).getByRole("button", { name: "Clear queue" }));
    expect(clear).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.queryByRole("region", { name: "Next in queue" })).toBeNull());
    expect(screen.queryByRole("dialog")).toBeNull();
    const next = screen.getByRole("region", { name: `Next from: ${album.title}` });
    expect(body(within(next).getAllByRole("row"))).toHaveLength(tracks.length - 1);
  });
});
