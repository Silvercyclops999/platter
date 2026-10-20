/**
 * The phone's bottom sheets: the queue, the sleep timer stacked on it, the
 * device sheet, one shared sheet for a song, adding to a playlist, the
 * library's sort and the create sheet. They stack; closing one shows the
 * one under it.
 */
import {
  ArrowDownToLine,
  CircleSlash,
  Bookmark,
  Check,
  Disc3,
  GitBranch,
  GripVertical,
  Heart,
  HeartOff,
  ListEnd,
  ListMinus,
  MoreHorizontal,
  ListMusic,
  ListPlus,
  MicVocal,
  Moon,
  Radio,
  Repeat,
  Repeat1,
  Search,
  Share,
  Shuffle,
  Pencil,
  Sparkles,
  Trash2,
  UserRound,
  UsersRound,
  type LucideIcon,
} from "lucide-react";
import { memo, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { Art } from "../components/Art";
import { trackArtistLine } from "../lib/format";
import { backend } from "../lib/backend";
import type { Album, Playlist, QueueItem, Track } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { openShareCard } from "../stores/shareCard";
import { bookmarkPage, bookmarkTrack } from "../stores/curate";
import { useApp } from "../stores/app";
import { useCircle } from "../stores/circle";
import { collaboratorPlaylists, editablePlaylists } from "../components/PlaylistPicker";
import { versionLabel } from "../components/FileTagBadges";
import { formatArchiveDate, SONG_STATUS_LABEL } from "../pages/era";
import type { DownloadItem, VersionItem } from "../lib/types";
import { retryDownload, toggleDownload, useDownloadItem, type DownloadTarget } from "../stores/downloads";
import { countText, phoneDownloadState } from "./PhoneDownload";
import { useSettings } from "../stores/settings";
import { openSongCredits } from "../stores/songCredits";
import { countLine, PlayingBars, SongRow } from "./pages/common";
import { SORT_LABEL, useLibraryView, type LibSort } from "../stores/library";
import { isLiked, setLiked } from "../stores/likes";
import { player, usePlayer } from "../stores/player";
import { askDelete, createPlaylist, editDetails, removeRows } from "../stores/playlists";
import { useToasts } from "../stores/toasts";
import { newSmartPlaylist } from "../stores/curate";
import { SheetFrame, SheetLeaving, tap } from "./bits";
import { FADE_MS, reduced } from "./motion";
import { DeviceSheet } from "./DeviceSheet";
import { SignalSheet } from "./SignalSheet";
import { phone, usePhoneUi, type Sheet } from "./phoneUi";
import { CUSTOM_MAX, CUSTOM_MIN, SLEEP_CHOICES, cancelSleep, sleepLabel, startSleep, useSleep } from "./sleepTimer";
import { Dialog } from "../components/Dialog";
import { RECENT_LIMIT } from "../shell/QueuePanel";
import { PersonFace, addedBy } from "../shell/ListenStrip";
import { useSocial } from "../stores/social";
import { SwipeUnder, useRowSwipe } from "./rowSwipe";

/** How long a closed sheet takes to slide away (0 drops it at once). */
let sheetLeaveMs = import.meta.env?.MODE === "test" ? 0 : 260;
/** Tests: how long closed sheets stay to slide away. */
export function setSheetLeaveMs(ms: number) {
  sheetLeaveMs = ms;
}

interface Keyed {
  key: string;
  sheet: Sheet;
}

/**
 * The open sheets, bottom to top. A sheet that closes, however it closes
 * (the dim area, a drag, Escape, a choice, the player closing), stays a
 * moment to slide down while its dim fades.
 */
export function SheetHost() {
  const sheets = usePhoneUi((s) => s.sheets);
  const keyed: Keyed[] = sheets.map((s, i) => ({ key: `${i}-${s.kind}`, sheet: s }));
  const [leaving, setLeaving] = useState<Keyed[]>([]);
  const [prev, setPrev] = useState(sheets);
  const timers = useRef(new Set<ReturnType<typeof setTimeout>>());
  const scheduled = useRef(new WeakSet<Keyed>());
  // Worked out while rendering, so a closed sheet never leaves the page for a frame.
  if (prev !== sheets) {
    const now = new Set(keyed.map((k) => k.key));
    const gone = prev.map((s, i) => ({ key: `${i}-${s.kind}`, sheet: s })).filter((k) => !now.has(k.key));
    setPrev(sheets);
    setLeaving((l) => [...l.filter((x) => !now.has(x.key)), ...(sheetLeaveMs > 0 ? gone : [])]);
  }
  // Each one goes once it has slid away.
  useEffect(() => {
    const due = leaving.filter((k) => !scheduled.current.has(k));
    if (!due.length) return;
    due.forEach((k) => scheduled.current.add(k));
    const t = setTimeout(() => {
      timers.current.delete(t);
      setLeaving((l) => l.filter((x) => !due.includes(x)));
    }, reduced() ? FADE_MS : sheetLeaveMs);
    timers.current.add(t);
  }, [leaving]);
  useEffect(() => () => timers.current.forEach(clearTimeout), []);
  const open = new Set(keyed.map((k) => k.key));
  return (
    <>
      {[...keyed, ...leaving.filter((k) => !open.has(k.key))].map((k) => (
        <SheetLeaving.Provider key={k.key} value={!open.has(k.key)}>
          <SheetFor sheet={k.sheet} />
        </SheetLeaving.Provider>
      ))}
    </>
  );
}

/** One sheet. Memoised: a sheet that starts closing redraws only its frame, never its rows. */
const SheetFor = memo(function SheetFor({ sheet }: { sheet: Sheet }) {
  switch (sheet.kind) {
    case "queue":
      return <QueueSheet />;
    case "sleep":
      return <SleepSheet />;
    case "device":
      return <DeviceSheet />;
    case "track":
      return <TrackSheet track={sheet.track} inPlaylist={sheet.inPlaylist} queueItemId={sheet.queueItemId} />;
    case "album":
      return <AlbumSheet album={sheet.album} />;
    case "playlist":
      return <PlaylistSheet playlist={sheet.playlist} />;
    case "addTo":
      return <AddToSheet track={sheet.track} />;
    case "versions":
      return <VersionsSheet track={sheet.track} />;
    case "librarySort":
      return <SortSheet />;
    case "create":
      return <CreateSheet />;
    case "signal":
      return <SignalSheet />;
  }
});

// ---------------------------------------------------------------- queue

/** Fixed row height in the queue (drag maths uses it). */
export const QUEUE_ROW_PX = 64;

/** Where a dragged row lands, from how far it moved. */
export function dropIndex(from: number, dy: number, count: number, rowPx = QUEUE_ROW_PX): number {
  return Math.max(0, Math.min(count - 1, from + Math.round(dy / rowPx)));
}

/** The queue sheet's two views, as on the computer. */
type QueueTab = "queue" | "history";

interface HandleDrag {
  id: string;
  from: number;
  dy: number;
}

/**
 * One queue row: cover, title, artist line, and the handle that drags it.
 * A swipe left takes it out of the queue; in Edit a tap picks it. `i` is
 * its place in play order (what a drag counts in).
 */
function QueueRow({
  item,
  i,
  count,
  editing,
  picked,
  drag,
  onPick,
  onHandleDown,
  onHandleMove,
  onHandleUp,
  onHandleCancel,
}: {
  item: QueueItem;
  i: number;
  count: number;
  editing: boolean;
  picked: boolean;
  drag: HandleDrag | null;
  onPick: () => void;
  onHandleDown: (e: ReactPointerEvent<HTMLElement>) => void;
  onHandleMove: (e: ReactPointerEvent<HTMLElement>) => void;
  onHandleUp: (e: ReactPointerEvent<HTMLElement>) => void;
  onHandleCancel: () => void;
}) {
  const dragging = drag?.id === item.itemId;
  // Swipe left: out of the queue, at once (as Remove from queue on the computer).
  const swipe = useRowSwipe({ left: () => void player.queueRemove(item.itemId) }, !editing && !drag);
  const t = item.track;
  return (
    <li
      ref={swipe.ref}
      className={`ph-queue__row ${swipe.className} ${dragging ? "is-dragging" : ""}`}
      {...swipe.props}
      style={{ ...swipe.props.style, ...(dragging ? { transform: `translateY(${drag!.dy}px)` } : {}) }}
    >
      <SwipeUnder right={false} left={<ListMinus size={22} />} />
      <div className="ph-queue__line">
        {editing && (
          <button type="button" className={`ph-select ${picked ? "is-on" : ""}`} role="checkbox" aria-checked={picked} aria-label={`Select ${t.title}`} onClick={onPick}>
            {picked && <Check size={14} aria-hidden="true" />}
          </button>
        )}
        <button type="button" className="ph-queue__song" onClick={() => (editing ? onPick() : void player.queueJump(item.itemId))}>
          <QueueSongText item={item} />
        </button>
        <AddedBy trackId={t.id} />
        {!editing && (
          // The song's sheet, with Remove from queue: the way out that isn't a swipe.
          <button type="button" className="ph-icon-btn" aria-label={`More for ${t.title}`} onClick={() => phone.push({ kind: "track", track: t, queueItemId: item.itemId })}>
            <MoreHorizontal size={20} aria-hidden="true" />
          </button>
        )}
        <span
          className="ph-handle"
          role="button"
          tabIndex={0}
          aria-label={`Move ${t.title}`}
          data-testid={`handle-${item.itemId}`}
          onPointerDown={(e) => {
            // The handle drags up and down; it never starts the row's swipe.
            e.stopPropagation();
            onHandleDown(e);
          }}
          onPointerMove={onHandleMove}
          onPointerUp={onHandleUp}
          onPointerCancel={onHandleCancel}
          onKeyDown={(e) => {
            if (e.key === "ArrowUp" && i > 0) void player.queueMove(item.itemId, i - 1);
            if (e.key === "ArrowDown" && i < count - 1) void player.queueMove(item.itemId, i + 1);
          }}
        >
          <GripVertical size={20} aria-hidden="true" />
        </span>
      </div>
    </li>
  );
}

/** A queue row's cover, title and artist line, with the sparkle on a smart shuffle pick. */
function QueueSongText({ item, current = false }: { item: QueueItem; current?: boolean }) {
  const t = item.track;
  return (
    <>
      <Art path={t.thumb} size={48} sleeve={{ title: t.albumTitle, artist: t.artistTitle }} />
      <span className="ph-row__text">
        <span className={`ph-row__title ${current ? "is-playing" : ""}`}>
          {current && <PlayingBars />}
          {t.title}
        </span>
        <span className="ph-row__sub">
          {item.source === "smart" && (
            <span className="ph-queue__smart" title="Smart shuffle suggestion" aria-label="Smart shuffle suggestion" role="img">
              <Sparkles size={12} />
            </span>
          )}
          {trackArtistLine(t)}
        </span>
      </span>
    </>
  );
}

/** Listening along: the face of whoever added this song. */
function AddedBy({ trackId, current = false }: { trackId: string; current?: boolean }) {
  const by = useSocial((s) => addedBy(s.listen, trackId, current));
  if (!by) return null;
  return (
    <span className="ph-queue__by" title={`Added by ${by.name}`} aria-label={`Added by ${by.name}`} role="img">
      <PersonFace person={by} size={22} />
    </span>
  );
}

/**
 * The queue, laid out as the computer's queue panel: Queue and Recently
 * played; Now playing, then Next in queue (what you added, with Clear
 * queue), then Next from the album or playlist. Rows drag by their handle
 * and swipe left out of the queue; Edit picks several to move up or remove.
 * Shuffle, repeat and the sleep timer sit at the foot, in thumb reach.
 */
export function QueueSheet() {
  const q = usePlayer((s) => s.state?.queue ?? null);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const shuffle = usePlayer((s) => !!s.state?.shuffle);
  const repeat = usePlayer((s) => s.state?.repeat ?? "off");
  const timer = useSleep((s) => s.timer);
  const [tab, setTab] = useState<QueueTab>("queue");
  const [editing, setEditing] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [drag, setDrag] = useState<HandleDrag | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const start = useRef<{ y: number; pointer: number } | null>(null);
  const upcoming = useMemo(() => (q ? [...q.userQueue, ...q.nextUp] : []), [q]);
  const current = q?.current ?? null;
  const playing = status === "playing" || status === "loading" || status === "buffering";
  // A ticking label while the timer counts down.
  const [, tick] = useState(0);
  useEffect(() => {
    if (timer?.kind !== "minutes") return;
    const t = setInterval(() => tick((n) => n + 1), 15_000);
    return () => clearInterval(t);
  }, [timer]);
  // Nothing left to edit: Edit ends by itself.
  const canEdit = tab === "queue" && upcoming.length > 0;
  useEffect(() => {
    if (!canEdit && editing) {
      setEditing(false);
      setPicked(new Set());
    }
  }, [canEdit, editing]);

  const toggle = (id: string) =>
    setPicked((p) => {
      const n = new Set(p);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const moveUp = async () => {
    const ids = upcoming.filter((i) => picked.has(i.itemId)).map((i) => i.itemId);
    // Each to the top, in their order.
    for (let k = 0; k < ids.length; k++) await player.queueMove(ids[k], k);
    setPicked(new Set());
  };
  const remove = async () => {
    for (const id of picked) await player.queueRemove(id);
    setPicked(new Set());
  };

  const handle = (item: QueueItem, from: number) => ({
    onHandleDown: (e: ReactPointerEvent<HTMLElement>) => {
      e.preventDefault();
      e.currentTarget.setPointerCapture?.(e.pointerId);
      start.current = { y: e.clientY, pointer: e.pointerId };
      setDrag({ id: item.itemId, from, dy: 0 });
    },
    onHandleMove: (e: ReactPointerEvent<HTMLElement>) => {
      if (!drag || start.current?.pointer !== e.pointerId) return;
      setDrag({ ...drag, dy: e.clientY - start.current.y });
    },
    onHandleUp: (e: ReactPointerEvent<HTMLElement>) => {
      if (!drag || start.current?.pointer !== e.pointerId) return;
      const to = dropIndex(drag.from, e.clientY - start.current.y, upcoming.length);
      start.current = null;
      setDrag(null);
      if (to !== drag.from) {
        tap("selection");
        void player.queueMove(drag.id, to);
      }
    },
    onHandleCancel: () => {
      start.current = null;
      setDrag(null);
    },
  });
  const row = (item: QueueItem, i: number) => (
    <QueueRow
      key={item.itemId}
      item={item}
      i={i}
      count={upcoming.length}
      editing={editing}
      picked={picked.has(item.itemId)}
      drag={drag}
      onPick={() => toggle(item.itemId)}
      {...handle(item, i)}
    />
  );

  // What the context is called: "Next from: <album>" (the computer's words).
  const ctxTitle = q?.context?.title || (q?.context?.kind === "liked" ? "Liked Songs" : null);
  const RepeatIcon = repeat === "one" ? Repeat1 : Repeat;
  return (
    <SheetFrame
      title={editing ? "Edit queue" : "Queue"}
      subtitle={editing ? `${picked.size} selected` : undefined}
      onClose={phone.pop}
      tall
      className="ph-queue"
      action={
        canEdit ? (
          <button
            type="button"
            className="ph-text-btn"
            onClick={() => {
              setEditing(!editing);
              setPicked(new Set());
            }}
          >
            {editing ? "Done" : "Edit"}
          </button>
        ) : undefined
      }
      footer={
        editing ? (
          <div className="ph-queue__edit-actions">
            <button type="button" className="ph-pill" disabled={!picked.size} onClick={() => void moveUp()}>
              Move up
            </button>
            <button type="button" className="ph-pill" disabled={!picked.size} onClick={() => void remove()}>
              Remove
            </button>
          </div>
        ) : (
          <div className="ph-queue__pills">
            <button type="button" className={`ph-pill ph-pill--big ${shuffle ? "is-on" : ""}`} aria-pressed={shuffle} onClick={() => (tap("selection"), void player.toggleShuffle())}>
              <Shuffle size={20} aria-hidden="true" />
              Shuffle
            </button>
            <button type="button" className={`ph-pill ph-pill--big ${repeat !== "off" ? "is-on" : ""}`} aria-pressed={repeat !== "off"} onClick={() => cycleRepeat()}>
              <RepeatIcon size={20} aria-hidden="true" />
              {repeat === "one" ? "This song" : "Repeat"}
            </button>
            <button type="button" className={`ph-pill ph-pill--big ${timer ? "is-on" : ""}`} onClick={() => phone.push({ kind: "sleep" })}>
              <Moon size={20} aria-hidden="true" />
              <span className="tabular">{sleepLabel(timer)}</span>
            </button>
          </div>
        )
      }
    >
      {!editing && (
        <div className="ph-seg ph-queue__tabs" role="tablist" aria-label="Queue views">
          {(
            [
              ["queue", "Queue"],
              ["history", "Recently played"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="tab"
              id={`ph-queue-tab-${id}`}
              aria-selected={tab === id}
              aria-controls="ph-queue-panel"
              className={`ph-seg__tab ${tab === id ? "is-on" : ""}`}
              onClick={() => (tap("selection"), setTab(id))}
            >
              {label}
            </button>
          ))}
        </div>
      )}
      <div id="ph-queue-panel" role="tabpanel" aria-labelledby={`ph-queue-tab-${tab}`}>
        {tab === "queue" ? (
          <>
            <section aria-labelledby="ph-q-now">
              <h3 id="ph-q-now" className="ph-queue__title">
                Now playing
              </h3>
              {current ? (
                <ul className="ph-queue__list">
                  <li className="ph-queue__row is-current">
                    <div className="ph-queue__line">
                      <button
                        type="button"
                        className="ph-queue__song"
                        aria-label={`${playing ? "Pause" : "Play"} ${current.track.title}`}
                        onClick={() => (tap("selection"), void player.toggle())}
                      >
                        <QueueSongText item={current} current />
                      </button>
                      <AddedBy trackId={current.track.id} current />
                    </div>
                  </li>
                </ul>
              ) : (
                <p className="ph-queue__empty">Nothing playing</p>
              )}
            </section>
            {q && q.userQueue.length > 0 && (
              <section aria-labelledby="ph-q-user">
                <div className="ph-queue__head">
                  <h3 id="ph-q-user" className="ph-queue__title">
                    Next in queue
                  </h3>
                  {!editing && (
                    <button type="button" className="ph-queue__action" onClick={() => setConfirmClear(true)}>
                      Clear queue
                    </button>
                  )}
                </div>
                <ul className="ph-queue__list">{q.userQueue.map((item, k) => row(item, k))}</ul>
              </section>
            )}
            {/* No placeholder: with nothing left in the context the section just isn't there. */}
            {q && q.nextUp.length > 0 && (
              <section aria-labelledby="ph-q-next">
                <h3 id="ph-q-next" className="ph-queue__title">
                  {ctxTitle ? (
                    <>
                      Next from: <span className="ph-queue__ctx">{ctxTitle}</span>
                    </>
                  ) : (
                    "Next up"
                  )}
                </h3>
                <ul className="ph-queue__list">{q.nextUp.map((item, k) => row(item, q.userQueue.length + k))}</ul>
              </section>
            )}
          </>
        ) : (
          <RecentlyPlayed />
        )}
      </div>
      <Dialog open={confirmClear} onClose={() => setConfirmClear(false)} label="Clear your queue?" className="confirm">
        <h2 className="confirm__title">Clear your queue?</h2>
        <p className="confirm__body">This removes the songs you added to your queue. You can't undo it.</p>
        <div className="confirm__actions">
          <button type="button" className="ph-pill" onClick={() => setConfirmClear(false)}>
            Cancel
          </button>
          <button
            type="button"
            className="ph-pill is-primary"
            onClick={() => {
              setConfirmClear(false);
              tap("selection");
              void player.queueClear();
            }}
          >
            Clear queue
          </button>
        </div>
      </Dialog>
    </SheetFrame>
  );
}

/**
 * Recently played: songs played on this device, newest first (a song
 * played twice shows twice). Loaded when the tab opens and again when the
 * song changes. A tap plays the list from that song.
 */
function RecentlyPlayed() {
  const trackId = usePlayer((s) => s.state?.track?.id);
  const [tracks, setTracks] = useState<Track[] | null>(null);
  useEffect(() => {
    let alive = true;
    backend.recentTracks(RECENT_LIMIT).then(
      (t) => alive && setTracks(t),
      () => alive && setTracks((cur) => cur ?? []),
    );
    return () => {
      alive = false;
    };
  }, [trackId]);
  if (!tracks) return null;
  if (!tracks.length) return <p className="ph-queue__empty">Songs you play will show up here.</p>;
  const ids = tracks.map((t) => t.id);
  return (
    <ul className="ph-songs ph-queue__recent" aria-label="Recently played">
      {tracks.map((t, i) => (
        <SongRow key={`${i}:${t.id}`} track={t} onPlay={() => (t.id === trackId ? void player.toggle() : void player.playTracks(ids, t.id))} />
      ))}
    </ul>
  );
}


// ---------------------------------------------------------------- sleep timer

export function SleepSheet() {
  const timer = useSleep((s) => s.timer);
  const [custom, setCustom] = useState(false);
  const [minutes, setMinutes] = useState("20");
  const n = Number(minutes);
  const valid = Number.isInteger(n) && n >= CUSTOM_MIN && n <= CUSTOM_MAX;
  const pick = (f: () => void) => {
    tap("selection");
    f();
    phone.pop();
  };
  return (
    <SheetFrame title="Sleep timer" subtitle={<SleepLine />} onClose={phone.pop} dimBack>
      <ul className="ph-rows" aria-label="Stop after">
        {timer && (
          <li>
            <button type="button" className="ph-row ph-row--text ph-row--danger" onClick={() => pick(cancelSleep)}>
              Turn off the timer
            </button>
          </li>
        )}
        {SLEEP_CHOICES.map((c) => {
          const on = sameChoice(timer, c.choice);
          return (
            <li key={c.label}>
              <button type="button" className={`ph-row ph-row--text ${on ? "is-on" : ""}`} aria-current={on || undefined} onClick={() => pick(() => startSleep(c.choice))}>
                {c.label}
                {on && <Check size={20} aria-hidden="true" className="ph-row__tick" />}
              </button>
            </li>
          );
        })}
        <li>
          {custom ? (
            <form
              className="ph-custom"
              onSubmit={(e) => {
                e.preventDefault();
                if (valid) pick(() => startSleep({ kind: "minutes", minutes: n }));
              }}
            >
              <label className="ph-custom__label">
                Minutes
                <input className="ph-custom__input tabular" type="number" inputMode="numeric" min={CUSTOM_MIN} max={CUSTOM_MAX} value={minutes} onChange={(e) => setMinutes(e.target.value)} autoFocus />
              </label>
              <button type="submit" className="ph-pill is-primary" disabled={!valid}>
                Start
              </button>
            </form>
          ) : (
            <button type="button" className="ph-row ph-row--text" onClick={() => setCustom(true)}>
              Custom length
            </button>
          )}
        </li>
      </ul>
    </SheetFrame>
  );
}

/** Whether the running timer is this choice. */
function sameChoice(t: ReturnType<typeof useSleep.getState>["timer"], c: (typeof SLEEP_CHOICES)[number]["choice"]): boolean {
  if (!t || t.kind !== c.kind) return false;
  return t.kind !== "minutes" || (c.kind === "minutes" && t.minutes === c.minutes);
}

const clockFmt = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

/** "Stops at 9:02, in 14 min", kept up to date while the sheet is open. */
function SleepLine() {
  const timer = useSleep((s) => s.timer);
  const [, tick] = useState(0);
  useEffect(() => {
    if (timer?.kind !== "minutes") return;
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [timer]);
  if (!timer) return <>Music fades out and stops.</>;
  if (timer.kind === "track") return <>Stops at the end of this song.</>;
  if (timer.kind === "album") return <>Stops at the end of this album.</>;
  const left = Math.max(0, timer.endsAt - Date.now());
  const mins = Math.ceil(left / 60_000);
  return (
    <span className="tabular">
      Stops at {clockFmt.format(new Date(timer.endsAt))}, in {mins} min
    </span>
  );
}

/** Repeat steps once per press, however fast the presses come. */
let lastRepeat = 0;
function cycleRepeat() {
  const now = Date.now();
  if (now - lastRepeat < 350) return;
  lastRepeat = now;
  tap("selection");
  void player.cycleRepeatNow();
}
export { cycleRepeat };

// ---------------------------------------------------------------- a song

export function TrackSheet({ track, inPlaylist, queueItemId }: { track: Track; inPlaylist?: import("./phoneUi").InPlaylist; queueItemId?: string }) {
  const liked = isLiked(track.id);
  const isCurrent = usePlayer((s) => s.state?.track?.id === track.id);
  // "Remove from this playlist": only from that playlist's own page.
  const fromPlaylist = inPlaylist ?? null;
  const act = (f: () => unknown, keep = false) => () => {
    tap("selection");
    if (!keep) phone.pop();
    void f();
  };
  const head = (
    <div className="ph-track-head">
      <Art path={track.thumb} size={56} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} />
      <span className="ph-row__text">
        <span className="ph-row__title">{track.title}</span>
        <span className="ph-row__sub">
          {trackArtistLine(track)} · {track.albumTitle}
        </span>
      </span>
    </div>
  );
  const items: Action[] = [
    { label: liked ? "Remove from Liked Songs" : "Add to Liked Songs", icon: liked ? HeartOff : Heart, run: act(() => setLiked([track], !liked, { toast: !liked ? false : undefined })) },
    { label: "Add to playlist", icon: ListPlus, run: act(() => phone.push({ kind: "addTo", track }), true) },
    { label: "Add to queue", icon: ListEnd, run: act(() => player.addToQueue([track])) },
    // From a queue row: take this one out (the same words as the computer's menu).
    ...(queueItemId ? [{ label: "Remove from queue", icon: ListMinus, run: act(() => player.queueRemove(queueItemId)) }] : []),
    ...(fromPlaylist
      ? [{ label: "Remove from this playlist", icon: Trash2, danger: true, run: act(() => removeRows(fromPlaylist.playlist, [fromPlaylist.itemId], fromPlaylist.order)) }]
      : []),
    ...(isCurrent
      ? [
          { label: "Lyrics", icon: MicVocal, run: act(() => (phone.closeSheets(), phone.openPlayer(), phone.openLyrics()), true) },
          { label: "Go to queue", icon: ListMusic, run: act(() => phone.show({ kind: "queue" }), true) },
        ]
      : []),
    { label: "Credits", icon: UsersRound, run: act(() => openSongCredits(track)) },
    { label: "Share", icon: Share, run: act(() => openShareCard({ kind: "track", id: track.id, title: track.title, subtitle: trackArtistLine(track), thumb: track.thumb })) },
    { label: "Go to album", icon: Disc3, run: act(() => phone.go({ name: "album", id: track.albumId }), true) },
    { label: "Go to artist", icon: UserRound, run: act(() => phone.go({ name: "artist", id: track.artistId }), true) },
    { label: "Song radio", icon: Radio, run: act(() => player.playRadio(`track:${track.id}`)) },
    ...((track.otherVersions ?? 0) > 0 ? [{ label: `Versions (${(track.otherVersions ?? 0) + 1})`, icon: GitBranch, run: act(() => phone.push({ kind: "versions", track }), true) }] : []),
    { label: "Bookmark this moment", icon: Bookmark, run: act(() => bookmarkTrack(track)) },
  ];
  return (
    <SheetFrame title={track.title} onClose={phone.pop} head={head}>
      <ActionList items={items} />
    </SheetFrame>
  );
}

interface Action {
  label: string;
  icon: LucideIcon;
  run: () => void;
  danger?: boolean;
}

/** A sheet's actions: an icon and a word each. */
function ActionList({ items }: { items: Action[] }) {
  return (
    <ul className="ph-rows ph-actions-list" aria-label="Actions">
      {items.map((i) => (
        <li key={i.label}>
          <button type="button" className={`ph-row ph-row--action ${i.danger ? "ph-row--danger" : ""}`} onClick={i.run}>
            <i.icon size={22} aria-hidden="true" className="ph-row__icon" />
            {i.label}
          </button>
        </li>
      ))}
    </ul>
  );
}

/** Download, Stop downloading or Remove download, by where the item stands (from its songs). */
function downloadAction(item: DownloadItem | undefined, target: DownloadTarget, act: (f: () => unknown) => () => void): Action {
  const state = phoneDownloadState(item);
  const label =
    state === "none" ? "Download" : state === "done" ? "Remove download" : state === "error" ? "Download again" : `Stop downloading (${countText(item!)})`;
  return {
    label,
    icon: state === "none" || state === "error" ? ArrowDownToLine : CircleSlash,
    danger: state === "done",
    run: act(() => (state === "error" ? retryDownload(target.kind, target.id) : toggleDownload(target))),
  };
}

function AlbumSheet({ album }: { album: Album }) {
  const act = (f: () => unknown) => () => {
    tap("selection");
    phone.closeSheets();
    void f();
  };
  const head = (
    <div className="ph-track-head">
      <Art path={album.thumb} size={56} sleeve={{ title: album.title, artist: album.artistTitle }} />
      <span className="ph-row__text">
        <span className="ph-row__title">{album.title}</span>
        <span className="ph-row__sub">{album.artistTitle}</span>
      </span>
    </div>
  );
  const dl = useDownloadItem("album", album.id);
  const items: Action[] = [
    downloadAction(dl, { kind: "album", id: album.id, title: album.title }, act),
    { label: "Add to queue", icon: ListEnd, run: act(() => player.queueAlbum(album.id)) },
    { label: "Artist radio", icon: Radio, run: act(() => player.playRadio(`artist:${album.artistId}`)) },
    { label: "Bookmark", icon: Bookmark, run: act(() => bookmarkPage("album", album.id, album.title, { subtitle: album.artistTitle, thumb: album.thumb })) },
    { label: "Share", icon: Share, run: act(() => openShareCard({ kind: "album", id: album.id, title: album.title, subtitle: album.artistTitle, thumb: album.thumb })) },
    { label: "Go to artist", icon: UserRound, run: act(() => phone.go({ name: "artist", id: album.artistId })) },
  ];
  return (
    <SheetFrame title={album.title} onClose={phone.pop} head={head}>
      <ActionList items={items} />
    </SheetFrame>
  );
}

function PlaylistSheet({ playlist }: { playlist: Playlist }) {
  const act = (f: () => unknown) => () => {
    tap("selection");
    phone.closeSheets();
    void f();
  };
  const head = (
    <div className="ph-track-head">
      <Art path={playlist.thumb} size={56} kind="playlist" />
      <span className="ph-row__text">
        <span className="ph-row__title">{playlist.title}</span>
        <span className="ph-row__sub">{countLine(playlist.trackCount, "song")}</span>
      </span>
    </div>
  );
  const own = !/^(blend|shared):/.test(playlist.id);
  const dl = useDownloadItem("playlist", playlist.id);
  const items: Action[] = [
    ...(/^(blend|shared|smart):/.test(playlist.id) ? [] : [downloadAction(dl, { kind: "playlist", id: playlist.id, title: playlist.title }, act)]),
    ...(own && !playlist.smart ? [{ label: "Edit name and description", icon: Pencil, run: act(() => editDetails(playlist)) }] : []),
    { label: "Share", icon: Share, run: act(() => openShareCard({ kind: "playlist", id: playlist.id, title: playlist.title, thumb: playlist.thumb })) },
    ...(own ? [{ label: "Delete playlist", icon: Trash2, danger: true, run: act(() => askDelete(playlist)) }] : []),
  ];
  return (
    <SheetFrame title={playlist.title} onClose={phone.pop} head={head}>
      <ActionList items={items} />
    </SheetFrame>
  );
}

/** How long the one-line "Added to" note stays. */
export const ADDED_NOTE_MS = 1500;

/**
 * Adds a song to a playlist: a success tap and a short one-line note with
 * the playlist's name, no cover or icon. A failure says so plainly.
 */
export async function addQuietly(p: Pick<Playlist, "id" | "title">, track: Track): Promise<boolean> {
  try {
    const done = await backend.playlistAdd(p.id, [track.id]);
    useLibraryView.getState().touch("playlist", done.id);
    tap("success");
    useToasts.getState().push({ kind: "info", message: `Added to ${done.title || p.title}` }, ADDED_NOTE_MS);
    return true;
  } catch {
    useToasts.getState().push({ kind: "error", message: `Couldn't add to ${p.title}` }, 4000);
    return false;
  }
}

/**
 * Add a song to one of your playlists: the same list the computer's Add to
 * playlist offers (your own playlists that take songs, and ones shared with
 * you as a collaborator), most recently changed first, with a find field
 * once there are a few. The sheet is as tall as its rows.
 */
function AddToSheet({ track }: { track: Track }) {
  // The app's own playlist list (kept current as playlists change), read
  // fresh as well in case it hasn't arrived yet.
  const known = useApp((s) => s.playlists);
  const shared = useCircle((s) => s.view.playlists);
  const { data: loaded, error, retry } = useLoad(() => backend.playlists(), []);
  const [query, setQuery] = useState("");
  const all = known && known.length ? known : loaded;
  const data = all ?? null;
  const lists = useMemo(() => (data ? editablePlaylists([...data.filter((p) => !/^(smart|blend):/.test(p.id)), ...collaboratorPlaylists(shared)], query) : []), [data, shared, query]);
  return (
    <SheetFrame title="Add to playlist" subtitle={track.title} onClose={phone.pop}>
      {(data?.length ?? 0) > 6 && (
        <label className="ph-field ph-addto__find">
          <Search size={18} aria-hidden="true" />
          <input type="search" className="ph-field__input" placeholder="Find a playlist" aria-label="Find a playlist" value={query} onChange={(e) => setQuery(e.target.value)} />
        </label>
      )}
      <ul className="ph-rows">
        <li>
          <button
            type="button"
            className="ph-row"
            onClick={() => {
              phone.closeSheets();
              void createPlaylist({ trackIds: [track.id] });
            }}
          >
            <span className="ph-row__glyph">
              <ListPlus size={22} aria-hidden="true" />
            </span>
            <span className="ph-row__text">
              <span className="ph-row__title">New playlist</span>
            </span>
          </button>
        </li>
        {lists.map((p) => (
          <li key={p.id}>
            <button
              type="button"
              className="ph-row"
              onClick={() => {
                phone.closeSheets();
                void addQuietly(p, track);
              }}
            >
              <Art path={p.thumb} size={48} kind="playlist" />
              <span className="ph-row__text">
                <span className="ph-row__title">{p.title}</span>
                <span className="ph-row__sub tabular">{countLine(p.trackCount, "song")}</span>
              </span>
            </button>
          </li>
        ))}
        {data && lists.length === 0 && <li className="ph-rows__note">{query ? `No playlist called "${query}".` : data.length ? "Your playlists are smart playlists, which fill themselves from their rules. Make a new playlist to add songs by hand." : "No playlists yet. Make one to add this song."}</li>}
        {!data && !error && [0, 1, 2].map((i) => <li key={i} className="ph-lib-row--ghost" />)}
        {error && !data && (
          <li>
            <button type="button" className="ph-row ph-row--text" onClick={retry}>
              Couldn't load your playlists. Try again
            </button>
          </li>
        )}
      </ul>
    </SheetFrame>
  );
}

/** "Live at the Mill · 2019 · Deluxe" under a version. */
export function versionLine(v: VersionItem): string {
  const when = v.date ? formatArchiveDate(v.date) : v.year ? String(v.year) : null;
  const album = v.track.albumTitle;
  const said = (x: string | null | undefined) => !!x && !album.toLowerCase().includes(x.toLowerCase());
  const extra = [v.edition, versionLabel(v.version)].filter(said).filter((x, i, all) => all.indexOf(x) === i);
  return [album, when, ...extra].filter(Boolean).join(" · ");
}

/**
 * Every version of a song in the library (the same list as the computer's
 * Other versions): each with its record and year, the one you came from
 * marked, and each playable. The lineage page is a tap away.
 */
function VersionsSheet({ track }: { track: Track }) {
  const { data, error, retry } = useLoad(() => backend.trackVersions(track.id), [track.id]);
  const nowId = usePlayer((s) => s.state?.track?.id);
  const lineageOn = useSettings((s) => s.settings?.showLineage !== false);
  const play = (id: string) => {
    tap();
    if (id === nowId) return void player.toggle();
    void player.playTracks((data ?? []).map((v) => v.track.id), id);
  };
  return (
    <SheetFrame title="Versions" subtitle={track.title} onClose={phone.pop}>
      <ul className="ph-rows" aria-label={`Versions of ${track.title}`}>
        {data?.map((v) => (
          <li key={v.track.id} className="ph-version">
            <button type="button" className="ph-row" onClick={() => play(v.track.id)} aria-current={v.current || undefined}>
              <Art path={v.track.thumb} size={48} sleeve={{ title: v.track.albumTitle, artist: v.track.artistTitle }} />
              <span className="ph-row__text">
                <span className={`ph-row__title ${v.track.id === nowId ? "is-playing" : ""}`}>
                  {v.track.title}
                  {v.current && <span className="ph-version__here"> · This one</span>}
                </span>
                <span className="ph-row__sub">{versionLine(v)}</span>
                {v.status !== "released" && <span className="ph-row__sub ph-version__status">{SONG_STATUS_LABEL[v.status]}</span>}
              </span>
            </button>
            <button
              type="button"
              className="ph-icon-btn"
              aria-label={`Go to ${v.track.albumTitle}`}
              onClick={() => {
                tap();
                phone.go({ name: "album", id: v.track.albumId });
              }}
            >
              <Disc3 size={20} aria-hidden="true" />
            </button>
          </li>
        ))}
        {!data && !error && [0, 1, 2].map((i) => <li key={i} className="ph-lib-row--ghost" />)}
        {error && !data && (
          <li>
            <button type="button" className="ph-row ph-row--text" onClick={retry}>
              Couldn't load the versions. Try again
            </button>
          </li>
        )}
      </ul>
      {lineageOn && data && data.length > 1 && (
        <button type="button" className="ph-pill ph-pill--wide" onClick={() => phone.go({ name: "lineage", id: track.id })}>
          <GitBranch size={16} aria-hidden="true" /> Every version on one timeline
        </button>
      )}
    </SheetFrame>
  );
}

// ---------------------------------------------------------------- library

const PHONE_SORTS: LibSort[] = ["recents", "added", "alpha", "creator"];

function SortSheet() {
  const sort = useLibraryView((s) => s.sort);
  const filter = useLibraryView((s) => s.filter);
  // Artists have no creator to sort by.
  const sorts = PHONE_SORTS.filter((x) => !(filter === "artist" && x === "creator"));
  return (
    <SheetFrame title="Sort by" onClose={phone.pop}>
      <ul className="ph-rows" role="radiogroup" aria-label="Sort by">
        {sorts.map((s) => (
          <li key={s}>
            <button
              type="button"
              role="radio"
              aria-checked={sort === s}
              className={`ph-row ph-row--text ${sort === s ? "is-on" : ""}`}
              onClick={() => {
                tap("selection");
                useLibraryView.getState().setSort(s);
                phone.pop();
              }}
            >
              {SORT_LABEL[s]}
              {sort === s && <Check size={20} aria-hidden="true" className="ph-row__tick" />}
            </button>
          </li>
        ))}
      </ul>
    </SheetFrame>
  );
}

function CreateSheet() {
  return (
    <SheetFrame title="Create" onClose={phone.pop}>
      <ul className="ph-rows">
        <li>
          <button
            type="button"
            className="ph-row"
            onClick={() => {
              phone.closeSheets();
              void createPlaylist();
            }}
          >
            <span className="ph-row__glyph">
              <ListPlus size={22} aria-hidden="true" />
            </span>
            <span className="ph-row__text">
              <span className="ph-row__title">Playlist</span>
              <span className="ph-row__sub">Songs you pick, in your order</span>
            </span>
          </button>
        </li>
        <li>
          <button
            type="button"
            className="ph-row"
            onClick={() => {
              phone.closeSheets();
              newSmartPlaylist();
            }}
          >
            <span className="ph-row__glyph">
              <Sparkles size={22} aria-hidden="true" />
            </span>
            <span className="ph-row__text">
              <span className="ph-row__title">Smart playlist</span>
              <span className="ph-row__sub">Fills itself from rules you set</span>
            </span>
          </button>
        </li>
      </ul>
    </SheetFrame>
  );
}
