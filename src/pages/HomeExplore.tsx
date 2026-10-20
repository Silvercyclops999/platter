/**
 * Explore at the foot of Home: the ways into the library that aren't shelves
 * (Crates, Sound map). Each can be pinned to Your Library.
 */
import { Pin, PinOff } from "lucide-react";
import { useId } from "react";
import { IconButton } from "../components/Button";
import { LikedCover } from "../components/LikedCover";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { PINNABLE, usePinnedPages, type PinnablePage } from "../stores/pinnedPages";
import { useSoundMapEntry } from "../stores/soundmap";
import "./home-explore.css";

export function HomeExplore() {
  const id = useId();
  const cratesOn = useSettings((s) => s.settings?.showCrates !== false);
  const soundMapOn = useSoundMapEntry();
  const pinned = usePinnedPages((s) => s.pages);
  const toggle = usePinnedPages((s) => s.toggle);
  const pages: PinnablePage[] = [...(cratesOn ? (["crates"] as const) : []), ...(soundMapOn ? (["soundMap"] as const) : [])];
  if (!pages.length) return null;
  return (
    <section className="shelf home-explore" aria-labelledby={id}>
      <div className="shelf__head">
        <h2 id={id} className="shelf__title">
          Explore
        </h2>
      </div>
      <ul className="home-explore__list">
        {pages.map((p) => {
          const on = pinned.includes(p);
          const { title, sub } = PINNABLE[p];
          return (
            <li key={p} className="home-explore__tile">
              <LikedCover size={64} icon={p} className="home-explore__icon" />
              <button type="button" className="home-explore__open" onClick={() => nav.go({ name: p })}>
                <span className="home-explore__title">{title}</span>
                <span className="home-explore__sub">{sub}</span>
              </button>
              <IconButton
                size="sm"
                label={on ? `Unpin ${title} from Your Library` : `Pin ${title} to Your Library`}
                icon={on ? <PinOff size={16} /> : <Pin size={16} />}
                onClick={() => toggle(p)}
              />
            </li>
          );
        })}
      </ul>
    </section>
  );
}
