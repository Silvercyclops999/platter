import { History } from "lucide-react";
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { AppError, ResumeOffer } from "../lib/types";
import { useCaps } from "../stores/caps";
import { useToasts } from "../stores/toasts";

/** "3 songs, saved by Phone app 5 minutes ago". */
export function offerLine(o: ResumeOffer, now = Date.now() / 1000): string {
  const songs = o.trackCount === 1 ? "1 song" : `${o.trackCount} songs`;
  const by = o.changedBy ? `, saved by ${o.changedBy}` : "";
  if (o.changedAt === null) return songs + by;
  const mins = Math.max(0, Math.round((now - o.changedAt) / 60));
  const when = mins < 1 ? "just now" : mins < 60 ? `${mins} min ago` : mins < 60 * 24 ? `${Math.round(mins / 60)} h ago` : `${Math.round(mins / 1440)} d ago`;
  return `${songs}${by} ${when}`;
}

/**
 * Pick up where you left off: the queue another app saved on a server that
 * keeps one, played here from the same song and place. Shows only when a
 * server in use keeps queues and one is saved.
 */
export function ResumeFrom() {
  const any = useCaps((s) => s.all.servers.some((x) => x.caps.resumeQueue));
  const [offers, setOffers] = useState<ResumeOffer[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    if (!any) return;
    let alive = true;
    backend.resumeQueues().then(
      (o) => alive && setOffers(o),
      () => alive && setOffers([]),
    );
    return () => {
      alive = false;
    };
  }, [any]);

  if (!any || !offers || offers.length === 0) return null;
  const pick = (o: ResumeOffer) => {
    setBusy(o.tag);
    backend
      .resumeQueuePlay(o.tag)
      .catch((e: AppError) => useToasts.getState().push({ kind: "error", message: e?.message || "Couldn't pick up that queue." }, 6000))
      .finally(() => setBusy(null));
  };
  return (
    <section className="output-section" aria-labelledby="output-resume">
      <h3 id="output-resume" className="output-section__title">
        Pick up where you left off
      </h3>
      <ul className="output-list">
        {offers.map((o) => (
          <li key={o.tag}>
            <button type="button" className="output-row" aria-busy={busy === o.tag || undefined} aria-label={`Play the queue saved on ${o.serverName}`} onClick={() => pick(o)}>
              <span className="output-row__icon" aria-hidden="true">
                <History size={20} />
              </span>
              <span className="output-row__text">
                <span className="output-row__name truncate">{o.serverName}</span>
                <span className="output-row__sub truncate">{busy === o.tag ? "Starting…" : offerLine(o)}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
