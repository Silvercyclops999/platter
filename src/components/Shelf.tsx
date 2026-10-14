import { ChevronLeft, ChevronRight } from "lucide-react";
import { Children, isValidElement, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { nextArtRange, sameRange, type ArtRange } from "../lib/shelfWindow";
import { ArtActive, FluidArtSize } from "./Art";
import { IconButton } from "./Button";
import { CardSkeleton } from "./Card";

export interface ShelfProps {
  title: string;
  onShowAll?: () => void;
  children?: ReactNode;
  loading?: boolean;
  skeletonKind?: "album" | "artist" | "playlist";
  /** Shown between the heading and the row (filter chips). */
  filters?: ReactNode;
}

/**
 * Horizontal row of cards with a "Show all" link and scroll arrows.
 *
 * Only cards on screen or near it get their artwork (see lib/shelfWindow);
 * cards far out of view keep their box and skeleton but hold no decoded
 * image. Art is requested at the cards' real rendered width.
 */
export function Shelf({ title, onShowAll, children, loading, skeletonKind = "album", filters }: ShelfProps) {
  const id = useId();
  const row = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ start: true, end: true });
  // Memoized: toArray clones the elements, and new elements on every scroll
  // event would re-render every card.
  const items = useMemo(() => (loading ? [] : Children.toArray(children)), [children, loading]);
  const count = items.length;
  // null until measured, so no card asks for art before the shelf knows its size.
  const [range, setRange] = useState<ArtRange | null>(null);
  const [artWidth, setArtWidth] = useState<number | null>(null);
  const countRef = useRef(count);
  countRef.current = count;

  const measure = () => {
    const el = row.current;
    if (!el) return;
    const start = el.scrollLeft <= 2;
    const end = el.scrollLeft + el.clientWidth >= el.scrollWidth - 2;
    setEdges((e) => (e.start === start && e.end === end ? e : { start, end }));
    const first = el.firstElementChild as HTMLElement | null;
    const itemWidth = first ? first.getBoundingClientRect().width : 0;
    const art = first?.querySelector(".card__art")?.getBoundingClientRect().width ?? 0;
    if (art > 0) setArtWidth(art);
    const n = countRef.current;
    setRange((prev) => {
      // No layout (tests, a hidden window): every card gets its art.
      const next =
        itemWidth > 0 && el.clientWidth > 0
          ? nextArtRange(prev, { scrollLeft: el.scrollLeft, viewport: el.clientWidth, itemWidth, count: n })
          : { start: 0, end: n };
      return sameRange(prev, next) ? prev : next;
    });
  };

  // Before paint, so the first frame already requests the right cards.
  useLayoutEffect(measure, [count, loading]);

  useEffect(() => {
    const el = row.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

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
      {filters}
      <FluidArtSize.Provider value={artWidth}>
        <div className="shelf__row" ref={row} onScroll={measure} role="list">
          {loading
            ? Array.from({ length: 8 }, (_, i) => (
                <div role="listitem" key={i} className="shelf__item">
                  <CardSkeleton kind={skeletonKind} />
                </div>
              ))
            : items.map((child, i) => (
                <ArtActive.Provider key={isValidElement(child) && child.key != null ? child.key : i} value={!!range && i >= range.start && i < range.end}>
                  {child}
                </ArtActive.Provider>
              ))}
        </div>
      </FluidArtSize.Provider>
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
