/**
 * The phone's bottom sheets: the queue, the sleep timer stacked on it, the
 * device sheet, one shared sheet for a song, adding to a playlist, the
 * library's sort and the create sheet. They stack; closing one shows the
 * one under it.
 */
import { Check, GripVertical, ListPlus, Moon, Pause, Play, Repeat, Repeat1, Shuffle, Sparkles } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { Art } from "../components/Art";
import { trackArtistLine } from "../lib/format";
import type { Album, QueueItem, Track } from "../lib/types";
import { SORT_LABEL, useLibraryView, type LibSort } from "../stores/library";
import { isLiked, setLiked } from "../stores/likes";
import { player, usePlayer } from "../stores/player";
import { addNow, createPlaylist, removeRows } from "../stores/playlists";
import { newSmartPlaylist } from "../stores/curate";
import { SheetFrame, tap } from "./bits";
import { DeviceSheet } from "./DeviceSheet";
import { phone, usePhoneUi, type Sheet } from "./phoneUi";
import { CUSTOM_MAX, CUSTOM_MIN, SLEEP_CHOICES, cancelSleep, sleepLabel, startSleep, useSleep } from "./sleepTimer";
import { playingFrom } from "./NowPlaying";

export function SheetHost() {
  const sheets = usePhoneUi((s) => s.sheets);
  return (
    <>
      {sheets.map((s, i) => (
        <SheetFor key={`${i}-${s.kind}`} sheet={s} />
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
      tap();
      void player.queueMove(drag.id, to);
    }
  };

  const RepeatIcon = repeat === "one" ? Repeat1 : Repeat;
  return (
    <SheetFrame
      title={editing ? "Edit queue" : "Queue"}
      subtitle={editing ? `${picked.size} selected` : `Playing from ${playingFrom(q?.context)}`}
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
            <button type="button" className={`ph-pill ph-pill--big ${shuffle ? "is-on" : ""}`} aria-pressed={shuffle} onClick={() => void player.toggleShuffle()}>
              <Shuffle size={20} aria-hidden="true" />
              Shuffle
            </button>
            <button type="button" className={`ph-pill ph-pill--big ${repeat !== "off" ? "is-on" : ""}`} aria-pressed={repeat !== "off"} onClick={() => void player.cycleRepeatNow()}>
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
          return (
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
    tap();
    f();
    phone.pop();
  };
  return (
    <SheetFrame title="Sleep timer" subtitle={timer ? `Stops: ${sleepLabel(timer)}` : "Music fades out and stops."} onClose={phone.pop}>
      <ul className="ph-rows" aria-label="Stop after">
        {SLEEP_CHOICES.map((c) => (
          <li key={c.label}>
            <button type="button" className="ph-row ph-row--text" onClick={() => pick(() => startSleep(c.choice))}>
              {c.label}
            </button>
          </li>
        ))}
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
        {timer && (
          <li>
            <button type="button" className="ph-row ph-row--text ph-row--quiet" onClick={() => pick(cancelSleep)}>
              Turn off the timer
            </button>
          </li>
        )}
      </ul>
    </SheetFrame>
  );
}

// ---------------------------------------------------------------- a song

export function TrackSheet({ track, inPlaylist }: { track: Track; inPlaylist?: import("./phoneUi").InPlaylist }) {
  const liked = isLiked(track.id);
  const isCurrent = usePlayer((s) => s.state?.track?.id === track.id);
  // "Remove from this playlist": only from that playlist's own page.
  const fromPlaylist = inPlaylist ?? null;
  const act = (f: () => unknown, keep = false) => () => {
    tap();
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
  const items: { label: string; run: () => void }[] = [
    { label: liked ? "Remove from Liked Songs" : "Add to Liked Songs", run: act(() => setLiked([track], !liked)) },
    { label: "Add to playlist", run: act(() => phone.push({ kind: "addTo", track }), true) },
    { label: "Play next", run: act(() => player.queueAdd([track.id], "next")) },
    { label: "Add to queue", run: act(() => player.addToQueue([track])) },
    ...(fromPlaylist
      ? [{ label: "Remove from this playlist", run: act(() => removeRows(fromPlaylist.playlist, [fromPlaylist.itemId], fromPlaylist.order)) }]
      : []),
    ...(isCurrent ? [{ label: "Go to queue", run: act(() => phone.show({ kind: "queue" }), true) }] : []),
    { label: "Go to album", run: act(() => phone.go({ name: "album", id: track.albumId }), true) },
    { label: "Go to artist", run: act(() => phone.go({ name: "artist", id: track.artistId }), true) },
    { label: "Song radio", run: act(() => player.playRadio(`track:${track.id}`)) },
  ];
  return (
    <SheetFrame title={track.title} onClose={phone.pop} head={head}>
      <ul className="ph-rows" aria-label="Actions">
        {items.map((i) => (
          <li key={i.label}>
            <button type="button" className="ph-row ph-row--text" onClick={i.run}>
              {i.label}
            </button>
          </li>
        ))}
      </ul>
    </SheetFrame>
  );
}

function AlbumSheet({ album }: { album: Album }) {
  const act = (f: () => unknown) => () => {
    tap();
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
  const items = [
    { label: "Add to queue", run: act(() => player.queueAlbum(album.id)) },
    { label: "Sleeve notes", run: act(() => phone.go({ name: "album", id: album.id, tab: "sleeve" })) },
    { label: "Go to artist", run: act(() => phone.go({ name: "artist", id: album.artistId })) },
  ];
  return (
    <SheetFrame title={album.title} onClose={phone.pop} head={head}>
      <ul className="ph-rows" aria-label="Actions">
        {items.map((i) => (
          <li key={i.label}>
            <button type="button" className="ph-row ph-row--text" onClick={i.run}>
              {i.label}
            </button>
          </li>
        ))}
      </ul>
    </SheetFrame>
  );
}

function AddToSheet({ track }: { track: Track }) {
  const items = useLibraryView((s) => s.items);
  useEffect(() => {
    if (items === null) void useLibraryView.getState().loadItems();
  }, [items]);
  const lists = (items ?? []).filter((i) => i.kind === "playlist" && !i.smart && !i.blend && !i.shared);
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
                void addNow({ id: p.id, title: p.title, thumb: p.thumb ?? null }, [track.id]);
              }}
            >
              <Art path={p.thumb} size={48} kind="playlist" />
              <span className="ph-row__text">
                <span className="ph-row__title">{p.title}</span>
                {p.trackCount != null && <span className="ph-row__sub tabular">{p.trackCount} songs</span>}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </SheetFrame>
  );
}

// ---------------------------------------------------------------- library

const PHONE_SORTS: LibSort[] = ["recents", "added", "alpha", "creator"];

function SortSheet() {
  const sort = useLibraryView((s) => s.sort);
  return (
    <SheetFrame title="Sort by" onClose={phone.pop}>
      <ul className="ph-rows" role="radiogroup" aria-label="Sort by">
        {PHONE_SORTS.map((s) => (
          <li key={s}>
            <button
              type="button"
              role="radio"
              aria-checked={sort === s}
              className={`ph-row ph-row--text ${sort === s ? "is-on" : ""}`}
              onClick={() => {
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
