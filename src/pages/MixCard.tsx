import { Pause, Play } from "lucide-react";
import { type CSSProperties, useState } from "react";
import { Art } from "../components/Art";
import { artUrl } from "../lib/backend";
import type { MixSummary } from "../lib/types";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { useContextState } from "./cards";
import "./mixes.css";

/** Daylist colours by the part of the day (warm dawn to deep night). */
export function daylistGradient(hour: number): [string, string] {
  if (hour < 5) return ["#3b2d6b", "#c2588f"];
  if (hour < 9) return ["#f2a77a", "#8fc1f0"];
  if (hour < 12) return ["#f5d27a", "#7fd6d0"];
  if (hour < 17) return ["#f5b971", "#e57a9a"];
  if (hour < 21) return ["#9d6ad6", "#f29b8f"];
  return ["#2f3f8f", "#b56ad6"];
}

/** Where a mix opens. */
export function openMix(mix: Pick<MixSummary, "id" | "kind">) {
  if (mix.kind === "radio") nav.go({ name: "radio", seed: mix.id });
  else nav.go({ name: "mix", id: mix.id });
}

/**
 * A mix's art: the leading artist's photo, toned, with a coloured strip
 * holding the mix's name (a cover collage when there's no photo). The
 * daylist is a soft gradient instead.
 */
export function MixArt({ mix, size = 200 }: { mix: MixSummary; size?: number }) {
  // A photo that fails to load falls back to the cover collage.
  const [failedPhoto, setFailedPhoto] = useState<string | null>(null);
  if (mix.kind === "daylist") {
    const [a, b] = daylistGradient(new Date().getHours());
    return (
      <span className="mix-art mix-art--daylist" style={{ "--mix-a": a, "--mix-b": b } as CSSProperties} aria-hidden="true">
        <span className="mix-art__daylist">daylist</span>
      </span>
    );
  }
  const photo = mix.kind === "topSongs" || mix.art === failedPhoto ? null : mix.art;
  return (
    <span className="mix-art" style={{ "--mix-color": mix.color } as CSSProperties} aria-hidden="true">
      {photo ? (
        <img
          className="mix-art__photo"
          src={artUrl(photo, size)}
          alt=""
          loading="lazy"
          draggable={false}
          onError={() => setFailedPhoto(photo)}
        />
      ) : (
        <span className="mix-art__collage">
          {Array.from({ length: 4 }, (_, i) => (
            <Art key={i} path={mix.covers[i % Math.max(1, mix.covers.length)] ?? null} fluid requestSize={size / 2} kind="album" />
          ))}
        </span>
      )}
      <span className="mix-art__strip">
        <span className="mix-art__name">{mix.kind === "topSongs" ? "Top Songs" : mix.title}</span>
      </span>
    </span>
  );
}

/** A mix card for Home's shelves. */
export function MixCard({ mix }: { mix: MixSummary }) {
  const { isContext, playing } = useContextState(mix.kind === "radio" ? "radio" : "mix", mix.id);
  const play = () => (isContext ? player.toggle() : player.playMix(mix.id));
  const title = mix.kind === "daylist" ? "daylist" : mix.title;
  const subtitle = mix.kind === "daylist" ? "Your day in a playlist." : mix.description;
  return (
    <div className={`card card--mix ${playing ? "is-playing" : ""}`} data-testid={`mix-card-${mix.id}`}>
      <div className="card__art">
        <MixArt mix={mix} />
        <button
          type="button"
          className="card__play"
          aria-label={`${playing ? "Pause" : "Play"} ${title}`}
          onClick={(e) => {
            e.stopPropagation();
            play();
          }}
        >
          {playing ? <Pause size={20} fill="currentColor" aria-hidden="true" /> : <Play size={20} fill="currentColor" aria-hidden="true" />}
        </button>
      </div>
      <button type="button" className="card__link" onClick={() => openMix(mix)} title={title}>
        <span className="card__title truncate">{title}</span>
      </button>
      {subtitle && <div className="card__subtitle mix-card__subtitle">{subtitle}</div>}
    </div>
  );
}
