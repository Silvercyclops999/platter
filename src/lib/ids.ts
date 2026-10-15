/**
 * Item ids from the core are opaque strings. Plex items carry their server
 * as a prefix (`<8 hex>:<rating key>`, see docs/IPC.md "Ids"). Ids this UI
 * saved before that (custom order, recents in localStorage) are the plain
 * rating key; `idVariants` lets those keep matching.
 */
const TAGGED = /^[0-9a-f]{8}:(.+)$/;

/** The id without its server tag (the Plex rating key), or the id itself. */
export function plainId(id: string): string {
  return TAGGED.exec(id)?.[1] ?? id;
}

/** The server tag of an id, or null. */
export function serverTagOf(id: string): string | null {
  return TAGGED.test(id) ? id.slice(0, 8) : null;
}

/** The id, then its plain form when it has a tag: look-ups try both. */
export function idVariants(id: string): string[] {
  const p = plainId(id);
  return p === id ? [id] : [id, p];
}
