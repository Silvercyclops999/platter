/**
 * The listening diary: albums played start to finish, as dated entries in
 * month sections (the sleeve, title, artist, rating and the start of the
 * note), or a year of covers by month. Filters for rated entries, entries
 * with notes and a year; search looks in notes, titles and artists. Each
 * entry opens an editor for its rating, note and date. Export saves the
 * entries shown as text or CSV. Kept on this computer unless the diary's
 * sync setting is on.
 */
import { ChevronLeft, ChevronRight, Download, Pencil } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { SearchField } from "../components/SearchField";
import { Skeleton } from "../components/Skeleton";
import { FilterChips, Tabs } from "../components/Tabs";
import { loadNoteFace } from "../design/looks/fonts";
import { backend } from "../lib/backend";
import { MONTHS, WEEKDAYS, byMonth, diaryYears, monthLabel, playedLine, useDiaryOn, useDiaryVersion, whereLine, yearRange } from "../lib/diary";
import { errorMessage } from "../lib/errorText";
import { plural } from "../lib/format";
import { useLoad } from "../lib/useLoad";
import type { DiaryItem, DiaryList, DiaryQuery } from "../lib/types";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { DiaryEntryDialog } from "./DiaryEntryDialog";
import { StarDisplay } from "./StarRating";
import "./ledger.css";
import "./diary.css";

type View = "entries" | "year";
type Filter = "all" | "rated" | "notes";

const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "rated", label: "Rated" },
  { id: "notes", label: "With notes" },
];

