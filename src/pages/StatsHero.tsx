/**
 * The top of the Stats overview: one sentence for the range (how long, on
 * how many days, and the share of the artist who took the most of it), the
 * totals in the readout face under it, and two facts as ruled lines
 * (longest streak, new artists found). Beside it, the record you played
 * most in the range, as its sleeve. The minutes chart right under the hero
 * backs the sentence up.
 */
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { Skeleton } from "../components/Skeleton";
import { formatPlays, shortDate } from "../lib/stats";
import type { StatsMoments, StatsOverview } from "../lib/statsTypes";
import { dayLabel, heroSentence } from "../lib/statsStory";
import { nav } from "../stores/router";
import "./ledger.css";

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <li className="ledger__row stats-fact">
      <span className="ledger__label stats-fact__label">{label}</span>
      <span className="ledger__value stats-fact__value">{children}</span>
    </li>
  );
}

/** The record behind the range: the album played most, as its sleeve. The artist the sentence already names isn't said again. */
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
        <span className="eyebrow">Played most</span>
        {open ? (
          <TextLink className="stats-record__title" onActivate={open}>
            {a.title}
          </TextLink>
        ) : (
          <span className="stats-record__title">{a.title}</span>
        )}
        <span className="stats-record__sub">
          {a.artist !== named && <>{a.artist} </>}
          <span className="num">{formatPlays(a.plays)}</span>
        </span>
      </figcaption>
    </figure>
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
        <p className="eyebrow stats-hero__eyebrow">
          {title}
          <span className="stats-hero__dates">{dates}</span>
        </p>
        <p className="lead stats-hero__sentence" data-testid="hero-sentence">
          You listened for <strong>{s.time}</strong> on <strong>{s.days}</strong>.
          {s.artist && (
            <>
              {" "}
              <strong>{s.artist}</strong> took {s.share}.
            </>
          )}
        </p>
        <p className="lead-facts stats-hero__totals">{totals.join(" · ")}</p>
        <ul className="ledger stats-hero__facts">
          <Fact label="Longest streak">
            {o.streak ? `${o.streak.days.toLocaleString()} ${o.streak.days === 1 ? "day" : "days"} in a row` : "None yet"}
            {o.streak && o.streak.days > 1 && (
              <small className="stats-fact__sub">
                {dayLabel(o.streak.from, o.to.slice(0, 4))} to {dayLabel(o.streak.to, o.to.slice(0, 4))}
              </small>
            )}
          </Fact>
          <Fact label={isAll ? "Artists you've played" : "New artists found"}>
            {isAll ? (
              t.artists.toLocaleString()
            ) : !moments ? (
              <Skeleton width={120} height={16} />
            ) : (
              <>
                {moments.newArtists ? `${moments.newArtists.toLocaleString()} new ${moments.newArtists === 1 ? "artist" : "artists"}` : "None this time"}
                {moments.discovery && <small className="stats-fact__sub truncate">Most played: {moments.discovery.artist.name}</small>}
              </>
            )}
          </Fact>
        </ul>
      </div>
      <TopRecord o={o} named={s.artist ?? null} />
    </section>
  );
}

/** The hero's shape while counting. */
export function StatsHeroSkeleton() {
  return (
    <div className="stats-hero stats-hero--loading" aria-hidden="true">
      <div className="stats-hero__main">
        <Skeleton width={180} height={12} />
        <Skeleton width="90%" height={40} style={{ marginTop: 16 }} />
        <Skeleton width="60%" height={40} style={{ marginTop: 8 }} />
        <Skeleton width={300} height={14} style={{ marginTop: 20 }} />
      </div>
      <Skeleton width="100%" height={220} radius="row" />
    </div>
  );
}
