/**
 * The phone's queue flow: a song row swipes right into the queue (a buzz,
 * "Added to queue", the row springs back), up and down stay a scroll, the
 * song sheet offers Add to queue only, and the queue sheet shows what you
 * added apart from what comes next from the album or playlist.
 */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type PlayerState, type QueueItem, type Track } from "../lib/types";
import { setReducedMotion } from "../design/motion";
import { resetHaptics } from "../phone/bits";
import { usePhoneForm } from "../phone/form";
import { SongRow } from "../phone/pages/common";
import { PhonePage, PhoneShell } from "../phone/PhoneShell";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { QUEUE_SWIPE_PX, SWIPE_MARK_PX, SWIPE_MAX_PX, swipeAdds, swipeFollow, swipeSide } from "../phone/rowSwipe";
import { useLikes } from "../stores/likes";
import { EDGE_PX } from "../phone/motion";
import { player, queuedToast, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const tracks = (): Track[] => mock.fx.tracks;
const item = (t: Track, i: number, source: QueueItem["source"] = "context"): QueueItem => ({ itemId: `q${i}`, track: t, source });

function play(t: Track, extra: Partial<PlayerState> = {}) {
  const s: PlayerState = {
    status: "playing",
    track: t,
    positionMs: 30_000,
    durationMs: t.durationMs,
    volume: 0.5,
    muted: false,
    shuffle: false,
    shuffledContexts: [],
    repeat: "off",
    queue: { current: item(t, 0), userQueue: [], nextUp: tracks().slice(1, 4).map((x, i) => item(x, i + 1)), history: [], context: { kind: "album", id: t.albumId, title: t.albumTitle } },
    ...extra,
  };
  act(() => usePlayer.setState({ state: s, local: s, positionMs: s.positionMs, durationMs: s.durationMs, remote: null }));
}

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  usePhoneUi.setState({ tab: "home", nowPlaying: false, lyrics: false, sheets: [] });
  useRouter.getState().reset({ name: "home" });
  useToasts.setState({ toasts: [] });
  resetHaptics();
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

afterEach(() => {
  setReducedMotion(false);
  usePhoneForm.setState({ phone: false });
  vi.restoreAllMocks();
});

function row(t: Track) {
  const { container } = render(
    <div className="phone">
      <ul>
        <SongRow track={t} onPlay={() => undefined} />
      </ul>
    </div>,
  );
  return container.querySelector<HTMLLIElement>("li.ph-song")!;
}

/** A press at (x, y) that moves through `path` and lets go at its last point. */
function swipe(el: Element, from: [number, number], path: [number, number][], opts: { cancel?: boolean } = {}) {
  fireEvent.pointerDown(el, { pointerId: 7, clientX: from[0], clientY: from[1], button: 0, pointerType: "touch" });
  for (const [x, y] of path) fireEvent.pointerMove(el, { pointerId: 7, clientX: x, clientY: y, pointerType: "touch" });
  const [x, y] = path[path.length - 1];
  if (opts.cancel) fireEvent.pointerCancel(el, { pointerId: 7, clientX: x, clientY: y, pointerType: "touch" });
  else fireEvent.pointerUp(el, { pointerId: 7, clientX: x, clientY: y, pointerType: "touch" });
}

describe("swipe a song row right to queue it", () => {
  it("the row follows the finger to the mark, then gives, and never goes left", () => {
    expect(swipeFollow(-40)).toBe(0);
    expect(swipeFollow(30)).toBe(30);
    expect(swipeFollow(QUEUE_SWIPE_PX)).toBe(QUEUE_SWIPE_PX);
    expect(swipeFollow(QUEUE_SWIPE_PX + 30)).toBe(QUEUE_SWIPE_PX + 10);
    expect(swipeFollow(1000)).toBe(SWIPE_MAX_PX);
    expect(swipeAdds("x", QUEUE_SWIPE_PX)).toBe(true);
    expect(swipeAdds("x", QUEUE_SWIPE_PX - 1)).toBe(false);
    expect(swipeAdds("y", 200)).toBe(false);
  });

  it("past the mark: a buzz, the song goes to the end of the queue, Added to queue shows, the row slides back", async () => {
    const t = tracks()[2];
    const add = vi.spyOn(backend, "queueAdd");
    const buzz = vi.spyOn(backend, "haptic");
    const li = row(t);
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 100, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 140, clientY: 302, pointerType: "touch" });
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("40px");
    expect(li).toHaveClass("is-swiping");
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 200, clientY: 304, pointerType: "touch" });
    expect(li).toHaveClass("is-armed");
    fireEvent.pointerUp(li, { pointerId: 7, clientX: 200, clientY: 304, pointerType: "touch" });
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("0px");
    expect(li).not.toHaveClass("is-swiping");
    expect(add).toHaveBeenCalledWith([t.id], "end");
    expect(buzz).toHaveBeenCalledWith("success");
    await waitFor(() => expect(useToasts.getState().toasts.map((x) => x.message)).toContain("Added to queue"));
  });

  it("short of the mark, the row springs back and nothing is queued", () => {
    const add = vi.spyOn(player, "addToQueue");
    const li = row(tracks()[2]);
    swipe(li, [100, 300], [[130, 300], [100 + QUEUE_SWIPE_PX - 10, 300]]);
    expect(add).not.toHaveBeenCalled();
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("0px");
  });

  it("a mostly up or down press is a scroll: the row stays put and nothing is queued", () => {
    const add = vi.spyOn(player, "addToQueue");
    const li = row(tracks()[2]);
    // Within the 10 px tap give nothing is decided; then it heads down.
    swipe(li, [100, 300], [[106, 304], [130, 400], [200, 420]]);
    expect(add).not.toHaveBeenCalled();
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("0px");
    expect(li).not.toHaveClass("is-swiping");
  });

  it("a leftward swipe, a cancelled one and a press at the left edge queue nothing", () => {
    const add = vi.spyOn(player, "addToQueue");
    const li = row(tracks()[2]);
    swipe(li, [300, 300], [[200, 300], [100, 300]]);
    swipe(li, [100, 300], [[200, 300], [260, 300]], { cancel: true });
    swipe(li, [8, 300], [[100, 300], [200, 300]]);
    expect(add).not.toHaveBeenCalled();
  });

  it("a swipe doesn't also play the song it started on", () => {
    const onPlay = vi.fn();
    vi.spyOn(player, "addToQueue").mockResolvedValue(undefined);
    const { container } = render(
      <ul>
        <SongRow track={tracks()[1]} onPlay={onPlay} />
      </ul>,
    );
    const li = container.querySelector<HTMLLIElement>("li.ph-song")!;
    const main = li.querySelector<HTMLButtonElement>(".ph-song__main")!;
    swipe(li, [100, 300], [[200, 300]]);
    fireEvent.click(main);
    expect(onPlay).not.toHaveBeenCalled();
    fireEvent.pointerDown(li, { pointerId: 8, clientX: 100, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerUp(li, { pointerId: 8, clientX: 100, clientY: 300, pointerType: "touch" });
    fireEvent.click(main);
    expect(onPlay).toHaveBeenCalledTimes(1);
  });

  it("under reduced motion the row doesn't slide, and the song is still queued", () => {
    setReducedMotion(true);
    const add = vi.spyOn(player, "addToQueue").mockResolvedValue(undefined);
    const li = row(tracks()[2]);
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 100, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 220, clientY: 300, pointerType: "touch" });
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("0px");
    fireEvent.pointerUp(li, { pointerId: 7, clientX: 220, clientY: 300, pointerType: "touch" });
    expect(add).toHaveBeenCalledWith([tracks()[2]]);
  });
});

