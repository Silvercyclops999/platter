/**
 * Stats: your listening for a range of days. Range chips (4 weeks, 6
 * months, this year, a picked year or month, all time, custom dates), the
 * totals, a bar chart by day or month, streak and busiest times, and top
 * songs, artists, albums and genres with Play these and Save as playlist.
 * Below them, monthly recaps you can share as a card, and comparing with a
 * friend when that's on. Everything is counted on this computer.
 */
import { BarChart3, ListPlus, Play, Share2 } from "lucide-react";
import { ShelfStatsSection } from "./ShelfPage";
import { lazy, Suspense, useId, useMemo, useState } from "react";
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { Button } from "../components/Button";
import { Select } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { ChipRow, Tabs } from "../components/Tabs";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import {
  WEEKDAYS,
  barFractions,
  formatMinutes,
  formatPlays,
  hourLabel,
  minutes,
  monthName,
  rangeLabel,
  shortDate,
  toRange,
  type RangeChoice,
} from "../lib/stats";
import type { MonthRecap, StatsOverview, StatsPoint, StatsSpan, StatsTopKind, TopSong } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";
import { player } from "../stores/player";
import { createPlaylist } from "../stores/playlists";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { CompareSection } from "./StatsCompare";
import "./stats.css";

const RecapCardDialog = lazy(() => import("./RecapCardDialog").then((m) => ({ default: m.RecapCardDialog })));

type ChipId = RangeChoice["kind"];
const CHIPS: { id: ChipId; label: string }[] = [
  { id: "4w", label: "4 weeks" },
  { id: "6m", label: "6 months" },
  { id: "year", label: "This year" },
  { id: "pickYear", label: "A year" },
  { id: "pickMonth", label: "A month" },
  { id: "all", label: "All time" },
  { id: "custom", label: "Custom" },
];

/** The years and months the pickers offer, newest first. */
function pickable(span: StatsSpan | undefined): { years: number[]; months: string[] } {
  const months = span?.months ?? [];
  const years = [...new Set(months.map((m) => Number(m.slice(0, 4))))];
  return { years, months };
}

function choiceFor(id: ChipId, span: StatsSpan | undefined, prev: RangeChoice): RangeChoice {
  const today = span?.today ?? new Date().toISOString().slice(0, 10);
  const { years, months } = pickable(span);
  switch (id) {
    case "pickYear":
      return { kind: "pickYear", year: years[0] ?? Number(today.slice(0, 4)) };
    case "pickMonth":
      return { kind: "pickMonth", month: months[0] ?? today.slice(0, 7) };
    case "custom":
      return prev.kind === "custom" ? prev : { kind: "custom", ...toRange({ kind: "4w" }, today) } as RangeChoice;
    default:
      return { kind: id } as RangeChoice;
  }
}

function RangePicker({ choice, span, onChange }: { choice: RangeChoice; span: StatsSpan | undefined; onChange: (c: RangeChoice) => void }) {
  const { years, months } = pickable(span);
  return (
    <div className="stats-range">
      <ChipRow label="Range" scroll>
        {CHIPS.map((c) => (
          <button
            key={c.id}
            type="button"
            aria-pressed={choice.kind === c.id}
            className={`chip ${choice.kind === c.id ? "is-selected" : ""}`}
            onClick={() => onChange(choiceFor(c.id, span, choice))}
          >
            {c.label}
          </button>
        ))}
      </ChipRow>
      {choice.kind === "pickYear" && years.length > 0 && (
        <Select
          label="Year"
          value={String(choice.year)}
          options={years.map((y) => ({ value: String(y), label: String(y) }))}
          onChange={(v) => onChange({ kind: "pickYear", year: Number(v) })}
        />
      )}
      {choice.kind === "pickMonth" && months.length > 0 && (
        <Select label="Month" value={choice.month} options={months.map((m) => ({ value: m, label: monthName(m) }))} onChange={(v) => onChange({ kind: "pickMonth", month: v })} />
      )}
      {choice.kind === "custom" && (
        <span className="stats-range__dates">
          <label>
            <span className="sr-only">From</span>
            <input
              type="date"
              className="stats-date"
              value={choice.from}
              max={span?.today}
              onChange={(e) => e.target.value && onChange({ ...choice, from: e.target.value })}
            />
          </label>
          <span aria-hidden="true">to</span>
          <label>
            <span className="sr-only">To</span>
            <input type="date" className="stats-date" value={choice.to} max={span?.today} onChange={(e) => e.target.value && onChange({ ...choice, to: e.target.value })} />
          </label>
        </span>
      )}
    </div>
  );
}

