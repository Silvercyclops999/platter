import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { useUi, type MenuItem } from "../stores/ui";

/** Global context menu host. Open with `useUi.getState().openMenu(...)`. */
export function ContextMenuHost() {
  const menu = useUi((s) => s.menu);
  const close = useUi((s) => s.closeMenu);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);

  // Clamp into the viewport once measured.
  useLayoutEffect(() => {
    if (!menu || !ref.current) {
      setPos(null);
      return;
    }
    const r = ref.current.getBoundingClientRect();
    const x = Math.min(menu.x, window.innerWidth - r.width - 8);
    const y = menu.y + r.height > window.innerHeight - 8 ? Math.max(8, menu.y - r.height) : menu.y;
    setPos({ x: Math.max(8, x), y });
  }, [menu]);

  // Focus the first item once the menu is visible (hidden elements can't take focus).
  useEffect(() => {
    if (!menu || !pos) return;
    ref.current?.querySelector<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])')?.focus();
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

  const items = () => Array.from(ref.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])') ?? []);
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
          <button
            type="button"
            role="menuitem"
            tabIndex={-1}
            aria-disabled={item.disabled || undefined}
            className={`context-menu__item ${item.danger ? "is-danger" : ""}`}
            onClick={() => select(item)}
          >
            <span className="truncate">{item.label}</span>
            {item.shortcut && <span className="context-menu__shortcut">{item.shortcut}</span>}
          </button>
        </div>
      ))}
    </div>,
    document.body,
  );
}

/** Open a menu anchored at a mouse event or below an element (keyboard). */
export function openContextMenu(
  label: string,
  items: MenuItem[],
  at: { clientX: number; clientY: number } | HTMLElement,
) {
  let x: number;
  let y: number;
  let returnFocus: HTMLElement | null = null;
  if (at instanceof HTMLElement) {
    const r = at.getBoundingClientRect();
    x = r.left + Math.min(r.width, 48);
    y = r.bottom;
    returnFocus = at;
  } else {
    x = at.clientX;
    y = at.clientY;
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  }
  useUi.getState().openMenu({ label, items, x, y, returnFocus });
}
