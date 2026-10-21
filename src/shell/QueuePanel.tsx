import { Sparkles, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import { Button, IconButton } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Tabs } from "../components/Tabs";
import { TrackList, TrackRow, queueable, trackMenuItems } from "../components/TrackRow";
import { VIRTUAL_ROWS_FROM, VirtualRows, type VirtualRowsHandle } from "../components/VirtualRows";
import { backend } from "../lib/backend";
import type { QueueItem, Track } from "../lib/types";
import { dragLeft, dropOnQueue, isSongDrag, useDrag } from "../stores/drag";
import { player, usePlayer } from "../stores/player";
import { useUi } from "../stores/ui";
import { PersonFace, addedBy } from "./ListenStrip";
import { useSocial } from "../stores/social";

type Tab = "queue" | "history";

const DRAG_TYPE = "text/x-platter-queue-item";

/** Where a dragged queue row would land: an insertion point 0..n in the user queue. */
export function dropTarget(e: { clientY: number; currentTarget: Element }, index: number): number {
  const r = e.currentTarget.getBoundingClientRect();
  return e.clientY > r.top + r.height / 2 ? index + 1 : index;
}

/** Convert an insertion point into queue_move's index (position after removal). */
export function moveIndex(from: number, insertAt: number): number {
  return insertAt > from ? insertAt - 1 : insertAt;
}

/** A queue row or songs from elsewhere: either can land in "Next in queue". */
const queueDrag = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes(DRAG_TYPE) || isSongDrag(e);

interface Reorder {
  insertAt: number | null;
  setInsertAt: (n: number | null) => void;
  /** A queue row or songs dropped at insertion point `at` of the user queue. */
  drop: (e: DragEvent, at: number) => void;
}

function QueueRow({
  item,
  index,
  current,
  removable,
  reorder,
  rowIndex,
}: {
  item: QueueItem;
  index: number;
  current?: boolean;
  /** Removable rows (the user queue and "Next up") can be moved into the user queue. */
  removable?: boolean;
  /** Drop wiring (user queue rows only). */
  reorder?: Reorder;
  /** Position for screen readers when the list only has some rows on the page. */
  rowIndex?: number;
}) {
  const playing = usePlayer((s) => current && s.state?.status === "playing");
  // Listening along: whose song this is, when a guest added it.
  const by = useSocial((s) => addedBy(s.listen, item.track.id, current));
  const remove = removable ? () => player.queueRemove(item.itemId) : undefined;
  const edge = reorder?.insertAt === index ? "before" : reorder?.insertAt === index + 1 ? "after" : null;
  return (
    <TrackRow
      compact
      track={item.track}
      rowKey={item.itemId}
      index={index + 1}
      rowIndex={rowIndex}
      active={current}
      playing={!!playing}
      showAlbum={false}
      onPlay={() => (current ? player.toggle() : player.queueJump(item.itemId))}
      onRemove={remove}
      trailing={
        by ? (
          <span className="added-by" title={`Added by ${by.name}`} aria-label={`Added by ${by.name}`} role="img">
            <PersonFace person={by} size={20} />
          </span>
        ) : current ? (
          <span className="track-row__slot" aria-hidden="true" />
        ) : undefined
      }
      // Smart shuffle suggestions: a small sparkle before the artist.
      subBadge={
        item.source === "smart" ? (
          <span className="queue-smart" title="Smart shuffle suggestion" aria-hidden="true">
            <Sparkles size={12} />
          </span>
        ) : undefined
      }
      labelNote={item.source === "smart" ? "smart shuffle suggestion" : undefined}
      menuItems={trackMenuItems(item.track, {
        afterQueue: remove ? [{ label: "Remove from queue", onSelect: remove }] : [],
      })}
      draggable={removable}
      dropEdge={edge}
      onDragStart={(e) => {
        if (!removable) return;
        e.dataTransfer.setData(DRAG_TYPE, item.itemId);
        e.dataTransfer.effectAllowed = "copyMove";
      }}
      onDragOver={(e) => {
        if (!reorder || !queueDrag(e)) return;
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = e.dataTransfer.types.includes(DRAG_TYPE) ? "move" : "copy";
        const at = dropTarget(e, index);
        reorder.setInsertAt(at);
      }}
      onDragEnd={() => reorder?.setInsertAt(null)}
      onDrop={(e) => {
        if (!reorder || !queueDrag(e)) return;
        e.preventDefault();
        e.stopPropagation();
        reorder.drop(e, dropTarget(e, index));
      }}
    />
  );
}

