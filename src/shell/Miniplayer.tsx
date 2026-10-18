/**
 * The miniplayer window's page: the art on a card coloured from the cover,
 * the title, artist and like control under it, and a top strip with
 * settings, a drag handle and close. Pointing at (or tabbing into) the art
 * brings up a dark scrim with one row of controls and the progress along
 * its bottom edge; at rest a thin line there shows how far through the song
 * it is. Pointing at mute pops up a vertical volume slider. The settings
 * icon swaps the content for a small settings panel.
 *
 * With the queue on, a tall enough window lists what's next under the art;
 * a wide one puts the title, progress and queue beside it.
 *
 * It runs in its own window with its own copy of the stores, fed by the
 * same core events as the main window.
 */
import {
  Disc3,
  GripHorizontal,
  Link,
  ListMinus,
  ListStart,
  Pause,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
  SlidersHorizontal,
  Sparkles,
  Volume1,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type DragEvent, type KeyboardEvent, type PointerEvent } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { ContextMenuHost, openContextMenu } from "../components/ContextMenu";
import { PlusGlyph, TickGlyph } from "../components/SaveButton";
import { Switch } from "../components/Switch";
import { Toaster } from "../components/Toast";
import { backend } from "../lib/backend";
import { NEUTRAL_HEADER } from "../lib/color";
import { formatDuration, trackArtistLine } from "../lib/format";
import type { QueueItem, Track } from "../lib/types";
import { setLiked, useIsLiked } from "../stores/likes";
import { player, usePlayer } from "../stores/player";
import { copyLinksAt, copyLink } from "../stores/share";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { Logo } from "./Logo";
import { useCoverColors } from "./nowPlaying";
import "./miniplayer.css";

/** Most up-next songs listed (the list scrolls; more would only cost memory). */
export const QUEUE_LIMIT = 100;

/** The next songs after the current one: the user's queue first, then the context. */
export function upNext(q: { userQueue: QueueItem[]; nextUp: QueueItem[] } | null | undefined, n = QUEUE_LIMIT): QueueItem[] {
  if (!q) return [];
  return [...q.userQueue, ...q.nextUp].slice(0, n);
}

/**
 * Track an element's size. Returns a callback ref, so an element that is
 * swapped out (the settings panel replaces the art and puts it back) is
 * measured again instead of the old, detached one reading 0 forever.
 */
function useSize<T extends HTMLElement>(): [(el: T | null) => void, { w: number; h: number }] {
  const [el, setEl] = useState<T | null>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    if (!el) return;
    const read = () => setSize((s) => (s.w === el.clientWidth && s.h === el.clientHeight ? s : { w: el.clientWidth, h: el.clientHeight }));
    read();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return [setEl, size];
}

/** Sizes the layout works from (they match miniplayer.css). */
export const STRIP_H = 28;
export const PAD = 8;
export const CARD_PAD = 12;
const META_H = 56;
const QUEUE_HEAD_H = 28;
/** Stacked, the queue shows from this window height up (the core grows the window to it). */
export const QUEUE_MIN_HEIGHT = 500;
/** Side by side once the window is this much wider than it is tall... */
export const SIDE_RATIO = 1.4;
/** ...and at least this wide, so the column beside the art has room. */
export const SIDE_MIN_WIDTH = 480;
/** Side by side, the queue shows from this height up. */
export const SIDE_QUEUE_MIN_HEIGHT = 300;
/** The narrowest the column beside the art gets. */
const SIDE_COLUMN_MIN = 200;
/** The art with the queue under it never gets smaller than this. */
const ART_WITH_QUEUE = 200;

export interface MiniLayout {
  /** Art on the left at full height, the title, progress and queue beside it. */
  side: boolean;
  /** The queue shows. */
  queue: boolean;
  /** The art card's fixed height (stacked with the queue) or width (side by side); null fills. */
  card: number | null;
}

/**
 * How the window lays out at a size. Stacked, the queue shows only from
 * QUEUE_MIN_HEIGHT up, and the art takes about half the height left so the
 * rest goes to queue rows (big windows get big art and more rows, never an
 * empty bottom). Wide windows go side by side when that setting is on.
 */