function Tile({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="stats-tile">
      <dt className="stats-tile__label">{label}</dt>
      <dd className="stats-tile__value">
        {value}
        {sub && <span className="stats-tile__sub">{sub}</span>}
      </dd>
    </div>
  );
}

function pointLabel(p: StatsPoint, bucket: "day" | "month"): string {
  return bucket === "day" ? shortDate(p.key) : monthName(p.key);
}

/** Minutes per day or month as plain bars. Hover or focus a bar for its numbers. */
export function BarChart({ points, bucket, label }: { points: StatsPoint[]; bucket: "day" | "month"; label: string }) {
  const fr = barFractions(points.map((p) => p.ms));
  const [hover, setHover] = useState<number | null>(null);
  const shown = hover != null ? points[hover] : null;
  const first = points[0];
  const last = points[points.length - 1];
  return (
    <figure className="stats-chart" aria-label={label}>
      <div className="stats-chart__readout" aria-live="polite">
        {shown ? `${pointLabel(shown, bucket)}: ${formatMinutes(shown.ms)}, ${formatPlays(shown.plays)}` : `Minutes listened per ${bucket}`}
      </div>
      <div className="stats-chart__bars" role="list" onMouseLeave={() => setHover(null)}>
        {points.map((p, i) => (
          <span
            key={p.key}
            role="listitem"
            className={`stats-chart__bar ${hover === i ? "is-hover" : ""}`}
            aria-label={`${pointLabel(p, bucket)}: ${minutes(p.ms)} minutes`}
            onMouseEnter={() => setHover(i)}
          >
            <span className="stats-chart__fill" style={{ height: `${Math.max(p.ms > 0 ? 2 : 0, fr[i] * 100)}%` }} />
          </span>
        ))}
      </div>
      {first && last && (
        <figcaption className="stats-chart__axis">
          <span>{pointLabel(first, bucket)}</span>
          <span>{pointLabel(last, bucket)}</span>
        </figcaption>
      )}
    </figure>
  );
}

function HourChart({ hours }: { hours: number[] }) {
  const fr = barFractions(hours);
  return (
    <div className="stats-hours" role="img" aria-label={`Listening by hour of the day. ${hours.map((h, i) => `${hourLabel(i)}: ${minutes(h)} minutes`).join(", ")}`}>
      {fr.map((f, i) => (
        <span key={i} className="stats-hours__bar" style={{ height: `${Math.max(hours[i] > 0 ? 4 : 0, f * 100)}%` }} />
      ))}
    </div>
  );
}

const TOP_TABS: { id: StatsTopKind; label: string }[] = [
  { id: "songs", label: "Songs" },
  { id: "artists", label: "Artists" },
  { id: "albums", label: "Albums" },
  { id: "genres", label: "Genres" },
];
const SHORT_LIST = 10;

function countLine(plays: number, ms: number): string {
  return `${formatPlays(plays)} · ${formatMinutes(ms)}`;
}

function SongRows({ songs, all }: { songs: TopSong[]; all: TopSong[] }) {
  const ids = all.map((s) => s.track?.id).filter((x): x is string => !!x);
  return (
    <TrackList label="Top songs" header={false}>
      {songs.map((s, i) =>
        s.track ? (
          <TrackRow
            key={s.key}
            rowKey={s.key}
            track={s.track}
            index={i + 1}
            showArt
            showAlbum={false}
            onPlay={() => void player.playTracks(ids, s.track!.id)}
            trailing={<span className="stats-count">{countLine(s.plays, s.ms)}</span>}
          />
        ) : (
          <div key={s.key} className="stats-row stats-row--gone" role="row">
            <span className="stats-row__num" role="gridcell">
              {i + 1}
            </span>
            <span className="stats-row__text" role="gridcell">
              <span className="stats-row__title truncate">{s.title}</span>
              <span className="stats-row__sub truncate">{s.artist} · not in your library</span>
            </span>
            <span className="stats-count" role="gridcell">
              {countLine(s.plays, s.ms)}
            </span>
          </div>
        ),
      )}
    </TrackList>
  );
}