/** Menu for several selected queue rows. */
const queueMultiMenu = (keys: string[], ids: string[], tracks: Track[]) => [
  { label: "Add to queue", onSelect: () => void player.addToQueue(queueable(ids, tracks)) },
  { label: "Remove from queue", separator: true, onSelect: () => keys.forEach((k) => player.queueRemove(k)) },
];

/** Songs the Recently played tab lists at most. */
export const RECENT_LIMIT = 50;

/**
 * Recently played: songs played on this device, newest first. A song
 * played twice shows twice. Loaded when the tab opens and again when the
 * song changes.
 */
function RecentlyPlayed() {
  const trackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
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
  if (!tracks.length) return <p className="queue-empty">Songs you play will show up here.</p>;
  const ids = tracks.map((t) => t.id);
  // Only the newest copy of the playing song is marked.
  const current = ids.indexOf(trackId ?? "");
  return (
    <TrackList label="Recently played" header={false}>
      {tracks.map((t, i) => (
        <TrackRow
          key={`${i}:${t.id}`}
          rowKey={`${i}:${t.id}`}
          compact
          track={t}
          index={i + 1}
          active={i === current}
          playing={i === current && status === "playing"}
          showAlbum={false}
          onPlay={() => (t.id === trackId ? player.toggle() : player.playTracks(ids, t.id))}
        />
      ))}
    </TrackList>
  );
}

/** Queue rows are this tall (compact rows). */
const QUEUE_ROW_PX = 64;

/**
 * One queue section's songs. A long queue only puts the rows near the
 * panel's view on the page; selection and keys still cover all of it.
 */
function QueueList({ label, items, reorder }: { label: string; items: QueueItem[]; reorder?: Reorder }) {
  const rows = useRef<VirtualRowsHandle>(null);
  const model = useMemo(() => ({ keys: items.map((i) => i.itemId), tracks: items.map((i) => i.track) }), [items]);
  const scrollToIndex = useCallback((i: number) => rows.current?.scrollToIndex(i), []);
  const windowed = items.length > VIRTUAL_ROWS_FROM;
  return (
    <TrackList
      label={label}
      header={false}
      multiMenu={queueMultiMenu}
      rowCount={windowed ? items.length : undefined}
      model={windowed ? model : undefined}
      scrollToIndex={windowed ? scrollToIndex : undefined}
    >
      <VirtualRows
        handle={rows}
        count={items.length}
        rowHeight={QUEUE_ROW_PX}
        spacer="div"
        scroller=".queue-panel__body"
        renderRow={(i) => <QueueRow key={items[i].itemId} item={items[i]} index={i} removable reorder={reorder} rowIndex={windowed ? i + 1 : undefined} />}
      />
    </TrackList>
  );
}

