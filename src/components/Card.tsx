import { Play } from "lucide-react";
import type { MouseEvent, ReactNode } from "react";
import { Art } from "./Art";
import { Skeleton } from "./Skeleton";

export interface CardProps {
  kind: "album" | "artist" | "playlist";
  title: string;
  subtitle?: ReactNode;
  thumb: string | null | undefined;
  onOpen: () => void;
  onPlay?: () => void;
  onContextMenu?: (e: MouseEvent<HTMLElement>) => void;
  /** Art request size in CSS px. */
  artSize?: number;
}

/**
 * Album / artist (round art) / playlist card. The whole card opens the item
 * via a stretched title button; the hover play button sits above it.
 */
export function Card({ kind, title, subtitle, thumb, onOpen, onPlay, onContextMenu, artSize = 200 }: CardProps) {
  return (
    <div className={`card card--${kind}`} onContextMenu={onContextMenu}>
      <div className="card__art">
        <Art path={thumb} fluid requestSize={artSize} round={kind === "artist"} kind={kind} />
        {onPlay && (
          <button
            type="button"
            className="card__play"
            aria-label={`Play ${title}`}
            onClick={(e) => {
              e.stopPropagation();
              onPlay();
            }}
          >
            <Play size={20} fill="currentColor" aria-hidden="true" />
          </button>
        )}
      </div>
      <button type="button" className="card__link" onClick={onOpen} title={title}>
        <span className="card__title truncate">{title}</span>
      </button>
      {subtitle && <div className="card__subtitle truncate">{subtitle}</div>}
    </div>
  );
}

export function CardSkeleton({ kind = "album" }: { kind?: "album" | "artist" | "playlist" }) {
  return (
    <div className={`card card--${kind} card--skeleton`} aria-hidden="true">
      <div className="card__art">
        <span className={`art ${kind === "artist" ? "art--round" : ""}`}>
          <span className="art__skeleton skeleton" />
        </span>
      </div>
      <Skeleton width="78%" height={14} style={{ marginTop: 2 }} />
      <Skeleton width="52%" height={12} style={{ marginTop: 8 }} />
    </div>
  );
}
