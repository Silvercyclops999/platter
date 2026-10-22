/**
 * Radio and mixes on the phone: the computer's page set for the phone.
 * Every mix made for you by kind, as ruled rows with its picture and length. Radio starts from any song, album or artist's sheet.
 */
import { useApp } from "../../stores/app";
import { useSettings } from "../../stores/settings";
import { openMix } from "../../pages/MixCard";
import { MixSleeve } from "../../pages/MixesPage";
import { duration } from "../../lib/statsStory";
import { useMixShelves } from "../../pages/mixShelves";
import type { MixSummary } from "../../lib/types";
import { PageFailed } from "./common";
import { Empty, GhostRows, GoPill, PageFrame, Row } from "./kit";

function Group({ title, mixes }: { title: string; mixes: MixSummary[] }) {
  if (!mixes.length) return null;
  return (
    <section className="ph-x__section mixes-page__group" aria-label={title}>
      <div className="lg-head">
        <h2 className="lg-head__title">{title}</h2>
        <span className="lg-head__aside">{mixes.length}</span>
      </div>
      <ul className="ph-rows ph-mixes__rows" aria-label={title}>
        {mixes.map((m) => (
          <Row
            key={m.id}
            className="ph-mixes__row"
            art={
              <span className="mix-row__sleeve">
                <MixSleeve mix={m} size={56} />
              </span>
            }
            title={m.kind === "daylist" ? "daylist" : m.title}
            sub={m.kind === "daylist" ? "Your day in a playlist" : m.description}
            end={m.durationMs > 0 ? <span className="mix-row__length">{duration(m.durationMs)}</span> : undefined}
            onOpen={() => openMix(m)}
          />
        ))}
      </ul>
    </section>
  );
}

export function PhoneMixes() {
  const on = useSettings((s) => s.settings?.homeMixes !== false);
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const { data, error, retry } = useMixShelves(on);
  const count = data ? data.madeForYou.length + data.moods.length + data.genres.length : 0;
  return (
    <PageFrame name="mixes" title="Radio and mixes" lede="Start radio from any song, album or artist with its menu. Mixes refresh as you listen." className="ph-mixes">
      {!on ? (
        <Empty title="Mixes are off" text="Turn on mixes in Settings and they show here." action={<GoPill route={{ name: "settings", row: "homeMixes" }}>Open Settings</GoPill>} />
      ) : error && !data ? (
        <PageFailed what="your mixes" retry={retry} />
      ) : !data ? (
        <GhostRows />
      ) : count === 0 ? (
        <Empty
          title={cachedOnly ? "No mixes on this phone" : "No mixes yet"}
          text={cachedOnly ? "Download a mix from its page and it plays here when Platter can't reach your server." : "Play a few albums and your first mixes appear here within a day."}
        />
      ) : (
        <>
          <Group title="Made for you" mixes={data.madeForYou} />
          <Group title="By mood" mixes={data.moods} />
          <Group title="By genre" mixes={data.genres} />
        </>
      )}
    </PageFrame>
  );
}
