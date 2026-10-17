/** Error wording shared by toasts and pages. */

/** Longest server or core message shown in a toast before it's cut. */
export const TOAST_REASON_MAX = 80;

/** The message an error carries, or "" when it has none. */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e.trim();
  return e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message).trim() : "";
}

/** A message cut to `max` characters at a word break, with an ellipsis. */
export function capReason(msg: string, max = TOAST_REASON_MAX): string {
  const one = msg.replace(/\s+/g, " ").trim();
  if (one.length <= max) return one;
  const cut = one.slice(0, max - 1);
  const at = cut.lastIndexOf(" ");
  return `${(at > max / 2 ? cut.slice(0, at) : cut).replace(/[\s.,;:]+$/, "")}…`;
}

/**
 * A toast line: what failed, then the error's own reason (shortened) when it
 * has one. "Couldn't save the playlist: The server is offline".
 */
export function failText(what: string, e: unknown): string {
  const m = errorMessage(e);
  return m ? `${what}: ${capReason(m)}` : what;
}
