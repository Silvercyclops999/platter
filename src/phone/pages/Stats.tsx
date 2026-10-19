/**
 * Stats on the phone: a range, the range told as the computer's Stats hero does,
 * then top artists, songs, records and genres, when you listened (each
 * chart led by its sentence), and Recaps and Compare as their own tabs.
 * The same counts as the computer's Stats page, from the same core calls.
 */
import { ListPlus, Play } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { failText } from "../../lib/errorText";
import { WEEKDAYS, barFractions, hourLabel, minutes, rangeLabel, toRange, type RangeChoice } from "../../lib/stats";
import type { StatsOverview } from "../../lib/statsTypes";
import { useLoad } from "../../lib/useLoad";
import { CompareSection } from "../../pages/StatsCompare";
import { MonthlySection, monthsIn } from "../../pages/StatsRecaps";
import { WrappedSection } from "../../pages/StatsWrapped";
import { StatsHero } from "../../pages/StatsHero";
import "../../pages/stats.css";
import { player } from "../../stores/player";
import { nav, useRouter, type StatsTab } from "../../stores/router";
import { useSettings } from "../../stores/settings";
import { toast } from "../../stores/toasts";
import { PageFailed, SongRow } from "./common";
import { Empty, GhostRows, GoPill, LinkList, PageFrame, Pills, Row, ShowMore, Tile, TileGrid } from "./kit";
import "../../pages/stats.css";

const NEVER = new Promise<never>(() => {});
const TABS: { id: StatsTab; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "recaps", label: "Recaps" },
  { id: "compare", label: "Compare" },
];

type ChipId = "4w" | "6m" | "year" | "all" | "month" | `y${number}`;

function choiceOf(id: Exclude<ChipId, "month">): RangeChoice {
  if (id.startsWith("y") && id !== "year") return { kind: "pickYear", year: Number(id.slice(1)) };
  return { kind: id as "4w" | "6m" | "year" | "all" };
}
function chipOf(c: RangeChoice): ChipId {
  if (c.kind === "pickYear") return `y${c.year}`;
  if (c.kind === "pickMonth") return "month";
  return c.kind === "custom" ? "4w" : c.kind;
}

/** "In the evening" from the busiest hour. */
export function partOfDay(h: number): string {
  if (h < 5) return "late at night";
  if (h < 12) return "in the morning";
  if (h < 17) return "in the afternoon";
  if (h < 21) return "in the evening";
  return "at night";
}

/** The sentence over the hours chart. */
export function hoursLine(o: Pick<StatsOverview, "busiestHour">): string {
  return o.busiestHour == null ? "No listening to chart yet." : `You listen most ${partOfDay(o.busiestHour)}, busiest around ${hourLabel(o.busiestHour)}.`;
}

/** The sentence over the weekdays chart. */
export function weekdaysLine(o: Pick<StatsOverview, "busiestWeekday">): string {
  return o.busiestWeekday == null ? "No listening to chart yet." : `${WEEKDAYS[o.busiestWeekday]} is your biggest day.`;
}

function Bars({ label, values, labels, mark }: { label: string; values: number[]; labels: string[]; mark: number | null }) {
  const f = barFractions(values);
  return (
    <div className="ph-bars-chart" role="img" aria-label={label}>
      <div className="ph-bars-chart__bars">
        {f.map((v, i) => (
          <span key={i} className={`ph-bars-chart__bar ${i === mark ? "is-mark" : ""}`} style={{ height: `${Math.max(2, Math.round(v * 100))}%` }} />
        ))}
      </div>
      <div className="ph-bars-chart__axis" aria-hidden="true">
        {labels.map((l, i) => (
          <span key={i}>{l}</span>
        ))}
      </div>
    </div>
  );
}

function Section({ title, children, note }: { title: string; children: ReactNode; note?: ReactNode }) {
  return (
    <section className="ph-x__section" aria-label={title}>
      <h2 className="ph-section-title">{title}</h2>
      {note && <p className="ph-section-sub">{note}</p>}
      {children}
    </section>
  );
}

