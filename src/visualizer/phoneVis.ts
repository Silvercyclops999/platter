/**
 * Whether the phone's now playing shows the visualizer in place of the
 * cover. Remembered on this device, like the expanded view's mode.
 */
import { create } from "zustand";

const KEY = "platter.phoneVisualizer.v1"; // gitleaks:allow (localStorage key name, not a secret)

function load(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

export const usePhoneVis = create<{ open: boolean }>()(() => ({ open: load() }));

export function setPhoneVis(open: boolean) {
  usePhoneVis.setState({ open });
  try {
    localStorage.setItem(KEY, open ? "1" : "0");
  } catch {
    /* storage unavailable: this session only */
  }
}

export const togglePhoneVis = () => setPhoneVis(!usePhoneVis.getState().open);
