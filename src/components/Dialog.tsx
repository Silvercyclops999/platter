import { useEffect, useRef, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  /** Accessible name. */
  label: string;
  children: ReactNode;
  className?: string;
  /** Element to focus on open (defaults to the first focusable). */
  initialFocus?: React.RefObject<HTMLElement | null>;
  placement?: "center" | "top";
}

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** What Tab can reach inside the dialog: enabled, and not hidden. */
export function tabbable(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter((el) => {
    if (el.closest("[hidden], [inert], [aria-hidden='true']")) return false;
    if (window.getComputedStyle(el).visibility === "hidden") return false;
    // display: none on the element or any parent up to the dialog.
    for (let n: HTMLElement | null = el; n && n !== root; n = n.parentElement) {
      if (window.getComputedStyle(n).display === "none") return false;
    }
    return true;
  });
}

/** Modal dialog: focus trap, Escape / backdrop click to close, focus restore. */
export function Dialog({ open, onClose, label, children, className = "", initialFocus, placement = "center" }: DialogProps) {
  const panel = useRef<HTMLDivElement>(null);
  const restore = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return;
    restore.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const t = setTimeout(() => {
      const target = initialFocus?.current ?? (panel.current ? tabbable(panel.current)[0] : null) ?? panel.current;
      target?.focus();
    }, 0);
    return () => {
      clearTimeout(t);
      restore.current?.focus?.();
    };
  }, [open, initialFocus]);

  if (!open) return null;

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose();
      return;
    }
    if (e.key !== "Tab" || !panel.current) return;
    const els = tabbable(panel.current);
    if (!els.length) {
      // Nothing to move to: keep focus on the dialog itself.
      e.preventDefault();
      panel.current.focus();
      return;
    }
    const first = els[0];
    const last = els[els.length - 1];
    const at = document.activeElement as HTMLElement | null;
    // Focus on the panel itself (as on open) or outside it: Tab goes to the
    // first control, Shift+Tab to the last.
    const lost = !at || at === panel.current || !panel.current.contains(at);
    if (e.shiftKey && (at === first || lost)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && (at === last || lost)) {
      e.preventDefault();
      first.focus();
    }
  };

  return createPortal(
    <div className={`dialog-backdrop dialog-backdrop--${placement}`} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
        className={`dialog ${className}`}
        onKeyDown={onKeyDown}
      >
        {children}
      </div>
    </div>,
    document.body,
  );
}