function Overview({ o, title, isAll }: { o: StatsOverview; title: string; isAll: boolean }) {
  const moments = useLoad(() => backend.statsMoments({ from: isAll ? null : o.from, to: isAll ? null : o.to }), [o.from, o.to, isAll]);
  const [songsShown, setSongsShown] = useState(10);
  const [artistsShown, setArtistsShown] = useState(5);
  const songs = o.topSongs.filter((s) => s.track);
  const ids = songs.map((s) => s.track!.id);
  const save = async () => {
    try {
      const pl = await backend.playlistCreate(`Top songs: ${title}`, ids.slice(0, 50));
      toast.info(`Saved as a playlist: ${pl.title}`);
    } catch (e) {
      toast.error(failText("Couldn't save the playlist", e));
    }
  };
  const hourLabels = Array.from({ length: 24 }, (_, h) => (h % 6 === 0 ? hourLabel(h).replace(" ", "").toLowerCase() : ""));
  return (
    <>
      <StatsHero o={o} moments={moments.data} title={title} isAll={isAll} />

      {o.topArtists.length > 0 && (
        <Section title="Top artists">
          <ul className="ph-rows">
            {o.topArtists.slice(0, artistsShown).map((a, i) => (
              <Row
                key={a.key}
                art={<Art path={a.thumb} size={48} round kind="artist" monogram={a.name} />}
                title={`${i + 1}. ${a.name}`}
                sub={`${a.plays.toLocaleString()} plays · ${minutes(a.ms).toLocaleString()} min`}
                onOpen={a.artistId ? () => nav.go({ name: "artist", id: a.artistId! }) : undefined}
              />
            ))}
          </ul>
          <ShowMore total={o.topArtists.length} shown={artistsShown} noun="artists" onMore={() => setArtistsShown((n) => n + 10)} />
        </Section>
      )}

      {songs.length > 0 && (
        <Section title="Top songs">
          <div className="ph-x__pill-row">
            <button type="button" className="ph-pill is-primary" onClick={() => void player.playTracks(ids)}>
              <Play size={16} fill="currentColor" aria-hidden="true" /> Play these
            </button>
            <button type="button" className="ph-pill" onClick={() => void save()}>
              <ListPlus size={16} aria-hidden="true" /> Save as playlist
            </button>
          </div>
          <ul className="ph-songs">
            {songs.slice(0, songsShown).map((s, i) => (
              <SongRow key={s.key} track={s.track!} index={i + 1} sub={`${s.artist} · ${s.plays.toLocaleString()} plays`} onPlay={() => void player.playTracks(ids, s.track!.id)} />
            ))}
          </ul>
          <ShowMore total={songs.length} shown={songsShown} noun="songs" onMore={() => setSongsShown((n) => n + 20)} />
        </Section>
      )}

      {o.topAlbums.length > 0 && (
        <Section title="Top records">
          <TileGrid label="Top records">
            {o.topAlbums.slice(0, 6).map((a) => (
              <Tile key={a.key} title={a.title} sub={`${a.artist} · ${a.plays.toLocaleString()} plays`} sleeveArtist={a.artist} thumb={a.thumb} onOpen={a.albumId ? () => nav.go({ name: "album", id: a.albumId! }) : undefined} />
            ))}
          </TileGrid>
        </Section>
      )}

      {o.topGenres.length > 0 && (
        <Section title="Top genres">
          <ol className="ph-genres">
            {o.topGenres.slice(0, 8).map((g, i) => (
              <li key={g.name} className="ph-genres__row">
                <span className="ph-genres__name">
                  {i + 1}. {g.name}
                </span>
                <span className="ph-genres__bar" style={{ width: `${Math.round(barFractions(o.topGenres.slice(0, 8).map((x) => x.ms))[i] * 100)}%` }} />
                <span className="ph-x__count">{g.plays.toLocaleString()} plays</span>
              </li>
            ))}
          </ol>
        </Section>
      )}

      <Section title="When you listened" note={hoursLine(o)}>
        <Bars label={`Listening by hour. ${hoursLine(o)}`} values={o.hours} labels={hourLabels} mark={o.busiestHour} />
        <p className="ph-section-sub ph-stats__second">{weekdaysLine(o)}</p>
        <Bars label={`Listening by weekday. ${weekdaysLine(o)}`} values={o.weekdays} labels={WEEKDAYS.map((d) => d.slice(0, 1))} mark={o.busiestWeekday} />
      </Section>
    </>
  );
}