export function miniLayout(w: number, h: number, o: { queue: boolean; sideBySide: boolean }): MiniLayout {
  if (w <= 0 || h <= 0) return { side: false, queue: false, card: null };
  const side = o.sideBySide && w >= SIDE_MIN_WIDTH && w > h * SIDE_RATIO;
  if (side) {
    const art = Math.max(0, Math.min(h - STRIP_H - 2 * PAD - 2 * CARD_PAD, w - 3 * PAD - 2 * CARD_PAD - SIDE_COLUMN_MIN));
    return { side, queue: o.queue && h >= SIDE_QUEUE_MIN_HEIGHT, card: art + 2 * CARD_PAD };
  }
  if (!o.queue || h < QUEUE_MIN_HEIGHT) return { side, queue: false, card: null };
  const room = h - STRIP_H - 2 * PAD - META_H - QUEUE_HEAD_H - 2 * CARD_PAD;
  const art = Math.max(0, Math.min(w - 2 * PAD - 2 * CARD_PAD, Math.max(ART_WITH_QUEUE, Math.floor(room / 2))));
  return { side, queue: true, card: art + 2 * CARD_PAD };
}

/** A vertical volume slider in a small pill (opens over the mute button). */
function VolumePop({ onLeave }: { onLeave: () => void }) {
  const volume = usePlayer((s) => s.state?.volume ?? 0.7);
  const muted = usePlayer((s) => s.state?.muted ?? false);
  const [local, setLocal] = useState<number | null>(null);
  const v = local ?? (muted ? 0 : volume);
  const track = useRef<HTMLDivElement>(null);
  const at = (e: PointerEvent) => {
    const r = track.current!.getBoundingClientRect();
    return Math.min(1, Math.max(0, (r.bottom - e.clientY) / Math.max(1, r.height)));
  };
  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture?.(e.pointerId);
    const x = at(e);
    setLocal(x);
    void player.volumeInput(x);
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (local === null) return;
    const x = at(e);
    setLocal(x);
    void player.volumeInput(x);
  };
  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    if (local === null) return;
    const x = at(e);
    setLocal(null);
    void player.volumeCommit(x);
  };
  // A drag the system took over (a gesture, the window losing the pointer) keeps where it got to.
  const onCancel = () => {
    if (local === null) return;
    setLocal(null);
    void player.volumeCommit(local);
  };
  const onKey = (e: KeyboardEvent) => {
    const step = e.key === "ArrowUp" || e.key === "ArrowRight" ? 0.1 : e.key === "ArrowDown" || e.key === "ArrowLeft" ? -0.1 : 0;
    if (!step) return;
    e.preventDefault();
    void player.nudgeVolume(step);
  };
  return (
    <div className="mini-volume" onPointerLeave={onLeave}>
      <div
        ref={track}
        className="mini-volume__track"
        role="slider"
        tabIndex={0}
        aria-label="Volume"
        aria-orientation="vertical"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(v * 100)}
        aria-valuetext={`${Math.round(v * 100)}%`}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onCancel}
        onKeyDown={onKey}
      >
        <span className="mini-volume__fill" style={{ height: `${v * 100}%` }} />
        <span className="mini-volume__knob" style={{ bottom: `${v * 100}%` }} />
      </div>
    </div>
  );
}

/** How far through the song, 0 to 100. */
function usePercent(): number {
  const positionMs = usePlayer((s) => s.positionMs);
  const durationMs = usePlayer((s) => s.durationMs);
  return durationMs > 0 ? Math.min(100, Math.max(0, (positionMs / durationMs) * 100)) : 0;
}

