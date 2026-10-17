/**
 * A Top Songs list: "Your Top Songs <year>" (100 songs ranked by that
 * year's plays) or All-Time. A banner with the year in big numerals over
 * the year's own record pattern, then the usual action row (play, shuffle,
 * save to Plex as a playlist, download, more, find in the list) and the
 * songs, where the number is the rank. Lists are made on this computer
 * from the listen log and imported history; nothing is added to the
 * library until it's saved.
 */
import { Ellipsis, ListPlus, Pause, Play, Search, Sparkles } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { DownloadButton } from "../components/DownloadButton";
import { ContextShuffleButton } from "../components/ShuffleButton";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton, trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatLongDuration, plural } from "../lib/format";
import type { TopSongsList, TopSongsRow } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";
import { useApp } from "../stores/app";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { wrapped } from "../stores/wrapped";
import { useContextState } from "./cards";
import { usePageBar } from "./detail";
import { toneStyle, YearBanner } from "./TopSongsTile";
import "./liked.css";
import "./top-songs.css";
import { LoadError } from "../components/LoadError";
import { failText } from "../lib/errorText";

/** The play context for a list (also its download id). */
export const topSongsKey = (year: number | null) => (year == null ? "topSongs:all" : `topSongs:${year}`);

/** The small labels under a row's title. */
export function rowNotes(r: TopSongsRow, opts: { allTime: boolean; showPlays: boolean }): string[] {
  const out: string[] = [];
  if (opts.allTime || opts.showPlays) out.push(plural(r.plays, "play"));
  if (!opts.allTime && r.firstOfYear) out.push("First play of the year");
  if (r.unreleased) out.push("Unreleased");
  return out;
}