function TopRows({ o, kind, count }: { o: StatsOverview; kind: StatsTopKind; count: number }) {
  if (kind === "songs") return <SongRows songs={o.topSongs.slice(0, count)} all={o.topSongs} />;
  const rows =
    kind === "artists"
      ? o.topArtists.map((a) => ({ key: a.key, title: a.name, sub: null as string | null, thumb: a.thumb, open: a.artistId ? () => nav.go({ name: "artist", id: a.artistId! }) : null, round: true, plays: a.plays, ms: a.ms, unreleased: false }))
      : kind === "albums"
        ? o.topAlbums.map((a) => ({ key: a.key, title: a.title, sub: a.artist, thumb: a.thumb, open: a.albumId ? () => nav.go({ name: "album", id: a.albumId! }) : null, round: false, plays: a.plays, ms: a.ms, unreleased: a.unreleased }))
        : o.topGenres.map((g) => ({ key: g.name, title: g.name, sub: null, thumb: null, open: null, round: false, plays: g.plays, ms: g.ms, unreleased: false }));
  return (
    <ol className="stats-list">
      {rows.slice(0, count).map((r, i) => (
        <li key={r.key} className="stats-row">
          <span className="stats-row__num">{i + 1}</span>
          {kind !== "genres" && (
            <span className="stats-row__art">
              <Art path={r.thumb} size={40} round={r.round} kind={kind === "artists" ? "artist" : "album"} />
            </span>
          )}
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
          <span className="stats-count">{countLine(r.plays, r.ms)}</span>
        </li>
      ))}
    </ol>
  );
}

function TopLists({ o, range, title }: { o: StatsOverview; range: ReturnType<typeof toRange>; title: string }) {
  const [kind, setKind] = useState<StatsTopKind>("songs");
  const [more, setMore] = useState(false);
  const panel = useId();
  const total = kind === "songs" ? o.topSongs.length : kind === "artists" ? o.topArtists.length : kind === "albums" ? o.topAlbums.length : o.topGenres.length;
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
  return (
    <section className="stats-section" aria-labelledby={`${panel}-h`}>
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
        {total === 0 ? <p className="stats-muted">Nothing here for this range.</p> : <TopRows o={o} kind={kind} count={more ? 50 : SHORT_LIST} />}
        {total > SHORT_LIST && (
          <Button size="sm" variant="ghost" className="stats-more" onClick={() => setMore(!more)}>
            {more ? "Show less" : "Show more"}
          </Button>
        )}
      </div>
    </section>
  );
}

function Overview({ o, range, title }: { o: StatsOverview; range: ReturnType<typeof toRange>; title: string }) {
  const t = o.totals;
  return (
    <>
      <dl className="stats-tiles">
        <Tile label="Minutes listened" value={minutes(t.ms).toLocaleString()} />
        <Tile label="Plays" value={t.plays.toLocaleString()} />
        <Tile label="Songs" value={t.songs.toLocaleString()} />
        <Tile label="Artists" value={t.artists.toLocaleString()} />
        <Tile label="Albums" value={t.albums.toLocaleString()} />
      </dl>
      <section className="stats-section stats-section--chart" aria-label="Listening over time">
        <BarChart points={o.series} bucket={o.bucket} label={`Minutes listened per ${o.bucket}, ${title}`} />
      </section>
      <section className="stats-section" aria-labelledby="stats-habits">
        <h2 id="stats-habits" className="stats-section__title">
          Listening habits
        </h2>
        <div className="stats-habits">
          <dl className="stats-tiles stats-tiles--small">
            <Tile
              label="Longest streak"
              value={o.streak ? `${o.streak.days} ${o.streak.days === 1 ? "day" : "days"}` : "None yet"}
              sub={o.streak && o.streak.days > 1 ? `${shortDate(o.streak.from)} to ${shortDate(o.streak.to)}` : undefined}
            />
            <Tile label="Days you listened" value={t.days.toLocaleString()} />
            <Tile label="Busiest hour" value={o.busiestHour != null ? hourLabel(o.busiestHour) : "None yet"} />
            <Tile label="Busiest day" value={o.busiestWeekday != null ? WEEKDAYS[o.busiestWeekday] : "None yet"} />
          </dl>
          <div className="stats-habits__hours">
            <HourChart hours={o.hours} />
            <div className="stats-chart__axis">
              <span>12 AM</span>
              <span>12 PM</span>
              <span>11 PM</span>
            </div>
          </div>
        </div>
      </section>
      <TopLists key={`${range.from}:${range.to}`} o={o} range={range} title={title} />
    </>
  );
}

