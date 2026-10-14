import { X } from "lucide-react";
import { useState } from "react";
import { IconButton } from "../components/Button";
import { Tabs } from "../components/Tabs";
import { TrackList, TrackRow, trackMenuItems } from "../components/TrackRow";
import type { QueueItem } from "../lib/types";
import { player, usePlayer } from "../stores/player";
import { useUi } from "../stores/ui";

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

function QueueRow({
  item,
  index,
  current,
  removable,
  reorder,
}: {
  item: QueueItem;
  index: number;
  current?: boolean;
  removable?: boolean;
  /** Drag-to-reorder wiring (user queue only). */
  reorder?: { insertAt: number | null; setInsertAt: (n: number | null) => void; drop: (itemId: string, insertAt: number) => void };
}) {
  const playing = usePlayer((s) => current && s.state?.status === "playing");
  const remove = removable ? () => player.queueRemove(item.itemId) : undefined;
  const edge = reorder?.insertAt === index ? "before" : reorder?.insertAt === index + 1 ? "after" : null;
  return (
    <TrackRow
      compact
      track={item.track}
      rowKey={item.itemId}
      index={index + 1}
      active={current}
      playing={!!playing}
      showAlbum={false}
      onPlay={() => (current ? player.toggle() : player.queueJump(item.itemId))}
      onRemove={remove}
      menuItems={trackMenuItems(item.track, {
        after: remove ? [{ label: "Remove from queue", separator: true, onSelect: remove }] : [],
      })}
      draggable={!!reorder}
      dropEdge={edge}
      onDragStart={(e) => {
        e.dataTransfer.setData(DRAG_TYPE, item.itemId);
        e.dataTransfer.effectAllowed = "move";
      }}
      onDragOver={(e) => {
        if (!reorder || !e.dataTransfer.types.includes(DRAG_TYPE)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        const at = dropTarget(e, index);
        if (at !== reorder.insertAt) reorder.setInsertAt(at);
      }}
      onDragEnd={() => reorder?.setInsertAt(null)}
      onDrop={(e) => {
        const id = e.dataTransfer.getData(DRAG_TYPE);
        if (!reorder || !id) return;
        e.preventDefault();
        reorder.drop(id, dropTarget(e, index));
        reorder.setInsertAt(null);
      }}
    />
  );
}

/** Menu for several selected queue rows. */
const queueMultiMenu = (keys: string[], ids: string[]) => [
  { label: "Add to queue", onSelect: () => player.queueAdd(ids, "end") },
  { label: "Remove from queue", separator: true, onSelect: () => keys.forEach((k) => player.queueRemove(k)) },
];

/** Right panel: Now playing, Next in queue (user), Next up (context), History. */
export function QueuePanel() {
  const queue = usePlayer((s) => s.state?.queue);
  const close = () => useUi.getState().setQueueOpen(false);
  const [tab, setTab] = useState<Tab>("queue");
  const [insertAt, setInsertAt] = useState<number | null>(null);
  const userQueue = queue?.userQueue ?? [];
  const reorder = {
    insertAt,
    setInsertAt,
    drop: (itemId: string, at: number) => {
      const from = userQueue.findIndex((i) => i.itemId === itemId);
      if (from < 0) return;
      const to = moveIndex(from, at);
      if (to !== from) player.queueMove(itemId, to);
    },
  };

  const ctxTitle = queue?.context?.title;
  const history = queue ? [...queue.history].reverse() : [];

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
      <div className="queue-panel__body" id="queue-panel" role="tabpanel" aria-labelledby={`queue-tab-${tab}`}>
        {tab === "queue" ? (
          <>
            <section className="queue-section" aria-labelledby="q-now">
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

            {queue && queue.userQueue.length > 0 && (
              <section className="queue-section" aria-labelledby="q-user">
                <div className="queue-section__row">
                  <h3 id="q-user" className="queue-section__title">
                    Next in queue
                  </h3>
                  <button type="button" className="queue-section__action" onClick={() => player.queueClear()}>
                    Clear queue
                  </button>
                </div>
                <TrackList label="Next in queue" header={false} multiMenu={queueMultiMenu}>
                  {queue.userQueue.map((it, i) => (
                    <QueueRow key={it.itemId} item={it} index={i} removable reorder={reorder} />
                  ))}
                </TrackList>
              </section>
            )}

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
              {queue && queue.nextUp.length > 0 ? (
                <TrackList label="Next up" header={false} multiMenu={queueMultiMenu}>
                  {queue.nextUp.map((it, i) => (
                    <QueueRow key={it.itemId} item={it} index={i} removable />
                  ))}
                </TrackList>
              ) : (
                <p className="queue-empty">Nothing else queued.</p>
              )}
            </section>
          </>
        ) : (
          <section className="queue-section" aria-label="Recently played">
            {history.length ? (
              <TrackList label="Recently played" header={false}>
                {history.map((it, i) => (
                  <QueueRow key={`${it.itemId}-${i}`} item={it} index={i} />
                ))}
              </TrackList>
            ) : (
              <p className="queue-empty">Songs you play will show up here.</p>
            )}
          </section>
        )}
      </div>
    </aside>
  );
}