describe("the song sheet and the queue sheet", () => {
  it("a song's sheet offers Add to queue and no separate Play next", async () => {
    const t = tracks()[3];
    render(<PhoneShell />);
    act(() => phone.push({ kind: "track", track: t }));
    const sheet = await screen.findByRole("dialog", { name: t.title });
    expect(within(sheet).getByRole("button", { name: "Add to queue" })).toBeInTheDocument();
    expect(within(sheet).queryByRole("button", { name: "Play next" })).toBeNull();
  });

  it("what you added shows as Next in queue, apart from Next from the album, under the playing song", async () => {
    const [t, a, b, c, d] = tracks();
    play(t, {
      queue: {
        current: item(t, 0),
        userQueue: [item(c, 10, "user"), item(d, 11, "user")],
        nextUp: [item(a, 1), item(b, 2)],
        history: [],
        context: { kind: "album", id: t.albumId, title: t.albumTitle },
      },
    });
    const move = vi.spyOn(player, "queueMove").mockResolvedValue(undefined);
    render(<PhoneShell />);
    act(() => phone.push({ kind: "queue" }));
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    // Now playing first, the song in the accent colour; a tap on it pauses.
    const now = within(queue).getByRole("region", { name: "Now playing" });
    expect(within(now).getByText(t.title)).toHaveClass("is-playing");
    expect(within(now).getByRole("button", { name: `Pause ${t.title}` })).toBeInTheDocument();
    const added = within(queue).getByRole("region", { name: "Next in queue" });
    expect(within(added).getAllByRole("button", { name: /^Move / }).map((h) => h.getAttribute("aria-label"))).toEqual([`Move ${c.title}`, `Move ${d.title}`]);
    const next = within(queue).getByRole("region", { name: `Next from: ${t.albumTitle}` });
    expect(within(next).getAllByRole("button", { name: /^Move / }).map((h) => h.getAttribute("aria-label"))).toEqual([`Move ${a.title}`, `Move ${b.title}`]);
    expect(within(queue).getByRole("heading", { name: `Next from: ${t.albumTitle}` })).toBeInTheDocument();
    // The added songs reorder by their handles.
    const handle = screen.getByTestId("handle-q11");
    fireEvent.pointerDown(handle, { pointerId: 2, clientY: 200 });
    fireEvent.pointerMove(handle, { pointerId: 2, clientY: 136 });
    fireEvent.pointerUp(handle, { pointerId: 2, clientY: 136 });
    expect(move).toHaveBeenCalledWith("q11", 0);
  });

  it("with nothing added there is no Next in queue section, and an untitled context is Next up", async () => {
    const t = tracks()[0];
    play(t, { queue: { current: item(t, 0), userQueue: [], nextUp: [item(tracks()[1], 1)], history: [], context: { kind: "tracks", id: null } } });
    render(<PhoneShell />);
    act(() => phone.push({ kind: "queue" }));
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    expect(within(queue).queryByRole("region", { name: "Next in queue" })).toBeNull();
    expect(within(queue).getByRole("region", { name: "Next up" })).toBeInTheDocument();
  });
});

