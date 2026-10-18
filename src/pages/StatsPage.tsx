/**
 * Stats: your listening for a range of days. Range chips (4 weeks, 6
 * months, this year, a picked year or month, all time, custom dates), then
 * three tabs. Overview: the range in a sentence with two facts, top songs,
 * artists, albums and genres with Play these and Save as playlist, when you
 * listened (each chart led by a sentence) and moments. Recaps: the range's
 * monthly recaps you can share as a card, then Wrapped and the Top Songs
 * lists. Compare: comparing with a friend when that's on. Everything is
 * counted on this computer.
 */
import { RotateCw, UserRound } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { Button } from "../components/Button";
import { LikedCover } from "../components/LikedCover";
import { Select } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { ChipRow, Tabs } from "../components/Tabs";
import { backend } from "../lib/backend";
import { monthName, rangeLabel, toRange, type RangeChoice } from "../lib/stats";
import type { StatsMoments, StatsOverview, StatsSpan } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";
import { nav, useRouter, type StatsTab } from "../stores/router";
import { useSettings } from "../stores/settings";
import { WhenYouListened } from "./StatsCharts";
import { CompareSection } from "./StatsCompare";
import { WrappedSection } from "./StatsWrapped";
import { StatsHero, StatsHeroSkeleton } from "./StatsHero";
import { MomentsSection } from "./StatsMoments";
import { MonthlySection, monthsIn } from "./StatsRecaps";
import { TopLists } from "./StatsTopLists";
import "./stats.css";

export { BarChart } from "./StatsCharts";

type ChipId = RangeChoice["kind"];

