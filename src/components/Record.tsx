/**
 * A 12-inch record drawn to scale in SVG, in millimetres round its spindle
 * (see shell/recordGeometry.ts). Matte black vinyl with a fine, uneven
 * groove texture; the gaps between songs sit where the songs really start;
 * a lead-in, a run-out and the locked groove; the cover printed on the label.
 * The light is drawn separately (RecordSheen) so it stays put while the
 * record turns.
 */
import { useId, useMemo, useState, type CSSProperties } from "react";
import { sleeveHue, type SleeveText } from "./Art";
import "./record.css";
import {
  EDGE_GROOVE_R,
  LABEL_R,
  LOCKED_R,
  MUSIC_START_R,
  RECORD_R,
  SPINDLE_HOLE_R,
  SPINDLE_R,
  innerMusicRadius,
  seeded,
  trackBands,
  type TrackBand,
} from "../shell/recordGeometry";

/** The songs on the side that is up: their lengths and which one plays (-1 for none). */
export interface RecordSide {
  durationsMs: number[];
  current: number;
}

/** What the label shows: the cover, or a printed label with the title. */
export interface RecordLabel {
  url?: string | null;
  sleeve?: SleeveText | null;
  /** A plain tint in place of a cover (the small look previews). */
  hue?: number;
  /** A plain printed colour (any CSS colour), as a record's own label often is. */
  color?: string;
}

/** A side of no known songs: one band of music filling a typical side. */
const UNKNOWN_SIDE = [19 * 60_000];

interface Ring {
  r: number;
  w: number;
  /** White (light catching the groove walls) or black (a shadowed groove). */
  light: boolean;
  a: number;
}

/**
 * The groove texture of one side: fine rings at uneven spacing and weight,
 * with broader, faint changes of tone where the cut runs louder or quieter.
 * Kept very low in contrast; on a real record they only show in the light.
 */
export function grooveRings(key: string, inner: number, detail: "full" | "low"): Ring[] {
  const rnd = seeded(key);
  const rings: Ring[] = [];
  const step = detail === "full" ? [0.45, 1.6] : [1.2, 3.4];
  for (let r = inner + 0.3; r < MUSIC_START_R - 0.2; ) {
    const gap = step[0] + rnd() * (step[1] - step[0]);
    const light = rnd() < 0.62;
    rings.push({ r, w: 0.18 + rnd() * (detail === "full" ? 0.55 : 0.9), light, a: light ? 0.012 + rnd() * 0.03 : 0.12 + rnd() * 0.2 });
    r += gap;
  }
  // Louder and quieter stretches: broad, faint differences in tone.
  for (let r = inner; r < MUSIC_START_R - 2; ) {
    const w = 2 + rnd() * 9;
    if (rnd() < 0.55) rings.push({ r: Math.min(MUSIC_START_R - w / 2, r + w / 2), w, light: rnd() < 0.6, a: 0.006 + rnd() * 0.014 });
    r += w + rnd() * 6;
  }
  return rings;
}

function sideOrUnknown(side: RecordSide | null | undefined): RecordSide {
  return side && side.durationsMs.length && side.durationsMs.some((d) => d > 0) ? side : { durationsMs: UNKNOWN_SIDE, current: -1 };
}

/**
 * The record's own surface and label, round (0, 0). Drawn inside a group the
 * caller turns. `detail: "low"` thins the grooves for small or many copies.
 */
