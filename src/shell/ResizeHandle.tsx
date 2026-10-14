import { useRef, type KeyboardEvent, type PointerEvent, type RefObject } from "react";

export interface DragSession {
  /** Pointer moved `dx` px from where the drag started; returns the new width. */
  move: (dx: number) => number;
  /** Pointer released: save the width. */
  end: () => void;
}

/**
 * Column-resize handle between two panels: a focusable separator. The pointer
 * is captured for the whole drag, and each move goes to the session (which
 * writes a CSS variable) rather than to React state. Arrow keys, Home and End
 * resize from the keyboard via `onKey`.
 */
export function ResizeHandle({
  label,
  value,
  min,
  max,
  className,
  shell,
  begin,
  onKey,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  className: string;
  /** The shell element; it gets `is-resizing` during a drag. */
  shell: RefObject<HTMLElement | null>;
  begin: () => DragSession;
  /** Handle a key; true when it resized. */
  onKey: (key: string) => boolean;
}) {
  const drag = useRef<{ x: number; id: number; session: DragSession } | null>(null);

  const finish = (el: HTMLElement) => {
    const d = drag.current;
    if (!d) return;
    drag.current = null;
    if (el.hasPointerCapture?.(d.id)) el.releasePointerCapture(d.id);
    shell.current?.classList.remove("is-resizing");
    d.session.end();
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      className={`resize-handle ${className}`}
      onPointerDown={(e: PointerEvent<HTMLDivElement>) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture?.(e.pointerId);
        drag.current = { x: e.clientX, id: e.pointerId, session: begin() };
        shell.current?.classList.add("is-resizing");
      }}
      onPointerMove={(e: PointerEvent<HTMLDivElement>) => {
        const d = drag.current;
        if (!d || e.pointerId !== d.id) return;
        const w = d.session.move(e.clientX - d.x);
        e.currentTarget.setAttribute("aria-valuenow", String(Math.round(w)));
      }}
      onPointerUp={(e) => finish(e.currentTarget)}
      onPointerCancel={(e) => finish(e.currentTarget)}
      onLostPointerCapture={(e) => finish(e.currentTarget)}
      onKeyDown={(e: KeyboardEvent<HTMLDivElement>) => {
        if (e.altKey || e.ctrlKey || e.metaKey) return;
        if (onKey(e.key)) e.preventDefault();
      }}
    />
  );
}
