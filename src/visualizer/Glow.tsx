/**
 * Glow: the cover's own colours, softened, breathe with the low end behind
 * the cover. The only decorative style, and a quiet one: a slow follower on
 * the bass sets the light's opacity and a few percent of size, nothing
 * flashes, and it is off under reduced motion. The light is the cover
 * itself, blurred, moved only by opacity and scale, so the compositor does
 * the work on the GPU.
 */
import { useRef } from "react";
import { Art } from "../components/Art";
import { usePlayer } from "../stores/player";
import { useFrameLoop } from "./draw";
import { latestFrame } from "./feed";
import { current, follow, lowEnd } from "./scale";

/** How slowly the light follows the bass (ms). */
const BREATH_MS = 420;

/** The light's opacity and scale for a low-end level 0..1. */
export function glowAt(level: number): { opacity: number; scale: number } {
  const v = Math.min(1, Math.max(0, level));
  return { opacity: 0.22 + 0.5 * v, scale: 1 + 0.06 * v };
}

export function Glow({ fps }: { fps: number }) {
  const track = usePlayer((s) => s.state?.track ?? null);
  const light = useRef<HTMLDivElement>(null);
  const level = useRef(0);
  useFrameLoop(
    (dt, now) => {
      const { frame, at } = latestFrame();
      const f = current(frame, at, now);
      level.current = follow(level.current, lowEnd(f?.bands ?? null), dt, BREATH_MS);
      const el = light.current;
      if (!el) return;
      const g = glowAt(level.current);
      el.style.opacity = g.opacity.toFixed(3);
      el.style.transform = `scale(${g.scale.toFixed(4)})`;
    },
    fps,
    !!track,
  );
  if (!track) return null;
  return (
    <div className="vis-glow" role="img" aria-label="The cover's colours, moving with the bass">
      <div ref={light} className="vis-glow__light" aria-hidden="true">
        <Art path={track.thumb} fluid requestSize={160} />
      </div>
      <div className="vis-glow__cover">
        <Art path={track.thumb} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} fluid requestSize={640} />
      </div>
    </div>
  );
}
