/**
 * The top of the Stats overview: one plain line for the range ("19 hrs over
 * 24 days, 329 plays") with its dates under it, and beside it the record you
 * played most, as its sleeve. No sentence about the numbers, no charts.
 */
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { Skeleton } from "../components/Skeleton";
import { formatPlays, shortDate } from "../lib/stats";
import type { StatsMoments, StatsOverview } from "../lib/statsTypes";
import { shortListen } from "./MonthStory";
import { nav } from "../stores/router";
import "./ledger.css";

/** "19 hrs over 24 days, 329 plays." */
export function heroLine(o: StatsOverview): string {
  const t = o.totals;
  return `${shortListen(t.ms)} over ${t.days.toLocaleString()} ${t.days === 1 ? "day" : "days"}, ${formatPlays(t.plays)}.`;
}

/** The record behind the range: the album played most, as its sleeve. */
function TopRecord({ o, named }: { o: StatsOverview; named: string | null }) {
  const a = o.topAlbums[0];
  if (!a) return null;
  const open = a.albumId ? () => nav.go({ name: "album", id: a.albumId! }) : null;
  return (
    <figure className="stats-record" data-testid="stats-record">
      <span className="stats-record__sleeve">
        <Art path={a.thumb} fluid requestSize={240} sleeve={{ title: a.title, artist: a.artist }} />
      </span>
      <figcaption className="stats-record__caption">
        <span className="lg-eyebrow">Played most</span>
        {open ? (
          <TextLink className="stats-record__title" onActivate={open}>
            {a.title}
          </TextLink>
        ) : (
          <span className="stats-record__title">{a.title}</span>
        )}
        <span className="stats-record__sub">
          {a.artist !== named && <>{a.artist} </>}
          <span className="lg-num">{formatPlays(a.plays)}</span>
        </span>
      </figcaption>
    </figure>
  );
}

export function StatsHero({ o, title }: { o: StatsOverview; moments?: StatsMoments | undefined; title: string; isAll?: boolean }) {
  const dates = o.from === o.to ? shortDate(o.from) : `${shortDate(o.from)} to ${shortDate(o.to)}`;
  return (
    <section className="stats-hero" aria-label={`${title} at a glance`} data-shot="hero" data-testid="stats-hero">
      <div className="stats-hero__main">
        <h2 className="stats-hero__line" data-testid="hero-sentence">
          {heroLine(o)}
        </h2>
        <p className="stats-hero__dates">{dates}</p>
      </div>
      <TopRecord o={o} named={null} />
    </section>
  );
}

/** The hero's shape while counting. */
export function StatsHeroSkeleton() {
  return (
    <div className="stats-hero stats-hero--loading" aria-hidden="true">
      <div className="stats-hero__main">
        <Skeleton width="60%" height={32} />
        <Skeleton width={180} height={14} style={{ marginTop: 12 }} />
      </div>
      <Skeleton width="100%" height={220} radius="row" />
    </div>
  );
}
