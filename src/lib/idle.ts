/**
 * Run `fn` once the first screen has painted and the page is idle (at most
 * `timeoutMs` later), so start-up work that nothing on screen needs yet
 * doesn't compete with the first paint. Returns a cancel function.
 */
export function afterFirstPaint(fn: () => void, timeoutMs = 2000): () => void {
  const w = typeof window !== "undefined" ? window : undefined;
  if (w && typeof w.requestIdleCallback === "function") {
    const id = w.requestIdleCallback(fn, { timeout: timeoutMs });
    return () => w.cancelIdleCallback(id);
  }
  const t = setTimeout(fn, 0);
  return () => clearTimeout(t);
}
