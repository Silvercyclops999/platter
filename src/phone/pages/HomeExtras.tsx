/**
 * Home's other ways in on the phone: the mixes made for you as a shelf, and
 * a short list of the places Home can't hold (what's new, the release
 * radar, new artists, on this day, the archive, your shelf, the sound map).
 * Each shows only while its feature is on.
 */
import { Archive, CalendarClock, Disc3, Map as MapIcon, Radar, Radio, Sparkles, UserPlus } from "lucide-react";
import type { ReactNode } from "react";
import { MixArt, openMix } from "../../pages/MixCard";
import { useMixShelves } from "../../pages/mixShelves";
import { useShowPhysical } from "../../stores/physical";
import { nav, type Route } from "../../stores/router";
import { useSettings } from "../../stores/settings";
import { useSoundMapOn } from "../../stores/soundmap";
import { tap } from "../bits";
import { Carousel } from "./common";

export function HomeMixes() {
  const on = useSettings((s) => s.settings?.homeMixes !== false);
  const { data } = useMixShelves(on);
  const list = data ? [...data.madeForYou, ...data.moods, ...data.genres] : [];
  if (!list.length) return null;
  return (
    <Carousel title="Made for you" onAll={() => nav.go({ name: "mixes" })}>
      {list.slice(0, 12).map((m) => (
        <button key={m.id} type="button" className="ph-card-tile ph-mix-tile" onClick={() => openMix(m)}>
          <span className="ph-mix-tile__art">
            <MixArt mix={m} size={272} />
          </span>
          <span className="ph-card-tile__title">{m.kind === "daylist" ? "daylist" : m.title}</span>
          <span className="ph-card-tile__sub">{m.kind === "daylist" ? "Your day in a playlist" : m.description}</span>
        </button>
      ))}
    </Carousel>
  );
}

interface Way {
  title: string;
  icon: ReactNode;
  route: Route;
}

export function HomeExplore() {
  const s = useSettings((x) => x.settings);
  const shelf = useShowPhysical();
  const map = useSoundMapOn();
  const ways: Way[] = [
    { title: "What's new", icon: <Sparkles size={20} />, route: { name: "whatsNew" } },
    ...(s?.homeMixes !== false ? [{ title: "Radio and mixes", icon: <Radio size={20} />, route: { name: "mixes" } as Route }] : []),
    ...(s?.releaseRadar !== false ? [{ title: "Release radar", icon: <Radar size={20} />, route: { name: "releaseRadar" } as Route }] : []),
    ...(s?.homeNewToYou !== false ? [{ title: "New to you", icon: <UserPlus size={20} />, route: { name: "newToYou" } as Route }] : []),
    ...(s?.homeOnThisDay !== false ? [{ title: "On this day", icon: <CalendarClock size={20} />, route: { name: "onThisDay" } as Route }] : []),
    { title: "Archive", icon: <Archive size={20} />, route: { name: "archive" } },
    ...(shelf ? [{ title: "Your shelf", icon: <Disc3 size={20} />, route: { name: "shelf" } as Route }] : []),
    ...(map ? [{ title: "Sound map", icon: <MapIcon size={20} />, route: { name: "soundMap" } as Route }] : []),
  ];
  return (
    <section className="ph-shelf" aria-label="Explore">
      <h2 className="ph-shelf__title">Explore</h2>
      <ul className="ph-explore">
        {ways.map((w) => (
          <li key={w.title}>
            <button
              type="button"
              className="ph-explore__tile"
              onClick={() => {
                tap();
                nav.go(w.route);
              }}
            >
              <span className="ph-explore__icon" aria-hidden="true">
                {w.icon}
              </span>
              <span className="ph-explore__title">{w.title}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
