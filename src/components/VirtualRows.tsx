import { observeElementRect, useVirtualizer, type Virtualizer } from "@tanstack/react-virtual";
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

/** The scroller's size, ignoring a zero height (no layout yet, or tests). */
function observeNonEmptyRect(instance: Virtualizer<HTMLElement, Element>, cb: (rect: { width: number; height: number }) => void) {
  return observeElementRect(instance, (rect) => cb(rect.height > 0 ? rect : (instance.options.initialRect ?? rect)));
}

/** Lists up to this long render every row; longer ones only what's on screen. */
export const VIRTUAL_ROWS_FROM = 200;

export interface VirtualRowsProps {
  count: number;
  /** About how tall one row is (px). Rows stay in normal flow, so a row
   * that differs only nudges the scrollbar, never overlaps. */
  rowHeight: number;
  /** One row: an `<li>` when inside a list. */
  renderRow: (index: number) => ReactNode;
  /** The element that wraps spacers (`li` inside a `ul`). */
  spacer?: "li" | "div";
}

/**
 * Rows of a long list (an imported playlist, the Missing list) rendered
 * against the main view's scroller: short lists render whole, long ones only
 * the rows near the viewport, with spacers keeping the scroll height.
 */
export function VirtualRows(props: VirtualRowsProps) {
  if (props.count <= VIRTUAL_ROWS_FROM) {
    return <>{Array.from({ length: props.count }, (_, i) => props.renderRow(i))}</>;
  }
  return <Windowed {...props} />;
}

function Windowed({ count, rowHeight, renderRow, spacer = "li" }: VirtualRowsProps) {
  const top = useRef<HTMLElement | null>(null);
  const [scrollEl, setScrollEl] = useState<HTMLElement | null>(null);
  const [offset, setOffset] = useState(0);

  useLayoutEffect(() => {
    const el = top.current;
    if (!el) return;
    const sc = el.closest<HTMLElement>(".main__scroll") ?? (document.scrollingElement as HTMLElement | null);
    setScrollEl(sc);
    const measure = () => sc && setOffset(el.getBoundingClientRect().top - sc.getBoundingClientRect().top + sc.scrollTop);
    measure();
    if (typeof ResizeObserver === "undefined" || !sc) return;
    const ro = new ResizeObserver(measure);
    ro.observe(sc);
    return () => ro.disconnect();
  }, []);

  const v = useVirtualizer({
    count,
    getScrollElement: () => scrollEl,
    estimateSize: () => rowHeight,
    overscan: 10,
    scrollMargin: offset,
    initialRect: { width: 1024, height: 900 },
    observeElementRect: observeNonEmptyRect,
  });
  const items = v.getVirtualItems();
  const first = items[0]?.index ?? 0;
  const last = items.length ? items[items.length - 1].index : -1;
  const before = first * rowHeight;
  const after = Math.max(0, (count - last - 1) * rowHeight);
  const Spacer = spacer;
  return (
    <>
      <Spacer ref={(el: HTMLElement | null) => void (top.current = el)} aria-hidden="true" className="virtual-spacer" style={{ height: before }} />
      {items.map((r) => renderRow(r.index))}
      <Spacer aria-hidden="true" className="virtual-spacer" style={{ height: after }} />
    </>
  );
}
