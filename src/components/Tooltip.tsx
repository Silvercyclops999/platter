import {
  Children,
  cloneElement,
  isValidElement,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type MouseEvent,
  type ReactElement,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

type AnyProps = Record<string, unknown> & {
  onMouseEnter?: (e: MouseEvent<HTMLElement>) => void;
  onMouseLeave?: (e: MouseEvent<HTMLElement>) => void;
  onFocus?: (e: FocusEvent<HTMLElement>) => void;
  onBlur?: (e: FocusEvent<HTMLElement>) => void;
  onPointerDown?: (e: unknown) => void;
};

export interface TooltipProps {
  label: ReactNode;
  children: ReactElement;
  placement?: "top" | "bottom" | "right";
  delayMs?: number;
  /** Also expose the label as aria-describedby (when it adds info beyond the accessible name). */
  describe?: boolean;
  /** Long text: wrap at the tooltip's max width instead of one line. */
  wrap?: boolean;
}

/** Short hover delay before a tooltip appears; it then fades in (quick) and leaves instantly. */
export const TOOLTIP_DELAY_MS = 200;

/** Hover/focus tooltip. Wraps a single focusable child. */
export function Tooltip({ label, children, placement = "top", delayMs = TOOLTIP_DELAY_MS, describe = false, wrap = false }: TooltipProps) {
  const id = useId();
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const tip = useRef<HTMLDivElement>(null);
  // Horizontal nudge that keeps a wide tooltip inside the window.
  const [nudge, setNudge] = useState(0);
  const child = Children.only(children);
  const props = (isValidElement(child) ? child.props : {}) as AnyProps;

  const clear = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  };
  const showFor = (el: HTMLElement, wait: number) => {
    clear();
    timer.current = setTimeout(() => {
      const r = el.getBoundingClientRect();
      if (placement === "right") setPos({ x: r.right + 8, y: r.top + r.height / 2 });
      else
        setPos({
          x: Math.min(Math.max(r.left + r.width / 2, 72), window.innerWidth - 72),
          y: placement === "top" ? r.top - 8 : r.bottom + 8,
        });
    }, wait);
  };
  const hide = () => {
    clear();
    setPos(null);
  };

  useEffect(() => {
    if (!pos) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && hide();
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", hide, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", hide, true);
    };
  }, [pos]);
  useEffect(() => clear, []);
  useLayoutEffect(() => {
    setNudge(0);
    if (!pos || !tip.current) return;
    const r = tip.current.getBoundingClientRect();
    const edge = 8;
    if (r.right > window.innerWidth - edge) setNudge(window.innerWidth - edge - r.right);
    else if (r.left < edge) setNudge(edge - r.left);
  }, [pos]);

  const cloned = cloneElement(child as ReactElement<AnyProps>, {
    onMouseEnter: (e: MouseEvent<HTMLElement>) => {
      props.onMouseEnter?.(e);
      showFor(e.currentTarget, delayMs);
    },
    onMouseLeave: (e: MouseEvent<HTMLElement>) => {
      props.onMouseLeave?.(e);
      hide();
    },
    onFocus: (e: FocusEvent<HTMLElement>) => {
      props.onFocus?.(e);
      let visible = false;
      try {
        visible = e.currentTarget.matches(":focus-visible");
      } catch {
        visible = false;
      }
      if (visible) showFor(e.currentTarget, TOOLTIP_DELAY_MS);
    },
    onBlur: (e: FocusEvent<HTMLElement>) => {
      props.onBlur?.(e);
      hide();
    },
    onPointerDown: (e: unknown) => {
      props.onPointerDown?.(e);
      hide();
    },
    ...(describe ? { "aria-describedby": id } : {}),
  });

  return (
    <>
      {cloned}
      {pos &&
        createPortal(
          <div
            id={id}
            role="tooltip"
            ref={tip}
            className={`tooltip tooltip--${placement} ${wrap ? "tooltip--wrap" : ""}`}
            style={{ left: pos.x + nudge, top: pos.y }}
          >
            {label}
          </div>,
          document.body,
        )}
      {!pos && describe && (
        <span id={id} className="sr-only">
          {label}
        </span>
      )}
    </>
  );
}
