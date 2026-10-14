import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { FluidArtSize } from "./Art";

export interface VirtualGridProps {
  count: number;
  renderItem: (index: number) => ReactNode;
  /** Minimum card column width (px). */
  minColumnWidth?: number;
  gap?: number;
  /** Extra height below the square art (title + subtitle + padding). */
  textHeight?: number;
  /** Called with the visible index range so callers can fetch pages lazily. */
  onRangeChange?: (start: number, end: number) => void;
  label: string;
}

/**
 * Card grid virtualized by rows against the main view's scroller, so a
 * 5,000-album library renders only what's on screen.
 */
export function VirtualGrid({
  count,
  renderItem,
  minColumnWidth = 176,
  gap = 8,
  textHeight = 100,
  onRangeChange,
  label,
}: VirtualGridProps) {
  const root = useRef<HTMLDivElement>(null);
  const [scrollEl, setScrollEl] = useState<HTMLElement | null>(null);
  const [width, setWidth] = useState(0);
  const [offset, setOffset] = useState(0);

  useLayoutEffect(() => {
    const el = root.current;
    if (!el) return;
    const sc = el.closest<HTMLElement>(".main__scroll") ?? (document.scrollingElement as HTMLElement | null);
    setScrollEl(sc);
    const measure = () => {
      setWidth(el.clientWidth);
      if (sc) setOffset(el.getBoundingClientRect().top - sc.getBoundingClientRect().top + sc.scrollTop);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const cols = Math.max(2, Math.floor((width + gap) / (minColumnWidth + gap)) || 2);
  const colWidth = width > 0 ? (width - gap * (cols - 1)) / cols : minColumnWidth;
  // Card: 12px padding each side around the square art.
  const artWidth = colWidth - 24;
  const rowHeight = Math.round(artWidth + textHeight) + gap;
  const rows = Math.ceil(count / cols);

  const v = useVirtualizer({
    count: rows,
    getScrollElement: () => scrollEl,
    estimateSize: () => rowHeight,
    // Rows mounted beyond the viewport each side. Their art loads too (it's
    // inside the lazy-load margin), so keep this small.
    overscan: 2,
    scrollMargin: offset,
    initialRect: { width: 1024, height: 800 },
  });

  useEffect(() => {
    v.measure();
  }, [rowHeight, v]);

  const items = v.getVirtualItems();
  const first = items[0]?.index ?? 0;
  const last = items[items.length - 1]?.index ?? 0;
  useEffect(() => {
    onRangeChange?.(first * cols, Math.min(count, (last + 1) * cols));
  }, [first, last, cols, count, onRangeChange]);

  return (
    // Cards request art for the real column width (not a fixed guess).
    <FluidArtSize.Provider value={width > 0 ? artWidth : null}>
      <div ref={root} className="vgrid" role="list" aria-label={label} aria-rowcount={rows} style={{ height: v.getTotalSize(), position: "relative" }}>
        {items.map((row) => (
          <div
            key={row.key}
            className="vgrid__row"
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              width: "100%",
              height: rowHeight - gap,
              transform: `translateY(${row.start - v.options.scrollMargin}px)`,
              display: "grid",
              gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
              columnGap: gap,
            }}
          >
            {Array.from({ length: cols }, (_, c) => {
              const i = row.index * cols + c;
              if (i >= count) return <div key={c} />;
              return (
                <div key={c} role="listitem" aria-posinset={i + 1} aria-setsize={count}>
                  {renderItem(i)}
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </FluidArtSize.Provider>
  );
}