describe("toasts, fingers and taps after a swipe", () => {
  it("queue toasts don't stack: a live one is replaced and counts on (the computer shares this)", () => {
    queuedToast(1, "/a.jpg");
    queuedToast(1, "/b.jpg");
    queuedToast(2);
    const shown = useToasts.getState().toasts;
    expect(shown.map((t) => t.message)).toEqual(["Added 4 songs to queue"]);
    // Gone: the next add starts a fresh count.
    act(() => useToasts.getState().dismiss(shown[0].id));
    queuedToast(1, "/c.jpg");
    expect(useToasts.getState().toasts.map((t) => t.message)).toEqual(["Added to queue"]);
    expect(useToasts.getState().toasts[0].art?.path).toBe("/c.jpg");
  });

  it("a second finger while one is swiping is ignored, and the swipe still adds once", () => {
    const add = vi.spyOn(player, "addToQueue").mockResolvedValue(undefined);
    const li = row(tracks()[2]);
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 100, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 140, clientY: 300, pointerType: "touch" });
    fireEvent.pointerDown(li, { pointerId: 9, clientX: 300, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 9, clientX: 310, clientY: 300, pointerType: "touch" });
    fireEvent.pointerUp(li, { pointerId: 9, clientX: 310, clientY: 300, pointerType: "touch" });
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("40px");
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 200, clientY: 300, pointerType: "touch" });
    fireEvent.pointerUp(li, { pointerId: 7, clientX: 200, clientY: 300, pointerType: "touch" });
    expect(add).toHaveBeenCalledTimes(1);
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("0px");
  });

  it("losing the pointer capture puts the row back and adds nothing", () => {
    const add = vi.spyOn(player, "addToQueue").mockResolvedValue(undefined);
    const li = row(tracks()[2]);
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 100, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 200, clientY: 300, pointerType: "touch" });
    expect(li).toHaveClass("is-armed");
    fireEvent.lostPointerCapture(li, { pointerId: 7 });
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("0px");
    expect(li).not.toHaveClass("is-swiping");
    expect(li).not.toHaveClass("is-armed");
    expect(add).not.toHaveBeenCalled();
  });

  it("crossing the mark ticks lightly, and letting go still buzzes success", () => {
    vi.spyOn(player, "addToQueue").mockResolvedValue(undefined);
    const buzz = vi.spyOn(backend, "haptic");
    const li = row(tracks()[2]);
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 100, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 150, clientY: 300, pointerType: "touch" });
    expect(buzz).not.toHaveBeenCalled();
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 100 + QUEUE_SWIPE_PX + 4, clientY: 300, pointerType: "touch" });
    expect(buzz).toHaveBeenLastCalledWith("light");
    // Moving on past the mark doesn't tick again.
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 220, clientY: 300, pointerType: "touch" });
    expect(buzz).toHaveBeenCalledTimes(1);
    fireEvent.pointerUp(li, { pointerId: 7, clientX: 220, clientY: 300, pointerType: "touch" });
    expect(buzz).toHaveBeenLastCalledWith("success");
  });

  it("after a swipe, cancelled or let go, a later Enter or tap on the row still plays it", async () => {
    const onPlay = vi.fn();
    vi.spyOn(player, "addToQueue").mockResolvedValue(undefined);
    const { container } = render(
      <ul>
        <SongRow track={tracks()[1]} onPlay={onPlay} />
      </ul>,
    );
    const li = container.querySelector<HTMLLIElement>("li.ph-song")!;
    const main = li.querySelector<HTMLButtonElement>(".ph-song__main")!;
    swipe(li, [100, 300], [[200, 300]], { cancel: true });
    await act(() => new Promise((r) => setTimeout(r, 0)));
    // A keyboard Enter is a click with no press before it.
    fireEvent.click(main, { detail: 0 });
    expect(onPlay).toHaveBeenCalledTimes(1);
    swipe(li, [100, 300], [[130, 300]]);
    await act(() => new Promise((r) => setTimeout(r, 0)));
    fireEvent.click(main, { detail: 0 });
    expect(onPlay).toHaveBeenCalledTimes(2);
  });
});

