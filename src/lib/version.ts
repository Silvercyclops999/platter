/**
 * Comparing release versions like "0.2.31". A leading "v" is allowed, and
 * anything after "-" or "+" (a pre-release or build tag) is ignored, so
 * "0.2.31-beta.2" counts as 0.2.31. Missing parts count as 0.
 */

/** [major, minor, patch], or null when it isn't a version. */
export function parseVersion(v: string | null | undefined): [number, number, number] | null {
  const core = (v ?? "").trim().replace(/^v/i, "").split(/[-+]/)[0];
  const m = /^(\d+)(?:\.(\d+))?(?:\.(\d+))?$/.exec(core);
  if (!m) return null;
  return [Number(m[1]), Number(m[2] ?? 0), Number(m[3] ?? 0)];
}

/** -1, 0 or 1 comparing `a` with `b`; null when either isn't a version. */
export function compareSemver(a: string | null | undefined, b: string | null | undefined): -1 | 0 | 1 | null {
  const pa = parseVersion(a);
  const pb = parseVersion(b);
  if (!pa || !pb) return null;
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] < pb[i] ? -1 : 1;
  }
  return 0;
}
