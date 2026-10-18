import { useId } from "react";

/**
 * useId led by a letter, for ids that labels point at (aria-labelledby).
 * React's own ids start with "_", and some DOMs can't look those up by id
 * quickly, so a page with hundreds of labelled rows gets slow to query.
 */
export function useDomId(): string {
  return `s${useId()}`;
}
