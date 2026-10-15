/**
 * The top of the Stats page: the range's minutes listened, large, over a
 * soft mosaic of its most played album covers, the top artist's photo, and
 * three headline facts (top song, longest streak, new artists found).
 */
import { Flame, Music2, Sparkles } from "lucide-react";
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { Skeleton } from "../components/Skeleton";
import { formatPlays, minutes, shortDate } from "../lib/stats";
import type { StatsMoments, StatsOverview } from "../lib/statsTypes";
import { player } from "../stores/player";
import { nav } from "../stores/router";

/** Covers for the mosaic: the range's top albums with art, repeated to fill. */
export function mosaicCovers(o: StatsOverview, n = 36): string[] {
  const own = [...o.topAlbums.map((a) => a.thumb), ...o.topSongs.map((s) => s.track?.thumb ?? null)].filter((t): t is string => !!t);
  const unique = [...new Set(own)];
  if (!unique.length) return [];
  return Array.from({ length: n }, (_, i) => unique[i % unique.length]);
}

function Fact({ label, icon, children }: { label: string; icon: React.ReactNode; children: React.ReactNode }) {
  return (
    <li className="stats-fact">
      <span className="stats-fact__icon" aria-hidden="true">
        {icon}
      </span>
      <span className="stats-fact__body">
        <span className="stats-fact__label">{label}</span>
        {children}
      </span>
    </li>
  );
}

export function StatsHero({ o, moments, title, isAll }: { o: StatsOverview; moments: StatsMoments | undefined; title: string; isAll: boolean }) {
  const t = o.totals;
  const covers = mosaicCovers(o);
  const artist = o.topArtists[0];
  const song = o.topSongs[0];
  const dates = o.from === o.to ? shortDate(o.from) : `${shortDate(o.from)} to ${shortDate(o.to)}`;
  const playTop = () => {
    const ids = o.topSongs.map((s) => s.track?.id).filter((x): x is string => !!x);
    if (song?.track) void player.playTracks(ids, song.track.id);
  };
  return (
    <section className="stats-hero" aria-label={`${title} at a glance`} data-shot="hero" data-testid="stats-hero">
      <div className="stats-hero__mosaic" aria-hidden="true">
        {covers.map((c, i) => (
          <span key={i} className="stats-hero__tile">
            <Art path={c} fluid requestSize={140} />
          </span>
        ))}
      </div>
      <div className="stats-hero__scrim" aria-hidden="true" />
      <div className="stats-hero__top">
        <div className="stats-hero__main">
          <p className="stats-hero__eyebrow">
            {title}
            <span className="stats-hero__dates">{dates}</span>
          </p>
          <p className="stats-hero__minutes" data-testid="hero-minutes">
            <span className="stats-hero__number">{minutes(t.ms).toLocaleString()}</span>
            <span className="stats-hero__unit">minutes listened</span>
          </p>
          <dl className="stats-hero__totals">
            {(
              [
                ["Plays", t.plays],
                ["Songs", t.songs],
                ["Artists", t.artists],
                ["Albums", t.albums],
                ["Days", t.days],
              ] as const
            ).map(([k, v]) => (
              <div key={k}>
                <dt>{k}</dt>
                <dd>{v.toLocaleString()}</dd>
              </div>
            ))}
          </dl>
        </div>
        {artist && (
          <figure className="stats-hero__artist">
            <span className="stats-hero__photo">
              <Art path={artist.thumb} fluid requestSize={168} round kind="artist" />
            </span>
            <figcaption>
              <span className="stats-hero__artist-label">Top artist</span>
              {artist.artistId ? (
                <TextLink className="stats-hero__artist-name" onActivate={() => nav.go({ name: "artist", id: artist.artistId! })}>
                  {artist.name}
                </TextLink>
              ) : (
                <span className="stats-hero__artist-name">{artist.name}</span>
              )}
              <span className="stats-hero__artist-sub">{formatPlays(artist.plays)}</span>
            </figcaption>
          </figure>
        )}
      </div>
      <ul className="stats-hero__facts">
        <Fact label="Top song" icon={<Music2 size={16} />}>
          {song ? (
            <span className="stats-fact__song">
              <span className="stats-fact__art">
                <Art path={song.track?.thumb ?? null} size={40} />
              </span>
              <span className="stats-fact__text">
                {song.track ? (
                  <TextLink className="stats-fact__value truncate" onActivate={playTop}>
                    {song.title}
                  </TextLink>
                ) : (
                  <span className="stats-fact__value truncate">{song.title}</span>
                )}
                <span className="stats-fact__sub truncate">
                  {song.artist} · {formatPlays(song.plays)}
                </span>
              </span>
            </span>
          ) : (
            <span className="stats-fact__value">None yet</span>
          )}
        </Fact>
        <Fact label="Longest streak" icon={<Flame size={16} />}>
          <span className="stats-fact__value">{o.streak ? `${o.streak.days.toLocaleString()} ${o.streak.days === 1 ? "day" : "days"} in a row` : "None yet"}</span>
          {o.streak && o.streak.days > 1 && (
            <span className="stats-fact__sub">
              {shortDate(o.streak.from)} to {shortDate(o.streak.to)}
            </span>
          )}
        </Fact>
        <Fact label={isAll ? "Artists you've played" : "New artists found"} icon={<Sparkles size={16} />}>
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
      <div className="stats-hero__top">
        <div className="stats-hero__main">
          <Skeleton width={180} height={14} />
          <Skeleton width={320} height={72} style={{ marginTop: 16 }} />
          <Skeleton width={360} height={32} style={{ marginTop: 20 }} />
        </div>
        <Skeleton width={168} height={168} radius="circle" />
      </div>
      <div className="stats-hero__facts">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} height={76} width="100%" radius="card" />
        ))}
      </div>
    </div>
  );
}