const TABS: { id: StatsTab; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "recaps", label: "Recaps" },
  { id: "compare", label: "Compare" },
];
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
      return prev.kind === "custom" ? prev : ({ kind: "custom", ...toRange({ kind: "4w" }, today) } as RangeChoice);
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
          <button key={c.id} type="button" aria-pressed={choice.kind === c.id} className={`chip ${choice.kind === c.id ? "is-selected" : ""}`} onClick={() => onChange(choiceFor(c.id, span, choice))}>
            {c.label}
          </button>
        ))}
      </ChipRow>
      {choice.kind === "pickYear" && years.length > 0 && (
        <Select label="Year" value={String(choice.year)} options={years.map((y) => ({ value: String(y), label: String(y) }))} onChange={(v) => onChange({ kind: "pickYear", year: Number(v) })} />
      )}
      {choice.kind === "pickMonth" && months.length > 0 && (
        <Select label="Month" value={choice.month} options={months.map((m) => ({ value: m, label: monthName(m) }))} onChange={(v) => onChange({ kind: "pickMonth", month: v })} />
      )}
      {choice.kind === "custom" && (
        <span className="stats-range__dates">
          <label>
            <span className="sr-only">From</span>
            <input type="date" className="stats-date" value={choice.from} max={span?.today} onChange={(e) => e.target.value && onChange({ ...choice, from: e.target.value })} />
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

/** Off, empty and failed: the Stats tile, a heading, a line and maybe a button. */
function StatsState({ title, text, action, tone = "plain" }: { title: string; text: ReactNode; action?: ReactNode; tone?: "plain" | "error" }) {
  return (
    <div className={`stats-state stats-state--${tone}`} role="status" data-shot="state">
      <span className="stats-state__art" aria-hidden="true">
        <LikedCover icon="stats" size={88} />
      </span>
      <h2 className="stats-state__title">{title}</h2>
      <p className="stats-state__text">{text}</p>
      {action}
    </div>
  );
}

function LoadingBody() {
  return (
    <div aria-hidden="true" className="stats-loading" data-testid="stats-loading">
      <StatsHeroSkeleton />
      <div className="stats-section">
        <Skeleton width={220} height={28} />
        <div className="stats-top" style={{ marginTop: 20 }}>
          <Skeleton width="100%" height={360} radius="card" />
          <div>
            {Array.from({ length: 6 }, (_, i) => (
              <div key={i} className="stats-row">
                <Skeleton width={40} height={40} />
                <span className="stats-row__text">
                  <Skeleton width="45%" height={14} />
                  <Skeleton width="25%" height={12} style={{ marginTop: 6 }} />
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

/** The range the page opens on: a month when asked, else the last 4 weeks. */
function firstChoice(month: string | undefined): RangeChoice {
  return month && /^\d{4}-\d{2}$/.test(month) ? { kind: "pickMonth", month } : { kind: "4w" };
}

export function StatsPage({ tab: openTab, month, friend }: { tab?: StatsTab; month?: string; friend?: string } = {}) {
  const on = useSettings((s) => s.settings?.statsPage !== false);
  const monthly = useSettings((s) => s.settings?.monthlyStats !== false);
  const wrappedOn = useSettings((s) => s.settings?.wrappedTopSongs !== false);
  const compare = useSettings((s) => !!s.settings?.statsCompare);
  const [tab, setTab] = useState<StatsTab>(openTab ?? (friend ? "compare" : "overview"));
  const [choice, setChoice] = useState<RangeChoice>(() => firstChoice(month));
  const [attempt, setAttempt] = useState(0);
  const { data: span } = useLoad(() => backend.statsSpan(), [attempt]);
  const today = span?.today ?? null;
  const range = useMemo(() => (today ? toRange(choice, today) : null), [choice, today]);
  const title = rangeLabel(choice);
  const { data: o, error } = useLoad(() => (range ? backend.statsOverview(range, 50) : new Promise<StatsOverview>(() => {})), [range, attempt]);
  const moments = useLoad(() => (range ? backend.statsMoments(range) : new Promise<StatsMoments>(() => {})), [range, attempt]);
  const isAll = choice.kind === "all";
  const isYear = choice.kind === "year" || choice.kind === "pickYear";

  const pickTab = (t: StatsTab) => {
    setTab(t);
    // Back and Forward come back to the same tab.
    const r = useRouter.getState();
    if (r.entries[r.index]?.route.name === "stats") r.replace({ name: "stats", tab: t });
  };

  const profileLink = (
    <Button size="sm" variant="ghost" icon={<UserRound size={14} />} onClick={() => nav.go({ name: "profile" })}>
      Your profile
    </Button>
  );

  if (!on)
    return (
      <div className="page-pad stats" data-testid="stats">
        <h1 className="stats__title">Stats</h1>
        <StatsState title="Stats are off" text="Turn on the Stats page in Settings to see your listening here." action={<Button onClick={() => nav.go({ name: "settings" })}>Open Settings</Button>} />
      </div>
    );

  const empty = span && !span.first;
  const panel = (() => {
    if (tab === "compare")
      return compare ? (
        <CompareSection friend={friend} />
      ) : (
        <StatsState
          title="Compare is off"
          text="Turn on Compare stats with friends in Settings to see your taste overlap with a friend who shares theirs. Only a summary is shared, never your history."
          action={<Button onClick={() => nav.go({ name: "settings" })}>Open Settings</Button>}
        />
      );
    if (tab === "recaps") {
      if (!monthly && !wrappedOn)
        return <StatsState title="Recaps are off" text="Turn on monthly recaps or Wrapped in Settings to see them here." action={<Button onClick={() => nav.go({ name: "settings" })}>Open Settings</Button>} />;
      return (
        <>
          {monthly && span && <MonthlySection months={monthsIn(span, range)} />}
          <WrappedSection />
        </>
      );
    }
    if (error && !o)
      return (
        <StatsState
          tone="error"
          title="Couldn't count your stats"
          text={error.message}
          action={
            <Button icon={<RotateCw size={14} />} onClick={() => setAttempt((n) => n + 1)}>
              Try again
            </Button>
          }
        />
      );
    if (!o || !range) return <LoadingBody />;
    return (
      <>
        <StatsHero o={o} moments={moments.data} title={title} isAll={isAll} />
        <TopLists key={`${range.from}:${range.to}`} o={o} range={range} title={title} />
        <WhenYouListened o={o} title={title} today={span?.today ?? o.to} />
        <MomentsSection moments={moments.data} rangeTitle={title} isYear={isYear} year={o.to.slice(0, 4)} error={!!moments.error} />
      </>
    );
  })();

  return (
    <div className="page-pad stats" data-testid="stats">
      <header className="stats__header">
        <div className="stats__head-row">
          <h1 className="stats__title">Stats</h1>
          {profileLink}
        </div>
        <p className="stats__lede">Counted on this computer from everything you've played here and the history you imported.</p>
      </header>
      {empty ? (
        <StatsState
          title="Nothing to count yet"
          text="Play some music, or import your past plays in Settings, and your stats show up here."
          action={<Button onClick={() => nav.go({ name: "settings", page: "libraryImport" })}>Import past plays</Button>}
        />
      ) : (
        <>
          {tab === "compare" ? (
            <div className="stats-range">
              <p className="stats-range__note">Compare looks at your top artists and songs from the last six months.</p>
            </div>
          ) : (
            <RangePicker choice={choice} span={span} onChange={setChoice} />
          )}
          <Tabs label="Stats" items={TABS} value={tab} onChange={pickTab} idPrefix="stats-tabs" className="stats-tabs" />
          <div id="stats-tabs-panel" role="tabpanel" aria-labelledby={`stats-tabs-tab-${tab}`} className="stats-panel" tabIndex={0} data-testid={`stats-panel-${tab}`}>
            {tab !== "compare" && (
              <h2 className="sr-only" aria-live="polite">
                {title}
              </h2>
            )}
            {panel}
          </div>
        </>
      )}
    </div>
  );
}
