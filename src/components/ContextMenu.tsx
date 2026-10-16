import { Check, ChevronRight } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { useUi, type ContextMenuState, type MenuItem } from "../stores/ui";
import { MENU_ICON_SIZE, MENU_ICON_STROKE, menuIconFor } from "./menuIcons";

/** Global context menu host. Open with `useUi.getState().openMenu(...)`. */
export function ContextMenuHost() {
  const menu = useUi((s) => s.menu);
  const close = useUi((s) => s.closeMenu);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);
  // The open submenu (item index) and a counter that moves focus into it.
  const [sub, setSub] = useState<number | null>(null);
  const [subFocus, setSubFocus] = useState(0);
  const subFocused = useRef(0);
  // Focus that came from the arrow keys opens a submenu; the first focus on open doesn't.
  const keyNav = useRef(false);
  useEffect(() => {
    setSub(null);
    subFocused.current = 0;
    keyNav.current = false;
    setSubFocus(0);
  }, [menu]);

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

  // Only this menu's own items; a submenu handles its keys itself.
  const items = () =>
    Array.from(ref.current?.querySelectorAll<HTMLElement>(':scope > [role="none"] > [role^="menuitem"]:not([aria-disabled="true"])') ?? []);
  const onKeyDown = (e: KeyboardEvent) => {
    if ((e.target as Element).closest(".context-menu__sub")) return;
    const list = items();
    const i = list.indexOf(document.activeElement as HTMLElement);
    const go = (n: number) => {
      keyNav.current = true;
      list[(n + list.length) % list.length]?.focus();
    };
    const at = list[i];
    switch (e.key) {
      case "ArrowRight":
      case "Enter":
      case " ":
        if (at?.dataset.submenu !== undefined) {
          e.preventDefault();
          setSub(Number(at.dataset.submenu));
          setSubFocus((n) => n + 1);
        }
        break;
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

  const select = (item: MenuItem, i: number) => {
    if (item.disabled) return;
    if (item.submenu) {
      setSub(i);
      setSubFocus((n) => n + 1);
      return;
    }
    close();
    item.onSelect?.();
  };
  // Icons line up in a column: when any item has one, the rest keep the space.
  const icons = menu.items.map((item) => (item.icon === undefined ? menuIconFor(item.label) : item.icon));
  const iconColumn = icons.some(Boolean);
  const subItem = sub !== null ? menu.items[sub] : undefined;
  const subAnchor = () => ref.current?.querySelector<HTMLElement>(`[data-submenu="${sub}"]`) ?? null;
  const focusReq = { want: subFocus > subFocused.current, done: () => (subFocused.current = subFocus) };

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
            aria-haspopup={item.submenu ? "menu" : undefined}
            aria-expanded={item.submenu ? sub === i : undefined}
            data-submenu={item.submenu ? i : undefined}
            className={`context-menu__item ${item.description ? "context-menu__item--two" : ""} ${item.danger ? "is-danger" : ""} ${item.checked ? "is-checked" : ""} ${item.submenu && sub === i ? "is-open" : ""}`}
            onClick={() => select(item, i)}
            onMouseEnter={() => !item.disabled && setSub(item.submenu ? i : null)}
            onFocus={() => {
              if (!keyNav.current) return;
              setSub(item.submenu && !item.disabled ? i : null);
            }}
          >
            {iconColumn && <MenuIcon icon={icons[i]} />}
            {item.description ? (
              <span className="context-menu__text">
                <span className="truncate context-menu__label">{item.label}</span>
                <span className="truncate context-menu__desc">{item.description}</span>
              </span>
            ) : (
              <span className="truncate context-menu__label">{item.label}</span>
            )}
            {item.shortcut && <span className="context-menu__shortcut">{item.shortcut}</span>}
            {item.checked && <Check size={16} aria-hidden="true" className="context-menu__check" />}
            {item.submenu && <ChevronRight size={16} aria-hidden="true" className="context-menu__chevron" />}
          </button>
        </div>
      ))}
      {subItem?.submenu && pos && (
        <Submenu
          key={sub}
          anchor={subAnchor}
          focusTick={focusReq}
          label={subItem.label}
          render={subItem.submenu}
          close={close}
          back={() => {
            setSub(null);
            subAnchor()?.focus();
          }}
        />
      )}
    </div>,
    document.body,
  );
}

