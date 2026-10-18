/**
 * The "?" beside a Settings row's title: its longer text in a small popover,
 * so the row itself keeps a one-line description.
 */
import { HelpCircle } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";

export function MoreText({ title, text }: { title: string; text: string }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const ref = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  }, [open]);
  return (
    <span className="setting-more" ref={ref}>
      <button
        type="button"
        className="setting-more__button"
        aria-label={`More about ${title}`}
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && open) {
            e.stopPropagation();
            setOpen(false);
          }
        }}
      >
        <HelpCircle size={16} aria-hidden="true" />
      </button>
      <span id={id} role="note" className="setting-more__pop" hidden={!open}>
        {text}
      </span>
    </span>
  );
}