/** Elapsed and total over a thin bar; pressing on the bar seeks. */
function SeekBar() {
  const positionMs = usePlayer((s) => s.positionMs);
  const durationMs = usePlayer((s) => s.durationMs);
  const pct = usePercent();
  return (
    <>
      <div className="mini__times tabular" aria-hidden="true">
        <span>{formatDuration(positionMs)}</span>
        <span>{formatDuration(durationMs)}</span>
      </div>
      <div
        className="mini__bar"
        role="progressbar"
        aria-label="Song progress"
        aria-valuemin={0}
        aria-valuemax={Math.round(durationMs / 1000)}
        aria-valuenow={Math.round(positionMs / 1000)}
        aria-valuetext={`${formatDuration(positionMs)} of ${formatDuration(durationMs)}`}
        onPointerDown={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          const f = Math.min(1, Math.max(0, (e.clientX - r.left) / Math.max(1, r.width)));
          if (durationMs > 0) void player.seek(Math.round(f * durationMs));
        }}
      >
        <span className="mini__bar-fill" style={{ width: `${pct}%` }} />
      </div>
    </>
  );
}

/** The thin line along the art's bottom edge, there at rest. */
function EdgeLine() {
  const pct = usePercent();
  return (
    <span className="mini__edge" aria-hidden="true">
      <span className="mini__edge-fill" data-testid="mini-edge-line" style={{ width: `${pct}%` }} />
    </span>
  );
}

