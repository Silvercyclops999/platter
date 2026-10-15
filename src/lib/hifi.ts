/** Words for bit-perfect output, shared by Settings, the player bar and the device picker. */
import type { Exclusive } from "./types";

/** "96 kHz", "44.1 kHz". */
export function formatRate(rate: number): string {
  return rate % 1000 === 0 ? `${rate / 1000} kHz` : `${(rate / 1000).toFixed(1)} kHz`;
}

/** One line saying how the output holds its device, or null when it's plain shared. */
export function exclusiveText(x: Exclusive | undefined): string | null {
  if (!x || x.state === "off") return null;
  if (x.state === "on") return `Bit-perfect at ${formatRate(x.rate)}, ${x.bits}-bit. Volume and sound settings are bypassed.`;
  return `Playing shared for now: ${x.reason}`;
}
