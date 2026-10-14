import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type RefObject } from "react";
import { createPortal } from "react-dom";
import { motion } from "../design/tokens";
import { Art } from "./Art";

/** Largest the cover is shown, in CSS px. */
export const COVER_MAX = 640;
/** Space kept clear around the cover and under it for the close button. */
const EDGE = 48;
const CLOSE_ROW = 56;

/** Cover size for a window: up to COVER_MAX, never wider or taller than the window allows. */
export function coverSize(width: number, height: number): number {
  return Math.max(120, Math.floor(Math.min(COVER_MAX, width - EDGE * 2, height - EDGE * 2 - CLOSE_ROW)));
}

function windowCoverSize(): number {
  return typeof window === "undefined" ? COVER_MAX : coverSize(window.innerWidth, window.innerHeight);
}

export interface CoverPopupProps {
  open: boolean;
  onClose: () => void;
  path: string | null | undefined;
  /** Accessible name of the popup, e.g. the album title. */
  title: string;
  /** Gets focus back when the popup closes. */
  returnFocus?: RefObject<HTMLElement | null>;
}

type Phase = "closed" | "opening" | "open" | "closing";

/**
 * Large cover over a dark scrim. Escape, a click on the scrim or the close
 * button closes it; a click on the cover doesn't. Focus stays on the close
 * button while it's open and goes back to `returnFocus` after. Opens and
 * closes in the medium motion band; under reduced motion only the fade runs.
 */
export function CoverPopup({ open, onClose, path, title, returnFocus }: CoverPopupProps) {
  const [phase, setPhase] = useState<Phase>("closed");
  const [size, setSize] = useState(windowCoverSize);
  const closeBtn = useRef<HTMLButtonElement>(null);

  // Mount closed, then flip to open on the next frame so the transition runs.
  // Closing plays the reverse, then unmounts.
  useLayoutEffect(() => {
    if (open) {
      setSize(windowCoverSize());
      setPhase((p) => (p === "open" ? p : "opening"));
    } else {
      setPhase((p) => (p === "closed" ? p : "closing"));
    }
  }, [open]);

  useEffect(() => {
    if (phase === "opening") {
      const raf = requestAnimationFrame(() => setPhase("open"));
      closeBtn.current?.focus();
      return () => cancelAnimationFrame(raf);
    }
    if (phase === "closing") {
      const t = setTimeout(() => setPhase("closed"), motion.medium);
      returnFocus?.current?.focus();
      return () => clearTimeout(t);
    }
  }, [phase, returnFocus]);

  useEffect(() => {
    if (phase !== "open" && phase !== "opening") return;
    const onResize = () => setSize(windowCoverSize());
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [phase]);

  if (phase === "closed") return null;

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      e.preventDefault();
      onClose();
    } else if (e.key === "Tab") {
      // The close button is the only stop.
      e.preventDefault();
      closeBtn.current?.focus();
    }
  };

  const shown = phase === "open";
  return createPortal(
    <div
      className={`cover-popup ${shown ? "is-open" : ""}`}
      role="dialog"
      aria-modal="true"
      aria-label={`${title} cover`}
      onKeyDown={onKeyDown}
      onClick={(e) => e.target === e.currentTarget && onClose()}
      data-phase={phase}
    >
      <div className="cover-popup__body" onClick={(e) => e.target === e.currentTarget && onClose()}>
        <div className="cover-popup__cover" style={{ width: size, height: size }}>
          <Art path={path} size={size} alt={`${title} cover`} />
        </div>
        <button ref={closeBtn} type="button" className="cover-popup__close" onClick={onClose}>
          Close
        </button>
      </div>
    </div>,
    document.body,
  );
}
