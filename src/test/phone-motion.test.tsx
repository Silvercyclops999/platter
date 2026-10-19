/**
 * How the phone moves and answers a touch: the press highlight, pages that
 * slide over each other and stay mounted until the move ends, the edge
 * swipe's thresholds, tabs that keep their scroll, reduced motion, haptics
 * behind their setting, sheets that slide away, and the status bar.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setReducedMotion } from "../design/motion";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type PlayerState, type Track } from "../lib/types";
import { HAPTIC_GAP_MS, resetHaptics, tap } from "../phone/bits";
import { usePhoneForm } from "../phone/form";
import { PRESSED, PRESS_MIN_MS, installTapGuard } from "../phone/gestures";
import { BACK_FRACTION, BACK_FLICK_SPEED, edgeClaims, edgeRelease } from "../phone/motion";
import { resetStackMemory } from "../phone/PageStack";
import { followStatusBar, PhoneShell, statusBarDark } from "../phone/PhoneShell";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { ADDED_NOTE_MS, addQuietly, setSheetLeaveMs } from "../phone/Sheets";
import { usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;

/** A stand-in for the Web Animations API whose animations end when the test says so. */
function fakeAnimations() {
  const made: { el: Element; frames: Keyframe[]; finish: () => void }[] = [];
  const animate = vi.fn(function (this: Element, frames: Keyframe[]) {
    let finish!: () => void;
    const finished = new Promise<void>((r) => (finish = r));
    made.push({ el: this, frames, finish });
    return { finished, cancel: () => {}, finish } as unknown as Animation;
  });
  (Element.prototype as unknown as { animate: unknown }).animate = animate;
  (Element.prototype as unknown as { getAnimations: unknown }).getAnimations = () => [];
  return {
    made,
    async finishAll() {
      await act(async () => {
        made.splice(0).forEach((m) => m.finish());
        await Promise.resolve();
      });
    },
  };
}

function removeFakeAnimations() {
  delete (Element.prototype as unknown as { animate?: unknown }).animate;
  delete (Element.prototype as unknown as { getAnimations?: unknown }).getAnimations;
}

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  usePhoneUi.setState({ tab: "home", nowPlaying: false, lyrics: false, sheets: [] });
  useRouter.getState().reset({ name: "home" });
  resetStackMemory();
  resetHaptics();
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

