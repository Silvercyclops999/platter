/**
 * On this day on the phone: what you played on today's date in earlier
 * years, one section a year with Play, songs not in the library listed but
 * skipped.
 */
import { Play } from "lucide-react";
import { Art } from "../../components/Art";
import { useOnThisDay, yearSummary, yearsAgo } from "../../pages/HistoryShelves";
import { player } from "../../stores/player";
import { useSettings } from "../../stores/settings";
import { toast } from "../../stores/toasts";
import { Empty, GhostRows, GoPill, PageFrame, Row } from "./kit";

function monthDay(date: string): string {
  const [y, m, d] = date.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString(undefined, { month: "long", day: "numeric" });
}

function playIds(ids: string[], first?: string) {
  if (!ids.length) return toast.info("None of these are in your library");
  void player.playTracks(ids, first ?? ids[0]);
}

export function PhoneOnThisDay() {
  const data = useOnThisDay();
  const on = useSettings((s) => s.settings?.homeOnThisDay !== false);
  const today = new Date();
  const date = data?.date ?? `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
  return (
    <PageFrame name="onThisDay" title="On this day" lede={`What you played on ${monthDay(date)} in earlier years.`}>
      {!on ? (
        <Empty title="On this day is off" text="Turn it on in Settings." action={<GoPill route={{ name: "settings", row: "homeOnThisDay" }}>Open Settings</GoPill>} />
      ) : !data ? (
        <GhostRows />
      ) : data.years.length === 0 ? (
        <Empty title="Nothing yet" text="Nothing was played on this date in earlier years yet." />
      ) : (
        data.years.map((y) => (
          <section key={y.year} className="ph-x__section" aria-label={String(y.year)}>
            <div className="ph-otd__head">
              <div>
                <h2 className="ph-section-title tabular">{y.year}</h2>
                <p className="ph-x__meta">
                  {yearsAgo(y.year, date)} · {yearSummary(y)}
                </p>
              </div>
              <button type="button" className="ph-play ph-play--small" aria-label={`Play ${y.year}`} disabled={!y.trackIds.length} onClick={() => playIds(y.trackIds)}>
                <Play size={24} fill="currentColor" aria-hidden="true" />
              </button>
            </div>
            <ul className="ph-rows">
              {y.songs.map((s) => (
                <Row
                  key={s.key}
                  className={s.trackId ? "" : "is-missing"}
                  art={<Art path={s.track?.thumb ?? null} size={48} kind="album" />}
                  title={s.title}
                  sub={`${s.artist} · ${s.plays === 1 ? "1 play" : `${s.plays} plays`}${s.trackId ? "" : " · Not in your library"}`}
                  onOpen={s.trackId ? () => playIds(y.trackIds, s.trackId!) : undefined}
                />
              ))}
            </ul>
          </section>
        ))
      )}
    </PageFrame>
  );
}
