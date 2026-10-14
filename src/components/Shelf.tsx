import { ChevronLeft, ChevronRight } from "lucide-react";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { IconButton } from "./Button";
import { CardSkeleton } from "./Card";

export interface ShelfProps {
  title: string;
  onShowAll?: () => void;
  children?: ReactNode;
  loading?: boolean;
  skeletonKind?: "album" | "artist" | "playlist";
}

/** Horizontal row of cards with a "Show all" link and scroll arrows. */
export function Shelf({ title, onShowAll, children, loading, skeletonKind = "album" }: ShelfProps) {
  const id = useId();
  const row = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ start: true, end: true });

  const measure = () => {
    const el = row.current;
    if (!el) return;
    setEdges({ start: el.scrollLeft <= 2, end: el.scrollLeft + el.clientWidth >= el.scrollWidth - 2 });
  };

  useEffect(() => {
    measure();
    const el = row.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [children, loading]);

  const scrollBy = (dir: 1 | -1) => {
    const el = row.current;
    if (!el) return;
    el.scrollBy({ left: dir * el.clientWidth * 0.8, behavior: "smooth" });
  };

  return (
    <section className="shelf" aria-labelledby={id}>
      <header className="shelf__head">
        <h2 id={id} className="shelf__title">
          {title}
        </h2>
        <div className="shelf__tools">
          {!edges.start && <IconButton size="sm" label={`Scroll ${title} left`} tooltip={false} icon={<ChevronLeft size={18} />} onClick={() => scrollBy(-1)} />}
          {!edges.end && <IconButton size="sm" label={`Scroll ${title} right`} tooltip={false} icon={<ChevronRight size={18} />} onClick={() => scrollBy(1)} />}
          {onShowAll && (
            <button type="button" className="shelf__all" onClick={onShowAll}>
              Show all
            </button>
          )}
        </div>
      </header>
      <div className="shelf__row" ref={row} onScroll={measure} role="list">
        {loading
          ? Array.from({ length: 8 }, (_, i) => (
              <div role="listitem" key={i} className="shelf__item">
                <CardSkeleton kind={skeletonKind} />
              </div>
            ))
          : children}
      </div>
    </section>
  );
}

export function ShelfItem({ children }: { children: ReactNode }) {
  return (
    <div role="listitem" className="shelf__item">
      {children}
    </div>
  );
}
