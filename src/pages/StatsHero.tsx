/**
 * The top of the Stats overview: one sentence for the range (how long, on
 * how many days, and the share of the artist who took the most of it), the
 * totals as a quiet line under it, and two facts in a row (longest streak,
 * new artists found). The top song and artist themselves lead the top
 * lists just below, so they aren't repeated here.
 */
import { Skeleton } from "../components/Skeleton";
import { shortDate } from "../lib/stats";
import type { StatsMoments, StatsOverview } from "../lib/statsTypes";
import { dayLabel, heroSentence } from "../lib/statsStory";

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <li className="stats-fact">
      <span className="stats-fact__label">{label}</span>
      {children}
    </li>
  );
}

export function StatsHero({ o, moments, title, isAll }: { o: StatsOverview; moments: StatsMoments | undefined; title: string; isAll: boolean }) {
  const t = o.totals;
  const s = heroSentence(o);
  const dates = o.from === o.to ? shortDate(o.from) : `${shortDate(o.from)} to ${shortDate(o.to)}`;
  const totals = (
    [
      ["plays", t.plays],
      ["songs", t.songs],
      ["artists", t.artists],
      ["albums", t.albums],
    ] as const
  ).map(([k, v]) => `${v.toLocaleString()} ${v === 1 ? k.slice(0, -1) : k}`);
  return (
    <section className="stats-hero" aria-label={`${title} at a glance`} data-shot="hero" data-testid="stats-hero">
      <div className="stats-hero__main">
        <p className="stats-hero__eyebrow">
          {title}
          <span className="stats-hero__dates">{dates}</span>
        </p>
        <p className="stats-hero__sentence" data-testid="hero-sentence">
          You listened for <strong>{s.time}</strong> on <strong>{s.days}</strong>.
          {s.artist && (
            <>
              {" "}
              <strong>{s.artist}</strong> took {s.share}.
            </>
          )}
        </p>
        <p className="stats-hero__totals">{totals.join(" · ")}</p>
      </div>
      <ul className="stats-hero__facts">
        <Fact label="Longest streak">
          <span className="stats-fact__value">{o.streak ? `${o.streak.days.toLocaleString()} ${o.streak.days === 1 ? "day" : "days"} in a row` : "None yet"}</span>
          {o.streak && o.streak.days > 1 && (
            <span className="stats-fact__sub">
              {dayLabel(o.streak.from, o.to.slice(0, 4))} to {dayLabel(o.streak.to, o.to.slice(0, 4))}
            </span>
          )}
        </Fact>
        <Fact label={isAll ? "Artists you've played" : "New artists found"}>
          {isAll ? (
            <span className="stats-fact__value">{t.artists.toLocaleString()}</span>
          ) : !moments ? (
            <Skeleton width={120} height={18} />
          ) : (
            <>
              <span className="stats-fact__value">{moments.newArtists ? `${moments.newArtists.toLocaleString()} new ${moments.newArtists === 1 ? "artist" : "artists"}` : "None this time"}</span>
              {moments.discovery && <span className="stats-fact__sub truncate">Most played: {moments.discovery.artist.name}</span>}
            </>
          )}
        </Fact>
      </ul>
    </section>
  );
}

/** The hero's shape while counting. */
export function StatsHeroSkeleton() {
  return (
    <div className="stats-hero stats-hero--loading" aria-hidden="true">
      <div className="stats-hero__main">
        <Skeleton width={180} height={14} />
        <Skeleton width="90%" height={36} style={{ marginTop: 16 }} />
        <Skeleton width="60%" height={36} style={{ marginTop: 8 }} />
        <Skeleton width={300} height={14} style={{ marginTop: 20 }} />
      </div>
    </div>
  );
}