/** Right panel: Now playing, Next in queue (user), Next from (context); Recently played. */
export function QueuePanel() {
  const queue = usePlayer((s) => s.state?.queue);
  const close = () => useUi.getState().setQueueOpen(false);
  const [tab, setTab] = useState<Tab>("queue");
  const [insertAt, setInsertAt] = useState<number | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const userQueue = queue?.userQueue ?? [];
  // A drag from elsewhere that ends (dropped, Escape) takes its line with it.
  const dragging = useDrag((s) => !!s.songs);
  useEffect(() => {
    if (!dragging) setInsertAt(null);
  }, [dragging]);
  const reorder: Reorder = {
    insertAt,
    setInsertAt,
    drop: (e, at) => {
      setInsertAt(null);
      const itemId = e.dataTransfer.getData(DRAG_TYPE);
      if (itemId) {
        const from = userQueue.findIndex((i) => i.itemId === itemId);
        // A "Next up" row moves into the user queue at that spot.
        if (from < 0) player.queueMove(itemId, at);
        else if (moveIndex(from, at) !== from) player.queueMove(itemId, moveIndex(from, at));
        return;
      }
      const songs = useDrag.getState().songs;
      if (songs && isSongDrag(e)) void dropOnQueue(songs, at);
    },
  };
  // Over the panel but not a user queue row: above it (now playing) goes
  // first, anywhere else at its end.
  const panelAt = (e: DragEvent) => ((e.target as Element).closest?.("[data-queue-section]")?.getAttribute("data-queue-section") === "now" ? 0 : userQueue.length);
  const panelOver = (e: DragEvent) => {
    if (tab !== "queue" || !queueDrag(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = e.dataTransfer.types.includes(DRAG_TYPE) ? "move" : "copy";
    setInsertAt(panelAt(e));
  };
  const panelDrop = {
    onDragEnter: panelOver,
    onDragOver: panelOver,
    onDragLeave: (e: DragEvent) => {
      if (dragLeft(e)) setInsertAt(null);
    },
    onDrop: (e: DragEvent) => {
      if (tab !== "queue" || !queueDrag(e)) return;
      e.preventDefault();
      reorder.drop(e, panelAt(e));
    },
  };

  const ctxTitle = queue?.context?.title;

  return (
    <aside className="queue-panel" aria-label="Queue">
      <header className="queue-panel__head">
        <Tabs
          label="Queue views"
          idPrefix="queue"
          items={[
            { id: "queue", label: "Queue" },
            { id: "history", label: "Recently played" },
          ]}
          value={tab}
          onChange={setTab}
        />
        <IconButton label="Close queue" icon={<X size={18} />} onClick={close} />
      </header>
      <div className="queue-panel__body" id="queue-panel" role="tabpanel" aria-labelledby={`queue-tab-${tab}`} {...panelDrop}>
        {tab === "queue" ? (
          <>
            <section className="queue-section" aria-labelledby="q-now" data-queue-section="now">
              <h3 id="q-now" className="queue-section__title">
                Now playing
              </h3>
              {queue?.current ? (
                <TrackList label="Now playing" header={false}>
                  <QueueRow item={queue.current} index={0} current />
                </TrackList>
              ) : (
                <p className="queue-empty">Nothing playing</p>
              )}
            </section>

            {queue && queue.userQueue.length === 0 && insertAt !== null && <div className="queue-drop-line" aria-hidden="true" />}
            {queue && queue.userQueue.length > 0 && (
              <section className="queue-section" aria-labelledby="q-user">
                <div className="queue-section__row">
                  <h3 id="q-user" className="queue-section__title">
                    Next in queue
                  </h3>
                  <button type="button" className="queue-section__action" onClick={() => setConfirmClear(true)}>
                    Clear queue
                  </button>
                </div>
                <QueueList label="Next in queue" items={queue.userQueue} reorder={reorder} />
              </section>
            )}

            {/* No placeholder: with nothing left in the context the section just isn't there. */}
            {queue && queue.nextUp.length > 0 && (
              <section className="queue-section" aria-labelledby="q-next">
                <h3 id="q-next" className="queue-section__title">
                  {ctxTitle ? (
                    <>
                      Next from: <span className="queue-section__ctx">{ctxTitle}</span>
                    </>
                  ) : (
                    "Next up"
                  )}
                </h3>
                <QueueList label="Next up" items={queue.nextUp} />
              </section>
            )}
          </>
        ) : (
          <section className="queue-section" aria-label="Recently played">
            <RecentlyPlayed />
          </section>
        )}
      </div>
      <ClearQueueDialog open={confirmClear} onClose={() => setConfirmClear(false)} />
    </aside>
  );
}

/** Confirm before clearing the user queue (the context's songs stay). */
function ClearQueueDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const cancel = useRef<HTMLButtonElement>(null);
  return (
    <Dialog open={open} onClose={onClose} label="Clear your queue?" className="guide queue-confirm" initialFocus={cancel}>
      <h2 className="guide__title">Clear your queue?</h2>
      <p className="queue-confirm__text">This removes the songs you added to your queue. You can't undo it.</p>
      <div className="guide__actions">
        <Button ref={cancel} onClick={onClose}>
          Cancel
        </Button>
        <Button
          variant="primary"
          onClick={() => {
            onClose();
            player.queueClear();
          }}
        >
          Clear queue
        </Button>
      </div>
    </Dialog>
  );
}
