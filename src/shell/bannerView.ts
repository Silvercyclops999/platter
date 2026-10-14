/**
 * What the banner at the top of the main view says for each app state. The
 * core decides the state and the wording (Health.banner); this maps it to a
 * tone and an action button, keeps "Last seen N min ago" current, and shows
 * library scan progress when nothing else needs saying.
 */
import type { BannerKind } from "../components/Banner";
import type { Health } from "../lib/types";

export type BannerAction = "signIn" | "signOut" | "libraryGuide";

export interface BannerView {
  kind: BannerKind;
  message: string;
  action?: BannerAction;
}

/** "just now", "4 min ago", "3 h ago", "2 days ago" for a duration in ms. */
export function agoText(ms: number): string {
  const mins = Math.floor(Math.max(0, ms) / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins} min ago`;
  if (mins < 48 * 60) return `${Math.floor(mins / 60)} h ago`;
  return `${Math.floor(mins / (24 * 60))} days ago`;
}

const TONE: Record<string, BannerKind> = {
  offline: "offline",
  noInternet: "offline",
  starting: "progress",
  updating: "progress",
  startTimeout: "error",
  relay: "warning",
  signIn: "warning",
  accessRemoved: "error",
  filesUnavailable: "error",
  noMusicLibrary: "info",
  diskFull: "warning",
};

const ACTION: Record<string, BannerAction> = {
  signIn: "signIn",
  accessRemoved: "signOut",
  noMusicLibrary: "libraryGuide",
};

export function healthBanner(h: Health | null, now: number = Date.now()): BannerView | null {
  if (!h) return null;
  const b = h.banner;
  if (b) {
    let message = b.message;
    // The core's text is as of when it was sent; keep the age current.
    if (b.kind === "offline" && h.lastSeenAt) {
      message = `Your server is offline. Last seen ${agoText(now - h.lastSeenAt)}.`;
    }
    return { kind: TONE[b.kind] ?? "info", message, action: ACTION[b.kind] };
  }
  if (h.scan && h.scan.total > 0) {
    return {
      kind: "progress",
      message: `Adding new music… ${h.scan.done.toLocaleString()} of ${h.scan.total.toLocaleString()}`,
    };
  }
  return null;
}