/** The scrim's controls row and the progress along the art's bottom edge. */
function Controls({ track }: { track: Track }) {
  const state = usePlayer((s) => s.state);
  const positionMs = usePlayer((s) => s.positionMs);
  const [volumeOpen, setVolumeOpen] = useState(false);
  const smartAllowed = useSettings((s) => !!s.settings?.smartShuffle);
  const playing = state?.status === "playing" || state?.status === "buffering" || state?.status === "loading";
  const repeat = state?.repeat ?? "off";
  const shuffle = state?.shuffle ?? false;
  const smart = !!state?.smartShuffle;
  const silent = !!state?.muted || (state?.volume ?? 1) <= 0;
  const VolIcon = silent ? VolumeX : (state?.volume ?? 1) < 0.5 ? Volume1 : Volume2;
  const share = () => {
    const at = Math.floor(positionMs / 1000);
    const art = track.thumb ? { path: track.thumb } : undefined;
    return at > 0 ? copyLinksAt("track", track.id, at, art) : copyLink(backend.shareInfo("track", track.id).then((i) => i.appLink), undefined, art);
  };
  return (
    <>
      <div className="mini__row">
        <span
          className="mini__mute"
          onPointerEnter={() => setVolumeOpen(true)}
          onBlur={(e) => {
            // Focus moving on from the button and the slider closes it.
            if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setVolumeOpen(false);
          }}
        >
          {volumeOpen && <VolumePop onLeave={() => setVolumeOpen(false)} />}
          <IconButton
            label={silent ? "Unmute" : "Mute"}
            tooltip={false}
            icon={<VolIcon size={18} />}
            onClick={() => player.toggleMute()}
            onFocus={() => setVolumeOpen(true)}
          />
        </span>
        <IconButton
          label={smart ? "Disable smart shuffle" : shuffle ? "Disable shuffle" : "Enable shuffle"}
          tooltip={false}
          pressed={shuffle}
          icon={
            smart ? (
              <span className="smart-shuffle-icon">
                <Shuffle size={18} />
                <Sparkles size={10} className="smart-shuffle-icon__spark" />
              </span>
            ) : (
              <Shuffle size={18} />
            )
          }
          onClick={() => player.toggleShuffle(smartAllowed)}
        />
        <IconButton label="Previous" tooltip={false} icon={<SkipBack size={20} fill="currentColor" />} onClick={() => player.previous()} />
        <IconButton
          label={playing ? "Pause" : "Play"}
          tooltip={false}
          size="xl"
          variant="accent"
          className="mini__play"
          icon={playing ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
          onClick={() => player.toggle()}
        />
        <IconButton label="Next" tooltip={false} icon={<SkipForward size={20} fill="currentColor" />} onClick={() => player.next()} />
        <IconButton
          label={repeat === "off" ? "Enable repeat" : repeat === "all" ? "Enable repeat one" : "Disable repeat"}
          tooltip={false}
          pressed={repeat !== "off"}
          icon={repeat === "one" ? <Repeat1 size={18} /> : <Repeat size={18} />}
          onClick={() => player.cycleRepeat(repeat)}
        />
        <IconButton label="Copy link" tooltip={false} className="mini__share" icon={<Link size={18} />} onClick={() => void share()} />
      </div>
      <div className="mini__progress">
        <SeekBar />
      </div>
    </>
  );
}

function LikeControl({ track }: { track: Track }) {
  const liked = useIsLiked(track.id);
  return (
    <button
      type="button"
      className={`save-btn save-btn--bar mini__like ${liked ? "is-liked" : ""}`}
      aria-label={liked ? `Remove ${track.title} from Liked Songs` : `Add ${track.title} to Liked Songs`}
      aria-pressed={liked}
      onClick={() => void setLiked([track], !liked, { toast: false })}
    >
      {liked ? <TickGlyph /> : <PlusGlyph />}
    </button>
  );
}

/** Title and artist on one line each; a long one is cut off, the whole of it in a tooltip. */
function Meta({ track }: { track: Track }) {
  const artists = trackArtistLine(track);
  return (
    <div className="mini__meta">
      <div className="mini__text">
        <p className="mini__title truncate" title={track.title}>
          {track.title}
        </p>
        <p className="mini__artist truncate" title={artists}>
          {artists}
        </p>
      </div>
      <LikeControl track={track} />
    </div>
  );
}

function SettingsPanel({ onDone }: { onDone: () => void }) {
  const s = useSettings((x) => x.settings);
  const tint = s?.miniplayerTint ?? true;
  const queue = s?.miniplayerQueue ?? false;
  const sideBySide = s?.miniplayerSideBySide ?? true;
  return (
    <section className="mini-settings" aria-labelledby="mini-settings-title">
      <h1 id="mini-settings-title" className="mini-settings__title">
        Miniplayer settings
      </h1>
      <div className="mini-settings__row">
        <span id="mini-tint">Background colour</span>
        <Switch checked={tint} labelledBy="mini-tint" onChange={(v) => void updateSettings({ miniplayerTint: v })} />
      </div>
      <div className="mini-settings__row">
        <span id="mini-queue">Show the queue</span>
        <Switch checked={queue} labelledBy="mini-queue" onChange={(v) => void updateSettings({ miniplayerQueue: v })} />
      </div>
      <div className="mini-settings__row">
        <span id="mini-side">Side by side when wide</span>
        <Switch checked={sideBySide} labelledBy="mini-side" onChange={(v) => void updateSettings({ miniplayerSideBySide: v })} />
      </div>
      <p className="mini-settings__note">The queue shows when the window is at least 500 tall, or beside the art when it's wide.</p>
      <Button variant="primary" className="mini-settings__done" onClick={onDone}>
        Done
      </Button>
    </section>
  );
}

const QUEUE_DRAG = "text/x-platter-mini-queue-item";

/** Where a row dragged over row `index` would land: an insertion point 0..n. */
export function dropPoint(e: { clientY: number; currentTarget: Element }, index: number): number {
  const r = e.currentTarget.getBoundingClientRect();
  return e.clientY > r.top + r.height / 2 ? index + 1 : index;
}

/**
 * A move to insertion point `at` of the list, as `queue_move` takes it (an
 * index in the user queue, counted after the moved row is taken out). Songs
 * only move within, or into, the user queue at the top of the list, so a
 * point further down lands at its end. Null when nothing would change.
 */
export function queueMoveFor(count: number, userCount: number, from: number, at: number): number | null {
  if (from < 0 || from >= count) return null;
  const inUser = from < userCount;
  const point = Math.max(0, Math.min(at, userCount));
  const to = inUser && point > from ? point - 1 : point;
  if (inUser && to === from) return null;
  return to;
}

interface RowProps {
  item: QueueItem;
  index: number;
  focused: boolean;
  dropEdge: "before" | "after" | null;
  onFocusRow: (i: number) => void;
  onMenu: (i: number, at: { clientX: number; clientY: number } | HTMLElement) => void;
  onDragOverRow: (e: DragEvent<HTMLLIElement>, i: number) => void;
  onDropRow: (e: DragEvent<HTMLLIElement>, i: number) => void;
  onDragEnd: () => void;
}

function QueueRow({ item, index, focused, dropEdge, onFocusRow, onMenu, onDragOverRow, onDropRow, onDragEnd }: RowProps) {
  const t = item.track;
  const artists = trackArtistLine(t);
  return (
    <li
      role="option"
      aria-selected={focused}
      tabIndex={focused ? 0 : -1}
      data-index={index}
      className={`mini-q__row ${dropEdge ? `is-drop-${dropEdge}` : ""}`}
      title={`${t.title} · ${artists}`}
      draggable
      onClick={() => void player.queueJump(item.itemId)}
      onFocus={() => onFocusRow(index)}
      onContextMenu={(e) => {
        e.preventDefault();
        onFocusRow(index);
        onMenu(index, e);
      }}
      onDragStart={(e) => {
        e.dataTransfer.setData(QUEUE_DRAG, item.itemId);
        e.dataTransfer.effectAllowed = "move";
      }}
      onDragOver={(e) => onDragOverRow(e, index)}
      onDrop={(e) => onDropRow(e, index)}
      onDragEnd={onDragEnd}
    >
      <span className="mini-q__text truncate">
        <span className="mini-q__song">{t.title}</span>
        <span className="mini-q__artist">
          {" · "}
          {artists}
        </span>
      </span>
      <span className="mini-q__end">
        <span className="mini-q__dur tabular">{formatDuration(t.durationMs)}</span>
        <Play size={14} fill="currentColor" className="mini-q__play" aria-hidden="true" />
      </span>
    </li>
  );
}

/**
 * The queue: a heading on a thin rule, then one line per song (title,
 * artists, length). Click or Enter plays a song; right-click or the menu key
 * gives Play next, Remove from queue and Go to album; rows drag (or move with
 * Alt+Up and Alt+Down) to reorder; Delete removes.
 */
function Queue({ userCount, items }: { userCount: number; items: QueueItem[] }) {
  const [focus, setFocus] = useState(0);
  const [insertAt, setInsertAt] = useState<number | null>(null);
  const list = useRef<HTMLUListElement>(null);
  const wantFocus = useRef(false);
  const at = Math.min(focus, Math.max(0, items.length - 1));

  // After a remove or a move, keep the keyboard on the list.
  useEffect(() => {
    if (!wantFocus.current) return;
    wantFocus.current = false;
    list.current?.querySelector<HTMLElement>(`[data-index="${at}"]`)?.focus();
  });

  const move = (from: number, point: number): number | null => {
    const to = queueMoveFor(items.length, userCount, from, point);
    if (to !== null) void player.queueMove(items[from].itemId, to);
    return to;
  };
  const remove = (i: number, undoable = false) => {
    const it = items[i];
    if (!it) return;
    wantFocus.current = true;
    void player.queueRemove(it.itemId);
    // A key press can remove by accident: it can be put back.
    if (undoable) toast.undo(`Removed ${it.track.title} from the queue`, () => void player.queueAdd([it.track.id], { index: i }));
  };
  const menu = (i: number, where: { clientX: number; clientY: number } | HTMLElement) => {
    const item = items[i];
    openContextMenu(
      item.track.title,
      [
        { label: "Play next", icon: ListStart, onSelect: () => void move(i, 0) },
        { label: "Remove from queue", icon: ListMinus, onSelect: () => remove(i) },
        {
          label: "Go to album",
          icon: Disc3,
          separator: true,
          onSelect: () => void backend.miniplayerGo("album", item.track.albumId).catch(() => {}),
        },
      ],
      where,
    );
  };

  const onKeyDown = (e: KeyboardEvent<HTMLUListElement>) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-index]");
    if (!row) return;
    const i = Number(row.dataset.index);
    const go = (n: number) => {
      e.preventDefault();
      const to = Math.max(0, Math.min(items.length - 1, n));
      setFocus(to);
      list.current?.querySelector<HTMLElement>(`[data-index="${to}"]`)?.focus();
    };
    if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      const to = move(i, e.key === "ArrowUp" ? i - 1 : i + 2);
      if (to !== null) {
        setFocus(to);
        wantFocus.current = true;
      }
      return;
    }
    if (e.key === "ArrowDown") go(i + 1);
    else if (e.key === "ArrowUp") go(i - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(items.length - 1);
    else if (e.key === "Enter") {
      e.preventDefault();
      void player.queueJump(items[i].itemId);
    } else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      remove(i, true);
    } else if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
      e.preventDefault();
      menu(i, row);
    }
  };

  const fromDrag = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes(QUEUE_DRAG);
  const onDragOverRow = (e: DragEvent<HTMLLIElement>, i: number) => {
    if (!fromDrag(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    // The line shows where the song will really land.
    setInsertAt(Math.min(dropPoint(e, i), userCount));
  };
  const onDropRow = (e: DragEvent<HTMLLIElement>, i: number) => {
    if (!fromDrag(e)) return;
    e.preventDefault();
    setInsertAt(null);
    const from = items.findIndex((x) => x.itemId === e.dataTransfer.getData(QUEUE_DRAG));
    if (from >= 0) move(from, dropPoint(e, i));
  };

  return (
    <section className="mini-q" aria-labelledby="mini-q-title">
      <h2 id="mini-q-title" className="mini-q__heading">
        <span>Queue</span>
      </h2>
      {items.length ? (
        <ul
          ref={list}
          className="mini-q__list"
          role="listbox"
          aria-labelledby="mini-q-title"
          onKeyDown={onKeyDown}
          onDragLeave={(e) => {
            if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setInsertAt(null);
          }}
        >
          {items.map((item, i) => (
            <QueueRow
              key={item.itemId}
              item={item}
              index={i}
              focused={i === at}
              dropEdge={insertAt === i ? "before" : insertAt === i + 1 && i === items.length - 1 ? "after" : null}
              onFocusRow={setFocus}
              onMenu={menu}
              onDragOverRow={onDragOverRow}
              onDropRow={onDropRow}
              onDragEnd={() => setInsertAt(null)}
            />
          ))}
        </ul>
      ) : (
        <p className="mini-q__empty">Nothing else is queued.</p>
      )}
    </section>
  );
}

/** Nothing playing: the last song played, dimmed, with Play; or the mark. */
function Idle({ size }: { size: number }) {
  const [last, setLast] = useState<Track | null | undefined>(undefined);
  useEffect(() => {
    let alive = true;
    backend.recentTracks(1).then(
      (t) => alive && setLast(t[0] ?? null),
      () => alive && setLast(null),
    );
    return () => {
      alive = false;
    };
  }, []);
  if (last === undefined) return null;
  if (!last) {
    return (
      <div className="mini__idle">
        <Logo size={56} />
        <p className="mini__empty">Nothing playing</p>
      </div>
    );
  }
  return (
    <div className="mini__art is-idle" style={{ width: size, height: size }} data-testid="mini-idle">
      {size > 0 && <Art path={last.thumb} size={size} />}
      <div className="mini__idle-scrim">
        <IconButton
          label={`Play ${last.title}`}
          tooltip={false}
          size="xl"
          variant="accent"
          className="mini__play"
          icon={<Play size={24} fill="currentColor" className="nudge-right" />}
          onClick={() => void player.playAlbum(last.albumId, last.id)}
        />
      </div>
    </div>
  );
}

/** Keys a focused control or text field handles itself. */
function ownsSpace(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  if (t.isContentEditable) return true;
  return !!t.closest("button, input, textarea, select, a[href], [role=slider], [role=switch], [role^=menuitem]");
}

/** The miniplayer page (the whole window). */
export function Miniplayer() {
  const track = usePlayer((s) => s.state?.track ?? null);
  const queueView = usePlayer((s) => s.state?.queue);
  const settings = useSettings((s) => s.settings);
  const tintOn = settings?.miniplayerTint ?? true;
  const colorsOn = settings?.artworkColours !== false;
  const colors = useCoverColors(colorsOn ? track?.thumb : null);
  const tint = (colorsOn && colors?.vivid) || NEUTRAL_HEADER.vivid;
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [active, setActive] = useState(false);
  const [rootRef, root] = useSize<HTMLDivElement>();
  const [artBox, box] = useSize<HTMLDivElement>();
  const layout = miniLayout(root.w, root.h, {
    queue: settings?.miniplayerQueue ?? false,
    sideBySide: settings?.miniplayerSideBySide ?? true,
  });
  const items = layout.queue ? upNext(queueView) : [];
  const artSize = Math.max(0, Math.floor(Math.min(box.w, box.h)));

  useEffect(() => {
    document.title = track ? `${track.title} · ${trackArtistLine(track)}` : "Platter miniplayer";
  }, [track]);

  // Space plays and pauses, wherever focus is, unless a control takes it.
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== " " || e.ctrlKey || e.metaKey || e.altKey || e.repeat || ownsSpace(e.target)) return;
      e.preventDefault();
      void player.toggle();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  const close = () => void backend.miniplayerSet(false).catch(() => {});
  const cardStyle = layout.card === null ? undefined : layout.side ? { flexBasis: layout.card, width: layout.card } : { flexBasis: layout.card, height: layout.card };

  const art = track ? (
    <div
      className={`mini__art ${active ? "is-active" : ""}`}
      style={{ width: artSize, height: artSize }}
      data-testid="mini-art"
      onPointerEnter={() => setActive(true)}
      onPointerLeave={() => setActive(false)}
      onFocus={() => setActive(true)}
      onBlur={(e) => !e.currentTarget.contains(e.relatedTarget as Node | null) && setActive(false)}
    >
      {artSize > 0 && <Art path={track.thumb} size={artSize} />}
      {!layout.side && <EdgeLine />}
      <div className="mini__scrim">
        <Controls track={track} />
      </div>
    </div>
  ) : (
    <Idle size={artSize} />
  );

  const queue = layout.queue && <Queue userCount={queueView?.userQueue.length ?? 0} items={items} />;

  return (
    <div
      ref={rootRef}
      className={`mini ${tintOn ? "is-tinted" : ""} ${layout.side ? "mini--side" : "mini--stacked"}`}
      style={{ "--mini-tint": tint } as React.CSSProperties}
    >
      <header className="mini__strip" data-tauri-drag-region>
        <IconButton
          label={settingsOpen ? "Close miniplayer settings" : "Miniplayer settings"}
          tooltip={false}
          size="sm"
          pressed={settingsOpen}
          icon={<SlidersHorizontal size={14} />}
          onClick={() => setSettingsOpen((o) => !o)}
        />
        <span className="mini__handle" data-tauri-drag-region aria-hidden="true">
          <GripHorizontal size={16} data-tauri-drag-region />
        </span>
        <IconButton label="Close miniplayer" tooltip={false} size="sm" icon={<X size={14} />} onClick={close} />
      </header>
      {settingsOpen ? (
        <SettingsPanel onDone={() => setSettingsOpen(false)} />
      ) : (
        <main className="mini__body" data-layout={layout.side ? "side" : "stacked"} data-queue={layout.queue ? "on" : "off"}>
          <div className={`mini__card ${layout.card === null ? "is-fill" : ""}`} style={cardStyle}>
            <div ref={artBox} className="mini__art-area">
              {art}
            </div>
          </div>
          {layout.side ? (
            <div className="mini__side">
              {track && <Meta track={track} />}
              {track && (
                <div className="mini__side-progress">
                  <SeekBar />
                </div>
              )}
              {queue}
            </div>
          ) : (
            <>
              {track && <Meta track={track} />}
              {queue}
            </>
          )}
        </main>
      )}
      <button
        type="button"
        className="mini__grip"
        aria-label="Resize the miniplayer"
        tabIndex={-1}
        onPointerDown={(e) => {
          e.preventDefault();
          void backend.windowResizeFromCorner().catch(() => {});
        }}
      >
        <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
          <path d="M9 3L3 9M9 6.5L6.5 9" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
        </svg>
      </button>
      <ContextMenuHost />
      <Toaster />
    </div>
  );
}
