/**
 * The phone's pages as a stack. A page opened from another slides in from
 * the right over it while the one under shifts left and dims; Back reverses
 * that, and so does a drag from the left edge, which follows the finger.
 * A tab cross-fades to its first page, back where you left it scrolled.
 * Both pages stay mounted until the move ends, so there is never a blank
 * frame. React draws only at the start and the end of a move; the frames
 * in between belong to the Web Animations API.
 */
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useRouter, type HistoryEntry } from "../stores/router";
import {
  FADE_MS,
  PUSH_MS,
  TAB_FADE_MS,
  UNDER_SHIFT,
  EDGE_PX,
  edgeClaims,
  edgeRelease,
  reduced,
  release,
  run,
  stackSignals,
} from "./motion";
import { usePhoneUi } from "./phoneUi";

export type MoveKind = "push" | "back" | "tab";

interface Move {
  id: number;
  kind: MoveKind;
  from: HistoryEntry;
  to: HistoryEntry;
  /** The tab being left, for a tab switch. */
  fromTab?: string;
}

/** Where each page was scrolled, by history entry, and each tab's first page. */
const scrollByKey = new Map<number, number>();
const scrollByTab = new Map<string, number>();

/** Forget every remembered scroll position (tests). */
export function resetStackMemory() {
  scrollByKey.clear();
  scrollByTab.clear();
  stackSignals.tabSwitch = null;
  stackSignals.swiped = false;
}

/** What a change of page is, from the history before it. `null` means no movement. */
export function classify(prev: { entries: HistoryEntry[]; index: number }, to: HistoryEntry, signal: { tabSwitch: { from: string; to: string } | null; swiped: boolean }): MoveKind | null {
  if (signal.swiped) return null;
  if (signal.tabSwitch) return signal.tabSwitch.from !== signal.tabSwitch.to ? "tab" : prev.index > 0 ? "back" : null;
  const at = prev.entries.findIndex((e) => e.key === to.key);
  if (at >= 0) return at < prev.index ? "back" : "push";
  // A fresh history (signed out, a reset from elsewhere) just appears.
  return useRouter.getState().entries.length === 1 ? null : "push";
}

let moveSeq = 0;