describe("swipe a song row left to like it", () => {
  it("past the mark on the left: a buzz, the song goes into Liked Songs, and the toast says so", async () => {
    const t = tracks()[2];
    act(() => useLikes.getState().setIds([]));
    const set = vi.spyOn(backend, "likesSet");
    const add = vi.spyOn(player, "addToQueue");
    const buzz = vi.spyOn(backend, "haptic");
    const li = row(t);
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 300, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 260, clientY: 302, pointerType: "touch" });
    // The row follows left, over the heart.
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("-40px");
    expect(li).toHaveClass("is-left");
    expect(li.querySelector(".ph-swipe__under--left")).not.toBeNull();
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 300 - SWIPE_MARK_PX - 10, clientY: 302, pointerType: "touch" });
    expect(li).toHaveClass("is-armed-left");
    expect(buzz).toHaveBeenLastCalledWith("light");
    fireEvent.pointerUp(li, { pointerId: 7, clientX: 300 - SWIPE_MARK_PX - 10, clientY: 302, pointerType: "touch" });
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("0px");
    expect(buzz).toHaveBeenLastCalledWith("success");
    expect(set).toHaveBeenCalledWith([t.id], true);
    expect(add).not.toHaveBeenCalled();
    await waitFor(() => expect(useToasts.getState().toasts.map((x) => x.message)).toContain("Added to Liked Songs"));
    expect(useLikes.getState().ids.has(t.id)).toBe(true);
  });

  it("a liked song swipes left out of Liked Songs, with its own toast", async () => {
    const t = tracks()[3];
    act(() => useLikes.getState().setIds([t.id]));
    const set = vi.spyOn(backend, "likesSet");
    const li = row(t);
    swipe(li, [300, 300], [[260, 300], [180, 300]]);
    expect(set).toHaveBeenCalledWith([t.id], false);
    await waitFor(() => expect(useToasts.getState().toasts.map((x) => x.message)).toContain("Removed from Liked Songs"));
    expect(useLikes.getState().ids.has(t.id)).toBe(false);
  });

  it("short of the mark nothing changes, and under reduced motion the like still happens without the row moving", () => {
    const t = tracks()[4];
    act(() => useLikes.getState().setIds([]));
    const set = vi.spyOn(backend, "likesSet");
    const li = row(t);
    swipe(li, [300, 300], [[270, 300], [300 - SWIPE_MARK_PX + 10, 300]]);
    expect(set).not.toHaveBeenCalled();
    setReducedMotion(true);
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 300, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 180, clientY: 300, pointerType: "touch" });
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("0px");
    fireEvent.pointerUp(li, { pointerId: 7, clientX: 180, clientY: 300, pointerType: "touch" });
    expect(set).toHaveBeenCalledWith([t.id], true);
  });

  it("the row follows left as far as right, and a left swipe doesn't play the song", () => {
    expect(swipeFollow(-40, { right: true, left: true })).toBe(-40);
    expect(swipeFollow(-1000, { right: true, left: true })).toBe(-SWIPE_MAX_PX);
    expect(swipeFollow(40, { left: true })).toBe(0);
    expect(swipeSide("x", -SWIPE_MARK_PX)).toBe("left");
    expect(swipeSide("x", SWIPE_MARK_PX)).toBe("right");
    expect(swipeSide("y", -200)).toBeNull();
    const onPlay = vi.fn();
    const { container } = render(
      <ul>
        <SongRow track={tracks()[1]} onPlay={onPlay} />
      </ul>,
    );
    const li = container.querySelector<HTMLLIElement>("li.ph-song")!;
    swipe(li, [300, 300], [[200, 300]]);
    fireEvent.click(li.querySelector(".ph-song__main")!);
    expect(onPlay).not.toHaveBeenCalled();
  });

  it("once the row has a sideways swipe, the page doesn't scroll under the finger", () => {
    const li = row(tracks()[2]);
    const early = new Event("touchmove", { cancelable: true, bubbles: true });
    li.dispatchEvent(early);
    expect(early.defaultPrevented).toBe(false);
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 100, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 140, clientY: 304, pointerType: "touch" });
    const during = new Event("touchmove", { cancelable: true, bubbles: true });
    li.dispatchEvent(during);
    expect(during.defaultPrevented).toBe(true);
    fireEvent.pointerUp(li, { pointerId: 7, clientX: 140, clientY: 304, pointerType: "touch" });
  });
});

