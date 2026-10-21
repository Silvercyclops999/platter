/**
 * What the time after the seek bar shows: the time left ("-1:42", as it
 * always has) or the song's length. Clicking it switches; the choice is
 * kept on this device (localStorage), with no settings row.
 */
import { create } from "zustand";

export type SeekEnd = "left" | "length";

const KEY = "platter.seekEnd.v1"; // gitleaks:allow (localStorage key name, not a secret)

function load(): SeekEnd {
  try {
    return localStorage.getItem(KEY) === "length" ? "length" : "left";
  } catch {
    return "left";
  }
}

export const useSeekEnd = create<{ end: SeekEnd }>()(() => ({ end: load() }));

export function toggleSeekEnd() {
  const end: SeekEnd = useSeekEnd.getState().end === "left" ? "length" : "left";
  useSeekEnd.setState({ end });
  try {
    localStorage.setItem(KEY, end);
  } catch {
    // Private windows can refuse storage; the switch still holds for now.
  }
}

/** For tests. */
export function resetSeekEnd() {
  useSeekEnd.setState({ end: load() });
}