export function PageStack({ render }: { render: (entry: HistoryEntry) => ReactNode }) {
  const entries = useRouter((s) => s.entries);
  const index = useRouter((s) => s.index);
  const current = entries[index];
  const [st, setSt] = useState<{ cur: HistoryEntry; move: Move | null; drag: HistoryEntry | null; scroll: Map<number, number> }>(() => ({
    cur: current,
    move: null,
    drag: null,
    scroll: new Map(),
  }));
  const seen = useRef({ entries, index });
  const host = useRef<HTMLDivElement>(null);
  const layers = useRef(new Map<number, HTMLDivElement>());
  const dim = useRef<HTMLDivElement>(null);

  // A new page: work out the move while rendering, so the first frame already has both pages.
  if (st.cur.key !== current.key) {
    const signal = { tabSwitch: stackSignals.tabSwitch, swiped: stackSignals.swiped };
    stackSignals.tabSwitch = null;
    stackSignals.swiped = false;
    const kind = classify(seen.current, current, signal);
    const scroll = new Map<number, number>();
    if (kind === "back") scroll.set(current.key, scrollByKey.get(current.key) ?? 0);
    if (kind === "tab" && signal.tabSwitch) scroll.set(current.key, scrollByTab.get(signal.tabSwitch.to) ?? 0);
    setSt({ cur: current, move: kind ? { id: ++moveSeq, kind, from: st.cur, to: current, fromTab: signal.tabSwitch?.from } : null, drag: null, scroll });
  } else if (st.cur !== current) {
    // The same page with a new route (a search query, a library filter).
    setSt({ ...st, cur: current });
  }

  // Remember where the page being left was scrolled, then run the move.
  const move = st.move;
  useLayoutEffect(() => {
    seen.current = { entries, index };
  });
  useLayoutEffect(() => {
    if (!move) return;
    const fromEl = layers.current.get(move.from.key);
    const toEl = layers.current.get(move.to.key);
    if (move.kind !== "back") remember(move.from, fromEl, seen.current, move.fromTab);
    const quiet = reduced();
    const anims: ReturnType<typeof run>[] = [];
    if (move.kind === "tab") anims.push(run(toEl, [{ opacity: 0 }, { opacity: 1 }], quiet ? FADE_MS : TAB_FADE_MS, { easing: "ease-out" }));
    else if (quiet) {
      if (move.kind === "push") anims.push(run(toEl, [{ opacity: 0 }, { opacity: 1 }], FADE_MS, { easing: "ease-out" }));
      else anims.push(run(fromEl, [{ opacity: 1 }, { opacity: 0 }], FADE_MS, { easing: "ease-out" }));
    } else if (move.kind === "push") {
      anims.push(run(toEl, [{ transform: "translateX(100%)" }, { transform: "none" }], PUSH_MS));
      anims.push(run(fromEl, [{ transform: "none" }, { transform: `translateX(${-UNDER_SHIFT * 100}%)` }], PUSH_MS));
      anims.push(run(dim.current, [{ opacity: 0 }, { opacity: 1 }], PUSH_MS));
    } else {
      anims.push(run(fromEl, [{ transform: "none" }, { transform: "translateX(100%)" }], PUSH_MS));
      anims.push(run(toEl, [{ transform: `translateX(${-UNDER_SHIFT * 100}%)` }, { transform: "none" }], PUSH_MS));
      anims.push(run(dim.current, [{ opacity: 1 }, { opacity: 0 }], PUSH_MS));
    }
    let live = true;
    void Promise.all(anims.map((a) => a.done)).then(() => {
      if (!live) return;
      // The page that stays lets go of its last frame (a held transform would trap fixed children).
      release(toEl);
      setSt((s) => (s.move?.id === move.id ? { ...s, move: null } : s));
    });
    return () => {
      live = false;
      anims.forEach((a) => a.cancel());
      release(toEl);
    };
  }, [move]);

  // Prune memory for pages that have left the history.
  useEffect(() => {
    const keys = new Set(entries.map((e) => e.key));
    for (const k of scrollByKey.keys()) if (!keys.has(k)) scrollByKey.delete(k);
  }, [entries]);

  // A tap on the tab you're on: its first page glides to the top.
  useEffect(() => {
    stackSignals.toTop = () => {
      const page = layers.current.get(useRouter.getState().entries[useRouter.getState().index].key)?.querySelector<HTMLElement>(".ph-page");
      if (!page) return;
      if (typeof page.scrollTo === "function") page.scrollTo({ top: 0, behavior: reduced() ? "auto" : "smooth" });
      else page.scrollTop = 0;
    };
    return () => {
      stackSignals.toTop = null;
    };
  }, []);

  const stValue = useRef(st);
  stValue.current = st;
  useEdgeSwipe(host, layers, dim, {
    canStart: () => !stValue.current.move && !stValue.current.drag && useRouter.getState().index > 0,
    begin: () => {
      const r = useRouter.getState();
      const under = r.entries[r.index - 1];
      const scroll = new Map([[under.key, scrollByKey.get(under.key) ?? 0]]);
      remember(r.entries[r.index], layers.current.get(r.entries[r.index].key), seen.current);
      setSt((s) => ({ ...s, drag: under, scroll }));
      return { top: r.entries[r.index].key, under: under.key };
    },
    finish: (back) => {
      if (back) {
        stackSignals.swiped = true;
        useRouter.getState().back();
      } else setSt((s) => ({ ...s, drag: null }));
    },
  });

  // Bottom to top: older pages under newer ones, which keeps the DOM order steady.
  const shown: { entry: HistoryEntry; role: "only" | "under" | "over" | "arriving" | "fading" | "leaving" }[] = [];
  if (move) {
    const fresh = (e: HistoryEntry) => (e.key === current.key ? current : e);
    if (move.kind === "push") shown.push({ entry: move.from, role: "under" }, { entry: fresh(move.to), role: "over" });
    else if (move.kind === "back") shown.push({ entry: fresh(move.to), role: "arriving" }, { entry: move.from, role: "leaving" });
    else shown.push({ entry: move.from, role: "under" }, { entry: fresh(move.to), role: "fading" });
    shown.sort((a, b) => a.entry.key - b.entry.key);
  } else if (st.drag) shown.push({ entry: st.drag, role: "under" }, { entry: current, role: "over" });
  else shown.push({ entry: current, role: "only" });
  const sliding = !!st.drag || (!!move && move.kind !== "tab" && !reduced());

  return (
    <div ref={host} className="ph-stack" data-testid="page-stack" data-moving={move ? move.kind : st.drag ? "drag" : undefined}>
      {shown.map(({ entry, role }) => {
        const hidden = role === "under" || role === "leaving";
        const cls = role === "over" || role === "leaving" ? (sliding ? "is-sliding" : "is-fading") : role === "fading" ? "is-fading" : role === "under" || role === "arriving" ? "is-under" : "";
        return (
          <Layer
            key={entry.key}
            entryKey={entry.key}
            className={cls}
            hidden={hidden}
            scroll={st.scroll.get(entry.key) ?? 0}
            refs={layers.current}
          >
            {render(entry)}
          </Layer>
        );
      })}
      {sliding && <div ref={dim} className="ph-stack-dim" aria-hidden="true" />}
    </div>
  );
}