/** Search waits for typing to pause. */
function useSettled(value: string, ms = 200): string {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

function Entry({ e, onEdit }: { e: DiaryItem; onEdit: () => void }) {
  const d = new Date(e.at);
  const where = whereLine(e);
  return (
    <li className="diary-entry">
      <time className="diary-entry__date" dateTime={new Date(e.at).toISOString()}>
        <span className="diary-entry__day">{d.getDate()}</span>
        <span className="diary-entry__weekday">{WEEKDAYS[d.getDay()]}</span>
      </time>
      {/* The title is the album's link for keyboards and screen readers; the
          sleeve is the same link for the mouse, so it isn't a second stop. */}
      <button
        type="button"
        className="diary-entry__sleeve"
        tabIndex={-1}
        aria-hidden="true"
        disabled={!e.inLibrary}
        onClick={() => nav.go({ name: "album", id: e.albumId })}
      >
        <span className="diary-entry__disc" aria-hidden="true" />
        <Art path={e.thumb} size={72} alt="" />
      </button>
      <div className="diary-entry__body">
        <div className="diary-entry__head">
          <h3 className="diary-entry__title">
            {e.inLibrary ? (
              <button type="button" className="diary-entry__link" onClick={() => nav.go({ name: "album", id: e.albumId })}>
                {e.title}
              </button>
            ) : (
              e.title
            )}
          </h3>
          {e.rating != null && <StarDisplay value={e.rating} size={14} />}
        </div>
        <p className="diary-entry__artist">
          {e.artist}
          {e.year ? <span className="diary-entry__year"> · {e.year}</span> : null}
        </p>
        <p className="diary-entry__meta">
          {playedLine(e)}
          {where && <span> · {where}</span>}
        </p>
        {e.note && <p className="diary-note diary-entry__note">{e.note}</p>}
      </div>
      <IconButton className="diary-entry__edit" label={`Edit the entry for ${e.title}`} size="md" icon={<Pencil size={16} />} onClick={onEdit} />
    </li>
  );
}

function Months({ list, onEdit }: { list: DiaryList; onEdit: (e: DiaryItem) => void }) {
  const months = useMemo(() => byMonth(list.entries), [list.entries]);
  return (
    <div className="diary-months">
      {months.map((m) => (
        <section key={m.key} className="diary-month" aria-labelledby={`diary-${m.key}`} data-month={m.key}>
          <header className="diary-month__head">
            <h2 id={`diary-${m.key}`} className="diary-month__title">
              {monthLabel(m.key)}
            </h2>
            <span className="diary-month__count">{plural(m.entries.length, "album")}</span>
          </header>
          <ol className="diary-month__list">
            {m.entries.map((e) => (
              <Entry key={e.id} e={e} onEdit={() => onEdit(e)} />
            ))}
          </ol>
        </section>
      ))}
    </div>
  );
}

/** Up to this many covers in a month's cell. */
const CELL_COVERS = 9;

function YearView({ year, list, onMonth }: { year: number; list: DiaryList; onMonth: (key: string) => void }) {
  const byKey = useMemo(() => new Map(byMonth(list.entries).map((m) => [m.key, m.entries])), [list.entries]);
  const total = list.entries.length;
  return (
    <div className="diary-year">
      <p className="diary-year__sum">{total === 0 ? `Nothing logged in ${year}.` : `${plural(total, "album")} in ${year}.`}</p>
      <ol className="diary-year__grid" aria-label={`${year} by month`}>
        {MONTHS.map((name, i) => {
          const key = `${year}-${String(i + 1).padStart(2, "0")}`;
          const entries = byKey.get(key) ?? [];
          return (
            <li key={key} className={`diary-cell ${entries.length ? "" : "is-empty"}`}>
              <button
                type="button"
                className="diary-cell__head"
                disabled={!entries.length}
                aria-label={`${name}: ${plural(entries.length, "album")}`}
                onClick={() => onMonth(key)}
              >
                <span className="diary-cell__month">{name}</span>
                <span className="diary-cell__count">{entries.length || ""}</span>
              </button>
              {entries.length > 0 && (
                <ul className="diary-cell__covers" aria-label={`${name} albums`}>
                  {entries.slice(0, CELL_COVERS).map((e) => (
                    <li key={e.id}>
                      <button
                        type="button"
                        className="diary-cell__cover"
                        aria-label={`${e.title} by ${e.artist}`}
                        title={`${e.title} · ${e.artist}`}
                        disabled={!e.inLibrary}
                        onClick={() => nav.go({ name: "album", id: e.albumId })}
                      >
                        <Art path={e.thumb} size={56} alt="" />
                      </button>
                    </li>
                  ))}
                  {entries.length > CELL_COVERS && <li className="diary-cell__more">+{entries.length - CELL_COVERS}</li>}
                </ul>
              )}
            </li>
          );
        })}
      </ol>
    </div>
  );
}

/**
 * The diary's sentence: how many records since when, the latest one, and
 * how you rated them. Only for the whole diary (no filter or search), when
 * every entry is in hand.
 */
/** The entry finished or logged last, whatever order the list came in (an edited date can move one). */
export function latestEntry(entries: DiaryItem[]): DiaryItem | null {
  return entries.reduce<DiaryItem | null>((best, e) => (!best || e.at > best.at ? e : best), null);
}

function DiaryLead({ list }: { list: DiaryList }) {
  const latest = latestEntry(list.entries)!;
  const since = list.firstAt != null ? new Date(list.firstAt) : null;
  const rated = list.entries.filter((e) => e.rating != null);
  const high = rated.filter((e) => (e.rating ?? 0) >= 4).length;
  const at = new Date(latest.at);
  return (
    <p className="lg-lead lg-lead--sm diary-head__lead" data-testid="diary-lead">
      <strong>{plural(list.total, "record", "records")}</strong> in your diary{since && ` since ${MONTHS[since.getMonth()]} ${since.getFullYear()}`}. The latest is <strong>{latest.title}</strong>, on{" "}
      {MONTHS[at.getMonth()].slice(0, 3)} {at.getDate()}.
      {high > 0 && (high === rated.length ? (high === 1 ? " The one you rated got four stars or more." : ` All ${high} you rated got four stars or more.`) : ` ${high} of the ${rated.length} you rated got four stars or more.`)}
    </p>
  );
}

/** `embedded`: shown as a tab inside Stats, under its heading. */
export function DiaryPage({ edit, embedded = false }: { edit?: string; embedded?: boolean }) {
  const Title = embedded ? "h2" : "h1";
  const frame = embedded ? "diary-page diary-page--embedded" : "page-pad diary-page";
  const on = useDiaryOn();
  const synced = useSettings((s) => !!s.settings?.syncDiary);
  const version = useDiaryVersion();
  const [view, setView] = useState<View>("entries");
  const [filter, setFilter] = useState<Filter>("all");
  const [year, setYear] = useState<number | null>(null);
  const [text, setText] = useState("");
  const words = useSettled(text);
  // The year view's year: the newest with entries until one is picked.
  const yearPicked = useRef(false);
  const [shownYear, setShownYearState] = useState<number>(() => new Date().getFullYear());
  const setShownYear = (y: number | ((was: number) => number)) => {
    yearPicked.current = true;
    setShownYearState(y);
  };
  const [editing, setEditing] = useState<DiaryItem | null>(null);
  const wantEdit = useRef(edit);
  const scrollTo = useRef<string | null>(null);

  useEffect(() => {
    void loadNoteFace();
  }, []);

  const base: DiaryQuery = { rated: filter === "rated" || undefined, withNotes: filter === "notes" || undefined, text: words.trim() || undefined };
  const query: DiaryQuery = view === "year" ? { ...base, ...yearRange(shownYear) } : { ...base, ...(year != null ? yearRange(year) : {}) };
  const loaded = useLoad(() => (on ? backend.diaryEntries(query) : Promise.resolve(null)), [on, query, version]);
  const list = loaded.data ?? null;
  const years = diaryYears(list?.firstAt ?? null, list?.lastAt ?? null);

  // The year view starts on the newest year with entries. `firstAt` and
  // `lastAt` describe the whole diary whatever the filters, so this is the
  // newest year of all, not of what the filters leave. An entry moved a day
  // ahead over New Year never starts it in a year still to come.
  const thisYear = new Date().getFullYear();
  const newestYear = years.length ? Math.min(years[0], thisYear) : null;
  useEffect(() => {
    if (newestYear == null || yearPicked.current) return;
    yearPicked.current = true;
    setShownYearState(newestYear);
  }, [newestYear]);

  // Opened to edit one entry (Log this album's toast).
  useEffect(() => {
    if (!wantEdit.current || !list) return;
    const e = list.entries.find((x) => x.id === wantEdit.current);
    wantEdit.current = undefined;
    if (e) setEditing(e);
  }, [list]);

  // Back from the year view to one month.
  useEffect(() => {
    if (view !== "entries" || !scrollTo.current || !list) return;
    const el = document.querySelector(`[data-month="${scrollTo.current}"]`);
    scrollTo.current = null;
    el?.scrollIntoView?.({ block: "start" });
  }, [view, list]);

  const exportAs = async (format: "text" | "csv") => {
    try {
      const name = await backend.diaryExport(format, view === "year" ? query : { ...base, ...(year != null ? yearRange(year) : {}) });
      if (name) toast.info(`Saved ${name}`);
    } catch (e) {
      toast.error(errorMessage(e));
    }
  };

  if (!on) {
    return (
      <div className={frame}>
        <Title className="page-title">Listening diary</Title>
        <div className="empty-state" role="status">
          <h2 className="empty-state__title">The listening diary is off</h2>
          <p className="empty-state__text">Turn it on to keep a private journal of the albums you play start to finish.</p>
          <Button variant="primary" onClick={() => void updateSettings({ keepDiary: true })}>
            Keep a listening diary
          </Button>
        </div>
      </div>
    );
  }

  const filtered = filter !== "all" || year != null || !!words.trim();
  const since = list?.firstAt != null ? new Date(list.firstAt) : null;
  const where = synced ? "Synced to your devices." : "Only on this computer.";
  // With the sentence above it, the line under it only says where the diary lives.
  const led = !!list && list.total > 0 && !filtered && list.entries.length === list.total;
  const lede =
    list == null
      ? " "
      : led
        ? where
        : list.total === 0
        ? "Albums you play start to finish are written here on their own."
        : since == null
          ? `${plural(list.total, "entry", "entries")}. ${where}`
          : `${plural(list.total, "entry", "entries")} since ${MONTHS[since.getMonth()]} ${since.getFullYear()}. ${where}`;

  return (
    <div className={frame}>
      <header className="diary-head">
        <p className="lg-eyebrow diary-head__eyebrow">Private</p>
        <Title className="page-title diary-head__title">Listening diary</Title>
        {led && <DiaryLead list={list} />}
        <p className="diary-head__lede">{lede}</p>
      </header>

      <div className="diary-bar">
        <Tabs<View>
          label="Diary view"
          idPrefix="diary-view"
          className="diary-bar__views"
          value={view}
          onChange={setView}
          items={[
            { id: "entries", label: "Entries" },
            { id: "year", label: "Year" },
          ]}
        />
        <FilterChips<Filter> label="Show" className="diary-bar__filters" items={FILTERS} value={filter} onChange={setFilter} />
        {view === "entries" ? (
          years.length > 0 && (
            <label className="diary-bar__year">
              <span className="sr-only">Year</span>
              <select aria-label="Year" value={year ?? ""} onChange={(e) => setYear(e.target.value ? Number(e.target.value) : null)}>
                <option value="">All years</option>
                {years.map((y) => (
                  <option key={y} value={y}>
                    {y}
                  </option>
                ))}
              </select>
            </label>
          )
        ) : (
          <div className="diary-bar__step" role="group" aria-label="Year">
            <IconButton label="Previous year" size="sm" icon={<ChevronLeft size={16} />} onClick={() => setShownYear((y) => y - 1)} />
            <span className="diary-bar__step-year" aria-live="polite">
              {shownYear}
            </span>
            <IconButton
              label="Next year"
              size="sm"
              icon={<ChevronRight size={16} />}
              disabled={shownYear >= thisYear}
              onClick={() => setShownYear((y) => y + 1)}
            />
          </div>
        )}
        <SearchField className="diary-bar__search" value={text} onChange={setText} label="Search your notes" placeholder="Search your notes" />
        <Button
          size="sm"
          variant="secondary"
          icon={<Download size={16} />}
          disabled={!list || list.total === 0}
          aria-haspopup="menu"
          onClick={(e) =>
            openContextMenu(
              "Export",
              [
                { label: "Export as text", onSelect: () => void exportAs("text") },
                { label: "Export as CSV", onSelect: () => void exportAs("csv") },
              ],
              e.currentTarget,
            )
          }
        >
          Export
        </Button>
      </div>

      {!list ? (
        loaded.error ? (
          <p className="diary-empty" role="alert">
            {errorMessage(loaded.error)}
          </p>
        ) : (
          <div aria-hidden="true" className="diary-skeleton">
            {[0, 1, 2].map((i) => (
              <Skeleton key={i} width="100%" height={96} radius="card" />
            ))}
          </div>
        )
      ) : view === "year" ? (
        <YearView
          year={shownYear}
          list={list}
          onMonth={(key) => {
            scrollTo.current = key;
            setYear(Number(key.slice(0, 4)));
            setView("entries");
          }}
        />
      ) : list.entries.length === 0 ? (
        <div className="empty-state diary-empty" role="status">
          <h2 className="empty-state__title">{filtered ? "No entries match" : "No entries yet"}</h2>
          <p className="empty-state__text">
            {filtered
              ? "Try another filter or search."
              : "Play an album from start to finish and it lands here, or log one from an album's menu with Log this album."}
          </p>
        </div>
      ) : (
        <Months list={list} onEdit={setEditing} />
      )}

      {editing && (
        <DiaryEntryDialog
          entry={editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            loaded.retry();
          }}
          onDeleted={() => {
            setEditing(null);
            loaded.retry();
          }}
        />
      )}
      {list && list.total > 0 && <p className="diary-foot">Your diary is never shared or scrobbled.</p>}
    </div>
  );
}
