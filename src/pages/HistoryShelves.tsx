/**
 * Home's history shelves, from every play (this app's and imported):
 * "On this day" (what you played on today's date in earlier years, one card
 * per year, and a page listing each year's songs) and "Time travel" (a mix
 * per release decade, plus the year you listened most). The core works both
 * out at most once a day. Songs no longer in the library are listed but
 * never played.
 */
import { Play } from "lucide-react";
import { type CSSProperties, type ReactNode, useEffect, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { CardSkeleton } from "../components/Card";
import { Shelf, ShelfItem } from "../components/Shelf";
import { backend } from "../lib/backend";
import type { HistoryMix, OnThisDay, OnThisDayYear, TimeTravel } from "../lib/statsTypes";
import { useApp } from "../stores/app";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./mixes.css";
import "./history-shelves.css";

const HUES = ["#f2a65a", "#e57a9a", "#8fc1f0", "#9dd47a", "#c79bf2", "#f5d27a", "#7fd6d0", "#f29b8f"];

/** A colour per year, or per decade for a decade, the same every time. */
export function historyColour(year: number, decade = false): string {
  // Years start further along the list, so a year rarely matches its decade's colour.
  return HUES[Math.abs(decade ? Math.floor(year / 10) : year + 3) % HUES.length];
}

/** "1990s". */
export const decadeLabel = (year: number) => `${year}s`;

/** "Your year: 2021" or "1990s". */
export function mixTitle(m: HistoryMix): string {
  return m.kind === "year" ? `Your year: ${m.year}` : decadeLabel(m.year);
}

/** "3 years ago" from the year of today's date. */
export function yearsAgo(year: number, today: string): string {
  const n = Number(today.slice(0, 4)) - year;
  return n === 1 ? "1 year ago" : `${n} years ago`;
}

/** Play library songs, or say there are none. */
async function playIds(ids: string[], first?: string) {
  if (!ids.length) {
    toast.info("None of these are in your library");
    return;
  }
  try {
    await player.playTracks(ids, first ?? ids[0]);
  } catch {
    toast.error("Couldn't play these");
  }
}

const LOADERS = {
  onThisDay: () => backend.homeOnThisDay(),
  timeTravel: () => backend.homeTimeTravel(),
};

/** Today's local date, as "2026-10-17". */
function localDate(d = new Date()): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** Today's local date, moving on at midnight and when the window comes back after a while. */
function useToday(): string {
  const [today, setToday] = useState(localDate);
  useEffect(() => {
    const check = () => setToday(localDate());
    const now = new Date();
    const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1).getTime() - now.getTime();
    const t = setTimeout(check, midnight + 1000);
    document.addEventListener("visibilitychange", check);
    return () => {
      clearTimeout(t);
      document.removeEventListener("visibilitychange", check);
    };
  }, [today]);
  return today;
}

/** Load one of the shelves while its setting is on and the server is there; again as the library changes and each new day. */
function useHistory<K extends keyof typeof LOADERS>(kind: K, on: boolean): Awaited<ReturnType<(typeof LOADERS)[K]>> | null {
  type T = Awaited<ReturnType<(typeof LOADERS)[K]>>;
  const libraryVersion = useApp((s) => s.libraryVersion);
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const today = useToday();
  const [data, setData] = useState<T | null>(null);
  useEffect(() => {
    if (!on || cachedOnly) {
      setData(null);
      return;
    }
    let alive = true;
    (LOADERS[kind]() as Promise<T>).then(
      (d) => alive && setData(d),
      () => alive && setData(null),
    );
    return () => {
      alive = false;
    };
  }, [kind, on, cachedOnly, libraryVersion, today]);
  return on && data?.shown ? data : null;
}

export function useOnThisDay(): OnThisDay | null {
  const on = useSettings((s) => s.settings?.homeOnThisDay !== false);
  return useHistory("onThisDay", on);
}

export function useTimeTravel(): TimeTravel | null {
  const on = useSettings((s) => s.settings?.homeTimeTravel !== false);
  return useHistory("timeTravel", on);
}

/** The collage with a coloured strip naming the card. */
function HistoryArt({ covers, label, colour }: { covers: string[]; label: string; colour: string }) {
  return (
    <span className="mix-art history-art" style={{ "--mix-color": colour } as CSSProperties} aria-hidden="true">
      <span className="mix-art__collage">
        {Array.from({ length: 4 }, (_, i) => (
          <Art key={i} path={covers[i % Math.max(1, covers.length)] ?? null} fluid requestSize={100} kind="album" />
        ))}
      </span>
      <span className="mix-art__strip">
        <span className="mix-art__name">{label}</span>
      </span>
    </span>
  );
}

function HistoryCard({
  testId,
  art,
  title,
  subtitle,
  onPlay,
  onOpen,
}: {
  testId: string;
  art: ReactNode;
  title: string;
  subtitle: string;
  onPlay: () => void;
  /** Where the title goes; without one the title is plain text (the play button plays). */
  onOpen?: () => void;
}) {
  return (
    <div className="card card--mix" data-testid={testId}>
      <div className="card__art">
        {art}
        <button
          type="button"
          className="card__play"
          aria-label={`Play ${title}`}
          onClick={(e) => {
            e.stopPropagation();
            onPlay();
          }}
        >
          <Play size={20} fill="currentColor" aria-hidden="true" />
        </button>
      </div>
      {onOpen ? (
        <button type="button" className="card__link" onClick={onOpen} title={title}>
          <span className="card__title truncate">{title}</span>
        </button>
      ) : (
        <span className="card__link" title={title}>
          <span className="card__title truncate">{title}</span>
        </span>
      )}
      <div className="card__subtitle mix-card__subtitle">{subtitle}</div>
    </div>
  );
}

