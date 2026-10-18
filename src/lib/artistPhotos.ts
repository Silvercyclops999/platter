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
            known.set(key, p);
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

/** True once the element has come near the view (at once without IntersectionObserver). */
export function useNearView(ref: RefObject<HTMLElement | null>, enabled = true): boolean {
  const [near, setNear] = useState(() => typeof IntersectionObserver === "undefined");
  useEffect(() => {
    const el = ref.current;
    if (near || !enabled || !el) return;
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
  const [found, setFound] = useState<CataloguePhoto | null>(() => (mbid ? (known.get(mbid) ?? null) : null));
  useEffect(() => {
    if (!want || !near || !mbid) return;
    let alive = true;
    void ask(mbid, () => backend.catalogueArtistPhoto(mbid, name)).then((p) => alive && setFound(p));
    return () => {
      alive = false;
    };
  }, [want, near, mbid, name]);
  if (given) return { photo: given, credit: null };
  return found ?? { photo: null, credit: null };
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
  const [found, setFound] = useState<string | null>(() => (key ? (known.get(key)?.photo ?? null) : null));
  useEffect(() => {
    if (!want || !near || !id) return;
    let alive = true;
    void ask(`artist:${id}`, () => backend.featuredArtistPhoto(id, name)).then((p) => alive && setFound(p.photo));
    return () => {
      alive = false;
    };
  }, [want, near, id, name]);
  return given ?? found;
}