afterEach(() => {
  removeFakeAnimations();
  setReducedMotion(false);
  setSheetLeaveMs(0);
  usePhoneForm.setState({ phone: false });
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("press feedback", () => {
  function setup() {
    const root = document.createElement("div");
    root.innerHTML = `<div class="scroller"><button type="button" class="row">Row</button></div><div class="other"></div><button type="button" disabled class="off">Off</button>`;
    document.body.appendChild(root);
    const stop = installTapGuard(root);
    const row = root.querySelector<HTMLButtonElement>(".row")!;
    return { root, row, scroller: root.querySelector<HTMLElement>(".scroller")!, other: root.querySelector<HTMLElement>(".other")!, off: root.querySelector<HTMLButtonElement>(".off")!, stop };
  }

  it("lights at once on pointerdown and clears on pointerup, holding a quick tap just long enough to see", () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
    const { row, root, stop } = setup();
    fireEvent.pointerDown(row, { clientX: 10, clientY: 10, pointerId: 1 });
    expect(row).toHaveClass(PRESSED);
    fireEvent.pointerUp(row, { clientX: 10, clientY: 10, pointerId: 1 });
    expect(row).toHaveClass(PRESSED);
    vi.advanceTimersByTime(PRESS_MIN_MS);
    expect(row).not.toHaveClass(PRESSED);
    // Held longer than that, it clears the moment the finger lifts.
    fireEvent.pointerDown(row, { clientX: 10, clientY: 10, pointerId: 2 });
    vi.advanceTimersByTime(PRESS_MIN_MS + 10);
    fireEvent.pointerUp(row, { clientX: 10, clientY: 10, pointerId: 2 });
    expect(row).not.toHaveClass(PRESSED);
    stop();
    root.remove();
  });

  it("clears on pointercancel, on a move past the slop and on a scroll of what it is in", () => {
    const { row, scroller, root, stop } = setup();
    fireEvent.pointerDown(row, { clientX: 10, clientY: 10, pointerId: 1 });
    fireEvent.pointerCancel(row, { pointerId: 1 });
    expect(row).not.toHaveClass(PRESSED);

    fireEvent.pointerDown(row, { clientX: 10, clientY: 10, pointerId: 1 });
    fireEvent.pointerMove(row, { clientX: 10, clientY: 40, pointerId: 1 });
    expect(row).not.toHaveClass(PRESSED);
    fireEvent.pointerUp(row, { clientX: 10, clientY: 40, pointerId: 1 });

    fireEvent.pointerDown(row, { clientX: 10, clientY: 10, pointerId: 1 });
    fireEvent.scroll(scroller);
    expect(row).not.toHaveClass(PRESSED);
    const click = vi.fn();
    row.addEventListener("click", click);
    fireEvent.pointerUp(row, { clientX: 10, clientY: 10, pointerId: 1 });
    fireEvent.click(row);
    expect(click).not.toHaveBeenCalled();
    stop();
    root.remove();
  });

  it("a scroll somewhere else doesn't eat the tap, and a disabled control never lights", () => {
    const { row, other, off, root, stop } = setup();
    const click = vi.fn();
    row.addEventListener("click", click);
    fireEvent.pointerDown(row, { clientX: 10, clientY: 10, pointerId: 1 });
    fireEvent.scroll(other);
    expect(row).toHaveClass(PRESSED);
    fireEvent.pointerUp(row, { clientX: 10, clientY: 10, pointerId: 1 });
    fireEvent.click(row);
    expect(click).toHaveBeenCalledTimes(1);
    fireEvent.pointerDown(off, { clientX: 10, clientY: 10, pointerId: 3 });
    expect(off).not.toHaveClass(PRESSED);
    stop();
    root.remove();
  });
});

describe("the page stack", () => {
  const layers = () => document.querySelectorAll(".ph-layer");

  it("a pushed page slides in over the one before, and both stay until the slide ends", async () => {
    const anims = fakeAnimations();
    render(<PhoneShell />);
    const album = mock.fx.albums[0];
    act(() => useRouter.getState().navigate({ name: "album", id: album.id }));
    expect(layers()).toHaveLength(2);
    expect(screen.getByTestId("page-stack")).toHaveAttribute("data-moving", "push");
    const slide = anims.made.find((m) => JSON.stringify(m.frames).includes("translateX(100%)"));
    expect(slide).toBeTruthy();
    expect(anims.made.some((m) => JSON.stringify(m.frames).includes("translateX(-30%)"))).toBe(true);
    // The page underneath is hidden from assistive tech while it sits under.
    expect(layers()[0]).toHaveAttribute("aria-hidden", "true");
    await anims.finishAll();
    expect(layers()).toHaveLength(1);
    expect(await screen.findByRole("heading", { name: album.title })).toBeInTheDocument();
  });

  it("Back slides the page off to the right, keeping both until it is gone", async () => {
    const anims = fakeAnimations();
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "album", id: mock.fx.albums[0].id }));
    await anims.finishAll();
    act(() => useRouter.getState().back());
    expect(layers()).toHaveLength(2);
    expect(screen.getByTestId("page-stack")).toHaveAttribute("data-moving", "back");
    expect(anims.made.some((m) => JSON.stringify(m.frames[m.frames.length - 1]).includes("translateX(100%)"))).toBe(true);
    await anims.finishAll();
    expect(layers()).toHaveLength(1);
  });

  it("a tab cross-fades, and each tab comes back where it was scrolled", async () => {
    const anims = fakeAnimations();
    // jsdom keeps no scroll positions; give it some.
    const tops = new WeakMap<Element, number>();
    const desc = Object.getOwnPropertyDescriptor(Element.prototype, "scrollTop");
    Object.defineProperty(Element.prototype, "scrollTop", {
      configurable: true,
      get(this: Element) {
        return tops.get(this) ?? 0;
      },
      set(this: Element, v: number) {
        tops.set(this, v);
      },
    });
    try {
      render(<PhoneShell />);
      const page = () => document.querySelector(".ph-layer:not([aria-hidden]) .ph-page")!;
      await waitFor(() => expect(page()).not.toBeNull());
      (page() as HTMLElement).scrollTop = 320;
      const tabs = screen.getByRole("navigation", { name: "Tabs" });
      fireEvent.click(within(tabs).getByRole("button", { name: "Library" }));
      expect(screen.getByTestId("page-stack")).toHaveAttribute("data-moving", "tab");
      const fade = anims.made[anims.made.length - 1];
      expect(fade.frames).toEqual([{ opacity: 0 }, { opacity: 1 }]);
      await anims.finishAll();
      fireEvent.click(within(tabs).getByRole("button", { name: "Home" }));
      await anims.finishAll();
      expect((page() as HTMLElement).scrollTop).toBe(320);
    } finally {
      if (desc) Object.defineProperty(Element.prototype, "scrollTop", desc);
    }
  });

  it("under reduced motion a push is a quick fade, with nothing sliding", async () => {
    setReducedMotion(true);
    const anims = fakeAnimations();
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "album", id: mock.fx.albums[0].id }));
    expect(anims.made.length).toBeGreaterThan(0);
    expect(anims.made.every((m) => !JSON.stringify(m.frames).includes("transform"))).toBe(true);
    await anims.finishAll();
    expect(layers()).toHaveLength(1);
  });

  it("without the animation API, pages change at once", () => {
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "album", id: mock.fx.albums[0].id }));
    return waitFor(() => expect(layers()).toHaveLength(1));
  });
});