/** A menu item's leading icon, or an empty slot of the same width. */
export function MenuIcon({ icon: Icon }: { icon: import("lucide-react").LucideIcon | null | undefined }) {
  return (
    <span className="context-menu__icon" aria-hidden="true">
      {Icon && <Icon size={MENU_ICON_SIZE} strokeWidth={MENU_ICON_STROKE} />}
    </span>
  );
}

/**
 * A panel beside its menu item: to the right, overlapping a little, or to
 * the left when there's no room; moved up to stay on screen. Focus moves in
 * only when asked (Right arrow, Enter, click); hovering just shows it.
 */
function Submenu({
  anchor,
  focusTick,
  label,
  render,
  close,
  back,
}: {
  anchor: () => HTMLElement | null;
  /** Whether focus should move in now, and how to say it did. */
  focusTick: { want: boolean; done: () => void };
  label: string;
  render: NonNullable<MenuItem["submenu"]>;
  close: () => void;
  back: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);

  useLayoutEffect(() => {
    const a = anchor()?.getBoundingClientRect();
    const el = ref.current;
    if (!a || !el) return;
    const r = el.getBoundingClientRect();
    setAt(placeSubmenu(a, { width: r.width, height: r.height }, { width: window.innerWidth, height: window.innerHeight }));
    // Only once placed: a hidden panel can't take focus.
  }, [anchor]);

  // Focus in on request (Right arrow, Enter or click on the item).
  useEffect(() => {
    if (!at || !focusTick.want) return;
    ref.current?.querySelector<HTMLElement>("input, [role^='menuitem']:not([aria-disabled='true'])")?.focus();
    focusTick.done();
  }, [focusTick, at]);

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      e.nativeEvent.stopImmediatePropagation();
      close();
      return;
    }
    const t = e.target as HTMLElement;
    const caretAtStart = !(t instanceof HTMLInputElement) || (t.selectionStart ?? 0) === 0;
    if (e.key === "ArrowLeft" && caretAtStart) {
      e.preventDefault();
      back();
    }
  };

  return (
    <div
      ref={ref}
      className="context-menu context-menu__sub"
      role="menu"
      aria-label={label}
      style={{ left: at?.x ?? 0, top: at?.y ?? 0, visibility: at ? "visible" : "hidden" }}
      onKeyDown={onKeyDown}
      onMouseDown={(e) => e.stopPropagation()}
    >
      {render({ close, back })}
    </div>
  );
}

/** Where a submenu goes beside its item `a` (see Submenu). */
export function placeSubmenu(
  a: { left: number; right: number; top: number },
  size: { width: number; height: number },
  view: { width: number; height: number },
): { x: number; y: number } {
  const OVERLAP = 4;
  let x = a.right - OVERLAP;
  if (x + size.width > view.width - EDGE) x = a.left - size.width + OVERLAP;
  const y = Math.max(EDGE, Math.min(a.top - 4, view.height - size.height - EDGE));
  return { x: Math.max(EDGE, x), y };
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
  /** Where focus goes when the menu closes, when not the button or the focused element. */
  returnTo?: HTMLElement | null,
) {
  const shown = items.filter((i) => !i.when || i.when());
  if (at instanceof HTMLElement) {
    const r = at.getBoundingClientRect();
    const anchor = { left: r.left, right: r.right, top: r.top, bottom: r.bottom };
    useUi.getState().openMenu({ label, items: shown, x: r.left, y: r.bottom, anchor, returnFocus: returnTo ?? at });
  } else {
    const returnFocus = returnTo ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    useUi.getState().openMenu({ label, items: shown, x: at.clientX, y: at.clientY, returnFocus });
  }
}