export function Vinyl({ side, label, seed, detail = "full" }: { side?: RecordSide | null; label: RecordLabel; seed: string; detail?: "full" | "low" }) {
  const raw = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const id = (s: string) => `rec${raw}-${s}`;
  const s = sideOrUnknown(side);
  const sideMs = s.durationsMs.reduce((a, b) => a + b, 0);
  const inner = innerMusicRadius(sideMs);
  const rings = useMemo(() => grooveRings(`${seed}:${s.durationsMs.join(",")}`, inner, detail), [seed, s.durationsMs, inner, detail]);
  const bands: TrackBand[] = useMemo(() => trackBands(s.durationsMs), [s.durationsMs]);
  const [failed, setFailed] = useState<string | null>(null);
  const showArt = !!label.url && failed !== label.url;
  const runOut = useMemo(() => {
    // A few turns of the run-out spiral, wide apart, between the music and the locked groove.
    const out: number[] = [];
    const n = 3;
    for (let i = 1; i <= n; i++) out.push(inner - ((inner - LOCKED_R) * i) / (n + 1));
    return out;
  }, [inner]);
  const title = label.sleeve?.title?.trim() || "";
  const hue = label.hue ?? (title ? sleeveHue(title) : 30);

  return (
    <g className="record">
      <defs>
        <clipPath id={id("label")}>
          <circle r={LABEL_R} />
        </clipPath>
        {/* Paper: a faint fibre of tiny specks, and the edge pressed a touch darker into the vinyl. */}
        <pattern id={id("paper")} width="7" height="7" patternUnits="userSpaceOnUse">
          <circle cx="1.2" cy="0.9" r="0.22" fill="#fff" opacity="0.07" />
          <circle cx="4.6" cy="2.3" r="0.18" fill="#000" opacity="0.1" />
          <circle cx="2.8" cy="4.9" r="0.25" fill="#fff" opacity="0.05" />
          <circle cx="6.1" cy="5.8" r="0.16" fill="#000" opacity="0.09" />
          <circle cx="0.4" cy="6.3" r="0.2" fill="#fff" opacity="0.06" />
          <circle cx="5.3" cy="0.4" r="0.15" fill="#000" opacity="0.07" />
        </pattern>
        <radialGradient id={id("press")}>
          <stop offset="0.86" stopColor="#000" stopOpacity="0" />
          <stop offset="1" stopColor="#000" stopOpacity="0.22" />
        </radialGradient>
        <radialGradient id={id("tint")} cx="0.35" cy="0.3" r="0.9">
          <stop offset="0" stopColor={`hsl(${hue} 55% 62%)`} />
          <stop offset="1" stopColor={`hsl(${hue} 40% 30%)`} />
        </radialGradient>
      </defs>

      {/* The vinyl. Near black, matte. */}
      <circle className="record__vinyl" r={RECORD_R} />
      {/* The edge: flat land, then a rounded bead that catches a little light. */}
      <circle r={(RECORD_R + EDGE_GROOVE_R) / 2} fill="none" stroke="#fff" strokeOpacity="0.018" strokeWidth={RECORD_R - EDGE_GROOVE_R} />
      <circle r={RECORD_R - 0.35} fill="none" stroke="#fff" strokeOpacity="0.05" strokeWidth="0.5" />
      {/* Lead-in: a turn or two at wide pitch. */}
      {[147.9, 147.0].map((r) => (
        <circle key={r} r={r} fill="none" stroke="#000" strokeOpacity="0.45" strokeWidth="0.3" />
      ))}

      {/* The music: fine grooves at uneven spacing and weight. */}
      <g fill="none">
        {rings.map((g, i) => (
          <circle key={i} r={g.r} stroke={g.light ? "#fff" : "#000"} strokeOpacity={g.a} strokeWidth={g.w} />
        ))}
      </g>
      {/* The playing song's band reads a shade lighter. */}
      {s.current >= 0 && bands[s.current] && (
        <circle
          className="record__now"
          r={(bands[s.current].outer + bands[s.current].inner) / 2}
          fill="none"
          stroke="#fff"
          strokeOpacity="0.022"
          strokeWidth={Math.max(0.4, bands[s.current].outer - bands[s.current].inner)}
        />
      )}
      {/* The gaps between songs: no modulation, so smoother and a little shinier. */}
      {bands.slice(1).map((b, i) => (
        <g key={i} className="record__gap" fill="none">
          <circle r={b.outer} stroke="#0a0a0a" strokeWidth="1.5" />
          <circle r={b.outer} stroke="#fff" strokeOpacity="0.045" strokeWidth="1.1" />
        </g>
      ))}

      {/* Run-out: smooth, with the spiral's few wide turns, then the locked groove. */}
      <circle r={(inner + LOCKED_R) / 2} fill="none" stroke="#fff" strokeOpacity="0.012" strokeWidth={Math.max(0, inner - LOCKED_R - 0.6)} />
      {runOut.map((r) => (
        <circle key={r} r={r} fill="none" stroke="#000" strokeOpacity="0.5" strokeWidth="0.25" />
      ))}
      <circle r={LOCKED_R} fill="none" stroke="#000" strokeOpacity="0.6" strokeWidth="0.35" />
      <circle r={LOCKED_R + 0.3} fill="none" stroke="#fff" strokeOpacity="0.03" strokeWidth="0.2" />

      {/* The label, printed: the cover cut round, paper grain, no border. */}
      <g clipPath={`url(#${id("label")})`} className="record__label">
        {label.color ? (
          <circle r={LABEL_R} style={{ fill: label.color }} />
        ) : showArt ? (
          <image
            href={label.url!}
            x={-LABEL_R}
            y={-LABEL_R}
            width={LABEL_R * 2}
            height={LABEL_R * 2}
            preserveAspectRatio="xMidYMid slice"
            onError={() => setFailed(label.url ?? null)}
          />
        ) : (
          <>
            <circle r={LABEL_R} fill={label.hue != null ? `url(#${id("tint")})` : `hsl(${hue} 22% 34%)`} />
            {title && label.hue == null && (
              <text className="record__label-title" y={-16} textAnchor="middle" fontSize="7.2" fill="#f1ebe0">
                {title.length > 26 ? `${title.slice(0, 25)}…` : title}
              </text>
            )}
            {label.sleeve?.artist && label.hue == null && (
              <text className="record__label-artist" y={22} textAnchor="middle" fontSize="5" fill="#f1ebe0" opacity="0.75">
                {label.sleeve.artist.length > 32 ? `${label.sleeve.artist.slice(0, 31)}…` : label.sleeve.artist}
              </text>
            )}
          </>
        )}
        <rect x={-LABEL_R} y={-LABEL_R} width={LABEL_R * 2} height={LABEL_R * 2} fill={`url(#${id("paper")})`} />
        <circle r={LABEL_R} fill={`url(#${id("press")})`} />
      </g>
    </g>
  );
}

