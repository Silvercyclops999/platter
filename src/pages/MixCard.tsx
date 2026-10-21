import { Pause, Play } from "lucide-react";
import { type CSSProperties } from "react";
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

/** The leading artists from a mix's "A, B, C and more" line, at most three. */
export function mixArtists(description: string): string[] {
  const m = /^(?:with )?(.+) and more$/.exec(description.trim());
  if (!m) return [];
  return m[1]
    .split(", ")
    .map((n) => n.trim())
    .filter(Boolean)
    .slice(0, 3);
}

/** The name a mix's sleeve prints. */
export const sleeveName = (mix: Pick<MixSummary, "kind" | "title">) => (mix.kind === "topSongs" ? "Top Songs" : mix.title);

/**
 * A mix's art: a printed sleeve. Paper, the mix's name in the serif, and
 * its leading artists under a brass rule (or what the mix is, when it
 * isn't made from a few artists). The sleeve always carries the name.
 * The daylist is a soft gradient instead; `named` false leaves its name
 * off, for a tile that prints it under the cover.
 */
export function MixArt({ mix, named = true }: { mix: MixSummary; named?: boolean }) {
  if (mix.kind === "daylist") {
    const [a, b] = daylistGradient(new Date().getHours());
    return (
      <span className="mix-art mix-art--daylist" style={{ "--mix-a": a, "--mix-b": b } as CSSProperties} aria-hidden="true">
        {named && <span className="mix-art__daylist">daylist</span>}
      </span>
    );
  }
  const artists = mixArtists(mix.description);
  return (
    <span className="mix-art mix-art--sleeve" data-testid="mix-sleeve" aria-hidden="true">
      <span className="mix-art__page">
        <span className="mix-art__title">{sleeveName(mix)}</span>
        {artists.length > 0 ? (
          <span className="mix-art__artists">
            {artists.map((n) => (
              <span key={n}>{n}</span>
            ))}
          </span>
        ) : (
          mix.description && <span className="mix-art__artists mix-art__artists--about">{mix.description}</span>
        )}
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
