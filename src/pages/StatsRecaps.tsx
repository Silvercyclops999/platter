/**
 * Monthly recaps: a card per month built from that month's covers, with
 * its minutes and the change on the month before, the daily listening,
 * and its top artist and song. Share opens the month as an image.
 */
import { Share2 } from "lucide-react";
import { lazy, Suspense, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { barFractions, change, minutes, monthName } from "../lib/stats";
import type { MonthRecap, StatsSpan } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";

const RecapCardDialog = lazy(() => import("./RecapCardDialog").then((m) => ({ default: m.RecapCardDialog })));

/** Four covers for a month's card, repeating when it had fewer albums. */
export function recapCovers(r: MonthRecap): (string | null)[] {
  const own = r.topAlbums.map((a) => a.thumb).filter((t): t is string => !!t);
  if (!own.length) return [null, null, null, null];
  return [0, 1, 2, 3].map((i) => own[i % own.length]);
}

function RecapCard({ month, onShare }: { month: string; onShare: (r: MonthRecap) => void }) {
  const { data: r, error } = useLoad(() => backend.statsMonthRecap(month), [month]);
  const fr = r ? barFractions(r.daily.map((d) => d.ms)) : [];
  const song = r?.topSongs[0];
  const artist = r?.topArtists[0];
  const pct = r ? change(r.totals.ms, r.prevMs) : null;
  const name = monthName(month);
  return (
    <li className="recap-card" data-testid="recap-card">
      <div className="recap-card__covers" aria-hidden="true">
        {r ? recapCovers(r).map((c, i) => <Art key={i} path={c} fluid requestSize={120} />) : [0, 1, 2, 3].map((i) => <Skeleton key={i} width="100%" height="100%" radius="row" />)}
      </div>
      <div className="recap-card__body">
        <h3 className="recap-card__month">
          {name.split(" ")[0]} <span>{name.split(" ")[1]}</span>
        </h3>
        {error ? (
          <p className="stats-muted">Couldn't load this month.</p>
        ) : !r ? (
          <>
            <Skeleton width="60%" height={28} />
            <Skeleton width="80%" height={14} style={{ marginTop: 12 }} />
          </>
        ) : (
          <>
            <p className="recap-card__minutes">
              {minutes(r.totals.ms).toLocaleString()} <span>minutes</span>
              {pct && <span className={`recap-card__change ${pct.startsWith("-") ? "is-down" : ""}`}>{pct}</span>}
            </p>
            <div className="recap-card__bars" aria-hidden="true">
              {fr.map((f, i) => (
                <span key={i} style={{ height: `${Math.max(r.daily[i].ms > 0 ? 8 : 0, f * 100)}%` }} />
              ))}
            </div>
            <div className="recap-card__tops">
              <div className="recap-card__top">
                <span className="recap-card__face" aria-hidden="true">
                  <Art path={artist?.thumb} size={28} round kind="artist" />
                </span>
                <span className="recap-card__top-text">
                  <span className="recap-card__label">Top artist</span>
                  <span className="recap-card__value truncate">{artist?.name ?? "None"}</span>
                </span>
              </div>
              <div className="recap-card__top">
                <span className="recap-card__face" aria-hidden="true">
                  <Art path={song?.track?.thumb} size={28} />
                </span>
                <span className="recap-card__top-text">
                  <span className="recap-card__label">Top song</span>
                  <span className="recap-card__value truncate">
                    {song ? song.title : "None"}
                    {song?.unreleased && <span className="stats-mark">Unreleased</span>}
                  </span>
                </span>
              </div>
            </div>
            <Button size="sm" icon={<Share2 size={14} />} onClick={() => onShare(r)} aria-label={`Share ${name}`}>
              Share
            </Button>
          </>
        )}
      </div>
    </li>
  );
}

export function MonthlySection({ span }: { span: StatsSpan }) {
  const [sharing, setSharing] = useState<MonthRecap | null>(null);
  const months = span.months.slice(0, 12);
  if (!months.length) return null;
  return (
    <section className="stats-section" aria-labelledby="stats-months" data-shot="recaps">
      <h2 id="stats-months" className="stats-section__title">
        Monthly recaps
      </h2>
      <ul className="recap-cards">
        {months.map((m) => (
          <RecapCard key={m} month={m} onShare={setSharing} />
        ))}
      </ul>
      {sharing && (
        <Suspense fallback={null}>
          <RecapCardDialog recap={sharing} onClose={() => setSharing(null)} />
        </Suspense>
      )}
    </section>
  );
}