export function PhoneStats({ tab: openTab, month, friend }: { tab?: StatsTab; month?: string; friend?: string }) {
  const on = useSettings((s) => s.settings?.statsPage !== false);
  const monthly = useSettings((s) => s.settings?.monthlyStats !== false);
  const compareOn = useSettings((s) => !!s.settings?.statsCompare);
  const wrappedOn = useSettings((s) => s.settings?.wrappedTopSongs !== false);
  const [tab, setTab] = useState<StatsTab>(openTab ?? (friend ? "compare" : "overview"));
  const [choice, setChoice] = useState<RangeChoice>(month && /^\d{4}-\d{2}$/.test(month) ? { kind: "pickMonth", month } : { kind: "4w" });
  const span = useLoad(() => (on ? backend.statsSpan() : NEVER), [on]);
  const today = span.data?.today ?? null;
  const range = useMemo(() => (today ? toRange(choice, today) : null), [choice, today]);
  const title = rangeLabel(choice);
  const o = useLoad(() => (range && on ? backend.statsOverview(range, 50) : NEVER), [range, on]);
  const years = useMemo(() => [...new Set((span.data?.months ?? []).map((m) => Number(m.slice(0, 4))))], [span.data]);
  const chips: { id: ChipId; label: string }[] = [
    { id: "4w", label: "4 weeks" },
    { id: "6m", label: "6 months" },
    { id: "year", label: "This year" },
    ...years.slice(1).map((y) => ({ id: `y${y}` as ChipId, label: String(y) })),
    { id: "all", label: "All time" },
  ];
  // Opened on one month (from a profile's "Show all"): that month leads the pills.
  if (choice.kind === "pickMonth") chips.unshift({ id: "month", label: title });
  const pickTab = (t: StatsTab) => {
    setTab(t);
    const r = useRouter.getState();
    if (r.entries[r.index]?.route.name === "stats") r.replace({ name: "stats", tab: t });
  };
  if (!on)
    return (
      <PageFrame name="stats" title="Stats">
        <Empty title="Stats are off" text="Turn on the Stats page in Settings to see your listening here." action={<GoPill route={{ name: "settings", row: "statsPage" }}>Open Settings</GoPill>} />
      </PageFrame>
    );
  const empty = span.data && !span.data.first;
  return (
    <PageFrame name="stats" title="Stats" lede="Counted from everything you've played and the history you imported." className="ph-stats">
      {empty ? (
        <Empty title="Nothing to count yet" text="Play some music, or import your past plays in Settings, and your stats show up here." action={<GoPill route={{ name: "settings", page: "libraryImport" }}>Import past plays</GoPill>} />
      ) : (
        <>
          <div className="ph-seg" role="tablist" aria-label="Stats">
            {TABS.map((t) => (
              <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} className={`ph-seg__tab ${tab === t.id ? "is-on" : ""}`} onClick={() => pickTab(t.id)}>
                {t.label}
              </button>
            ))}
          </div>
          {tab !== "compare" && <Pills label="Range" items={chips} value={chipOf(choice)} onChange={(id) => id !== "month" && setChoice(choiceOf(id))} />}
          {tab === "overview" &&
            (o.error && !o.data ? (
              <PageFailed what="your stats" retry={o.retry} />
            ) : !o.data ? (
              <GhostRows />
            ) : (
              <>
                <h2 className="sr-only ph-stats__range">{title}</h2>
                <Overview key={`${range?.from}:${range?.to}`} o={o.data} title={title} isAll={!range?.from && !range?.to} />
              </>
            ))}
          {tab === "recaps" && (
            <div className="ph-wrap-in">
              {!monthly && !wrappedOn ? (
                <Empty title="Recaps are off" text="Turn on monthly recaps or Wrapped in Settings to see them here." />
              ) : (
                <>
                  {monthly && span.data && <MonthlySection months={monthsIn(span.data, range)} />}
                  <WrappedSection />
                </>
              )}
            </div>
          )}
          {tab === "compare" && (
            <div className="ph-wrap-in">
              {compareOn ? (
                <CompareSection friend={friend} />
              ) : (
                <Empty
                  title="Compare is off"
                  text="Turn on Compare stats with friends in Settings to see your taste overlap with a friend who shares theirs. Only a summary is shared, never your history."
                  action={<GoPill route={{ name: "settings", row: "statsCompare" }}>Open Settings</GoPill>}
                />
              )}
            </div>
          )}
          <LinkList
            label="More"
            items={[
              ...(wrappedOn ? [{ title: "Your Top Songs", sub: "Each year's hundred, and all time", route: { name: "topSongs", ...(years[0] ? { year: years[0] } : {}) } as const }] : []),
              { title: "Listening diary", route: { name: "diary" } },
              { title: "Recents", route: { name: "recents" } },
            ]}
          />
        </>
      )}
    </PageFrame>
  );
}