describe("edge swipe back", () => {
  it("goes back past a third of the width or on a quick flick, and springs back short of that", () => {
    const width = 390;
    expect(edgeRelease({ dx: width * BACK_FRACTION, vx: 0, width })).toBe("back");
    expect(edgeRelease({ dx: width * BACK_FRACTION - 5, vx: 0, width })).toBe("stay");
    expect(edgeRelease({ dx: 40, vx: BACK_FLICK_SPEED, width })).toBe("back");
    expect(edgeRelease({ dx: 40, vx: BACK_FLICK_SPEED - 0.1, width })).toBe("stay");
    expect(edgeRelease({ dx: 0, vx: 2, width })).toBe("stay");
  });

  it("only claims a press heading right, leaving scrolls and leftward shelves alone", () => {
    expect(edgeClaims(3, 2)).toBeNull();
    expect(edgeClaims(20, 4)).toBe(true);
    expect(edgeClaims(4, 20)).toBe(false);
    expect(edgeClaims(-20, 2)).toBe(false);
  });

  it("a drag from the edge follows the finger and lets go of the page it springs back to", async () => {
    vi.useFakeTimers({ toFake: ["performance"] });
    const anims = fakeAnimations();
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "album", id: mock.fx.albums[0].id }));
    await anims.finishAll();
    const stack = screen.getByTestId("page-stack");
    Object.defineProperty(stack, "clientWidth", { configurable: true, value: 390 });
    fireEvent.pointerDown(stack, { clientX: 4, clientY: 300, pointerId: 7, pointerType: "touch" });
    fireEvent.pointerMove(stack, { clientX: 40, clientY: 302, pointerId: 7, pointerType: "touch" });
    expect(stack).toHaveAttribute("data-moving", "drag");
    expect(document.querySelectorAll(".ph-layer")).toHaveLength(2);
    vi.advanceTimersByTime(100);
    fireEvent.pointerMove(stack, { clientX: 60, clientY: 302, pointerId: 7, pointerType: "touch" });
    fireEvent.pointerUp(stack, { clientX: 60, clientY: 302, pointerId: 7, pointerType: "touch" });
    await anims.finishAll();
    await waitFor(() => expect(document.querySelectorAll(".ph-layer")).toHaveLength(1));
    expect(useRouter.getState().entries[useRouter.getState().index].route.name).toBe("album");
  });

  it("a drag past a third of the width goes back, with no second slide of its own", async () => {
    vi.useFakeTimers({ toFake: ["performance"] });
    const anims = fakeAnimations();
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "album", id: mock.fx.albums[0].id }));
    await anims.finishAll();
    const stack = screen.getByTestId("page-stack");
    Object.defineProperty(stack, "clientWidth", { configurable: true, value: 390 });
    fireEvent.pointerDown(stack, { clientX: 4, clientY: 300, pointerId: 9, pointerType: "touch" });
    fireEvent.pointerMove(stack, { clientX: 40, clientY: 300, pointerId: 9, pointerType: "touch" });
    vi.advanceTimersByTime(200);
    fireEvent.pointerMove(stack, { clientX: 200, clientY: 300, pointerId: 9, pointerType: "touch" });
    fireEvent.pointerUp(stack, { clientX: 200, clientY: 300, pointerId: 9, pointerType: "touch" });
    await anims.finishAll();
    await waitFor(() => expect(useRouter.getState().entries[useRouter.getState().index].route.name).toBe("home"));
    expect(stack).not.toHaveAttribute("data-moving");
    expect(document.querySelectorAll(".ph-layer")).toHaveLength(1);
  });

  it("a press that starts away from the edge never drags the page", async () => {
    const anims = fakeAnimations();
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "album", id: mock.fx.albums[0].id }));
    await anims.finishAll();
    const stack = screen.getByTestId("page-stack");
    fireEvent.pointerDown(stack, { clientX: 120, clientY: 300, pointerId: 8, pointerType: "touch" });
    fireEvent.pointerMove(stack, { clientX: 300, clientY: 300, pointerId: 8, pointerType: "touch" });
    expect(stack).not.toHaveAttribute("data-moving");
  });
});

