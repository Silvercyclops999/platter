/**
 * "What's new" after an update: the features each version brought, each
 * linked to its help page where one exists (and its setup guide, when
 * guides are on). Newest version first.
 */
import type { GuideId } from "../stores/guides";
import type { HelpRef } from "../pages/settingsMeta";

export interface NewThing {
  text: string;
  help?: HelpRef;
  guide?: GuideId;
}

export interface Release {
  version: string;
  items: NewThing[];
}

export const RELEASES: Release[] = [
  {
    version: "0.2.21",
    items: [
      { text: "Help is built in. Open it from the app menu, search it with the Help chip, or press a \"?\" next to busier screens. It works offline.", help: ["getting-started", "getting-help"] },
      { text: "Setup guides walk you through Lidarr, library import, inviting a friend and Stream Deck, one step at a time.", help: ["getting-started", "getting-help"] },
      { text: "Settings are tidier: rarely needed options sit under Advanced in each section, and each section can be reset." },
      { text: "Export your settings to a file and import them on another computer. Keys stay in the keychain." },
    ],
  },
];

/** -1, 0 or 1, comparing dotted version numbers. */
export function compareVersions(a: string, b: string): number {
  const pa = a.split(/[.-]/).map((x) => Number.parseInt(x, 10) || 0);
  const pb = b.split(/[.-]/).map((x) => Number.parseInt(x, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d < 0 ? -1 : 1;
  }
  return 0;
}

/** What's new since `seen`, up to `current`. Nothing on a first run (no `seen`). */
export function newSince(seen: string | null, current: string, releases: Release[] = RELEASES): Release[] {
  if (!seen) return [];
  return releases.filter((r) => compareVersions(r.version, seen) > 0 && compareVersions(r.version, current) <= 0);
}

const KEY = "platter.seenVersion.v1"; // gitleaks:allow (localStorage key name, not a secret)

export function seenVersion(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

export function setSeenVersion(v: string) {
  try {
    localStorage.setItem(KEY, v);
  } catch {
    /* storage unavailable */
  }
}