/** Rows matching a find, keeping their rank. */
export function findRows(rows: TopSongsRow[], query: string): { row: TopSongsRow; rank: number }[] {
  const ranked = rows.map((row, i) => ({ row, rank: i + 1 }));
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return ranked;
  return ranked.filter(({ row: { track: t } }) => {
    const hay = `${t.title} ${t.trackArtist ?? ""} ${t.artistTitle} ${t.albumTitle}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

function Header({ list, year, user }: { list: TopSongsList | null; year: number | null; user: string | null }) {
  const title = year == null ? "All-Time Top Songs" : `Your Top Songs ${year}`;
  return (
    <header className="top-songs__header" style={toneStyle(year)}>
      <div className="top-songs__banner">
        <YearBanner year={year} />
        <h1 className="top-songs__title" aria-label={title}>
          <span className="top-songs__kicker" aria-hidden="true">
            {year == null ? "All-Time" : "Your"} Top Songs
          </span>
          <span className={`top-songs__numerals ${year == null ? "top-songs__numerals--words" : ""}`} aria-hidden="true">
            {year ?? "All time"}
          </span>
        </h1>
      </div>
      <div className="top-songs__about">
        <p className="top-songs__lede">{year == null ? "The songs you've played most, ever." : "The songs you loved most this year"}</p>
        <div className="detail-header__meta">
          <span className="top-songs__made">Made for {user ?? "you"}</span>
          {list ? (
            <>
              <span aria-hidden="true">·</span>
              <span>{plural(list.rows.length, "song")},</span>
              <span className="detail-header__muted">{formatLongDuration(list.durationMs)}</span>
            </>
          ) : (
            <Skeleton width={120} height={12} />
          )}
        </div>
      </div>
    </header>
  );
}

export function TopSongsPage({ year }: { year: number | null }) {
  const key = topSongsKey(year);
  const on = useSettings((s) => s.settings?.wrappedTopSongs !== false);
  const showPlays = useSettings((s) => !!s.settings?.topSongsShowPlays);
  const { data: list, error, retry } = useLoad(() => backend.topSongsList(year), [year, on]);
  const user = useApp((s) => s.auth?.username ?? null);
  const { isContext, playing } = useContextState("mix", key);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const [findOpen, setFindOpen] = useState(false);
  const [query, setQuery] = useState("");
  const findRef = useRef<HTMLInputElement>(null);
  const title = year == null ? "All-Time Top Songs" : `Your Top Songs ${year}`;
  const ids = useMemo(() => list?.rows.map((r) => r.track.id) ?? [], [list]);
  const shown = useMemo(() => (list ? findRows(list.rows, query) : []), [list, query]);
  const play = (start?: string) => player.playMix(key, ids, start);
  const toggle = () => (isContext ? player.toggle() : play());
  usePageBar(list ? title : undefined, isContext && playing, toggle);

  if (!on)
    return (
      <p className="page-pad page-error" data-testid="top-songs">
        Wrapped and Top Songs are off. Turn them on in Settings, under Stats.
      </p>
    );
  if (error && !list)
    return error.kind === "notFound" ? (
      <p className="page-pad page-error" data-testid="top-songs">
        {`There's no Top Songs list for ${year}. A year needs 30 plays for one.`}
      </p>
    ) : (
      <div data-testid="top-songs">
        <LoadError what={`Top Songs ${year}`} error={error} onRetry={retry} />
      </div>
    );

  const save = async () => {
    if (!ids.length) return;
    try {
      const pl = await backend.playlistCreate(title, ids);
      toast.info(`Saved to Plex as a playlist: ${pl.title}`);
    } catch (e) {
      toast.error(failText(`Couldn't save the playlist`, e));
    }
  };

  const pageMenu = (el: HTMLElement) =>
    openContextMenu(
      `Options for ${title}`,
      [
        { label: "Add to queue", disabled: !list?.rows.length, onSelect: () => void player.addToQueue(list!.rows.map((r) => r.track), null) },
        { label: "Save as playlist", disabled: !ids.length, onSelect: () => void save() },
        ...(year != null ? [{ label: `Open ${year} Wrapped`, onSelect: () => wrapped.open(year) }] : []),
        { label: "Find in this list", onSelect: () => setFindOpen(true) },
      ],
      el,
    );

  const allTime = year == null;
  return (
    <div className="detail top-songs" data-testid="top-songs">
      <Header list={list ?? null} year={year} user={user} />
      <div className="detail-body">
        <div className="action-row">
          <IconButton
            label={isContext && playing ? `Pause ${title}` : `Play ${title}`}
            variant="accent"
            size="xl"
            disabled={!ids.length}
            icon={isContext && playing ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
            onClick={toggle}
          />
          <ContextShuffleButton id={key} kind="mix" name={title} />
          <IconButton label="Save to Plex as a playlist" size="lg" icon={<ListPlus size={22} />} disabled={!ids.length} onClick={() => void save()} />
          <DownloadButton kind="mix" id={key} title={title} trackIds={() => ids} disabled={!ids.length} />
          <IconButton label={`More options for ${title}`} size="lg" icon={<Ellipsis size={22} />} disabled={!list} onClick={(e) => pageMenu(e.currentTarget)} />
          {year != null && (
            <button type="button" className="top-songs__wrapped" onClick={() => wrapped.open(year)}>
              <Sparkles size={16} aria-hidden="true" />
              <span>{year} Wrapped</span>
            </button>
          )}
          <div className={`liked-find top-songs__find ${findOpen ? "is-open" : ""}`}>
            {findOpen ? (
              <>
                <Search size={16} aria-hidden="true" className="liked-find__icon" />
                <input
                  ref={findRef}
                  autoFocus
                  type="search"
                  className="liked-find__input"
                  aria-label={`Find in ${title}`}
                  placeholder={`Find in ${title}`}
                  value={query}
                  spellCheck={false}
                  autoComplete="off"
                  onChange={(e) => setQuery(e.target.value)}
                  onBlur={() => !query && setFindOpen(false)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") {
                      e.preventDefault();
                      e.stopPropagation();
                      if (query) setQuery("");
                      else setFindOpen(false);
                    }
                  }}
                />
              </>
            ) : (
              <IconButton label={`Find in ${title}`} icon={<Search size={18} />} disabled={!list} onClick={() => setFindOpen(true)} />
            )}
          </div>
        </div>
        <TrackList label={`${title} songs`}>
          {list
            ? shown.map(({ row: r, rank }) => (
                <TrackRow
                  key={r.track.id}
                  track={r.track}
                  index={rank}
                  showArt
                  notes={rowNotes(r, { allTime, showPlays })}
                  labelNote={`number ${rank}${r.unreleased ? ", unreleased" : ""}`}
                  active={r.track.id === nowTrackId}
                  playing={r.track.id === nowTrackId && status === "playing"}
                  onPlay={() => play(r.track.id)}
                  menuItems={trackMenuItems(r.track)}
                />
              ))
            : Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showArt />)}
        </TrackList>
        {list && query && !shown.length && (
          <p className="home__empty" role="status">
            Nothing in this list matches “{query}”.
          </p>
        )}
        {list && !list.rows.length && (
          <p className="home__empty" role="status">
            None of these songs are in your library right now.{" "}
            <button type="button" className="see-more" onClick={() => nav.go({ name: "stats" })}>
              Open Stats
            </button>
          </p>
        )}
      </div>
    </div>
  );
}