describe("haptics", () => {
  it("buzz only with Haptics on controls on, once for a fast double tap", async () => {
    const spy = vi.spyOn(backend, "haptic");
    tap();
    tap();
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith("light");
    await backend.settingsSet({ ...DEFAULT_SETTINGS, phoneHaptics: false });
    await loadSettings();
    resetHaptics();
    tap("success");
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("taps further apart than the gap each buzz", () => {
    vi.useFakeTimers({ toFake: ["performance"] });
    const spy = vi.spyOn(backend, "haptic");
    tap("selection");
    vi.advanceTimersByTime(HAPTIC_GAP_MS + 1);
    tap("selection");
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it("a core without haptics fails quietly", async () => {
    const spy = vi.spyOn(backend, "haptic").mockRejectedValue(new Error("unknown command"));
    expect(() => tap("medium")).not.toThrow();
    await Promise.resolve();
    expect(spy).toHaveBeenCalled();
  });

  it("picking a tab is a selection tick", () => {
    const spy = vi.spyOn(backend, "haptic");
    render(<PhoneShell />);
    fireEvent.click(within(screen.getByRole("navigation", { name: "Tabs" })).getByRole("button", { name: "Search" }));
    expect(spy).toHaveBeenCalledWith("selection");
  });
});

describe("sheets", () => {
  it("slide away on close, however they close, before they leave the page", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    setSheetLeaveMs(260);
    render(<PhoneShell />);
    act(() => phone.show({ kind: "librarySort" }));
    expect(screen.getByRole("dialog", { name: "Sort by" })).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("sheet-dim"));
    const layer = document.querySelector(".ph-sheet-layer")!;
    expect(layer).toHaveClass("is-closing");
    expect(layer).toHaveAttribute("aria-hidden", "true");
    expect(screen.queryByRole("dialog", { name: "Sort by" })).not.toBeInTheDocument();
    act(() => void vi.advanceTimersByTime(300));
    expect(document.querySelector(".ph-sheet-layer")).toBeNull();
  });
});

describe("feedback without toasts", () => {
  it("liking from now playing fills the heart and says nothing else", async () => {
    const t: Track = mock.fx.tracks[3];
    const s = { status: "paused", track: t, positionMs: 0, durationMs: t.durationMs, volume: 1, muted: false, shuffle: false, shuffledContexts: [], repeat: "off", queue: { current: { itemId: "q0", track: t, source: "context" }, userQueue: [], nextUp: [], history: [], context: null } } as unknown as PlayerState;
    act(() => usePlayer.setState({ state: s, local: s, positionMs: 0, durationMs: t.durationMs, remote: null }));
    useToasts.setState({ toasts: [] });
    render(<PhoneShell />);
    act(() => phone.openPlayer());
    const np = await screen.findByRole("dialog", { name: "Now playing" });
    fireEvent.click(within(np).getByRole("button", { name: "Add to Liked Songs" }));
    expect(await within(np).findByRole("button", { name: "Liked. More for this song" })).toBeInTheDocument();
    expect(useToasts.getState().toasts).toHaveLength(0);
  });
});

describe("adding to a playlist", () => {
  it("gives a success tap and one short line with the playlist's name", async () => {
    useToasts.setState({ toasts: [] });
    const spy = vi.spyOn(backend, "haptic");
    const push = vi.spyOn(useToasts.getState(), "push");
    const list = (await backend.playlists()).find((p) => !p.smart && !/^(smart|blend|shared):/.test(p.id))!;
    expect(await addQuietly(list, mock.fx.tracks[0])).toBe(true);
    expect(spy).toHaveBeenCalledWith("success");
    expect(push).toHaveBeenCalledWith({ kind: "info", message: `Added to ${list.title}` }, ADDED_NOTE_MS);
  });
});

describe("the status bar", () => {
  it("is told once at the start and again when the look turns light or dark, and quietly fails off the phone", () => {
    const spy = vi.spyOn(backend, "setStatusBarStyle");
    const root = document.createElement("div");
    root.dataset.themeBase = "dark";
    const stop = followStatusBar(root);
    expect(spy).toHaveBeenLastCalledWith(false);
    expect(statusBarDark("light")).toBe(true);
    stop();
    spy.mockRejectedValue(new Error("no status bar"));
    root.dataset.themeBase = "light";
    expect(() => followStatusBar(root)()).not.toThrow();
    expect(spy).toHaveBeenLastCalledWith(true);
  });
});