describe("swipes on the album page", () => {
  it("a song on an album swipes right into the queue without starting it, and left into Liked Songs", async () => {
    const t = tracks()[0];
    act(() => useLikes.getState().setIds([]));
    const add = vi.spyOn(player, "addToQueue").mockResolvedValue(undefined);
    const playAlbum = vi.spyOn(player, "playAlbum").mockResolvedValue(undefined);
    const set = vi.spyOn(backend, "likesSet");
    const { container } = render(
      <div className="phone">
        <PhonePage route={{ name: "album", id: t.albumId }} />
      </div>,
    );
    await screen.findAllByRole("list", { name: /songs$/i });
    const li = [...container.querySelectorAll<HTMLLIElement>("li.ph-song")].find((x) => x.textContent?.includes(t.title))!;
    expect(li).toHaveClass("ph-swipe");
    swipe(li, [100, 300], [[140, 300], [200, 300]]);
    fireEvent.click(li.querySelector(".ph-song__main")!);
    expect(add).toHaveBeenCalledWith([expect.objectContaining({ id: t.id })]);
    expect(playAlbum).not.toHaveBeenCalled();
    swipe(li, [300, 300], [[260, 300], [180, 300]]);
    expect(set).toHaveBeenCalledWith([t.id], true);
  });
});