/**
 * The light on the record: a broad, soft streak from one fixed lamp, the way
 * grooves throw light back in a band across the centre. Not part of the
 * turning group. A static CSS conic gradient, masked off the label, so it
 * costs nothing per frame.
 */
export function RecordSheen({ strength = 1 }: { strength?: number }) {
  const a = (n: number) => `rgba(255,255,255,${(n * strength).toFixed(3)})`;
  const style: CSSProperties = {
    width: "100%",
    height: "100%",
    borderRadius: "50%",
    background: `conic-gradient(from -52deg, ${a(0.06)} 0deg, ${a(0.03)} 14deg, ${a(0.008)} 30deg, transparent 48deg 132deg, ${a(0.006)} 150deg, ${a(0.028)} 170deg, ${a(0.04)} 180deg, ${a(0.028)} 190deg, ${a(0.006)} 210deg, transparent 228deg 312deg, ${a(0.008)} 330deg, ${a(0.03)} 346deg, ${a(0.06)} 360deg)`,
    // Off the label (paper doesn't shine) and softening into the music from the run-out.
    WebkitMaskImage: `radial-gradient(circle closest-side, transparent ${(LABEL_R / RECORD_R) * 100}%, #000 ${((LOCKED_R + 8) / RECORD_R) * 100}%, #000 99%, transparent 100%)`,
    maskImage: `radial-gradient(circle closest-side, transparent ${(LABEL_R / RECORD_R) * 100}%, #000 ${((LOCKED_R + 8) / RECORD_R) * 100}%, #000 99%, transparent 100%)`,
  };
  return (
    <foreignObject className="record__sheen" x={-RECORD_R} y={-RECORD_R} width={RECORD_R * 2} height={RECORD_R * 2} pointerEvents="none">
      <div style={style} />
    </foreignObject>
  );
}

/** The spindle, steel, standing through the label. */
export function Spindle() {
  const raw = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  return (
    <g className="record__spindle">
      <defs>
        <radialGradient id={`sp${raw}`} cx="0.38" cy="0.34" r="0.75">
          <stop offset="0" stopColor="#eceae6" />
          <stop offset="0.55" stopColor="#a9a7a2" />
          <stop offset="1" stopColor="#5b5955" />
        </radialGradient>
      </defs>
      <circle r={SPINDLE_R + 0.45} fill="#000" opacity="0.35" />
      <circle r={SPINDLE_R} fill={`url(#sp${raw})`} />
      {/* The chamfered tip. */}
      <circle r={SPINDLE_R * 0.55} fill="none" stroke="#fff" strokeOpacity="0.25" strokeWidth="0.3" />
    </g>
  );
}

/**
 * A record on its own, for a sleeve it's half out of or a small preview:
 * the record, its light and the spindle hole, in one SVG that fills its box.
 */
export function RecordDisc({ label, side, seed, detail = "low", className = "" }: { label: RecordLabel; side?: RecordSide | null; seed: string; detail?: "full" | "low"; className?: string }) {
  return (
    <svg className={`record-disc ${className}`} viewBox={`${-RECORD_R} ${-RECORD_R} ${RECORD_R * 2} ${RECORD_R * 2}`} aria-hidden="true" focusable="false">
      <Vinyl side={side} label={label} seed={seed} detail={detail} />
      <RecordSheen />
      {/* Off the platter the hole shows through. */}
      <circle r={SPINDLE_HOLE_R} className="record-disc__hole" />
    </svg>
  );
}
