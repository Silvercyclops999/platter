/**
 * Monthly recaps: a ruled row per month. The sleeve of the month's top
 * record, the month, a sentence with its minutes, the change on the month
 * before and who and what led it, and the month's days as thin stems.
 * Share opens the month as an image.
 */
import { Share2 } from "lucide-react";
import { lazy, Suspense, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { barFractions, change, minutes, monthName } from "../lib/stats";
import type { MonthRecap, StatsRange, StatsSpan } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";
import "./ledger.css";

const RecapCardDialog = lazy(() => import("./RecapCardDialog").then((m) => ({ default: m.RecapCardDialog })));

/** Four covers for a month's card, repeating when it had fewer albums. */
export function recapCovers(r: MonthRecap): (string | null)[] {
  const own = r.topAlbums.map((a) => a.thumb).filter((t): t is string => !!t);
  if (!own.length) return [null, null, null, null];
  return [0, 1, 2, 3].map((i) => own[i % own.length]);
}

function RecapLine({ r }: { r: MonthRecap }) {
  const song = r.topSongs[0];
  const artist = r.topArtists[0];
  const pct = change(r.totals.ms, r.prevMs);
  return (
    <p className="recap__line">
      <span className="num">{minutes(r.totals.ms).toLocaleString()}</span> minutes
      {pct && (
        <>
          , <span className="num">{pct}</span> on the month before
        </>
      )}
      .
      {artist && (
        <>
          {" "}
          Mostly <strong>{artist.name}</strong>
          {song ? (
            <>
              , and <strong>{song.title}</strong>
              {song.unreleased && <span className="stats-mark">Unreleased</span>} played most.
            </>
          ) : (
            "."
          )}
        </>
      )}
    </p>
  );
}

function RecapCard({ month, onShare }: { month: string; onShare: (r: MonthRecap) => void }) {
  const { data: r, error } = useLoad(() => backend.statsMonthRecap(month), [month]);
  const fr = r ? barFractions(r.daily.map((d) => d.ms)) : [];
  const record = r?.topAlbums[0];
  const name = monthName(month);
  const [m, y] = name.split(" ");
  return (
    <li className="recap" data-testid="recap-card">
      <span className="recap__sleeve" aria-hidden="true">
        {r ? <Art path={record?.thumb} fluid requestSize={120} sleeve={record ? { title: record.title, artist: record.artist } : { title: m }} /> : <Skeleton width="100%" height="100%" radius="row" />}
      </span>
      <div className="recap__text">
        <h3 className="recap__month">
          {m} <span className="num">{y}</span>
        </h3>
        {error ? <p className="stats-muted">Couldn't load this month.</p> : !r ? <Skeleton width="80%" height={16} style={{ marginTop: 8 }} /> : <RecapLine r={r} />}
      </div>
      <span className="recap__days" aria-hidden="true">
        {r && fr.map((f, i) => <span key={i} className={r.daily[i].ms > 0 ? "" : "is-off"} style={{ height: `${Math.max(r.daily[i].ms > 0 ? 10 : 0, f * 100)}%` }} />)}
      </span>
      {r && (
        <Button size="sm" variant="ghost" className="recap__share" icon={<Share2 size={14} />} onClick={() => onShare(r)} aria-label={`Share ${name}`}>
          Share
        </Button>
      )}
    </li>
  );
}

/** The months with plays that touch a range, newest first, at most `max`. */
export function monthsIn(span: StatsSpan, range: StatsRange | null, max = 12): string[] {
  const from = range?.from?.slice(0, 7) ?? null;
  const to = range?.to?.slice(0, 7) ?? null;
  return span.months.filter((m) => (!from || m >= from) && (!to || m <= to)).slice(0, max);
}

export function MonthlySection({ months }: { months: string[] }) {
  const [sharing, setSharing] = useState<MonthRecap | null>(null);
  return (
    <section className="stats-section" aria-labelledby="stats-months" data-shot="recaps">
      <div className="ledger-head">
        <h2 id="stats-months" className="ledger-head__title stats-section__title">
          Monthly recaps
        </h2>
      </div>
      {months.length === 0 ? (
        <p className="stats-muted">No month in this range has plays. Pick a longer range to see more months.</p>
      ) : (
        <ul className="recaps">
          {months.map((m) => (
            <RecapCard key={m} month={m} onShare={setSharing} />
          ))}
        </ul>
      )}
      {sharing && (
        <Suspense fallback={null}>
          <RecapCardDialog recap={sharing} onClose={() => setSharing(null)} />
        </Suspense>
      )}
    </section>
  );
}
