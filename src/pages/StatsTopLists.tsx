/**
 * Top songs, artists, albums and genres for a range. The #1 gets a large
 * card; every row has its artwork and a slim bar for its plays next to the
 * #1's. Play these and Save as playlist play or keep the list's songs.
 */
import { ListPlus, Play } from "lucide-react";
import { useId, useState } from "react";
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { Button } from "../components/Button";
import { Tabs } from "../components/Tabs";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatPlays, type toRange } from "../lib/stats";
import { duration } from "../lib/statsStory";
import { shareOfTop } from "../lib/statsCharts";
import type { StatsOverview, StatsTopKind, TopSong } from "../lib/statsTypes";
import { player } from "../stores/player";
import { createPlaylist } from "../stores/playlists";
import { nav } from "../stores/router";
import { toast } from "../stores/toasts";

const TOP_TABS: { id: StatsTopKind; label: string }[] = [
  { id: "songs", label: "Songs" },
  { id: "artists", label: "Artists" },
  { id: "albums", label: "Albums" },
  { id: "genres", label: "Genres" },
];
const SHORT_LIST = 10;

/** One row of any list, ready to draw. */
interface Row {
  key: string;
  title: string;
  sub: string | null;
  thumb: string | null;
  round: boolean;
  open: (() => void) | null;
  plays: number;
  ms: number;
  unreleased: boolean;
  song: TopSong | null;
}

function rowsFor(o: StatsOverview, kind: StatsTopKind): Row[] {
  switch (kind) {
    case "songs":
      return o.topSongs.map((s) => ({
        key: s.key,
        title: s.title,
        sub: s.artist,
        thumb: s.track?.thumb ?? null,
        round: false,
        open: null,
        plays: s.plays,
        ms: s.ms,
        unreleased: s.unreleased,
        song: s,
      }));
    case "artists":
      return o.topArtists.map((a) => ({
        key: a.key,
        title: a.name,
        sub: null,
        thumb: a.thumb,
        round: true,
        open: a.artistId ? () => nav.go({ name: "artist", id: a.artistId! }) : null,
        plays: a.plays,
        ms: a.ms,
        unreleased: false,
        song: null,
      }));
    case "albums":
      return o.topAlbums.map((a) => ({
        key: a.key,
        title: a.title,
        sub: a.artist,
        thumb: a.thumb,
        round: false,
        open: a.albumId ? () => nav.go({ name: "album", id: a.albumId! }) : null,
        plays: a.plays,
        ms: a.ms,
        unreleased: a.unreleased,
        song: null,
      }));
    case "genres":
      return o.topGenres.map((g) => ({ key: g.name, title: g.name, sub: null, thumb: g.thumb, round: false, open: null, plays: g.plays, ms: g.ms, unreleased: false, song: null }));
  }
}

const countLine = (plays: number, ms: number) => `${formatPlays(plays)} · ${duration(ms)}`;

/** The slim bar for a row's plays against the #1's. */
function ShareBar({ share }: { share: number }) {
  return (
    <span className="stats-share" aria-hidden="true">
      <span className="stats-share__fill" style={{ width: `${Math.max(2, share * 100)}%` }} />
    </span>
  );
}

function Count({ plays, ms, share }: { plays: number; ms: number; share: number }) {
  return (
    <span className="stats-count">
      <ShareBar share={share} />
      <span className="stats-count__text">{countLine(plays, ms)}</span>
    </span>
  );
}

function RowArt({ r, kind, size }: { r: Row; kind: StatsTopKind; size: number }) {
  if (kind === "genres" && !r.thumb)
    return (
      <span className="stats-genre-tile" style={{ width: size, height: size }} aria-hidden="true">
        {r.title.slice(0, 1).toUpperCase()}
      </span>
    );
  return <Art path={r.thumb} size={size} round={r.round} kind={kind === "artists" ? "artist" : "album"} />;
}

/** The #1, large. */
function Feature({ r, kind, playSongs }: { r: Row; kind: StatsTopKind; playSongs: (first: string) => void }) {
  const label = kind === "songs" ? "Your top song" : kind === "artists" ? "Your top artist" : kind === "albums" ? "Your top album" : "Your top genre";
  const trackId = r.song?.track?.id;
  return (
    <article className="stats-feature" aria-label={`${label}: ${r.title}`} data-testid="stats-feature">
      <span className={`stats-feature__art ${r.round ? "is-round" : ""}`}>
        {kind === "genres" && !r.thumb ? (
          <span className="stats-genre-tile stats-genre-tile--fluid">{r.title.slice(0, 1).toUpperCase()}</span>
        ) : (
          <Art path={r.thumb} fluid requestSize={240} round={r.round} kind={kind === "artists" ? "artist" : "album"} />
        )}
        <span className="stats-feature__rank" aria-hidden="true">
          #1
        </span>
      </span>
      <span className="stats-feature__eyebrow">{label}</span>
      {r.open ? (
        <TextLink className="stats-feature__title" onActivate={r.open}>
          {r.title}
        </TextLink>
      ) : (
        <span className="stats-feature__title">{r.title}</span>
      )}
      {(r.sub || r.unreleased) && (
        <span className="stats-feature__sub">
          {r.unreleased && <span className="stats-mark">Unreleased</span>}
          {r.sub}
        </span>
      )}
      <span className="stats-feature__count">{countLine(r.plays, r.ms)}</span>
      {trackId && (
        <Button size="sm" variant="primary" className="stats-feature__play" icon={<Play size={14} fill="currentColor" />} onClick={() => playSongs(trackId)} aria-label={`Play ${r.title}`}>
          Play
        </Button>
      )}
    </article>
  );
}