describe("the queue sheet reads like the computer's queue panel", () => {
  function openQueue() {
    const [t, a, b, c, d] = tracks();
    play(t, {
      queue: {
        current: item(t, 0),
        userQueue: [item(c, 10, "user"), item(d, 11, "user")],
        nextUp: [item(a, 1), item(b, 2, "smart")],
        history: [],
        context: { kind: "album", id: t.albumId, title: t.albumTitle },
      },
    });
    render(<PhoneShell />);
    act(() => phone.push({ kind: "queue" }));
    return { t, a, b, c, d };
  }

  it("Queue and Recently played tabs; Now playing, Next in queue with Clear queue, then Next from, in that order", async () => {
    const { t } = openQueue();
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    expect(within(queue).getAllByRole("tab").map((x) => x.textContent)).toEqual(["Queue", "Recently played"]);
    expect(within(queue).getByRole("tab", { name: "Queue" })).toHaveAttribute("aria-selected", "true");
    const headings = within(queue)
      .getAllByRole("heading", { level: 3 })
      .map((h) => h.textContent);
    expect(headings).toEqual(["Now playing", "Next in queue", `Next from: ${t.albumTitle}`]);
    expect(within(queue).getByRole("button", { name: "Clear queue" })).toBeInTheDocument();
    // No phone-only extras: no "Playing from" line under the title.
    expect(within(queue).queryByText(/^Playing from/)).toBeNull();
    // A smart shuffle pick carries the sparkle, as on the computer.
    expect(within(queue).getAllByRole("img", { name: "Smart shuffle suggestion" })).toHaveLength(1);
  });

  it("Clear queue asks first, then clears only what you added", async () => {
    openQueue();
    const clear = vi.spyOn(player, "queueClear").mockResolvedValue(undefined);
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    fireEvent.click(within(queue).getByRole("button", { name: "Clear queue" }));
    const ask = await screen.findByRole("dialog", { name: "Clear your queue?" });
    fireEvent.click(within(ask).getByRole("button", { name: "Cancel" }));
    expect(clear).not.toHaveBeenCalled();
    fireEvent.click(within(queue).getByRole("button", { name: "Clear queue" }));
    fireEvent.click(within(await screen.findByRole("dialog", { name: "Clear your queue?" })).getByRole("button", { name: "Clear queue" }));
    expect(clear).toHaveBeenCalledTimes(1);
  });

  it("nothing playing says Nothing playing under Now playing", async () => {
    act(() => usePlayer.setState({ state: null, local: null }));
    render(<PhoneShell />);
    act(() => phone.push({ kind: "queue" }));
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    const now = within(queue).getByRole("region", { name: "Now playing" });
    expect(within(now).getByText("Nothing playing")).toBeInTheDocument();
    expect(within(queue).queryByRole("button", { name: "Edit" })).toBeNull();
  });

  it("Recently played lists the songs played here, newest first", async () => {
    const recent = tracks().slice(5, 8);
    vi.spyOn(backend, "recentTracks").mockResolvedValue(recent);
    openQueue();
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    fireEvent.click(within(queue).getByRole("tab", { name: "Recently played" }));
    const list = await within(queue).findByRole("list", { name: "Recently played" });
    expect(within(list).getAllByRole("listitem").map((li) => li.querySelector(".ph-row__title")?.textContent)).toEqual(recent.map((t) => t.title));
  });

  it("swiping a queued song left takes it out of the queue; the handle still drags and never swipes", async () => {
    const { c, a } = openQueue();
    const remove = vi.spyOn(player, "queueRemove").mockResolvedValue(undefined);
    const move = vi.spyOn(player, "queueMove").mockResolvedValue(undefined);
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    const rowOf = (title: string) => within(queue).getByRole("button", { name: `Move ${title}` }).closest("li")!;
    swipe(rowOf(c.title), [300, 300], [[260, 300], [180, 300]]);
    expect(remove).toHaveBeenCalledWith("q10");
    // A song from the album swipes out too.
    swipe(rowOf(a.title), [300, 300], [[260, 300], [180, 300]]);
    expect(remove).toHaveBeenLastCalledWith("q1");
    // Swiping right in the queue does nothing.
    remove.mockClear();
    swipe(rowOf(c.title), [100, 300], [[140, 300], [220, 300]]);
    expect(remove).not.toHaveBeenCalled();
    // The handle: a slanted drag on it moves the row and is no swipe.
    const handle = screen.getByTestId("handle-q11");
    fireEvent.pointerDown(handle, { pointerId: 3, clientX: 300, clientY: 200 });
    fireEvent.pointerMove(handle, { pointerId: 3, clientX: 180, clientY: 136 });
    fireEvent.pointerUp(handle, { pointerId: 3, clientX: 180, clientY: 136 });
    expect(remove).not.toHaveBeenCalled();
    expect(move).toHaveBeenCalledWith("q11", 0);
  });

  it("in Edit, rows don't swipe", async () => {
    const { c } = openQueue();
    const remove = vi.spyOn(player, "queueRemove").mockResolvedValue(undefined);
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    fireEvent.click(within(queue).getByRole("button", { name: "Edit" }));
    const li = within(queue).getByRole("button", { name: `Move ${c.title}` }).closest("li")!;
    swipe(li, [300, 300], [[260, 300], [180, 300]]);
    expect(remove).not.toHaveBeenCalled();
  });
});

