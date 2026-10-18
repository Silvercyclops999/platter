/**
 * The "?" beside a Settings row's title: its longer text in a small popover,
 * so the row itself keeps a one-line description.
 */
import { HelpCircle } from "lucide-react";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";

/** Space kept between the popover and the window's edge. */
const EDGE = 8;

export function MoreText({ title, text }: { title: string; text: string }) {
  const [open, setOpen] = useState(false);
  const [shift, setShift] = useState(0);
  const id = useId();
  const ref = useRef<HTMLSpanElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const pop = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  }, [open]);
  // Kept inside the window: a "?" near the right edge opens its text to the left.
  useLayoutEffect(() => {
    if (!open) {
      setShift(0);
      return;
    }
    const r = pop.current?.getBoundingClientRect();
    if (!r || !r.width) return;
    const over = r.right - (window.innerWidth - EDGE);
    if (over > 0) setShift(-Math.min(over, Math.max(0, r.left - EDGE)));
  }, [open]);
  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  };
  return (
    <span
      className="setting-more"
      ref={ref}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open) {
          e.stopPropagation();
          close(true);
        }
      }}
      onBlur={(e) => {
        // Tabbing away closes it.
        if (open && !ref.current?.contains(e.relatedTarget as Node | null)) setOpen(false);
      }}
    >
      <button
        ref={button}
        type="button"
        className="setting-more__button"
        aria-label={`More about ${title}`}
        aria-expanded={open}
        aria-controls={id}
        aria-describedby={open ? id : undefined}
        onClick={() => setOpen((v) => !v)}
      >
        <HelpCircle size={16} aria-hidden="true" />
      </button>
      <span id={id} ref={pop} role="note" className="setting-more__pop" hidden={!open} style={shift ? { transform: `translateX(${shift}px)` } : undefined}>
        {text}
      </span>
    </span>
  );
}