function ListRows({ rows, kind, shares, start, playSongs }: { rows: Row[]; kind: StatsTopKind; shares: number[]; start: number; playSongs: (first: string) => void }) {
  if (kind === "songs")
    return (
      <TrackList label="Top songs" header={false}>
        {rows.map((r, i) =>
          r.song?.track ? (
            <TrackRow
              key={r.key}
              rowKey={r.key}
              track={r.song.track}
              index={start + i}
              showArt
              showAlbum={false}
              onPlay={() => playSongs(r.song!.track!.id)}
              trailing={<Count plays={r.plays} ms={r.ms} share={shares[start - 1 + i]} />}
            />
          ) : (
            <div key={r.key} className="stats-row stats-row--gone" role="row">
              <span className="stats-row__num" role="gridcell">
                {start + i}
              </span>
              <span className="stats-row__art" role="gridcell">
                <Art path={r.thumb} size={40} />
              </span>
              <span className="stats-row__text" role="gridcell">
                <span className="stats-row__title truncate">{r.title}</span>
                <span className="stats-row__sub truncate">{r.sub} · not in your library</span>
              </span>
              <span role="gridcell">
                <Count plays={r.plays} ms={r.ms} share={shares[start - 1 + i]} />
              </span>
            </div>
          ),
        )}
      </TrackList>
    );
  return (
    <ol className="stats-list" start={start}>
      {rows.map((r, i) => (
        <li key={r.key} className="stats-row">
          <span className="stats-row__num">{start + i}</span>
          <span className="stats-row__art">
            <RowArt r={r} kind={kind} size={40} />
          </span>
          <span className="stats-row__text">
            {r.open ? (
              <TextLink className="stats-row__title truncate" onActivate={r.open}>
                {r.title}
              </TextLink>
            ) : (
              <span className="stats-row__title truncate">{r.title}</span>
            )}
            {(r.sub || r.unreleased) && (
              <span className="stats-row__sub truncate">
                {r.unreleased && <span className="stats-mark">Unreleased</span>}
                {r.sub}
              </span>
            )}
          </span>
          <Count plays={r.plays} ms={r.ms} share={shares[start - 1 + i]} />
        </li>
      ))}
    </ol>
  );
}

export function TopLists({ o, range, title }: { o: StatsOverview; range: ReturnType<typeof toRange>; title: string }) {
  const [kind, setKind] = useState<StatsTopKind>("songs");
  const [more, setMore] = useState(false);
  const panel = useId();
  const rows = rowsFor(o, kind);
  const total = rows.length;
  const shares = shareOfTop(rows.map((r) => r.plays));
  const songIds = o.topSongs.map((s) => s.track?.id).filter((x): x is string => !!x);
  const playSongs = (first: string) => void player.playTracks(songIds, first);
  const ids = () => backend.statsPlayIds(range, kind, 50);
  const playThese = async () => {
    try {
      const list = await ids();
      if (!list.length) return toast.info("None of these are in your library");
      await player.playTracks(list, list[0]);
    } catch {
      toast.error("Couldn't play these");
    }
  };
  const save = async () => {
    try {
      const list = await ids();
      if (!list.length) return toast.info("None of these are in your library");
      await createPlaylist({ title: `Top ${kind}, ${title}`, trackIds: list, open: false });
    } catch {
      toast.error("Couldn't make the playlist");
    }
  };
  const shown = rows.slice(1, more ? 50 : SHORT_LIST);
  return (
    <section className="stats-section" aria-labelledby={`${panel}-h`} data-shot="tops">
      <div className="stats-section__head">
        <h2 id={`${panel}-h`} className="stats-section__title">
          Your top {kind}
        </h2>
        <span className="stats-section__actions">
          <Button size="sm" variant="primary" icon={<Play size={14} fill="currentColor" />} disabled={!total} onClick={() => void playThese()}>
            Play these
          </Button>
          <Button size="sm" icon={<ListPlus size={14} />} disabled={!total} onClick={() => void save()}>
            Save as playlist
          </Button>
        </span>
      </div>
      <Tabs
        label="Top lists"
        items={TOP_TABS}
        value={kind}
        idPrefix={panel}
        onChange={(k) => {
          setKind(k);
          setMore(false);
        }}
      />
      <div id={`${panel}-${kind}`} role="tabpanel" className="stats-section__panel">
        {total === 0 ? (
          <p className="stats-muted">Nothing here for this range.</p>
        ) : (
          <div className={`stats-top stats-top--${kind}`}>
            <Feature r={rows[0]} kind={kind} playSongs={playSongs} />
            <div className="stats-top__rest">
              {shown.length > 0 && <ListRows rows={shown} kind={kind} shares={shares} start={2} playSongs={playSongs} />}
              {total > SHORT_LIST && (
                <Button size="sm" variant="ghost" className="stats-more" onClick={() => setMore(!more)}>
                  {more ? "Show less" : "Show more"}
                </Button>
              )}
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
