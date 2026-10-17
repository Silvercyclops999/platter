import { Disc3, ListMusic, User } from "lucide-react";
import { createContext, useContext, useLayoutEffect, useRef, useState } from "react";
import { artUrl } from "../lib/backend";

/**
 * Rendered width (CSS px) of fluid art inside a layout that knows it, such as
 * the virtual grid's columns. Fluid art outside one uses its `requestSize`.
 */
export const FluidArtSize = createContext<number | null>(null);

/**
 * False while a layout (such as a shelf) has this art scrolled far out of
 * view: the box and skeleton stay, but no image is requested or kept decoded.
 */
export const ArtActive = createContext(true);

export interface ArtProps {
  path: string | null | undefined;
  /** Display size in CSS px (the box is square). Use `fluid` to fill the parent width. */
  size?: number;
  fluid?: boolean;
  /** Expected display size in CSS px when fluid and no `FluidArtSize` is provided. */
  requestSize?: number;
  round?: boolean;
  kind?: "album" | "artist" | "playlist";
  alt?: string;
  className?: string;
  onLoad?: (img: HTMLImageElement) => void;
  /** A name whose initials stand in when there's no picture (instead of the blank icon). */
  monogram?: string;
}

/** Up to two initials of a name ("Cold Coast" is CC, "Ostra" is O). */
export function initials(name: string): string {
  const words = name
    .replace(/^the\s+/i, "")
    .split(/[\s&+/-]+/)
    .map((w) => w.replace(/[^\p{L}\p{N}]/gu, ""))
    .filter(Boolean);
  return words
    .slice(0, 2)
    .map((w) => w[0]!.toLocaleUpperCase())
    .join("");
}

/**
 * Fixed-aspect artwork box: reserves its square before the image arrives (no
 * layout shift), shows a skeleton until loaded, then fades the image in. Art
 * that is already decoded when it mounts (from cache) shows at once, without
 * the fade.
 *
 * Images are requested at the rendered size times the display's pixel ratio,
 * rounded up to a shared step (see lib/artSize), since decoded art is most of
 * the WebView's memory.
 */
export function Art({ path, size = 48, fluid, requestSize, round, kind = "album", alt = "", className = "", onLoad, monogram }: ArtProps) {
  const fluidSize = useContext(FluidArtSize);
  const cssSize = fluid ? (fluidSize ?? requestSize ?? 200) : size;
  const active = useContext(ArtActive);
  const wanted = artUrl(path, cssSize);
  const src = active ? wanted : undefined;
  // Track which src loaded/failed so a new src starts fresh without an effect.
  const [loadedSrc, setLoadedSrc] = useState<string | null>(null);
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  // Set when the image finished loading after it was shown: only then fade.
  const [fadeSrc, setFadeSrc] = useState<string | null>(null);
  const img = useRef<HTMLImageElement>(null);
  // Released art starts over when it comes back: shown at once if the image
  // is still decoded in the cache, otherwise skeleton then fade.
  if (!src && (loadedSrc !== null || fadeSrc !== null)) {
    setLoadedSrc(null);
    setFadeSrc(null);
  }
  const loaded = !!src && loadedSrc === src;
  const failed = !!src && failedSrc === src;

  // Already complete on mount (or on a src change): mark it loaded before the
  // first paint so it neither flashes the skeleton nor fades.
  useLayoutEffect(() => {
    const el = img.current;
    if (src && el?.complete && el.naturalWidth > 0) setLoadedSrc(src);
  }, [src]);

  const style = fluid ? undefined : { width: size, height: size };
  const Fallback = kind === "artist" ? User : kind === "playlist" ? ListMusic : Disc3;
  const box = Math.round(cssSize);

  return (
    <span className={`art ${round ? "art--round" : ""} ${loaded ? "is-loaded" : ""} ${loaded && fadeSrc === src ? "is-fading" : ""} ${className}`} style={style}>
      {(!wanted || failed) &&
        (monogram && initials(monogram) ? (
          <span className="art__fallback art__monogram" aria-hidden="true" style={{ fontSize: Math.max(12, Math.round(cssSize * 0.36)) }}>
            {initials(monogram)}
          </span>
        ) : (
          <span className="art__fallback" aria-hidden="true">
            <Fallback size={fluid ? 40 : Math.max(16, Math.round(size * 0.38))} strokeWidth={1.5} />
          </span>
        ))}
      {wanted && !failed && !loaded && <span className="art__skeleton skeleton" aria-hidden="true" />}
      {src && !failed && (
        <img
          ref={img}
          src={src}
          width={box}
          height={box}
          alt={alt}
          draggable={false}
          loading="lazy"
          decoding="async"
          onLoad={(e) => {
            if (loadedSrc !== src) {
              setLoadedSrc(src ?? null);
              setFadeSrc(src ?? null);
            }
            onLoad?.(e.currentTarget);
          }}
          onError={() => setFailedSrc(src ?? null)}
        />
      )}
    </span>
  );
}