/** Note where a page being left was scrolled; a tab's first page also counts for the tab. */
function remember(entry: HistoryEntry, el: HTMLElement | undefined, prev: { entries: HistoryEntry[]; index: number }, leavingTab?: string) {
  const page = el?.querySelector<HTMLElement>(".ph-page");
  if (!page) return;
  scrollByKey.set(entry.key, page.scrollTop);
  if (leavingTab) scrollByTab.set(leavingTab, page.scrollTop);
  else if (prev.entries[0]?.key === entry.key) scrollByTab.set(usePhoneUi.getState().tab, page.scrollTop);
}

function Layer({
  entryKey,
  className,
  hidden,
  scroll,
  refs,
  children,
}: {
  entryKey: number;
  className: string;
  hidden: boolean;
  scroll: number;
  refs: Map<number, HTMLDivElement>;
  children: ReactNode;
}) {
  const el = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const node = el.current;
    if (!node) return;
    refs.set(entryKey, node);
    return () => {
      if (refs.get(entryKey) === node) refs.delete(entryKey);
    };
  }, [entryKey, refs]);
  // Back where it was: at once if the page is drawn, or as its rows arrive.
  useLayoutEffect(() => (el.current && scroll > 0 ? restoreScroll(el.current, scroll) : undefined), []); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div ref={el} className={`ph-layer ${className}`} aria-hidden={hidden || undefined} inert={hidden || undefined}>
      {children}
    </div>
  );
}

/**
 * Scrolls a layer's page to `top`. A page still loading isn't tall enough
 * yet, so it tries again as its content arrives, for up to a second and a
 * half or until the person touches it. Returns a function that stops it.
 */
export function restoreScroll(layer: HTMLElement, top: number): () => void {
  const apply = () => {
    const page = layer.querySelector<HTMLElement>(".ph-page");
    if (!page) return false;
    page.scrollTop = top;
    return Math.abs(page.scrollTop - top) <= 1;
  };
  if (apply() || typeof MutationObserver === "undefined") return () => {};
  const mo = new MutationObserver(() => {
    if (apply()) stop();
  });
  mo.observe(layer, { childList: true, subtree: true });
  const timer = setTimeout(() => stop(), 1500);
  const stop = () => {
    mo.disconnect();
    clearTimeout(timer);
    layer.removeEventListener("pointerdown", stop, true);
    layer.removeEventListener("wheel", stop, true);
  };
  layer.addEventListener("pointerdown", stop, true);
  layer.addEventListener("wheel", stop, true);
  return stop;
}

interface Swipe {
  id: number;
  x0: number;
  y0: number;
  claimed: boolean | null;
  dx: number;
  vx: number;
  lastX: number;
  lastT: number;
  width: number;
  keys: { top: number; under: number } | null;
}

/**
 * The drag from the left edge. A press within EDGE_PX of the edge is
 * watched; once it heads right more than up or down it is claimed (and the
 * page stops scrolling), and the page follows the finger. Up or down first,
 * or leftwards (a shelf), it lets go and the page scrolls as usual.
 */
