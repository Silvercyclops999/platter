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
import { PhoneShell } from "../phone/PhoneShell";
import { phone, usePhoneUi } from "../phone/phoneUi";
import { QUEUE_SWIPE_PX, SWIPE_MAX_PX, swipeAdds, swipeFollow } from "../phone/rowSwipe";
import { player, usePlayer } from "../stores/player";
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
    // The playing song first, in the accent colour with its own pause.
    const now = queue.querySelector(".ph-queue__now")!;
    expect(within(now as HTMLElement).getByText(t.title)).toHaveClass("is-playing");
    expect(within(now as HTMLElement).getByRole("button", { name: "Pause" })).toBeInTheDocument();
    const added = within(queue).getByRole("list", { name: "Next in queue" });
    expect(within(added).getAllByRole("button", { name: /^Move / }).map((h) => h.getAttribute("aria-label"))).toEqual([`Move ${c.title}`, `Move ${d.title}`]);
    const next = within(queue).getByRole("list", { name: `Next from: ${t.albumTitle}` });
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
    expect(within(queue).queryByRole("list", { name: "Next in queue" })).toBeNull();
    expect(within(queue).getByRole("list", { name: "Next up" })).toBeInTheDocument();
  });
});
