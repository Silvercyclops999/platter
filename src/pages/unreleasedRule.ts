/**
 * "Share unreleased music": one rule for demos, leaks and other unreleased
 * songs leaving this computer. Choosing it sets each feature's own switch;
 * those switches stay under Advanced as exceptions. The core gives settings
 * saved before the rule the one their switches already make.
 */
import type { Settings, UnreleasedSharing } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";

type Key =
  | "discordShowUnreleased"
  | "discordUnreleasedArt"
  | "mixUnreleased"
  | "scrobbleUnreleased"
  | "listenHideUnreleased"
  | "shareUnreleased"
  | "shareTranscriptionsUnreleased";

/** Each feature's switch, and whether on means it shares. */
export const UNRELEASED_OVERRIDES: readonly { key: Key; onShares: boolean }[] = [
  { key: "discordShowUnreleased", onShares: true },
  { key: "discordUnreleasedArt", onShares: true },
  { key: "mixUnreleased", onShares: true },
  { key: "scrobbleUnreleased", onShares: true },
  { key: "listenHideUnreleased", onShares: false },
  { key: "shareUnreleased", onShares: true },
  { key: "shareTranscriptionsUnreleased", onShares: true },
];

export const UNRELEASED_RULE_OPTIONS: readonly { value: UnreleasedSharing; label: string }[] = [
  { value: "never", label: "Never" },
  { value: "ask", label: "Ask each time" },
  { value: "always", label: "Always" },
];

/** What choosing a rule writes: Never and Always set every switch; Ask puts each back to how it starts. */
export function patchForRule(rule: UnreleasedSharing): Partial<Settings> {
  const p: Partial<Settings> = { unreleasedSharing: rule };
  for (const o of UNRELEASED_OVERRIDES) {
    const v = rule === "ask" ? DEFAULT_SETTINGS[o.key] : rule === "always" ? o.onShares : !o.onShares;
    (p as Record<string, unknown>)[o.key] = v;
  }
  return p;
}

/** The switches that differ from what the rule sets (shown as exceptions). */
export function exceptionsTo(s: Settings): Key[] {
  const want = patchForRule(s.unreleasedSharing ?? "ask");
  return UNRELEASED_OVERRIDES.filter((o) => s[o.key] !== want[o.key]).map((o) => o.key);
}
