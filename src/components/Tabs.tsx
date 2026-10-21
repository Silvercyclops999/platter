import { ChevronLeft, ChevronRight } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { useRovingRow } from "./roving";

export interface TabItem<T extends string> {
  id: T;
  label: string;
}

export interface TabsProps<T extends string> {
  label: string;
  items: TabItem<T>[];
  value: T;
  onChange: (v: T) => void;
  /** id prefix used to link tabs to their panel (aria-controls). */
  idPrefix?: string;
  className?: string;
}

/** Tab list with roving focus (arrows, Home/End). */
export function Tabs<T extends string>({ label, items, value, onChange, idPrefix = "tabs", className = "" }: TabsProps<T>) {
  const ref = useRef<HTMLDivElement>(null);
  const onKeyDown = (e: KeyboardEvent) => {
    const i = items.findIndex((t) => t.id === value);
    let n: number | null = null;
    if (e.key === "ArrowRight") n = (i + 1) % items.length;
    if (e.key === "ArrowLeft") n = (i - 1 + items.length) % items.length;
    if (e.key === "Home") n = 0;
    if (e.key === "End") n = items.length - 1;
    if (n == null) return;
    e.preventDefault();
    onChange(items[n].id);
    ref.current?.querySelectorAll<HTMLElement>('[role="tab"]')[n]?.focus();
  };
  return (
    <div ref={ref} role="tablist" aria-label={label} className={`tabs ${className}`} onKeyDown={onKeyDown}>
      {items.map((t) => (
        <button
          key={t.id}
          type="button"
          role="tab"
          id={`${idPrefix}-tab-${t.id}`}
          aria-selected={t.id === value}
          aria-controls={`${idPrefix}-panel`}
          tabIndex={t.id === value ? 0 : -1}
          className={`tab ${t.id === value ? "is-selected" : ""}`}
          onClick={() => onChange(t.id)}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}

export interface FilterChipsProps<T extends string> {
  label: string;
  items: TabItem<T>[];
  value: T;
  onChange: (v: T) => void;
  className?: string;
  /** One row that scrolls sideways (with arrow buttons) instead of wrapping. */
  scroll?: boolean;
}

/** Pill filter chips (single select), one Tab stop with arrows inside. */
export function FilterChips<T extends string>({ label, items, value, onChange, className = "", scroll = false }: FilterChipsProps<T>) {
  return (
    <ChipRow label={label} className={className} scroll={scroll}>
      {items.map((t) => (
        <button
          key={t.id}
          type="button"
          aria-pressed={t.id === value}
          className={`chip ${t.id === value ? "is-selected" : ""}`}
          onClick={() => onChange(t.id)}
        >
          {t.label}
        </button>
      ))}
    </ChipRow>
  );
}

/**
 * A row of chip buttons: one Tab stop, arrows move between chips. With
 * `scroll` it stays one line and scrolls sideways, with arrow buttons at the
 * edges that have more.
 */
export function ChipRow({ label, className = "", scroll = false, children }: { label: string; className?: string; scroll?: boolean; children: ReactNode }) {
  const roving = useRovingRow();
  const edges = useScrollEdges(roving.ref, scroll);
  const row = (
    <div role="group" aria-label={label} className={`chips ${scroll ? "chips--scroll" : className}`} {...roving} onScroll={edges.update}>
      {children}
    </div>
  );
  if (!scroll) return row;
  const by = (dir: 1 | -1) => roving.ref.current?.scrollBy?.({ left: dir * 200, behavior: "smooth" });
  return (
    <div className={`chips-scroll ${edges.start ? "has-before" : ""} ${edges.end ? "has-after" : ""} ${className}`}>
      {row}
      {/* Mouse helpers; the keyboard scrolls by moving between chips. */}
      {edges.start && (
        <button type="button" tabIndex={-1} className="chips-scroll__btn chips-scroll__btn--prev" aria-label="Scroll back" onClick={() => by(-1)}>
          <ChevronLeft size={18} aria-hidden="true" />
        </button>
      )}
      {edges.end && (
        <button type="button" tabIndex={-1} className="chips-scroll__btn chips-scroll__btn--next" aria-label="Scroll for more" onClick={() => by(1)}>
          <ChevronRight size={18} aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

/** Whether a sideways scroller has more to show before and after what's visible. */
export function useScrollEdges(ref: React.RefObject<HTMLElement | null>, on: boolean) {
  const [edges, setEdges] = useState({ start: false, end: false });
  const update = () => {
    const el = ref.current;
    if (!el || !on) return;
    const start = el.scrollLeft > 1;
    const end = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
    setEdges((e) => (e.start === start && e.end === end ? e : { start, end }));
  };
  useEffect(() => {
    const el = ref.current;
    if (!el || !on) return;
    update();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  });
  return { ...edges, update };
}
