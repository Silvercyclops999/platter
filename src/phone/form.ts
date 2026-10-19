/**
 * Whether the app runs as a phone: the phone build says so through the
 * core's app info, and the browser mock takes `?phone=1` so the phone
 * layout can be worked on at a phone's width.
 */
import { create } from "zustand";
import { backend, isTauri } from "../lib/backend";

/** `?phone=1` in the address (the browser mock). */
export function phoneFromUrl(search: string): boolean {
  return new URLSearchParams(search).get("phone") === "1";
}

/** `?notch=1`: the browser mock draws a notch's and home indicator's safe areas. */
export function notchFromUrl(search: string): boolean {
  return new URLSearchParams(search).get("notch") === "1";
}

/** A phone's web view, before the core has answered. */
export function phoneFromAgent(ua: string): boolean {
  return /iPhone|iPod|Android.+Mobile/i.test(ua);
}

function initial(): boolean {
  if (typeof window === "undefined") return false;
  if (phoneFromUrl(window.location.search)) return true;
  return isTauri() && phoneFromAgent(navigator.userAgent);
}

export const usePhoneForm = create<{ phone: boolean }>()(() => ({ phone: initial() }));

/** Ask the core what kind of device this is (the phone build answers "phone"). */
export function checkFormWithCore(): Promise<void> {
  if (!isTauri()) return Promise.resolve();
  return backend.appInfo().then(
    (i) => {
      if (i.form) usePhoneForm.setState({ phone: i.form === "phone" });
    },
    () => {},
  );
}
