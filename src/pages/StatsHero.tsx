/**
 * The top of the Stats page: one sentence for the range (how long, on how
 * many days, and the artist who took the most of it), the totals as a
 * quiet line under it, the top artist's photo, and three facts in a row
 * (top song, longest streak, new artists found).
 */
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { Skeleton } from "../components/Skeleton";
import { formatPlays, shortDate } from "../lib/stats";
import type { StatsMoments, StatsOverview } from "../lib/statsTypes";
import { dayLabel, heroSentence } from "../lib/statsStory";
import { player } from "../stores/player";
import { nav } from "../stores/router";

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
  const artist = o.topArtists[0];
  const song = o.topSongs[0];
  const s = heroSentence(o);
  const dates = o.from === o.to ? shortDate(o.from) : `${shortDate(o.from)} to ${shortDate(o.to)}`;
  const openArtist = artist?.artistId ? () => nav.go({ name: "artist", id: artist.artistId! }) : null;
  const playTop = () => {
    const ids = o.topSongs.map((x) => x.track?.id).filter((x): x is string => !!x);
    if (song?.track) void player.playTracks(ids, song.track.id);
  };
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
      <div className="stats-hero__top">
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
        {artist && (
          <figure className="stats-hero__artist">
            <span className="stats-hero__photo">
              <Art path={artist.thumb} fluid requestSize={168} round kind="artist" />
            </span>
            <figcaption>
              <span className="stats-hero__artist-label">Top artist</span>
              {openArtist ? (
                <TextLink className="stats-hero__artist-name" onActivate={openArtist}>
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
        <Fact label="Top song">
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
      <div className="stats-hero__top">
        <div className="stats-hero__main">
          <Skeleton width={180} height={14} />
          <Skeleton width="90%" height={36} style={{ marginTop: 16 }} />
          <Skeleton width="60%" height={36} style={{ marginTop: 8 }} />
          <Skeleton width={300} height={14} style={{ marginTop: 20 }} />
        </div>
        <Skeleton width={152} height={152} radius="circle" />
      </div>
    </div>
  );
}
