import { Disc3, ListMusic, User } from "lucide-react";
import { useState } from "react";
import { artUrl } from "../lib/backend";

export interface ArtProps {
  path: string | null | undefined;
  /** Display size in CSS px (the box is square). Use `fluid` to fill the parent width. */
  size?: number;
  fluid?: boolean;
  /** Requested image size when fluid. */
  requestSize?: number;
  round?: boolean;
  kind?: "album" | "artist" | "playlist";
  alt?: string;
  className?: string;
  onLoad?: (img: HTMLImageElement) => void;
}

/**
 * Fixed-aspect artwork box: reserves its square before the image arrives (no
 * layout shift), shows a skeleton until loaded, then fades the image in.
 */
export function Art({ path, size = 48, fluid, requestSize, round, kind = "album", alt = "", className = "", onLoad }: ArtProps) {
  const src = artUrl(path, requestSize ?? size);
  // Track which src loaded/failed so a new src starts fresh without an effect.
  const [loadedSrc, setLoadedSrc] = useState<string | null>(null);
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  const loaded = !!src && loadedSrc === src;
  const failed = !!src && failedSrc === src;

  const style = fluid ? undefined : { width: size, height: size };
  const Fallback = kind === "artist" ? User : kind === "playlist" ? ListMusic : Disc3;

  return (
    <span className={`art ${round ? "art--round" : ""} ${loaded ? "is-loaded" : ""} ${className}`} style={style}>
      {(!src || failed) && (
        <span className="art__fallback" aria-hidden="true">
          <Fallback size={fluid ? 40 : Math.max(16, Math.round(size * 0.38))} strokeWidth={1.5} />
        </span>
      )}
      {src && !failed && !loaded && <span className="art__skeleton skeleton" aria-hidden="true" />}
      {src && !failed && (
        <img
          src={src}
          alt={alt}
          draggable={false}
          loading="lazy"
          decoding="async"
          onLoad={(e) => {
            setLoadedSrc(src ?? null);
            onLoad?.(e.currentTarget);
          }}
          onError={() => setFailedSrc(src ?? null)}
        />
      )}
    </span>
  );
}
