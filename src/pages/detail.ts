/** Shared behaviour for album, artist and playlist pages. */
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { displaySize, font } from "../design/tokens";
import { useUi } from "../stores/ui";

/** Header title sizes in px, step 1 (largest) to step 5. */
export const TITLE_STEPS: readonly number[] = displaySize;

/** In the compact header the title starts at this step. */
export const COMPACT_FIRST_STEP = 4;

/** Below this main-column width the header uses its compact variant. Keep in step with pages.css. */
export const COMPACT_HEADER_BELOW = 760;

/**
 * Pick the title step (1 = largest): the largest step at which the title fits
 * on one line in `available` px. Text width scales with font size, so the
 * title is measured once as `widthPerPx` (its width at a 1px font size).
 * Nothing fits: the smallest step, where the title wraps freely.
 */
export function chooseTitleStep(
  widthPerPx: number,
  available: number,
  compact = false,
  steps: readonly number[] = TITLE_STEPS,
): number {
  const first = compact ? Math.min(COMPACT_FIRST_STEP, steps.length) : 1;
  for (let step = first; step <= steps.length; step++) {
    if (widthPerPx * steps[step - 1] <= available) return step;
  }
  return steps.length;
}

let measureCtx: CanvasRenderingContext2D | null | undefined;
const widthCache = new Map<string, number>();
/** Size the canvas measures at; widths are divided back down to 1px. */
const MEASURE_PX = 100;
/** Title letter-spacing in em (matches .detail-header__title). */
const TITLE_TRACKING = -0.02;

/**
 * Width of `text` per px of font size, in the extra-bold title face. Uses a
 * shared offscreen canvas (no DOM layout). Falls back to an estimate where
 * canvas text isn't available (tests).
 */
export function measureTitle(text: string): number {
  const hit = widthCache.get(text);
  if (hit !== undefined) return hit;
  if (measureCtx === undefined) {
    try {
      measureCtx = document.createElement("canvas").getContext("2d");
    } catch {
      measureCtx = null;
    }
  }
  let perPx: number;
  if (measureCtx && typeof measureCtx.measureText === "function") {
    measureCtx.font = `800 ${MEASURE_PX}px ${font.family}`;
    const raw = measureCtx.measureText(text).width;
    perPx = (raw + TITLE_TRACKING * MEASURE_PX * Array.from(text).length) / MEASURE_PX;
  } else {
    perPx = Array.from(text).length * 0.6;
  }
  widthCache.set(text, perPx);
  return perPx;
}

/** Forget cached widths (fonts finished loading). */
function clearTitleWidths() {
  widthCache.clear();
}

/**
 * Header title sizing. Observes the text column's width (ResizeObserver, so
 * no layout reads of our own) and picks the largest display step at which the
 * title fits on one line. Returns refs for the header and text column, the
 * chosen step and whether the compact variant is on.
 */
export function useTitleStep(text: string | undefined) {
  const header = useRef<HTMLElement>(null);
  const column = useRef<HTMLDivElement>(null);
  const [state, setState] = useState<{ step: number; compact: boolean }>({ step: 1, compact: false });

  useLayoutEffect(() => {
    const col = column.current;
    const head = header.current;
    if (!col || !head || !text) return;
    let colWidth = col.clientWidth;
    let headWidth = head.clientWidth;
    const update = () => {
      if (!colWidth) return;
      const compact = headWidth > 0 && headWidth < COMPACT_HEADER_BELOW;
      const step = chooseTitleStep(measureTitle(text), colWidth - 2, compact);
      setState((s) => (s.step === step && s.compact === compact ? s : { step, compact }));
    };
    update();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) {
        if (e.target === col) colWidth = e.contentRect.width;
        else headWidth = e.borderBoxSize?.[0]?.inlineSize ?? e.contentRect.width;
      }
      update();
    });
    ro.observe(col);
    ro.observe(head);
    // Widths measured before the web font arrived are off; measure again.
    let live = true;
    document.fonts?.ready.then(() => {
      if (!live) return;
      clearTitleWidths();
      update();
    });
    return () => {
      live = false;
      ro.disconnect();
    };
  }, [text]);

  return { header, column, step: state.step, compact: state.compact };
}

/** Publish the page title and play state for the sticky header in MainView. */
export function usePageBar(title: string | undefined, playing: boolean, onPlay: () => void) {
  const play = useRef(onPlay);
  play.current = onPlay;
  useEffect(() => {
    if (!title) return;
    useUi.getState().setPageBar({ title, playing, onPlay: () => play.current() });
  }, [title, playing]);
  useEffect(() => () => useUi.getState().setPageBar(null), []);
}
