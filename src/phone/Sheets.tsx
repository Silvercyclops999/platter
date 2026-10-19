/**
 * The phone's bottom sheets: the queue, the sleep timer stacked on it, the
 * device sheet, one shared sheet for a song, adding to a playlist, the
 * library's sort and the create sheet. They stack; closing one shows the
 * one under it.
 */
import {
  Check,
  Disc3,
  GripVertical,
  Heart,
  HeartOff,
  ListEnd,
  ListMusic,
  ListPlus,
  ListStart,
  MicVocal,
  Moon,
  Pause,
  Play,
  Radio,
  Repeat,
  Repeat1,
  Share,
  Shuffle,
  Pencil,
  Sparkles,
  Trash2,
  UserRound,
  UsersRound,
  type LucideIcon,
} from "lucide-react";
import { Fragment, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { Art } from "../components/Art";
import { trackArtistLine } from "../lib/format";
import { backend } from "../lib/backend";
import type { Album, Playlist, QueueItem, Track } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { openShareCard } from "../stores/shareCard";
import { openSongCredits } from "../stores/songCredits";
import { countLine } from "./pages/common";
import { SORT_LABEL, useLibraryView, type LibSort } from "../stores/library";
import { isLiked, setLiked } from "../stores/likes";
import { player, usePlayer } from "../stores/player";
import { askDelete, createPlaylist, editDetails, removeRows } from "../stores/playlists";
import { useToasts } from "../stores/toasts";
import { newSmartPlaylist } from "../stores/curate";
import { SheetFrame, SheetLeaving, tap } from "./bits";
import { FADE_MS, reduced } from "./motion";
import { DeviceSheet } from "./DeviceSheet";
import { phone, usePhoneUi, type Sheet } from "./phoneUi";
import { CUSTOM_MAX, CUSTOM_MIN, SLEEP_CHOICES, cancelSleep, sleepLabel, startSleep, useSleep } from "./sleepTimer";
import { playingFrom } from "./NowPlaying";

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

function SheetFor({ sheet }: { sheet: Sheet }) {
  switch (sheet.kind) {
    case "queue":
      return <QueueSheet />;
    case "sleep":
      return <SleepSheet />;
    case "device":
      return <DeviceSheet />;
    case "track":
      return <TrackSheet track={sheet.track} inPlaylist={sheet.inPlaylist} />;
    case "album":
      return <AlbumSheet album={sheet.album} />;
    case "playlist":
      return <PlaylistSheet playlist={sheet.playlist} />;
    case "addTo":
      return <AddToSheet track={sheet.track} />;
    case "librarySort":
      return <SortSheet />;
    case "create":
      return <CreateSheet />;
  }
}

// ---------------------------------------------------------------- queue

/** Fixed row height in the queue (drag maths uses it). */
export const QUEUE_ROW_PX = 64;

/** Where a dragged row lands, from how far it moved. */
export function dropIndex(from: number, dy: number, count: number, rowPx = QUEUE_ROW_PX): number {
  return Math.max(0, Math.min(count - 1, from + Math.round(dy / rowPx)));
}

export function QueueSheet() {
  const q = usePlayer((s) => s.state?.queue ?? null);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const shuffle = usePlayer((s) => !!s.state?.shuffle);
  const repeat = usePlayer((s) => s.state?.repeat ?? "off");
  const timer = useSleep((s) => s.timer);
  const [editing, setEditing] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [drag, setDrag] = useState<{ id: string; from: number; dy: number } | null>(null);
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

  const onHandleDown = (e: ReactPointerEvent<HTMLElement>, item: QueueItem, from: number) => {
    e.preventDefault();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    start.current = { y: e.clientY, pointer: e.pointerId };
    setDrag({ id: item.itemId, from, dy: 0 });
  };
  const onHandleMove = (e: ReactPointerEvent<HTMLElement>) => {
    if (!drag || start.current?.pointer !== e.pointerId) return;
    setDrag({ ...drag, dy: e.clientY - start.current.y });
  };
  const onHandleUp = (e: ReactPointerEvent<HTMLElement>) => {
    if (!drag || start.current?.pointer !== e.pointerId) return;
    const to = dropIndex(drag.from, e.clientY - start.current.y, upcoming.length);
    start.current = null;
    setDrag(null);
    if (to !== drag.from) {
      tap("selection");
      void player.queueMove(drag.id, to);
    }
  };

  const RepeatIcon = repeat === "one" ? Repeat1 : Repeat;
  return (
    <SheetFrame
      title={editing ? "Edit queue" : "Queue"}
      subtitle={editing ? `${picked.size} selected` : `Playing from ${playingFrom(q?.context, current?.source)}`}
      onClose={phone.pop}
      tall
      className="ph-queue"
      action={
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
      {current && (
        <div className="ph-queue__now">
          <Art path={current.track.thumb} size={48} sleeve={{ title: current.track.albumTitle, artist: current.track.artistTitle }} />
          <span className="ph-row__text">
            <span className="ph-row__title is-playing">{current.track.title}</span>
            <span className="ph-row__sub">{trackArtistLine(current.track)}</span>
          </span>
          <button type="button" className="ph-play ph-play--row" aria-label={playing ? "Pause" : "Play"} onClick={() => void player.toggle()}>
            {playing ? <Pause size={20} fill="currentColor" aria-hidden="true" /> : <Play size={20} fill="currentColor" aria-hidden="true" />}
          </button>
        </div>
      )}
      {upcoming.length > 0 && <h3 className="ph-queue__next">Next up</h3>}
      <ul className="ph-queue__list" aria-label="Next up">
        {upcoming.map((item, i) => {
          const on = picked.has(item.itemId);
          const dragging = drag?.id === item.itemId;
          const autoplayStarts = item.source === "autoplay" && upcoming[i - 1]?.source !== "autoplay";
          return (
            <Fragment key={item.itemId}>
            {autoplayStarts && (
              <li className="ph-queue__next" role="presentation">
                Autoplay: chosen to follow
              </li>
            )}
            <li
              key={item.itemId}
              className={`ph-queue__row ${dragging ? "is-dragging" : ""}`}
              style={dragging ? { transform: `translateY(${drag!.dy}px)` } : undefined}
            >
              {editing && (
                <button
                  type="button"
                  className={`ph-select ${on ? "is-on" : ""}`}
                  role="checkbox"
                  aria-checked={on}
                  aria-label={`Select ${item.track.title}`}
                  onClick={() => toggle(item.itemId)}
                >
                  {on && <Check size={14} aria-hidden="true" />}
                </button>
              )}
              <button type="button" className="ph-queue__song" onClick={() => (editing ? toggle(item.itemId) : void player.queueJump(item.itemId))}>
                <Art path={item.track.thumb} size={44} sleeve={{ title: item.track.albumTitle, artist: item.track.artistTitle }} />
                <span className="ph-row__text">
                  <span className="ph-row__title">{item.track.title}</span>
                  <span className="ph-row__sub">{trackArtistLine(item.track)}</span>
                </span>
              </button>
              <span
                className="ph-handle"
                role="button"
                tabIndex={0}
                aria-label={`Move ${item.track.title}`}
                data-testid={`handle-${item.itemId}`}
                onPointerDown={(e) => onHandleDown(e, item, i)}
                onPointerMove={onHandleMove}
                onPointerUp={onHandleUp}
                onPointerCancel={() => {
                  start.current = null;
                  setDrag(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "ArrowUp" && i > 0) void player.queueMove(item.itemId, i - 1);
                  if (e.key === "ArrowDown" && i < upcoming.length - 1) void player.queueMove(item.itemId, i + 1);
                }}
              >
                <GripVertical size={20} aria-hidden="true" />
              </span>
            </li>
            </Fragment>
          );
        })}
      </ul>
      {!current && !upcoming.length && <p className="ph-rows__note">Nothing is queued. Play something and it shows here.</p>}
    </SheetFrame>
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

export function TrackSheet({ track, inPlaylist }: { track: Track; inPlaylist?: import("./phoneUi").InPlaylist }) {
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
    { label: "Play next", icon: ListStart, run: act(() => player.queueAdd([track.id], "next")) },
    { label: "Add to queue", icon: ListEnd, run: act(() => player.addToQueue([track])) },
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
  const items: Action[] = [
    { label: "Add to queue", icon: ListEnd, run: act(() => player.queueAlbum(album.id)) },
    { label: "Artist radio", icon: Radio, run: act(() => player.playRadio(`artist:${album.artistId}`)) },
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
  const items: Action[] = [
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

function AddToSheet({ track }: { track: Track }) {
  const { data, error, retry } = useLoad(() => backend.playlists(), []);
  // Only lists you can add to: not smart playlists, Blends or ones shared with you.
  const lists = (data ?? []).filter((p) => !p.smart && !/^(smart|blend|shared):/.test(p.id));
  return (
    <SheetFrame title="Add to playlist" subtitle={track.title} onClose={phone.pop} tall>
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
