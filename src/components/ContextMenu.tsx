import { Check } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { useUi, type ContextMenuState, type MenuItem } from "../stores/ui";

/** Global context menu host. Open with `useUi.getState().openMenu(...)`. */
export function ContextMenuHost() {
  const menu = useUi((s) => s.menu);
  const close = useUi((s) => s.closeMenu);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);

  // Place once measured (before paint, so it opens fully formed in one frame).
  useLayoutEffect(() => {
    if (!menu || !ref.current) {
      setPos(null);
      return;
    }
    const r = ref.current.getBoundingClientRect();
    setPos(placeMenu(menu, { width: r.width, height: r.height }, { width: window.innerWidth, height: window.innerHeight }));
  }, [menu]);

  // Focus the first item once the menu is visible (hidden elements can't take focus).
  useEffect(() => {
    if (!menu || !pos) return;
    ref.current?.querySelector<HTMLElement>('[role^="menuitem"]:not([aria-disabled="true"])')?.focus();
  }, [menu, pos]);

  useEffect(() => {
    if (!menu) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    const onKey = (e: globalThis.KeyboardEvent) => {
      // Escape closes even if focus somehow left the menu.
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    const onBlur = () => close();
    const onResize = () => close();
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("resize", onResize);
    };
  }, [menu, close]);

  if (!menu) return null;

  const items = () => Array.from(ref.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]:not([aria-disabled="true"])') ?? []);
  const onKeyDown = (e: KeyboardEvent) => {
    const list = items();
    const i = list.indexOf(document.activeElement as HTMLElement);
    const go = (n: number) => list[(n + list.length) % list.length]?.focus();
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        go(i + 1);
        break;
      case "ArrowUp":
        e.preventDefault();
        go(i - 1);
        break;
      case "Home":
        e.preventDefault();
        go(0);
        break;
      case "End":
        e.preventDefault();
        go(list.length - 1);
        break;
      case "Escape":
      case "Tab":
        e.preventDefault();
        e.nativeEvent.stopImmediatePropagation();
        close();
        break;
    }
  };

  const select = (item: MenuItem) => {
    if (item.disabled) return;
    close();
    item.onSelect?.();
  };

  return createPortal(
    <div
      ref={ref}
      role="menu"
      aria-label={menu.label}
      className="context-menu"
      style={{ left: pos?.x ?? menu.x, top: pos?.y ?? menu.y, visibility: pos ? "visible" : "hidden" }}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      {menu.items.map((item, i) => (
        <div key={i} role="none">
          {item.separator && <div role="separator" className="context-menu__sep" />}
          {item.heading && (
            <div className="context-menu__heading" aria-hidden="true">
              {item.heading}
            </div>
          )}
          <button
            type="button"
            role={item.checked === undefined ? "menuitem" : "menuitemradio"}
            aria-checked={item.checked}
            tabIndex={-1}
            aria-disabled={item.disabled || undefined}
            className={`context-menu__item ${item.danger ? "is-danger" : ""} ${item.checked ? "is-checked" : ""}`}
            onClick={() => select(item)}
          >
            <span className="truncate">{item.label}</span>
            {item.shortcut && <span className="context-menu__shortcut">{item.shortcut}</span>}
            {item.checked && <Check size={16} aria-hidden="true" className="context-menu__check" />}
          </button>
        </div>
      ))}
    </div>,
    document.body,
  );
}

const EDGE = 8;
const GAP = 4;

/**
 * Where a menu of `size` goes in a `view`. At a pointer: top-left corner at
 * the pointer, flipped up (bottom edge at the pointer) when it would run off
 * the bottom. At an element (an options button, or a row from the keyboard):
 * just below it, aligned to it: a button in the right half of the window
 * lines up with the menu's right edge, otherwise the left edges line up.
 * Flipped above the element when there's no room below. Always kept on screen.
 */
export function placeMenu(
  menu: Pick<ContextMenuState, "x" | "y" | "anchor">,
  size: { width: number; height: number },
  view: { width: number; height: number },
): { x: number; y: number } {
  let x: number;
  let y: number;
  const a = menu.anchor;
  if (a) {
    const width = a.right - a.left;
    const rightHalf = a.left + width / 2 > view.width / 2;
    x = width < size.width && rightHalf ? a.right - size.width : a.left;
    y = a.bottom + GAP;
    if (y + size.height > view.height - EDGE) y = a.top - GAP - size.height;
  } else {
    x = menu.x;
    y = menu.y;
    if (y + size.height > view.height - EDGE) y = menu.y - size.height;
  }
  x = Math.max(EDGE, Math.min(x, view.width - size.width - EDGE));
  y = Math.max(EDGE, Math.min(y, view.height - size.height - EDGE));
  return { x, y };
}

/**
 * Open a menu at a mouse event (pointer) or aligned to an element (options
 * button or keyboard). Items whose `when` says no are left out.
 */
export function openContextMenu(
  label: string,
  items: MenuItem[],
  at: { clientX: number; clientY: number } | HTMLElement,
) {
  const shown = items.filter((i) => !i.when || i.when());
  if (at instanceof HTMLElement) {
    const r = at.getBoundingClientRect();
    const anchor = { left: r.left, right: r.right, top: r.top, bottom: r.bottom };
    useUi.getState().openMenu({ label, items: shown, x: r.left, y: r.bottom, anchor, returnFocus: at });
  } else {
    const returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    useUi.getState().openMenu({ label, items: shown, x: at.clientX, y: at.clientY, returnFocus });
  }
}
