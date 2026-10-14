import { useRef, type KeyboardEvent } from "react";

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
}

/** Pill filter chips (single select). */
export function FilterChips<T extends string>({ label, items, value, onChange, className = "" }: FilterChipsProps<T>) {
  return (
    <div role="group" aria-label={label} className={`chips ${className}`}>
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
    </div>
  );
}
