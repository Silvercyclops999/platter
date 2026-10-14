/** Shared behaviour for album, artist and playlist pages. */
import { useEffect, useLayoutEffect, useRef, type CSSProperties } from "react";
import { ART_TRANSITION } from "../lib/viewTransition";
import { useUi } from "../stores/ui";

/** Header art takes part in the card-to-page art transition. */
export const headerArtStyle: CSSProperties = { viewTransitionName: ART_TRANSITION };

/** Largest size for which `fits` says the title fits; the smallest otherwise. */
export function pickFitSize(sizes: number[], fits: (size: number) => boolean): number {
  for (const s of sizes) if (fits(s)) return s;
  return sizes[sizes.length - 1];
}

/**
 * Big page titles step down through the type scale until they fit in two
 * lines of the header, re-measured when the window resizes.
 */
export function useFitTitle<T extends HTMLElement>(text: string | undefined) {
  const ref = useRef<T>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || !text) return;
    const fit = () => {
      const sizes = window.innerWidth < 1100 ? [32, 24, 20] : [48, 32, 24];
      const size = pickFitSize(sizes, (s) => {
        el.style.fontSize = `${s}px`;
        return el.scrollHeight <= el.clientHeight + 1 && el.scrollWidth <= el.clientWidth + 1;
      });
      el.style.fontSize = `${size}px`;
    };
    fit();
    window.addEventListener("resize", fit);
    return () => window.removeEventListener("resize", fit);
  }, [text]);
  return ref;
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