/** A small card for one month: minutes, top artist and song, daily bars. */
function RecapCard({ month, onShare }: { month: string; onShare: (r: MonthRecap) => void }) {
  const { data: r, error } = useLoad(() => backend.statsMonthRecap(month), [month]);
  const fr = r ? barFractions(r.daily.map((d) => d.ms)) : [];
  const song = r?.topSongs[0];
  return (
    <li className="recap-card" data-testid="recap-card">
      <h3 className="recap-card__month">{monthName(month)}</h3>
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
          </p>
          <div className="recap-card__bars" aria-hidden="true">
            {fr.map((f, i) => (
              <span key={i} style={{ height: `${Math.max(r.daily[i].ms > 0 ? 6 : 0, f * 100)}%` }} />
            ))}
          </div>
          <dl className="recap-card__tops">
            <div>
              <dt>Top artist</dt>
              <dd className="truncate">{r.topArtists[0]?.name ?? "None"}</dd>
            </div>
            <div>
              <dt>Top song</dt>
              <dd className="truncate">
                {song ? song.title : "None"}
                {song?.unreleased && <span className="stats-mark">Unreleased</span>}
              </dd>
            </div>
          </dl>
          <Button size="sm" icon={<Share2 size={14} />} onClick={() => onShare(r)} aria-label={`Share ${monthName(month)}`}>
            Share
          </Button>
        </>
      )}
    </li>
  );
}

function MonthlySection({ span }: { span: StatsSpan }) {
  const [sharing, setSharing] = useState<MonthRecap | null>(null);
  const months = span.months.slice(0, 12);
  if (!months.length) return null;
  return (
    <section className="stats-section" aria-labelledby="stats-months">
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

export function StatsPage() {
  const on = useSettings((s) => s.settings?.statsPage !== false);
  const monthly = useSettings((s) => s.settings?.monthlyStatsOnProfile !== false);
  const compare = useSettings((s) => !!s.settings?.statsCompare);
  const [choice, setChoice] = useState<RangeChoice>({ kind: "4w" });
  const { data: span } = useLoad(() => backend.statsSpan(), []);
  const today = span?.today ?? null;
  const range = useMemo(() => (today ? toRange(choice, today) : null), [choice, today]);
  const title = rangeLabel(choice);
  const { data: o, error } = useLoad(() => (range ? backend.statsOverview(range, 50) : new Promise<StatsOverview>(() => {})), [range]);

  if (!on)
    return (
      <div className="page-pad stats" data-testid="stats">
        <h1 className="stats__title">Stats</h1>
        <div className="empty-state" role="status">
          <BarChart3 size={40} strokeWidth={1.5} aria-hidden="true" />
          <h2 className="empty-state__title">Stats are off</h2>
          <p className="empty-state__text">Turn on the Stats page in Settings to see your listening here.</p>
          <Button onClick={() => nav.go({ name: "settings" })}>Open Settings</Button>
        </div>
      </div>
    );

  const empty = span && !span.first;
  return (
    <div className="page-pad stats" data-testid="stats">
      <h1 className="stats__title">Stats</h1>
      <p className="stats__lede">Counted on this computer from everything you've played here and the history you imported.</p>
      {empty ? (
        <div className="empty-state" role="status">
          <BarChart3 size={40} strokeWidth={1.5} aria-hidden="true" />
          <h2 className="empty-state__title">Nothing to count yet</h2>
          <p className="empty-state__text">Play some music, or import your past plays in Settings, and your stats show up here.</p>
        </div>
      ) : (
        <>
          <RangePicker choice={choice} span={span} onChange={setChoice} />
          <h2 className="stats__range" aria-live="polite">
            {title}
            {o && <span className="stats__range-dates">{o.from === o.to ? shortDate(o.from) : `${shortDate(o.from)} to ${shortDate(o.to)}`}</span>}
          </h2>
          {error && !o ? (
            <p className="page-error">Couldn't count your stats: {error.message}</p>
          ) : !o || !range ? (
            <div aria-hidden="true" className="stats-loading">
              <div className="stats-tiles">
                {Array.from({ length: 5 }, (_, i) => (
                  <Skeleton key={i} height={72} width="100%" />
                ))}
              </div>
              <Skeleton height={160} width="100%" style={{ marginTop: 24 }} />
            </div>
          ) : (
            <Overview o={o} range={range} title={title} />
          )}
          <ShelfStatsSection />
          {monthly && span && <MonthlySection span={span} />}
          {compare && <CompareSection />}
        </>
      )}
    </div>
  );
}