function useEdgeSwipe(
  host: React.RefObject<HTMLDivElement | null>,
  layers: React.MutableRefObject<Map<number, HTMLDivElement>>,
  dim: React.RefObject<HTMLDivElement | null>,
  api: { canStart: () => boolean; begin: () => { top: number; under: number }; finish: (back: boolean) => void },
) {
  const apiRef = useRef(api);
  apiRef.current = api;
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    let g: Swipe | null = null;
    let settling = false;
    const now = () => (typeof performance !== "undefined" ? performance.now() : Date.now());
    const parts = (s: Swipe) => (s.keys ? { top: layers.current.get(s.keys.top), under: layers.current.get(s.keys.under) } : { top: undefined, under: undefined });
    const place = (dx: number) => {
      if (!g) return;
      const { top, under } = parts(g);
      const p = Math.max(0, Math.min(1, dx / g.width));
      if (top) top.style.transform = `translateX(${dx}px)`;
      if (under) under.style.transform = `translateX(${-UNDER_SHIFT * g.width * (1 - p)}px)`;
      if (dim.current) dim.current.style.opacity = String(1 - p);
    };
    const down = (e: PointerEvent) => {
      if (settling || g) return;
      if (e.pointerType === "mouse" && e.button !== 0) return;
      const left = el.getBoundingClientRect().left;
      if (e.clientX - left > EDGE_PX || !apiRef.current.canStart()) return;
      g = { id: e.pointerId, x0: e.clientX, y0: e.clientY, claimed: null, dx: 0, vx: 0, lastX: e.clientX, lastT: now(), width: el.clientWidth || 1, keys: null };
    };
    const move = (e: PointerEvent) => {
      if (!g || e.pointerId !== g.id) return;
      const dx = e.clientX - g.x0;
      const dy = e.clientY - g.y0;
      if (g.claimed === null) {
        const c = edgeClaims(dx, dy);
        if (c === null) return;
        if (!c) {
          g = null;
          return;
        }
        g.claimed = true;
        el.setPointerCapture?.(e.pointerId);
        g.keys = apiRef.current.begin();
      }
      const t = now();
      const dt = Math.max(1, t - g.lastT);
      g.vx = (e.clientX - g.lastX) / dt;
      g.lastX = e.clientX;
      g.lastT = t;
      g.dx = Math.max(0, dx);
      // The layers draw a frame after the claim; the first moves land once they exist.
      place(g.dx);
    };
    const end = (e: PointerEvent, cancelled: boolean) => {
      if (!g || e.pointerId !== g.id) return;
      const s = g;
      g = null;
      if (!s.claimed) return;
      const outcome = cancelled ? "stay" : edgeRelease({ dx: s.dx, vx: s.vx, width: s.width });
      settle(s, outcome === "back");
    };
    const settle = (s: Swipe, back: boolean) => {
      const { top, under } = parts(s);
      settling = true;
      const left = back ? s.width - s.dx : s.dx;
      const speed = Math.max(Math.abs(s.vx), 0.9);
      const ms = reduced() ? 0 : Math.round(Math.min(PUSH_MS, Math.max(160, left / speed)));
      const p = Math.max(0, Math.min(1, s.dx / s.width));
      const anims = [
        run(top, [{ transform: `translateX(${s.dx}px)` }, { transform: back ? `translateX(${s.width}px)` : "none" }], ms),
        run(under, [{ transform: `translateX(${-UNDER_SHIFT * s.width * (1 - p)}px)` }, { transform: back ? "none" : `translateX(${-UNDER_SHIFT * s.width}px)` }], ms),
        run(dim.current, [{ opacity: 1 - p }, { opacity: back ? 0 : 1 }], ms),
      ];
      void Promise.all(anims.map((a) => a.done)).then(() => {
        settling = false;
        if (back) {
          if (top) top.style.transform = `translateX(${s.width}px)`;
          if (under) under.style.transform = "";
        } else if (top) top.style.transform = "";
        release(under);
        if (!back) release(top);
        if (dim.current) dim.current.style.opacity = "";
        apiRef.current.finish(back);
      });
    };
    // Once a press from the edge heads right, the page under it mustn't scroll.
    const touchMove = (e: TouchEvent) => {
      if (!g || !e.cancelable) return;
      const t = e.touches[0];
      if (!t) return;
      const dx = t.clientX - g.x0;
      const dy = t.clientY - g.y0;
      if (g.claimed || (dx > 0 && dx >= Math.abs(dy))) e.preventDefault();
    };
    const up = (e: PointerEvent) => end(e, false);
    const cancel = (e: PointerEvent) => end(e, true);
    el.addEventListener("pointerdown", down, true);
    el.addEventListener("pointermove", move, true);
    el.addEventListener("pointerup", up, true);
    el.addEventListener("pointercancel", cancel, true);
    el.addEventListener("touchmove", touchMove, { passive: false, capture: true });
    return () => {
      el.removeEventListener("pointerdown", down, true);
      el.removeEventListener("pointermove", move, true);
      el.removeEventListener("pointerup", up, true);
      el.removeEventListener("pointercancel", cancel, true);
      el.removeEventListener("touchmove", touchMove, { capture: true } as EventListenerOptions);
    };
  }, [host, layers, dim]);
}
