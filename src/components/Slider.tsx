import { useRef, useState, type KeyboardEvent, type PointerEvent } from "react";

export interface SliderProps {
  /** Accessible name, e.g. "Seek" or "Volume". */
  label: string;
  value: number;
  min?: number;
  max: number;
  /** Arrow-key step. PageUp/PageDown move 10% of the range. */
  step?: number;
  /** Called continuously while dragging / on each key press. */
  onChange?: (v: number) => void;
  /** Called once when the user lets go (pointer up) and on each key press. */
  onCommit?: (v: number) => void;
  /** aria-valuetext, e.g. "1:02 of 3:45". */
  valueText?: (v: number) => string;
  disabled?: boolean;
  className?: string;
  /** Optional buffered amount, drawn behind the fill (seek bar). */
  buffered?: number;
  /** Bubble above the pointer while hovering or dragging, e.g. the time to seek to. */
  hoverText?: (v: number) => string;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/**
 * Horizontal slider for seek and volume. Keyboard: arrows (step),
 * PageUp/PageDown (10%), Home/End. Hovering or focusing shows the thumb and
 * the accent fill; dragging holds a local value until release.
 */
export function Slider({
  label,
  value,
  min = 0,
  max,
  step = 1,
  onChange,
  onCommit,
  valueText,
  disabled,
  className = "",
  buffered,
  hoverText,
}: SliderProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const shown = drag ?? value;
  const range = max - min || 1;
  const pct = clamp(((shown - min) / range) * 100, 0, 100);
  const bufPct = buffered != null ? clamp(((buffered - min) / range) * 100, 0, 100) : null;

  const fromPointer = (clientX: number) => {
    const r = trackRef.current!.getBoundingClientRect();
    const ratio = r.width > 0 ? clamp((clientX - r.left) / r.width, 0, 1) : 0;
    return min + ratio * range;
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled || e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).focus();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    const v = fromPointer(e.clientX);
    setDrag(v);
    onChange?.(v);
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    const v = fromPointer(e.clientX);
    if (hoverText) setHover(v);
    if (drag == null) return;
    setDrag(v);
    onChange?.(v);
  };
  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    if (drag == null) return;
    const v = fromPointer(e.clientX);
    setDrag(null);
    onCommit?.(v);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (disabled) return;
    const big = range / 10;
    let next: number | null = null;
    switch (e.key) {
      case "ArrowRight":
      case "ArrowUp":
        next = value + step;
        break;
      case "ArrowLeft":
      case "ArrowDown":
        next = value - step;
        break;
      case "PageUp":
        next = value + big;
        break;
      case "PageDown":
        next = value - big;
        break;
      case "Home":
        next = min;
        break;
      case "End":
        next = max;
        break;
    }
    if (next == null) return;
    e.preventDefault();
    e.stopPropagation();
    next = clamp(next, min, max);
    onChange?.(next);
    onCommit?.(next);
  };

  return (
    <div
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={Math.round(shown * 1000) / 1000}
      aria-valuetext={valueText?.(shown)}
      aria-disabled={disabled || undefined}
      className={`slider ${drag != null ? "is-dragging" : ""} ${disabled ? "is-disabled" : ""} ${className}`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => setDrag(null)}
      onPointerLeave={() => setHover(null)}
      onKeyDown={onKeyDown}
    >
      <div className="slider__track" ref={trackRef}>
        {bufPct != null && <div className="slider__buffer" style={{ width: `${bufPct}%` }} />}
        <div className="slider__fill" style={{ width: `${pct}%` }} />
        <div className="slider__thumb" style={{ left: `${pct}%` }} />
      </div>
      {hoverText && hover != null && !disabled && (
        <div className="slider__hover tabular" aria-hidden="true" style={{ left: `${clamp(((hover - min) / range) * 100, 0, 100)}%` }}>
          {hoverText(hover)}
        </div>
      )}
    </div>
  );
}
