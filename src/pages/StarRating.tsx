/**
 * Five stars in half steps: a read-only display for diary entries and an
 * input (a slider: arrow keys move half a star, Page Up and Page Down a
 * whole one, 1 to 5 jump, Delete clears; a click on a star's left or right
 * half picks it).
 */
import { Star } from "lucide-react";
import type { KeyboardEvent, MouseEvent } from "react";
import { starsText } from "../lib/diary";

function Stars({ value, size }: { value: number; size: number }) {
  return (
    <>
      {[1, 2, 3, 4, 5].map((n) => {
        const fill = value >= n ? 100 : value >= n - 0.5 ? 50 : 0;
        return (
          <span key={n} className="stars__star" style={{ width: size, height: size }}>
            <Star size={size} className="stars__empty" aria-hidden="true" />
            {fill > 0 && (
              <span className="stars__fill" style={{ width: `${fill}%` }}>
                <Star size={size} fill="currentColor" aria-hidden="true" />
              </span>
            )}
          </span>
        );
      })}
    </>
  );
}

export function StarDisplay({ value, size = 14 }: { value: number; size?: number }) {
  return (
    <span className="stars stars--display" role="img" aria-label={`Rated ${value} out of 5`}>
      <Stars value={value} size={size} />
    </span>
  );
}

const clamp = (v: number) => Math.max(0, Math.min(5, Math.round(v * 2) / 2));

export function StarInput({
  value,
  onChange,
  label,
  labelledBy,
  size = 26,
}: {
  value: number | null;
  onChange: (v: number | null) => void;
  /** The slider's name, or the id of visible text that names it. */
  label?: string;
  labelledBy?: string;
  size?: number;
}) {
  const v = value ?? 0;
  const set = (next: number) => onChange(clamp(next) === 0 ? null : clamp(next));
  const onKey = (e: KeyboardEvent<HTMLSpanElement>) => {
    let next: number | null = null;
    if (e.key === "ArrowRight" || e.key === "ArrowUp") next = v + 0.5;
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown") next = v - 0.5;
    else if (e.key === "PageUp") next = v + 1;
    else if (e.key === "PageDown") next = v - 1;
    else if (e.key === "Home" || e.key === "Delete" || e.key === "Backspace" || e.key === "0") next = 0;
    else if (e.key === "End") next = 5;
    else if (/^[1-5]$/.test(e.key)) next = Number(e.key);
    if (next == null) return;
    e.preventDefault();
    set(next);
  };
  // A click, not a press, so a press that drags away picks nothing.
  const onPick = (e: MouseEvent<HTMLSpanElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    if (r.width <= 0) return;
    const at = Math.ceil(((e.clientX - r.left) / r.width) * 10) / 2;
    // A click on the rating it already has clears it.
    set(at === v ? 0 : at);
  };
  return (
    <span
      className="stars stars--input"
      role="slider"
      tabIndex={0}
      aria-label={labelledBy ? undefined : label}
      aria-labelledby={labelledBy}
      aria-orientation="horizontal"
      aria-valuemin={0}
      aria-valuemax={5}
      aria-valuenow={v}
      aria-valuetext={value == null ? "No rating" : starsText(value)}
      onKeyDown={onKey}
      onClick={onPick}
    >
      <Stars value={v} size={size} />
    </span>
  );
}
