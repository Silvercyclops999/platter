/**
 * Radio and mixes on the phone: every mix made for you, by kind, two
 * across. Radio starts from any song, album or artist's sheet.
 */
import { useApp } from "../../stores/app";
import { useSettings } from "../../stores/settings";
import { MixArt, openMix } from "../../pages/MixCard";
import { useMixShelves } from "../../pages/mixShelves";
import type { MixSummary } from "../../lib/types";
import { PageFailed } from "./common";
import { Empty, GhostRows, GoPill, PageFrame, Tile, TileGrid } from "./kit";

function Group({ title, mixes }: { title: string; mixes: MixSummary[] }) {
  if (!mixes.length) return null;
  return (
    <section className="ph-x__section" aria-label={title}>
      <h2 className="ph-section-title">{title}</h2>
      <TileGrid label={title}>
        {mixes.map((m) => (
          <Tile key={m.id} title={m.kind === "daylist" ? "daylist" : m.title} sub={m.kind === "daylist" ? "Your day in a playlist" : m.description} art={<MixArt mix={m} size={320} named={false} />} onOpen={() => openMix(m)} />
        ))}
      </TileGrid>
    </section>
  );
}

export function PhoneMixes() {
  const on = useSettings((s) => s.settings?.homeMixes !== false);
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const { data, error, retry } = useMixShelves(on);
  const count = data ? data.madeForYou.length + data.moods.length + data.genres.length : 0;
  return (
    <PageFrame name="mixes" title="Radio and mixes" lede="Start radio from any song, album or artist with its menu. Mixes refresh as you listen.">
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
