import { Pause, Play } from "lucide-react";
import { useRef, type MouseEvent, type ReactNode } from "react";
import { Art } from "./Art";
import { Skeleton } from "./Skeleton";

export interface CardProps {
  kind: "album" | "artist" | "playlist";
  title: string;
  subtitle?: ReactNode;
  thumb: string | null | undefined;
  /** Open the item. Gets the art box so the page can morph it into its header. */
  onOpen: (art: HTMLElement | null) => void;
  /** Play (or, while `playing`, pause) without navigating. */
  onPlay?: () => void;
  /** This item is the playing context: the button shows Pause and stays visible. */
  playing?: boolean;
  onContextMenu?: (e: MouseEvent<HTMLElement>) => void;
  /** Art request size in CSS px. */
  artSize?: number;
}

/**
 * Album / artist (round art) / playlist card. The whole card opens the item
 * via a stretched title button; the hover play button sits above it.
 */
export function Card({ kind, title, subtitle, thumb, onOpen, onPlay, playing, onContextMenu, artSize = 200 }: CardProps) {
  const art = useRef<HTMLDivElement>(null);
  return (
    <div className={`card card--${kind} ${playing ? "is-playing" : ""}`} onContextMenu={onContextMenu}>
      <div className="card__art" ref={art}>
        <Art path={thumb} fluid requestSize={artSize} round={kind === "artist"} kind={kind} />
        {onPlay && (
          <button
            type="button"
            className="card__play"
            aria-label={`${playing ? "Pause" : "Play"} ${title}`}
            onClick={(e) => {
              e.stopPropagation();
              onPlay();
            }}
          >
            {playing ? <Pause size={20} fill="currentColor" aria-hidden="true" /> : <Play size={20} fill="currentColor" aria-hidden="true" />}
          </button>
        )}
      </div>
      <button type="button" className="card__link" onClick={() => onOpen(art.current?.querySelector<HTMLElement>(".art") ?? null)} title={title}>
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