describe("left swipes keep the right swipe's guards", () => {
  it("a cancelled left swipe or a lost capture likes nothing", () => {
    act(() => useLikes.getState().setIds([]));
    const set = vi.spyOn(backend, "likesSet");
    const li = row(tracks()[2]);
    swipe(li, [300, 300], [[260, 300], [180, 300]], { cancel: true });
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 300, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 180, clientY: 300, pointerType: "touch" });
    expect(li).toHaveClass("is-armed-left");
    fireEvent.lostPointerCapture(li, { pointerId: 7 });
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("0px");
    expect(set).not.toHaveBeenCalled();
  });

  it("a second finger during a left swipe is ignored, and the like happens once", () => {
    act(() => useLikes.getState().setIds([]));
    const set = vi.spyOn(backend, "likesSet");
    const li = row(tracks()[2]);
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 300, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 260, clientY: 300, pointerType: "touch" });
    fireEvent.pointerDown(li, { pointerId: 9, clientX: 200, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 9, clientX: 100, clientY: 300, pointerType: "touch" });
    fireEvent.pointerUp(li, { pointerId: 9, clientX: 100, clientY: 300, pointerType: "touch" });
    expect(li.style.getPropertyValue("--ph-swipe")).toBe("-40px");
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 180, clientY: 300, pointerType: "touch" });
    fireEvent.pointerUp(li, { pointerId: 7, clientX: 180, clientY: 300, pointerType: "touch" });
    expect(set).toHaveBeenCalledTimes(1);
  });

  it("a press inside the left edge is the back swipe's: no queue, no like", () => {
    const add = vi.spyOn(player, "addToQueue");
    const set = vi.spyOn(backend, "likesSet");
    const li = row(tracks()[2]);
    swipe(li, [EDGE_PX - 4, 300], [[60, 300], [200, 300]]);
    swipe(li, [EDGE_PX - 4, 300], [[EDGE_PX - 4, 300], [EDGE_PX - 4 - 100, 300]]);
    expect(add).not.toHaveBeenCalled();
    expect(set).not.toHaveBeenCalled();
    expect(li).not.toHaveClass("is-swiping");
  });

  it("an up or down drag leaves touchmove alone, so the list scrolls", () => {
    const li = row(tracks()[2]);
    fireEvent.pointerDown(li, { pointerId: 7, clientX: 100, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 104, clientY: 340, pointerType: "touch" });
    // Then sideways: still a scroll, the row never takes it.
    fireEvent.pointerMove(li, { pointerId: 7, clientX: 220, clientY: 350, pointerType: "touch" });
    const move = new Event("touchmove", { cancelable: true, bubbles: true });
    li.dispatchEvent(move);
    expect(move.defaultPrevented).toBe(false);
    fireEvent.pointerUp(li, { pointerId: 7, clientX: 220, clientY: 350, pointerType: "touch" });
    // After a sideways swipe ends, scrolling is free again.
    swipe(li, [100, 300], [[140, 300], [200, 300]]);
    const after = new Event("touchmove", { cancelable: true, bubbles: true });
    li.dispatchEvent(after);
    expect(after.defaultPrevented).toBe(false);
  });

  it("rows don't each carry a blocking listener: their list holds one for all of them", () => {
    const rowAdds = vi.spyOn(HTMLLIElement.prototype, "addEventListener");
    const listAdds = vi.spyOn(HTMLUListElement.prototype, "addEventListener");
    render(
      <div className="phone">
        <ul>
          {tracks()
            .slice(0, 5)
            .map((t) => (
              <SongRow key={t.id} track={t} onPlay={() => undefined} />
            ))}
        </ul>
      </div>,
    );
    expect(rowAdds.mock.calls.filter(([type]) => type === "touchmove")).toHaveLength(0);
    expect(listAdds.mock.calls.filter(([type]) => type === "touchmove")).toHaveLength(1);
  });

  it("a mouse press that leaves the row before picking a way is dropped", () => {
    const add = vi.spyOn(player, "addToQueue");
    const li = row(tracks()[2]);
    fireEvent.pointerDown(li, { pointerId: 1, clientX: 100, clientY: 300, button: 0, pointerType: "mouse" });
    fireEvent.pointerLeave(li, { pointerId: 1, clientX: 100, clientY: 260, pointerType: "mouse" });
    // Coming back in with the button still down doesn't pick the old press up.
    fireEvent.pointerMove(li, { pointerId: 1, clientX: 200, clientY: 300, pointerType: "mouse" });
    fireEvent.pointerUp(li, { pointerId: 1, clientX: 200, clientY: 300, pointerType: "mouse" });
    expect(li).not.toHaveClass("is-swiping");
    expect(add).not.toHaveBeenCalled();
  });

  it("a tap after a left swipe plays the song", async () => {
    const onPlay = vi.fn();
    act(() => useLikes.getState().setIds([]));
    const { container } = render(
      <ul>
        <SongRow track={tracks()[1]} onPlay={onPlay} />
      </ul>,
    );
    const li = container.querySelector<HTMLLIElement>("li.ph-song")!;
    const main = li.querySelector<HTMLButtonElement>(".ph-song__main")!;
    swipe(li, [300, 300], [[260, 300], [180, 300]]);
    fireEvent.click(main);
    expect(onPlay).not.toHaveBeenCalled();
    await act(() => new Promise((r) => setTimeout(r, 0)));
    fireEvent.pointerDown(li, { pointerId: 8, clientX: 150, clientY: 300, button: 0, pointerType: "touch" });
    fireEvent.pointerUp(li, { pointerId: 8, clientX: 150, clientY: 300, pointerType: "touch" });
    fireEvent.click(main);
    expect(onPlay).toHaveBeenCalledTimes(1);
  });
});

describe("removing one song from the queue without a swipe", () => {
  it("a queued row's More opens its sheet with Remove from queue", async () => {
    const [t, a, , c] = tracks();
    play(t, {
      queue: {
        current: item(t, 0),
        userQueue: [item(c, 10, "user")],
        nextUp: [item(a, 1)],
        history: [],
        context: { kind: "album", id: t.albumId, title: t.albumTitle },
      },
    });
    const remove = vi.spyOn(player, "queueRemove").mockResolvedValue(undefined);
    render(<PhoneShell />);
    act(() => phone.push({ kind: "queue" }));
    const queue = await screen.findByRole("dialog", { name: "Queue" });
    fireEvent.click(within(queue).getByRole("button", { name: `More for ${c.title}` }));
    const sheet = await screen.findByRole("dialog", { name: c.title });
    fireEvent.click(within(sheet).getByRole("button", { name: "Remove from queue" }));
    expect(remove).toHaveBeenCalledWith("q10");
    // A song's sheet opened anywhere else doesn't offer it.
    act(() => phone.push({ kind: "track", track: a }));
    const other = await screen.findByRole("dialog", { name: a.title });
    expect(within(other).queryByRole("button", { name: "Remove from queue" })).toBeNull();
  });
});
