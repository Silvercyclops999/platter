/**
 * Put `key` in a cache that keeps at most `cap` entries, dropping the
 * oldest first, so long sessions don't grow memory without end.
 */
export function setBounded<K, V>(map: Map<K, V>, key: K, value: V, cap: number): void {
  map.delete(key);
  map.set(key, value);
  while (map.size > cap) {
    const oldest = map.keys().next();
    if (oldest.done) break;
    map.delete(oldest.value);
  }
}
