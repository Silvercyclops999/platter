import { useFullWindow } from "../stores/fullWindow";
import { useLayout } from "../stores/theme";

/**
 * True while Deck shows lyrics as its page: the lyrics button and the
 * lyrics route open the page under the faceplate, and the expanded view
 * stays shut.
 */
export function useDeckLyrics(): boolean {
  const frame = useLayout().frame;
  return useFullWindow((s) => frame === "deck" && s.open && s.mode === "lyrics");
}
