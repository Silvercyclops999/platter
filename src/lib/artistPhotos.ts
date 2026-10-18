/**
 * Photos of artists outside the library, asked for as their card or row
 * comes into view, a few at a time, and kept for the session (the core
 * keeps them on disk; the image itself goes through the art cache).
 */
import { useEffect, useState, type RefObject } from "react";
import { backend } from "./backend";
import type { CataloguePhoto } from "./types";

/** Most photo look-ups at once. */
export const PHOTO_PARALLEL = 3;
/** How far outside the view a card starts asking. */
const NEAR = "300px";

const known = new Map<string, CataloguePhoto>();
const waiting: (() => void)[] = [];
let running = 0;

function next() {
  while (running < PHOTO_PARALLEL && waiting.length) {
    running += 1;
    waiting.shift()!();
  }
}

/** One look-up, queued behind the others. `key` names it in the session's cache. */
function ask(key: string, look: () => Promise<CataloguePhoto>): Promise<CataloguePhoto> {
  const hit = known.get(key);
  if (hit) return Promise.resolve(hit);
  return new Promise((resolve) => {
    waiting.push(() => {
      look()
        .then(
          (p) => {
            // Only real photos are kept: an empty answer (offline, a
            // source down) is asked again next time.
            if (p.photo) known.set(key, p);
            resolve(p);
          },
          () => resolve({ photo: null, credit: null }),
        )
        .finally(() => {
          running -= 1;
          next();
        });
    });
    next();
  });
}

/** Forget the session's photos (tests). */
export function resetArtistPhotos() {
  known.clear();
}

/**
 * True once the element has come near the view. Without IntersectionObserver
 * there's no telling, so it's `withoutObserver` (near at once by default).
 */
export function useNearView(ref: RefObject<HTMLElement | null>, enabled = true, withoutObserver = true): boolean {
  const [near, setNear] = useState(() => withoutObserver && typeof IntersectionObserver === "undefined");
  useEffect(() => {
    const el = ref.current;
    if (near || !enabled || !el || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver((entries) => entries.some((e) => e.isIntersecting) && setNear(true), { rootMargin: NEAR });
    io.observe(el);
    return () => io.disconnect();
  }, [ref, near, enabled]);
  return near;
}

/**
 * The photo to show for a catalogue artist: the one they came with, else
 * one looked up once the element is near the view. Null while unknown and
 * when no source has one (the card shows a monogram then).
 */
export function useArtistPhoto(ref: RefObject<HTMLElement | null>, mbid: string | null | undefined, name: string, given: string | null | undefined): CataloguePhoto {
  const want = !given && !!mbid;
  const near = useNearView(ref, want);
  // Kept with the artist it was found for, so a reused card never shows the last artist's photo.
  const [found, setFound] = useState<{ key: string; p: CataloguePhoto } | null>(null);
  useEffect(() => {
    if (!want || !near || !mbid) return;
    let alive = true;
    void ask(mbid, () => backend.catalogueArtistPhoto(mbid, name)).then((p) => alive && setFound({ key: mbid, p }));
    return () => {
      alive = false;
    };
  }, [want, near, mbid, name]);
  if (given) return { photo: given, credit: null };
  const hit = mbid ? (found?.key === mbid ? found.p : known.get(mbid)) : undefined;
  return hit ?? { photo: null, credit: null };
}

/**
 * The photo for a library (or featured) artist the server has none for:
 * the catalogue's, looked up by name once the element is near the view.
 * Null while unknown, offline, and when no source has one.
 */
export function useFeaturedPhoto(ref: RefObject<HTMLElement | null>, id: string | null | undefined, name: string, given: string | null | undefined): string | null {
  const key = id ? `artist:${id}` : null;
  const want = !given && !!id && !!name;
  const near = useNearView(ref, want);
  const [found, setFound] = useState<{ key: string; photo: string | null } | null>(null);
  useEffect(() => {
    if (!want || !near || !key || !id) return;
    let alive = true;
    void ask(key, () => backend.featuredArtistPhoto(id, name)).then((p) => alive && setFound({ key, photo: p.photo }));
    return () => {
      alive = false;
    };
  }, [want, near, key, id, name]);
  if (given) return given;
  if (!key) return null;
  return found?.key === key ? found.photo : (known.get(key)?.photo ?? null);
}