/** "14 songs, 2 not in your library". */
export function yearSummary(y: OnThisDayYear): string {
  const songs = y.songs.length === 1 ? "1 song" : `${y.songs.length} songs`;
  const missing = y.songs.length - y.trackIds.length;
  return missing > 0 ? `${songs}, ${missing} not in your library` : songs;
}

/** Home's On this day shelf. Hidden while off or when there's nothing. */
export function OnThisDayShelf() {
  const data = useOnThisDay();
  if (!data || data.years.length === 0) return null;
  return (
    <Shelf title="On this day" skeletonKind="album" onShowAll={() => nav.go({ name: "onThisDay" })}>
      {data.years.map((y) => (
        <ShelfItem key={y.year}>
          <HistoryCard
            testId={`on-this-day-${y.year}`}
            art={<HistoryArt covers={y.covers} label={String(y.year)} colour={historyColour(y.year)} />}
            title={yearsAgo(y.year, data.date)}
            subtitle={yearSummary(y)}
            onPlay={() => void playIds(y.trackIds)}
            onOpen={() => nav.go({ name: "onThisDay" })}
          />
        </ShelfItem>
      ))}
    </Shelf>
  );
}

/** Home's Time travel shelf: your year first, then the decades, oldest first. */
export function TimeTravelShelf() {
  const data = useTimeTravel();
  if (!data) return null;
  const mixes = [...(data.yourYear ? [data.yourYear] : []), ...data.decades];
  if (mixes.length === 0) return null;
  return (
    <Shelf title="Time travel" skeletonKind="album">
      {mixes.map((m) => {
        const title = mixTitle(m);
        const play = () => void playIds(m.trackIds);
        return (
          <ShelfItem key={m.id}>
            <HistoryCard
              testId={`time-travel-${m.id}`}
              art={
                <HistoryArt
                  covers={m.covers}
                  label={m.kind === "year" ? `Your year ${m.year}` : decadeLabel(m.year)}
                  colour={historyColour(m.year, m.kind === "decade")}
                />
              }
              title={title}
              subtitle={m.kind === "year" ? "Your most played from the year you listened most." : `Your most played from the ${decadeLabel(m.year)}.`}
              onPlay={play}
            />
          </ShelfItem>
        );
      })}
    </Shelf>
  );
}

/** "October 17". */
function monthDay(date: string): string {
  const [y, m, d] = date.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString(undefined, { month: "long", day: "numeric" });
}

function YearSection({ y, today }: { y: OnThisDayYear; today: string }) {
  const id = `otd-${y.year}`;
  return (
    <section className="otd-year" aria-labelledby={id}>
      <div className="otd-year__head">
        <h2 id={id} className="otd-year__title">
          {y.year}
        </h2>
        <span className="otd-year__sub">
          {yearsAgo(y.year, today)}, {y.plays === 1 ? "1 play" : `${y.plays.toLocaleString()} plays`}
        </span>
        <Button size="sm" variant="primary" icon={<Play size={14} fill="currentColor" />} disabled={!y.trackIds.length} onClick={() => void playIds(y.trackIds)}>
          Play
        </Button>
      </div>
      <ol className="otd-list">
        {y.songs.map((s, i) => {
          const inLibrary = !!s.trackId;
          return (
            <li key={s.key} className={`otd-row ${inLibrary ? "" : "is-missing"}`}>
              <span className="otd-row__n">{i + 1}</span>
              <Art path={s.track?.thumb ?? null} size={40} kind="album" />
              <span className="otd-row__text">
                {inLibrary ? (
                  <button type="button" className="otd-row__title truncate" onClick={() => void playIds(y.trackIds, s.trackId!)}>
                    {s.title}
                  </button>
                ) : (
                  <span className="otd-row__title truncate">{s.title}</span>
                )}
                <span className="otd-row__artist truncate">{s.artist}</span>
              </span>
              {!inLibrary && <span className="otd-row__missing">Not in your library</span>}
              <span className="otd-row__plays">{s.plays === 1 ? "1 play" : `${s.plays} plays`}</span>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

export function OnThisDayPage() {
  const data = useOnThisDay();
  const on = useSettings((s) => s.settings?.homeOnThisDay !== false);
  const today = useToday();
  const date = data?.date ?? today;
  return (
    <div className="page-pad otd-page">
      <h1 className="page-title">On this day</h1>
      <p className="otd-page__sub">What you played on {monthDay(date)} in earlier years. Songs that aren't in your library are listed but skipped.</p>
      {!on ? (
        <p className="otd-page__empty" role="status">
          On this day is off. Turn it on in Settings.
        </p>
      ) : !data ? (
        <div className="otd-page__loading" aria-hidden="true">
          <CardSkeleton kind="album" />
        </div>
      ) : data.years.length === 0 ? (
        <p className="otd-page__empty" role="status">
          Nothing played on this date in earlier years yet.
        </p>
      ) : (
        data.years.map((y) => <YearSection key={y.year} y={y} today={date} />)
      )}
    </div>
  );
}
