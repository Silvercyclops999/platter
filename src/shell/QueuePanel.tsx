import { X } from "lucide-react";
import { useState } from "react";
import { IconButton } from "../components/Button";
import { Tabs } from "../components/Tabs";
import { TrackList, TrackRow, trackMenuItems } from "../components/TrackRow";
import type { QueueItem } from "../lib/types";
import { player, usePlayer } from "../stores/player";
import { useUi } from "../stores/ui";

type Tab = "queue" | "history";

function QueueRow({ item, index, current, removable, draggable, onDropAt }: {
  item: QueueItem;
  index: number;
  current?: boolean;
  removable?: boolean;
  draggable?: boolean;
  onDropAt?: (itemId: string, toIndex: number) => void;
}) {
  const playing = usePlayer((s) => current && s.state?.status === "playing");
  return (
    <TrackRow
      compact
      track={item.track}
      index={index + 1}
      active={current}
      playing={!!playing}
      showAlbum={false}
      onPlay={() => (current ? player.toggle() : player.queueJump(item.itemId))}
      menuItems={trackMenuItems(item.track, {
        after: removable ? [{ label: "Remove from queue", separator: true, onSelect: () => player.queueRemove(item.itemId) }] : [],
      })}
      draggable={draggable}
      onDragStart={(e) => {
        e.dataTransfer.setData("text/x-platter-queue-item", item.itemId);
        e.dataTransfer.effectAllowed = "move";
      }}
      onDragOver={(e) => {
        if (onDropAt && e.dataTransfer.types.includes("text/x-platter-queue-item")) e.preventDefault();
      }}
      onDrop={(e) => {
        const id = e.dataTransfer.getData("text/x-platter-queue-item");
        if (id && onDropAt) {
          e.preventDefault();
          onDropAt(id, index);
        }
      }}
    />
  );
}

/** Right panel: Now playing, Next in queue (user), Next up (context), History. */
export function QueuePanel() {
  const queue = usePlayer((s) => s.state?.queue);
  const close = () => useUi.getState().setQueueOpen(false);
  const [tab, setTab] = useState<Tab>("queue");

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
                <TrackList label="Next in queue" header={false}>
                  {queue.userQueue.map((it, i) => (
                    <QueueRow key={it.itemId} item={it} index={i} removable draggable onDropAt={(id, to) => player.queueMove(id, to)} />
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
                <TrackList label="Next up" header={false}>
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
